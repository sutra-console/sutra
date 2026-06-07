//! Device discovery + connection management for sutra.
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

pub const SUTRA_VID: u16 = 0x1209;
pub const SUTRA_PID: u16 = 0xC550;
const CONSOLE_CAP: usize = 64 * 1024; // rolling DATA-console buffer for the UI/MCP

#[derive(Debug, Clone, Serialize)]
pub struct PortDesc {
    pub name: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub product: Option<String>,
    pub manufacturer: Option<String>,
    pub serial_number: Option<String>,
    pub is_sutra: bool,
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
    pub has_cmd: bool, // true only with a sutra (relays/LED/INFO available)
    pub params: SerialParams,
}

/// A snippet as mirrored from the app. The MCP server holds this so it can
/// *run* snippets by name without ever exposing `text` (secrets stay hidden).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnippetRec {
    pub name: String,
    pub text: String,
    #[serde(default)]
    pub secret: bool,
}

/// Name-only view returned to the LLM (no `text`).
#[derive(Debug, Clone, Serialize)]
pub struct SnippetMeta {
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
    pub snippets_run: bool,
    pub snippets_create: bool,
    pub connection: bool,
}

impl Default for McpToolFlags {
    fn default() -> Self {
        McpToolFlags {
            console_read: true,
            console_write: true,
            outputs: true,
            snippets_run: true,
            snippets_create: true,
            connection: true,
        }
    }
}

struct Connection {
    cmd: Option<Box<dyn SerialPort>>, // None for a generic (non-sutra) port
    data_writer: Box<dyn SerialPort>,
    stop: Arc<AtomicBool>,
}

pub struct Shared {
    conn: Mutex<Option<Connection>>,
    seq: Mutex<u8>,
    console: Mutex<VecDeque<u8>>,
    params: Mutex<SerialParams>,
    data_name: Mutex<Option<String>>,
    cmd_name: Mutex<Option<String>>,
    snippets: Mutex<Vec<SnippetRec>>,
    snippets_path: Mutex<Option<std::path::PathBuf>>,
    app: Mutex<Option<AppHandle>>,
    mcp_tools: Mutex<McpToolFlags>,
}

impl Default for Shared {
    fn default() -> Self {
        Shared {
            conn: Mutex::new(None),
            seq: Mutex::new(0),
            console: Mutex::new(VecDeque::with_capacity(CONSOLE_CAP)),
            params: Mutex::new(SerialParams::default()),
            data_name: Mutex::new(None),
            cmd_name: Mutex::new(None),
            snippets: Mutex::new(Vec::new()),
            snippets_path: Mutex::new(None),
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
        let mut c = self.console.lock().unwrap();
        for &b in bytes {
            if c.len() >= CONSOLE_CAP {
                c.pop_front();
            }
            c.push_back(b);
        }
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
            let is_sutra = vid == Some(SUTRA_VID) && pid == Some(SUTRA_PID);
            PortDesc { name: p.port_name, vid, pid, product, manufacturer, serial_number, is_sutra }
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
        list_ports().into_iter().filter(|p| p.is_sutra).map(|p| p.name).collect();
    if ports.len() < 2 {
        return Err(format!("expected 2 sutra ports, found {}", ports.len()));
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
        while !stop.load(Ordering::Relaxed) {
            match port.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => {
                    shared.push_console(&buf[..n]);
                    let _ = app.emit("ttl://data", buf[..n].to_vec());
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => {
                    let _ = app.emit("ttl://data-error", ());
                    break;
                }
            }
        }
    });
}

/// Connect a DATA port. `cmd_name` is the sutra CMD interface, or None for a
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
        .ok_or("no command port — connect a sutra for relay/LED/INFO")?;
    cmd.write_all(&wire).map_err(|e| format!("cmd write: {e}"))?;
    cmd.flush().ok();
    let resp = read_response(cmd, 1000)?;
    Ok(resp.into())
}

/// Last `max` bytes of the DATA console as lossy UTF-8.
pub fn read_console(shared: &Arc<Shared>, max: usize) -> String {
    let c = shared.console.lock().unwrap();
    let n = c.len().min(max);
    let bytes: Vec<u8> = c.iter().skip(c.len() - n).copied().collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

// ---- snippet store (backend-owned, persisted, mirrored to UI + MCP) --------

/// Wire up the persistence path + app handle and load snippets from disk.
pub fn init_snippets(shared: &Arc<Shared>, app: AppHandle, path: std::path::PathBuf) {
    *shared.app.lock().unwrap() = Some(app);
    if let Ok(data) = std::fs::read(&path) {
        if let Ok(list) = serde_json::from_slice::<Vec<SnippetRec>>(&data) {
            *shared.snippets.lock().unwrap() = list;
        }
    }
    *shared.snippets_path.lock().unwrap() = Some(path);
}

fn persist(shared: &Arc<Shared>) {
    let list = shared.snippets.lock().unwrap().clone();
    if let Some(path) = shared.snippets_path.lock().unwrap().clone() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&list) {
            let _ = std::fs::write(&path, json);
        }
    }
    // notify the UI so LLM-created/changed snippets appear live
    if let Some(app) = shared.app.lock().unwrap().clone() {
        let _ = app.emit("ttl://snippets", &list);
    }
}

