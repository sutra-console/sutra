// egui yantra runtime (Path B): renders a YantraSpec in the webview; the React host
// owns the data-flow (bus + native mlua) and is authoritative. Two modes:
//  - interact: draw host-pushed values/overrides; report input via on_event.
//  - edit: an egui visual editor (multi-select, drag, resize, snap, align, undo) that
//    saves the spec back via on_save.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use eframe::egui;
use egui::{Align2, Color32, FontId, Margin, Pos2, Rect, RichText, Rounding, Sense, Stroke, Vec2};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

#[derive(Default)]
struct Shared {
    spec: Value,
    state: Value,
    theme: Value,
    on_event: Option<js_sys::Function>,
    on_save: Option<js_sys::Function>,
    theme_dirty: bool,
    editing: bool,
    selected: Vec<usize>,
    undo: Vec<Value>,
    redo: Vec<Value>,
}

thread_local! {
    static SHARED: Rc<RefCell<Shared>> = Rc::new(RefCell::new(Shared::default()));
}

#[wasm_bindgen]
pub fn start(
    canvas: web_sys::HtmlCanvasElement,
    spec_json: String,
    on_event: js_sys::Function,
    on_save: js_sys::Function,
) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let shared = SHARED.with(|s| s.clone());
    {
        let mut sh = shared.borrow_mut();
        sh.spec = serde_json::from_str(&spec_json).unwrap_or(Value::Null);
        sh.on_event = Some(on_event);
        sh.on_save = Some(on_save);
        sh.theme_dirty = true;
    }
    let app_shared = shared.clone();
    wasm_bindgen_futures::spawn_local(async move {
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |_cc| {
                    Ok(Box::new(YantraApp {
                        shared: app_shared,
                        sliders: HashMap::new(),
                        toggles: HashMap::new(),
                        colors: HashMap::new(),
                    }))
                }),
            )
            .await
            .expect("eframe start failed");
    });
    Ok(())
}

#[wasm_bindgen]
pub fn set_state(state_json: String) {
    let v: Value = serde_json::from_str(&state_json).unwrap_or(Value::Null);
    SHARED.with(|s| s.borrow_mut().state = v);
}
#[wasm_bindgen]
pub fn set_theme(theme_json: String) {
    let v: Value = serde_json::from_str(&theme_json).unwrap_or(Value::Null);
    SHARED.with(|s| {
        let mut sh = s.borrow_mut();
        sh.theme = v;
        sh.theme_dirty = true;
    });
}
#[wasm_bindgen]
pub fn set_edit(editing: bool) {
    SHARED.with(|s| {
        let mut sh = s.borrow_mut();
        sh.editing = editing;
        if !editing {
            sh.selected.clear();
        }
    });
}

struct YantraApp {
    shared: Rc<RefCell<Shared>>,
    sliders: HashMap<String, f32>,
    toggles: HashMap<String, bool>,
    colors: HashMap<String, [u8; 3]>,
}

// ---- anchor math (ported from skrit.ts) -------------------------------------
fn num(w: &Value, k: &str, d: f32) -> f32 {
    w.get(k).and_then(|v| v.as_f64()).map(|f| f as f32).unwrap_or(d)
}
fn anchor(w: &Value, k: &str, d: &str) -> String {
    w.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string()
}
fn resolve_axis(mode: &str, a: f32, b: f32, parent: f32) -> (f32, f32) {
    match mode {
        "scale" => (a / 100.0 * parent, b / 100.0 * parent),
        "center" => (parent / 2.0 + a - b / 2.0, b),
        "end" => (parent - a - b, b),
        "stretch" => (a, (parent - a - b).max(0.0)),
        _ => (a, b),
    }
}
fn store_axis(mode: &str, start: f32, size: f32, parent: f32) -> (f32, f32) {
    match mode {
        "scale" => (
            if parent != 0.0 { start / parent * 100.0 } else { 0.0 },
            if parent != 0.0 { size / parent * 100.0 } else { 0.0 },
        ),
        "center" => (start + size / 2.0 - parent / 2.0, size),
        "end" => (parent - start - size, size),
        "stretch" => (start, (parent - start - size).max(0.0)),
        _ => (start, size),
    }
}
fn r2(n: f32) -> f32 {
    (n * 100.0).round() / 100.0
}
/// Absolute px rect of a widget within a canvas of (cw, ch).
fn widget_rect(w: &Value, origin: Pos2, cw: f32, ch: f32) -> Rect {
    let a_h = anchor(w, "anchorH", "scale");
    let a_v = anchor(w, "anchorV", "start");
    let dw = if a_h == "scale" { 25.0 } else { 100.0 };
    let dh = if a_v == "scale" { 25.0 } else { 48.0 };
    let (sx, ww) = resolve_axis(&a_h, num(w, "x", 0.0), num(w, "w", dw), cw);
    let (sy, hh) = resolve_axis(&a_v, num(w, "y", 0.0), num(w, "h", dh), ch);
    Rect::from_min_size(origin + Vec2::new(sx, sy), Vec2::new(ww.max(8.0), hh.max(8.0)))
}

