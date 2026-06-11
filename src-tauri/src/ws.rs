//! WebSocket client: connect a Duta over the network transport.
//!
//! The network transport is muxed (`caps.muxed`), so this reuses the same mux
//! demux + response matcher (`Shared::mux_rx`) as a single-USB-CDC link: a read
//! task feeds incoming WS binary bytes to a `MuxReader` (DATA → console, CMD →
//! `mux_rx`), and `send_cmd`/`data_write` push framed bytes to a write task via
//! a channel. After connecting, it runs the `AUTH` handshake.

use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

use crate::protocol::{is_event, msg, mux_wrap, status, Frame, Frame as F, MuxReader, RESP_FLAG};
use crate::serial::{RespFrame, Shared};

/// Live WebSocket link: a channel to the write task carrying raw wire bytes.
pub struct WsLink {
    tx: UnboundedSender<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WsConnectResult {
    pub name: String,
    pub default_cred: bool, // device still on the factory password; prompt a change
}

/// Connect to `url` (ws:// or wss://), authenticate with `password`, and wire the
/// mux stream into the shared console + response matcher.
pub async fn connect(
    shared: Arc<Shared>,
    app: AppHandle,
    url: String,
    password: String,
) -> Result<WsConnectResult, String> {
    crate::serial::disconnect(&shared); // drop any existing link first

    let (stream, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("connect {url}: {e}"))?;
    let (mut write, mut read) = stream.split();

    let (out_tx, mut out_rx) = unbounded_channel::<Vec<u8>>();
    let (resp_tx, resp_rx) = std::sync::mpsc::channel::<Frame>();

    let s2 = shared.clone();
    let a2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut reader = MuxReader::new();
        loop {
            tokio::select! {
                msg = read.next() => match msg {
                    Some(Ok(Message::Binary(data))) => {
                        for (ch, payload) in reader.push(data.as_ref()) {
                            if ch == crate::protocol::mux::DATA {
                                s2.push_console(&payload);
                                let _ = a2.emit("sutra://data", payload);
                            } else if let Ok(f) = Frame::from_raw(&payload) {
                                if is_event(f.typ) {
                                    let _ = a2.emit("sutra://event", (f.typ, f.body));
                                } else {
                                    let _ = resp_tx.send(f);
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // ping/pong handled by tungstenite
                    Some(Err(_)) => break,
                },
                out = out_rx.recv() => match out {
                    Some(bytes) => {
                        if write.send(Message::Binary(bytes)).await.is_err() {
                            break;
                        }
                    }
                    None => break, // link dropped
                },
            }
        }
        let _ = a2.emit("sutra://link", false);
    });

    *shared.mux_rx_slot() = Some(resp_rx);
    *shared.ws_slot() = Some(WsLink { tx: out_tx });
    let _ = app.emit("sutra://link", true);

    // AUTH handshake: routed through send_cmd (which now targets this WS link).
    let auth = {
        let sh = shared.clone();
        let pw = password.into_bytes();
        tokio::task::spawn_blocking(move || crate::serial::send_cmd(&sh, msg::AUTH, pw))
            .await
            .map_err(|e| e.to_string())??
    };
    if auth.status != Some(status::OK) {
        crate::serial::disconnect(&shared);
        return Err("authentication failed (wrong password?)".into());
    }

    // INFO: read the device name + the default-credential flag (flags = body[9]).
    let (name, default_cred) = {
        let sh = shared.clone();
        let info = tokio::task::spawn_blocking(move || crate::serial::send_cmd(&sh, msg::INFO, vec![]))
            .await
            .map_err(|e| e.to_string())??;
        let default_cred = info.body.get(9).copied().unwrap_or(0) & 0x02 != 0;
        let nm = {
            let sh = shared.clone();
            tokio::task::spawn_blocking(move || crate::serial::send_cmd(&sh, msg::DEVICE_NAME, vec![]))
                .await
                .ok()
                .and_then(|r| r.ok())
                .map(|r| String::from_utf8_lossy(r.body.get(1..).unwrap_or(&[])).into_owned())
                .unwrap_or_else(|| "Duta".into())
        };
        (nm, default_cred)
    };

    Ok(WsConnectResult { name, default_cred })
}

/// Send a CMD over the WebSocket and await the matching response (mux framing).
pub fn send_cmd(shared: &Arc<Shared>, typ: u8, body: Vec<u8>) -> Result<RespFrame, String> {
    use std::time::{Duration, Instant};
    let _lock = shared.cmd_lock_guard();
    let seq = shared.next_seq();
    let wire = F::new(typ, seq, body).to_mux_wire().map_err(|e| format!("encode: {e:?}"))?;

    {
        let g = shared.mux_rx_slot();
        if let Some(rx) = g.as_ref() {
            while rx.try_recv().is_ok() {}
        }
    }
    {
        let g = shared.ws_slot();
        let link = g.as_ref().ok_or("not connected (ws)")?;
        link.tx.send(wire).map_err(|_| "ws link closed")?;
    }
    let g = shared.mux_rx_slot();
    let rx = g.as_ref().ok_or("not connected (ws)")?;
    let deadline = Instant::now() + Duration::from_millis(2000);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timeout waiting for CMD response (ws)".into());
        }
        match rx.recv_timeout(remaining) {
            Ok(f) if f.seq == seq && f.typ & RESP_FLAG != 0 => return Ok(f.into()),
            Ok(_) => continue,
            Err(_) => return Err("timeout waiting for CMD response (ws)".into()),
        }
    }
}

/// Write console bytes to the WebSocket DATA channel (host -> target).
pub fn data_write(shared: &Arc<Shared>, bytes: &[u8]) -> Result<(), String> {
    let g = shared.ws_slot();
    let link = g.as_ref().ok_or("not connected (ws)")?;
    link.tx
        .send(mux_wrap(crate::protocol::mux::DATA, bytes))
        .map_err(|_| "ws link closed".into())
}

/// Tear down a WebSocket link (called from `serial::disconnect`).
pub fn disconnect(shared: &Arc<Shared>) {
    // Dropping the sender closes the write channel, which ends the read/write task.
    *shared.ws_slot() = None;
}

// ---- LAN auto-discovery (mDNS/DNS-SD) ---------------------------------------
// Dutas advertise `_skrit._tcp` once they join WiFi (TXT: name, vendor); browse
// the service type for a few seconds and return everything that resolved.

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredDuta {
    pub name: String, // device name from TXT (falls back to the instance name)
    pub vendor: String,
    pub host: String, // duta-xxxx.local.
    pub ip: String,
    pub port: u16,
    pub url: String, // ready-to-connect ws://ip:port/
}

pub fn discover(timeout_ms: u64) -> Result<Vec<DiscoveredDuta>, String> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};
    let daemon = ServiceDaemon::new().map_err(|e| format!("mdns: {e}"))?;
    let rx = daemon.browse("_skrit._tcp.local.").map_err(|e| format!("mdns browse: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let mut found: Vec<DiscoveredDuta> = Vec::new();
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let ip = match info.get_addresses().iter().find(|a| a.is_ipv4()) {
                    Some(a) => a.to_string(),
                    None => continue,
                };
                let port = info.get_port();
                let txt = |k: &str| {
                    info.get_property_val_str(k).map(str::to_string).unwrap_or_default()
                };
                let mut name = txt("name");
                if name.is_empty() {
                    name = info.get_fullname().split('.').next().unwrap_or("Duta").to_string();
                }
                if !found.iter().any(|d| d.ip == ip && d.port == port) {
                    found.push(DiscoveredDuta {
                        name,
                        vendor: txt("vendor"),
                        host: info.get_hostname().to_string(),
                        ip: ip.clone(),
                        port,
                        url: format!("ws://{ip}:{port}/"),
                    });
                }
            }
            Ok(_) => {}
            Err(_) => break, // timeout
        }
    }
    let _ = daemon.shutdown();
    Ok(found)
}
