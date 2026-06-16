use eframe::egui;
use egui::{Pos2, Rect, Stroke};
use serde_json::{json, Value};

use crate::geometry::{num, r2};
use crate::state::Shared;

use super::selection::sel_widgets;
// ---- edit helpers -----------------------------------------------------------
pub(super) fn draw_grid(ui: &egui::Ui, canvas: Rect) {
    let col = ui
        .visuals()
        .widgets
        .noninteractive
        .bg_stroke
        .color
        .gamma_multiply(0.25);
    let step = 24.0;
    let mut x = canvas.min.x;
    while x < canvas.max.x {
        ui.painter().line_segment(
            [Pos2::new(x, canvas.min.y), Pos2::new(x, canvas.max.y)],
            Stroke::new(1.0, col),
        );
        x += step;
    }
    let mut y = canvas.min.y;
    while y < canvas.max.y {
        ui.painter().line_segment(
            [Pos2::new(canvas.min.x, y), Pos2::new(canvas.max.x, y)],
            Stroke::new(1.0, col),
        );
        y += step;
    }
}
pub(super) fn push_undo(sh: &mut Shared) {
    if sh.undo.last() == Some(&sh.spec) {
        return; // coalesce identical snapshots (e.g. focus without an edit)
    }
    sh.undo.push(sh.spec.clone());
    if sh.undo.len() > 100 {
        sh.undo.remove(0);
    }
    sh.redo.clear();
}
pub(super) fn undo(sh: &mut Shared) {
    if let Some(prev) = sh.undo.pop() {
        sh.redo.push(sh.spec.clone());
        sh.spec = prev;
        sh.selected.clear();
    }
}
pub(super) fn redo(sh: &mut Shared) {
    if let Some(next) = sh.redo.pop() {
        sh.undo.push(sh.spec.clone());
        sh.spec = next;
        sh.selected.clear();
    }
}
pub(super) fn add_widget(spec: &mut Value, ty: &str) {
    if !spec.is_object() {
        *spec = json!({});
    }
    let arr = spec
        .as_object_mut()
        .unwrap()
        .entry("widgets")
        .or_insert(json!([]));
    if let Some(a) = arr.as_array_mut() {
        let n = a
            .iter()
            .filter(|w| w.get("type").and_then(|t| t.as_str()) == Some(ty))
            .count()
            + 1;
        let mut w = json!({
            "type": ty, "name": format!("{ty}{n}"), "label": ty,
            "x": 4, "y": 8, "w": 30, "h": 48, "anchorH": "scale", "anchorV": "start"
        });
        if ty == "select" {
            // a starter selector: two options, no actions wired yet
            w["options"] = json!([{ "label": "A" }, { "label": "B" }]);
        }
        a.push(w);
    }
}
pub(super) fn add_frame(spec: &mut Value) {
    if !spec.is_object() {
        *spec = json!({});
    }
    let arr = spec
        .as_object_mut()
        .unwrap()
        .entry("frames")
        .or_insert(json!([]));
    if let Some(a) = arr.as_array_mut() {
        let n = a.len() + 1;
        a.push(json!({
            "id": format!("frame{n}"), "name": format!("frame{n}"),
            "x": 4, "y": 8, "w": 40, "h": 40, "anchorH": "scale", "anchorV": "scale", "clip": true
        }));
    }
}
pub(super) fn delete_widget(spec: &mut Value, i: usize) {
    if let Some(a) = spec.get_mut("widgets").and_then(|w| w.as_array_mut()) {
        if i < a.len() {
            a.remove(i);
        }
    }
}
/// Delete a frame, reparenting its direct children (widgets + child frames) to root
/// so they don't vanish (a dangling `frame`/`parent` id renders nowhere).
pub(super) fn delete_frame(spec: &mut Value, fi: usize) {
    let id = spec
        .get("frames")
        .and_then(|a| a.as_array())
        .and_then(|a| a.get(fi))
        .and_then(|f| f.get("id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if let Some(a) = spec.get_mut("frames").and_then(|f| f.as_array_mut()) {
        if fi < a.len() {
            a.remove(fi);
        }
    }
    let Some(id) = id else { return };
    if let Some(ws) = spec.get_mut("widgets").and_then(|w| w.as_array_mut()) {
        for w in ws.iter_mut() {
            if w.get("frame").and_then(|v| v.as_str()) == Some(id.as_str()) {
                if let Some(o) = w.as_object_mut() {
                    o.remove("frame");
                }
            }
        }
    }
    if let Some(fs) = spec.get_mut("frames").and_then(|f| f.as_array_mut()) {
        for f in fs.iter_mut() {
            if f.get("parent").and_then(|v| v.as_str()) == Some(id.as_str()) {
                if let Some(o) = f.as_object_mut() {
                    o.remove("parent");
                }
            }
        }
    }
}
/// Align the selection (in stored units, like the React editor's align).
pub(super) fn align_selected(sh: &mut Shared, key: &str) {
    let sel = sel_widgets(&sh.selected);
    if sel.len() < 2 {
        return;
    }
    let Some(arr) = sh.spec.get_mut("widgets").and_then(|w| w.as_array_mut()) else {
        return;
    };
    // operate in stored x/y/w/h (close enough; assumes a shared anchor family)
    let xs: Vec<(f32, f32)> = sel
        .iter()
        .filter_map(|&i| arr.get(i))
        .map(|w| (num(w, "x", 0.0), num(w, "w", 30.0)))
        .collect();
    let ys: Vec<(f32, f32)> = sel
        .iter()
        .filter_map(|&i| arr.get(i))
        .map(|w| (num(w, "y", 0.0), num(w, "h", 48.0)))
        .collect();
    let min_x = xs.iter().map(|t| t.0).fold(f32::MAX, f32::min);
    let max_r = xs.iter().map(|t| t.0 + t.1).fold(f32::MIN, f32::max);
    let cx = (min_x + max_r) / 2.0;
    let min_y = ys.iter().map(|t| t.0).fold(f32::MAX, f32::min);
    let max_b = ys.iter().map(|t| t.0 + t.1).fold(f32::MIN, f32::max);
    let cy = (min_y + max_b) / 2.0;
    for &i in &sel {
        if let Some(w) = arr.get_mut(i) {
            let ww = num(w, "w", 30.0);
            let hh = num(w, "h", 48.0);
            match key {
                "left" => w["x"] = json!(r2(min_x)),
                "cx" => w["x"] = json!(r2(cx - ww / 2.0)),
                "right" => w["x"] = json!(r2(max_r - ww)),
                "top" => w["y"] = json!(r2(min_y)),
                "cy" => w["y"] = json!(r2(cy - hh / 2.0)),
                "bottom" => w["y"] = json!(r2(max_b - hh)),
                _ => {}
            }
        }
    }
}
