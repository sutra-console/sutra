//! sutra-extcap — Dutas as live Wireshark capture interfaces.
//! ============================================================================
//! A Wireshark *extcap* program (the same mechanism Nordic's BLE sniffer uses):
//! Wireshark runs it with --extcap-interfaces to list capture sources, then
//! with --capture to stream packets into a fifo. This binary browses mDNS for
//! Dutas advertising `_skrit._tcp`, connects over WebSocket, authenticates
//! (AUTH, default password "duta"), and writes the device's DATA stream as
//! classic-pcap records (LINKTYPE_USER0): each WS-delivered chunk of console
//! bytes becomes one timestamped packet.
//!
//! Install: copy the built binary into Wireshark's extcap folder
//! (Help ▸ About ▸ Folders ▸ Personal Extcap path). "Duta: <name>" interfaces
//! then appear on the capture screen whenever a Duta is on the LAN.
//!
//! v1 scope: WebSocket Dutas, raw UART DATA as USER0. When typed streams land
//! (I²C/CAN), records gain real link types and the container moves to pcapng.

use std::io::Write;

use sutra_lib::protocol::{msg, mux, Frame, MuxReader, RESP_FLAG};
use sutra_lib::ws;

const LINKTYPE_USER0: u32 = 147;
const EXTCAP_VERSION: &str = env!("CARGO_PKG_VERSION");

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |name: &str| args.iter().any(|a| a == name);

    if has("--extcap-interfaces") {
        list_interfaces();
    } else if has("--extcap-dlts") {
        println!("dlt {{number={LINKTYPE_USER0}}}{{name=USER0}}{{display=skrit DATA stream (raw console bytes)}}");
    } else if has("--extcap-config") {
        // The options Wireshark shows in the interface's gear dialog.
        println!("arg {{number=0}}{{call=--password}}{{display=Device password}}{{tooltip=The Duta's session password (AUTH)}}{{type=password}}{{default=duta}}");
        println!("arg {{number=1}}{{call=--max-time}}{{display=Stop after (seconds)}}{{tooltip=0 = capture until stopped}}{{type=integer}}{{default=0}}");
    } else if has("--capture") {
        let iface = arg_value(&args, "--extcap-interface").unwrap_or_default();
        let fifo = arg_value(&args, "--fifo").unwrap_or_default();
        let password = arg_value(&args, "--password").unwrap_or_else(|| "duta".into());
        let max_time: u64 =
            arg_value(&args, "--max-time").and_then(|v| v.parse().ok()).unwrap_or(0);
        if iface.is_empty() || fifo.is_empty() {
            eprintln!("--capture needs --extcap-interface and --fifo");
            std::process::exit(1);
        }
        if let Err(e) = capture(&iface, &fifo, &password, max_time) {
            eprintln!("capture failed: {e}");
            std::process::exit(1);
        }
    } else {
        // includes Wireshark's bare `--extcap-version=…` probe
        println!("extcap {{version={EXTCAP_VERSION}}}{{display=Sutra Duta bridge}}{{help=https://github.com/sutra-console/sutra}}");
    }
}

/// Browse mDNS and print one extcap interface per Duta. The interface *value*
/// is the ws:// URL — Wireshark hands it back verbatim for --capture.
fn list_interfaces() {
    println!("extcap {{version={EXTCAP_VERSION}}}{{display=Sutra Duta bridge}}{{help=https://github.com/sutra-console/sutra}}");
    let found = ws::discover(2500).unwrap_or_default();
    for d in found {
        let label = if d.name.is_empty() { d.host.trim_end_matches('.').to_string() } else { d.name.clone() };
        println!(
            "interface {{value={url}}}{{display=Duta: {label} — {ip} (skrit DATA)}}",
            url = d.url,
            ip = d.ip
        );
    }
}

// ---- pcap writing ------------------------------------------------------------

fn pcap_global_header(linktype: u32) -> [u8; 24] {
    let mut h = [0u8; 24];
    h[0..4].copy_from_slice(&0xA1B2_C3D4u32.to_le_bytes()); // magic (usec timestamps)
    h[4..6].copy_from_slice(&2u16.to_le_bytes()); // version 2.4
    h[6..8].copy_from_slice(&4u16.to_le_bytes());
    // thiszone(4) + sigfigs(4) stay zero
    h[16..20].copy_from_slice(&65535u32.to_le_bytes()); // snaplen
    h[20..24].copy_from_slice(&linktype.to_le_bytes());
    h
}

fn write_record(out: &mut impl Write, data: &[u8]) -> std::io::Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let mut hdr = [0u8; 16];
    hdr[0..4].copy_from_slice(&(now.as_secs() as u32).to_le_bytes());
    hdr[4..8].copy_from_slice(&now.subsec_micros().to_le_bytes());
    hdr[8..12].copy_from_slice(&(data.len() as u32).to_le_bytes());
    hdr[12..16].copy_from_slice(&(data.len() as u32).to_le_bytes());
    out.write_all(&hdr)?;
    out.write_all(data)?;
    out.flush()
}

// ---- the capture loop ----------------------------------------------------------

fn capture(url: &str, fifo: &str, password: &str, max_time: u64) -> Result<(), String> {
    // Wireshark creates the fifo (a named pipe on Windows) and reads from it;
    // we open it like a file. For testing, any plain file path works too.
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create(true) // no-op for an existing pipe; lets tests use a file
        .truncate(false)
        .open(fifo)
        .map_err(|e| format!("open fifo {fifo}: {e}"))?;
    out.write_all(&pcap_global_header(LINKTYPE_USER0)).map_err(|e| format!("fifo: {e}"))?;
    out.flush().ok();

    let (mut sock, _resp) = tungstenite::connect(url).map_err(|e| format!("connect {url}: {e}"))?;

    // AUTH, then confirm the OK before streaming.
    let auth = Frame::new(msg::AUTH, 1, password.as_bytes().to_vec())
        .to_mux_wire()
        .map_err(|e| format!("auth frame: {e:?}"))?;
    sock.send(tungstenite::Message::Binary(auth)).map_err(|e| format!("auth send: {e}"))?;

    let mut reader = MuxReader::new();
    let mut authed = false;
    let started = std::time::Instant::now();

    loop {
        if max_time > 0 && started.elapsed().as_secs() >= max_time {
            return Ok(());
        }
        let msg_in = match sock.read() {
            Ok(m) => m,
            Err(tungstenite::Error::ConnectionClosed) => return Ok(()),
            Err(e) => return Err(format!("ws read: {e}")),
        };
        let bytes = match msg_in {
            tungstenite::Message::Binary(b) => b,
            tungstenite::Message::Close(_) => return Ok(()),
            _ => continue, // ping/pong handled by tungstenite internally
        };
        for (ch, payload) in reader.push(&bytes) {
            if ch == mux::CMD {
                if let Ok(f) = Frame::from_raw(&payload) {
                    if f.typ == (msg::AUTH | RESP_FLAG) {
                        if f.status() == Some(0) {
                            authed = true;
                        } else {
                            return Err("device rejected the password".into());
                        }
                    }
                }
            } else if ch == mux::DATA && authed && !payload.is_empty() {
                // One console chunk = one pcap packet. Wireshark closing its end
                // of the pipe surfaces as a write error — that's our stop signal.
                if write_record(&mut out, &payload).is_err() {
                    return Ok(());
                }
            }
        }
    }
}
