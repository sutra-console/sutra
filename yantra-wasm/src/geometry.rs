use egui::{Pos2, Rect, Vec2};
use serde_json::Value;

pub(crate) fn num(w: &Value, k: &str, d: f32) -> f32 {
    w.get(k)
        .and_then(|v| v.as_f64())
        .map(|f| f as f32)
        .unwrap_or(d)
}
pub(crate) fn anchor(w: &Value, k: &str, d: &str) -> String {
    w.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string()
}
pub(crate) fn resolve_axis(mode: &str, a: f32, b: f32, parent: f32) -> (f32, f32) {
    match mode {
        "scale" => (a / 100.0 * parent, b / 100.0 * parent),
        "center" => (parent / 2.0 + a - b / 2.0, b),
        "end" => (parent - a - b, b),
        "stretch" => (a, (parent - a - b).max(0.0)),
        _ => (a, b),
    }
}
pub(crate) fn store_axis(mode: &str, start: f32, size: f32, parent: f32) -> (f32, f32) {
    match mode {
        "scale" => (
            if parent != 0.0 {
                start / parent * 100.0
            } else {
                0.0
            },
            if parent != 0.0 {
                size / parent * 100.0
            } else {
                0.0
            },
        ),
        "center" => (start + size / 2.0 - parent / 2.0, size),
        "end" => (parent - start - size, size),
        "stretch" => (start, (parent - start - size).max(0.0)),
        _ => (start, size),
    }
}
pub(crate) fn r2(n: f32) -> f32 {
    (n * 100.0).round() / 100.0
}
/// Absolute px rect of a widget within a canvas of (cw, ch).
pub(crate) fn widget_rect(w: &Value, origin: Pos2, cw: f32, ch: f32) -> Rect {
    let a_h = anchor(w, "anchorH", "scale");
    let a_v = anchor(w, "anchorV", "start");
    let dw = if a_h == "scale" { 25.0 } else { 100.0 };
    let dh = if a_v == "scale" { 25.0 } else { 48.0 };
    let (sx, ww) = resolve_axis(&a_h, num(w, "x", 0.0), num(w, "w", dw), cw);
    let (sy, hh) = resolve_axis(&a_v, num(w, "y", 0.0), num(w, "h", dh), ch);
    Rect::from_min_size(
        origin + Vec2::new(sx, sy),
        Vec2::new(ww.max(8.0), hh.max(8.0)),
    )
}
