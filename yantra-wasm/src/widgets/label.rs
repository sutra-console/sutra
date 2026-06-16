use eframe::egui;
use egui::RichText;

pub(super) fn render(ui: &mut egui::Ui, label: &str, val: &str) {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.label(
            RichText::new(if val.is_empty() { label } else { val })
                .size(12.0)
                .strong(),
        );
    });
}
