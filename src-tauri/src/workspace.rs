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
    Ok(path.to_path_buf())
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
