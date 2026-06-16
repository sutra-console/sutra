use std::collections::HashMap;

use egui::{Pos2, Rect, Vec2};
use serde_json::Value;

use crate::geometry::widget_rect;
use crate::widgets::WidgetStyle;
/// One placed tabs widget in the editor: chrome to draw + clickable tab bar.
pub(super) struct EditTabBar {
    pub(super) rect: Rect,
    pub(super) bar_rect: Rect,
    pub(super) tabs: Vec<(String, String)>, // (id, label)
    pub(super) active: String,
    pub(super) key: String,
    pub(super) style: WidgetStyle,
}

/// Walk the container tree (honoring active tabs) into a flat placement list:
/// `out_w` = (widget idx, abs rect, parent rect); `out_f` = frame rects;
/// `out_t` = tabs widgets (drawn as chrome, their children become placements).
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_edit_layout(
    container: &str,
    rect: Rect,
    widgets: &[Value],
    frames: &[Value],
    tabs: &HashMap<String, String>,
    out_w: &mut Vec<(usize, Rect, Rect)>,
    out_f: &mut Vec<(usize, Rect, Rect)>, // (frame idx, abs rect, parent rect)
    out_t: &mut Vec<EditTabBar>,
) {
    let is_root = container == "root";
    for (fi, f) in frames.iter().enumerate() {
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
        // hidden frame: skip it and its whole subtree (like a hidden widget)
        if f.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false) {
            continue;
        }
        let fr = widget_rect(f, rect.min, rect.width(), rect.height());
        out_f.push((fi, fr, rect));
        let content = WidgetStyle::from_node(f, None, None).content_rect(fr);
        collect_edit_layout(id, content, widgets, frames, tabs, out_w, out_f, out_t);
    }
    for (i, w) in widgets.iter().enumerate() {
        if w.get("hidden").and_then(|h| h.as_bool()).unwrap_or(false) {
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
            let style = WidgetStyle::from_node(w, None, None);
            let key = w
                .get("name")
                .and_then(|n| n.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("#{i}"));
            let arr = w
                .get("tabs")
                .and_then(|t| t.as_array())
                .cloned()
                .unwrap_or_default();
            let list: Vec<(String, String)> = arr
                .iter()
                .map(|t| {
                    let id = t
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let lbl = t
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&id)
                        .to_string();
                    (id, lbl)
                })
                .collect();
            let first = list.first().map(|t| t.0.clone()).unwrap_or_default();
            let mut active = tabs.get(&key).cloned().unwrap_or_else(|| first.clone());
            if !list.iter().any(|t| t.0 == active) {
                active = first;
            }
            let bar_h = 26.0;
            let inner = style.content_rect(wr);
            let bar_rect = Rect::from_min_size(inner.min, Vec2::new(inner.width(), bar_h));
            let content_rect =
                Rect::from_min_max(Pos2::new(inner.min.x, inner.min.y + bar_h), inner.max);
            out_t.push(EditTabBar {
                rect: wr,
                bar_rect,
                tabs: list,
                active: active.clone(),
                key,
                style,
            });
            collect_edit_layout(
                &active,
                content_rect,
                widgets,
                frames,
                tabs,
                out_w,
                out_f,
                out_t,
            );
        } else {
            out_w.push((i, wr, rect));
        }
    }
}