// ---- theme ------------------------------------------------------------------
fn parse_rgb(s: &str) -> Option<Color32> {
    let inner = s.trim().trim_start_matches("rgba").trim_start_matches("rgb");
    let inner = inner.trim_matches(|c| c == '(' || c == ')');
    let mut it = inner.split(',').map(|p| p.trim().parse::<f32>().ok());
    let r = it.next()??;
    let g = it.next()??;
    let b = it.next()??;
    Some(Color32::from_rgb(r as u8, g as u8, b as u8))
}
fn token(theme: &Value, key: &str) -> Option<Color32> {
    theme.get(key).and_then(|v| v.as_str()).and_then(parse_rgb)
}
fn apply_visuals(ctx: &egui::Context, theme: &Value) {
    let bg = token(theme, "background");
    let dark = bg.map(|c| (c.r() as u32 + c.g() as u32 + c.b() as u32) < 384).unwrap_or(true);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    if let Some(c) = bg {
        v.panel_fill = c;
    }
    if let Some(c) = token(theme, "card") {
        v.window_fill = c;
        v.extreme_bg_color = c;
        v.widgets.noninteractive.bg_fill = c;
        v.widgets.inactive.bg_fill = c;
    }
    if let Some(c) = token(theme, "foreground") {
        v.override_text_color = Some(c);
    }
    if let Some(c) = token(theme, "primary") {
        v.selection.bg_fill = c.gamma_multiply(0.35);
        v.selection.stroke.color = c;
        v.hyperlink_color = c;
    }
    if let Some(c) = token(theme, "border") {
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, c);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, c);
    }
    v.widgets.inactive.rounding = Rounding::same(6.0);
    v.widgets.hovered.rounding = Rounding::same(6.0);
    v.widgets.active.rounding = Rounding::same(6.0);
    ctx.set_visuals(v);
}

impl eframe::App for YantraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let (theme, theme_dirty, editing) = {
            let mut sh = self.shared.borrow_mut();
            let td = sh.theme_dirty;
            sh.theme_dirty = false;
            (sh.theme.clone(), td, sh.editing)
        };
        if theme_dirty {
            apply_visuals(ctx, &theme);
        }
        ctx.request_repaint_after(Duration::from_millis(50));
        if editing {
            self.edit_ui(ctx);
        } else {
            self.interact_ui(ctx);
        }
    }
}

