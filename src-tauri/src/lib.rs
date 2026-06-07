mod mcp;
mod protocol;
mod serial;

use std::sync::{Arc, Mutex};

use serde::Serialize;
use serial::{ConnState, McpToolFlags, PortDesc, RespFrame, SerialParams, Shared, SnippetRec};
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

/// Run snippet text through the macro player (escapes + `+++DELAY/ENTER/CTRL...+++`).
#[tauri::command]
fn run_text(state: tauri::State<AppState>, text: String) {
    serial::play(&state.shared, &text);
}

#[tauri::command]
fn send_cmd(state: tauri::State<AppState>, typ: u8, body: Vec<u8>) -> Result<RespFrame, String> {
    serial::send_cmd(&state.shared, typ, body)
}

#[tauri::command]
fn read_console(state: tauri::State<AppState>, max: usize) -> String {
    serial::read_console(&state.shared, max)
}

// ---- snippets (backend-owned store; mirrored to UI + MCP) ----
#[tauri::command]
fn snippets_get(state: tauri::State<AppState>) -> Vec<SnippetRec> {
    serial::snippets_all(&state.shared)
}

#[tauri::command]
fn snippet_upsert(state: tauri::State<AppState>, name: String, text: String, secret: bool) {
    serial::snippet_upsert(&state.shared, SnippetRec { name, text, secret });
}

#[tauri::command]
fn snippet_delete(state: tauri::State<AppState>, name: String) {
    serial::snippet_delete(&state.shared, &name);
}

#[tauri::command]
fn snippets_set(state: tauri::State<AppState>, snippets: Vec<SnippetRec>) {
    serial::snippets_set(&state.shared, snippets);
}

#[tauri::command]
fn mcp_start(state: tauri::State<AppState>, port: u16) -> McpStatus {
    let mut guard = state.mcp.lock().unwrap();
    if let Some((ct, _)) = guard.take() {
        ct.cancel();
    }
    let ct = mcp::start(state.shared.clone(), port);
    *guard = Some((ct, port));
    McpStatus { running: true, url: Some(format!("http://127.0.0.1:{port}/mcp")) }
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
            serial::init_snippets(&state.shared, app.handle().clone(), dir.join("snippets.json"));
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
            send_cmd,
            read_console,
            snippets_get,
            snippet_upsert,
            snippet_delete,
            snippets_set,
            mcp_start,
            mcp_stop,
            mcp_status,
            set_mcp_tools
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
