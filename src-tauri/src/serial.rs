//! Device discovery + connection management for Duta.
//!
//! Shared state (Arc<Shared>) is reachable from both Tauri commands and the
//! embedded MCP server: the live connection, a rolling console buffer, the
//! DATA serial params, and the request sequence counter.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serialport::{DataBits, Parity, SerialPort, SerialPortType, StopBits};
use tauri::{AppHandle, Emitter};

use crate::protocol::{is_event, msg, mux_wrap, Frame, FrameReader, MuxReader};

pub const DUTA_VID: u16 = 0x1209; // pid.codes: CH552 dual-CDC (+ the Zephyr nRF builds)
pub const DUTA_PID: u16 = 0xC550; // the CH552 dual-CDC product id
// VIDs the single-port muxed boards enumerate as. These mark a port as a Duta
// *candidate* — autodetect confirms with a skrit-mux PING before claiming it.
const VID_ESPRESSIF: u16 = 0x303A; // ESP32-S3/C3 native USB-Serial/JTAG
const VID_RASPBERRY_PI: u16 = 0x2E8A; // RP2040/RP2350 (arduino-pico)

/// Could this port be a Duta? Exact ids plus the vendor ids our muxed boards
/// use; a candidate is only *claimed* after it answers a skrit-mux PING.
fn duta_candidate(vid: Option<u16>) -> bool {
    matches!(vid, Some(DUTA_VID) | Some(VID_ESPRESSIF) | Some(VID_RASPBERRY_PI))
}
const CONSOLE_CAP: usize = 64 * 1024; // rolling DATA-console buffer for the UI/MCP

#[derive(Debug, Clone, Serialize)]
pub struct PortDesc {
    pub name: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub product: Option<String>,
    pub manufacturer: Option<String>,
    pub serial_number: Option<String>,
    pub is_duta: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RespFrame {
    pub typ: u8,
    pub seq: u8,
    pub status: Option<u8>,
    pub body: Vec<u8>,
}

impl From<Frame> for RespFrame {
    fn from(f: Frame) -> Self {
        RespFrame { status: f.status(), typ: f.typ, seq: f.seq, body: f.body }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialParams {
    pub baud: u32,
    pub data_bits: u8,
    pub parity: String, // "none" | "odd" | "even"
    pub stop_bits: u8,  // 1 | 2
}

impl Default for SerialParams {
    fn default() -> Self {
        SerialParams { baud: 115_200, data_bits: 8, parity: "none".into(), stop_bits: 1 }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnState {
    pub connected: bool,
    pub data_port: Option<String>,
    pub cmd_port: Option<String>,
    pub has_cmd: bool, // true only with a Duta (relays/LED/INFO available)
    pub params: SerialParams,
}

/// A macro as mirrored from the app. The MCP server holds this so it can
/// *run* macros by name without ever exposing `text` (secrets stay hidden).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroRec {
    pub name: String,
    pub text: String,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub set: String, // project/collection this macro belongs to ("" = default)
    /// skrit-mc tier (1=replay, 2=interactive, 3=app-only). Derived from `text`;
    /// recomputed on every read, so a stored/incoming value is ignored.
    #[serde(default)]
    pub tier: u8,
}

/// Name-only view returned to the LLM (no `text`).
#[derive(Debug, Clone, Serialize)]
pub struct MacroMeta {
    pub name: String,
    pub secret: bool,
}

/// Which groups of MCP tools are exposed to the LLM (set from the Settings page).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolFlags {
    pub console_read: bool,
    pub console_write: bool,
    pub outputs: bool,
    pub macros_run: bool,
    pub macros_create: bool,
    pub connection: bool,
}

impl Default for McpToolFlags {
    fn default() -> Self {
        McpToolFlags {
            console_read: true,
            console_write: true,
            outputs: true,
            macros_run: true,
            macros_create: true,
            connection: true,
        }
    }
}

struct Connection {
    cmd: Option<Box<dyn SerialPort>>, // None for a generic port or a muxed Duta
    data_writer: Box<dyn SerialPort>, // also the CMD writer on a muxed link
    stop: Arc<AtomicBool>,
    muxed: bool, // single port carries DATA + CMD via skrit-mux
    // The reader thread owns a cloned port handle; disconnect must JOIN it or
    // the OS keeps the port busy and the next open fails (reconnect bug).
    reader: Option<std::thread::JoinHandle<()>>,
}

/// Rolling console buffer with a monotonic total, so a macro can scan only the
/// output that arrived after a given point (for WAITFOR / RUN sentinels).
#[derive(Default)]
struct ConsoleBuf {
    buf: VecDeque<u8>,
    total: u64,
}

impl ConsoleBuf {
    fn push(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.buf.len() >= CONSOLE_CAP {
                self.buf.pop_front();
            }
            self.buf.push_back(b);
        }
        self.total += bytes.len() as u64;
    }
    fn tail(&self, max: usize) -> Vec<u8> {
        let n = self.buf.len().min(max);
        self.buf.iter().skip(self.buf.len() - n).copied().collect()
    }
    /// Bytes received since `seq`, plus the new total.
    fn since(&self, seq: u64) -> (Vec<u8>, u64) {
        let oldest = self.total - self.buf.len() as u64;
        let start = if seq < oldest { 0 } else { (seq - oldest) as usize };
        (self.buf.iter().skip(start).copied().collect(), self.total)
    }
}

pub struct Shared {
    conn: Mutex<Option<Connection>>,
    seq: Mutex<u8>,
    console: Mutex<ConsoleBuf>,
    params: Mutex<SerialParams>,
    data_name: Mutex<Option<String>>,
    cmd_name: Mutex<Option<String>>,
    macros: Mutex<Vec<MacroRec>>,
    runs: Mutex<Vec<MacroRun>>, // in-flight macro runs (cancellable)
    macros_path: Mutex<Option<std::path::PathBuf>>,
    app: Mutex<Option<AppHandle>>,
    mcp_tools: Mutex<McpToolFlags>,
    // skrit-mux + BLE: the reader thread/notification task delivers CMD responses
    // here; send_cmd consumes them. cmd_lock serializes a request/response round-trip.
    mux_rx: Mutex<Option<std::sync::mpsc::Receiver<Frame>>>,
    cmd_lock: Mutex<()>,
    // Active BLE link, if connected over Bluetooth instead of serial.
    ble: Mutex<Option<crate::ble::BleLink>>,
    // Active WebSocket link, if connected over the network.
    ws: Mutex<Option<crate::ws::WsLink>>,
}

impl Default for Shared {
    fn default() -> Self {
        Shared {
            conn: Mutex::new(None),
            seq: Mutex::new(0),
            console: Mutex::new(ConsoleBuf::default()),
            params: Mutex::new(SerialParams::default()),
            data_name: Mutex::new(None),
            cmd_name: Mutex::new(None),
            macros: Mutex::new(Vec::new()),
            runs: Mutex::new(Vec::new()),
            macros_path: Mutex::new(None),
            app: Mutex::new(None),
            mcp_tools: Mutex::new(McpToolFlags::default()),
            mux_rx: Mutex::new(None),
            cmd_lock: Mutex::new(()),
            ble: Mutex::new(None),
            ws: Mutex::new(None),
        }
    }
}

impl Shared {
    pub(crate) fn next_seq(&self) -> u8 {
        let mut s = self.seq.lock().unwrap();
        *s = s.wrapping_add(1);
        *s
    }

    pub(crate) fn push_console(&self, bytes: &[u8]) {
        self.console.lock().unwrap().push(bytes);
    }

    // Accessors so the BLE module can share the response matcher + cmd serialization.
    pub(crate) fn ble_slot(&self) -> std::sync::MutexGuard<'_, Option<crate::ble::BleLink>> {
        self.ble.lock().unwrap()
    }
    pub(crate) fn ws_slot(&self) -> std::sync::MutexGuard<'_, Option<crate::ws::WsLink>> {
        self.ws.lock().unwrap()
    }
    pub(crate) fn mux_rx_slot(
        &self,
    ) -> std::sync::MutexGuard<'_, Option<std::sync::mpsc::Receiver<Frame>>> {
        self.mux_rx.lock().unwrap()
    }
    pub(crate) fn cmd_lock_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.cmd_lock.lock().unwrap()
    }
}

