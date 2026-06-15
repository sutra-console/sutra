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
    drag: Option<Drag>,
    marquee: Option<Pos2>, // rubber-band select origin (canvas px)
    selected_frame: Option<usize>, // index into spec.frames (frames select one at a time)
    pending_group: bool, // layers context-menu "Group into Frame" → applied in the canvas pass
}

/// An in-progress move/resize. Captured once at drag-start so we map the *absolute*
/// pointer delta onto the original geometry — pointer-following, never accumulating.
#[derive(Clone)]
struct Drag {
    resize: bool, // false = move whole selection, true = resize the single item
    frames: bool, // target spec.frames instead of spec.widgets
    start: Pos2,  // pointer pos at drag start
    items: Vec<DragItem>,
}
#[derive(Clone)]
struct DragItem {
    idx: usize,
    ah: String,
    av: String,
    sx: f32, // original resolved absolute px geometry
    sy: f32,
    w: f32,
    h: f32,
    px: f32, // parent container origin + size (for nested store_axis)
    py: f32,
    pw: f32,
    ph: f32,
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
                        tabs: HashMap::new(),
                        selects: HashMap::new(),
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
    tabs: HashMap<String, String>, // tabs-widget key → active tab id (legacy widget)
    selects: HashMap<String, usize>, // select-widget key → active option index
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
        let fstate = state.get("frames").cloned().unwrap_or(Value::Null); // per-frame overrides (hidden)
        let emit = |ev: Value| {
            if let Some(f) = &on_event {
                let _ = f.call1(&JsValue::NULL, &JsValue::from_str(&ev.to_string()));
            }
        };
        let widgets = spec.get("widgets").and_then(|w| w.as_array()).cloned().unwrap_or_default();
        let frames = spec.get("frames").and_then(|f| f.as_array()).cloned().unwrap_or_default();
        let panel_fill = ctx.style().visuals.panel_fill;
        let mut root = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(panel_fill))
            .show(ctx, |ui| {
                root = Some((ui.max_rect(), ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()))));
            });
        // recurse outside the panel closure so `self` is free to be borrowed mutably
        if let Some((canvas, mut ui)) = root {
            self.render_container(&mut ui, "root", canvas, &widgets, &frames, &wstate, &fstate, &emit);
        }
    }

    /// Recursively render a container ("root" | frame id | tab-pane id) and its
    /// child frames + widgets, relative to `rect`. Mirrors React's CanvasNodes.
    #[allow(clippy::too_many_arguments)]
    fn render_container(
        &mut self,
        ui: &mut egui::Ui,
        container: &str,
        rect: Rect,
        widgets: &[Value],
        frames: &[Value],
        wstate: &Value,
        fstate: &Value,
        emit: &dyn Fn(Value),
    ) {
        let is_root = container == "root";
        // child frames first (so widgets paint on top within this container)
        for f in frames {
            let id = f.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let parent = f.get("parent").and_then(|v| v.as_str());
            let tab = f.get("tab").and_then(|v| v.as_str());
            let is_child = if is_root {
                parent.is_none() && tab.is_none()
            } else {
                tab == Some(container) || (parent == Some(container) && tab.is_none())
            };
            if !is_child {
                continue;
            }
            // frame visibility: static `hidden` OR a host/script override
            let fhidden = f.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false)
                || fstate.get(id).and_then(|s| s.get("hidden")).and_then(|h| h.as_bool()).unwrap_or(false);
            if fhidden {
                continue;
            }
            let fr = widget_rect(f, rect.min, rect.width(), rect.height());
            ui.painter().rect(
                fr,
                Rounding::same(6.0),
                Color32::TRANSPARENT,
                Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color.gamma_multiply(0.6)),
            );
            let clip = f.get("clip").and_then(|c| c.as_bool()).unwrap_or(true);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(fr));
            if clip {
                child.set_clip_rect(fr);
            }
            self.render_container(&mut child, id, fr, widgets, frames, wstate, fstate, emit);
        }
        // child widgets
        for (i, w) in widgets.iter().enumerate() {
            let name = w.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let ws = wstate.get(name).cloned().unwrap_or(Value::Null);
            let hidden = w.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false)
                || ws.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false);
            if hidden {
                continue;
            }
            let frame = w.get("frame").and_then(|v| v.as_str());
            let tab = w.get("tab").and_then(|v| v.as_str());
            let is_child = if is_root {
                frame.is_none() && tab.is_none()
            } else {
                tab == Some(container) || (frame == Some(container) && tab.is_none())
            };
            if !is_child {
                continue;
            }
            let wr = widget_rect(w, rect.min, rect.width(), rect.height());
            if w.get("type").and_then(|t| t.as_str()) == Some("tabs") {
                self.render_tabs(ui, i, w, wr, widgets, frames, wstate, fstate, emit);
                continue;
            }
            let (ty, _n, label, val, bg, fg) = display_of(w, wstate);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(wr));
            child.set_clip_rect(wr);
            draw_interact_widget(&mut child, wr, &ty, name, &label, &val, bg, fg, emit, w, self);
        }
    }

    /// A `tabs` widget: a card with a tab bar over the active pane's content.
    #[allow(clippy::too_many_arguments)]
    fn render_tabs(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        w: &Value,
        rect: Rect,
        widgets: &[Value],
        frames: &[Value],
        wstate: &Value,
        fstate: &Value,
        emit: &dyn Fn(Value),
    ) {
        let tabs = w.get("tabs").and_then(|t| t.as_array()).cloned().unwrap_or_default();
        let key = w.get("name").and_then(|n| n.as_str()).map(str::to_string).unwrap_or_else(|| format!("#{i}"));
        let first = tabs.first().and_then(|t| t.get("id")).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let mut active = self.tabs.get(&key).cloned().unwrap_or_else(|| first.clone());
        if !tabs.iter().any(|t| t.get("id").and_then(|v| v.as_str()) == Some(active.as_str())) {
            active = first;
        }

        // card frame
        ui.painter().rect(
            rect,
            Rounding::same(6.0),
            ui.visuals().widgets.noninteractive.bg_fill,
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );
        let bar_h = 26.0;
        let bar_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), bar_h));
        let content_rect = Rect::from_min_max(Pos2::new(rect.min.x, rect.min.y + bar_h), rect.max);

        let mut clicked: Option<String> = None;
        let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(bar_rect.shrink(4.0)));
        bar.set_clip_rect(bar_rect);
        bar.horizontal_wrapped(|ui| {
            for t in &tabs {
                let tid = t.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let tlabel = t.get("label").and_then(|v| v.as_str()).unwrap_or(&tid).to_string();
                if ui.selectable_label(active == tid, tlabel).clicked() {
                    clicked = Some(tid);
                }
            }
        });
        if let Some(t) = clicked {
            active = t;
        }
        self.tabs.insert(key, active.clone());

        // separator under the bar
        ui.painter().line_segment(
            [Pos2::new(rect.min.x, rect.min.y + bar_h), Pos2::new(rect.max.x, rect.min.y + bar_h)],
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );

        let mut content = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
        content.set_clip_rect(content_rect);
        self.render_container(&mut content, &active, content_rect, widgets, frames, wstate, fstate, emit);
    }

    // ---- edit: multi-select, drag, resize, snap, align, undo ----------------
    fn edit_ui(&mut self, ctx: &egui::Context) {
        // keyboard: undo/redo, delete, deselect — suppressed while a text field is focused.
        let typing = ctx.wants_keyboard_input();
        let (undo_key, redo_key, del_key, esc_key) = ctx.input(|i| {
            let z = i.modifiers.command && i.key_pressed(egui::Key::Z);
            (
                z && !i.modifiers.shift,
                (z && i.modifiers.shift) || (i.modifiers.command && i.key_pressed(egui::Key::Y)),
                !typing && (i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)),
                !typing && i.key_pressed(egui::Key::Escape),
            )
        });

        let mut add: Option<String> = None;
        let (mut do_delete, mut do_save, mut do_undo, mut do_redo) = (del_key, false, undo_key, redo_key);
        let mut align: Option<&str> = None;
        let mut do_group = false; // wrap the selection in a new frame (resolved in the canvas pass)
        egui::TopBottomPanel::top("ed_toolbar").show(ctx, |ui| {
            // ASCII-only labels: egui's default font lacks box-drawing/emoji glyphs.
            ui.horizontal_wrapped(|ui| {
                ui.menu_button("Add", |ui| {
                    for t in ["button", "slider", "toggle", "readout", "label", "color", "select", "frame"] {
                        if ui.button(t).clicked() {
                            add = Some(t.to_string());
                            ui.close_menu();
                        }
                    }
                });
                if ui.button("Delete").on_hover_text("Delete selected").clicked() {
                    do_delete = true;
                }
                ui.separator();
                if ui.button("Undo").clicked() {
                    do_undo = true;
                }
                if ui.button("Redo").clicked() {
                    do_redo = true;
                }
                ui.separator();
                ui.label("Align");
                for (lbl, key, tip) in [
                    ("L", "left", "Align left"), ("C", "cx", "Align centers (horizontal)"), ("R", "right", "Align right"),
                    ("T", "top", "Align top"), ("M", "cy", "Align middles (vertical)"), ("B", "bottom", "Align bottom"),
                ] {
                    if ui.button(lbl).on_hover_text(tip).clicked() {
                        align = Some(key);
                    }
                }
                ui.separator();
                if ui.button("Group").on_hover_text("Wrap the selected widgets in a new frame").clicked() {
                    do_group = true;
                }
                if ui.button("Save").clicked() {
                    do_save = true;
                }
            });
        });

        // toolbar mutations
        {
            let mut sh = self.shared.borrow_mut();
            if esc_key {
                sh.selected.clear();
                sh.selected_frame = None;
            }
            if let Some(t) = add {
                push_undo(&mut sh);
                if t == "frame" {
                    add_frame(&mut sh.spec);
                    let n = sh.spec.get("frames").and_then(|f| f.as_array()).map(|a| a.len()).unwrap_or(0);
                    sh.selected.clear();
                    sh.selected_frame = if n > 0 { Some(n - 1) } else { None };
                } else {
                    add_widget(&mut sh.spec, &t);
                    let n = sh.spec.get("widgets").and_then(|w| w.as_array()).map(|a| a.len()).unwrap_or(0);
                    sh.selected_frame = None;
                    sh.selected = if n > 0 { vec![n - 1] } else { vec![] };
                }
            }
            if do_delete && !sh.selected.is_empty() {
                push_undo(&mut sh);
                let mut sel = sh.selected.clone();
                sel.sort_unstable();
                for i in sel.iter().rev() {
                    delete_widget(&mut sh.spec, *i);
                }
                sh.selected.clear();
            } else if do_delete {
                if let Some(fi) = sh.selected_frame {
                    push_undo(&mut sh);
                    if let Some(a) = sh.spec.get_mut("frames").and_then(|f| f.as_array_mut()) {
                        if fi < a.len() { a.remove(fi); }
                    }
                    sh.selected_frame = None;
                }
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

        self.inspector_panel(ctx);

        let bg = ctx.style().visuals.panel_fill;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(bg))
            .show(ctx, |ui| {
                let canvas = ui.max_rect();
                draw_grid(ui, canvas);
                let widgets = self.shared.borrow().spec.get("widgets").and_then(|w| w.as_array()).cloned().unwrap_or_default();
                let frames = self.shared.borrow().spec.get("frames").and_then(|f| f.as_array()).cloned().unwrap_or_default();
                let selected = self.shared.borrow().selected.clone();
                let selected_frame = self.shared.borrow().selected_frame;
                let wstate = self.shared.borrow().state.get("widgets").cloned().unwrap_or(Value::Null);
                let shift = ui.input(|i| i.modifiers.shift);
                let accent = ui.visuals().selection.stroke.color;
                let border = ui.visuals().widgets.noninteractive.bg_stroke.color;

                // walk the container tree (honoring active tabs) into placements
                let tabs_snapshot = self.tabs.clone();
                let mut placements: Vec<(usize, Rect, Rect)> = Vec::new();
                let mut frame_rects: Vec<(usize, Rect, Rect)> = Vec::new();
                let mut tabbars: Vec<EditTabBar> = Vec::new();
                collect_edit_layout("root", canvas, &widgets, &frames, &tabs_snapshot, &mut placements, &mut frame_rects, &mut tabbars);
                let parent_of: HashMap<usize, Rect> = placements.iter().map(|(i, _, p)| (*i, *p)).collect();
                let frame_parent_of: HashMap<usize, Rect> = frame_rects.iter().map(|(i, _, p)| (*i, *p)).collect();

                // empty-canvas click clears selection; drag = marquee. Registered FIRST so
                // the frame/widget interactions (added after) sit on top and win the pointer.
                let bg_resp = ui.interact(canvas, egui::Id::new("ed_bg"), Sense::click_and_drag());
                let marquee0 = self.shared.borrow().marquee;

                let mut click_sel: Option<(usize, bool)> = None;
                let mut click_frame: Option<usize> = None;
                let mut begin: Option<(usize, bool, bool, Pos2)> = None; // (idx, resize, frames, start)
                let mut pointer: Option<Pos2> = None;
                let mut stop = false;
                let mut tab_switch: Option<(String, String)> = None;

                // frame outlines + click-to-select + drag-to-move + resize handle (above bg,
                // below widgets so child widgets still win their own areas). Moving a frame
                // moves its children, which render relative to it. The selected frame is
                // highlighted so a layers-panel selection shows on the canvas.
                for (fi, fr, _parent) in frame_rects.iter().copied() {
                    let sel = selected_frame == Some(fi);
                    let stroke = if sel { Stroke::new(2.0, accent) } else { Stroke::new(1.0, border.gamma_multiply(0.8)) };
                    ui.painter().rect_stroke(fr, Rounding::same(6.0), stroke);
                    if frames.get(fi).and_then(|f| f.get("locked")).and_then(|l| l.as_bool()).unwrap_or(false) {
                        continue;
                    }
                    let fid = egui::Id::new(("edf", fi));
                    let fresp = ui.interact(fr, fid, Sense::click_and_drag());
                    if fresp.clicked() {
                        click_frame = Some(fi);
                    }
                    if fresp.drag_started() {
                        begin = Some((fi, false, true, fresp.interact_pointer_pos().unwrap_or(fr.min)));
                    }
                    if fresp.dragged() {
                        pointer = fresp.interact_pointer_pos();
                    }
                    if fresp.drag_stopped() {
                        stop = true;
                    }
                    if sel {
                        let h = Rect::from_min_size(fr.max - Vec2::splat(11.0), Vec2::splat(11.0));
                        let hr = ui.interact(h, fid.with("rs"), Sense::drag());
                        ui.painter().rect_filled(h, Rounding::same(2.0), accent);
                        if hr.drag_started() {
                            begin = Some((fi, true, true, hr.interact_pointer_pos().unwrap_or(fr.max)));
                        }
                        if hr.dragged() {
                            pointer = hr.interact_pointer_pos();
                        }
                        if hr.drag_stopped() {
                            stop = true;
                        }
                    }
                }

                // tabs widgets: chrome + clickable bar (children render as placements below)
                for tb in &tabbars {
                    ui.painter().rect(tb.rect, Rounding::same(6.0), ui.visuals().widgets.noninteractive.bg_fill, Stroke::new(1.0, border));
                    let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(tb.bar_rect.shrink(4.0)));
                    bar.set_clip_rect(tb.bar_rect);
                    bar.horizontal_wrapped(|ui| {
                        for (id, label) in &tb.tabs {
                            if ui.selectable_label(&tb.active == id, label).clicked() {
                                tab_switch = Some((tb.key.clone(), id.clone()));
                            }
                        }
                    });
                    ui.painter().line_segment([tb.bar_rect.left_bottom(), tb.bar_rect.right_bottom()], Stroke::new(1.0, border));
                }

                // widget placements: WYSIWYG render + glass-pane select/drag/resize
                for (i, rect, _parent) in placements.iter().copied() {
                    let w = &widgets[i];
                    let is_sel = selected.contains(&i);
                    let id = egui::Id::new(("ed", i));
                    let (ty, name, label, val, dbg, dfg) = display_of(w, &wstate);
                    let noop = |_v: Value| {};
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    child.set_clip_rect(rect);
                    draw_interact_widget(&mut child, rect, &ty, &name, &label, &val, dbg, dfg, &noop, w, self);

                    // locked: not selectable/draggable on the canvas (still in the layer tree).
                    if w.get("locked").and_then(|l| l.as_bool()).unwrap_or(false) {
                        let stroke = if is_sel { Stroke::new(2.0, accent) } else { Stroke::new(1.0, border.gamma_multiply(0.3)) };
                        ui.painter().rect_stroke(rect, Rounding::same(5.0), stroke);
                        continue;
                    }

                    let resp = ui.interact(rect, id, Sense::click_and_drag());
                    resp.context_menu(|ui| {
                        if ui.button("Group into Frame").clicked() {
                            do_group = true;
                            ui.close_menu();
                        }
                    });
                    if resp.clicked() {
                        click_sel = Some((i, shift));
                    }
                    if resp.drag_started() {
                        begin = Some((i, false, false, resp.interact_pointer_pos().unwrap_or(rect.min)));
                    }
                    if resp.dragged() {
                        pointer = resp.interact_pointer_pos();
                    }
                    if resp.drag_stopped() {
                        stop = true;
                    }
                    let stroke = if is_sel {
                        Stroke::new(2.0, accent)
                    } else if resp.hovered() {
                        Stroke::new(1.0, accent.gamma_multiply(0.5))
                    } else {
                        Stroke::new(1.0, border.gamma_multiply(0.5))
                    };
                    ui.painter().rect_stroke(rect, Rounding::same(5.0), stroke);
                    if is_sel && selected.len() == 1 {
                        let h = Rect::from_min_size(rect.max - Vec2::splat(11.0), Vec2::splat(11.0));
                        let hr = ui.interact(h, id.with("rs"), Sense::drag());
                        ui.painter().rect_filled(h, Rounding::same(2.0), accent);
                        if hr.drag_started() {
                            begin = Some((i, true, false, hr.interact_pointer_pos().unwrap_or(rect.max)));
                        }
                        if hr.dragged() {
                            pointer = hr.interact_pointer_pos();
                        }
                        if hr.drag_stopped() {
                            stop = true;
                        }
                    }
                }
                if let Some((k, v)) = tab_switch {
                    self.tabs.insert(k, v);
                }
                let clicked_empty = bg_resp.clicked() && click_sel.is_none() && begin.is_none();

                // marquee rubber-band (drag started on empty canvas)
                let mut marquee_rect: Option<Rect> = None;
                let mq_start = marquee0.or_else(|| if bg_resp.drag_started() { bg_resp.interact_pointer_pos() } else { None });
                if let (Some(start), Some(cur)) = (mq_start, bg_resp.interact_pointer_pos()) {
                    if bg_resp.dragged() || bg_resp.drag_stopped() {
                        let r = Rect::from_two_pos(start, cur);
                        ui.painter().rect(r, Rounding::same(0.0), accent.gamma_multiply(0.10), Stroke::new(1.0, accent));
                        marquee_rect = Some(r);
                    }
                }

                let mut sh = self.shared.borrow_mut();
                if bg_resp.drag_started() {
                    sh.marquee = bg_resp.interact_pointer_pos();
                }
                if bg_resp.drag_stopped() {
                    if let Some(r) = marquee_rect {
                        let mut sel: Vec<usize> = if shift { sh.selected.clone() } else { vec![] };
                        for (i, wr, _p) in &placements {
                            let locked = widgets.get(*i).and_then(|w| w.get("locked")).and_then(|l| l.as_bool()).unwrap_or(false);
                            if !locked && r.intersects(*wr) && !sel.contains(i) {
                                sel.push(*i);
                            }
                        }
                        sh.selected = sel;
                        if !sh.selected.is_empty() {
                            sh.selected_frame = None;
                        }
                    }
                    sh.marquee = None;
                }
                if let Some(fi) = click_frame {
                    sh.selected.clear();
                    sh.selected_frame = Some(fi);
                }
                if let Some((i, sh_held)) = click_sel {
                    sh.selected_frame = None;
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
                    sh.selected_frame = None;
                }
                if let Some((i, resize, frames_t, start)) = begin {
                    if frames_t {
                        sh.selected.clear();
                        sh.selected_frame = Some(i);
                        push_undo(&mut sh);
                        sh.drag = Some(capture_drag(resize, true, start, &sh.spec, &[i], &frame_parent_of));
                    } else {
                        if resize {
                            // keep current selection (single)
                        } else if !sh.selected.contains(&i) {
                            sh.selected = if shift { let mut s = sh.selected.clone(); s.push(i); s } else { vec![i] };
                        }
                        push_undo(&mut sh);
                        let idxs: Vec<usize> = if resize { vec![i] } else { sh.selected.clone() };
                        sh.drag = Some(capture_drag(resize, false, start, &sh.spec, &idxs, &parent_of));
                    }
                }
                if let (Some(pos), Some(drag)) = (pointer, sh.drag.clone()) {
                    let total = pos - drag.start;
                    apply_drag(&mut sh.spec, &drag, total, shift);
                }
                if stop {
                    sh.drag = None;
                }
                // Group into Frame: wrap the selection (uses the just-walked abs rects).
                // do_group = toolbar/canvas this pass; pending_group = layers context menu.
                let do_group = do_group || sh.pending_group;
                sh.pending_group = false;
                if do_group && !sh.selected.is_empty() {
                    let sel = sh.selected.clone();
                    let abs: HashMap<usize, Rect> = placements.iter().map(|(i, r, _)| (*i, *r)).collect();
                    push_undo(&mut sh);
                    if let Some(fi) = group_into_frame(&mut sh.spec, &sel, &abs, canvas) {
                        sh.selected.clear();
                        sh.selected_frame = Some(fi);
                    }
                }
            });
    }

    // ---- layers + property inspector (right side panel) ---------------------
    fn inspector_panel(&mut self, ctx: &egui::Context) {
        let shift = ctx.input(|i| i.modifiers.shift);
        egui::SidePanel::right("ed_inspector").default_width(230.0).show(ctx, |ui| {
            let mut sh = self.shared.borrow_mut();
            let count = sh.spec.get("widgets").and_then(|w| w.as_array()).map(|a| a.len()).unwrap_or(0);
            let fcount = sh.spec.get("frames").and_then(|f| f.as_array()).map(|a| a.len()).unwrap_or(0);
            // frame ids (for the membership dropdown)
            let frame_ids: Vec<String> = (0..fcount)
                .map(|i| sh.spec["frames"][i].get("id").and_then(|v| v.as_str()).unwrap_or("").to_string())
                .collect();

            let accent_c = ui.visuals().selection.stroke.color;
            let muted_c = ui.visuals().weak_text_color();
            ui.add_space(4.0);
            ui.strong("Layers");
            ui.label(RichText::new("drag to reorder · L = lock").size(9.0).weak());
            let mut toggle_sel: Option<usize> = None;
            let mut hide_toggle: Option<usize> = None;
            let mut del: Option<usize> = None;
            let mut sel_frame: Option<usize> = None;
            let mut hide_frame: Option<usize> = None;
            let mut del_frame: Option<usize> = None;
            let mut collapse_frame: Option<usize> = None;
            let mut lock_w: Option<usize> = None;
            let mut lock_f: Option<usize> = None;
            let mut reorder_w: Option<(usize, usize)> = None; // (from, to) within widgets
            let mut reorder_f: Option<(usize, usize)> = None; // (from, to) within frames
            let mut reparent_w: Option<(usize, usize)> = None; // (widget idx, frame idx) — drop widget on a frame row
            let mut group_click = false; // layers "Group into Frame" → sh.pending_group

            // snapshot the tree structure, then flatten to (depth, item) display rows
            let frames_meta: Vec<(usize, String, Option<String>, bool)> = (0..fcount)
                .map(|fi| {
                    let f = &sh.spec["frames"][fi];
                    (
                        fi,
                        f.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        f.get("parent").and_then(|v| v.as_str()).map(str::to_string),
                        f.get("collapsed").and_then(|c| c.as_bool()).unwrap_or(false),
                    )
                })
                .collect();
            let widget_frame: Vec<(usize, Option<String>)> = (0..count)
                .map(|i| (i, sh.spec["widgets"][i].get("frame").and_then(|v| v.as_str()).map(str::to_string)))
                .collect();
            let mut rows: Vec<(usize, LayerDrag)> = Vec::new();
            build_layer_rows(None, 0, &frames_meta, &widget_frame, &mut rows);

            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                for (depth, item) in &rows {
                    ui.horizontal(|ui| {
                        ui.add_space(*depth as f32 * 12.0);
                        match *item {
                            LayerDrag::Frame(fi) => {
                                let f = &sh.spec["frames"][fi];
                                let nm = f.get("name").and_then(|n| n.as_str()).or_else(|| f.get("id").and_then(|n| n.as_str())).unwrap_or("").to_string();
                                let hidden = f.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false);
                                let collapsed = f.get("collapsed").and_then(|c| c.as_bool()).unwrap_or(false);
                                let is_sel = sh.selected_frame == Some(fi);
                                if ui.add(egui::Button::new(if collapsed { ">" } else { "v" }).small().frame(false)).on_hover_text("Expand/collapse").clicked() {
                                    collapse_frame = Some(fi);
                                }
                                if ui.add(egui::Button::new(if hidden { "-" } else { "o" }).small().frame(false)).on_hover_text("Show/hide").clicked() {
                                    hide_frame = Some(fi);
                                }
                                let flocked = f.get("locked").and_then(|l| l.as_bool()).unwrap_or(false);
                                if ui.add(egui::Button::new(RichText::new("L").color(if flocked { accent_c } else { muted_c })).small().frame(false)).on_hover_text("Lock/unlock (canvas)").clicked() {
                                    lock_f = Some(fi);
                                }
                                // one widget, click_and_drag sense: a click (no movement) selects;
                                // a press past egui's drag threshold starts a drag. Click is preserved.
                                let lab = ui.selectable_label(is_sel, format!("[] {nm}"));
                                let resp = ui.interact(lab.rect, lab.id, egui::Sense::click_and_drag());
                                if resp.clicked() {
                                    sel_frame = Some(fi);
                                }
                                if resp.drag_started() {
                                    egui::DragAndDrop::set_payload(ui.ctx(), LayerDrag::Frame(fi));
                                }
                                if resp.dragged() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                }
                                if resp.dnd_hover_payload::<LayerDrag>().is_some() {
                                    ui.painter().hline(resp.rect.x_range(), resp.rect.top(), Stroke::new(2.0, accent_c));
                                }
                                resp.context_menu(|ui| {
                                    if ui.button("Delete frame").clicked() { del_frame = Some(fi); ui.close_menu(); }
                                });
                                if let Some(p) = resp.dnd_release_payload::<LayerDrag>() {
                                    match *p {
                                        LayerDrag::Frame(from) => reorder_f = Some((from, fi)),
                                        LayerDrag::Widget(from) => reparent_w = Some((from, fi)),
                                    }
                                }
                            }
                            LayerDrag::Widget(i) => {
                                let w = &sh.spec["widgets"][i];
                                let name = w.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                                let ty = w.get("type").and_then(|t| t.as_str()).unwrap_or("").to_string();
                                let hidden = w.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false);
                                let is_sel = sh.selected.contains(&i);
                                if ui.add(egui::Button::new(if hidden { "-" } else { "o" }).small().frame(false)).on_hover_text("Show/hide").clicked() {
                                    hide_toggle = Some(i);
                                }
                                let wlocked = w.get("locked").and_then(|l| l.as_bool()).unwrap_or(false);
                                if ui.add(egui::Button::new(RichText::new("L").color(if wlocked { accent_c } else { muted_c })).small().frame(false)).on_hover_text("Lock/unlock (canvas)").clicked() {
                                    lock_w = Some(i);
                                }
                                let txt = if name.is_empty() { format!("{ty} #{i}") } else { format!("{name}  ({ty})") };
                                let lab = ui.selectable_label(is_sel, txt);
                                let resp = ui.interact(lab.rect, lab.id, egui::Sense::click_and_drag());
                                if resp.clicked() {
                                    toggle_sel = Some(i);
                                }
                                if resp.drag_started() {
                                    egui::DragAndDrop::set_payload(ui.ctx(), LayerDrag::Widget(i));
                                }
                                if resp.dragged() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                }
                                if resp.dnd_hover_payload::<LayerDrag>().is_some() {
                                    ui.painter().hline(resp.rect.x_range(), resp.rect.top(), Stroke::new(2.0, accent_c));
                                }
                                resp.context_menu(|ui| {
                                    if ui.button("Group into Frame").clicked() { group_click = true; ui.close_menu(); }
                                    if ui.button("Delete").clicked() { del = Some(i); ui.close_menu(); }
                                });
                                if let Some(p) = resp.dnd_release_payload::<LayerDrag>() {
                                    if let LayerDrag::Widget(from) = *p {
                                        reorder_w = Some((from, i));
                                    }
                                }
                            }
                        }
                    });
                }
            });
            if let Some(fi) = collapse_frame {
                let cur = sh.spec["frames"][fi].get("collapsed").and_then(|c| c.as_bool()).unwrap_or(false);
                sh.spec["frames"][fi]["collapsed"] = json!(!cur);
            }
            if let Some(i) = lock_w {
                push_undo(&mut sh);
                let cur = sh.spec["widgets"][i].get("locked").and_then(|l| l.as_bool()).unwrap_or(false);
                sh.spec["widgets"][i]["locked"] = json!(!cur);
            }
            if let Some(fi) = lock_f {
                push_undo(&mut sh);
                let cur = sh.spec["frames"][fi].get("locked").and_then(|l| l.as_bool()).unwrap_or(false);
                sh.spec["frames"][fi]["locked"] = json!(!cur);
            }
            if let Some(fi) = sel_frame {
                sh.selected.clear();
                sh.selected_frame = Some(fi);
            }
            if let Some(fi) = hide_frame {
                push_undo(&mut sh);
                let cur = sh.spec["frames"][fi].get("hidden").and_then(|h| h.as_bool()).unwrap_or(false);
                sh.spec["frames"][fi]["hidden"] = json!(!cur);
            }
            if let Some(fi) = del_frame {
                push_undo(&mut sh);
                if let Some(a) = sh.spec.get_mut("frames").and_then(|f| f.as_array_mut()) {
                    if fi < a.len() { a.remove(fi); }
                }
                sh.selected_frame = None;
            }
            if let Some(i) = toggle_sel {
                sh.selected_frame = None;
                if shift {
                    if let Some(p) = sh.selected.iter().position(|x| *x == i) { sh.selected.remove(p); }
                    else { sh.selected.push(i); }
                } else {
                    sh.selected = vec![i];
                }
            }
            if let Some(i) = hide_toggle {
                push_undo(&mut sh);
                let cur = sh.spec["widgets"][i].get("hidden").and_then(|h| h.as_bool()).unwrap_or(false);
                sh.spec["widgets"][i]["hidden"] = json!(!cur);
            }
            if let Some(i) = del {
                push_undo(&mut sh);
                delete_widget(&mut sh.spec, i);
                sh.selected.clear();
            }
            if let Some((from, to)) = reorder_w {
                if from != to {
                    push_undo(&mut sh);
                    // drag the whole selection if the dragged row is part of a multi-select
                    if sh.selected.len() > 1 && sh.selected.contains(&from) {
                        let sel = sh.selected.clone();
                        let n = sel.len();
                        let start = move_many_in_array(&mut sh.spec, "widgets", sel, to);
                        sh.selected = (start..start + n).collect();
                    } else {
                        let ni = move_in_array(&mut sh.spec, "widgets", from, to);
                        sh.selected = vec![ni];
                    }
                    sh.selected_frame = None;
                }
            }
            if let Some((from, to)) = reorder_f {
                if from != to {
                    push_undo(&mut sh);
                    let ni = move_in_array(&mut sh.spec, "frames", from, to);
                    sh.selected_frame = Some(ni);
                    sh.selected.clear();
                }
            }
            if let Some((wi, fi)) = reparent_w {
                // drop a widget (or the whole multi-selection) into frame fi
                let fid = sh.spec["frames"][fi].get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if !fid.is_empty() {
                    push_undo(&mut sh);
                    let targets = if sh.selected.len() > 1 && sh.selected.contains(&wi) { sh.selected.clone() } else { vec![wi] };
                    for t in targets {
                        if let Some(w) = sh.spec.get_mut("widgets").and_then(|a| a.as_array_mut()).and_then(|a| a.get_mut(t)) {
                            w["frame"] = json!(fid);
                            if let Some(o) = w.as_object_mut() {
                                o.remove("tab");
                            }
                        }
                    }
                }
            }
            if group_click {
                sh.pending_group = true; // the canvas pass has the geometry to build the frame
            }

            ui.separator();
            ui.strong("Properties");

            // frame inspector
            if let Some(fi) = sh.selected_frame {
                if fi >= fcount {
                    sh.selected_frame = None;
                    return;
                }
                let f = sh.spec["frames"][fi].clone();
                edit_frame_props(ui, &mut sh, fi, &f);
                return;
            }

            let sel = sh.selected.clone();
            if sel.len() != 1 {
                ui.weak(if sel.is_empty() { "No selection" } else { "Multiple selected" });
                return;
            }
            let i = sel[0];
            if i >= count {
                return;
            }
            let w = sh.spec["widgets"][i].clone();
            let mut name = w.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let mut label = w.get("label").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let (mut x, mut y, mut ww, mut hh) = (num(&w, "x", 0.0), num(&w, "y", 0.0), num(&w, "w", 30.0), num(&w, "h", 48.0));
            let mut ah = anchor(&w, "anchorH", "scale");
            let mut av = anchor(&w, "anchorV", "start");
            // current frame membership ("" = root)
            let mut member = w.get("frame").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let mut changed = false;
            let mut snapshot = false; // gesture start → one undo entry

            ui.add_space(2.0);
            ui.label(RichText::new(w.get("type").and_then(|t| t.as_str()).unwrap_or("?")).weak());
            egui::Grid::new("props").num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
                ui.label("name");
                let r = ui.text_edit_singleline(&mut name);
                if r.gained_focus() { snapshot = true; }
                if r.changed() { changed = true; }
                ui.end_row();
                ui.label("label");
                let r = ui.text_edit_singleline(&mut label);
                if r.gained_focus() { snapshot = true; }
                if r.changed() { changed = true; }
                ui.end_row();
                for (lbl, v) in [("x", &mut x), ("y", &mut y), ("w", &mut ww), ("h", &mut hh)] {
                    ui.label(lbl);
                    let r = ui.add(egui::DragValue::new(v).speed(0.5));
                    if r.drag_started() { snapshot = true; }
                    if r.changed() { changed = true; }
                    ui.end_row();
                }
                for (lbl, cur, id) in [("anchor H", &mut ah, "ah"), ("anchor V", &mut av, "av")] {
                    ui.label(lbl);
                    let before = cur.clone();
                    egui::ComboBox::from_id_salt(id).selected_text(cur.as_str()).show_ui(ui, |ui| {
                        for opt in ["scale", "start", "center", "end", "stretch"] {
                            ui.selectable_value(cur, opt.to_string(), opt);
                        }
                    });
                    if *cur != before { snapshot = true; changed = true; }
                    ui.end_row();
                }
                // frame membership
                ui.label("in frame");
                let before = member.clone();
                let shown = if member.is_empty() { "(root)".to_string() } else { member.clone() };
                egui::ComboBox::from_id_salt("memb").selected_text(shown).show_ui(ui, |ui| {
                    ui.selectable_value(&mut member, String::new(), "(root)");
                    for fid in &frame_ids {
                        ui.selectable_value(&mut member, fid.clone(), fid.as_str());
                    }
                });
                if member != before { snapshot = true; changed = true; }
                ui.end_row();
            });
            if changed {
                if snapshot { push_undo(&mut sh); }
                let wm = &mut sh.spec["widgets"][i];
                wm["name"] = json!(name);
                wm["label"] = json!(label);
                wm["x"] = json!(r2(x));
                wm["y"] = json!(r2(y));
                wm["w"] = json!(r2(ww));
                wm["h"] = json!(r2(hh));
                wm["anchorH"] = json!(ah);
                wm["anchorV"] = json!(av);
                if member.is_empty() {
                    wm.as_object_mut().map(|o| o.remove("frame"));
                } else {
                    wm["frame"] = json!(member);
                }
            }
        });
    }
}

