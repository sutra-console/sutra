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

use crate::protocol::{msg, Frame, FrameReader};

pub const DUTA_VID: u16 = 0x1209;
pub const DUTA_PID: u16 = 0xC550;
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
    cmd: Option<Box<dyn SerialPort>>, // None for a generic (non-Duta) port
    data_writer: Box<dyn SerialPort>,
    stop: Arc<AtomicBool>,
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
    macros_path: Mutex<Option<std::path::PathBuf>>,
    app: Mutex<Option<AppHandle>>,
    mcp_tools: Mutex<McpToolFlags>,
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
            macros_path: Mutex::new(None),
            app: Mutex::new(None),
            mcp_tools: Mutex::new(McpToolFlags::default()),
        }
    }
}

impl Shared {
    fn next_seq(&self) -> u8 {
        let mut s = self.seq.lock().unwrap();
        *s = s.wrapping_add(1);
        *s
    }

    fn push_console(&self, bytes: &[u8]) {
        self.console.lock().unwrap().push(bytes);
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
            let is_duta = vid == Some(DUTA_VID) && pid == Some(DUTA_PID);
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
                for r in reader.push(&buf[..n]) {
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
    let ports: Vec<String> =
        list_ports().into_iter().filter(|p| p.is_duta).map(|p| p.name).collect();
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

// ---- connection ------------------------------------------------------------

fn spawn_data_reader(
    app: AppHandle,
    mut port: Box<dyn SerialPort>,
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        let mut online = true;
        while !stop.load(Ordering::Relaxed) {
            match port.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => {
                    if !online {
                        online = true;
                        let _ = app.emit("ttl://link", true); // target came back
                    }
                    shared.push_console(&buf[..n]);
                    let _ = app.emit("ttl://data", buf[..n].to_vec());
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => {
                    // The port dropped (unplug / device reset). Keep the
                    // connection open, report the link offline, and try to reopen
                    // so it comes back online on its own.
                    if online {
                        online = false;
                        let _ = app.emit("ttl://link", false);
                    }
                    std::thread::sleep(Duration::from_millis(750));
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
                                // still our connection — swap in the fresh handles
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
    });
}

/// Connect a DATA port. `cmd_name` is the Duta CMD interface, or None for a
/// generic serial port (console only — no relay/LED/INFO).
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
    spawn_data_reader(app, reader, stop.clone(), shared.clone());

    *shared.data_name.lock().unwrap() = Some(data_name.to_string());
    *shared.cmd_name.lock().unwrap() = cmd_name.map(|s| s.to_string());
    *shared.conn.lock().unwrap() = Some(Connection { cmd, data_writer: data, stop });
    Ok(())
}

/// Re-open just the DATA port with the current serial params (keeps CMD).
pub fn reconnect_data(shared: &Arc<Shared>, app: AppHandle) -> Result<(), String> {
    let data_name = shared.data_name.lock().unwrap().clone().ok_or("not connected")?;
    let params = shared.params.lock().unwrap().clone();
    let mut data = open_data(&data_name, &params)?;
    let _ = data.write_data_terminal_ready(true);
    let reader = data.try_clone().map_err(|e| format!("clone data: {e}"))?;
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut guard = shared.conn.lock().unwrap();
        let conn = guard.as_mut().ok_or("not connected")?;
        conn.stop.store(true, Ordering::Relaxed); // stop old reader
        conn.data_writer = data;
        conn.stop = stop.clone();
    }
    spawn_data_reader(app, reader, stop, shared.clone());
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
    if let Some(conn) = shared.conn.lock().unwrap().take() {
        conn.stop.store(true, Ordering::Relaxed);
        drop(conn.cmd);
        drop(conn.data_writer);
    }
}

pub fn state(shared: &Arc<Shared>) -> ConnState {
    let has_cmd = shared.conn.lock().unwrap().as_ref().map_or(false, |c| c.cmd.is_some());
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

/// connect/set_params variants the MCP server can call — they pull the AppHandle
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
    let mut guard = shared.conn.lock().unwrap();
    let conn = guard.as_mut().ok_or("not connected")?;
    conn.data_writer.write_all(bytes).map_err(|e| format!("data write: {e}"))?;
    conn.data_writer.flush().ok();
    Ok(())
}

pub fn send_cmd(shared: &Arc<Shared>, typ: u8, body: Vec<u8>) -> Result<RespFrame, String> {
    let seq = shared.next_seq();
    let wire = Frame::new(typ, seq, body).to_wire().map_err(|e| format!("encode: {e:?}"))?;
    let mut guard = shared.conn.lock().unwrap();
    let conn = guard.as_mut().ok_or("not connected")?;
    let cmd = conn
        .cmd
        .as_mut()
        .ok_or("no command port — connect a Duta for relay/LED/INFO")?;
    cmd.write_all(&wire).map_err(|e| format!("cmd write: {e}"))?;
    cmd.flush().ok();
    let resp = read_response(cmd, 1000)?;
    Ok(resp.into())
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
    // notify the UI so LLM-created/changed macros appear live
    if let Some(app) = shared.app.lock().unwrap().clone() {
        let _ = app.emit("ttl://macros", &list);
    }
}

/// Full macro list (for the app UI — includes text).
pub fn macros_all(shared: &Arc<Shared>) -> Vec<MacroRec> {
    shared.macros.lock().unwrap().clone()
}

/// Literal strings typed by SECRET macros (bare lines + STRING args, escapes
/// applied) — the bytes that could echo back. Used to redact MCP console reads.
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
    out.sort_by(|a, b| b.len().cmp(&a.len())); // redact longest matches first
    out.dedup();
    out
}

/// Name-only list (for the LLM — never includes text).
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
        Some(t) => {
            play(shared, &t);
            Ok(())
        }
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
    Call(String),         // $Name — run another macro inline
    SetOut(String, bool), // SET <name|index> <0|1> — drive an output over CMD
    WaitIo(String, Cmp, i64), // WAITIO <name> <op> <value> — wait on an input
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

fn parse_macro(s: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut prev: Vec<Step> = Vec::new(); // for REPEAT
    for raw in s.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start();
        // $Name — call another macro inline
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
                // last token = value, everything before = target name (allows spaces)
                Some((name, val)) => vec![Step::SetOut(
                    name.trim().to_string(),
                    matches!(val.trim().to_ascii_lowercase().as_str(), "1" | "on" | "true" | "high"),
                )],
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

/// Block until `needle` appears in console output after `*since`. Returns false on timeout.
fn wait_for(shared: &Arc<Shared>, needle: &[u8], timeout_ms: u64, since: &mut u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms.min(MAX_WAIT_MS));
    let mut acc: Vec<u8> = Vec::new();
    while Instant::now() < deadline {
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
fn run_command(shared: &Arc<Shared>, cmd: &str, timeout_ms: u64, since: &mut u64) -> Option<i64> {
    let id = next_marker_id();
    let needle = format!("sutra_{id}_:");
    // The "" splits the literal so the echoed command line never matches `needle`.
    let line = format!("{cmd}; echo \"sut\"\"ra_{id}_:$?\"\r");
    let _ = data_write(shared, line.as_bytes());

    let deadline = Instant::now() + Duration::from_millis(timeout_ms.min(MAX_WAIT_MS));
    let nb = needle.as_bytes();
    let mut acc: Vec<u8> = Vec::new();
    while Instant::now() < deadline {
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
    shared: &Arc<Shared>,
    name: &str,
    cmp: Cmp,
    threshold: i64,
    timeout_ms: u64,
    in_names: &mut Vec<String>,
    in_loaded: &mut bool,
) -> bool {
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
        None => return false, // unknown input — can't satisfy
    };
    let deadline = Instant::now() + Duration::from_millis(timeout_ms.min(MAX_WAIT_MS));
    while Instant::now() < deadline {
        if let Some(v) = read_input(shared, idx) {
            if cmp_ok(v as i64, cmp, threshold) {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn run_steps(shared: &Arc<Shared>, steps: &[Step], depth: u32) {
    let mut stack: Vec<IfFrame> = Vec::new();
    let active = |st: &[IfFrame]| st.last().map_or(true, |f| f.active);
    let mut last_exit: Option<i64> = None;
    let mut timeout: u64 = 10_000;
    let mut since: u64 = console_seq(shared);
    let mut out_names: Vec<String> = Vec::new();
    let mut out_loaded = false;
    let mut in_names: Vec<String> = Vec::new();
    let mut in_loaded = false;

    for step in steps {
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
            Step::Delay(ms) => std::thread::sleep(Duration::from_millis((*ms).min(MAX_WAIT_MS))),
            Step::Timeout(ms) => timeout = *ms,
            Step::WaitFor(t) => {
                if !wait_for(shared, t.as_bytes(), timeout, &mut since) {
                    break; // timeout aborts the macro
                }
            }
            Step::Run(cmd) => {
                last_exit = run_command(shared, cmd, timeout, &mut since);
            }
            Step::WaitOk => {
                if last_exit != Some(0) {
                    break;
                }
            }
            Step::Call(name) => {
                if depth < MAX_CALL_DEPTH {
                    if let Some(text) = macro_text_by_name(shared, name) {
                        let sub = parse_macro(&text);
                        run_steps(shared, &sub, depth + 1);
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
                    let _ = send_cmd(shared, crate::protocol::msg::OUTPUT_SET, vec![i, *val as u8]);
                }
            }
            Step::WaitIo(name, cmp, threshold) => {
                if !wait_io(shared, name, *cmp, *threshold, timeout, &mut in_names, &mut in_loaded) {
                    break; // timeout or unknown input aborts the macro
                }
            }
            Step::If(_) | Step::Else | Step::End => {}
        }
    }
}

/// Execute a macro macro against the DATA port on a background thread.
pub fn play(shared: &Arc<Shared>, text: &str) {
    let steps = parse_macro(text);
    let shared = shared.clone();
    std::thread::spawn(move || run_steps(&shared, &steps, 0));
}
