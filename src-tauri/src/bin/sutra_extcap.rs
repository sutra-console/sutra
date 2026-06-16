//! sutra-extcap â€” Dutas as live Wireshark capture interfaces.
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
//! (Help â–¸ About â–¸ Folders â–¸ Personal Extcap path). "Duta: <name>" interfaces
//! then appear on the capture screen whenever a Duta is on the LAN.
//!
//! Interfaces come from BOTH transports: mDNS-discovered WebSocket Dutas
//! (coexist with a Sutra USB session â€” the console is teed) and directly
//! USB-attached ones (candidate ports verified with a mux PING; serial is
//! exclusive, so a port Sutra holds is skipped). Raw UART DATA as USER0;
//! when typed streams land (IÂ²C/CAN), records gain real link types and the
//! container moves to pcapng.

use std::io::Write;
use std::time::{Duration, Instant};

use sutra_lib::protocol::{msg, mux, Frame, MuxReader, RESP_FLAG};
use sutra_lib::{serial, ws};

const LINKTYPE_USER0: u32 = 147;
// Bluetooth LE Link Layer with a 10-byte pseudo-header. Wireshark's native
// `btle` dissector reads this directly â€” full BLE decode, no Lua needed.
const LINKTYPE_BLUETOOTH_LE_LL_WITH_PHDR: u32 = 256;
const DATA_KIND_BLE_SNIFF: u8 = 4; // SKRIT_DATA_BLE_SNIFF
                                   // IEEE 802.15.4 TAP: a TLV pseudo-header + the MAC frame. Wireshark's native
                                   // 802.15.4 stack decodes Zigbee/Thread/6LoWPAN/Matter from it â€” no Lua.
const LINKTYPE_IEEE802_15_4_TAP: u32 = 283;
const DATA_KIND_IEEE802154: u8 = 7; // SKRIT_DATA_IEEE802154
const EXTCAP_VERSION: &str = env!("CARGO_PKG_VERSION");

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// The pcap link type + DLT label for a DATA kind. ble-sniff gets the native
/// BLE LL link type; everything else stays the opaque raw-DATA stream (USER0).
fn dlt_for_kind(kind: u8) -> (u32, &'static str, &'static str) {
    match kind {
        DATA_KIND_BLE_SNIFF => (
            LINKTYPE_BLUETOOTH_LE_LL_WITH_PHDR,
            "BLUETOOTH_LE_LL_WITH_PHDR",
            "Bluetooth LE Link Layer (native btle dissector)",
        ),
        DATA_KIND_IEEE802154 => (
            LINKTYPE_IEEE802_15_4_TAP,
            "IEEE802_15_4_TAP",
            "IEEE 802.15.4 TAP (native Zigbee/Thread dissector)",
        ),
        _ => (
            LINKTYPE_USER0,
            "USER0",
            "skrit DATA stream (raw console bytes)",
        ),
    }
}

