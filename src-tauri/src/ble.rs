//! BLE central — connect a Duta over Bluetooth LE (btleplug).
//!
//! Mirrors the firmware's **dual-channel** BLE model: a Nordic UART Service
//! carries the raw DATA console, and a sibling skrit CMD service carries the
//! framed CMD protocol. We subscribe to both TX characteristics; DATA-TX bytes
//! go to the shared console buffer (so `read_console` + the terminal work
//! unchanged) and CMD-TX frames flow into the existing response matcher
//! (`Shared::mux_rx`), so `send_cmd` works exactly as on a serial link.

use std::sync::Arc;
use std::time::{Duration, Instant};

use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Manager, Peripheral};
use futures::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::protocol::{is_event, Frame, FrameReader, RESP_FLAG};
use crate::serial::{RespFrame, Shared};

// Dual BLE: two skrit GATT services — DATA (NUS-compatible UUID) + CMD (6E41 base).
const DATA_RX: Uuid = Uuid::from_u128(0x6e400002_b5a3_f393_e0a9_e50e24dcca9e); // write
const DATA_TX: Uuid = Uuid::from_u128(0x6e400003_b5a3_f393_e0a9_e50e24dcca9e); // notify
const CMD_SVC: Uuid = Uuid::from_u128(0x6e410001_b5a3_f393_e0a9_e50e24dcca9e);
const CMD_RX: Uuid = Uuid::from_u128(0x6e410002_b5a3_f393_e0a9_e50e24dcca9e); // write
const CMD_TX: Uuid = Uuid::from_u128(0x6e410003_b5a3_f393_e0a9_e50e24dcca9e); // notify

const WRITE_CHUNK: usize = 180; // stay under a typical negotiated ATT MTU

/// A scanned BLE device (shown to the UI for connection).
#[derive(Debug, Clone, Serialize)]
pub struct BleDevice {
    pub id: String,
    pub name: String,
}

/// Live BLE connection — the peripheral plus the two write characteristics.
/// CMD responses ride `Shared::mux_rx` (set up at connect time).
pub struct BleLink {
    pub peripheral: Peripheral,
    pub data_rx: Characteristic,
    pub cmd_rx: Characteristic,
    pub name: String,
}

async fn adapter() -> Result<btleplug::platform::Adapter, String> {
    let mgr = Manager::new().await.map_err(|e| e.to_string())?;
    mgr.adapters()
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .next()
        .ok_or_else(|| "no BLE adapter found".into())
}

/// True if a peripheral looks like a Duta (advertises the skrit CMD service, or
/// is named Duta*).
async fn is_duta(p: &Peripheral) -> Option<String> {
    let props = p.properties().await.ok().flatten()?;
    let name = props.local_name.unwrap_or_default();
    if props.services.contains(&CMD_SVC) || name.starts_with("Duta") {
        Some(name)
    } else {
        None
    }
}

/// Scan for Duta peripherals for `secs` seconds.
pub async fn scan(secs: u64) -> Result<Vec<BleDevice>, String> {
    let adapter = adapter().await?;
    adapter
        .start_scan(ScanFilter { services: vec![CMD_SVC] })
        .await
        .map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let mut out = Vec::new();
    for p in adapter.peripherals().await.map_err(|e| e.to_string())? {
        if let Some(name) = is_duta(&p).await {
            out.push(BleDevice { id: p.id().to_string(), name });
        }
    }
    let _ = adapter.stop_scan().await;
    Ok(out)
}

