use eframe::egui;
use egui::{RichText, Stroke};
use serde_json::json;

use crate::app::YantraApp;
use crate::geometry::{anchor, num, r2};
use crate::state::Sel;

use super::frame_props::edit_frame_props;
use super::selection::{
    build_layer_rows, move_in_array, move_many_in_array, sel_widgets, LayerDrag,
};
use super::state_ops::{delete_frame, delete_widget, push_undo};

#[derive(Clone, Copy)]
struct LayerClick {
    item: Sel,
    row_index: usize,
    shift: bool,
    toggle: bool,
}

impl YantraApp {
    pub(super) fn inspector_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("ed_inspector")
            .default_width(230.0)
            .show(ctx, |ui| {
                let mut sh = self.shared.borrow_mut();
                let count = sh
                    .spec
                    .get("widgets")
                    .and_then(|w| w.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                let fcount = sh
                    .spec
                    .get("frames")
                    .and_then(|f| f.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                // frame ids (for the membership dropdown)
                let frame_ids: Vec<String> = (0..fcount)
                    .map(|i| {
                        sh.spec["frames"][i]
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string()
                    })
                    .collect();

                let accent_c = ui.visuals().selection.stroke.color;
                let muted_c = ui.visuals().weak_text_color();
                ui.add_space(4.0);
                ui.strong("Layers");
                ui.label(RichText::new("drag to reorder · L = lock").size(9.0).weak());
                let mut sel_click: Option<LayerClick> = None;
                let mut hide_toggle: Option<usize> = None;
                let mut del: Option<usize> = None;
                let mut hide_frame: Option<usize> = None;
                let mut del_frame: Option<usize> = None;
                let mut collapse_frame: Option<usize> = None;
                let mut lock_w: Option<usize> = None;
                let mut lock_f: Option<usize> = None;
                let mut reorder_w: Option<(usize, usize)> = None; // (from, to) within widgets
                let mut reorder_f: Option<(usize, usize)> = None; // (from, to) within frames
                let mut reparent_w: Option<(usize, usize)> = None; // (widget idx, frame idx) — drop widget on a frame row
                let mut group_click = false; // layers "Group into Frame" → sh.pending_group

                // snapshot the tree structure, then flatten to (depth, item) display rows
                let frames_meta: Vec<(usize, String, Option<String>, bool)> = (0..fcount)
                    .map(|fi| {
                        let f = &sh.spec["frames"][fi];
                        (
                            fi,
                            f.get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            f.get("parent").and_then(|v| v.as_str()).map(str::to_string),
                            f.get("collapsed")
                                .and_then(|c| c.as_bool())
                                .unwrap_or(false),
                        )
                    })
                    .collect();
                let widget_frame: Vec<(usize, Option<String>)> = (0..count)
                    .map(|i| {
                        (
                            i,
                            sh.spec["widgets"][i]
                                .get("frame")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        )
                    })
                    .collect();
                let mut rows: Vec<(usize, LayerDrag)> = Vec::new();
                build_layer_rows(None, 0, &frames_meta, &widget_frame, &mut rows);

                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for (row_index, (depth, item)) in rows.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.add_space(*depth as f32 * 12.0);
                                match *item {
                                    LayerDrag::Frame(fi) => {
                                        let f = &sh.spec["frames"][fi];
                                        let nm = f
                                            .get("name")
                                            .and_then(|n| n.as_str())
                                            .or_else(|| f.get("id").and_then(|n| n.as_str()))
                                            .unwrap_or("")
                                            .to_string();
                                        let hidden = f
                                            .get("hidden")
                                            .and_then(|h| h.as_bool())
                                            .unwrap_or(false);
                                        let collapsed = f
                                            .get("collapsed")
                                            .and_then(|c| c.as_bool())
                                            .unwrap_or(false);
                                        let is_sel = sh.selected.contains(&Sel::Frame(fi));
                                        if ui
                                            .add(
                                                egui::Button::new(if collapsed {
                                                    ">"
                                                } else {
                                                    "v"
                                                })
                                                .small()
                                                .frame(false),
                                            )
                                            .on_hover_text("Expand/collapse")
                                            .clicked()
                                        {
                                            collapse_frame = Some(fi);
                                        }
                                        if ui
                                            .add(
                                                egui::Button::new(if hidden { "-" } else { "o" })
                                                    .small()
                                                    .frame(false),
                                            )
                                            .on_hover_text("Show/hide")
                                            .clicked()
                                        {
                                            hide_frame = Some(fi);
                                        }
                                        let flocked = f
                                            .get("locked")
                                            .and_then(|l| l.as_bool())
                                            .unwrap_or(false);
                                        if ui
                                            .add(
                                                egui::Button::new(RichText::new("L").color(
                                                    if flocked { accent_c } else { muted_c },
                                                ))
                                                .small()
                                                .frame(false),
                                            )
                                            .on_hover_text("Lock/unlock (canvas)")
                                            .clicked()
                                        {
                                            lock_f = Some(fi);
                                        }
                                        // one widget, click_and_drag sense: a click (no movement) selects;
                                        // a press past egui's drag threshold starts a drag. Click is preserved.
                                        let lab = ui.selectable_label(is_sel, format!("[] {nm}"));
                                        let resp = ui.interact(
                                            lab.rect,
                                            lab.id,
                                            egui::Sense::click_and_drag(),
                                        );
                                        if lab.clicked() {
                                            let (shift, toggle_modifier) = ui.input(|i| {
                                                (
                                                    i.modifiers.shift,
                                                    i.modifiers.ctrl || i.modifiers.command,
                                                )
                                            });
                                            sel_click = Some(LayerClick {
                                                item: Sel::Frame(fi),
                                                row_index,
                                                shift,
                                                toggle: toggle_modifier,
                                            });
                                        }
                                        if resp.drag_started() {
                                            egui::DragAndDrop::set_payload(
                                                ui.ctx(),
                                                LayerDrag::Frame(fi),
                                            );
                                        }
                                        if resp.dragged() {
                                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                        }
                                        if resp.dnd_hover_payload::<LayerDrag>().is_some() {
                                            ui.painter().hline(
                                                resp.rect.x_range(),
                                                resp.rect.top(),
                                                Stroke::new(2.0, accent_c),
                                            );
                                        }
                                        resp.context_menu(|ui| {
                                            if ui.button("Delete frame").clicked() {
                                                del_frame = Some(fi);
                                                ui.close_menu();
                                            }
                                        });
                                        if let Some(p) = resp.dnd_release_payload::<LayerDrag>() {
                                            match *p {
                                                LayerDrag::Frame(from) => {
                                                    reorder_f = Some((from, fi))
                                                }
                                                LayerDrag::Widget(from) => {
                                                    reparent_w = Some((from, fi))
                                                }
                                            }
                                        }
                                    }
                                    LayerDrag::Widget(i) => {
                                        let w = &sh.spec["widgets"][i];
                                        let name = w
                                            .get("name")
                                            .and_then(|n| n.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        let ty = w
                                            .get("type")
                                            .and_then(|t| t.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        let hidden = w
                                            .get("hidden")
                                            .and_then(|h| h.as_bool())
                                            .unwrap_or(false);
                                        let is_sel = sh.selected.contains(&Sel::Widget(i));
                                        if ui
                                            .add(
                                                egui::Button::new(if hidden { "-" } else { "o" })
                                                    .small()
                                                    .frame(false),
                                            )
                                            .on_hover_text("Show/hide")
                                            .clicked()
                                        {
                                            hide_toggle = Some(i);
                                        }
                                        let wlocked = w
                                            .get("locked")
                                            .and_then(|l| l.as_bool())
                                            .unwrap_or(false);
                                        if ui
                                            .add(
                                                egui::Button::new(RichText::new("L").color(
                                                    if wlocked { accent_c } else { muted_c },
                                                ))
                                                .small()
                                                .frame(false),
                                            )
                                            .on_hover_text("Lock/unlock (canvas)")
                                            .clicked()
                                        {
                                            lock_w = Some(i);
                                        }
                                        let txt = if name.is_empty() {
                                            format!("{ty} #{i}")
                                        } else {
                                            format!("{name}  ({ty})")
                                        };
                                        let lab = ui.selectable_label(is_sel, txt);
                                        let resp = ui.interact(
                                            lab.rect,
                                            lab.id,
                                            egui::Sense::click_and_drag(),
                                        );
                                        if lab.clicked() {
                                            let (shift, toggle_modifier) = ui.input(|i| {
                                                (
                                                    i.modifiers.shift,
                                                    i.modifiers.ctrl || i.modifiers.command,
                                                )
                                            });
                                            sel_click = Some(LayerClick {
                                                item: Sel::Widget(i),
                                                row_index,
                                                shift,
                                                toggle: toggle_modifier,
                                            });
                                        }
                                        if resp.drag_started() {
                                            egui::DragAndDrop::set_payload(
                                                ui.ctx(),
                                                LayerDrag::Widget(i),
                                            );
                                        }
                                        if resp.dragged() {
                                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                        }
                                        if resp.dnd_hover_payload::<LayerDrag>().is_some() {
                                            ui.painter().hline(
                                                resp.rect.x_range(),
                                                resp.rect.top(),
                                                Stroke::new(2.0, accent_c),
                                            );
                                        }
                                        resp.context_menu(|ui| {
                                            if ui.button("Group into Frame").clicked() {
                                                group_click = true;
                                                ui.close_menu();
                                            }
                                            if ui.button("Delete").clicked() {
                                                del = Some(i);
                                                ui.close_menu();
                                            }
                                        });
                                        if let Some(p) = resp.dnd_release_payload::<LayerDrag>() {
                                            if let LayerDrag::Widget(from) = *p {
                                                reorder_w = Some((from, i));
                                            }
                                        }
                                    }
                                }
                            });
                        }
                    });
                if let Some(fi) = collapse_frame {
                    let cur = sh.spec["frames"][fi]
                        .get("collapsed")
                        .and_then(|c| c.as_bool())
                        .unwrap_or(false);
                    sh.spec["frames"][fi]["collapsed"] = json!(!cur);
                }
                if let Some(i) = lock_w {
                    push_undo(&mut sh);
                    let cur = sh.spec["widgets"][i]
                        .get("locked")
                        .and_then(|l| l.as_bool())
                        .unwrap_or(false);
                    sh.spec["widgets"][i]["locked"] = json!(!cur);
                }
                if let Some(fi) = lock_f {
                    push_undo(&mut sh);
                    let cur = sh.spec["frames"][fi]
                        .get("locked")
                        .and_then(|l| l.as_bool())
                        .unwrap_or(false);
                    sh.spec["frames"][fi]["locked"] = json!(!cur);
                }
                if let Some(click) = sel_click {
                    let (selected, anchor) =
                        apply_layer_selection(&sh.selected, sh.layer_anchor_row, &rows, click);
                    sh.selected = selected;
                    sh.layer_anchor_row = anchor;
                }
                if let Some(fi) = hide_frame {
                    push_undo(&mut sh);
                    let cur = sh.spec["frames"][fi]
                        .get("hidden")
                        .and_then(|h| h.as_bool())
                        .unwrap_or(false);
                    sh.spec["frames"][fi]["hidden"] = json!(!cur);
                }
                if let Some(fi) = del_frame {
                    push_undo(&mut sh);
                    delete_frame(&mut sh.spec, fi);
                    sh.selected.clear();
                }
                if let Some(i) = hide_toggle {
                    push_undo(&mut sh);
                    let cur = sh.spec["widgets"][i]
                        .get("hidden")
                        .and_then(|h| h.as_bool())
                        .unwrap_or(false);
                    sh.spec["widgets"][i]["hidden"] = json!(!cur);
                }
                if let Some(i) = del {
                    push_undo(&mut sh);
                    delete_widget(&mut sh.spec, i);
                    sh.selected.clear();
                }
                if let Some((from, to)) = reorder_w {
                    if from != to {
                        push_undo(&mut sh);
                        // drag the whole widget selection if the dragged row is part of it
                        let wsel = sel_widgets(&sh.selected);
                        if wsel.len() > 1 && wsel.contains(&from) {
                            let n = wsel.len();
                            let start = move_many_in_array(&mut sh.spec, "widgets", wsel, to);
                            sh.selected = (start..start + n).map(Sel::Widget).collect();
                        } else {
                            let ni = move_in_array(&mut sh.spec, "widgets", from, to);
                            sh.selected = vec![Sel::Widget(ni)];
                        }
                    }
                }
                if let Some((from, to)) = reorder_f {
                    if from != to {
                        push_undo(&mut sh);
                        let ni = move_in_array(&mut sh.spec, "frames", from, to);
                        sh.selected = vec![Sel::Frame(ni)];
                    }
                }
                if let Some((wi, fi)) = reparent_w {
                    // drop a widget (or the whole multi-widget selection) into frame fi
                    let fid = sh.spec["frames"][fi]
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !fid.is_empty() {
                        push_undo(&mut sh);
                        let wsel = sel_widgets(&sh.selected);
                        let targets = if wsel.len() > 1 && wsel.contains(&wi) {
                            wsel
                        } else {
                            vec![wi]
                        };
                        for t in targets {
                            if let Some(w) = sh
                                .spec
                                .get_mut("widgets")
                                .and_then(|a| a.as_array_mut())
                                .and_then(|a| a.get_mut(t))
                            {
                                w["frame"] = json!(fid);
                                if let Some(o) = w.as_object_mut() {
                                    o.remove("tab");
                                }
                            }
                        }
                    }
                }
                if group_click {
                    sh.pending_group = true; // the canvas pass has the geometry to build the frame
                }

                ui.separator();
                ui.strong("Properties");

                let sel = sh.selected.clone();
                if sel.len() != 1 {
                    ui.weak(if sel.is_empty() {
                        "No selection"
                    } else {
                        "Multiple selected"
                    });
                    return;
                }
                // single selection → frame or widget inspector
                let i = match sel[0] {
                    Sel::Frame(fi) => {
                        if fi >= fcount {
                            sh.selected.clear();
                            return;
                        }
                        let f = sh.spec["frames"][fi].clone();
                        edit_frame_props(ui, &mut sh, fi, &f);
                        return;
                    }
                    Sel::Widget(wi) => wi,
                };
                if i >= count {
                    return;
                }
                let w = sh.spec["widgets"][i].clone();
                let mut name = w
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut label = w
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let (mut x, mut y, mut ww, mut hh) = (
                    num(&w, "x", 0.0),
                    num(&w, "y", 0.0),
                    num(&w, "w", 30.0),
                    num(&w, "h", 48.0),
                );
                let mut ah = anchor(&w, "anchorH", "scale");
                let mut av = anchor(&w, "anchorV", "start");
                // current frame membership ("" = root)
                let mut member = w
                    .get("frame")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut changed = false;
                let mut snapshot = false; // gesture start → one undo entry

                ui.add_space(2.0);
                ui.label(
                    RichText::new(w.get("type").and_then(|t| t.as_str()).unwrap_or("?")).weak(),
                );
                egui::Grid::new("props")
                    .num_columns(2)
                    .spacing([6.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("name");
                        let r = ui.text_edit_singleline(&mut name);
                        if r.gained_focus() {
                            snapshot = true;
                        }
                        if r.changed() {
                            changed = true;
                        }
                        ui.end_row();
                        ui.label("label");
                        let r = ui.text_edit_singleline(&mut label);
                        if r.gained_focus() {
                            snapshot = true;
                        }
                        if r.changed() {
                            changed = true;
                        }
                        ui.end_row();
                        for (lbl, v) in
                            [("x", &mut x), ("y", &mut y), ("w", &mut ww), ("h", &mut hh)]
                        {
                            ui.label(lbl);
                            let r = ui.add(egui::DragValue::new(v).speed(0.5));
                            if r.drag_started() {
                                snapshot = true;
                            }
                            if r.changed() {
                                changed = true;
                            }
                            ui.end_row();
                        }
                        for (lbl, cur, id) in
                            [("anchor H", &mut ah, "ah"), ("anchor V", &mut av, "av")]
                        {
                            ui.label(lbl);
                            let before = cur.clone();
                            egui::ComboBox::from_id_salt(id)
                                .selected_text(cur.as_str())
                                .show_ui(ui, |ui| {
                                    for opt in ["scale", "start", "center", "end", "stretch"] {
                                        ui.selectable_value(cur, opt.to_string(), opt);
                                    }
                                });
                            if *cur != before {
                                snapshot = true;
                                changed = true;
                            }
                            ui.end_row();
                        }
                        // frame membership
                        ui.label("in frame");
                        let before = member.clone();
                        let shown = if member.is_empty() {
                            "(root)".to_string()
                        } else {
                            member.clone()
                        };
                        egui::ComboBox::from_id_salt("memb")
                            .selected_text(shown)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut member, String::new(), "(root)");
                                for fid in &frame_ids {
                                    ui.selectable_value(&mut member, fid.clone(), fid.as_str());
                                }
                            });
                        if member != before {
                            snapshot = true;
                            changed = true;
                        }
                        ui.end_row();
                    });
                if changed {
                    if snapshot {
                        push_undo(&mut sh);
                    }
                    let wm = &mut sh.spec["widgets"][i];
                    wm["name"] = json!(name);
                    wm["label"] = json!(label);
                    wm["x"] = json!(r2(x));
                    wm["y"] = json!(r2(y));
                    wm["w"] = json!(r2(ww));
                    wm["h"] = json!(r2(hh));
                    wm["anchorH"] = json!(ah);
                    wm["anchorV"] = json!(av);
                    if member.is_empty() {
                        wm.as_object_mut().map(|o| o.remove("frame"));
                    } else {
                        wm["frame"] = json!(member);
                    }
                }
            });
    }
}

