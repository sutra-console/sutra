use eframe::egui;
use egui::{Color32, Margin, Rect, Rounding, Stroke, Vec2};
use serde_json::Value;

use crate::theme::parse_rgb;

#[derive(Clone, Copy)]
struct RendererPass {
    fill: Option<Color32>,
    stroke: Option<Color32>,
    stroke_width: f32,
    radius: f32,
    padding_x: f32,
    padding_y: f32,
    has_padding: bool,
}

#[derive(Clone)]
pub(crate) struct WidgetStyle {
    passes: Vec<RendererPass>,
    pub(crate) fg: Option<Color32>,
}

impl WidgetStyle {
    pub(crate) fn from_node(
        node: &Value,
        bg_override: Option<Color32>,
        fg_override: Option<Color32>,
    ) -> Self {
        let fg = fg_override.or_else(|| color_field(node, "fg"));
        let mut passes = node
            .get("renderers")
            .and_then(|v| v.as_array())
            .map(|items| items.iter().filter_map(renderer_pass).collect::<Vec<_>>())
            .unwrap_or_default();
        if let Some(fill) = bg_override {
            passes.push(RendererPass {
                fill: Some(fill),
                stroke: None,
                stroke_width: 0.0,
                radius: 6.0,
                padding_x: 0.0,
                padding_y: 0.0,
                has_padding: false,
            });
        }

        Self { passes, fg }
    }

    pub(crate) fn has_chrome(&self) -> bool {
        !self.passes.is_empty()
    }

    pub(crate) fn fill_or(&self, default: Color32) -> Color32 {
        self.passes
            .iter()
            .filter_map(|pass| pass.fill)
            .last()
            .unwrap_or(default)
    }

    pub(crate) fn stroke_or(&self, default: Color32) -> Stroke {
        let Some(pass) = self.passes.iter().rev().find(|pass| pass.stroke.is_some()) else {
            return Stroke::new(1.0, default);
        };
        if pass.stroke_width <= 0.0 {
            Stroke::NONE
        } else {
            Stroke::new(pass.stroke_width, pass.stroke.unwrap_or(default))
        }
    }

    pub(crate) fn rounding(&self) -> Rounding {
        let radius = self
            .passes
            .iter()
            .rev()
            .map(|pass| pass.radius)
            .next()
            .unwrap_or(6.0);
        Rounding::same(radius)
    }

    pub(crate) fn content_rect(&self, outer: Rect) -> Rect {
        let padding = self
            .passes
            .iter()
            .filter(|pass| pass.has_padding)
            .fold(Vec2::ZERO, |acc, pass| {
                acc + Vec2::new(pass.padding_x, pass.padding_y)
            });
        outer.shrink2(padding)
    }

    pub(crate) fn paint_rect(
        &self,
        ui: &egui::Ui,
        rect: Rect,
        default_fill: Color32,
        default_stroke: Color32,
    ) {
        if self.passes.is_empty() {
            return;
        }
        for pass in &self.passes {
            let stroke = match pass.stroke {
                Some(color) if pass.stroke_width > 0.0 => Stroke::new(pass.stroke_width, color),
                Some(_) => Stroke::NONE,
                None => Stroke::new(0.0, default_stroke),
            };
            ui.painter().rect(
                rect,
                Rounding::same(pass.radius),
                pass.fill.unwrap_or(default_fill),
                stroke,
            );
        }
    }

    fn margin(&self) -> Margin {
        let padding = self
            .passes
            .iter()
            .filter(|pass| pass.has_padding)
            .fold(Vec2::ZERO, |acc, pass| {
                acc + Vec2::new(pass.padding_x, pass.padding_y)
            });
        Margin::symmetric(padding.x, padding.y)
    }
}

pub(super) fn show_card(ui: &mut egui::Ui, style: WidgetStyle, body: impl FnOnce(&mut egui::Ui)) {
    let fill = style.fill_or(ui.visuals().faint_bg_color);
    egui::Frame::none()
        .fill(fill)
        .stroke(style.stroke_or(ui.visuals().widgets.noninteractive.bg_stroke.color))
        .rounding(style.rounding())
        .inner_margin(style.margin())
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            body(ui);
        });
}

fn renderer_pass(node: &Value) -> Option<RendererPass> {
    if !node.is_object() {
        return None;
    }
    let fill = color_field(node, "fill");
    let stroke = match node.get("stroke") {
        Some(Value::Bool(false)) => Some(Color32::TRANSPARENT),
        Some(Value::String(s)) => parse_rgb(s),
        _ => None,
    };
    let stroke_width = number_field(node, "strokeWidth").unwrap_or(1.0);
    let radius = number_field(node, "radius").unwrap_or(6.0);
    let padding = padding(node);
    let (padding_x, padding_y) = padding.unwrap_or((0.0, 0.0));
    if fill.is_none()
        && stroke.is_none()
        && node.get("strokeWidth").is_none()
        && node.get("radius").is_none()
        && padding.is_none()
    {
        return None;
    }
    Some(RendererPass {
        fill,
        stroke,
        stroke_width,
        radius,
        padding_x,
        padding_y,
        has_padding: padding.is_some(),
    })
}

fn color_field(node: &Value, key: &str) -> Option<Color32> {
    node.get(key).and_then(|v| v.as_str()).and_then(parse_rgb)
}

fn number_field(node: &Value, key: &str) -> Option<f32> {
    node.get(key).and_then(|v| v.as_f64()).map(|n| n as f32)
}

fn padding(node: &Value) -> Option<(f32, f32)> {
    let v = node.get("padding")?;
    if let Some(n) = v.as_f64() {
        let n = n as f32;
        return Some((n, n));
    }
    if let Some(obj) = v.as_object() {
        let horizontal = obj
            .get("x")
            .or_else(|| obj.get("horizontal"))
            .and_then(|v| v.as_f64())
            .map(|n| n as f32)
            .unwrap_or(8.0);
        let vertical = obj
            .get("y")
            .or_else(|| obj.get("vertical"))
            .and_then(|v| v.as_f64())
            .map(|n| n as f32)
            .unwrap_or(5.0);
        return Some((horizontal, vertical));
    }
    None
}
