//! Workspace: a user-chosen folder holding a `.sutra/` directory where Sutra
//! keeps its per-project state â€” macros (`macros.json`) and capture exports
//! (`captures/*.pcap`). The chosen path persists in the app config dir; with no
//! workspace selected, macros fall back to the app data dir and captures prompt
//! for a location.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

/// Where we remember the selected workspace path.
fn marker_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("workspace.txt"))
}

/// The currently selected workspace folder, if any (and it still exists).
pub fn current(app: &AppHandle) -> Option<PathBuf> {
    let p = std::fs::read_to_string(marker_path(app)?).ok()?;
    let path = PathBuf::from(p.trim());
    path.is_dir().then_some(path)
}

/// The `.sutra/` dir inside the workspace, creating it (and `captures/`) if set.
fn dot_sutra(app: &AppHandle) -> Option<PathBuf> {
    let ws = current(app)?;
    let dot = ws.join(".sutra");
    let _ = std::fs::create_dir_all(dot.join("captures"));
    Some(dot)
}

/// The `.sutra/` path if a workspace is selected, *without* creating anything
/// (for read-only status checks — e.g. the vault layer).
pub fn dot_sutra_existing(app: &AppHandle) -> Option<PathBuf> {
    current(app).map(|ws| ws.join(".sutra"))
}

/// Recompute the workspace's managed `.gitignore` block (security config changed).
pub fn refresh_gitignore(app: &AppHandle) {
    if let Some(dot) = dot_sutra(app) {
        rewrite_gitignore(&dot);
    }
}

/// Where the macro store lives: `<ws>/.sutra/macros.json`, else the app data dir.
pub fn macros_path(app: &AppHandle) -> PathBuf {
    if let Some(dot) = dot_sutra(app) {
        return dot.join("macros.json");
    }
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("macros.json")
}

/// Persist `path` as the workspace and lay out `.sutra/`. Returns the path.
fn set(app: &AppHandle, path: &Path) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Err("not a folder".into());
    }
    let marker = marker_path(app).ok_or("no config dir")?;
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&marker, path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    let dot = path.join(".sutra");
    let _ = std::fs::create_dir_all(dot.join("captures"));
    rewrite_gitignore(&dot);
    seed_i2c_example(&dot.join("i2c"));
    seed_yantra_example(&dot.join("yantra"));
    Ok(path.to_path_buf())
}

// Keep workspace secrets out of version control if the folder is a git repo. The
// block is regenerated from the security config (see vault::gitignore_flags); we
// only manage the marked region, so user-added lines outside it are preserved.
const GI_BEGIN: &str = "# >>> sutra managed (do not edit this block)";
const GI_END: &str = "# <<< sutra managed";

fn rewrite_gitignore(dot: &Path) {
    let _ = std::fs::create_dir_all(dot);
    let (ignore_secrets, ignore_vault, ignore_captures) = crate::vault::gitignore_flags(dot);
    let mut b = String::new();
    b.push_str(GI_BEGIN);
    b.push_str("\n# Sutra keeps device secrets out of version control.\n");
    if ignore_secrets {
        b.push_str("keys.json\nnetworks.json\nnetworks.json.bak\nmacros.json\n");
    }
    if ignore_vault {
        b.push_str("# Encrypted vault — enable git tracking in Security to share it:\nsecrets.age\n");
    }
    if ignore_captures {
        b.push_str("# Captures can contain unencrypted frames/packets:\ncaptures/\n");
    }
    b.push_str("# Shareable (no secrets): i2c/ and yantra/ are NOT ignored.\n");
    b.push_str(GI_END);
    b.push('\n');

    let existing = std::fs::read_to_string(dot.join(".gitignore")).unwrap_or_default();
    let _ = std::fs::write(dot.join(".gitignore"), merge_managed_block(&existing, &b));
}

/// Splice the managed block into `existing`, replacing any prior managed region
/// and preserving everything outside it.
fn merge_managed_block(existing: &str, block: &str) -> String {
    if let (Some(start), Some(end)) = (existing.find(GI_BEGIN), existing.find(GI_END)) {
        if start < end {
            let after = end + GI_END.len();
            let tail = existing[after..].trim_start_matches('\n');
            let head = &existing[..start];
            return format!("{head}{block}{tail}");
        }
    }
    if existing.trim().is_empty() {
        block.to_string()
    } else {
        format!("{}\n\n{block}", existing.trim_end())
    }
}

// ---- I2C device definitions (.sutra/i2c/*.json) ----------------------------

