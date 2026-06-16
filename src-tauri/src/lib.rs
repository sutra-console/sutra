mod ble;
mod boards;
pub mod interview; // live ZDP interview: sniffed frame → node discovery
pub mod lua; // sandboxed Lua scripting for .yantra control surfaces
pub mod macrovars; // {$name} macro-variable substitution (Zigbee inject + general)
mod mcp;
pub mod protocol;
pub mod serial;
pub mod vault; // at-rest secret encryption (the "Security" subsystem)
mod workspace;
pub mod ws;
pub mod zigbee; // Zigbee NWK/APS AES-CCM* security (network-model phase B)

use std::sync::{Arc, Mutex};

use serde::Serialize;
use serial::{ConnState, MacroRec, McpToolFlags, PortDesc, RespFrame, SerialParams, Shared};
use tauri::{Emitter, Manager};
use tokio_util::sync::CancellationToken;
use vault::{SecurityStatus, Vault};

#[derive(Default)]
struct AppState {
    shared: Arc<Shared>,
    mcp: Mutex<Option<(CancellationToken, u16)>>,
    lua: lua::LuaEngine, // per-surface yantra script VMs
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

// NOTE: every command that opens / probes a serial port (or scans a radio) is an
// `async fn` that offloads the blocking work via `spawn_blocking`. Synchronous
// Tauri commands run on the MAIN (UI) thread, so a slow Windows COM `open()` — a
// contended port stalls multiple seconds on the OS serial semaphore — would
// freeze the whole app. spawn_blocking keeps that off the UI thread.
#[tauri::command]
async fn list_ports() -> Vec<PortDesc> {
    tauri::async_runtime::spawn_blocking(serial::list_ports)
        .await
        .unwrap_or_default()
}

#[tauri::command]
async fn autodetect() -> Result<DetectResult, String> {
    tauri::async_runtime::spawn_blocking(|| {
        serial::autodetect().map(|(data, cmd)| DetectResult { data, cmd })
    })
    .await
    .map_err(|e| format!("background task failed: {e}"))?
}

/// Find a single-port muxed Duta: probe every candidate port (ESP32 / Pico /
/// nRF vendor ids) with a skrit-mux PING and return the first that answers.
#[tauri::command]
async fn autodetect_mux() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(serial::autodetect_mux)
        .await
        .map_err(|e| format!("background task failed: {e}"))?
}

/// Probe Duta-candidate ports for their board name (skips the connected port) so
/// the ports list can show e.g. "COM5 · Duta S3-Zero". Returns (port, name) pairs.
#[tauri::command]
async fn probe_boards(state: tauri::State<'_, AppState>) -> Result<Vec<(String, String)>, String> {
    let shared = state.shared.clone();
    Ok(tauri::async_runtime::spawn_blocking(move || serial::probe_boards(&shared))
        .await
        .unwrap_or_default())
}

/// Local board-def JSON files (<app_data>/boards/) that extend/override the
/// frontend's built-in board database.
#[tauri::command]
fn list_boards(app: tauri::AppHandle) -> Vec<serde_json::Value> {
    boards::list_boards(&app)
}

#[tauri::command]
async fn connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    data_port: String,
    cmd_port: Option<String>,
) -> Result<(), String> {
    let shared = state.shared.clone();
    tauri::async_runtime::spawn_blocking(move || {
        serial::connect(&shared, app, &data_port, cmd_port.as_deref())
    })
    .await
    .map_err(|e| format!("background task failed: {e}"))?
}

/// Connect a single-port muxed Duta (ESP32 / Pico / nRF52840): DATA + CMD share
/// one stream via skrit-mux.
#[tauri::command]
async fn connect_muxed(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    port: String,
) -> Result<(), String> {
    let shared = state.shared.clone();
    tauri::async_runtime::spawn_blocking(move || serial::connect_muxed(&shared, app, &port))
        .await
        .map_err(|e| format!("background task failed: {e}"))?
}

// Async because disconnect() join()s the reader thread, which can take up to the
// reader's read timeout (and a CDC port can lag releasing) — off the UI thread.
#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let shared = state.shared.clone();
    tauri::async_runtime::spawn_blocking(move || serial::disconnect(&shared))
        .await
        .map_err(|e| format!("background task failed: {e}"))
}

