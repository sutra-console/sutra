use egui::{Color32, Rounding, Stroke};
use serde_json::Value;

pub(crate) fn parse_rgb(s: &str) -> Option<Color32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    let inner = s.trim_start_matches("rgba").trim_start_matches("rgb");
    let inner = inner.trim_matches(|c| c == '(' || c == ')');
    let mut it = inner.split(',').map(|p| p.trim().parse::<f32>().ok());
    let r = it.next()??;
    let g = it.next()??;
    let b = it.next()??;
    Some(Color32::from_rgb(r as u8, g as u8, b as u8))
}

fn parse_hex(hex: &str) -> Option<Color32> {
    let expanded;
    let hex = if hex.len() == 3 {
        expanded = hex.chars().flat_map(|c| [c, c]).collect::<String>();
        expanded.as_str()
    } else {
        hex
    };
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}
pub(crate) fn token(theme: &Value, key: &str) -> Option<Color32> {
    theme.get(key).and_then(|v| v.as_str()).and_then(parse_rgb)
}
pub(crate) fn apply_visuals(ctx: &egui::Context, theme: &Value) {
    let bg = token(theme, "background");
    let dark = bg
        .map(|c| (c.r() as u32 + c.g() as u32 + c.b() as u32) < 384)
        .unwrap_or(true);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    if let Some(c) = bg {
        v.panel_fill = c;
    }
    if let Some(c) = token(theme, "card") {
        v.window_fill = c;
        v.extreme_bg_color = c;
        v.widgets.noninteractive.bg_fill = c;
        v.widgets.inactive.bg_fill = c;
    }
    if let Some(c) = token(theme, "foreground") {
        v.override_text_color = Some(c);
    }
    if let Some(c) = token(theme, "primary") {
        v.selection.bg_fill = c.gamma_multiply(0.35);
        v.selection.stroke.color = c;
        v.hyperlink_color = c;
    }
    if let Some(c) = token(theme, "border") {
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, c);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, c);
    }
    v.widgets.inactive.rounding = Rounding::same(6.0);
    v.widgets.hovered.rounding = Rounding::same(6.0);
    v.widgets.active.rounding = Rounding::same(6.0);
    ctx.set_visuals(v);
}