const I2C_EXAMPLE: &str = r#"{
  "name": "Example device",
  "addr": 60,
  "registers": [
    { "name": "Config",  "reg": 1,   "bytes": 1, "access": "rw", "control": "number", "desc": "8-bit config register" },
    { "name": "Enable",  "reg": 2,   "bytes": 1, "access": "rw", "control": "toggle" },
    { "name": "Level",   "reg": 3,   "bytes": 1, "access": "rw", "control": "slider", "min": 0, "max": 255 },
    { "name": "Mode",    "reg": 4,   "bytes": 1, "access": "rw", "control": "enum",
      "options": [ { "label": "Off", "value": 0 }, { "label": "Auto", "value": 1 }, { "label": "Manual", "value": 2 } ] },
    { "name": "Reset",   "reg": 255, "bytes": 0, "access": "w",  "control": "button", "desc": "command, no value" }
  ]
}
"#;

// Starter .yantra: a generic MTK/NMEA GPS over UART. Each widget sends UART text;
// readouts match a capture group on the live console stream. Hand-editable —
// this is what a vendor would ship to make Sutra their device's config app.
const GPS_YANTRA: &str = r#"# GPS module control surface (generic MTK / NMEA over UART).
# Widgets send the `send:` text to the device; readouts watch the console for `match`.
name: GPS Module
description: Configure an MTK/NMEA GPS over UART.
cols: 6
widgets:
  - { type: button, label: Hot Start,     x: 0, y: 0, w: 2, h: 1, send: "$PMTK101*32\r\n" }
  - { type: button, label: Cold Start,    x: 2, y: 0, w: 2, h: 1, send: "$PMTK103*30\r\n" }
  - { type: button, label: Factory Reset, x: 4, y: 0, w: 2, h: 1, send: "$PMTK104*37\r\n" }
  - type: select
    label: Update rate
    x: 0
    y: 1
    w: 3
    h: 1
    options:
      - { label: "1 Hz",  send: "$PMTK220,1000*1F\r\n" }
      - { label: "5 Hz",  send: "$PMTK220,200*2C\r\n" }
      - { label: "10 Hz", send: "$PMTK220,100*2F\r\n" }
  - type: select
    label: NMEA output
    x: 3
    y: 1
    w: 3
    h: 1
    options:
      - { label: "RMC + GGA", send: "$PMTK314,0,1,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0*28\r\n" }
      - { label: "All",       send: "$PMTK314,-1*04\r\n" }
  - { type: readout, label: Fix quality, x: 0, y: 2, w: 3, h: 1, match: "GGA,[^,]*,[^,]*,[^,]*,[^,]*,[^,]*,([0-9])", help: "0 none · 1 GPS · 2 DGPS" }
  - { type: readout, label: Satellites,  x: 3, y: 2, w: 3, h: 1, match: "GGA(?:,[^,]*){6},0*([0-9]+)" }
# Actions aren't UART-only. A widget's `send:` (or a toggle's on:/off:, a select
# option's send:) is transport-agnostic — a bare string is a raw DATA write
# (UART/console); the object forms target other buses:
#   send: { i2c:    { addr: 0x60, write: [0x01, 0xff], read: 0 } }   # I2C transfer
#   send: { invoke: { id: 1, args: [10, 20] } }                       # device INVOKE command
#   send: { cfg:    { key: 0x10, str: "value" } }                     # CFG set
# (SPI is future — it needs a skrit SPI vocabulary first.)
# Future: a `transform:`/`script:` (Lua) per widget, and plugin widget `type`s.
"#;

/// Drop a starter def into an empty i2c/ dir so the controls view has something
/// to show; the user edits/adds JSON files describing their real devices.
fn seed_i2c_example(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let empty = std::fs::read_dir(dir).map(|mut d| d.next().is_none()).unwrap_or(true);
    if empty {
        let _ = std::fs::write(dir.join("example.json"), I2C_EXAMPLE);
    }
}

/// Every I2C device definition in `<ws>/.sutra/i2c/*.json` (raw JSON values).
pub fn list_i2c_defs(app: &AppHandle) -> Vec<serde_json::Value> {
    let Some(dot) = dot_sutra(app) else { return Vec::new() };
    let dir = dot.join("i2c");
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if e.path().extension().is_some_and(|x| x == "json") {
                if let Some(v) = std::fs::read(e.path())
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                {
                    out.push(v);
                }
            }
        }
    }
    out
}

// ---- .yantra control surfaces (.sutra/yantra/*.yantra) ---------------------
// A .yantra is a YAML doc describing a device's controls: a list of widgets,
// each bound to a command/macro it sends (and optionally a value it reads). Sutra
// renders it as a panel — ship a .yantra and Sutra becomes that device's config
// app. Parsed YAML→JSON here (loose, so the schema can grow widget types/scripts/
// plugins without a rigid Rust struct); the frontend renders from the JSON.

#[derive(serde::Serialize)]
pub struct YantraDoc {
    pub file: String,             // filename, e.g. "gps.yantra"
    pub doc: serde_json::Value,   // the parsed spec
}