/// Scan for Duta peripherals over Bluetooth LE (blocks ~3s — off the UI thread).
#[tauri::command]
async fn ble_scan() -> Result<Vec<ble::BleDevice>, String> {
    tauri::async_runtime::spawn_blocking(|| tauri::async_runtime::block_on(ble::scan(3)))
        .await
        .map_err(|e| format!("background task failed: {e}"))?
}

/// Connect a Duta over BLE by scanned device id (dual skrit GATT services: DATA + CMD).
#[tauri::command]
async fn ble_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    let shared = state.shared.clone();
    tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(ble::connect(shared, app, id))
    })
    .await
    .map_err(|e| format!("background task failed: {e}"))?
}

/// Browse the LAN for Dutas advertising _skrit._tcp (mDNS); blocks ~timeout_ms.
#[tauri::command]
async fn ws_discover(timeout_ms: Option<u64>) -> Result<Vec<ws::DiscoveredDuta>, String> {
    tauri::async_runtime::spawn_blocking(move || ws::discover(timeout_ms.unwrap_or(2500)))
        .await
        .map_err(|e| format!("background task failed: {e}"))?
}

/// Connect a Duta over the network (WebSocket), authenticating with `password`.
#[tauri::command]
async fn ws_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
    password: String,
) -> Result<ws::WsConnectResult, String> {
    let shared = state.shared.clone();
    tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(ws::connect(shared, app, url, password))
    })
    .await
    .map_err(|e| format!("background task failed: {e}"))?
}

#[tauri::command]
fn conn_state(state: tauri::State<AppState>) -> ConnState {
    serial::state(&state.shared)
}

/// Apply DATA serial params (baud/parity/stop/databits); reconnects the DATA port.
#[tauri::command]
async fn set_data_params(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    params: SerialParams,
) -> Result<(), String> {
    let shared = state.shared.clone();
    tauri::async_runtime::spawn_blocking(move || serial::set_params(&shared, app, params))
        .await
        .map_err(|e| format!("background task failed: {e}"))?
}

#[tauri::command]
async fn reconnect_data(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let shared = state.shared.clone();
    tauri::async_runtime::spawn_blocking(move || serial::reconnect_data(&shared, app))
        .await
        .map_err(|e| format!("background task failed: {e}"))?
}

#[tauri::command]
fn data_write(state: tauri::State<AppState>, bytes: Vec<u8>) -> Result<(), String> {
    serial::data_write(&state.shared, &bytes)
}

