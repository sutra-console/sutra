use eframe::egui;
use serde_json::{json, Value};

pub(super) fn render(ui: &mut egui::Ui, name: &str, label: &str, emit: &dyn Fn(Value)) {
    if ui
        .add_sized(ui.available_size(), egui::Button::new(label))
        .clicked()
    {
        emit(json!({ "kind": "press", "name": name }));
    }
}