/// Every .yantra in `<ws>/.sutra/yantra/`, parsed YAML→JSON. Invalid files are
/// skipped (a bad one shouldn't hide the rest).
pub fn list_yantras(app: &AppHandle) -> Vec<YantraDoc> {
    let Some(dot) = dot_sutra(app) else { return Vec::new() };
    let dir = dot.join("yantra");
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "yantra" || x == "yaml" || x == "yml") {
                if let Some(doc) = std::fs::read_to_string(&p)
                    .ok()
                    .and_then(|s| serde_yaml::from_str::<serde_json::Value>(&s).ok())
                {
                    out.push(YantraDoc { file: e.file_name().to_string_lossy().into_owned(), doc });
                }
            }
        }
    }
    out.sort_by(|a, b| a.file.cmp(&b.file));
    out
}

/// Sanitize a user-supplied .yantra filename and force the `.yantra` extension.
/// Returns None if nothing usable remains.
fn safe_yantra_name(file: &str) -> Option<String> {
    let stem = std::path::Path::new(file.trim())
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.replace(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'), "-"))
        .map(|s| s.trim_matches('-').to_string())
        .filter(|s| !s.is_empty())?;
    Some(format!("{stem}.yantra"))
}

/// Write a control surface to `<ws>/.sutra/yantra/<file>` as YAML (the editor edits
/// the JSON spec; we serialize it back to a `.yantra`). Returns the saved filename.
pub fn save_yantra(app: &AppHandle, file: &str, spec: serde_json::Value) -> Result<String, String> {
    let dir = dot_sutra(app).ok_or("no workspace selected")?.join("yantra");
    let _ = std::fs::create_dir_all(&dir);
    let name = safe_yantra_name(file).ok_or("invalid file name")?;
    let yaml = serde_yaml::to_string(&spec).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(&name), yaml).map_err(|e| e.to_string())?;
    Ok(name)
}

/// Create a blank control surface named `name` and return its filename.
pub fn create_yantra(app: &AppHandle, name: &str) -> Result<String, String> {
    let title = name.trim();
    if title.is_empty() {
        return Err("name required".into());
    }
    let spec = serde_json::json!({ "name": title, "cols": 6, "widgets": [] });
    save_yantra(app, title, spec)
}

/// Import an external `.yantra`/`.yaml`/`.json` file into the workspace. The
/// source is parsed (validating it) and re-serialized as YAML under
/// `.sutra/yantra/`; the name comes from the source file stem and is
/// de-duplicated so an import never clobbers an existing surface. Returns the
/// saved filename.
pub fn import_yantra(app: &AppHandle, path: &str) -> Result<String, String> {
    let dir = dot_sutra(app).ok_or("no workspace selected")?.join("yantra");
    let _ = std::fs::create_dir_all(&dir);
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    // YAML is a JSON superset, so this accepts .yantra/.yaml/.json alike.
    let spec: serde_json::Value =
        serde_yaml::from_str(&text).map_err(|_| "not a valid .yantra (YAML/JSON) file".to_string())?;
    let base = safe_yantra_name(path).ok_or("invalid file name")?;
    let stem = base.trim_end_matches(".yantra");
    let mut name = format!("{stem}.yantra");
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("{stem}-{n}.yantra");
        n += 1;
    }
    let yaml = serde_yaml::to_string(&spec).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(&name), yaml).map_err(|e| e.to_string())?;
    Ok(name)
}

/// Delete a control surface file from the workspace.
pub fn delete_yantra(app: &AppHandle, file: &str) -> Result<(), String> {
    let dir = dot_sutra(app).ok_or("no workspace selected")?.join("yantra");
    let name = safe_yantra_name(file).ok_or("invalid file name")?;
    std::fs::remove_file(dir.join(name)).map_err(|e| e.to_string())
}

fn seed_yantra_example(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let empty = std::fs::read_dir(dir).map(|mut d| d.next().is_none()).unwrap_or(true);
    if empty {
        let _ = std::fs::write(dir.join("gps.yantra"), GPS_YANTRA);
    }
}

// ---- pcap export -----------------------------------------------------------

const DLT_BLUETOOTH_LE_LL_WITH_PHDR: u32 = 256;

fn pcap_header(linktype: u32) -> Vec<u8> {
    let mut h = vec![0u8; 24];
    h[0..4].copy_from_slice(&0xA1B2_C3D4u32.to_le_bytes());
    h[4..6].copy_from_slice(&2u16.to_le_bytes());
    h[6..8].copy_from_slice(&4u16.to_le_bytes());
    h[16..20].copy_from_slice(&65535u32.to_le_bytes());
    h[20..24].copy_from_slice(&linktype.to_le_bytes());
    h
}