fn apply_layer_selection(
    selected: &[Sel],
    layer_anchor_row: Option<usize>,
    rows: &[(usize, LayerDrag)],
    click: LayerClick,
) -> (Vec<Sel>, Option<usize>) {
    if click.shift {
        let Some(clicked_pos) = valid_row_index(rows, click.row_index) else {
            return (vec![click.item], Some(click.row_index));
        };
        let anchor = layer_anchor_row
            .and_then(|row| valid_row_index(rows, row))
            .or_else(|| selected.last().and_then(|item| row_position(rows, *item)))
            .unwrap_or(clicked_pos);
        let (start, end) = if anchor <= clicked_pos {
            (anchor, clicked_pos)
        } else {
            (clicked_pos, anchor)
        };
        let range: Vec<Sel> = rows[start..=end]
            .iter()
            .map(|(_, item)| layer_item_selection(*item))
            .collect();
        if click.toggle {
            let selected = selected
                .iter()
                .copied()
                .filter(|item| !range.contains(item))
                .collect();
            return (selected, layer_anchor_row);
        } else {
            return (range, layer_anchor_row);
        }
    }

    if click.toggle {
        let mut selected = selected.to_vec();
        if let Some(p) = selected.iter().position(|x| *x == click.item) {
            selected.remove(p);
        } else {
            selected.push(click.item);
        }
        return (selected, Some(click.row_index));
    }

    (vec![click.item], Some(click.row_index))
}

fn valid_row_index(rows: &[(usize, LayerDrag)], row: usize) -> Option<usize> {
    (row < rows.len()).then_some(row)
}

fn row_position(rows: &[(usize, LayerDrag)], item: Sel) -> Option<usize> {
    rows.iter()
        .position(|(_, row_item)| layer_item_selection(*row_item) == item)
}

fn layer_item_selection(item: LayerDrag) -> Sel {
    match item {
        LayerDrag::Widget(i) => Sel::Widget(i),
        LayerDrag::Frame(i) => Sel::Frame(i),
    }
}
