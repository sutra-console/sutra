mod button;
mod color;
mod label;
mod placeholder;
mod readout;
mod select;
mod slider;
mod style;
mod toggle;

use eframe::egui;
use egui::{Color32, Rect};
use serde_json::Value;

use crate::app::YantraApp;
use crate::theme::parse_rgb;

use self::style::show_card;
pub(crate) use self::style::WidgetStyle;

pub(crate) fn display_of(
    w: &Value,
    wstate: &Value,
) -> (
    String,
    String,
    String,
    String,
    Option<Color32>,
    Option<Color32>,
) {
    let ty = w
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let name = w
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    let ws = wstate.get(&name).cloned().unwrap_or(Value::Null);
    let val = ws
        .get("value")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            o => o.to_string(),
        })
        .unwrap_or_default();
    let label = ws
        .get("label")
        .and_then(|l| l.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            w.get("label")
                .and_then(|l| l.as_str())
                .unwrap_or("")
                .to_string()
        });
    let bg = ws.get("fill").and_then(|v| v.as_str()).and_then(parse_rgb);
    let fg = ws.get("fg").and_then(|v| v.as_str()).and_then(parse_rgb);
    (ty, name, label, val, bg, fg)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_interact_widget(
    ui: &mut egui::Ui,
    rect: Rect,
    ty: &str,
    name: &str,
    label: &str,
    val: &str,
    bg: Option<Color32>,
    fg: Option<Color32>,
    emit: &dyn Fn(Value),
    w: &Value,
    app: &mut YantraApp,
) {
    let _ = rect;
    let style = WidgetStyle::from_node(w, bg, fg);
    if let Some(c) = style.fg {
        ui.visuals_mut().override_text_color = Some(c);
    }

    match ty {
        "label" if style.has_chrome() => show_card(ui, style, |ui| label::render(ui, label, val)),
        "label" => label::render(ui, label, val),
        "button" if style.has_chrome() => {
            show_card(ui, style, |ui| button::render(ui, name, label, emit))
        }
        "button" => button::render(ui, name, label, emit),
        "toggle" if style.has_chrome() => {
            show_card(ui, style, |ui| toggle::render(ui, name, label, emit, app))
        }
        "toggle" => toggle::render(ui, name, label, emit, app),
        "readout" => readout::render(ui, label, val, style),
        "slider" => slider::render(ui, name, label, w, style, emit, app),
        "select" => select::render(ui, name, label, w, style, emit, app),
        "color" => color::render(ui, name, label, style, emit, app),
        other => placeholder::render(ui, other),
    }
}