fn pcap_record(out: &mut Vec<u8>, ts_ms: u32, data: &[u8]) {
    out.extend_from_slice(&(ts_ms / 1000).to_le_bytes());
    out.extend_from_slice(&((ts_ms % 1000) * 1000).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
}

/// Build a pcap (DLT_BLUETOOTH_LE_LL_WITH_PHDR) from raw ble-sniff records
/// (each: ts(4)Â·ch(1)Â·rssi(1)Â·aa(4)Â·len(1)Â·pdu). One LL packet per record.
fn ble_pcap(records: &[Vec<u8>]) -> Vec<u8> {
    let mut out = pcap_header(DLT_BLUETOOTH_LE_LL_WITH_PHDR);
    for rec in records {
        if rec.len() < 11 {
            continue;
        }
        let ts = u32::from_le_bytes([rec[0], rec[1], rec[2], rec[3]]);
        let ch = rec[4];
        let sig_power = (rec[5] as i8).wrapping_neg(); // record holds the -dBm magnitude
        let aa = &rec[6..10];
        let pdu_len = rec[10] as usize;
        if rec.len() < 11 + pdu_len {
            continue;
        }
        let pdu = &rec[11..11 + pdu_len];
        // 10-byte pseudo-header
        let mut data = vec![0u8; 10];
        data[0] = ch;
        data[1] = sig_power as u8;
        data[4..8].copy_from_slice(&0x8E89BED6u32.to_le_bytes()); // reference access address
        // flags: dewhitened | sig-power-valid | ref-AA-valid | crc-checked | crc-valid
        let flags: u16 = 0x0001 | 0x0002 | 0x0010 | 0x0040 | 0x0080;
        data[8..10].copy_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(aa); // LL packet: access address â€¦
        data.extend_from_slice(pdu); //            â€¦ + PDU
        pcap_record(&mut out, ts, &data);
    }
    out
}

const LINKTYPE_IEEE802_15_4_TAP: u32 = 283;

/// Build a pcap (LINKTYPE_IEEE802_15_4_TAP) from raw ieee802154 records
/// (each: ts(4)Â·ch(1)Â·rssi(1,signed)Â·lqi(1)Â·flags(1)Â·len(1)Â·psdu). One TAP
/// packet per record: a TLV pseudo-header + the MAC frame (FCS dropped). Mirrors
/// sutra-extcap so saved captures and live Wireshark captures look identical.
fn ieee154_pcap(records: &[Vec<u8>]) -> Vec<u8> {
    fn push_tlv(buf: &mut Vec<u8>, typ: u16, val: &[u8]) {
        buf.extend_from_slice(&typ.to_le_bytes());
        buf.extend_from_slice(&(val.len() as u16).to_le_bytes());
        buf.extend_from_slice(val);
        while !buf.len().is_multiple_of(4) {
            buf.push(0);
        }
    }
    let mut out = pcap_header(LINKTYPE_IEEE802_15_4_TAP);
    for rec in records {
        if rec.len() < 9 {
            continue;
        }
        let ts = u32::from_le_bytes([rec[0], rec[1], rec[2], rec[3]]);
        let channel = rec[4];
        let rssi = rec[5] as i8;
        let lqi = rec[6];
        let plen = rec[8] as usize;
        if plen < 2 || rec.len() < 9 + plen {
            continue;
        }
        let mac = &rec[9..9 + plen - 2]; // drop the trailing FCS field

        let mut tlvs = Vec::new();
        push_tlv(&mut tlvs, 0, &[0u8]); // FCS type: none present
        push_tlv(&mut tlvs, 1, &(rssi as f32).to_le_bytes()); // RSS dBm
        let mut ch = (channel as u16).to_le_bytes().to_vec();
        ch.push(0); // channel page 0
        push_tlv(&mut tlvs, 3, &ch);
        push_tlv(&mut tlvs, 10, &[lqi]); // LQI

        let mut data = Vec::with_capacity(4 + tlvs.len() + mac.len());
        data.push(0); // version
        data.push(0); // reserved
        data.extend_from_slice(&((4 + tlvs.len()) as u16).to_le_bytes());
        data.extend_from_slice(&tlvs);
        data.extend_from_slice(mac);
        pcap_record(&mut out, ts, &data);
    }
    out
}

/// Save the given raw ieee802154 records as a pcap (workspace or save dialog).
pub fn save_ieee154_pcap(app: &AppHandle, name: &str, records: Vec<Vec<u8>>) -> Result<String, String> {
    if records.is_empty() {
        return Err("no frames to save".into());
    }
    let bytes = ieee154_pcap(&records);
    let safe = name.replace(['/', '\\', ':'], "_");
    let path = if let Some(dot) = dot_sutra(app) {
        dot.join("captures").join(format!("{safe}.pcap"))
    } else {
        match app.dialog().file().add_filter("pcap", &["pcap"]).set_file_name(format!("{safe}.pcap")).blocking_save_file() {
            Some(p) => p.into_path().map_err(|e| e.to_string())?,
            None => return Err("cancelled".into()),
        }
    };
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

// ---- workspace credential store (.sutra/keys.json) -------------------------
// Per-workspace secrets that unlock a network â€” saved beside the captures they
// decrypt. Zigbee network/link keys today; BLE keys / device passwords can join
// the same file later.

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
pub struct ZigbeeKey {
    pub key: String,   // 32 hex chars (the 16-byte network or trust-center key)
    pub label: String, // human label (network name / PAN)
}
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct WorkspaceKeys {
    #[serde(default)]
    pub zigbee: Vec<ZigbeeKey>,
}

/// Load the workspace key store (empty if no workspace or no file yet).
pub fn load_keys(app: &AppHandle) -> WorkspaceKeys {
    dot_sutra(app)
        .and_then(|d| crate::vault::read_secret(app, &d, "keys.json"))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

// ---- workspace network model (.sutra/networks.json) ------------------------
// The network is the unit everything hangs off: its decryption key lives here
// (not as a device/link param â€” the firmware stays a dumb radio), alongside the
// nodes we discover *passively* from sniffed traffic. Active discovery (ZDP) and
// control fill in manufacturer/endpoints/clusters later.

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
pub struct NetNode {
    pub addr: String,             // short address, "0x1234"
    #[serde(default)]
    pub name: String,             // user nickname ("Living-room lamp"); "" = show the addr
    #[serde(default)]
    pub role: String,             // Coordinator / Router / End Device / Node (inferred)
    #[serde(default)]
    pub channels: Vec<u8>,        // 802.15.4 channels it's been heard on
    #[serde(default)]
    pub count: u32,               // frames observed
    #[serde(default)]
    pub last_seen: String,        // ISO-ish stamp set by the host when saved
    // -- enriched by active discovery (Phase B+), absent from passive capture --
    #[serde(default)]
    pub ieee: String,             // 64-bit IEEE/EUI address
    #[serde(default)]
    pub manufacturer: String,
    #[serde(default)]
    pub endpoints: Vec<NetEndpoint>,
}
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
pub struct NetEndpoint {
    pub id: u8,
    #[serde(default)]
    pub clusters: Vec<String>,    // input cluster ids, "0x0006"
}
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
pub struct Network {
    #[serde(default)]
    pub label: String,            // human name
    #[serde(default)]
    pub pan: String,              // PAN id, "0x39fd" ("" until known)
    #[serde(default)]
    pub channel: u8,              // 0 = unknown
    #[serde(default)]
    pub key: String,              // network/trust-center key, 32 hex (decryption)
    #[serde(default)]
    pub protocol: String,         // "" / "zigbee" (default) or "thread" — picks the tshark key table
    #[serde(default)]
    pub nodes: Vec<NetNode>,
    // -- host-side injector state (phase B): when Sutra builds + transmits
    //    encrypted frames it acts as a member with its OWN identity. Persisted so
    //    the frame counter advances monotonically across runs (anti-replay) and we
    //    never reuse a (key, counter) pair. Coordinator-safety: src/eui must stay
    //    distinct from any real node's. Assigned on first use (see serial.rs).
    #[serde(default)]
    pub inject_src: u16,          // our short address (0 = unassigned)
    #[serde(default)]
    pub inject_eui: String,       // our EUI-64, 16 hex ("" = unassigned)
    #[serde(default)]
    pub frame_counter: u32,       // next NWK frame counter to use
}
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Networks {
    #[serde(default)]
    pub networks: Vec<Network>,
    /// Label of the network macro `{$…}` variables resolve against. Empty ⇒ the
    /// first network with a key. This is the "active network" session config.
    #[serde(default)]
    pub active: String,
}

/// Index of the active network for macro-variable resolution: the one whose
/// label matches `active`, else the first that actually has a key.
pub fn active_network_index(nets: &Networks) -> Option<usize> {
    let want = nets.active.trim().to_lowercase();
    if !want.is_empty() {
        if let Some(i) = nets
            .networks
            .iter()
            .position(|n| n.label.trim().to_lowercase() == want)
        {
            return Some(i);
        }
    }
    nets.networks.iter().position(|n| !n.key.trim().is_empty())
}

/// Load the workspace network model. If `networks.json` doesn't exist yet but a
/// legacy `keys.json` does, migrate its keys into keyless-but-keyed networks so
/// no decryption key is lost in the move.
pub fn load_networks(app: &AppHandle) -> Networks {
    if let Some(dir) = dot_sutra(app) {
        if let Some(b) = crate::vault::read_secret(app, &dir, "networks.json") {
            if let Ok(n) = serde_json::from_slice::<Networks>(&b) {
                return n;
            }
        }
    }
    // migrate legacy keys.json â†’ one network per stored key
    let legacy = load_keys(app);
    Networks {
        networks: legacy
            .zigbee
            .into_iter()
            .map(|k| Network { label: k.label, key: k.key, ..Default::default() })
            .collect(),
        ..Default::default()
    }
}

/// Set a node's nickname on the active network, atomically (load → set → save) so
/// a concurrent passive-observe write can't clobber the rename. Creates the node
/// if it isn't in the model yet. `addr` is "0x1234".
pub fn set_node_name(app: &AppHandle, addr: &str, name: &str) -> Result<(), String> {
    let mut nets = load_networks(app);
    let idx = active_network_index(&nets).ok_or("no active network")?;
    let net = &mut nets.networks[idx];
    match net.nodes.iter_mut().find(|n| n.addr.eq_ignore_ascii_case(addr)) {
        Some(n) => n.name = name.to_string(),
        None => net.nodes.push(NetNode {
            addr: addr.to_string(),
            name: name.to_string(),
            ..Default::default()
        }),
    }
    save_networks(app, &nets)
}

/// Persist the workspace network model to `<ws>/.sutra/networks.json`.
pub fn save_networks(app: &AppHandle, nets: &Networks) -> Result<(), String> {
    let dot = dot_sutra(app).ok_or("no workspace selected")?;
    let json = serde_json::to_vec_pretty(nets).map_err(|e| e.to_string())?;
    crate::vault::write_secret(app, &dot, "networks.json", &json)
}

/// The (key, label, protocol) tuples to hand tshark for decryption — every
/// network that has a key. Protocol picks the tshark key table (Zigbee vs Thread).
pub fn dissect_keys(app: &AppHandle) -> Vec<(String, String, String)> {
    load_networks(app)
        .networks
        .into_iter()
        .filter(|n| !n.key.trim().is_empty())
        .map(|n| {
            let label = if n.label.is_empty() { n.pan.clone() } else { n.label.clone() };
            (n.key, label, n.protocol)
        })
        .collect()
}

// ---- tshark dissection -----------------------------------------------------
// Thread / Zigbee / Matter are standard, fully-dissected protocols â€” we don't
// reimplement them. Instead we shell out to Wireshark's `tshark` as a dissection
// engine: build the same LINKTYPE_IEEE802_15_4_TAP pcap, let tshark decode the
// whole stack, and consume the per-packet Protocol/Info columns.

/// Resolve the tshark binary: an explicit override (a file or its directory)
/// from Sutra's settings, else autodetect â€” PATH, then the usual install dirs.
fn resolve_tshark(explicit: Option<&str>) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "tshark.exe" } else { "tshark" };
    if let Some(p) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb); // a full path to the binary
        }
        let in_dir = pb.join(exe);
        if in_dir.is_file() {
            return Some(in_dir); // a directory containing it
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let p = dir.join(exe);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let candidates: &[&str] = if cfg!(windows) {
        &[
            r"C:\Program Files\Wireshark\tshark.exe",
            r"C:\Program Files (x86)\Wireshark\tshark.exe",
        ]
    } else {
        &[
            "/usr/bin/tshark",
            "/usr/local/bin/tshark",
            "/opt/homebrew/bin/tshark",
            "/Applications/Wireshark.app/Contents/MacOS/tshark",
        ]
    };
    candidates.iter().map(PathBuf::from).find(|p| p.is_file())
}

