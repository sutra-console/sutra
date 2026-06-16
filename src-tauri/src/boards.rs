//! Local board definitions. The Board view's pin maps ship as built-ins in the
//! frontend; this exposes a local override/extension folder
//! (<app_data>/boards/*.json) so boards can be added or overridden without a
//! rebuild — and, later, a board-defs server can pull JSON into the same folder.
//! Each file is a board def: { id, name, vendor, model, mcu, brokenOut[] |
//! brokenOutAll, uses[], roles[], i2c{sda,scl} } (see src/lib/boards.ts).
use std::path::PathBuf;

use tauri::{AppHandle, Manager};

/// <app_data>/boards/, created on demand. Global (not per-workspace) — boards are
/// about hardware, reusable across workspaces.
fn boards_dir(app: &AppHandle) -> Option<PathBuf> {
    let d = app.path().app_data_dir().ok()?.join("boards");
    let _ = std::fs::create_dir_all(&d);
    Some(d)
}

/// Every board-def JSON in the boards folder (raw values; the frontend merges
/// them over its built-ins by id). Empty by default; invalid files are skipped.
pub fn list_boards(app: &AppHandle) -> Vec<serde_json::Value> {
    let Some(dir) = boards_dir(app) else {
        return Vec::new();
    };
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