// ---- discovery -------------------------------------------------------------

pub fn list_ports() -> Vec<PortDesc> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let (vid, pid, product, manufacturer, serial_number) = match p.port_type {
                SerialPortType::UsbPort(u) => {
                    (Some(u.vid), Some(u.pid), u.product, u.manufacturer, u.serial_number)
                }
                _ => (None, None, None, None, None),
            };
            let is_duta = duta_candidate(vid);
            PortDesc { name: p.port_name, vid, pid, product, manufacturer, serial_number, is_duta }
        })
        .collect()
}

fn open_cmd(name: &str, timeout_ms: u64) -> Result<Box<dyn SerialPort>, String> {
    serialport::new(name, 115_200)
        .timeout(Duration::from_millis(timeout_ms))
        .open()
        .map_err(|e| format!("open {name}: {e}"))
}

fn open_data(name: &str, p: &SerialParams) -> Result<Box<dyn SerialPort>, String> {
    let data_bits = match p.data_bits {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    };
    let parity = match p.parity.as_str() {
        "odd" => Parity::Odd,
        "even" => Parity::Even,
        _ => Parity::None,
    };
    let stop_bits = if p.stop_bits == 2 { StopBits::Two } else { StopBits::One };
    serialport::new(name, p.baud)
        .data_bits(data_bits)
        .parity(parity)
        .stop_bits(stop_bits)
        .timeout(Duration::from_millis(50))
        .open()
        .map_err(|e| format!("open {name}: {e}"))
}

fn read_response(port: &mut Box<dyn SerialPort>, timeout_ms: u64) -> Result<Frame, String> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut reader = FrameReader::new();
    let mut buf = [0u8; 128];
    while Instant::now() < deadline {
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                if let Some(r) = reader.push(&buf[..n]).into_iter().next() {
                    return r.map_err(|e| format!("frame error: {e:?}"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(format!("read: {e}")),
        }
    }
    Err("timeout waiting for CMD response".into())
}

pub fn probe_is_cmd(name: &str) -> bool {
    let mut port = match open_cmd(name, 250) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let _ = port.write_data_terminal_ready(true);
    let frame = match Frame::new(msg::PING, 0xA5, vec![]).to_wire() {
        Ok(w) => w,
        Err(_) => return false,
    };
    if port.write_all(&frame).is_err() {
        return false;
    }
    let _ = port.flush();
    matches!(read_response(&mut port, 400), Ok(f) if f.typ == (msg::PING | crate::protocol::RESP_FLAG))
}

pub fn autodetect() -> Result<(String, String), String> {
    // dual-CDC pairing only applies to the exact CH552 id — candidate VIDs
    // (ESP32 / Pico) are single-port muxed and must not be paired here.
    let ports: Vec<String> = list_ports()
        .into_iter()
        .filter(|p| p.vid == Some(DUTA_VID) && p.pid == Some(DUTA_PID))
        .map(|p| p.name)
        .collect();
    if ports.len() < 2 {
        return Err(format!("expected 2 Duta ports, found {}", ports.len()));
    }
    for cand in &ports {
        if probe_is_cmd(cand) {
            let data = ports.iter().find(|n| *n != cand).cloned().unwrap();
            return Ok((data, cand.clone()));
        }
    }
    Ok((ports[0].clone(), ports[1].clone()))
}

/// Probe every Duta-candidate port with a skrit-mux PING; first to answer wins.
pub fn autodetect_mux() -> Result<String, String> {
    let cands: Vec<String> =
        list_ports().into_iter().filter(|p| p.is_duta).map(|p| p.name).collect();
    if cands.is_empty() {
        return Err("no Duta-capable ports found".into());
    }
    for name in &cands {
        if probe_is_mux(name) {
            return Ok(name.clone());
        }
    }
    Err(format!("no candidate port answered skrit-mux (probed {})", cands.join(", ")))
}

// ---- connection ------------------------------------------------------------

fn spawn_data_reader(
    app: AppHandle,
    mut port: Box<dyn SerialPort>,
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        let mut online = true;
        while !stop.load(Ordering::Relaxed) {
            match port.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => {
                    if !online {
                        online = true;
                        let _ = app.emit("sutra://link", true); // target came back
                    }
                    shared.push_console(&buf[..n]);
                    let _ = app.emit("sutra://data", buf[..n].to_vec());
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => {
                    // The port dropped (unplug / device reset). Keep the
                    // connection open, report the link offline, and try to reopen
                    // so it comes back online on its own.
                    if online {
                        online = false;
                        let _ = app.emit("sutra://link", false);
                    }
                    // nap before retrying, but wake promptly on stop (snappy disconnect)
                    for _ in 0..15 {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    let name = match shared.data_name.lock().unwrap().clone() {
                        Some(n) => n,
                        None => break,
                    };
                    let params = shared.params.lock().unwrap().clone();
                    if let Ok(mut fresh) = open_data(&name, &params) {
                        let _ = fresh.write_data_terminal_ready(true);
                        if let Ok(rd) = fresh.try_clone() {
                            let mut guard = shared.conn.lock().unwrap();
                            match guard.as_mut() {
                                // still our connection: swap in the fresh handles
                                Some(c) if Arc::ptr_eq(&c.stop, &stop) => {
                                    c.data_writer = fresh;
                                    drop(guard);
                                    port = rd;
                                }
                                // replaced by a reconnect, or disconnected
                                _ => return,
                            }
                        }
                    }
                }
            }
        }
    })
}