impl YantraApp {
    // ---- interact: styled widget cards, host-authoritative values ----------
    fn interact_ui(&mut self, ctx: &egui::Context) {
        let (spec, state, on_event) = {
            let sh = self.shared.borrow();
            (sh.spec.clone(), sh.state.clone(), sh.on_event.clone())
        };
        let wstate = state.get("widgets").cloned().unwrap_or(Value::Null);
        let emit = |ev: Value| {
            if let Some(f) = &on_event {
                let _ = f.call1(&JsValue::NULL, &JsValue::from_str(&ev.to_string()));
            }
        };
        let widgets = spec.get("widgets").and_then(|w| w.as_array()).cloned().unwrap_or_default();
        let panel_fill = ctx.style().visuals.panel_fill;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(panel_fill))
            .show(ctx, |ui| {
                let canvas = ui.max_rect();
                for w in &widgets {
                    let ty = w.get("type").and_then(|t| t.as_str()).unwrap_or("").to_string();
                    let name = w.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                    let ws = wstate.get(&name).cloned().unwrap_or(Value::Null);
                    let hidden = w.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false)
                        || ws.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false);
                    if hidden {
                        continue;
                    }
                    let rect = widget_rect(w, canvas.min, canvas.width(), canvas.height());
                    let val = ws
                        .get("value")
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            Value::Null => String::new(),
                            o => o.to_string(),
                        })
                        .unwrap_or_default();
                    let label = ws
                        .get("label")
                        .and_then(|l| l.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| w.get("label").and_then(|l| l.as_str()).unwrap_or("").to_string());
                    let bg = ws.get("color").and_then(|v| v.as_str()).and_then(parse_rgb);
                    let fg = ws.get("fg").and_then(|v| v.as_str()).and_then(parse_rgb);

                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    child.set_clip_rect(rect);
                    draw_interact_widget(&mut child, rect, &ty, &name, &label, &val, bg, fg, &emit, w, self);
                }
            });
    }

    // ---- edit: multi-select, drag, resize, snap, align, undo ----------------
    fn edit_ui(&mut self, ctx: &egui::Context) {
        // keyboard: ctrl+z / ctrl+shift+z (or ctrl+y)
        let (undo_key, redo_key) = ctx.input(|i| {
            let z = i.modifiers.command && i.key_pressed(egui::Key::Z);
            (z && !i.modifiers.shift, (z && i.modifiers.shift) || (i.modifiers.command && i.key_pressed(egui::Key::Y)))
        });

        let mut add: Option<String> = None;
        let (mut do_delete, mut do_save, mut do_undo, mut do_redo) = (false, false, undo_key, redo_key);
        let mut align: Option<&str> = None;
        egui::TopBottomPanel::top("ed_toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.menu_button("➕ Add", |ui| {
                    for t in ["button", "slider", "toggle", "readout", "label", "color"] {
                        if ui.button(t).clicked() {
                            add = Some(t.to_string());
                            ui.close_menu();
                        }
                    }
                });
                if ui.button("🗑").on_hover_text("Delete selected").clicked() {
                    do_delete = true;
                }
                ui.separator();
                if ui.button("↶").on_hover_text("Undo").clicked() {
                    do_undo = true;
                }
                if ui.button("↷").on_hover_text("Redo").clicked() {
                    do_redo = true;
                }
                ui.separator();
                for (lbl, key, tip) in [
                    ("⊢", "left", "Align left"), ("｜", "cx", "Align center"), ("⊣", "right", "Align right"),
                    ("⊤", "top", "Align top"), ("－", "cy", "Align middle"), ("⊥", "bottom", "Align bottom"),
                ] {
                    if ui.button(lbl).on_hover_text(tip).clicked() {
                        align = Some(key);
                    }
                }
                ui.separator();
                if ui.button("💾 Save").clicked() {
                    do_save = true;
                }
                ui.weak("edit");
            });
        });

        // toolbar mutations
        {
            let mut sh = self.shared.borrow_mut();
            if let Some(t) = add {
                push_undo(&mut sh);
                add_widget(&mut sh.spec, &t);
                let n = sh.spec.get("widgets").and_then(|w| w.as_array()).map(|a| a.len()).unwrap_or(0);
                sh.selected = if n > 0 { vec![n - 1] } else { vec![] };
            }
            if do_delete && !sh.selected.is_empty() {
                push_undo(&mut sh);
                let mut sel = sh.selected.clone();
                sel.sort_unstable();
                for i in sel.iter().rev() {
                    delete_widget(&mut sh.spec, *i);
                }
                sh.selected.clear();
            }
            if let Some(key) = align {
                push_undo(&mut sh);
                align_selected(&mut sh, key);
            }
            if do_undo {
                undo(&mut sh);
            }
            if do_redo {
                redo(&mut sh);
            }
            if do_save {
                let s = sh.spec.to_string();
                if let Some(f) = &sh.on_save {
                    let _ = f.call1(&JsValue::NULL, &JsValue::from_str(&s));
                }
            }
        }

        let bg = ctx.style().visuals.panel_fill;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(bg))
            .show(ctx, |ui| {
                let canvas = ui.max_rect();
                draw_grid(ui, canvas);
                let (cw, ch) = (canvas.width(), canvas.height());
                let widgets = self.shared.borrow().spec.get("widgets").and_then(|w| w.as_array()).cloned().unwrap_or_default();
                let selected = self.shared.borrow().selected.clone();
                let shift = ui.input(|i| i.modifiers.shift);

                let mut click_sel: Option<(usize, bool)> = None; // (index, shift)
                let mut drag_delta: Option<Vec2> = None; // move whole selection
                let mut resize: Option<(usize, Vec2)> = None;
                let accent = ui.visuals().selection.stroke.color;

                for (i, w) in widgets.iter().enumerate() {
                    if w.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false) {
                        continue;
                    }
                    let rect = widget_rect(w, canvas.min, cw, ch);
                    let is_sel = selected.contains(&i);
                    let id = egui::Id::new(("ed", i));
                    let resp = ui.interact(rect, id, Sense::click_and_drag());
                    if resp.clicked() {
                        click_sel = Some((i, shift));
                    }
                    if resp.drag_started() && !is_sel {
                        click_sel = Some((i, shift));
                    }
                    if resp.dragged() && (is_sel || click_sel.map(|c| c.0 == i).unwrap_or(false)) {
                        let mut d = resp.drag_delta();
                        if shift {
                            d = Vec2::new((d.x / 8.0).round() * 8.0, (d.y / 8.0).round() * 8.0);
                        }
                        drag_delta = Some(d);
                    }
                    // box
                    let stroke = if is_sel { Stroke::new(2.0, accent) } else { Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color) };
                    ui.painter().rect(rect, Rounding::same(5.0), ui.visuals().faint_bg_color, stroke);
                    let lbl = w
                        .get("label")
                        .and_then(|l| l.as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| w.get("type").and_then(|t| t.as_str()).unwrap_or("?").to_string());
                    ui.painter().text(rect.center(), Align2::CENTER_CENTER, lbl, FontId::proportional(11.0), ui.visuals().text_color());
                    // resize handle on a single selection
                    if is_sel && selected.len() == 1 {
                        let h = Rect::from_min_size(rect.max - Vec2::splat(11.0), Vec2::splat(11.0));
                        let hr = ui.interact(h, id.with("rs"), Sense::drag());
                        ui.painter().rect_filled(h, Rounding::same(2.0), accent);
                        if hr.dragged() {
                            let mut d = hr.drag_delta();
                            if shift {
                                d = Vec2::new((d.x / 8.0).round() * 8.0, (d.y / 8.0).round() * 8.0);
                            }
                            resize = Some((i, d));
                        }
                    }
                }
                // empty-canvas click clears selection
                let bg_resp = ui.interact(canvas, egui::Id::new("ed_bg"), Sense::click());
                let clicked_empty = bg_resp.clicked() && click_sel.is_none();

                if click_sel.is_some() || drag_delta.is_some() || resize.is_some() || clicked_empty {
                    let mut sh = self.shared.borrow_mut();
                    if let Some((i, sh_held)) = click_sel {
                        if sh_held {
                            if let Some(p) = sh.selected.iter().position(|x| *x == i) {
                                sh.selected.remove(p);
                            } else {
                                sh.selected.push(i);
                            }
                        } else if !sh.selected.contains(&i) {
                            sh.selected = vec![i];
                        }
                    } else if clicked_empty {
                        sh.selected.clear();
                    }
                    if let Some(d) = drag_delta {
                        push_undo(&mut sh);
                        let sel = sh.selected.clone();
                        for i in sel {
                            move_widget(&mut sh.spec, i, d, cw, ch);
                        }
                    }
                    if let Some((i, d)) = resize {
                        push_undo(&mut sh);
                        resize_widget(&mut sh.spec, i, d, cw, ch);
                    }
                }
            });
    }
}

