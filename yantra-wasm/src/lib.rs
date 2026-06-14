// Path-B runtime (W2 bridge): egui renders a YantraSpec; the host (React) owns the
// data-flow and is authoritative. Each change the host pushes a render-state
// (`set_state`) + theme (`set_theme`); egui draws from it. Widget input is reported
// back via the host `on_event` callback — the host routes it through runAction / the
// native mlua tick. So the WASM never touches the device; it's render + input only.
// Flat render for now (tab/frame nesting is a follow-up); `hidden` overrides honored.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use eframe::egui;
use serde_json::Value;
use wasm_bindgen::prelude::*;

#[derive(Default)]
struct Shared {
    spec: Value,
    state: Value, // { widgets: { name: { value, color, fg, label, hidden, disabled } } }
    theme: Value, // { background, foreground, card, primary, border, … } as "rgb(r, g, b)"
    on_event: Option<js_sys::Function>,
    theme_dirty: bool,
}

thread_local! {
    static SHARED: Rc<RefCell<Shared>> = Rc::new(RefCell::new(Shared::default()));
}

/// Boot egui onto a React-owned canvas. `on_event(json)` is called on widget input.
#[wasm_bindgen]
pub fn start(
    canvas: web_sys::HtmlCanvasElement,
    spec_json: String,
    on_event: js_sys::Function,
) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let shared = SHARED.with(|s| s.clone());
    {
        let mut sh = shared.borrow_mut();
        sh.spec = serde_json::from_str(&spec_json).unwrap_or(Value::Null);
        sh.on_event = Some(on_event);
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

/// Host → wasm: the per-widget render state (display values + presentation overrides).
#[wasm_bindgen]
pub fn set_state(state_json: String) {
    let v: Value = serde_json::from_str(&state_json).unwrap_or(Value::Null);
    SHARED.with(|s| s.borrow_mut().state = v);
}

/// Host → wasm: the resolved theme token colours (so egui matches the app).
#[wasm_bindgen]
pub fn set_theme(theme_json: String) {
    let v: Value = serde_json::from_str(&theme_json).unwrap_or(Value::Null);
    SHARED.with(|s| {
        let mut sh = s.borrow_mut();
        sh.theme = v;
        sh.theme_dirty = true;
    });
}

struct YantraApp {
    shared: Rc<RefCell<Shared>>,
    sliders: HashMap<String, f32>,
    toggles: HashMap<String, bool>,
    colors: HashMap<String, [u8; 3]>,
}

// ---- helpers ----------------------------------------------------------------
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
fn parse_rgb(s: &str) -> Option<egui::Color32> {
    let inner = s.trim().trim_start_matches("rgba").trim_start_matches("rgb");
    let inner = inner.trim_matches(|c| c == '(' || c == ')');
    let mut it = inner.split(',').map(|p| p.trim().parse::<f32>().ok());
    let r = it.next()??;
    let g = it.next()??;
    let b = it.next()??;
    Some(egui::Color32::from_rgb(r as u8, g as u8, b as u8))
}
fn token(theme: &Value, key: &str) -> Option<egui::Color32> {
    theme.get(key).and_then(|v| v.as_str()).and_then(parse_rgb)
}

/// Build egui visuals from the app theme tokens.
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
    }
    if let Some(c) = token(theme, "foreground") {
        v.override_text_color = Some(c);
    }
    if let Some(c) = token(theme, "primary") {
        v.selection.bg_fill = c.gamma_multiply(0.5);
        v.hyperlink_color = c;
    }
    if let Some(c) = token(theme, "border") {
        v.widgets.noninteractive.bg_stroke.color = c;
    }
    ctx.set_visuals(v);
}