/// Connect a DATA port. `cmd_name` is the Duta CMD interface, or None for a
/// generic serial port (console only, no relay/LED/INFO).
pub fn connect(
    shared: &Arc<Shared>,
    app: AppHandle,
    data_name: &str,
    cmd_name: Option<&str>,
) -> Result<(), String> {
    disconnect(shared);
    let params = shared.params.lock().unwrap().clone();

    let mut data = open_data(data_name, &params)?;
    let _ = data.write_data_terminal_ready(true);

    let cmd = match cmd_name {
        Some(n) => {
            let mut c = open_cmd(n, 500)?;
            // firmware gates CMD replies on DTR
            let _ = c.write_data_terminal_ready(true);
            let _ = c.write_request_to_send(true);
            Some(c)
        }
        None => None,
    };

    let reader = data.try_clone().map_err(|e| format!("clone data: {e}"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let handle = spawn_data_reader(app.clone(), reader, stop.clone(), shared.clone());

    *shared.mux_rx.lock().unwrap() = None;
    *shared.data_name.lock().unwrap() = Some(data_name.to_string());
    *shared.cmd_name.lock().unwrap() = cmd_name.map(|s| s.to_string());
    *shared.conn.lock().unwrap() =
        Some(Connection { cmd, data_writer: data, stop, muxed: false, reader: Some(handle) });
    let _ = app.emit("sutra://connected", ()); // sync the UI (esp. for MCP-initiated connects)
    Ok(())
}

// ---- skrit-mux: one port carrying DATA + CMD -------------------------------

/// Read a single muxed port until a CMD-channel response frame arrives or the
/// deadline passes (used to probe whether a port is a muxed Duta).
fn read_mux_response(port: &mut Box<dyn SerialPort>, timeout_ms: u64) -> Option<Frame> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut reader = MuxReader::new();
    let mut buf = [0u8; 128];
    while Instant::now() < deadline {
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                for (ch, payload) in reader.push(&buf[..n]) {
                    if ch == crate::protocol::mux::CMD {
                        if let Ok(f) = Frame::from_raw(&payload) {
                            if f.is_response() {
                                return Some(f);
                            }
                        }
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return None,
        }
    }
    None
}

/// True if `name` answers a skrit-mux PING with a PONG, i.e. it's a single-port
/// (ESP32 / Pico / nRF) Duta rather than a dual-CDC one or a plain console.
pub fn probe_is_mux(name: &str) -> bool {
    let params = SerialParams::default();
    let mut port = match open_data(name, &params) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let _ = port.write_data_terminal_ready(true);
    let wire = match Frame::new(msg::PING, 0xA6, vec![]).to_mux_wire() {
        Ok(w) => w,
        Err(_) => return false,
    };
    if port.write_all(&wire).is_err() {
        return false;
    }
    let _ = port.flush();
    let ok = matches!(read_mux_response(&mut port, 400),
             Some(f) if f.typ == (msg::PING | crate::protocol::RESP_FLAG));
    // park the lines so the close can't form the ESP32 USJ reset pattern
    let _ = port.write_data_terminal_ready(false);
    ok
}

fn spawn_mux_reader(
    app: AppHandle,
    mut port: Box<dyn SerialPort>,
    tx: std::sync::mpsc::Sender<Frame>,
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = MuxReader::new();
        let mut buf = [0u8; 256];
        let mut online = true;
        while !stop.load(Ordering::Relaxed) {
            match port.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => {
                    if !online {
                        online = true;
                        let _ = app.emit("sutra://link", true);
                    }
                    for (ch, payload) in reader.push(&buf[..n]) {
                        if ch == crate::protocol::mux::DATA {
                            shared.push_console(&payload);
                            let _ = app.emit("sutra://data", payload);
                        } else if let Ok(f) = Frame::from_raw(&payload) {
                            if is_event(f.typ) {
                                let _ = app.emit("sutra://event", (f.typ, f.body));
                            } else {
                                let _ = tx.send(f); // a CMD response for send_cmd
                            }
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => {
                    if online {
                        online = false;
                        let _ = app.emit("sutra://link", false);
                    }
                    // nap before retrying, but wake promptly on stop so disconnect
                    // doesn't block joining us for the whole retry interval.
                    for _ in 0..15 {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    let name = match shared.data_name.lock().unwrap().clone() {
                        Some(n) => n,
                        None => break,
                    };
                    let params = shared.params.lock().unwrap().clone();
                    if let Ok(mut fresh) = open_data(&name, &params) {
                        let _ = fresh.write_data_terminal_ready(true);
                        if let Ok(rd) = fresh.try_clone() {
                            let mut guard = shared.conn.lock().unwrap();
                            match guard.as_mut() {
                                Some(c) if Arc::ptr_eq(&c.stop, &stop) => {
                                    c.data_writer = fresh;
                                    drop(guard);
                                    port = rd;
                                    reader = MuxReader::new();
                                }
                                _ => return,
                            }
                        }
                    }
                }
            }
        }
    })
}

/// Connect a single muxed Duta port (ESP32 / Pico / nRF52840): DATA + CMD share
/// one stream via skrit-mux. The reader demuxes; `send_cmd` works as on dual.
pub fn connect_muxed(shared: &Arc<Shared>, app: AppHandle, name: &str) -> Result<(), String> {
    disconnect(shared);
    let params = shared.params.lock().unwrap().clone();
    let mut port = open_data(name, &params)?;
    // DTR only — NEVER raise RTS on a muxed board: the ESP32 USB-Serial/JTAG
    // turns DTR/RTS edge patterns into chip reset / download-mode entry
    // (rst:0x15), bricking the session until a reflash. Hardware-verified.
    let _ = port.write_data_terminal_ready(true);

    let reader = port.try_clone().map_err(|e| format!("clone port: {e}"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = spawn_mux_reader(app.clone(), reader, tx, stop.clone(), shared.clone());

    *shared.mux_rx.lock().unwrap() = Some(rx);
    *shared.data_name.lock().unwrap() = Some(name.to_string());
    *shared.cmd_name.lock().unwrap() = Some(name.to_string());
    *shared.conn.lock().unwrap() =
        Some(Connection { cmd: None, data_writer: port, stop, muxed: true, reader: Some(handle) });
    let _ = app.emit("sutra://connected", ()); // sync the UI (esp. for MCP-initiated connects)
    Ok(())
}

/// What `autodetect_any` found: a dual-CDC Duta (two ports) or a muxed one (one).
pub enum Detected {
    Dual { data: String, cmd: String },
    Mux(String),
}

/// Find a connected Duta, dual-CDC or muxed: a CH552 dual-CDC pair wins, else
/// the first candidate port that answers a skrit-mux PING.
pub fn autodetect_any() -> Result<Detected, String> {
    if let Ok((data, cmd)) = autodetect() {
        return Ok(Detected::Dual { data, cmd });
    }
    autodetect_mux().map(Detected::Mux)
}

/// MCP/auto connect: detect a Duta (dual or muxed) and connect it. Returns a
/// short human description of what it connected.
pub fn mcp_connect_auto(shared: &Arc<Shared>) -> Result<String, String> {
    let app = shared.app.lock().unwrap().clone().ok_or("app handle not ready")?;
    match autodetect_any()? {
        Detected::Dual { data, cmd } => {
            connect(shared, app, &data, Some(&cmd))?;
            Ok(format!("connected dual-CDC Duta (DATA={data}, CMD={cmd})"))
        }
        Detected::Mux(port) => {
            connect_muxed(shared, app, &port)?;
            Ok(format!("connected muxed Duta on {port}"))
        }
    }
}

/// Re-open just the DATA port with the current serial params (keeps CMD).
pub fn reconnect_data(shared: &Arc<Shared>, app: AppHandle) -> Result<(), String> {
    let data_name = shared.data_name.lock().unwrap().clone().ok_or("not connected")?;
    let params = shared.params.lock().unwrap().clone();
    // Stop + join the old reader BEFORE reopening: its cloned handle keeps the
    // port busy and the fresh open would fail.
    let old = {
        let mut guard = shared.conn.lock().unwrap();
        let conn = guard.as_mut().ok_or("not connected")?;
        conn.stop.store(true, Ordering::Relaxed);
        conn.reader.take()
    };
    if let Some(h) = old {
        let _ = h.join();
    }
    let mut data = open_data(&data_name, &params)?;
    let _ = data.write_data_terminal_ready(true);
    let reader = data.try_clone().map_err(|e| format!("clone data: {e}"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let handle = spawn_data_reader(app, reader, stop.clone(), shared.clone());
    {
        let mut guard = shared.conn.lock().unwrap();
        let conn = guard.as_mut().ok_or("not connected")?;
        conn.data_writer = data;
        conn.stop = stop;
        conn.reader = Some(handle);
    }
    Ok(())
}

pub fn set_params(shared: &Arc<Shared>, app: AppHandle, params: SerialParams) -> Result<(), String> {
    *shared.params.lock().unwrap() = params;
    if shared.conn.lock().unwrap().is_some() {
        reconnect_data(shared, app)?;
    }
    Ok(())
}

pub fn disconnect(shared: &Arc<Shared>) {
    // Clear the names FIRST: the reader thread's auto-reopen path re-acquires
    // the port by name, and a zombie holding the handle breaks the next connect.
    *shared.data_name.lock().unwrap() = None;
    *shared.cmd_name.lock().unwrap() = None;
    // Take the connection out as a STATEMENT so the conn mutex guard is dropped
    // here — NOT held across the join() below. (In edition 2021 an `if let`
    // scrutinee's temporaries live for the whole block; holding the lock while
    // joining deadlocks the reader thread, which locks conn in its reopen path.)
    let taken = shared.conn.lock().unwrap().take();
    if let Some(mut conn) = taken {
        conn.stop.store(true, Ordering::Relaxed);
        let reader = conn.reader.take();
        // Park DTR/RTS low BEFORE closing: the OS-defined line order on close
        // otherwise forms the ESP32 USB-Serial/JTAG reset/download pattern.
        if let Some(mut c) = conn.cmd {
            let _ = c.write_request_to_send(false);
            let _ = c.write_data_terminal_ready(false);
            drop(c);
        }
        let mut dw = conn.data_writer;
        let _ = dw.write_request_to_send(false);
        let _ = dw.write_data_terminal_ready(false);
        drop(dw);
        // Join outside any lock: the OS only frees the port once the reader's
        // cloned handle drops (read timeout 50ms; worst case its 750ms retry nap).
        if let Some(h) = reader {
            let _ = h.join();
        }
    }
    crate::ble::disconnect(shared);
    crate::ws::disconnect(shared);
    *shared.mux_rx.lock().unwrap() = None;
}

pub fn state(shared: &Arc<Shared>) -> ConnState {
    // Network (WS) link.
    if shared.ws.lock().unwrap().is_some() {
        return ConnState {
            connected: true,
            data_port: Some("WebSocket".into()),
            cmd_port: None,
            has_cmd: true,
            params: shared.params.lock().unwrap().clone(),
        };
    }
    // BLE link: report it as a connected "port" with the CMD channel available.
    if let Some(link) = shared.ble.lock().unwrap().as_ref() {
        return ConnState {
            connected: true,
            data_port: Some(format!("BLE: {}", link.name)),
            cmd_port: None,
            has_cmd: true,
            params: shared.params.lock().unwrap().clone(),
        };
    }
    // A muxed link has the CMD channel too (INFO/relays available over the mux).
    let has_cmd =
        shared.conn.lock().unwrap().as_ref().is_some_and(|c| c.cmd.is_some() || c.muxed);
    ConnState {
        connected: shared.conn.lock().unwrap().is_some(),
        data_port: shared.data_name.lock().unwrap().clone(),
        cmd_port: shared.cmd_name.lock().unwrap().clone(),
        has_cmd,
        params: shared.params.lock().unwrap().clone(),
    }
}

pub fn get_params(shared: &Arc<Shared>) -> SerialParams {
    shared.params.lock().unwrap().clone()
}

/// Store params without reconnecting (used right before a connect).
pub fn store_params(shared: &Arc<Shared>, params: SerialParams) {
    *shared.params.lock().unwrap() = params;
}

pub fn get_mcp_tools(shared: &Arc<Shared>) -> McpToolFlags {
    shared.mcp_tools.lock().unwrap().clone()
}

pub fn set_mcp_tools(shared: &Arc<Shared>, flags: McpToolFlags) {
    *shared.mcp_tools.lock().unwrap() = flags;
}

/// connect/set_params variants the MCP server can call: they pull the AppHandle
/// stashed in Shared (set during app setup) so the reader thread can be spawned.
pub fn mcp_connect(shared: &Arc<Shared>, data_name: &str, cmd_name: Option<&str>) -> Result<(), String> {
    let app = shared.app.lock().unwrap().clone().ok_or("app handle not ready")?;
    connect(shared, app, data_name, cmd_name)
}

pub fn mcp_set_params(shared: &Arc<Shared>, params: SerialParams) -> Result<(), String> {
    let app = shared.app.lock().unwrap().clone().ok_or("app handle not ready")?;
    set_params(shared, app, params)
}

pub fn data_write(shared: &Arc<Shared>, bytes: &[u8]) -> Result<(), String> {
    if shared.ws.lock().unwrap().is_some() {
        return crate::ws::data_write(shared, bytes);
    }
    if shared.ble.lock().unwrap().is_some() {
        return crate::ble::data_write(shared, bytes);
    }
    let mut guard = shared.conn.lock().unwrap();
    let conn = guard.as_mut().ok_or("not connected")?;
    // On a muxed link the console rides the DATA channel; otherwise it's the raw port.
    if conn.muxed {
        conn.data_writer
            .write_all(&mux_wrap(crate::protocol::mux::DATA, bytes))
            .map_err(|e| format!("data write: {e}"))?;
    } else {
        conn.data_writer.write_all(bytes).map_err(|e| format!("data write: {e}"))?;
    }
    conn.data_writer.flush().ok();
    Ok(())
}

pub fn send_cmd(shared: &Arc<Shared>, typ: u8, body: Vec<u8>) -> Result<RespFrame, String> {
    if shared.ws.lock().unwrap().is_some() {
        return crate::ws::send_cmd(shared, typ, body);
    }
    if shared.ble.lock().unwrap().is_some() {
        return crate::ble::send_cmd(shared, typ, body);
    }
    let seq = shared.next_seq();
    let muxed = shared.conn.lock().unwrap().as_ref().is_some_and(|c| c.muxed);
    if muxed {
        return send_cmd_mux(shared, typ, seq, body);
    }
    let wire = Frame::new(typ, seq, body).to_wire().map_err(|e| format!("encode: {e:?}"))?;
    let mut guard = shared.conn.lock().unwrap();
    let conn = guard.as_mut().ok_or("not connected")?;
    let cmd = conn
        .cmd
        .as_mut()
        .ok_or("no command port: connect a Duta for relay/LED/INFO")?;
    cmd.write_all(&wire).map_err(|e| format!("cmd write: {e}"))?;
    cmd.flush().ok();
    let resp = read_response(cmd, 1000)?;
    Ok(resp.into())
}

/// Send a CMD on a muxed link: write the wrapped frame, then wait for the
/// reader thread to deliver the matching-seq response. `cmd_lock` serializes the
/// whole round-trip so concurrent callers don't steal each other's replies.
fn send_cmd_mux(shared: &Arc<Shared>, typ: u8, seq: u8, body: Vec<u8>) -> Result<RespFrame, String> {
    let _lock = shared.cmd_lock.lock().unwrap();
    let wire = Frame::new(typ, seq, body).to_mux_wire().map_err(|e| format!("encode: {e:?}"))?;
    // Hold the receiver for the whole round-trip; drain any stale frame BEFORE we
    // write, so a fast reader can't have its real response discarded afterwards.
    let rx_guard = shared.mux_rx.lock().unwrap();
    let rx = rx_guard.as_ref().ok_or("not connected")?;
    while rx.try_recv().is_ok() {}
    {
        let mut guard = shared.conn.lock().unwrap();
        let conn = guard.as_mut().ok_or("not connected")?;
        conn.data_writer.write_all(&wire).map_err(|e| format!("cmd write: {e}"))?;
        conn.data_writer.flush().ok();
    }
    let deadline = Instant::now() + Duration::from_millis(1000);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timeout waiting for CMD response".into());
        }
        match rx.recv_timeout(remaining) {
            Ok(f) if f.seq == seq && f.is_response() => return Ok(f.into()),
            Ok(_) => continue, // a response to some other request; keep waiting
            Err(_) => return Err("timeout waiting for CMD response".into()),
        }
    }
}

/// Last `max` bytes of the DATA console as lossy UTF-8.
pub fn read_console(shared: &Arc<Shared>, max: usize) -> String {
    String::from_utf8_lossy(&shared.console.lock().unwrap().tail(max)).into_owned()
}

fn console_since(shared: &Arc<Shared>, seq: u64) -> (Vec<u8>, u64) {
    shared.console.lock().unwrap().since(seq)
}

fn console_seq(shared: &Arc<Shared>) -> u64 {
    shared.console.lock().unwrap().total
}

// ---- macro store (backend-owned, persisted, mirrored to UI + MCP) --------

/// Wire up the persistence path + app handle and load macros from disk.
pub fn init_macros(shared: &Arc<Shared>, app: AppHandle, path: std::path::PathBuf) {
    *shared.app.lock().unwrap() = Some(app);
    if let Ok(data) = std::fs::read(&path) {
        if let Ok(list) = serde_json::from_slice::<Vec<MacroRec>>(&data) {
            *shared.macros.lock().unwrap() = list;
        }
    }
    *shared.macros_path.lock().unwrap() = Some(path);
}

/// Re-point the macro store at a new path (workspace change). If the new file
/// exists, its macros replace the in-memory set; otherwise the current set is
/// migrated to it. Persists + notifies the UI either way.
pub fn relocate_macros(shared: &Arc<Shared>, path: std::path::PathBuf) {
    if let Ok(data) = std::fs::read(&path) {
        if let Ok(list) = serde_json::from_slice::<Vec<MacroRec>>(&data) {
            *shared.macros.lock().unwrap() = list;
        }
    }
    *shared.macros_path.lock().unwrap() = Some(path);
    persist(shared); // writes the (possibly migrated) set + emits sutra://macros
}

fn persist(shared: &Arc<Shared>) {
    let list = shared.macros.lock().unwrap().clone();
    if let Some(path) = shared.macros_path.lock().unwrap().clone() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&list) {
            let _ = std::fs::write(&path, json);
        }
    }
    // notify the UI so LLM-created/changed macros appear live (with fresh tiers)
    if let Some(app) = shared.app.lock().unwrap().clone() {
        let _ = app.emit("sutra://macros", &tiered(&list));
    }
}

/// Full macro list (for the app UI: includes text + derived `tier`).
pub fn macros_all(shared: &Arc<Shared>) -> Vec<MacroRec> {
    tiered(&shared.macros.lock().unwrap())
}

/// Literal strings typed by SECRET macros (bare lines + STRING args, escapes
/// applied): the bytes that could echo back. Used to redact MCP console reads.
pub fn secret_literals(shared: &Arc<Shared>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let snips = shared.macros.lock().unwrap();
    for s in snips.iter().filter(|s| s.secret) {
        for raw in s.text.split('\n') {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            let trimmed = line.trim_start();
            let mut w = trimmed.splitn(2, char::is_whitespace);
            let mut kw = w.next().unwrap_or("").to_ascii_uppercase();
            let mut rest = w.next().unwrap_or("");
            if kw == "Q" || kw == "QUACK" {
                let mut w2 = rest.trim_start().splitn(2, char::is_whitespace);
                kw = w2.next().unwrap_or("").to_ascii_uppercase();
                rest = w2.next().unwrap_or("");
            }
            let literal: Option<String> = match kw.as_str() {
                "STRING" | "STRINGLN" => Some(rest.to_string()),
                // command-only lines carry no typed secret
                "REM" | "#" | "ENTER" | "CR" | "LF" | "CRLF" | "TAB" | "ESC" | "SPACE" | "DELAY"
                | "WAIT" | "CTRL" | "CONTROL" | "HEX" | "REPEAT" | "TIMEOUT" | "WAITFOR"
                | "EXPECT" | "RUN" | "SMARTWAIT" | "DO" | "WAITOK" | "IF" | "ELSE" | "END"
                | "ENDIF" | "FI" => None,
                _ => Some(line.trim().to_string()), // bare line
            };
            if let Some(lit) = literal {
                let processed =
                    String::from_utf8_lossy(&process_escapes(&lit)).trim().to_string();
                if processed.len() >= 3 {
                    out.push(processed);
                }
            }
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.len())); // redact longest matches first
    out.dedup();
    out
}

/// Name-only list (for the LLM, never includes text).
pub fn macro_metas(shared: &Arc<Shared>) -> Vec<MacroMeta> {
    shared
        .macros
        .lock()
        .unwrap()
        .iter()
        .map(|s| MacroMeta { name: s.name.clone(), secret: s.secret })
        .collect()
}

/// Insert or replace a macro by name.
pub fn macro_upsert(shared: &Arc<Shared>, rec: MacroRec) {
    {
        let mut list = shared.macros.lock().unwrap();
        if let Some(existing) = list.iter_mut().find(|s| s.name == rec.name) {
            *existing = rec;
        } else {
            list.push(rec);
        }
    }
    persist(shared);
}

pub fn macro_delete(shared: &Arc<Shared>, name: &str) {
    shared.macros.lock().unwrap().retain(|s| s.name != name);
    persist(shared);
}

/// Replace the whole store (used to sync from the UI in one shot).
pub fn macros_set(shared: &Arc<Shared>, list: Vec<MacroRec>) {
    *shared.macros.lock().unwrap() = list;
    persist(shared);
}

/// Merge imported macros into the store (upsert by name). Returns the count.
pub fn macros_import(shared: &Arc<Shared>, recs: Vec<MacroRec>) -> usize {
    let n = recs.len();
    {
        let mut list = shared.macros.lock().unwrap();
        for rec in recs {
            if let Some(existing) = list.iter_mut().find(|s| s.name == rec.name) {
                *existing = rec;
            } else {
                list.push(rec);
            }
        }
    }
    persist(shared);
    n
}

/// Run a macro by name through the macro player. Never returns the text.
pub fn run_macro(shared: &Arc<Shared>, name: &str) -> Result<(), String> {
    let text = shared
        .macros
        .lock()
        .unwrap()
        .iter()
        .find(|s| s.name == name)
        .map(|s| s.text.clone());
    match text {
        Some(t) => play(shared, name, &t),
        None => Err(format!("no macro named '{name}'")),
    }
}

// ---- macro macro player (Bash Bunny / DuckyScript + expect) ---------------
//
// One command per line (case-insensitive). A line with no command keyword is
// typed verbatim + Enter. Commands:
//   REM / #               comment
//   STRING / STRINGLN     type text (no newline / + Enter)
//   ENTER CR LF CRLF TAB ESC SPACE
//   DELAY/WAIT <ms>       pause
//   CTRL <c>              control byte (CTRL c -> 0x03)
//   HEX <hh hh ..>        raw bytes
//   REPEAT <n>            repeat previous line n times
//   Q/QUACK <cmd>         Bash Bunny prefix
//   TIMEOUT <ms>          wait timeout for WAITFOR/RUN (default 10000)
//   WAITFOR/EXPECT <text> block until text appears on the console
//   RUN/SMARTWAIT/DO <cmd>  run cmd, wait for completion, capture exit code
//   WAITOK                abort if the last RUN's exit code != 0
//   IF OK | IF FAIL ... [ELSE] ... END   branch on last RUN exit code
// Text honors escapes: \n \r \t \0 \xHH \\.

const MAX_WAIT_MS: u64 = 600_000;

#[derive(Clone)]
enum Step {
    Bytes(Vec<u8>),
    Delay(u64),
    Timeout(u64),
    WaitFor(String),
    Run(String),
    WaitOk,
    If(bool), // true = IF OK, false = IF FAIL
    Else,
    End,
    Call(String),         // $Name: run another macro inline
    SetOut(String, u16),       // SET <name|index> <0|1|duty>: 0/1 = digital, 2..1023 = PWM duty
    SetRgb(String, u8, u8, u8), // RGB <name|index> <#RRGGBB>: fill an addressable output
    WaitIo(String, Cmp, i64), // WAITIO <name> <op> <value>: wait on an input
}

#[derive(Clone, Copy)]
enum Cmp {
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
}

fn parse_cmp(s: &str) -> Option<Cmp> {
    match s {
        ">" => Some(Cmp::Gt),
        "<" => Some(Cmp::Lt),
        ">=" => Some(Cmp::Ge),
        "<=" => Some(Cmp::Le),
        "==" | "=" => Some(Cmp::Eq),
        "!=" | "<>" => Some(Cmp::Ne),
        _ => None,
    }
}

fn cmp_ok(v: i64, cmp: Cmp, t: i64) -> bool {
    match cmp {
        Cmp::Gt => v > t,
        Cmp::Lt => v < t,
        Cmp::Ge => v >= t,
        Cmp::Le => v <= t,
        Cmp::Eq => v == t,
        Cmp::Ne => v != t,
    }
}

fn process_escapes(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push(b'\n'),
                Some('r') => out.push(b'\r'),
                Some('t') => out.push(b'\t'),
                Some('0') => out.push(0),
                Some('\\') => out.push(b'\\'),
                Some('x') => {
                    let h: String = (0..2).filter_map(|_| chars.next()).collect();
                    if let Ok(b) = u8::from_str_radix(&h, 16) {
                        out.push(b);
                    }
                }
                Some(other) => {
                    out.push(b'\\');
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
                }
                None => out.push(b'\\'),
            }
        } else {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    out
}

/// Parse a known command keyword + its argument. None ⇒ not a command.
fn parse_command(kw: &str, rest: &str) -> Option<Vec<Step>> {
    let bytes = |b: Vec<u8>| Some(vec![Step::Bytes(b)]);
    match kw {
        "REM" | "#" => Some(vec![]),
        "STRING" => bytes(process_escapes(rest)),
        "STRINGLN" => {
            let mut b = process_escapes(rest);
            b.push(b'\r');
            bytes(b)
        }
        "ENTER" | "CR" => bytes(vec![b'\r']),
        "LF" => bytes(vec![b'\n']),
        "CRLF" => bytes(vec![b'\r', b'\n']),
        "TAB" => bytes(vec![b'\t']),
        "ESC" | "ESCAPE" => bytes(vec![0x1b]),
        "SPACE" => bytes(vec![b' ']),
        "DELAY" | "WAIT" => Some(rest.trim().parse::<u64>().ok().map(Step::Delay).into_iter().collect()),
        "CTRL" | "CONTROL" => Some(
            rest.trim()
                .chars()
                .next()
                .map(|c| Step::Bytes(vec![(c.to_ascii_uppercase() as u8) & 0x1f]))
                .into_iter()
                .collect(),
        ),
        "HEX" => bytes(rest.split_whitespace().filter_map(|h| u8::from_str_radix(h, 16).ok()).collect()),
        _ => None,
    }
}

/// skrit-mc tier of one step. Control-flow ops (`WaitOk`/`If`/`Else`/`End`) are
/// transparent (they ride whatever read set the outcome), so they cost tier 1;
/// the read op (`WaitFor`/`WaitIo` = 2) or `Run` (= 3) is what dominates.
fn step_tier(step: &Step) -> u8 {
    match step {
        Step::WaitFor(_) | Step::WaitIo(..) => 2,
        Step::Run(_) => 3,
        _ => 1, // Bytes, Delay, Timeout, SetOut, WaitOk, If/Else/End, Call
    }
}

/// Highest tier a macro's text needs, inlining `$call` against `list`
/// (cycle-/depth-guarded; an over-deep chain is treated as app-only).
fn tier_of_text(list: &[MacroRec], text: &str, depth: u32) -> u8 {
    if depth > MAX_CALL_DEPTH {
        return 3;
    }
    let mut t = 1u8;
    for step in parse_macro(text) {
        let st = match &step {
            Step::Call(name) => {
                let want = name.trim().to_lowercase();
                list.iter()
                    .find(|m| m.name.trim().to_lowercase() == want)
                    .map(|m| tier_of_text(list, &m.text, depth + 1))
                    .unwrap_or(1)
            }
            s => step_tier(s),
        };
        if st > t {
            t = st;
        }
    }
    t
}

/// Clone `list` with each record's `tier` freshly computed (resolves `$call`
/// across the whole list, so an edit to a callee re-tiers its callers).
fn tiered(list: &[MacroRec]) -> Vec<MacroRec> {
    list.iter()
        .cloned()
        .map(|mut m| {
            m.tier = tier_of_text(list, &m.text, 0);
            m
        })
        .collect()
}

fn parse_macro(s: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut prev: Vec<Step> = Vec::new(); // for REPEAT
    for raw in s.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start();
        // $Name: call another macro inline
        if let Some(name) = trimmed.strip_prefix('$') {
            let st = vec![Step::Call(name.trim().to_string())];
            prev = st.clone();
            steps.extend(st);
            continue;
        }
        let mut w = trimmed.splitn(2, char::is_whitespace);
        let mut kw = w.next().unwrap_or("").to_ascii_uppercase();
        let mut rest = w.next().unwrap_or("");
        // Bash Bunny Q/QUACK prefix: unwrap to the inner command
        if kw == "Q" || kw == "QUACK" {
            let mut w2 = rest.trim_start().splitn(2, char::is_whitespace);
            kw = w2.next().unwrap_or("").to_ascii_uppercase();
            rest = w2.next().unwrap_or("");
        }
        match kw.as_str() {
            "REPEAT" => {
                let n = rest.trim().parse::<usize>().unwrap_or(0);
                for _ in 0..n {
                    steps.extend(prev.iter().cloned());
                }
                continue;
            }
            "REM" | "#" => continue,
            _ => {}
        }
        let line_steps: Vec<Step> = match kw.as_str() {
            "TIMEOUT" => vec![Step::Timeout(rest.trim().parse().unwrap_or(10_000))],
            "WAITFOR" | "EXPECT" => vec![Step::WaitFor(rest.to_string())],
            "RUN" | "SMARTWAIT" | "DO" => vec![Step::Run(rest.to_string())],
            "WAITOK" => vec![Step::WaitOk],
            "SET" => match rest.trim().rsplit_once(char::is_whitespace) {
                // last token = value, everything before = target name (allows spaces).
                // 0/off and 1/on are digital; a number 2..1023 is a PWM duty.
                Some((name, val)) => {
                    let v = match val.trim().to_ascii_lowercase().as_str() {
                        "1" | "on" | "true" | "high" => 1u16,
                        "0" | "off" | "false" | "low" => 0u16,
                        n => n.parse::<u16>().unwrap_or(0).min(1023),
                    };
                    vec![Step::SetOut(name.trim().to_string(), v)]
                }
                None => vec![],
            },
            "RGB" => match rest.trim().rsplit_once(char::is_whitespace) {
                // last token = #RRGGBB (hash optional), before = output name
                Some((name, hex)) => {
                    let h = hex.trim().trim_start_matches('#');
                    match (h.len() == 6, u32::from_str_radix(h, 16)) {
                        (true, Ok(v)) => vec![Step::SetRgb(
                            name.trim().to_string(),
                            (v >> 16) as u8,
                            (v >> 8) as u8,
                            v as u8,
                        )],
                        _ => vec![],
                    }
                }
                None => vec![],
            },
            "WAITIO" => {
                let p: Vec<&str> = rest.split_whitespace().collect();
                match (p.first(), p.get(1).and_then(|o| parse_cmp(o)), p.get(2)) {
                    (Some(name), Some(cmp), Some(val)) => {
                        vec![Step::WaitIo(name.to_string(), cmp, val.parse::<i64>().unwrap_or(0))]
                    }
                    _ => vec![],
                }
            }
            "IF" => vec![Step::If(!rest.trim().eq_ignore_ascii_case("fail"))],
            "ELSE" => vec![Step::Else],
            "END" | "ENDIF" | "FI" => vec![Step::End],
            _ => match parse_command(&kw, rest) {
                Some(v) => v,
                // bare line: type it verbatim (preserving indentation) + Enter
                None => {
                    let mut b = process_escapes(line);
                    b.push(b'\r');
                    vec![Step::Bytes(b)]
                }
            },
        };
        if !matches!(kw.as_str(), "IF" | "ELSE" | "END" | "ENDIF" | "FI") {
            prev = line_steps.clone();
        }
        steps.extend(line_steps);
    }
    steps
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn next_marker_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(1);
    C.fetch_add(1, Ordering::Relaxed)
}

// ---- macro run registry (in-flight macros; cancellable) --------------------
struct MacroRun {
    id: u64,
    name: String,
    status: String,
    cancel: Arc<AtomicBool>,
}

#[derive(Serialize, Clone)]
pub struct MacroRunInfo {
    pub id: u64,
    pub name: String,
    pub status: String,
}

/// Per-run context threaded through the player so steps can report status and
/// honor cancellation.
struct MacroCtx {
    shared: Arc<Shared>,
    app: Option<AppHandle>,
    id: u64,
    cancel: Arc<AtomicBool>,
}

fn next_run_id() -> u64 {
    use std::sync::atomic::AtomicU64;
    static C: AtomicU64 = AtomicU64::new(1);
    C.fetch_add(1, Ordering::Relaxed)
}

pub fn macro_runs(shared: &Arc<Shared>) -> Vec<MacroRunInfo> {
    shared
        .runs
        .lock()
        .unwrap()
        .iter()
        .map(|r| MacroRunInfo { id: r.id, name: r.name.clone(), status: r.status.clone() })
        .collect()
}

fn emit_runs(shared: &Arc<Shared>, app: &Option<AppHandle>) {
    if let Some(app) = app {
        let _ = app.emit("sutra://runs", macro_runs(shared));
    }
}

fn set_run_status(ctx: &MacroCtx, status: &str) {
    {
        let mut runs = ctx.shared.runs.lock().unwrap();
        if let Some(r) = runs.iter_mut().find(|r| r.id == ctx.id) {
            r.status = status.to_string();
        }
    }
    emit_runs(&ctx.shared, &ctx.app);
}

/// Request cancellation of a run by id (the player thread stops at the next check).
pub fn cancel_run(shared: &Arc<Shared>, id: u64) {
    if let Some(r) = shared.runs.lock().unwrap().iter().find(|r| r.id == id) {
        r.cancel.store(true, Ordering::Relaxed);
    }
}

/// Sleep that bails out promptly on cancellation.
fn cancellable_sleep(ctx: &MacroCtx, ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms.min(MAX_WAIT_MS));
    while Instant::now() < end {
        if ctx.cancel.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Block until `needle` appears in console output after `*since`. Returns false on timeout/cancel.
fn wait_for(ctx: &MacroCtx, needle: &[u8], timeout_ms: u64, since: &mut u64) -> bool {
    let shared = &ctx.shared;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms.min(MAX_WAIT_MS));
    let mut acc: Vec<u8> = Vec::new();
    while Instant::now() < deadline {
        if ctx.cancel.load(Ordering::Relaxed) {
            return false;
        }
        let (new, total) = console_since(shared, *since);
        *since = total;
        if !new.is_empty() {
            acc.extend_from_slice(&new);
            if find_sub(&acc, needle).is_some() {
                return true;
            }
            if acc.len() > 16384 {
                acc.drain(0..acc.len() - 16384);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Run a command and capture its exit code via a split-marker sentinel.
fn run_command(ctx: &MacroCtx, cmd: &str, timeout_ms: u64, since: &mut u64) -> Option<i64> {
    let shared = &ctx.shared;
    let id = next_marker_id();
    let needle = format!("sutra_{id}_:");
    // The "" splits the literal so the echoed command line never matches `needle`.
    let line = format!("{cmd}; echo \"sut\"\"ra_{id}_:$?\"\r");
    let _ = data_write(shared, line.as_bytes());

    let deadline = Instant::now() + Duration::from_millis(timeout_ms.min(MAX_WAIT_MS));
    let nb = needle.as_bytes();
    let mut acc: Vec<u8> = Vec::new();
    while Instant::now() < deadline {
        if ctx.cancel.load(Ordering::Relaxed) {
            return None;
        }
        let (new, total) = console_since(shared, *since);
        *since = total;
        if !new.is_empty() {
            acc.extend_from_slice(&new);
            if let Some(pos) = find_sub(&acc, nb) {
                let after = &acc[pos + nb.len()..];
                if let Some(nl) = after.iter().position(|&b| b == b'\n' || b == b'\r') {
                    let num = String::from_utf8_lossy(&after[..nl]);
                    return Some(num.trim().parse::<i64>().unwrap_or(-1));
                }
            }
            if acc.len() > 16384 {
                acc.drain(0..acc.len() - 16384);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None // timeout
}

struct IfFrame {
    active: bool,
    taken: bool,
    parent: bool,
}

const MAX_CALL_DEPTH: u32 = 8;

fn macro_text_by_name(shared: &Arc<Shared>, name: &str) -> Option<String> {
    let want = name.trim().to_lowercase();
    shared
        .macros
        .lock()
        .unwrap()
        .iter()
        .find(|s| s.name.trim().to_lowercase() == want)
        .map(|s| s.text.clone())
}

/// index -> name for the device's outputs (via INFO + OUTPUT_DESC).
fn load_output_names(shared: &Arc<Shared>) -> Vec<String> {
    use crate::protocol::msg;
    let n = match send_cmd(shared, msg::INFO, vec![]) {
        Ok(r) => *r.body.get(4).unwrap_or(&0), // n_outputs
        Err(_) => return Vec::new(),
    };
    (0..n)
        .map(|i| match send_cmd(shared, msg::OUTPUT_DESC, vec![i]) {
            Ok(r) => String::from_utf8_lossy(r.body.get(3..).unwrap_or(&[])).into_owned(),
            Err(_) => String::new(),
        })
        .collect()
}

fn norm_name(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).flat_map(|c| c.to_lowercase()).collect()
}

/// index -> name for the device's inputs (via INFO n_inputs + INPUT_DESC).
fn load_input_names(shared: &Arc<Shared>) -> Vec<String> {
    use crate::protocol::msg;
    let n = match send_cmd(shared, msg::INFO, vec![]) {
        Ok(r) => *r.body.get(7).unwrap_or(&0), // n_inputs
        Err(_) => return Vec::new(),
    };
    (0..n)
        .map(|i| match send_cmd(shared, msg::INPUT_DESC, vec![i]) {
            Ok(r) => String::from_utf8_lossy(r.body.get(3..).unwrap_or(&[])).into_owned(),
            Err(_) => String::new(),
        })
        .collect()
}

/// Read one input's current value (digital 0/1, analog 0-1023).
fn read_input(shared: &Arc<Shared>, idx: u8) -> Option<u16> {
    use crate::protocol::msg;
    match send_cmd(shared, msg::INPUT_GET, vec![idx]) {
        Ok(r) if r.body.first() == Some(&0) => {
            let lo = *r.body.get(2).unwrap_or(&0) as u16;
            let hi = *r.body.get(3).unwrap_or(&0) as u16;
            Some((hi << 8) | lo)
        }
        _ => None,
    }
}

/// Poll a named input until it satisfies the comparison, or timeout. Returns
/// false on timeout or an unknown input (caller aborts the macro).
fn wait_io(
    ctx: &MacroCtx,
    name: &str,
    cmp: Cmp,
    threshold: i64,
    timeout_ms: u64,
    in_names: &mut Vec<String>,
    in_loaded: &mut bool,
) -> bool {
    let shared = &ctx.shared;
    let idx = if let Ok(i) = name.parse::<u8>() {
        Some(i)
    } else {
        if !*in_loaded {
            *in_names = load_input_names(shared);
            *in_loaded = true;
        }
        let want = norm_name(name);
        in_names.iter().position(|n| norm_name(n) == want).map(|p| p as u8)
    };
    let idx = match idx {
        Some(i) => i,
        None => return false, // unknown input: can't satisfy
    };
    let deadline = Instant::now() + Duration::from_millis(timeout_ms.min(MAX_WAIT_MS));
    while Instant::now() < deadline {
        if ctx.cancel.load(Ordering::Relaxed) {
            return false;
        }
        if let Some(v) = read_input(shared, idx) {
            if cmp_ok(v as i64, cmp, threshold) {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn run_steps(ctx: &MacroCtx, steps: &[Step], depth: u32) {
    let shared = &ctx.shared;
    let mut stack: Vec<IfFrame> = Vec::new();
    let active = |st: &[IfFrame]| st.last().is_none_or(|f| f.active);
    let mut last_exit: Option<i64> = None;
    let mut timeout: u64 = 10_000;
    let mut since: u64 = console_seq(shared);
    let mut out_names: Vec<String> = Vec::new();
    let mut out_loaded = false;
    let mut in_names: Vec<String> = Vec::new();
    let mut in_loaded = false;

    for step in steps {
        if ctx.cancel.load(Ordering::Relaxed) {
            return; // cancelled
        }
        // control flow is processed regardless of the active state (to track nesting)
        match step {
            Step::If(want_ok) => {
                let parent = active(&stack);
                let cond = (last_exit == Some(0)) == *want_ok;
                let a = parent && cond;
                stack.push(IfFrame { active: a, taken: a, parent });
                continue;
            }
            Step::Else => {
                if let Some(f) = stack.last_mut() {
                    f.active = f.parent && !f.taken;
                    f.taken = true;
                }
                continue;
            }
            Step::End => {
                stack.pop();
                continue;
            }
            _ => {}
        }
        if !active(&stack) {
            continue;
        }
        match step {
            Step::Bytes(b) => {
                let _ = data_write(shared, b);
            }
            Step::Delay(ms) => {
                set_run_status(ctx, &format!("delay {ms}ms"));
                cancellable_sleep(ctx, *ms);
            }
            Step::Timeout(ms) => timeout = *ms,
            Step::WaitFor(t) => {
                set_run_status(ctx, &format!("waiting for: {t}"));
                if !wait_for(ctx, t.as_bytes(), timeout, &mut since) {
                    break; // timeout/cancel aborts the macro
                }
            }
            Step::Run(cmd) => {
                set_run_status(ctx, &format!("running: {cmd}"));
                last_exit = run_command(ctx, cmd, timeout, &mut since);
            }
            Step::WaitOk => {
                if last_exit != Some(0) {
                    break;
                }
            }
            Step::Call(name) => {
                set_run_status(ctx, &format!("call: {name}"));
                if depth < MAX_CALL_DEPTH {
                    if let Some(text) = macro_text_by_name(shared, name) {
                        let sub = parse_macro(&text);
                        run_steps(ctx, &sub, depth + 1);
                    }
                }
            }
            Step::SetOut(target, val) => {
                let idx = if let Ok(i) = target.parse::<u8>() {
                    Some(i)
                } else {
                    if !out_loaded {
                        out_names = load_output_names(shared);
                        out_loaded = true;
                    }
                    let want = norm_name(target);
                    out_names.iter().position(|nm| norm_name(nm) == want).map(|p| p as u8)
                };
                if let Some(i) = idx {
                    if *val > 1 {
                        // PWM duty (the device rescales 0..1023 to its resolution)
                        let _ = send_cmd(
                            shared,
                            crate::protocol::msg::OUTPUT_PWM,
                            vec![i, (*val & 0xFF) as u8, (*val >> 8) as u8],
                        );
                    } else {
                        let _ =
                            send_cmd(shared, crate::protocol::msg::OUTPUT_SET, vec![i, *val as u8]);
                    }
                }
            }
            Step::SetRgb(target, r, g, b) => {
                let idx = if let Ok(i) = target.parse::<u8>() {
                    Some(i)
                } else {
                    if !out_loaded {
                        out_names = load_output_names(shared);
                        out_loaded = true;
                    }
                    let want = norm_name(target);
                    out_names.iter().position(|nm| norm_name(nm) == want).map(|p| p as u8)
                };
                if let Some(i) = idx {
                    // 4-byte body = fill the whole strip
                    let _ = send_cmd(shared, crate::protocol::msg::OUTPUT_RGB, vec![i, *r, *g, *b]);
                }
            }
            Step::WaitIo(name, cmp, threshold) => {
                set_run_status(ctx, &format!("waiting: {name}"));
                if !wait_io(ctx, name, *cmp, *threshold, timeout, &mut in_names, &mut in_loaded) {
                    break; // timeout/cancel/unknown input aborts the macro
                }
            }
            Step::If(_) | Step::Else | Step::End => {}
        }
    }
}

/// Execute a macro against the DATA port on a background thread, tracked as a
/// cancellable run in the registry.
// Sutra's host-side injector identity, assigned to a network on first use. The
// short address is high (unlikely to collide with an assigned node); the EUI-64
// is locally-administered (bit 1 of the first octet set) and spells "SUTRA".
// Coordinator-safety: these must differ from every real node on the network.
const DEFAULT_INJECT_SRC: u16 = 0x7fff;
const DEFAULT_INJECT_EUI: &str = "0253555452410001"; // 02:53('S')55('U')54('T')52('R')41('A')00 01

fn parse_fixed_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let t = s.trim().trim_start_matches("0x");
    if t.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn parse_u16_prefixed(s: &str) -> u16 {
    u16::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap_or(0)
}

/// Resolve every `{$…}` in `text` against the active network, persisting the
/// advanced frame counter + assigned injector identity. A macro with no `{$`
/// is returned unchanged and touches no workspace state.
fn resolve_macro_text(shared: &Arc<Shared>, text: &str) -> Result<String, String> {
    use crate::macrovars::{resolve_text, VarContext};
    // fast path: nothing to resolve and no VAR directive → no workspace I/O.
    let has_var = text
        .lines()
        .any(|l| l.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("VAR")));
    if !text.contains("{$") && !has_var {
        return Ok(text.to_string());
    }
    let app = shared
        .app
        .lock()
        .unwrap()
        .clone()
        .ok_or("macro variables need a workspace (no app handle)")?;
    let mut nets = crate::workspace::load_networks(&app);
    let idx = crate::workspace::active_network_index(&nets);

    let mut ctx = if let Some(i) = idx {
        let n = &mut nets.networks[i];
        // assign a stable injector identity on first use (coordinator-safety).
        if n.inject_src == 0 {
            n.inject_src = DEFAULT_INJECT_SRC;
        }
        if n.inject_eui.trim().is_empty() {
            n.inject_eui = DEFAULT_INJECT_EUI.to_string();
        }
        VarContext {
            key: parse_fixed_hex::<16>(&n.key),
            pan: parse_u16_prefixed(&n.pan),
            channel: n.channel,
            src_short: n.inject_src,
            src_eui64: parse_fixed_hex::<8>(&n.inject_eui).unwrap_or_default(),
            frame_counter: n.frame_counter,
            seq: 0,
            vars: Default::default(),
        }
    } else {
        VarContext::default() // no network → network vars error helpfully
    };

    let resolved = resolve_text(&mut ctx, text)?;

    // Persist the advanced counter + assigned identity for the next run.
    if let Some(i) = idx {
        nets.networks[i].frame_counter = ctx.frame_counter;
        let _ = crate::workspace::save_networks(&app, &nets);
    }
    Ok(resolved)
}

pub fn play(shared: &Arc<Shared>, name: &str, text: &str) -> Result<(), String> {
    let text = resolve_macro_text(shared, text)?; // {$…} → resolved (counter consumed once)
    let steps = parse_macro(&text);
    let shared = shared.clone();
    let app = shared.app.lock().unwrap().clone();
    let id = next_run_id();
    let cancel = Arc::new(AtomicBool::new(false));
    shared.runs.lock().unwrap().push(MacroRun {
        id,
        name: name.to_string(),
        status: "running".into(),
        cancel: cancel.clone(),
    });
    emit_runs(&shared, &app);
    std::thread::spawn(move || {
        let ctx = MacroCtx { shared: shared.clone(), app: app.clone(), id, cancel };
        run_steps(&ctx, &steps, 0);
        shared.runs.lock().unwrap().retain(|r| r.id != id);
        emit_runs(&shared, &app);
    });
    Ok(())
}

#[cfg(test)]
mod hw_tests {
    // Hardware-in-the-loop smoke tests — need a real Duta plugged in, so they
    // are #[ignore]d for CI. Run locally with:  cargo test -- --ignored hw_
    use super::*;

    #[test]
    #[ignore]
    fn hw_autodetect_mux_finds_a_duta() {
        let ports = list_ports();
        let cands: Vec<_> = ports.iter().filter(|p| p.is_duta).collect();
        println!("candidates: {cands:?}");
        let port = autodetect_mux().expect("a muxed Duta should answer the probe");
        println!("muxed Duta on {port}");
    }
}