/// Reframe one ieee802154 DATA record into a LINKTYPE_IEEE802_15_4_TAP packet:
/// a TLV pseudo-header (FCS type, RSS, channel, LQI) followed by the MAC frame.
/// Wireshark's native 802.15.4 dissector decodes the rest (Zigbee / Thread / â€¦).
///
/// Input record (PROTOCOL.md "IEEE 802.15.4 sniffer"):
///   ts_ms(4 LE) Â· channel(1) Â· rssi(1, signed dBm) Â· lqi(1) Â· flags(1) Â·
///     psdu_len(1) Â· psduâ€¦   (psdu includes the 2-byte FCS)
fn ieee802154_tap_packet(rec: &[u8]) -> Option<Vec<u8>> {
    if rec.len() < 9 {
        return None;
    }
    let channel = rec[4];
    let rssi = rec[5] as i8;
    let lqi = rec[6];
    let plen = rec[8] as usize;
    if plen < 2 || rec.len() < 9 + plen {
        return None;
    }
    // The PHR length counts the 2-byte FCS, but the radio doesn't hand us a
    // usable FCS (it checks the CRC in hardware and we drop bad frames). Present
    // the MAC frame WITHOUT the trailing FCS and tell the TAP header "no FCS",
    // so Wireshark dissects cleanly instead of flagging every frame "Bad FCS".
    let psdu = &rec[9..9 + plen - 2];

    // Each TLV: type(u16 LE) Â· length(u16 LE) Â· value, value padded to 4 bytes.
    fn push_tlv(buf: &mut Vec<u8>, typ: u16, val: &[u8]) {
        buf.extend_from_slice(&typ.to_le_bytes());
        buf.extend_from_slice(&(val.len() as u16).to_le_bytes());
        buf.extend_from_slice(val);
        while !buf.len().is_multiple_of(4) {
            buf.push(0);
        }
    }
    let mut tlvs = Vec::new();
    push_tlv(&mut tlvs, 0, &[0u8]); // FCS type: 0 = none present (radio stripped it)
    push_tlv(&mut tlvs, 1, &(rssi as f32).to_le_bytes()); // RSS, dBm (float32)
    let mut ch = (channel as u16).to_le_bytes().to_vec();
    ch.push(0); // channel page 0 (2.4 GHz O-QPSK)
    push_tlv(&mut tlvs, 3, &ch); // channel assignment
    push_tlv(&mut tlvs, 10, &[lqi]); // LQI

    let tap_len = 4 + tlvs.len(); // header is 4-aligned; TLVs each padded to 4
    let mut pkt = Vec::with_capacity(tap_len + plen);
    pkt.push(0); // version
    pkt.push(0); // reserved
    pkt.extend_from_slice(&(tap_len as u16).to_le_bytes());
    pkt.extend_from_slice(&tlvs);
    pkt.extend_from_slice(psdu);
    Some(pkt)
}

/// Reframe one ble-sniff DATA record into a LINKTYPE_BLUETOOTH_LE_LL_WITH_PHDR
/// packet so Wireshark's `btle` dissector decodes it natively.
///
/// Input record (PROTOCOL.md "BLE sniffer"):
///   ts_ms(4 LE) Â· channel(1) Â· rssi(1, magnitude) Â· access-address(4 LE) Â·
///   pdu_len(1) Â· pduâ€¦
/// Output packet: 10-byte LE pseudo-header + access-address(4) + pdu + CRC(3).
/// The radio already de-whitened and CRC-checked, so we set the CRC-valid flag
/// and append a 3-byte CRC placeholder (the real CRC is discarded on-device).
fn ble_ll_packet(rec: &[u8]) -> Option<Vec<u8>> {
    if rec.len() < 11 {
        return None;
    }
    // The phdr's first byte is the PHYSICAL RF channel (0..39 by frequency), not
    // the BLE logical channel index. The dissector uses it to recognise primary
    // advertising channels (logical 37/38/39 = RF 0/12/39); get this wrong and it
    // decodes adv PDUs as data/extended ("Unknown"). We only sniff advertising.
    let rf_channel = match rec[4] {
        37 => 0,
        38 => 12,
        39 => 39,
        other => other, // data channels would map by frequency; unused here
    };
    let signal = (-(rec[5] as i16)) as i8 as u8; // stored magnitude -> negative dBm
    let aa = &rec[6..10]; // access address, little-endian (on-air order)
    let plen = rec[10] as usize;
    if plen < 2 || rec.len() < 11 + plen {
        return None;
    }
    let pdu = &rec[11..11 + plen];

    // flags (LE): dewhitened | signal-valid | ref-AA-valid | CRC-checked | CRC-valid
    let flags: u16 = 0x0001 | 0x0002 | 0x0010 | 0x0400 | 0x0800;
    let mut pkt = Vec::with_capacity(10 + 4 + plen + 3);
    pkt.push(rf_channel); // physical RF channel (frequency index)
    pkt.push(signal); // signal power, dBm
    pkt.push(0); // noise power (not measured; flag left clear)
    pkt.push(0); // access-address offenses
    pkt.extend_from_slice(aa); // reference access address (LE)
    pkt.extend_from_slice(&flags.to_le_bytes());
    // the LL packet itself: AA + PDU + CRC
    pkt.extend_from_slice(aa);
    pkt.extend_from_slice(pdu);
    pkt.extend_from_slice(&[0, 0, 0]); // CRC placeholder (marked valid via flags)
    Some(pkt)
}