#[derive(serde::Serialize)]
pub struct DecodedRow {
    pub num: u32,
    pub protocol: String,              // Wireshark's Protocol column (ZigBee, Thread, â€¦)
    pub summary: String,               // Wireshark's Info column â€” the "what happened" line
    pub fields: Vec<(String, String)>, // (name, value) â€” the decoded field tree, drill-down
}

/// Fields surfaced on drill-down. What's visible depends on the kind â€” and on
/// whether the payload is encrypted (most Zigbee is, so only the NWK header shows
/// unless a network key is configured).
const FIELD_WHITELIST: &[&str] = &[
    "wpan.src16", "wpan.dst16", "wpan.dst_pan", "wpan.src64", "wpan.dst64",
    "zbee_nwk.src", "zbee_nwk.dst", "zbee_nwk.radius", "zbee_nwk.seqno", "zbee_nwk.cmd",
    "zbee_aps.cluster", "zbee_aps.profile", "zbee_aps.src", "zbee_aps.dst", "zbee_aps.cmd",
    "zbee_zcl.cmd.id", "zbee_zcl.attr.id",
    // Thread / Matter (Matter-over-Thread = 6LoWPAN · IPv6 · UDP · CoAP/Matter)
    "mle.cmd", "coap.code", "coap.mid", "coap.token", "ipv6.src", "ipv6.dst", "udp.port",
];

