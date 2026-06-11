mod ble;
mod mcp;
pub mod protocol;
pub mod serial;
mod workspace;
pub mod ws;

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

/// Find a single-port muxed Duta: probe every candidate port (ESP32 / Pico /
/// nRF vendor ids) with a skrit-mux PING and return the first that answers.
#[tauri::command]
fn autodetect_mux() -> Result<String, String> {
    serial::autodetect_mux()
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

/// Connect a single-port muxed Duta (ESP32 / Pico / nRF52840): DATA + CMD share
/// one stream via skrit-mux.
#[tauri::command]
fn connect_muxed(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    port: String,
) -> Result<(), String> {
    serial::connect_muxed(&state.shared, app, &port)
}

#[tauri::command]
fn disconnect(state: tauri::State<AppState>) {
    serial::disconnect(&state.shared);
}

/// Scan for Duta peripherals over Bluetooth LE (blocks ~3s).
#[tauri::command]
fn ble_scan() -> Result<Vec<ble::BleDevice>, String> {
    tauri::async_runtime::block_on(ble::scan(3))
}

/// Connect a Duta over BLE by scanned device id (dual skrit GATT services: DATA + CMD).
#[tauri::command]
fn ble_connect(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<String, String> {
    tauri::async_runtime::block_on(ble::connect(state.shared.clone(), app, id))
}

/// Browse the LAN for Dutas advertising _skrit._tcp (mDNS); blocks ~timeout_ms.
#[tauri::command]
fn ws_discover(timeout_ms: Option<u64>) -> Result<Vec<ws::DiscoveredDuta>, String> {
    ws::discover(timeout_ms.unwrap_or(2500))
}

/// Connect a Duta over the network (WebSocket), authenticating with `password`.
#[tauri::command]
fn ws_connect(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    url: String,
    password: String,
) -> Result<ws::WsConnectResult, String> {
    tauri::async_runtime::block_on(ws::connect(state.shared.clone(), app, url, password))
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
fn macro_upsert(state: tauri::State<AppState>, name: String, text: String, secret: bool, set: String) {
    serial::macro_upsert(&state.shared, MacroRec { name, text, secret, set, tier: 0 });
}

#[derive(Serialize, serde::Deserialize)]
struct MacroSetFile {
    set: String,
    version: u32,
    macros: Vec<MacroRec>,
}

/// Export a set (or all macros) to a JSON file at `path`.
#[tauri::command]
fn export_set(state: tauri::State<AppState>, path: String, set: Option<String>) -> Result<(), String> {
    let all = serial::macros_all(&state.shared);
    let macros: Vec<MacroRec> = match &set {
        Some(s) => all.into_iter().filter(|m| &m.set == s).collect(),
        None => all,
    };
    let doc = MacroSetFile { set: set.unwrap_or_default(), version: 1, macros };
    let json = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

/// Import a macro-set JSON file at `path` (merge by name). Returns the count.
#[tauri::command]
fn import_set(state: tauri::State<AppState>, path: String) -> Result<usize, String> {
    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let doc: MacroSetFile = serde_json::from_str(&json).map_err(|e| format!("bad set file: {e}"))?;
    let recs: Vec<MacroRec> = doc
        .macros
        .into_iter()
        .map(|mut m| {
            if m.set.is_empty() {
                m.set = doc.set.clone();
            }
            m
        })
        .collect();
    Ok(serial::macros_import(&state.shared, recs))
}

#[tauri::command]
fn macro_delete(state: tauri::State<AppState>, name: String) {
    serial::macro_delete(&state.shared, &name);
}

// ---- workspace (a folder with a .sutra/ for macros + captures) ----
#[tauri::command]
fn get_workspace(app: tauri::AppHandle) -> Option<String> {
    workspace::current(&app).map(|p| p.to_string_lossy().into_owned())
}

/// Open a folder picker; on selection, adopt it as the workspace and re-point
/// the macro store into its .sutra/ (loading existing macros or migrating).
#[tauri::command]
fn pick_workspace(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<Option<String>, String> {
    let picked = workspace::pick(&app)?;
    if picked.is_some() {
        serial::relocate_macros(&state.shared, workspace::macros_path(&app));
    }
    Ok(picked)
}

/// Save raw ble-sniff records as a pcap (into the workspace's captures/, or via
/// a save dialog if no workspace). Returns the written path.
#[tauri::command]
fn save_ble_pcap(app: tauri::AppHandle, name: String, records: Vec<Vec<u8>>) -> Result<String, String> {
    workspace::save_ble_pcap(&app, &name, records)
}

/// Save raw ieee802154 records as an 802.15.4-TAP pcap (workspace or dialog).
#[tauri::command]
fn save_ieee154_pcap(app: tauri::AppHandle, name: String, records: Vec<Vec<u8>>) -> Result<String, String> {
    workspace::save_ieee154_pcap(&app, &name, records)
}

/// Is Wireshark's tshark available (gates the in-app decode action)?
#[tauri::command]
fn tshark_available(tshark_path: Option<String>) -> bool {
    workspace::tshark_available(tshark_path)
}

/// Dissect raw ieee802154 records with tshark (rtshark) → per-packet decode rows.
#[tauri::command]
fn dissect_ieee154(
    records: Vec<Vec<u8>>,
    tshark_path: Option<String>,
) -> Result<Vec<workspace::DecodedRow>, String> {
    workspace::dissect_ieee154(records, tshark_path)
}

/// The I2C device definitions in the workspace's .sutra/i2c/ (raw JSON).
#[tauri::command]
fn list_i2c_defs(app: tauri::AppHandle) -> Vec<serde_json::Value> {
    workspace::list_i2c_defs(&app)
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
    // to free. Retry the bind briefly before giving up.
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
    Err(format!("could not bind 127.0.0.1:{port}: {last}"))
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
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            let state = app.state::<AppState>();
            // macros live in the workspace's .sutra/ if one is selected, else app data
            let path = workspace::macros_path(app.handle());
            serial::init_macros(&state.shared, app.handle().clone(), path);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_ports,
            autodetect,
            autodetect_mux,
            connect,
            connect_muxed,
            ble_scan,
            ble_connect,
            ws_discover,
            ws_connect,
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
            get_workspace,
            pick_workspace,
            save_ble_pcap,
            save_ieee154_pcap,
            tshark_available,
            dissect_ieee154,
            list_i2c_defs,
            export_set,
            import_set,
            mcp_start,
            mcp_stop,
            mcp_status,
            set_mcp_tools
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