/// Frame one DATA record for the chosen link type. ble-sniff records become LL
/// packets; any other kind is written through unchanged (USER0). Returns None to
/// drop a record (malformed ble-sniff capture).
fn frame_record(kind: u8, payload: &[u8]) -> Option<Vec<u8>> {
    match kind {
        DATA_KIND_BLE_SNIFF => ble_ll_packet(payload),
        DATA_KIND_IEEE802154 => ieee802154_tap_packet(payload),
        _ => Some(payload.to_vec()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |name: &str| args.iter().any(|a| a == name);

    if has("--extcap-interfaces") {
        list_interfaces();
    } else if has("--extcap-dlts") {
        // Report the link type that matches what this device actually bridges, so
        // the dropdown agrees with the captured stream (ble-sniff -> native BTLE).
        let iface = arg_value(&args, "--extcap-interface").unwrap_or_default();
        let password = arg_value(&args, "--password").unwrap_or_else(|| "duta".into());
        let kind = probe_kind(&iface, &password).unwrap_or(0);
        let (num, name, display) = dlt_for_kind(kind);
        println!("dlt {{number={num}}}{{name={name}}}{{display={display}}}");
    } else if has("--extcap-config") {
        // The options Wireshark shows in the interface's gear dialog.
        println!("arg {{number=0}}{{call=--password}}{{display=Device password}}{{tooltip=The Duta's session password (AUTH)}}{{type=password}}{{default=duta}}");
        println!("arg {{number=1}}{{call=--max-time}}{{display=Stop after (seconds)}}{{tooltip=0 = capture until stopped}}{{type=integer}}{{default=0}}");
    } else if has("--capture") {
        let iface = arg_value(&args, "--extcap-interface").unwrap_or_default();
        let fifo = arg_value(&args, "--fifo").unwrap_or_default();
        let password = arg_value(&args, "--password").unwrap_or_else(|| "duta".into());
        let max_time: u64 = arg_value(&args, "--max-time")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if iface.is_empty() || fifo.is_empty() {
            eprintln!("--capture needs --extcap-interface and --fifo");
            std::process::exit(1);
        }
        if let Err(e) = capture(&iface, &fifo, &password, max_time) {
            eprintln!("capture failed: {e}");
            std::process::exit(1);
        }
    } else {
        // includes Wireshark's bare `--extcap-version=â€¦` probe
        println!("extcap {{version={EXTCAP_VERSION}}}{{display=Sutra Duta bridge}}{{help=https://github.com/sutra-console/sutra}}");
    }
}

/// List capture interfaces from both transports. The interface *value* is what
/// Wireshark hands back for --capture: a ws:// URL, or a serial port name.
fn list_interfaces() {
    println!("extcap {{version={EXTCAP_VERSION}}}{{display=Sutra Duta bridge}}{{help=https://github.com/sutra-console/sutra}}");
    // USB first: probe candidate ports (skips ports another app holds open).
    for p in serial::list_ports() {
        if !p.is_duta {
            continue;
        }
        if let Some(name) = usb_probe_name(&p.name) {
            println!(
                "interface {{value={port}}}{{display=Duta: {name} â€” {port} (USB, skrit DATA)}}",
                port = p.name
            );
        }
    }
    // Then the network: every Duta advertising _skrit._tcp.
    for d in ws::discover(2500).unwrap_or_default() {
        let label = if d.name.is_empty() {
            d.host.trim_end_matches('.').to_string()
        } else {
            d.name.clone()
        };
        println!(
            "interface {{value={url}}}{{display=Duta: {label} â€” {ip} (WiFi, skrit DATA)}}",
            url = d.url,
            ip = d.ip
        );
    }
}

// ---- USB (serial) leg ----------------------------------------------------------

fn open_usb(port: &str, timeout: Duration) -> Result<Box<dyn serialport::SerialPort>, String> {
    let mut p = serialport::new(port, 115_200)
        .timeout(timeout)
        .open()
        .map_err(|e| format!("open {port}: {e}"))?;
    // ESP32 USB-Serial/JTAG line discipline: DTR only, never RTS.
    let _ = p.write_data_terminal_ready(true);
    Ok(p)
}

fn park_lines(p: &mut Box<dyn serialport::SerialPort>) {
    // Park DTR/RTS low before closing so the OS close order can't form the
    // ESP32 reset/download pattern.
    let _ = p.write_request_to_send(false);
    let _ = p.write_data_terminal_ready(false);
}

/// Verify a candidate port is a muxed Duta and fetch its name. Returns None for
/// busy ports (another app owns it) and non-Dutas.
fn usb_probe_name(port: &str) -> Option<String> {
    let mut p = open_usb(port, Duration::from_millis(100)).ok()?;
    let mut reader = MuxReader::new();
    let mut buf = [0u8; 256];

    let mut roundtrip = |typ: u8, seq: u8| -> Option<Frame> {
        let wire = Frame::new(typ, seq, vec![]).to_mux_wire().ok()?;
        p.write_all(&wire).ok()?;
        let _ = p.flush();
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            let n = match p.read(&mut buf) {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(_) => return None,
            };
            for (ch, payload) in reader.push(&buf[..n]) {
                if ch == mux::CMD {
                    if let Ok(f) = Frame::from_raw(&payload) {
                        if f.typ == (typ | RESP_FLAG) {
                            return Some(f);
                        }
                    }
                }
            }
        }
        None
    };

    let pong = roundtrip(msg::PING, 0xE1);
    let name = if pong.is_some() {
        roundtrip(msg::DEVICE_NAME, 0xE2)
            .and_then(|f| String::from_utf8(f.body.get(1..).unwrap_or_default().to_vec()).ok())
            .filter(|s| !s.is_empty())
    } else {
        None
    };
    park_lines(&mut p);
    if pong.is_some() {
        Some(name.unwrap_or_else(|| "Duta".into()))
    } else {
        None
    }
}

/// Ask a device what its DATA channel carries (DATA_DESC -> kind). Returns the
/// kind byte (uart=0, â€¦, ble-sniff=4, i2c=6), or None if it doesn't answer.
/// Used to pick the pcap link type up front. Best-effort: a device that predates
/// DATA_DESC just looks like uart, so we fall back to the raw USER0 stream.
fn probe_kind(iface: &str, password: &str) -> Option<u8> {
    if iface.is_empty() {
        return None;
    }
    if iface.starts_with("ws://") || iface.starts_with("wss://") {
        probe_kind_ws(iface, password)
    } else {
        let mut p = open_usb(iface, Duration::from_millis(100)).ok()?;
        let mut reader = MuxReader::new();
        let mut buf = [0u8; 256];
        let kind = usb_data_kind(&mut p, &mut reader, &mut buf);
        park_lines(&mut p);
        kind
    }
}

/// DATA_DESC roundtrip on an already-open serial port, reusing the caller's mux
/// reader/buffer so a follow-on capture loses no frames. Reply body = [status,
/// kind, nameâ€¦]; we want body[1].
fn usb_data_kind(
    p: &mut Box<dyn serialport::SerialPort>,
    reader: &mut MuxReader,
    buf: &mut [u8],
) -> Option<u8> {
    let wire = Frame::new(msg::DATA_DESC, 0xD1, vec![])
        .to_mux_wire()
        .ok()?;
    p.write_all(&wire).ok()?;
    let _ = p.flush();
    let deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < deadline {
        let n = match p.read(buf) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(_) => return None,
        };
        for (ch, payload) in reader.push(&buf[..n]) {
            if ch == mux::CMD {
                if let Ok(f) = Frame::from_raw(&payload) {
                    if f.typ == (msg::DATA_DESC | RESP_FLAG) {
                        return f.body.get(1).copied();
                    }
                }
            }
        }
    }
    None
}

