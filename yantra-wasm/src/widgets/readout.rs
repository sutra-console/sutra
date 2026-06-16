use eframe::egui;
use egui::RichText;

use super::style::{show_card, WidgetStyle};

pub(super) fn render(ui: &mut egui::Ui, label: &str, val: &str, style: WidgetStyle) {
    let muted = ui.visuals().weak_text_color();
    let value = if val.is_empty() { "-" } else { val };
    show_card(ui, style, |ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(label).size(11.0).color(muted));
            ui.label(RichText::new(value).size(18.0).monospace());
        });
    });
}
