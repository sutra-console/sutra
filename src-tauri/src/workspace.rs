//! Workspace: a user-chosen folder holding a `.sutra/` directory where Sutra
//! keeps its per-project state — macros (`macros.json`) and capture exports
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
    let _ = std::fs::create_dir_all(path.join(".sutra").join("captures"));
    seed_i2c_example(&path.join(".sutra").join("i2c"));
    Ok(path.to_path_buf())
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
/// (each: ts(4)·ch(1)·rssi(1)·aa(4)·len(1)·pdu). One LL packet per record.
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
        data.extend_from_slice(aa); // LL packet: access address …
        data.extend_from_slice(pdu); //            … + PDU
        pcap_record(&mut out, ts, &data);
    }
    out
}

const LINKTYPE_IEEE802_15_4_TAP: u32 = 283;

/// Build a pcap (LINKTYPE_IEEE802_15_4_TAP) from raw ieee802154 records
/// (each: ts(4)·ch(1)·rssi(1,signed)·lqi(1)·flags(1)·len(1)·psdu). One TAP
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
// Per-workspace secrets that unlock a network — saved beside the captures they
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
        .map(|d| d.join("keys.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the workspace key store to `<ws>/.sutra/keys.json`.
pub fn save_keys(app: &AppHandle, keys: WorkspaceKeys) -> Result<(), String> {
    let path = dot_sutra(app).ok_or("no workspace selected")?.join("keys.json");
    let json = serde_json::to_string_pretty(&keys).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

// ---- tshark dissection -----------------------------------------------------
// Thread / Zigbee / Matter are standard, fully-dissected protocols — we don't
// reimplement them. Instead we shell out to Wireshark's `tshark` as a dissection
// engine: build the same LINKTYPE_IEEE802_15_4_TAP pcap, let tshark decode the
// whole stack, and consume the per-packet Protocol/Info columns.

/// Resolve the tshark binary: an explicit override (a file or its directory)
/// from Sutra's settings, else autodetect — PATH, then the usual install dirs.
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
    pub protocol: String,              // Wireshark's Protocol column (ZigBee, Thread, …)
    pub summary: String,               // Wireshark's Info column — the "what happened" line
    pub fields: Vec<(String, String)>, // (name, value) — the decoded field tree, drill-down
}

/// Fields surfaced on drill-down. What's visible depends on the kind — and on
/// whether the payload is encrypted (most Zigbee is, so only the NWK header shows
/// unless a network key is configured).
const FIELD_WHITELIST: &[&str] = &[
    "wpan.src16", "wpan.dst16", "wpan.dst_pan",
    "zbee_nwk.src", "zbee_nwk.dst", "zbee_nwk.radius", "zbee_nwk.seqno", "zbee_nwk.cmd",
    "zbee_aps.cluster", "zbee_aps.profile", "zbee_aps.src", "zbee_aps.dst", "zbee_aps.cmd",
    "zbee_zcl.cmd.id", "zbee_zcl.attr.id", "coap.code", "coap.mid", "mle.cmd",
];

/// A Zigbee key as a tshark `uat:zigbee_pc_keys` preference (Key, Byte Order,
/// Label). Feeding it lets tshark decrypt NWK/APS so the summary/fields climb from
/// "Command" to the real ZCL command/cluster.
fn zigbee_key_pref(key: &str, label: &str) -> String {
    let label = label.replace(['"', ','], " ");
    format!("uat:zigbee_pc_keys:\"{}\",\"Normal\",\"{}\"", key.trim(), label)
}

/// Whether tshark is reachable (gates/loads the Decode action in the UI).
pub fn tshark_available(tshark_path: Option<String>) -> bool {
    resolve_tshark(tshark_path.as_deref()).is_some()
}

/// tshark's column summary per packet (frame num, Protocol, Info) — the rendered
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
    keys: Vec<(String, String)>,
) -> Result<Vec<DecodedRow>, String> {
    if records.is_empty() {
        return Ok(vec![]);
    }
    let bin = resolve_tshark(tshark_path.as_deref())
        .ok_or("Wireshark (tshark) not found — set its path in Settings")?;
    let dir = bin.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();

    let bytes = ieee154_pcap(&records);
    let tmp = std::env::temp_dir().join("sutra_dissect.pcap");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    let tmp_str = tmp.to_string_lossy().into_owned();

    let key_prefs: Vec<String> = keys
        .iter()
        .filter(|(k, _)| !k.trim().is_empty())
        .map(|(k, l)| zigbee_key_pref(k, l))
        .collect();

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

#[cfg(test)]
mod tests {
    use super::*;

    // End-to-end: a crafted 802.15.4 data frame through ieee154_pcap -> rtshark.
    // Skipped when tshark isn't installed (so CI without Wireshark stays green).
    #[test]
    fn dissect_roundtrip() {
        if !tshark_available(None) {
            eprintln!("tshark not found — skipping dissect_roundtrip");
            return;
        }
        // record: ts(4)·ch·rssi·lqi·flags·plen·psdu (psdu = MAC frame + 2 FCS bytes).
        // MAC: FCF 0x8841 (data, PAN-compressed, short addrs), seq 1, dst PAN abcd,
        // dst ffff (bcast), src 0000, payload, + FCS.
        let psdu: &[u8] =
            &[0x41, 0x88, 0x01, 0xcd, 0xab, 0xff, 0xff, 0x00, 0x00, 0xde, 0xad, 0x12, 0x34];
        let mut rec = vec![0u8, 0, 0, 0, 15, 0xD0, 0xFF, 0x01, psdu.len() as u8];
        rec.extend_from_slice(psdu);

        let rows = dissect_ieee154(vec![rec.clone()], None, vec![]).expect("dissect");
        assert_eq!(rows.len(), 1, "one packet");

        // A Zigbee key must be ACCEPTED by tshark (format check) even if it doesn't
        // decrypt this frame — a bad -o would make tshark exit non-zero -> Err.
        let key = ("5A6967426565416C6C69616E63653039".to_string(), "ZLL".to_string());
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