/// DATA_DESC over WebSocket (connect + AUTH + DATA_DESC). Best-effort probe for
/// --extcap-dlts; capture() does its own kind probe inline while streaming.
fn probe_kind_ws(url: &str, password: &str) -> Option<u8> {
    let (mut sock, _resp) = tungstenite::connect(url).ok()?;
    let auth = Frame::new(msg::AUTH, 1, password.as_bytes().to_vec())
        .to_mux_wire()
        .ok()?;
    sock.send(tungstenite::Message::Binary(auth)).ok()?;
    let desc = Frame::new(msg::DATA_DESC, 0xD1, vec![])
        .to_mux_wire()
        .ok()?;
    let mut reader = MuxReader::new();
    let mut sent_desc = false;
    let deadline = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < deadline {
        let m = sock.read().ok()?;
        let bytes = match m {
            tungstenite::Message::Binary(b) => b,
            tungstenite::Message::Close(_) => return None,
            _ => continue,
        };
        for (ch, payload) in reader.push(&bytes) {
            if ch != mux::CMD {
                continue;
            }
            if let Ok(f) = Frame::from_raw(&payload) {
                if f.typ == (msg::AUTH | RESP_FLAG) && f.status() == Some(0) && !sent_desc {
                    sock.send(tungstenite::Message::Binary(desc.clone())).ok()?;
                    sent_desc = true;
                } else if f.typ == (msg::DATA_DESC | RESP_FLAG) {
                    let _ = sock.close(None);
                    return f.body.get(1).copied();
                }
            }
        }
    }
    None
}

