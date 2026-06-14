// Path-B spike: prove egui compiles to wasm32-unknown-unknown and mounts into a
// React-owned <canvas>. A trivial surface (slider + color picker + label) stands
// in for the real yantra renderer; logic/data stay in the app's native mlua side.
use eframe::egui;
use wasm_bindgen::prelude::*;

/// Boot egui onto an existing canvas (React passes the element). Returns once the
/// async runner is spawned; egui then drives its own rAF loop.
#[wasm_bindgen]
pub fn start(canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    wasm_bindgen_futures::spawn_local(async move {
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|_cc| Ok(Box::new(SpikeApp::default()))),
            )
            .await
            .expect("eframe start failed");
    });
    Ok(())
}

#[derive(Default)]
struct SpikeApp {
    value: f32,
    color: [u8; 3],
}

impl eframe::App for SpikeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("yantra-wasm (egui spike)");
            ui.add(egui::Slider::new(&mut self.value, 0.0..=255.0).text("value"));
            ui.color_edit_button_srgb(&mut self.color);
            ui.label(format!("value = {:.0}", self.value));
        });
    }
}
