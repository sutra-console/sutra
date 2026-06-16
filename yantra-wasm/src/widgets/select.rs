use eframe::egui;
use egui::RichText;
use serde_json::{json, Value};

use crate::app::YantraApp;

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
    let opts = w
        .get("options")
        .and_then(|o| o.as_array())
        .cloned()
        .unwrap_or_default();
    let active = app.selects.get(name).copied();
    let mut pick: Option<usize> = None;

    show_card(ui, style, |ui| {
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
        let olabel = opts
            .get(oi)
            .and_then(|o| o.get("label"))
            .and_then(|l| l.as_str())
            .unwrap_or("")
            .to_string();
        emit(json!({ "kind": "select", "name": name, "index": oi, "value": olabel }));
    }
}
