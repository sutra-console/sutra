use eframe::egui;
use egui::Color32;
use serde_json::{json, Value};

use crate::app::YantraApp;

pub(super) fn render(
    ui: &mut egui::Ui,
    name: &str,
    label: &str,
    emit: &dyn Fn(Value),
    app: &mut YantraApp,
) {
    let on = app.toggles.entry(name.to_string()).or_insert(false);
    let txt = if label.is_empty() {
        (if *on { "on" } else { "off" }).to_string()
    } else {
        label.to_string()
    };
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