impl eframe::App for YantraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // snapshot shared, drop the borrow before rendering/emitting (on_event re-enters JS)
        let (spec, state, theme, theme_dirty, on_event) = {
            let mut sh = self.shared.borrow_mut();
            let td = sh.theme_dirty;
            sh.theme_dirty = false;
            (sh.spec.clone(), sh.state.clone(), sh.theme.clone(), td, sh.on_event.clone())
        };
        if theme_dirty {
            apply_visuals(ctx, &theme);
        }
        ctx.request_repaint_after(Duration::from_millis(50)); // reflect host state pushes

        let wstate = state.get("widgets").cloned().unwrap_or(Value::Null);
        let emit = |ev: serde_json::Value| {
            if let Some(f) = &on_event {
                let _ = f.call1(&JsValue::NULL, &JsValue::from_str(&ev.to_string()));
            }
        };

        let screen = ctx.screen_rect();
        let (cw, ch) = (screen.width(), screen.height());
        egui::CentralPanel::default().show(ctx, |_ui| {});

        let widgets = spec.get("widgets").and_then(|w| w.as_array()).cloned().unwrap_or_default();
        for w in &widgets {
            let ty = w.get("type").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let name = w.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
            // per-widget host state (overrides + display value)
            let ws = wstate.get(&name).cloned().unwrap_or(Value::Null);
            let hidden = w.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false)
                || ws.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false);
            if hidden {
                continue;
            }
            let val_str = ws
                .get("value")
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                })
                .unwrap_or_default();
            let label = ws
                .get("label")
                .and_then(|l| l.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| w.get("label").and_then(|l| l.as_str()).unwrap_or("").to_string());

            let dw = if anchor(w, "anchorH", "scale") == "scale" { 25.0 } else { 100.0 };
            let dh = if anchor(w, "anchorV", "start") == "scale" { 25.0 } else { 48.0 };
            let (sx, ww) = resolve_axis(&anchor(w, "anchorH", "scale"), num(w, "x", 0.0), num(w, "w", dw), cw);
            let (sy, hh) = resolve_axis(&anchor(w, "anchorV", "start"), num(w, "y", 0.0), num(w, "h", dh), ch);
            let pos = screen.min + egui::vec2(sx, sy);
            let size = egui::vec2(ww.max(1.0), hh.max(1.0));

            egui::Area::new(egui::Id::new(("yw", &name)))
                .fixed_pos(pos)
                .order(egui::Order::Middle)
                .show(ctx, |ui| {
                    ui.set_max_size(size);
                    if let Some(c) = ws.get("fg").and_then(|v| v.as_str()).and_then(parse_rgb) {
                        ui.visuals_mut().override_text_color = Some(c);
                    }
                    match ty.as_str() {
                        "label" | "readout" => {
                            ui.label(if val_str.is_empty() { label } else { val_str });
                        }
                        "button" => {
                            if ui.add_sized(size, egui::Button::new(&label)).clicked() {
                                emit(serde_json::json!({ "kind": "press", "name": name }));
                            }
                        }
                        "slider" => {
                            let init = num(w, "value", num(w, "min", 0.0));
                            let v = self.sliders.entry(name.clone()).or_insert(init);
                            let (min, max) = (num(w, "min", 0.0), num(w, "max", 100.0));
                            if ui.add(egui::Slider::new(v, min..=max).text(&label)).changed() {
                                emit(serde_json::json!({ "kind": "value", "name": name, "value": *v }));
                            }
                        }
                        "toggle" => {
                            let on = self.toggles.entry(name.clone()).or_insert(false);
                            if ui.checkbox(on, &label).changed() {
                                emit(serde_json::json!({ "kind": "value", "name": name, "value": *on }));
                            }
                        }
                        "color" => {
                            let c = self.colors.entry(name.clone()).or_insert([128, 128, 128]);
                            ui.horizontal(|ui| {
                                if ui.color_edit_button_srgb(c).changed() {
                                    let hex = format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
                                    emit(serde_json::json!({ "kind": "value", "name": name, "value": hex }));
                                }
                                if !label.is_empty() {
                                    ui.label(&label);
                                }
                            });
                        }
                        other => {
                            ui.weak(format!("{other}?"));
                        }
                    }
                });
        }
    }
}