/// A Zigbee key as a tshark `uat:zigbee_pc_keys` preference (Key, Byte Order,
/// Label). Feeding it lets tshark decrypt NWK/APS so the summary/fields climb from
/// "Command" to the real ZCL command/cluster.
fn zigbee_key_pref(key: &str, label: &str) -> String {
    let label = label.replace(['"', ','], " ");
    format!("uat:zigbee_pc_keys:\"{}\",\"Normal\",\"{}\"", key.trim(), label)
}

/// A Thread network (master) key as a tshark `uat:ieee802154_keys` entry with
/// "Thread hash" — Wireshark derives the rotating MAC keys from it. Paired with
/// thr_auto_acq_thr_seq_ctr so the key-sequence counter is picked up from MLE.
fn thread_key_pref(key: &str) -> String {
    format!("uat:ieee802154_keys:\"{}\",\"0\",\"Thread hash\"", key.trim())
}

/// Whether tshark is reachable (gates/loads the Decode action in the UI).
pub fn tshark_available(tshark_path: Option<String>) -> bool {
    resolve_tshark(tshark_path.as_deref()).is_some()
}

/// tshark's column summary per packet (frame num, Protocol, Info) â€” the rendered
/// "what happened" line. A separate -T fields pass, since rtshark's PDML carries
/// the field tree but not the columns.
fn tshark_columns(
    bin: &Path,
    pcap: &str,
    key_prefs: &[String],
) -> Result<Vec<(u32, String, String)>, String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.args([
        "-r", pcap, "-T", "fields",
        "-e", "frame.number", "-e", "_ws.col.Protocol", "-e", "_ws.col.Info",
        "-E", "separator=/t",
    ]);
    for p in key_prefs {
        cmd.arg("-o").arg(p);
    }
    let out = cmd.output().map_err(|e| format!("run tshark: {e}"))?;
    if !out.status.success() {
        return Err(format!("tshark: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|line| {
            let mut it = line.splitn(3, '\t');
            let num = it.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
            (num, it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string())
        })
        .collect())
}