/// Frame property editor (name, geometry, anchors, clip), used by the inspector.
fn edit_frame_props(ui: &mut egui::Ui, sh: &mut Shared, fi: usize, f: &Value) {
    let mut name = f.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mut id = f.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let (mut x, mut y, mut ww, mut hh) = (num(f, "x", 0.0), num(f, "y", 0.0), num(f, "w", 40.0), num(f, "h", 40.0));
    let mut ah = anchor(f, "anchorH", "scale");
    let mut av = anchor(f, "anchorV", "scale");
    let mut clip = f.get("clip").and_then(|c| c.as_bool()).unwrap_or(true);
    let mut changed = false;
    let mut snapshot = false;

    ui.add_space(2.0);
    ui.label(RichText::new("frame").weak());
    egui::Grid::new("fprops").num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
        ui.label("id");
        let r = ui.text_edit_singleline(&mut id);
        if r.gained_focus() { snapshot = true; }
        if r.changed() { changed = true; }
        ui.end_row();
        ui.label("name");
        let r = ui.text_edit_singleline(&mut name);
        if r.gained_focus() { snapshot = true; }
        if r.changed() { changed = true; }
        ui.end_row();
        for (lbl, v) in [("x", &mut x), ("y", &mut y), ("w", &mut ww), ("h", &mut hh)] {
            ui.label(lbl);
            let r = ui.add(egui::DragValue::new(v).speed(0.5));
            if r.drag_started() { snapshot = true; }
            if r.changed() { changed = true; }
            ui.end_row();
        }
        for (lbl, cur, gid) in [("anchor H", &mut ah, "fah"), ("anchor V", &mut av, "fav")] {
            ui.label(lbl);
            let before = cur.clone();
            egui::ComboBox::from_id_salt(gid).selected_text(cur.as_str()).show_ui(ui, |ui| {
                for opt in ["scale", "start", "center", "end", "stretch"] {
                    ui.selectable_value(cur, opt.to_string(), opt);
                }
            });
            if *cur != before { snapshot = true; changed = true; }
            ui.end_row();
        }
        ui.label("clip");
        if ui.checkbox(&mut clip, "").changed() { snapshot = true; changed = true; }
        ui.end_row();
    });
    if changed {
        if snapshot { push_undo(sh); }
        let fm = &mut sh.spec["frames"][fi];
        fm["id"] = json!(id);
        fm["name"] = json!(name);
        fm["x"] = json!(r2(x));
        fm["y"] = json!(r2(y));
        fm["w"] = json!(r2(ww));
        fm["h"] = json!(r2(hh));
        fm["anchorH"] = json!(ah);
        fm["anchorV"] = json!(av);
        fm["clip"] = json!(clip);
    }
}

