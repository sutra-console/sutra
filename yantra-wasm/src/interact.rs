use eframe::egui;
use egui::{Color32, Pos2, Rect, Stroke, Vec2};
use serde_json::Value;
use wasm_bindgen::prelude::JsValue;

use crate::app::YantraApp;
use crate::geometry::widget_rect;
use crate::widgets::{display_of, draw_interact_widget, WidgetStyle};

impl YantraApp {
    // ---- interact: styled widget cards, host-authoritative values ----------
    pub(crate) fn interact_ui(&mut self, ctx: &egui::Context) {
        let (spec, state, on_event) = {
            let sh = self.shared.borrow();
            (sh.spec.clone(), sh.state.clone(), sh.on_event.clone())
        };
        let wstate = state.get("widgets").cloned().unwrap_or(Value::Null);
        let fstate = state.get("frames").cloned().unwrap_or(Value::Null); // per-frame overrides (hidden)
        let emit = |ev: Value| {
            if let Some(f) = &on_event {
                let _ = f.call1(&JsValue::NULL, &JsValue::from_str(&ev.to_string()));
            }
        };
        let widgets = spec
            .get("widgets")
            .and_then(|w| w.as_array())
            .cloned()
            .unwrap_or_default();
        let frames = spec
            .get("frames")
            .and_then(|f| f.as_array())
            .cloned()
            .unwrap_or_default();
        let stage = spec.get("stage").cloned().unwrap_or(Value::Null);
        let panel_fill = ctx.style().visuals.panel_fill;
        let mut root = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(panel_fill))
            .show(ctx, |ui| {
                let outer = ui.max_rect();
                let style = WidgetStyle::from_node(&stage, None, None);
                let canvas = if stage.is_object() && style.has_chrome() {
                    style.paint_rect(ui, outer, panel_fill, Color32::TRANSPARENT);
                    style.content_rect(outer)
                } else {
                    outer
                };
                root = Some((
                    canvas,
                    ui.new_child(egui::UiBuilder::new().max_rect(canvas)),
                ));
            });
        // recurse outside the panel closure so `self` is free to be borrowed mutably
        if let Some((canvas, mut ui)) = root {
            self.render_container(
                &mut ui, "root", canvas, &widgets, &frames, &wstate, &fstate, &emit,
            );
        }
    }

    /// Recursively render a container ("root" | frame id | tab-pane id) and its
    /// child frames + widgets, relative to `rect`. Mirrors React's CanvasNodes.
    #[allow(clippy::too_many_arguments)]
    fn render_container(
        &mut self,
        ui: &mut egui::Ui,
        container: &str,
        rect: Rect,
        widgets: &[Value],
        frames: &[Value],
        wstate: &Value,
        fstate: &Value,
        emit: &dyn Fn(Value),
    ) {
        let is_root = container == "root";
        // child frames first (so widgets paint on top within this container)
        for f in frames {
            let id = f.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let parent = f.get("parent").and_then(|v| v.as_str());
            let tab = f.get("tab").and_then(|v| v.as_str());
            let is_child = if is_root {
                parent.is_none() && tab.is_none()
            } else {
                tab == Some(container) || (parent == Some(container) && tab.is_none())
            };
            if !is_child {
                continue;
            }
            // frame visibility: static `hidden` OR a host/script override
            let fhidden = f.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false)
                || fstate
                    .get(id)
                    .and_then(|s| s.get("hidden"))
                    .and_then(|h| h.as_bool())
                    .unwrap_or(false);
            if fhidden {
                continue;
            }
            let fr = widget_rect(f, rect.min, rect.width(), rect.height());
            let style = WidgetStyle::from_node(f, None, None);
            style.paint_rect(
                ui,
                fr,
                Color32::TRANSPARENT,
                ui.visuals()
                    .widgets
                    .noninteractive
                    .bg_stroke
                    .color
                    .gamma_multiply(0.6),
            );
            let clip = f.get("clip").and_then(|c| c.as_bool()).unwrap_or(true);
            let content = style.content_rect(fr);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content));
            if clip {
                child.set_clip_rect(fr);
            }
            self.render_container(
                &mut child, id, content, widgets, frames, wstate, fstate, emit,
            );
        }
        // child widgets
        for (i, w) in widgets.iter().enumerate() {
            let name = w.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let ws = wstate.get(name).cloned().unwrap_or(Value::Null);
            let hidden = w.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false)
                || ws.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false);
            if hidden {
                continue;
            }
            let frame = w.get("frame").and_then(|v| v.as_str());
            let tab = w.get("tab").and_then(|v| v.as_str());
            let is_child = if is_root {
                frame.is_none() && tab.is_none()
            } else {
                tab == Some(container) || (frame == Some(container) && tab.is_none())
            };
            if !is_child {
                continue;
            }
            let wr = widget_rect(w, rect.min, rect.width(), rect.height());
            if w.get("type").and_then(|t| t.as_str()) == Some("tabs") {
                self.render_tabs(ui, i, w, wr, widgets, frames, wstate, fstate, emit);
                continue;
            }
            let (ty, _n, label, val, bg, fg) = display_of(w, wstate);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(wr));
            child.set_clip_rect(wr);
            draw_interact_widget(
                &mut child, wr, &ty, name, &label, &val, bg, fg, emit, w, self,
            );
        }
    }

    /// A `tabs` widget: a card with a tab bar over the active pane's content.
    #[allow(clippy::too_many_arguments)]
    fn render_tabs(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        w: &Value,
        rect: Rect,
        widgets: &[Value],
        frames: &[Value],
        wstate: &Value,
        fstate: &Value,
        emit: &dyn Fn(Value),
    ) {
        let tabs = w
            .get("tabs")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        let key = w
            .get("name")
            .and_then(|n| n.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("#{i}"));
        let first = tabs
            .first()
            .and_then(|t| t.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut active = self
            .tabs
            .get(&key)
            .cloned()
            .unwrap_or_else(|| first.clone());
        if !tabs
            .iter()
            .any(|t| t.get("id").and_then(|v| v.as_str()) == Some(active.as_str()))
        {
            active = first;
        }

        // card frame
        let style = WidgetStyle::from_node(w, None, None);
        style.paint_rect(
            ui,
            rect,
            ui.visuals().widgets.noninteractive.bg_fill,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        );
        let bar_h = 26.0;
        let inner = style.content_rect(rect);
        let bar_rect = Rect::from_min_size(inner.min, Vec2::new(inner.width(), bar_h));
        let content_rect =
            Rect::from_min_max(Pos2::new(inner.min.x, inner.min.y + bar_h), inner.max);

        let mut clicked: Option<String> = None;
        let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(bar_rect.shrink(4.0)));
        bar.set_clip_rect(bar_rect);
        bar.horizontal_wrapped(|ui| {
            for t in &tabs {
                let tid = t
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tlabel = t
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&tid)
                    .to_string();
                if ui.selectable_label(active == tid, tlabel).clicked() {
                    clicked = Some(tid);
                }
            }
        });
        if let Some(t) = clicked {
            active = t;
        }
        self.tabs.insert(key, active.clone());

        // separator under the bar
        ui.painter().line_segment(
            [
                Pos2::new(rect.min.x, rect.min.y + bar_h),
                Pos2::new(rect.max.x, rect.min.y + bar_h),
            ],
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );

        let mut content = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
        content.set_clip_rect(content_rect);
        self.render_container(
            &mut content,
            &active,
            content_rect,
            widgets,
            frames,
            wstate,
            fstate,
            emit,
        );
    }
}