/// Dissect raw ieee802154 records with tshark into per-packet rows: the upper-
/// layer protocol + Wireshark's Info summary + the decoded field tree. `keys` are
/// (hex, label) Zigbee keys for decrypting the payload. Rows map 1:1 by order.
pub fn dissect_ieee154(
    records: Vec<Vec<u8>>,
    tshark_path: Option<String>,
    keys: Vec<(String, String, String)>, // (hex key, label, protocol: ""/zigbee or thread)
) -> Result<Vec<DecodedRow>, String> {
    if records.is_empty() {
        return Ok(vec![]);
    }
    let bin = resolve_tshark(tshark_path.as_deref())
        .ok_or("Wireshark (tshark) not found â€” set its path in Settings")?;
    let dir = bin.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();

    let bytes = ieee154_pcap(&records);
    let tmp = std::env::temp_dir().join("sutra_dissect.pcap");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    let tmp_str = tmp.to_string_lossy().into_owned();

    let mut key_prefs: Vec<String> = keys
        .iter()
        .filter(|(k, _, _)| !k.trim().is_empty())
        .map(|(k, l, proto)| {
            if proto.eq_ignore_ascii_case("thread") {
                thread_key_pref(k)
            } else {
                zigbee_key_pref(k, l)
            }
        })
        .collect();
    // Thread derives its MAC keys from a rotating sequence counter — let tshark
    // auto-acquire it from MLE so the derived keys line up with the traffic.
    if keys.iter().any(|(k, _, p)| !k.trim().is_empty() && p.eq_ignore_ascii_case("thread")) {
        key_prefs.push("thread.thr_auto_acq_thr_seq_ctr:TRUE".to_string());
    }

    // Field tree (rtshark), decrypted when a key matches. PATH points at the
    // resolved Wireshark dir so a non-PATH install + its DLLs both resolve.
    let mut b = rtshark::RTSharkBuilder::builder().input_path(&tmp_str).env_path(&dir);
    for p in &key_prefs {
        b = b.option(p);
    }
    let mut rt = b.spawn().map_err(|e| format!("tshark: {e}"))?;

    let mut by_num: std::collections::HashMap<u32, Vec<(String, String)>> =
        std::collections::HashMap::new();
    while let Some(pkt) = rt.read().map_err(|e| format!("tshark read: {e}"))? {
        let num = pkt
            .layer_name("frame")
            .and_then(|l| l.metadata("frame.number"))
            .and_then(|m| m.value().parse().ok())
            .unwrap_or(0);
        let mut fields = Vec::new();
        for layer in pkt.iter() {
            for &w in FIELD_WHITELIST {
                if let Some(m) = layer.metadata(w) {
                    fields.push((w.to_string(), m.value().to_string()));
                }
            }
        }
        by_num.insert(num, fields);
    }

    // The semantic summary (Info column), merged with the field tree by frame num.
    let cols = tshark_columns(&bin, &tmp_str, &key_prefs)?;
    let mut rows = Vec::with_capacity(cols.len());
    for (num, protocol, summary) in cols {
        let fields = by_num.remove(&num).unwrap_or_default();
        rows.push(DecodedRow { num, protocol, summary, fields });
    }
    Ok(rows)
}

