use eframe::egui;
use egui::RichText;
use serde_json::{json, Value};

use crate::app::YantraApp;
use crate::geometry::{num, r2};

use super::style::{show_card, WidgetStyle};

pub(super) fn render(
    ui: &mut egui::Ui,
    name: &str,
    label: &str,
    w: &Value,
    style: WidgetStyle,
    emit: &dyn Fn(Value),
    app: &mut YantraApp,
) {
    let muted = ui.visuals().weak_text_color();
    let init = num(w, "value", num(w, "min", 0.0));
    let v = app.sliders.entry(name.to_string()).or_insert(init);
    let (min, max) = (num(w, "min", 0.0), num(w, "max", 100.0));
    let shown = format!("{}", r2(*v));
    let mut changed = false;

    show_card(ui, style, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).size(11.0).color(muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(&shown).monospace());
            });
        });
        ui.spacing_mut().slider_width = (ui.available_width() - 8.0).max(40.0);
        if ui
            .add(egui::Slider::new(v, min..=max).show_value(false))
            .changed()
        {
            changed = true;
        }
    });

    if changed {
        emit(json!({ "kind": "value", "name": name, "value": *v }));
    }
}