/// Full snippet list (for the app UI — includes text).
pub fn snippets_all(shared: &Arc<Shared>) -> Vec<SnippetRec> {
    shared.snippets.lock().unwrap().clone()
}

/// Name-only list (for the LLM — never includes text).
pub fn snippet_metas(shared: &Arc<Shared>) -> Vec<SnippetMeta> {
    shared
        .snippets
        .lock()
        .unwrap()
        .iter()
        .map(|s| SnippetMeta { name: s.name.clone(), secret: s.secret })
        .collect()
}

/// Insert or replace a snippet by name.
pub fn snippet_upsert(shared: &Arc<Shared>, rec: SnippetRec) {
    {
        let mut list = shared.snippets.lock().unwrap();
        if let Some(existing) = list.iter_mut().find(|s| s.name == rec.name) {
            *existing = rec;
        } else {
            list.push(rec);
        }
    }
    persist(shared);
}

pub fn snippet_delete(shared: &Arc<Shared>, name: &str) {
    shared.snippets.lock().unwrap().retain(|s| s.name != name);
    persist(shared);
}

/// Replace the whole store (used to sync from the UI in one shot).
pub fn snippets_set(shared: &Arc<Shared>, list: Vec<SnippetRec>) {
    *shared.snippets.lock().unwrap() = list;
    persist(shared);
}

/// Run a snippet by name through the macro player. Never returns the text.
pub fn run_snippet(shared: &Arc<Shared>, name: &str) -> Result<(), String> {
    let text = shared
        .snippets
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
        None => Err(format!("no snippet named '{name}'")),
    }
}

// ---- snippet macro player (Bash Bunny / DuckyScript style) ------------------
//
// One command per line (case-insensitive). A line with no command keyword is
// typed literally followed by Enter. Commands:
//   REM <text>            comment
//   STRING <text>         type text (no newline)
//   STRINGLN <text>       type text + Enter
//   ENTER / CR / LF / CRLF
//   TAB / ESC / SPACE
//   DELAY <ms> / WAIT <ms>
//   CTRL <c>              control byte (CTRL c -> 0x03)
//   HEX <hh hh ..>        raw bytes
//   REPEAT <n>            repeat the previous line n times
//   Q <cmd> / QUACK <cmd> Bash Bunny prefix (e.g. Q STRING foo, Q ENTER)
// Text (STRING and bare lines) honors escapes: \n \r \t \0 \xHH \\.

#[derive(Clone)]
enum Step {
    Bytes(Vec<u8>),
    Delay(u64),
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

fn parse_snippet(s: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut prev: Vec<Step> = Vec::new(); // for REPEAT
    for raw in s.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start();
        let mut w = trimmed.splitn(2, char::is_whitespace);
        let mut kw = w.next().unwrap_or("").to_ascii_uppercase();
        let mut rest = w.next().unwrap_or("");
        // Bash Bunny Q/QUACK prefix: unwrap to the inner command
        if kw == "Q" || kw == "QUACK" {
            let mut w2 = rest.trim_start().splitn(2, char::is_whitespace);
            kw = w2.next().unwrap_or("").to_ascii_uppercase();
            rest = w2.next().unwrap_or("");
        }
        if kw == "REPEAT" {
            let n = rest.trim().parse::<usize>().unwrap_or(0);
            for _ in 0..n {
                steps.extend(prev.iter().cloned());
            }
            continue;
        }
        let line_steps = match parse_command(&kw, rest) {
            Some(v) => v,
            // bare line: type it verbatim (preserving indentation) + Enter
            None => {
                let mut b = process_escapes(line);
                b.push(b'\r');
                vec![Step::Bytes(b)]
            }
        };
        if kw != "REM" && kw != "#" {
            prev = line_steps.clone();
        }
        steps.extend(line_steps);
    }
    steps
}

/// Execute a snippet macro against the DATA port (background thread; honors delays).
pub fn play(shared: &Arc<Shared>, text: &str) {
    let steps = parse_snippet(text);
    let shared = shared.clone();
    std::thread::spawn(move || {
        for step in steps {
            match step {
                Step::Bytes(b) => {
                    let _ = data_write(&shared, &b);
                }
                Step::Delay(ms) => std::thread::sleep(Duration::from_millis(ms.min(60_000))),
            }
        }
    });
}
