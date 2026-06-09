mod mcp;
mod protocol;
mod serial;

use std::sync::{Arc, Mutex};

use serde::Serialize;
use serial::{ConnState, McpToolFlags, PortDesc, RespFrame, SerialParams, Shared, MacroRec};
use tauri::Manager;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct AppState {
    shared: Arc<Shared>,
    mcp: Mutex<Option<(CancellationToken, u16)>>,
}

#[derive(Serialize)]
struct DetectResult {
    data: String,
    cmd: String,
}

#[derive(Serialize)]
struct McpStatus {
    running: bool,
    url: Option<String>,
}

#[tauri::command]
fn list_ports() -> Vec<PortDesc> {
    serial::list_ports()
}

#[tauri::command]
fn autodetect() -> Result<DetectResult, String> {
    serial::autodetect().map(|(data, cmd)| DetectResult { data, cmd })
}

#[tauri::command]
fn connect(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    data_port: String,
    cmd_port: Option<String>,
) -> Result<(), String> {
    serial::connect(&state.shared, app, &data_port, cmd_port.as_deref())
}

#[tauri::command]
fn disconnect(state: tauri::State<AppState>) {
    serial::disconnect(&state.shared);
}

#[tauri::command]
fn conn_state(state: tauri::State<AppState>) -> ConnState {
    serial::state(&state.shared)
}

/// Apply DATA serial params (baud/parity/stop/databits); reconnects the DATA port.
#[tauri::command]
fn set_data_params(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    params: SerialParams,
) -> Result<(), String> {
    serial::set_params(&state.shared, app, params)
}

#[tauri::command]
fn reconnect_data(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<(), String> {
    serial::reconnect_data(&state.shared, app)
}

#[tauri::command]
fn data_write(state: tauri::State<AppState>, bytes: Vec<u8>) -> Result<(), String> {
    serial::data_write(&state.shared, &bytes)
}

/// Run macro text through the macro player (escapes + `+++DELAY/ENTER/CTRL...+++`).
#[tauri::command]
fn run_text(state: tauri::State<AppState>, text: String, name: Option<String>) {
    serial::play(&state.shared, name.as_deref().unwrap_or("macro"), &text);
}

/// In-flight macro runs (for the queue panel).
#[tauri::command]
fn macro_runs(state: tauri::State<AppState>) -> Vec<serial::MacroRunInfo> {
    serial::macro_runs(&state.shared)
}

/// Request cancellation of a running macro by id.
#[tauri::command]
fn cancel_run(state: tauri::State<AppState>, id: u64) {
    serial::cancel_run(&state.shared, id);
}

#[tauri::command]
fn send_cmd(state: tauri::State<AppState>, typ: u8, body: Vec<u8>) -> Result<RespFrame, String> {
    serial::send_cmd(&state.shared, typ, body)
}

#[tauri::command]
fn read_console(state: tauri::State<AppState>, max: usize) -> String {
    serial::read_console(&state.shared, max)
}

// ---- macros (backend-owned store; mirrored to UI + MCP) ----
#[tauri::command]
fn macros_get(state: tauri::State<AppState>) -> Vec<MacroRec> {
    serial::macros_all(&state.shared)
}

#[tauri::command]
fn macro_upsert(state: tauri::State<AppState>, name: String, text: String, secret: bool) {
    serial::macro_upsert(&state.shared, MacroRec { name, text, secret });
}

#[tauri::command]
fn macro_delete(state: tauri::State<AppState>, name: String) {
    serial::macro_delete(&state.shared, &name);
}

#[tauri::command]
fn macros_set(state: tauri::State<AppState>, macros: Vec<MacroRec>) {
    serial::macros_set(&state.shared, macros);
}

#[tauri::command]
fn mcp_start(state: tauri::State<AppState>, port: u16) -> Result<McpStatus, String> {
    let mut guard = state.mcp.lock().unwrap();
    if let Some((ct, _)) = guard.take() {
        ct.cancel(); // tell the old server to release the socket
    }
    // Graceful shutdown of the old server is async, so the port may take a moment
    // to free — retry the bind briefly before giving up.
    let mut last = String::new();
    for _ in 0..15 {
        match mcp::start(state.shared.clone(), port) {
            Ok(ct) => {
                *guard = Some((ct, port));
                return Ok(McpStatus {
                    running: true,
                    url: Some(format!("http://127.0.0.1:{port}/mcp")),
                });
            }
            Err(e) => {
                last = e;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    Err(format!("could not bind 127.0.0.1:{port} — {last}"))
}

#[tauri::command]
fn mcp_stop(state: tauri::State<AppState>) -> McpStatus {
    if let Some((ct, _)) = state.mcp.lock().unwrap().take() {
        ct.cancel();
    }
    McpStatus { running: false, url: None }
}

/// Set which MCP tool groups are exposed to the LLM (applies on next MCP session).
#[tauri::command]
fn set_mcp_tools(state: tauri::State<AppState>, flags: McpToolFlags) {
    serial::set_mcp_tools(&state.shared, flags);
}

#[tauri::command]
fn mcp_status(state: tauri::State<AppState>) -> McpStatus {
    match &*state.mcp.lock().unwrap() {
        Some((_, port)) => {
            McpStatus { running: true, url: Some(format!("http://127.0.0.1:{port}/mcp")) }
        }
        None => McpStatus { running: false, url: None },
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .setup(|app| {
            let state = app.state::<AppState>();
            let dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            serial::init_macros(&state.shared, app.handle().clone(), dir.join("macros.json"));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_ports,
            autodetect,
            connect,
            disconnect,
            conn_state,
            set_data_params,
            reconnect_data,
            data_write,
            run_text,
            macro_runs,
            cancel_run,
            send_cmd,
            read_console,
            macros_get,
            macro_upsert,
            macro_delete,
            macros_set,
            mcp_start,
            mcp_stop,
            mcp_status,
            set_mcp_tools
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