/// Capture from a USB-attached Duta: demux the serial stream, DATA -> pcap.
/// (No AUTH â€” USB links aren't session-gated.)
fn capture_usb(port: &str, out: &mut std::fs::File, max_time: u64) -> Result<(), String> {
    let mut p = open_usb(port, Duration::from_millis(50))?;
    let mut reader = MuxReader::new();
    let mut buf = [0u8; 512];
    // Probe the DATA kind first so the pcap header carries the right link type;
    // the same reader continues into the stream loop (no frame loss).
    let kind = usb_data_kind(&mut p, &mut reader, &mut buf).unwrap_or(0);
    let (linktype, _, _) = dlt_for_kind(kind);
    out.write_all(&pcap_global_header(linktype))
        .map_err(|e| format!("fifo: {e}"))?;
    out.flush().ok();
    let started = Instant::now();
    loop {
        if max_time > 0 && started.elapsed().as_secs() >= max_time {
            park_lines(&mut p);
            return Ok(());
        }
        let n = match p.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => {
                park_lines(&mut p);
                return Err(format!("serial read: {e}"));
            }
        };
        for (ch, payload) in reader.push(&buf[..n]) {
            if ch != mux::DATA || payload.is_empty() {
                continue;
            }
            if let Some(pkt) = frame_record(kind, &payload) {
                if write_record(out, &pkt).is_err() {
                    park_lines(&mut p); // Wireshark closed its end â€” done
                    return Ok(());
                }
            }
        }
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

fn capture(iface: &str, fifo: &str, password: &str, max_time: u64) -> Result<(), String> {
    // Wireshark creates the fifo (a named pipe on Windows) and reads from it;
    // we open it like a file. For testing, any plain file path works too.
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create(true) // no-op for an existing pipe; lets tests use a file
        .truncate(false)
        .open(fifo)
        .map_err(|e| format!("open fifo {fifo}: {e}"))?;

    // USB writes its own pcap header after probing the kind (see capture_usb).
    if !iface.starts_with("ws://") && !iface.starts_with("wss://") {
        return capture_usb(iface, &mut out, max_time); // a serial port name
    }
    let url = iface;
    let (mut sock, _resp) = tungstenite::connect(url).map_err(|e| format!("connect {url}: {e}"))?;

    // AUTH, then DATA_DESC to learn the kind, then stream. The pcap header is
    // written once we know the link type; DATA before that is dropped (ms).
    let auth = Frame::new(msg::AUTH, 1, password.as_bytes().to_vec())
        .to_mux_wire()
        .map_err(|e| format!("auth frame: {e:?}"))?;
    sock.send(tungstenite::Message::Binary(auth))
        .map_err(|e| format!("auth send: {e}"))?;
    let desc = Frame::new(msg::DATA_DESC, 0xD1, vec![])
        .to_mux_wire()
        .map_err(|e| format!("desc frame: {e:?}"))?;

    let mut reader = MuxReader::new();
    let mut authed = false;
    let mut kind: Option<u8> = None; // Some once the header is written
    let mut auth_at: Option<Instant> = None;
    let started = std::time::Instant::now();

    loop {
        if max_time > 0 && started.elapsed().as_secs() >= max_time {
            return Ok(());
        }
        // Fallback: a device that doesn't answer DATA_DESC (predates it) still
        // captures â€” default to the raw USER0 stream after a short grace period.
        if kind.is_none() {
            if let Some(t) = auth_at {
                if t.elapsed() > Duration::from_millis(1500) {
                    out.write_all(&pcap_global_header(LINKTYPE_USER0))
                        .map_err(|e| format!("fifo: {e}"))?;
                    out.flush().ok();
                    kind = Some(0);
                }
            }
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
                            auth_at = Some(Instant::now());
                            sock.send(tungstenite::Message::Binary(desc.clone()))
                                .map_err(|e| format!("desc send: {e}"))?;
                        } else {
                            return Err("device rejected the password".into());
                        }
                    } else if f.typ == (msg::DATA_DESC | RESP_FLAG) && kind.is_none() {
                        let k = f.body.get(1).copied().unwrap_or(0);
                        let (linktype, _, _) = dlt_for_kind(k);
                        out.write_all(&pcap_global_header(linktype))
                            .map_err(|e| format!("fifo: {e}"))?;
                        out.flush().ok();
                        kind = Some(k);
                    }
                }
            } else if ch == mux::DATA && authed && !payload.is_empty() {
                // Hold DATA until DATA_DESC has set the link type + header.
                if let Some(k) = kind {
                    // Wireshark closing its pipe end surfaces as a write error â€”
                    // that's our stop signal.
                    if let Some(pkt) = frame_record(k, &payload) {
                        if write_record(&mut out, &pkt).is_err() {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // Reframe a known 802.15.4 data frame and drop a one-packet pcap for tshark.
    #[test]
    fn ieee802154_tap_pcap() {
        // MAC data frame: FCF 0x8841 (data, PAN-compressed, short dst+src),
        // seq 1, dst PAN abcd, dst ffff (bcast), src 0000, payload, + 2 FCS bytes.
        let psdu: &[u8] = &[
            0x41, 0x88, 0x01, 0xcd, 0xab, 0xff, 0xff, 0x00, 0x00, 0xde, 0xad, 0x12, 0x34,
        ];
        let mut rec = vec![0u8, 0, 0, 0, 15, 0xD0u8, 0xFF, 0x01, psdu.len() as u8];
        rec.extend_from_slice(psdu);

        let tap = ieee802154_tap_packet(&rec).expect("reframe");
        assert_eq!(tap[0], 0, "version");
        assert_eq!(
            u16::from_le_bytes([tap[2], tap[3]]),
            36,
            "tap header length"
        );
        // MAC frame follows the header, minus the 2-byte FCS we drop.
        assert_eq!(&tap[36..], &psdu[..psdu.len() - 2], "MAC frame (no FCS)");

        let path = std::env::temp_dir().join("duta_154_tap.pcap");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&pcap_global_header(LINKTYPE_IEEE802_15_4_TAP))
            .unwrap();
        write_record(&mut f, &tap).unwrap();
        eprintln!("wrote {}", path.display());
    }
}