// ---- edit helpers -----------------------------------------------------------
fn draw_grid(ui: &egui::Ui, canvas: Rect) {
    let col = ui.visuals().widgets.noninteractive.bg_stroke.color.gamma_multiply(0.25);
    let step = 24.0;
    let mut x = canvas.min.x;
    while x < canvas.max.x {
        ui.painter().line_segment([Pos2::new(x, canvas.min.y), Pos2::new(x, canvas.max.y)], Stroke::new(1.0, col));
        x += step;
    }
    let mut y = canvas.min.y;
    while y < canvas.max.y {
        ui.painter().line_segment([Pos2::new(canvas.min.x, y), Pos2::new(canvas.max.x, y)], Stroke::new(1.0, col));
        y += step;
    }
}
fn push_undo(sh: &mut Shared) {
    sh.undo.push(sh.spec.clone());
    if sh.undo.len() > 100 {
        sh.undo.remove(0);
    }
    sh.redo.clear();
}
fn undo(sh: &mut Shared) {
    if let Some(prev) = sh.undo.pop() {
        sh.redo.push(sh.spec.clone());
        sh.spec = prev;
        sh.selected.clear();
    }
}
fn redo(sh: &mut Shared) {
    if let Some(next) = sh.redo.pop() {
        sh.undo.push(sh.spec.clone());
        sh.spec = next;
        sh.selected.clear();
    }
}
fn add_widget(spec: &mut Value, ty: &str) {
    if !spec.is_object() {
        *spec = json!({});
    }
    let arr = spec.as_object_mut().unwrap().entry("widgets").or_insert(json!([]));
    if let Some(a) = arr.as_array_mut() {
        let n = a.iter().filter(|w| w.get("type").and_then(|t| t.as_str()) == Some(ty)).count() + 1;
        a.push(json!({
            "type": ty, "name": format!("{ty}{n}"), "label": ty,
            "x": 4, "y": 8, "w": 30, "h": 48, "anchorH": "scale", "anchorV": "start"
        }));
    }
}
fn delete_widget(spec: &mut Value, i: usize) {
    if let Some(a) = spec.get_mut("widgets").and_then(|w| w.as_array_mut()) {
        if i < a.len() {
            a.remove(i);
        }
    }
}
fn move_widget(spec: &mut Value, i: usize, d: Vec2, cw: f32, ch: f32) {
    let Some(w) = spec.get_mut("widgets").and_then(|x| x.as_array_mut()).and_then(|a| a.get_mut(i)) else { return };
    let a_h = anchor(w, "anchorH", "scale");
    let a_v = anchor(w, "anchorV", "start");
    let dw = if a_h == "scale" { 25.0 } else { 100.0 };
    let dh = if a_v == "scale" { 25.0 } else { 48.0 };
    let (sx, ww) = resolve_axis(&a_h, num(w, "x", 0.0), num(w, "w", dw), cw);
    let (sy, hh) = resolve_axis(&a_v, num(w, "y", 0.0), num(w, "h", dh), ch);
    let (a, b) = store_axis(&a_h, sx + d.x, ww, cw);
    let (c, e) = store_axis(&a_v, sy + d.y, hh, ch);
    w["x"] = json!(r2(a));
    w["w"] = json!(r2(b));
    w["y"] = json!(r2(c));
    w["h"] = json!(r2(e));
}
fn resize_widget(spec: &mut Value, i: usize, d: Vec2, cw: f32, ch: f32) {
    let Some(w) = spec.get_mut("widgets").and_then(|x| x.as_array_mut()).and_then(|a| a.get_mut(i)) else { return };
    let a_h = anchor(w, "anchorH", "scale");
    let a_v = anchor(w, "anchorV", "start");
    let dw = if a_h == "scale" { 25.0 } else { 100.0 };
    let dh = if a_v == "scale" { 25.0 } else { 48.0 };
    let (sx, ww) = resolve_axis(&a_h, num(w, "x", 0.0), num(w, "w", dw), cw);
    let (sy, hh) = resolve_axis(&a_v, num(w, "y", 0.0), num(w, "h", dh), ch);
    let (a, b) = store_axis(&a_h, sx, (ww + d.x).max(8.0), cw);
    let (c, e) = store_axis(&a_v, sy, (hh + d.y).max(8.0), ch);
    w["x"] = json!(r2(a));
    w["w"] = json!(r2(b));
    w["y"] = json!(r2(c));
    w["h"] = json!(r2(e));
}
/// Align the selection (in stored units, like the React editor's align).
fn align_selected(sh: &mut Shared, key: &str) {
    let sel = sh.selected.clone();
    if sel.len() < 2 {
        return;
    }
    let Some(arr) = sh.spec.get_mut("widgets").and_then(|w| w.as_array_mut()) else { return };
    // operate in stored x/y/w/h (close enough; assumes a shared anchor family)
    let xs: Vec<(f32, f32)> = sel.iter().filter_map(|&i| arr.get(i)).map(|w| (num(w, "x", 0.0), num(w, "w", 30.0))).collect();
    let ys: Vec<(f32, f32)> = sel.iter().filter_map(|&i| arr.get(i)).map(|w| (num(w, "y", 0.0), num(w, "h", 48.0))).collect();
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

// ---- interact widget drawing (styled cards) ---------------------------------
#[allow(clippy::too_many_arguments)]
fn draw_interact_widget(
    ui: &mut egui::Ui,
    rect: Rect,
    ty: &str,
    name: &str,
    label: &str,
    val: &str,
    bg: Option<Color32>,
    fg: Option<Color32>,
    emit: &dyn Fn(Value),
    w: &Value,
    app: &mut YantraApp,
) {
    let muted = ui.visuals().weak_text_color();
    let card = bg.unwrap_or(ui.visuals().widgets.noninteractive.bg_fill);
    let border = ui.visuals().widgets.noninteractive.bg_stroke.color;
    let frame = egui::Frame::none()
        .fill(card)
        .stroke(Stroke::new(1.0, border))
        .rounding(Rounding::same(6.0))
        .inner_margin(Margin::same(6.0));
    let _ = rect;
    frame.show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        if let Some(c) = fg {
            ui.visuals_mut().override_text_color = Some(c);
        }
        match ty {
            "label" => {
                ui.label(RichText::new(if val.is_empty() { label } else { val }).strong());
            }
            "readout" => {
                ui.vertical(|ui| {
                    ui.label(RichText::new(label).size(10.0).color(muted));
                    ui.label(RichText::new(if val.is_empty() { "—" } else { val }).size(18.0).monospace());
                });
            }
            "button" => {
                if ui.add_sized(ui.available_size(), egui::Button::new(label)).clicked() {
                    emit(json!({ "kind": "press", "name": name }));
                }
            }
            "slider" => {
                let init = num(w, "value", num(w, "min", 0.0));
                let v = app.sliders.entry(name.to_string()).or_insert(init);
                let (min, max) = (num(w, "min", 0.0), num(w, "max", 100.0));
                ui.label(RichText::new(label).size(10.0).color(muted));
                if ui.add(egui::Slider::new(v, min..=max)).changed() {
                    emit(json!({ "kind": "value", "name": name, "value": *v }));
                }
            }
            "toggle" => {
                let on = app.toggles.entry(name.to_string()).or_insert(false);
                if ui.add_sized(ui.available_size(), egui::SelectableLabel::new(*on, format!("{label}: {}", if *on { "on" } else { "off" }))).clicked() {
                    *on = !*on;
                    emit(json!({ "kind": "value", "name": name, "value": *on }));
                }
            }
            "color" => {
                let c = app.colors.entry(name.to_string()).or_insert([128, 128, 128]);
                ui.horizontal(|ui| {
                    if ui.color_edit_button_srgb(c).changed() {
                        emit(json!({ "kind": "value", "name": name, "value": format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]) }));
                    }
                    if !label.is_empty() {
                        ui.label(RichText::new(label).size(10.0).color(muted));
                    }
                });
            }
            other => {
                ui.weak(format!("{other}?"));
            }
        }
    });
}
