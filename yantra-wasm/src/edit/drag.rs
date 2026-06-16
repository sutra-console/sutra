use std::collections::HashMap;

use egui::{Pos2, Rect, Vec2};
use serde_json::{json, Value};

use crate::geometry::{anchor, num, r2, resolve_axis, store_axis};
use crate::state::{Drag, DragItem, Sel};
pub(super) fn group_into_frame(
    spec: &mut Value,
    sel: &[usize],
    abs: &HashMap<usize, Rect>,
    canvas: Rect,
) -> Option<usize> {
    let rects: Vec<Rect> = sel.iter().filter_map(|i| abs.get(i).copied()).collect();
    if rects.is_empty() {
        return None;
    }
    let mut bbox = rects[0];
    for r in &rects[1..] {
        bbox = bbox.union(*r);
    }
    let bbox = bbox.expand(6.0);
    if !spec.is_object() {
        *spec = json!({});
    }
    let n = spec
        .get("frames")
        .and_then(|f| f.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
        + 1;
    let id = format!("group{n}");
    let (fx, fw) = store_axis(
        "scale",
        bbox.min.x - canvas.min.x,
        bbox.width(),
        canvas.width(),
    );
    let (fy, fh) = store_axis(
        "scale",
        bbox.min.y - canvas.min.y,
        bbox.height(),
        canvas.height(),
    );
    let frame = json!({
        "id": id.clone(), "name": id.clone(),
        "x": r2(fx), "y": r2(fy), "w": r2(fw), "h": r2(fh),
        "anchorH": "scale", "anchorV": "scale", "clip": true
    });
    spec.as_object_mut()
        .unwrap()
        .entry("frames")
        .or_insert(json!([]))
        .as_array_mut()
        .unwrap()
        .push(frame);
    let fidx = n - 1;
    // reparent: each widget's coords become relative to the frame bbox
    for &i in sel {
        let Some(r) = abs.get(&i).copied() else {
            continue;
        };
        let Some(w) = spec
            .get_mut("widgets")
            .and_then(|a| a.as_array_mut())
            .and_then(|a| a.get_mut(i))
        else {
            continue;
        };
        let ah = anchor(w, "anchorH", "scale");
        let av = anchor(w, "anchorV", "start");
        let (x, ww) = store_axis(&ah, r.min.x - bbox.min.x, r.width(), bbox.width());
        let (y, hh) = store_axis(&av, r.min.y - bbox.min.y, r.height(), bbox.height());
        w["x"] = json!(r2(x));
        w["w"] = json!(r2(ww));
        w["y"] = json!(r2(y));
        w["h"] = json!(r2(hh));
        w["frame"] = json!(id);
        if let Some(o) = w.as_object_mut() {
            o.remove("tab");
        }
    }
    Some(fidx)
}

pub(super) fn capture_drag(
    resize: bool,
    start: Pos2,
    spec: &Value,
    items_sel: &[Sel],
    wparent: &HashMap<usize, Rect>,
    fparent: &HashMap<usize, Rect>,
) -> Drag {
    let mut items = Vec::new();
    for s in items_sel {
        let (frames, idx, parent_of, key) = match s {
            Sel::Widget(i) => (false, *i, wparent, "widgets"),
            Sel::Frame(fi) => (true, *fi, fparent, "frames"),
        };
        let Some(pr) = parent_of.get(&idx) else {
            continue;
        };
        let Some(w) = spec
            .get(key)
            .and_then(|a| a.as_array())
            .and_then(|a| a.get(idx))
        else {
            continue;
        };
        let ah = anchor(w, "anchorH", "scale");
        let av = anchor(w, "anchorV", if frames { "scale" } else { "start" });
        let dw = if ah == "scale" { 25.0 } else { 100.0 };
        let dh = if av == "scale" { 25.0 } else { 48.0 };
        let (sx, ww) = resolve_axis(&ah, num(w, "x", 0.0), num(w, "w", dw), pr.width());
        let (sy, hh) = resolve_axis(&av, num(w, "y", 0.0), num(w, "h", dh), pr.height());
        items.push(DragItem {
            idx,
            frames,
            ah,
            av,
            sx: pr.min.x + sx, // absolute
            sy: pr.min.y + sy,
            w: ww,
            h: hh,
            px: pr.min.x,
            py: pr.min.y,
            pw: pr.width(),
            ph: pr.height(),
        });
    }
    Drag {
        resize,
        start,
        items,
    }
}
fn snap(v: f32, on: bool) -> f32 {
    if on {
        (v / 8.0).round() * 8.0
    } else {
        v
    }
}
/// Apply the total pointer delta to the captured drag, writing stored (parent-relative) units back.
pub(super) fn apply_drag(spec: &mut Value, drag: &Drag, total: Vec2, snap_on: bool) {
    for it in &drag.items {
        let key = if it.frames { "frames" } else { "widgets" };
        let Some(w) = spec
            .get_mut(key)
            .and_then(|a| a.as_array_mut())
            .and_then(|a| a.get_mut(it.idx))
        else {
            continue;
        };
        if drag.resize {
            let nw = snap((it.w + total.x).max(8.0), snap_on);
            let nh = snap((it.h + total.y).max(8.0), snap_on);
            let (x, ww) = store_axis(&it.ah, it.sx - it.px, nw, it.pw);
            let (y, hh) = store_axis(&it.av, it.sy - it.py, nh, it.ph);
            w["x"] = json!(r2(x));
            w["w"] = json!(r2(ww));
            w["y"] = json!(r2(y));
            w["h"] = json!(r2(hh));
        } else {
            let nax = snap(it.sx + total.x, snap_on);
            let nay = snap(it.sy + total.y, snap_on);
            let (x, ww) = store_axis(&it.ah, nax - it.px, it.w, it.pw);
            let (y, hh) = store_axis(&it.av, nay - it.py, it.h, it.ph);
            w["x"] = json!(r2(x));
            w["w"] = json!(r2(ww));
            w["y"] = json!(r2(y));
            w["h"] = json!(r2(hh));
        }
    }
}