/// Layer-list drag payload (which list + source index).
#[derive(Clone, Copy)]
enum LayerDrag {
    Widget(usize),
    Frame(usize),
}

/// Wrap the selected widgets in a new frame sized to their bounding box, reparenting
/// each into it (coords made relative to the frame). `abs` = each widget's absolute
/// px rect; `canvas` = the root rect. Returns the new frame's index.
fn group_into_frame(spec: &mut Value, sel: &[usize], abs: &HashMap<usize, Rect>, canvas: Rect) -> Option<usize> {
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
    let n = spec.get("frames").and_then(|f| f.as_array()).map(|a| a.len()).unwrap_or(0) + 1;
    let id = format!("group{n}");
    let (fx, fw) = store_axis("scale", bbox.min.x - canvas.min.x, bbox.width(), canvas.width());
    let (fy, fh) = store_axis("scale", bbox.min.y - canvas.min.y, bbox.height(), canvas.height());
    let frame = json!({
        "id": id.clone(), "name": id.clone(),
        "x": r2(fx), "y": r2(fy), "w": r2(fw), "h": r2(fh),
        "anchorH": "scale", "anchorV": "scale", "clip": true
    });
    spec.as_object_mut().unwrap().entry("frames").or_insert(json!([])).as_array_mut().unwrap().push(frame);
    let fidx = n - 1;
    // reparent: each widget's coords become relative to the frame bbox
    for &i in sel {
        let Some(r) = abs.get(&i).copied() else { continue };
        let Some(w) = spec.get_mut("widgets").and_then(|a| a.as_array_mut()).and_then(|a| a.get_mut(i)) else { continue };
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

/// Move an array element from→to (drop-before semantics), returning its new index.
fn move_in_array(spec: &mut Value, key: &str, from: usize, to: usize) -> usize {
    let Some(arr) = spec.get_mut(key).and_then(|v| v.as_array_mut()) else { return from };
    if from >= arr.len() {
        return from;
    }
    let item = arr.remove(from);
    let insert_at = if to > from { to - 1 } else { to }.min(arr.len());
    arr.insert(insert_at, item);
    insert_at
}

/// Flatten the frame/widget tree into display rows (depth, item), honoring each
/// frame's collapsed state. `container` = None for root (frame.parent / widget.frame absent).
fn build_layer_rows(
    container: Option<&str>,
    depth: usize,
    frames_meta: &[(usize, String, Option<String>, bool)], // (idx, id, parent, collapsed)
    widget_frame: &[(usize, Option<String>)],              // (idx, frame membership)
    rows: &mut Vec<(usize, LayerDrag)>,
) {
    for (fi, id, parent, collapsed) in frames_meta {
        if parent.as_deref() == container {
            rows.push((depth, LayerDrag::Frame(*fi)));
            if !collapsed {
                build_layer_rows(Some(id), depth + 1, frames_meta, widget_frame, rows);
            }
        }
    }
    for (i, frame) in widget_frame {
        if frame.as_deref() == container {
            rows.push((depth, LayerDrag::Widget(*i)));
        }
    }
}

/// Move several elements (preserving their order) to before `to`. Returns the new
/// start index of the moved block.
fn move_many_in_array(spec: &mut Value, key: &str, mut idxs: Vec<usize>, to: usize) -> usize {
    let Some(arr) = spec.get_mut(key).and_then(|v| v.as_array_mut()) else { return to };
    idxs.sort_unstable();
    idxs.dedup();
    idxs.retain(|&i| i < arr.len());
    if idxs.is_empty() {
        return to;
    }
    let mut removed: Vec<Value> = idxs.iter().rev().map(|&i| arr.remove(i)).collect();
    removed.reverse(); // back to ascending original order
    let before = idxs.iter().filter(|&&i| i < to).count();
    let insert_at = to.saturating_sub(before).min(arr.len());
    for (k, item) in removed.into_iter().enumerate() {
        arr.insert(insert_at + k, item);
    }
    insert_at
}
/// Snapshot the resolved absolute px geometry of `idxs` at drag start, plus each
/// widget's parent container rect (so store-back is parent-relative for nesting).
fn capture_drag(resize: bool, frames: bool, start: Pos2, spec: &Value, idxs: &[usize], parent_of: &HashMap<usize, Rect>) -> Drag {
    let key = if frames { "frames" } else { "widgets" };
    let mut items = Vec::new();
    if let Some(arr) = spec.get(key).and_then(|w| w.as_array()) {
        for &idx in idxs {
            let Some(pr) = parent_of.get(&idx) else { continue };
            if let Some(w) = arr.get(idx) {
                let ah = anchor(w, "anchorH", "scale");
                let av = anchor(w, "anchorV", if frames { "scale" } else { "start" });
                let dw = if ah == "scale" { 25.0 } else { 100.0 };
                let dh = if av == "scale" { 25.0 } else { 48.0 };
                let (sx, ww) = resolve_axis(&ah, num(w, "x", 0.0), num(w, "w", dw), pr.width());
                let (sy, hh) = resolve_axis(&av, num(w, "y", 0.0), num(w, "h", dh), pr.height());
                items.push(DragItem {
                    idx, ah, av,
                    sx: pr.min.x + sx, // absolute
                    sy: pr.min.y + sy,
                    w: ww, h: hh,
                    px: pr.min.x, py: pr.min.y, pw: pr.width(), ph: pr.height(),
                });
            }
        }
    }
    Drag { resize, frames, start, items }
}
fn snap(v: f32, on: bool) -> f32 {
    if on { (v / 8.0).round() * 8.0 } else { v }
}
/// Apply the total pointer delta to the captured drag, writing stored (parent-relative) units back.
fn apply_drag(spec: &mut Value, drag: &Drag, total: Vec2, snap_on: bool) {
    let key = if drag.frames { "frames" } else { "widgets" };
    let Some(arr) = spec.get_mut(key).and_then(|w| w.as_array_mut()) else { return };
    for it in &drag.items {
        let Some(w) = arr.get_mut(it.idx) else { continue };
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

/// One placed tabs widget in the editor: chrome to draw + clickable tab bar.
struct EditTabBar {
    rect: Rect,
    bar_rect: Rect,
    tabs: Vec<(String, String)>, // (id, label)
    active: String,
    key: String,
}

/// Walk the container tree (honoring active tabs) into a flat placement list:
/// `out_w` = (widget idx, abs rect, parent rect); `out_f` = frame rects;
/// `out_t` = tabs widgets (drawn as chrome, their children become placements).
#[allow(clippy::too_many_arguments)]
fn collect_edit_layout(
    container: &str,
    rect: Rect,
    widgets: &[Value],
    frames: &[Value],
    tabs: &HashMap<String, String>,
    out_w: &mut Vec<(usize, Rect, Rect)>,
    out_f: &mut Vec<(usize, Rect, Rect)>, // (frame idx, abs rect, parent rect)
    out_t: &mut Vec<EditTabBar>,
) {
    let is_root = container == "root";
    for (fi, f) in frames.iter().enumerate() {
        let id = f.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let parent = f.get("parent").and_then(|v| v.as_str());
        let tab = f.get("tab").and_then(|v| v.as_str());
        let is_child = if is_root { parent.is_none() && tab.is_none() } else { tab == Some(container) || (parent == Some(container) && tab.is_none()) };
        if !is_child {
            continue;
        }
        let fr = widget_rect(f, rect.min, rect.width(), rect.height());
        out_f.push((fi, fr, rect));
        collect_edit_layout(id, fr, widgets, frames, tabs, out_w, out_f, out_t);
    }
    for (i, w) in widgets.iter().enumerate() {
        if w.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false) {
            continue;
        }
        let frame = w.get("frame").and_then(|v| v.as_str());
        let tab = w.get("tab").and_then(|v| v.as_str());
        let is_child = if is_root { frame.is_none() && tab.is_none() } else { tab == Some(container) || (frame == Some(container) && tab.is_none()) };
        if !is_child {
            continue;
        }
        let wr = widget_rect(w, rect.min, rect.width(), rect.height());
        if w.get("type").and_then(|t| t.as_str()) == Some("tabs") {
            let key = w.get("name").and_then(|n| n.as_str()).map(str::to_string).unwrap_or_else(|| format!("#{i}"));
            let arr = w.get("tabs").and_then(|t| t.as_array()).cloned().unwrap_or_default();
            let list: Vec<(String, String)> = arr
                .iter()
                .map(|t| {
                    let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let lbl = t.get("label").and_then(|v| v.as_str()).unwrap_or(&id).to_string();
                    (id, lbl)
                })
                .collect();
            let first = list.first().map(|t| t.0.clone()).unwrap_or_default();
            let mut active = tabs.get(&key).cloned().unwrap_or_else(|| first.clone());
            if !list.iter().any(|t| t.0 == active) {
                active = first;
            }
            let bar_h = 26.0;
            let bar_rect = Rect::from_min_size(wr.min, Vec2::new(wr.width(), bar_h));
            let content_rect = Rect::from_min_max(Pos2::new(wr.min.x, wr.min.y + bar_h), wr.max);
            out_t.push(EditTabBar { rect: wr, bar_rect, tabs: list, active: active.clone(), key });
            collect_edit_layout(&active, content_rect, widgets, frames, tabs, out_w, out_f, out_t);
        } else {
            out_w.push((i, wr, rect));
        }
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
    if sh.undo.last() == Some(&sh.spec) {
        return; // coalesce identical snapshots (e.g. focus without an edit)
    }
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
fn add_frame(spec: &mut Value) {
    if !spec.is_object() {
        *spec = json!({});
    }
    let arr = spec.as_object_mut().unwrap().entry("frames").or_insert(json!([]));
    if let Some(a) = arr.as_array_mut() {
        let n = a.len() + 1;
        a.push(json!({
            "id": format!("frame{n}"), "name": format!("frame{n}"),
            "x": 4, "y": 8, "w": 40, "h": 40, "anchorH": "scale", "anchorV": "scale", "clip": true
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

/// Resolve a widget's display fields from spec + host render-state.
/// Returns (type, name, label, value, bg, fg).
fn display_of(w: &Value, wstate: &Value) -> (String, String, String, String, Option<Color32>, Option<Color32>) {
    let ty = w.get("type").and_then(|t| t.as_str()).unwrap_or("").to_string();
    let name = w.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
    let ws = wstate.get(&name).cloned().unwrap_or(Value::Null);
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
    (ty, name, label, val, bg, fg)
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
    let _ = rect;
    if let Some(c) = fg {
        ui.visuals_mut().override_text_color = Some(c);
    }
    // A faint "bg-muted/20" card, used only by readout/slider/select widgets —
    // labels and buttons render bare, matching the React renderer.
    let card = |ui: &mut egui::Ui, body: &dyn Fn(&mut egui::Ui)| {
        let fill = bg.unwrap_or(ui.visuals().faint_bg_color);
        egui::Frame::none()
            .fill(fill)
            .stroke(Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
            .rounding(Rounding::same(6.0))
            .inner_margin(Margin::symmetric(8.0, 5.0))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                body(ui);
            });
    };
    match ty {
        // bare text, vertically centered (React: flex h-full items-center text-xs font-medium)
        "label" => {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.label(RichText::new(if val.is_empty() { label } else { val }).size(12.0).strong());
            });
        }
        // bare full-size button
        "button" => {
            if ui.add_sized(ui.available_size(), egui::Button::new(label)).clicked() {
                emit(json!({ "kind": "press", "name": name }));
            }
        }
        // bare full-size toggle (default/outline like the React Button variant)
        "toggle" => {
            let on = app.toggles.entry(name.to_string()).or_insert(false);
            let txt = if label.is_empty() { (if *on { "on" } else { "off" }).to_string() } else { label.to_string() };
            let btn = egui::Button::new(txt).fill(if *on {
                ui.visuals().selection.bg_fill
            } else {
                Color32::TRANSPARENT
            });
            if ui.add_sized(ui.available_size(), btn).clicked() {
                *on = !*on;
                emit(json!({ "kind": "value", "name": name, "value": *on }));
            }
        }
        "readout" => {
            let value = if val.is_empty() { "—".to_string() } else { val.to_string() };
            card(ui, &|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(label).size(11.0).color(muted));
                    ui.label(RichText::new(&value).size(18.0).monospace());
                });
            });
        }
        "slider" => {
            let init = num(w, "value", num(w, "min", 0.0));
            let v = app.sliders.entry(name.to_string()).or_insert(init);
            let (min, max) = (num(w, "min", 0.0), num(w, "max", 100.0));
            let shown = format!("{}", r2(*v));
            let mut changed = false;
            // a card with interactive content (the `card` helper's closure can't borrow v)
            let fill = bg.unwrap_or(ui.visuals().faint_bg_color);
            egui::Frame::none()
                .fill(fill)
                .stroke(Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
                .rounding(Rounding::same(6.0))
                .inner_margin(Margin::symmetric(8.0, 5.0))
                .show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(label).size(11.0).color(muted));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(&shown).monospace());
                        });
                    });
                    // stretch the slider track to the card width (egui defaults to a fixed width)
                    ui.spacing_mut().slider_width = (ui.available_width() - 8.0).max(40.0);
                    if ui.add(egui::Slider::new(v, min..=max).show_value(false)).changed() {
                        changed = true;
                    }
                });
            if changed {
                emit(json!({ "kind": "value", "name": name, "value": *v }));
            }
        }
        "select" => {
            let opts = w.get("options").and_then(|o| o.as_array()).cloned().unwrap_or_default();
            let active = app.selects.get(name).copied();
            let mut pick: Option<usize> = None;
            egui::Frame::none()
                .fill(bg.unwrap_or(ui.visuals().faint_bg_color))
                .stroke(Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
                .rounding(Rounding::same(6.0))
                .inner_margin(Margin::symmetric(8.0, 5.0))
                .show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    if !label.is_empty() {
                        ui.label(RichText::new(label).size(11.0).color(muted));
                    }
                    ui.horizontal_wrapped(|ui| {
                        for (oi, o) in opts.iter().enumerate() {
                            let olabel = o.get("label").and_then(|l| l.as_str()).unwrap_or("?");
                            if ui.selectable_label(active == Some(oi), olabel).clicked() {
                                pick = Some(oi);
                            }
                        }
                    });
                });
            if let Some(oi) = pick {
                app.selects.insert(name.to_string(), oi);
                let olabel = opts.get(oi).and_then(|o| o.get("label")).and_then(|l| l.as_str()).unwrap_or("").to_string();
                emit(json!({ "kind": "select", "name": name, "index": oi, "value": olabel }));
            }
        }
        "color" => {
            let c = app.colors.entry(name.to_string()).or_insert([128, 128, 128]);
            let mut changed = false;
            egui::Frame::none()
                .fill(bg.unwrap_or(ui.visuals().faint_bg_color))
                .stroke(Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
                .rounding(Rounding::same(6.0))
                .inner_margin(Margin::symmetric(8.0, 5.0))
                .show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.horizontal(|ui| {
                        if ui.color_edit_button_srgb(c).changed() {
                            changed = true;
                        }
                        if !label.is_empty() {
                            ui.label(RichText::new(label).size(11.0).color(muted));
                        }
                    });
                });
            if changed {
                emit(json!({ "kind": "value", "name": name, "value": format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]) }));
            }
        }
        other => {
            // dashed placeholder (React: border-dashed, muted)
            let r = ui.max_rect();
            ui.painter().rect_stroke(r, Rounding::same(6.0), Stroke::new(1.0, muted));
            ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("{other}?"), FontId::proportional(10.0), muted);
        }
    }
}