// ---- Tauri commands --------------------------------------------------------

/// Open a folder picker and adopt the chosen folder as the workspace.
pub fn pick(app: &AppHandle) -> Result<Option<String>, String> {
    match app.dialog().file().blocking_pick_folder() {
        Some(p) => {
            let path = p.into_path().map_err(|e| e.to_string())?;
            let set_path = set(app, &path)?;
            Ok(Some(set_path.to_string_lossy().into_owned()))
        }
        None => Ok(None), // cancelled
    }
}

/// Adopt an already-known folder as the workspace (e.g. an Open Recent entry).
/// Errors if the path no longer points at a folder.
pub fn set_known(app: &AppHandle, path: &str) -> Result<String, String> {
    let set_path = set(app, Path::new(path))?;
    Ok(set_path.to_string_lossy().into_owned())
}

/// Forget the current workspace: macros + captures fall back to the app data dir.
pub fn close(app: &AppHandle) -> Result<(), String> {
    if let Some(marker) = marker_path(app) {
        if marker.exists() {
            std::fs::remove_file(&marker).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Export the workspace network model (discovered nodes + keys) to `path` as JSON.
pub fn export_networks(app: &AppHandle, path: &str) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&load_networks(app)).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// Save the given raw ble-sniff records as a pcap. With a workspace, writes to
/// `<ws>/.sutra/captures/<name>.pcap`; otherwise opens a save dialog. Returns the path.
pub fn save_ble_pcap(app: &AppHandle, name: &str, records: Vec<Vec<u8>>) -> Result<String, String> {
    if records.is_empty() {
        return Err("no packets to save".into());
    }
    let bytes = ble_pcap(&records);
    let safe = name.replace(['/', '\\', ':'], "_");
    let path = if let Some(dot) = dot_sutra(app) {
        dot.join("captures").join(format!("{safe}.pcap"))
    } else {
        match app.dialog().file().add_filter("pcap", &["pcap"]).set_file_name(format!("{safe}.pcap")).blocking_save_file() {
            Some(p) => p.into_path().map_err(|e| e.to_string())?,
            None => return Err("cancelled".into()),
        }
    };
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    // End-to-end: a crafted 802.15.4 data frame through ieee154_pcap -> rtshark.
    // Skipped when tshark isn't installed (so CI without Wireshark stays green).
    #[test]
    fn dissect_roundtrip() {
        if !tshark_available(None) {
            eprintln!("tshark not found â€” skipping dissect_roundtrip");
            return;
        }
        // record: ts(4)Â·chÂ·rssiÂ·lqiÂ·flagsÂ·plenÂ·psdu (psdu = MAC frame + 2 FCS bytes).
        // MAC: FCF 0x8841 (data, PAN-compressed, short addrs), seq 1, dst PAN abcd,
        // dst ffff (bcast), src 0000, payload, + FCS.
        let psdu: &[u8] =
            &[0x41, 0x88, 0x01, 0xcd, 0xab, 0xff, 0xff, 0x00, 0x00, 0xde, 0xad, 0x12, 0x34];
        let mut rec = vec![0u8, 0, 0, 0, 15, 0xD0, 0xFF, 0x01, psdu.len() as u8];
        rec.extend_from_slice(psdu);

        let rows = dissect_ieee154(vec![rec.clone()], None, vec![]).expect("dissect");
        assert_eq!(rows.len(), 1, "one packet");

        // A Zigbee key must be ACCEPTED by tshark (format check) even if it doesn't
        // decrypt this frame â€” a bad -o would make tshark exit non-zero -> Err.
        let key = ("5A6967426565416C6C69616E63653039".to_string(), "ZLL".to_string(), String::new());
        match dissect_ieee154(vec![rec], None, vec![key]) {
            Ok(r) => eprintln!("with-key ok: protocol={} summary={}", r[0].protocol, r[0].summary),
            Err(e) => panic!("tshark rejected the zigbee key preference: {e}"),
        }
        eprintln!("protocol={} summary={} fields={:?}", rows[0].protocol, rows[0].summary, rows[0].fields);
        assert!(
            rows[0].fields.iter().any(|(k, _)| k == "wpan.src16"),
            "extracted the wpan source address"
        );
    }
}
