// Path-B renderer (slice 1): egui draws a YantraSpec's widgets in the webview.
// The spec is the same loose JSON the rest of the app uses; logic/data still live
// in the native mlua side (bridge comes in slice 2). Flat render for now — tab/
// frame nesting and live values/events are next.
use std::collections::HashMap;

use eframe::egui;
use serde_json::Value;
use wasm_bindgen::prelude::*;

/// Boot egui onto a React-owned canvas with a YantraSpec (JSON string).
#[wasm_bindgen]
pub fn start(canvas: web_sys::HtmlCanvasElement, spec_json: String) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let spec: Value = serde_json::from_str(&spec_json).unwrap_or(Value::Null);
    wasm_bindgen_futures::spawn_local(async move {
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |_cc| Ok(Box::new(YantraApp::new(spec)))),
            )
            .await
            .expect("eframe start failed");
    });
    Ok(())
}

struct YantraApp {
    spec: Value,
    sliders: HashMap<String, f32>,
    toggles: HashMap<String, bool>,
    colors: HashMap<String, [u8; 3]>,
}

impl YantraApp {
    fn new(spec: Value) -> Self {
        Self { spec, sliders: HashMap::new(), toggles: HashMap::new(), colors: HashMap::new() }
    }
}

// ---- the YantraWidget anchor model (ported from skrit.ts axisStyle/resolveAxis) ----
fn num(w: &Value, k: &str, d: f32) -> f32 {
    w.get(k).and_then(|v| v.as_f64()).map(|f| f as f32).unwrap_or(d)
}
fn anchor_h(w: &Value) -> &str {
    w.get("anchorH").and_then(|v| v.as_str()).unwrap_or("scale")
}
fn anchor_v(w: &Value) -> &str {
    w.get("anchorV").and_then(|v| v.as_str()).unwrap_or("start")
}
/// One axis → (start_px, size_px) within a parent of `parent` px.
fn resolve_axis(mode: &str, a: f32, b: f32, parent: f32) -> (f32, f32) {
    match mode {
        "scale" => (a / 100.0 * parent, b / 100.0 * parent),
        "center" => (parent / 2.0 + a - b / 2.0, b),
        "end" => (parent - a - b, b),
        "stretch" => (a, (parent - a - b).max(0.0)),
        _ => (a, b), // start
    }
}

impl eframe::App for YantraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let screen = ctx.screen_rect();
        let (cw, ch) = (screen.width(), screen.height());
        egui::CentralPanel::default().show(ctx, |_ui| {}); // background

        let widgets = self
            .spec
            .get("widgets")
            .and_then(|w| w.as_array())
            .cloned()
            .unwrap_or_default();

        for w in &widgets {
            if w.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false) {
                continue;
            }
            let ty = w.get("type").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let name = w.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
            let label = w.get("label").and_then(|l| l.as_str()).unwrap_or("").to_string();

            let dw = if anchor_h(w) == "scale" { 25.0 } else { 100.0 };
            let dh = if anchor_v(w) == "scale" { 25.0 } else { 48.0 };
            let (sx, ww) = resolve_axis(anchor_h(w), num(w, "x", 0.0), num(w, "w", dw), cw);
            let (sy, hh) = resolve_axis(anchor_v(w), num(w, "y", 0.0), num(w, "h", dh), ch);
            let pos = screen.min + egui::vec2(sx, sy);
            let size = egui::vec2(ww.max(1.0), hh.max(1.0));

            egui::Area::new(egui::Id::new(("yw", &name, &ty)))
                .fixed_pos(pos)
                .order(egui::Order::Middle)
                .show(ctx, |ui| {
                    ui.set_max_size(size);
                    match ty.as_str() {
                        "label" | "readout" => {
                            ui.label(&label);
                        }
                        "button" => {
                            let _ = ui.add_sized(size, egui::Button::new(&label));
                        }
                        "slider" => {
                            let init = num(w, "value", num(w, "min", 0.0));
                            let v = self.sliders.entry(name.clone()).or_insert(init);
                            let (min, max) = (num(w, "min", 0.0), num(w, "max", 100.0));
                            ui.add(egui::Slider::new(v, min..=max).text(&label));
                        }
                        "toggle" => {
                            let on = self.toggles.entry(name.clone()).or_insert(false);
                            ui.checkbox(on, &label);
                        }
                        "color" => {
                            let c = self.colors.entry(name.clone()).or_insert([128, 128, 128]);
                            ui.horizontal(|ui| {
                                ui.color_edit_button_srgb(c);
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