/// Connect to a scanned device by id, discover the DATA + CMD services, and wire
/// notifications into the shared console + response matcher.
pub async fn connect(shared: Arc<Shared>, app: AppHandle, id: String) -> Result<String, String> {
    crate::serial::disconnect(&shared); // drop any existing (serial or BLE) link

    let adapter = adapter().await?;
    adapter
        .start_scan(ScanFilter { services: vec![CMD_SVC] })
        .await
        .map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let peripheral = adapter
        .peripherals()
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|p| p.id().to_string() == id)
        .ok_or("device not found (re-scan and retry)")?;
    let _ = adapter.stop_scan().await;

    peripheral.connect().await.map_err(|e| format!("connect: {e}"))?;
    peripheral.discover_services().await.map_err(|e| format!("discover: {e}"))?;

    let chars = peripheral.characteristics();
    let find = |u: Uuid| -> Result<Characteristic, String> {
        chars.iter().find(|c| c.uuid == u).cloned().ok_or_else(|| format!("missing characteristic {u}"))
    };
    let data_rx = find(DATA_RX)?;
    let data_tx = find(DATA_TX)?;
    let cmd_rx = find(CMD_RX)?;
    let cmd_tx = find(CMD_TX)?;

    peripheral.subscribe(&data_tx).await.map_err(|e| format!("subscribe DATA: {e}"))?;
    peripheral.subscribe(&cmd_tx).await.map_err(|e| format!("subscribe CMD: {e}"))?;

    // Route notifications: DATA-TX -> console, CMD-TX -> response matcher (mux_rx).
    let (tx, rx) = std::sync::mpsc::channel::<Frame>();
    let notif = peripheral.clone();
    let s2 = shared.clone();
    let a2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut stream = match notif.notifications().await {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut reader = FrameReader::new();
        while let Some(n) = stream.next().await {
            if n.uuid == DATA_TX {
                s2.push_console(&n.value);
                let _ = a2.emit("sutra://data", n.value);
            } else if n.uuid == CMD_TX {
                for f in reader.push(&n.value).into_iter().flatten() {
                    if is_event(f.typ) {
                        let _ = a2.emit("sutra://event", (f.typ, f.body));
                    } else {
                        let _ = tx.send(f);
                    }
                }
            }
        }
        // The notification stream ended — the peripheral disconnected.
        let _ = a2.emit("sutra://link", false);
    });

    let name = is_duta(&peripheral).await.unwrap_or_else(|| "Duta BLE".into());
    *shared.mux_rx_slot() = Some(rx);
    *shared.ble_slot() = Some(BleLink { peripheral, data_rx, cmd_rx, name: name.clone() });
    let _ = app.emit("sutra://link", true);
    Ok(name)
}

/// Send a CMD frame over the BLE CMD service and await the matching response.
/// Mirrors `serial::send_cmd_mux` but writes to a GATT characteristic.
pub fn send_cmd(shared: &Arc<Shared>, typ: u8, body: Vec<u8>) -> Result<RespFrame, String> {
    let _lock = shared.cmd_lock_guard();
    let seq = shared.next_seq();
    // Dual-CDC CMD framing (no mux channel tag): 0x00 COBS(frame) 0x00.
    let wire = Frame::new(typ, seq, body).to_wire().map_err(|e| format!("encode: {e:?}"))?;

    let (peripheral, cmd_rx) = {
        let g = shared.ble_slot();
        let link = g.as_ref().ok_or("not connected (BLE)")?;
        (link.peripheral.clone(), link.cmd_rx.clone())
    };

    // Drain stale frames, then write, then await the matching seq.
    {
        let g = shared.mux_rx_slot();
        if let Some(rx) = g.as_ref() {
            while rx.try_recv().is_ok() {}
        }
    }
    tauri::async_runtime::block_on(peripheral.write(&cmd_rx, &wire, WriteType::WithoutResponse))
        .map_err(|e| format!("BLE write: {e}"))?;

    let g = shared.mux_rx_slot();
    let rx = g.as_ref().ok_or("not connected (BLE)")?;
    let deadline = Instant::now() + Duration::from_millis(2000);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timeout waiting for CMD response (BLE)".into());
        }
        match rx.recv_timeout(remaining) {
            Ok(f) if f.seq == seq && f.typ & RESP_FLAG != 0 => return Ok(f.into()),
            Ok(_) => continue,
            Err(_) => return Err("timeout waiting for CMD response (BLE)".into()),
        }
    }
}

/// Write console bytes to the BLE DATA service (host -> target), chunked to MTU.
pub fn data_write(shared: &Arc<Shared>, bytes: &[u8]) -> Result<(), String> {
    let (peripheral, data_rx) = {
        let g = shared.ble_slot();
        let link = g.as_ref().ok_or("not connected (BLE)")?;
        (link.peripheral.clone(), link.data_rx.clone())
    };
    tauri::async_runtime::block_on(async move {
        for chunk in bytes.chunks(WRITE_CHUNK) {
            peripheral
                .write(&data_rx, chunk, WriteType::WithoutResponse)
                .await
                .map_err(|e| format!("BLE write: {e}"))?;
        }
        Ok(())
    })
}

/// Tear down a BLE link (called from `serial::disconnect`).
pub fn disconnect(shared: &Arc<Shared>) {
    if let Some(link) = shared.ble_slot().take() {
        let p = link.peripheral;
        tauri::async_runtime::spawn(async move {
            let _ = p.disconnect().await;
        });
    }
}
