use eframe::egui;
use egui::{Align2, FontId, Rounding, Stroke};

pub(super) fn render(ui: &mut egui::Ui, widget_type: &str) {
    let muted = ui.visuals().weak_text_color();
    let r = ui.max_rect();
    ui.painter()
        .rect_stroke(r, Rounding::same(6.0), Stroke::new(1.0, muted));
    ui.painter().text(
        r.center(),
        Align2::CENTER_CENTER,
        format!("{widget_type}?"),
        FontId::proportional(10.0),
        muted,
    );
}