/// Run macro text through the macro player (escapes + `+++DELAY/ENTER/CTRL...+++`).
#[tauri::command]
fn run_text(
    state: tauri::State<AppState>,
    text: String,
    name: Option<String>,
) -> Result<(), String> {
    serial::play(&state.shared, name.as_deref().unwrap_or("macro"), &text)
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
fn macro_upsert(
    state: tauri::State<AppState>,
    name: String,
    text: String,
    secret: bool,
    set: String,
) {
    serial::macro_upsert(
        &state.shared,
        MacroRec {
            name,
            text,
            secret,
            set,
            tier: 0,
        },
    );
}

#[derive(Serialize, serde::Deserialize)]
struct MacroSetFile {
    set: String,
    version: u32,
    macros: Vec<MacroRec>,
}

/// Export a set (or all macros) to a JSON file at `path`.
#[tauri::command]
fn export_set(
    state: tauri::State<AppState>,
    path: String,
    set: Option<String>,
) -> Result<(), String> {
    let all = serial::macros_all(&state.shared);
    let macros: Vec<MacroRec> = match &set {
        Some(s) => all.into_iter().filter(|m| &m.set == s).collect(),
        None => all,
    };
    let doc = MacroSetFile {
        set: set.unwrap_or_default(),
        version: 1,
        macros,
    };
    let json = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

/// Import a macro-set JSON file at `path` (merge by name). Returns the count.
#[tauri::command]
fn import_set(state: tauri::State<AppState>, path: String) -> Result<usize, String> {
    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let doc: MacroSetFile =
        serde_json::from_str(&json).map_err(|e| format!("bad set file: {e}"))?;
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
fn pick_workspace(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
) -> Result<Option<String>, String> {
    let picked = workspace::pick(&app)?;
    if picked.is_some() {
        // Silent unlock with the cleartext app key (no-op if no vault / password set),
        // then load macros from the (now-unlocked) store.
        if let Some(dot) = workspace::dot_sutra_existing(&app) {
            vault::auto_unlock(&app, &dot);
        }
        serial::relocate_macros(&state.shared, workspace::macros_path(&app));
    }
    Ok(picked)
}

/// Adopt a known folder path as the workspace (Open Recent) and re-point macros.
#[tauri::command]
fn set_workspace(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    path: String,
) -> Result<String, String> {
    let set = workspace::set_known(&app, &path)?;
    serial::relocate_macros(&state.shared, workspace::macros_path(&app));
    Ok(set)
}

/// Forget the current workspace; macros fall back to the app data dir.
#[tauri::command]
fn close_workspace(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<(), String> {
    workspace::close(&app)?;
    serial::relocate_macros(&state.shared, workspace::macros_path(&app));
    Ok(())
}

/// Export the workspace network model (discovered nodes + keys) to a JSON file.
#[tauri::command]
fn export_networks(app: tauri::AppHandle, path: String) -> Result<(), String> {
    workspace::export_networks(&app, &path)
}

/// Save raw ble-sniff records as a pcap (into the workspace's captures/, or via
/// a save dialog if no workspace). Returns the written path.
#[tauri::command]
fn save_ble_pcap(
    app: tauri::AppHandle,
    name: String,
    records: Vec<Vec<u8>>,
) -> Result<String, String> {
    workspace::save_ble_pcap(&app, &name, records)
}

/// Save raw ieee802154 records as an 802.15.4-TAP pcap (workspace or dialog).
#[tauri::command]
fn save_ieee154_pcap(
    app: tauri::AppHandle,
    name: String,
    records: Vec<Vec<u8>>,
) -> Result<String, String> {
    workspace::save_ieee154_pcap(&app, &name, records)
}

/// Is Wireshark's tshark available (gates the in-app decode action)?
#[tauri::command]
fn tshark_available(tshark_path: Option<String>) -> bool {
    workspace::tshark_available(tshark_path)
}

/// Dissect raw ieee802154 records with tshark (rtshark) → per-packet decode rows.
/// Uses the workspace's saved Zigbee keys to decrypt the payload where possible.
#[tauri::command]
fn dissect_ieee154(
    app: tauri::AppHandle,
    records: Vec<Vec<u8>>,
    tshark_path: Option<String>,
) -> Result<Vec<workspace::DecodedRow>, String> {
    let keys = workspace::dissect_keys(&app); // (key, label, protocol) — Zigbee or Thread
    workspace::dissect_ieee154(records, tshark_path, keys)
}

/// Read the workspace network model (keys + discovered nodes).
#[tauri::command]
fn get_networks(app: tauri::AppHandle) -> workspace::Networks {
    workspace::load_networks(&app)
}

/// Persist the workspace network model.
#[tauri::command]
fn set_networks(app: tauri::AppHandle, networks: workspace::Networks) -> Result<(), String> {
    workspace::save_networks(&app, &networks)
}

/// Set a node's nickname on the active network (atomic load→set→save).
#[tauri::command]
fn set_node_name(app: tauri::AppHandle, addr: String, name: String) -> Result<(), String> {
    workspace::set_node_name(&app, &addr, &name)
}

/// Try to decode a sniffed 802.15.4 MAC frame (no FCS) as a ZDP reply against the
/// active network; on success merges the discovery into the node model. The
/// 802.15.4 panel calls this for each captured frame during an interview.
#[tauri::command]
fn zdp_ingest(app: tauri::AppHandle, frame: Vec<u8>) -> Option<interview::ZdpDiscovery> {
    interview::ingest_mac_frame(&app, &frame)
}

/// Batch-observe sniffed MAC frames against the active network: ZDP replies feed
/// active discovery, and any other application frame passively records its node's
/// endpoints/clusters. Returns how many frames changed the model. The 802.15.4
/// panel calls this for each batch of captured frames.
#[tauri::command]
fn observe_frames(app: tauri::AppHandle, frames: Vec<Vec<u8>>) -> interview::IngestResult {
    interview::ingest_frames(&app, &frames)
}

/// The I2C device definitions in the workspace's .sutra/i2c/ (raw JSON).
#[tauri::command]
fn list_i2c_defs(app: tauri::AppHandle) -> Vec<serde_json::Value> {
    workspace::list_i2c_defs(&app)
}

/// The .yantra control surfaces in the workspace's .sutra/yantra/ (parsed YAML→JSON).
#[tauri::command]
fn list_yantras(app: tauri::AppHandle) -> Vec<workspace::YantraDoc> {
    workspace::list_yantras(&app)
}

/// Write a control surface spec back to its .yantra file (visual editor save).
#[tauri::command]
fn save_yantra(
    app: tauri::AppHandle,
    file: String,
    spec: serde_json::Value,
) -> Result<String, String> {
    workspace::save_yantra(&app, &file, spec)
}

/// Create a new blank control surface; returns its filename.
#[tauri::command]
fn create_yantra(app: tauri::AppHandle, name: String) -> Result<String, String> {
    workspace::create_yantra(&app, &name)
}

/// Delete a control surface file.
#[tauri::command]
fn delete_yantra(app: tauri::AppHandle, file: String) -> Result<(), String> {
    workspace::delete_yantra(&app, &file)
}

/// Import an external .yantra/.yaml/.json file into the workspace; returns the saved filename.
#[tauri::command]
fn import_yantra(app: tauri::AppHandle, path: String) -> Result<String, String> {
    workspace::import_yantra(&app, &path)
}

/// Run one reactive tick of a yantra surface's Lua scripts (per-surface persistent
/// VM). `vars` is the frontend bus snapshot; returns { sets, sends, logs } to apply.
#[tauri::command]
fn yantra_eval(
    state: tauri::State<AppState>,
    key: String,
    script: String,
    widgets: Vec<lua::WidgetScript>,
    vars: serde_json::Value,
) -> Result<lua::EvalOut, String> {
    state.lua.eval(&key, &script, &widgets, vars)
}

/// Event wiring: call a named handler (a widget's `on:{press:fn}` etc.) in the
/// surface VM, passing the current bus (`vars`) + the event payload (`args`).
/// Returns the handler's { sets, frames, sends, logs } for the frontend to apply.
#[tauri::command]
fn yantra_call(
    state: tauri::State<AppState>,
    key: String,
    script: String,
    widgets: Vec<lua::WidgetScript>,
    func: String,
    vars: serde_json::Value,
    args: serde_json::Value,
) -> Result<lua::EvalOut, String> {
    state.lua.call(&key, &script, &widgets, &func, vars, args)
}

// ---- security: at-rest secret encryption ----

/// Current Security panel state for the active workspace.
#[tauri::command]
fn security_status(app: tauri::AppHandle) -> SecurityStatus {
    vault::status(&app, workspace::dot_sutra_existing(&app).as_deref())
}

/// Encrypt the workspace's secrets into the vault (optionally password-protecting the
/// app key). Removes the plaintext, unlocks the session, refreshes the .gitignore.
#[tauri::command]
fn security_enable(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    password: Option<String>,
) -> Result<SecurityStatus, String> {
    let dot = workspace::dot_sutra_existing(&app).ok_or("select a workspace first")?;
    vault::enable(&app, &dot, password)?;
    workspace::refresh_gitignore(&app);
    serial::reload_macros(&state.shared);
    let _ = app.emit("sutra://vault", ());
    Ok(vault::status(&app, Some(&dot)))
}

/// Decrypt the vault back to plaintext files and turn encryption off (needs unlock).
#[tauri::command]
fn security_disable(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
) -> Result<SecurityStatus, String> {
    let dot = workspace::dot_sutra_existing(&app).ok_or("select a workspace first")?;
    vault::disable(&app, &dot)?;
    workspace::refresh_gitignore(&app);
    serial::reload_macros(&state.shared);
    let _ = app.emit("sutra://vault", ());
    Ok(vault::status(&app, Some(&dot)))
}

/// Unlock the active workspace's vault (password required iff the app key is protected).
#[tauri::command]
fn vault_unlock(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    password: Option<String>,
) -> Result<SecurityStatus, String> {
    let dot = workspace::dot_sutra_existing(&app).ok_or("select a workspace first")?;
    vault::unlock(&app, &dot, password)?;
    serial::reload_macros(&state.shared);
    let _ = app.emit("sutra://vault", ());
    Ok(vault::status(&app, Some(&dot)))
}

/// Forget the decrypted session (lock the workspace; secrets become unreadable).
#[tauri::command]
fn vault_lock(app: tauri::AppHandle, state: tauri::State<AppState>) -> SecurityStatus {
    vault::lock(&app);
    serial::reload_macros(&state.shared);
    let _ = app.emit("sutra://vault", ());
    vault::status(&app, workspace::dot_sutra_existing(&app).as_deref())
}

/// Set, change, or clear the app-key password (`new` empty/None clears it).
#[tauri::command]
fn security_set_password(
    app: tauri::AppHandle,
    old: Option<String>,
    new: Option<String>,
) -> Result<SecurityStatus, String> {
    vault::set_password(&app, old, new)?;
    Ok(vault::status(
        &app,
        workspace::dot_sutra_existing(&app).as_deref(),
    ))
}

/// Generate a fresh app key (re-keys an encrypted workspace; needs unlock + no password).
#[tauri::command]
fn app_key_regenerate(app: tauri::AppHandle) -> Result<SecurityStatus, String> {
    let dot = workspace::dot_sutra_existing(&app);
    vault::regenerate_app_key(&app, dot.as_deref())?;
    Ok(vault::status(&app, dot.as_deref()))
}

/// Toggle whether git tracks the encrypted vault and/or captures (re-runs .gitignore).
#[tauri::command]
fn security_set_git_track(
    app: tauri::AppHandle,
    vault_tracked: Option<bool>,
    captures_tracked: Option<bool>,
) -> Result<SecurityStatus, String> {
    let dot = workspace::dot_sutra_existing(&app).ok_or("select a workspace first")?;
    vault::set_git_track(&dot, vault_tracked, captures_tracked)?;
    workspace::refresh_gitignore(&app);
    Ok(vault::status(&app, Some(&dot)))
}

/// Add a collaborator's public key (age `age1…` or SSH) as a vault recipient (needs unlock).
#[tauri::command]
fn security_add_recipient(
    app: tauri::AppHandle,
    pubkey: String,
    label: String,
) -> Result<SecurityStatus, String> {
    let dot = workspace::dot_sutra_existing(&app).ok_or("select a workspace first")?;
    vault::add_recipient(&app, &dot, &pubkey, &label)?;
    Ok(vault::status(&app, Some(&dot)))
}

/// Remove a vault recipient and re-encrypt (needs unlock; refuses this app's own key).
#[tauri::command]
fn security_remove_recipient(
    app: tauri::AppHandle,
    pubkey: String,
) -> Result<SecurityStatus, String> {
    let dot = workspace::dot_sutra_existing(&app).ok_or("select a workspace first")?;
    vault::remove_recipient(&app, &dot, &pubkey)?;
    Ok(vault::status(&app, Some(&dot)))
}

/// Install/remove a pre-commit hook that blocks committing plaintext secrets.
#[tauri::command]
fn security_set_git_hooks(app: tauri::AppHandle, on: bool) -> Result<SecurityStatus, String> {
    let dot = workspace::dot_sutra_existing(&app).ok_or("select a workspace first")?;
    vault::set_git_hooks(&app, &dot, on)?;
    Ok(vault::status(&app, Some(&dot)))
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
    McpStatus {
        running: false,
        url: None,
    }
}

/// Set which MCP tool groups are exposed to the LLM (applies on next MCP session).
#[tauri::command]
fn set_mcp_tools(state: tauri::State<AppState>, flags: McpToolFlags) {
    serial::set_mcp_tools(&state.shared, flags);
}

#[tauri::command]
fn mcp_status(state: tauri::State<AppState>) -> McpStatus {
    match &*state.mcp.lock().unwrap() {
        Some((_, port)) => McpStatus {
            running: true,
            url: Some(format!("http://127.0.0.1:{port}/mcp")),
        },
        None => McpStatus {
            running: false,
            url: None,
        },
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState::default())
        .manage(Vault::default())
        .setup(|app| {
            // silent unlock with the cleartext app key (no-op if no vault / password set)
            if let Some(dot) = workspace::dot_sutra_existing(app.handle()) {
                vault::auto_unlock(app.handle(), &dot);
            }
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
            probe_boards,
            list_boards,
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
            set_workspace,
            close_workspace,
            export_networks,
            save_ble_pcap,
            save_ieee154_pcap,
            tshark_available,
            dissect_ieee154,
            get_networks,
            set_networks,
            set_node_name,
            zdp_ingest,
            observe_frames,
            list_i2c_defs,
            list_yantras,
            save_yantra,
            create_yantra,
            delete_yantra,
            import_yantra,
            yantra_eval,
            yantra_call,
            security_status,
            security_enable,
            security_disable,
            vault_unlock,
            vault_lock,
            security_set_password,
            app_key_regenerate,
            security_set_git_track,
            security_add_recipient,
            security_remove_recipient,
            security_set_git_hooks,
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
