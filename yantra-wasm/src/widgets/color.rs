use eframe::egui;
use egui::RichText;
use serde_json::{json, Value};

use crate::app::YantraApp;

use super::style::{show_card, WidgetStyle};

pub(super) fn render(
    ui: &mut egui::Ui,
    name: &str,
    label: &str,
    style: WidgetStyle,
    emit: &dyn Fn(Value),
    app: &mut YantraApp,
) {
    let muted = ui.visuals().weak_text_color();
    let c = app
        .colors
        .entry(name.to_string())
        .or_insert([128, 128, 128]);
    let mut changed = false;

    show_card(ui, style, |ui| {
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
        emit(
            json!({ "kind": "value", "name": name, "value": format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]) }),
        );
    }
}
