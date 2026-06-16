use std::collections::HashMap;

mod drag;
mod frame_props;
mod inspector;
mod layout;
mod selection;
mod state_ops;

use eframe::egui;
use egui::{Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use serde_json::Value;
use wasm_bindgen::prelude::JsValue;

use crate::app::YantraApp;
use crate::state::Sel;
use crate::widgets::{display_of, draw_interact_widget, WidgetStyle};

use self::drag::{apply_drag, capture_drag, group_into_frame};
use self::layout::{collect_edit_layout, EditTabBar};
use self::selection::{click_target, sel_widgets};
use self::state_ops::{
    add_frame, add_widget, align_selected, delete_frame, delete_widget, draw_grid, push_undo, redo,
    undo,
};

impl YantraApp {
    pub(crate) fn edit_ui(&mut self, ctx: &egui::Context) {
        // keyboard: undo/redo, delete, deselect — suppressed while a text field is focused.
        let typing = ctx.wants_keyboard_input();
        let (undo_key, redo_key, del_key, esc_key) = ctx.input(|i| {
            let z = i.modifiers.command && i.key_pressed(egui::Key::Z);
            (
                z && !i.modifiers.shift,
                (z && i.modifiers.shift) || (i.modifiers.command && i.key_pressed(egui::Key::Y)),
                !typing
                    && (i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)),
                !typing && i.key_pressed(egui::Key::Escape),
            )
        });

        let mut add: Option<String> = None;
        let (mut do_delete, mut do_save, mut do_undo, mut do_redo) =
            (del_key, false, undo_key, redo_key);
        let mut align: Option<&str> = None;
        let mut do_group = false; // wrap the selection in a new frame (resolved in the canvas pass)
        egui::TopBottomPanel::top("ed_toolbar").show(ctx, |ui| {
            // ASCII-only labels: egui's default font lacks box-drawing/emoji glyphs.
            ui.horizontal_wrapped(|ui| {
                ui.menu_button("Add", |ui| {
                    for t in [
                        "button", "slider", "toggle", "readout", "label", "color", "select",
                        "frame",
                    ] {
                        if ui.button(t).clicked() {
                            add = Some(t.to_string());
                            ui.close_menu();
                        }
                    }
                });
                if ui
                    .button("Delete")
                    .on_hover_text("Delete selected")
                    .clicked()
                {
                    do_delete = true;
                }
                ui.separator();
                if ui.button("Undo").clicked() {
                    do_undo = true;
                }
                if ui.button("Redo").clicked() {
                    do_redo = true;
                }
                ui.separator();
                ui.label("Align");
                for (lbl, key, tip) in [
                    ("L", "left", "Align left"),
                    ("C", "cx", "Align centers (horizontal)"),
                    ("R", "right", "Align right"),
                    ("T", "top", "Align top"),
                    ("M", "cy", "Align middles (vertical)"),
                    ("B", "bottom", "Align bottom"),
                ] {
                    if ui.button(lbl).on_hover_text(tip).clicked() {
                        align = Some(key);
                    }
                }
                ui.separator();
                if ui
                    .button("Group")
                    .on_hover_text("Wrap the selected widgets in a new frame")
                    .clicked()
                {
                    do_group = true;
                }
                if ui.button("Save").clicked() {
                    do_save = true;
                }
            });
        });

        // toolbar mutations
        {
            let mut sh = self.shared.borrow_mut();
            if esc_key {
                sh.selected.clear();
            }
            if let Some(t) = add {
                push_undo(&mut sh);
                if t == "frame" {
                    add_frame(&mut sh.spec);
                    let n = sh
                        .spec
                        .get("frames")
                        .and_then(|f| f.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    sh.selected = if n > 0 {
                        vec![Sel::Frame(n - 1)]
                    } else {
                        vec![]
                    };
                } else {
                    add_widget(&mut sh.spec, &t);
                    let n = sh
                        .spec
                        .get("widgets")
                        .and_then(|w| w.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    sh.selected = if n > 0 {
                        vec![Sel::Widget(n - 1)]
                    } else {
                        vec![]
                    };
                }
            }
            if do_delete && !sh.selected.is_empty() {
                push_undo(&mut sh);
                let sel = sh.selected.clone();
                // remove widgets high→low, then frames high→low (orphaned children → root)
                let mut wi = sel_widgets(&sel);
                wi.sort_unstable();
                for i in wi.iter().rev() {
                    delete_widget(&mut sh.spec, *i);
                }
                let mut fi: Vec<usize> = sel
                    .iter()
                    .filter_map(|s| {
                        if let Sel::Frame(f) = s {
                            Some(*f)
                        } else {
                            None
                        }
                    })
                    .collect();
                fi.sort_unstable();
                for f in fi.iter().rev() {
                    delete_frame(&mut sh.spec, *f);
                }
                sh.selected.clear();
            }
            if let Some(key) = align {
                push_undo(&mut sh);
                align_selected(&mut sh, key);
            }
            if do_undo {
                undo(&mut sh);
            }
            if do_redo {
                redo(&mut sh);
            }
            if do_save {
                let s = sh.spec.to_string();
                if let Some(f) = &sh.on_save {
                    let _ = f.call1(&JsValue::NULL, &JsValue::from_str(&s));
                }
            }
        }

        self.inspector_panel(ctx);

        let bg = ctx.style().visuals.panel_fill;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(bg))
            .show(ctx, |ui| {
                let stage = self
                    .shared
                    .borrow()
                    .spec
                    .get("stage")
                    .cloned()
                    .unwrap_or(Value::Null);
                let outer = ui.max_rect();
                let stage_style = WidgetStyle::from_node(&stage, None, None);
                let canvas = if stage.is_object() && stage_style.has_chrome() {
                    stage_style.paint_rect(ui, outer, bg, Color32::TRANSPARENT);
                    stage_style.content_rect(outer)
                } else {
                    outer
                };
                draw_grid(ui, canvas);
                let widgets = self
                    .shared
                    .borrow()
                    .spec
                    .get("widgets")
                    .and_then(|w| w.as_array())
                    .cloned()
                    .unwrap_or_default();
                let frames = self
                    .shared
                    .borrow()
                    .spec
                    .get("frames")
                    .and_then(|f| f.as_array())
                    .cloned()
                    .unwrap_or_default();
                let selected = self.shared.borrow().selected.clone();
                let wstate = self
                    .shared
                    .borrow()
                    .state
                    .get("widgets")
                    .cloned()
                    .unwrap_or(Value::Null);
                let shift = ui.input(|i| i.modifiers.shift);
                let accent = ui.visuals().selection.stroke.color;
                let border = ui.visuals().widgets.noninteractive.bg_stroke.color;

                // walk the container tree (honoring active tabs) into placements
                let tabs_snapshot = self.tabs.clone();
                let mut placements: Vec<(usize, Rect, Rect)> = Vec::new();
                let mut frame_rects: Vec<(usize, Rect, Rect)> = Vec::new();
                let mut tabbars: Vec<EditTabBar> = Vec::new();
                collect_edit_layout(
                    "root",
                    canvas,
                    &widgets,
                    &frames,
                    &tabs_snapshot,
                    &mut placements,
                    &mut frame_rects,
                    &mut tabbars,
                );
                let parent_of: HashMap<usize, Rect> =
                    placements.iter().map(|(i, _, p)| (*i, *p)).collect();
                let frame_parent_of: HashMap<usize, Rect> =
                    frame_rects.iter().map(|(i, _, p)| (*i, *p)).collect();
                let frame_by_id: HashMap<String, usize> = (0..frames.len())
                    .filter_map(|fi| {
                        frames[fi]
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(|id| (id.to_string(), fi))
                    })
                    .collect();

                // empty-canvas click clears selection; drag = marquee. Registered FIRST so
                // the frame/widget interactions (added after) sit on top and win the pointer.
                let bg_resp = ui.interact(canvas, egui::Id::new("ed_bg"), Sense::click_and_drag());
                let marquee0 = self.shared.borrow().marquee;

                let mut click_item: Option<(Sel, bool)> = None; // (item, shift)
                let mut begin: Option<(Sel, bool, Pos2)> = None; // (item, resize, start)
                let mut pointer: Option<Pos2> = None;
                let mut stop = false;
                let mut tab_switch: Option<(String, String)> = None;
                let only = |s: Sel| selected.len() == 1 && selected[0] == s; // sole selection

                // frame outlines + click-to-select + drag-to-move + resize handle (above bg,
                // below widgets so child widgets still win their own areas). Moving a frame
                // moves its children, which render relative to it. A selected frame is
                // highlighted so a layers-panel selection shows on the canvas.
                for (fi, fr, _parent) in frame_rects.iter().copied() {
                    let sel = selected.contains(&Sel::Frame(fi));
                    let frame_style = frames
                        .get(fi)
                        .map(|f| WidgetStyle::from_node(f, None, None))
                        .unwrap_or_else(|| WidgetStyle::from_node(&Value::Null, None, None));
                    frame_style.paint_rect(
                        ui,
                        fr,
                        Color32::TRANSPARENT,
                        border.gamma_multiply(0.8),
                    );
                    let stroke = if sel {
                        Stroke::new(2.0, accent)
                    } else {
                        Stroke::new(1.0, border.gamma_multiply(0.8))
                    };
                    ui.painter().rect_stroke(fr, Rounding::same(6.0), stroke);
                    if frames
                        .get(fi)
                        .and_then(|f| f.get("locked"))
                        .and_then(|l| l.as_bool())
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    let fid = egui::Id::new(("edf", fi));
                    let fresp = ui.interact(fr, fid, Sense::click_and_drag());
                    if fresp.clicked() {
                        click_item = Some((Sel::Frame(fi), shift));
                    }
                    if fresp.drag_started() {
                        begin = Some((
                            Sel::Frame(fi),
                            false,
                            fresp.interact_pointer_pos().unwrap_or(fr.min),
                        ));
                    }
                    if fresp.dragged() {
                        pointer = fresp.interact_pointer_pos();
                    }
                    if fresp.drag_stopped() {
                        stop = true;
                    }
                    if only(Sel::Frame(fi)) {
                        let h = Rect::from_min_size(fr.max - Vec2::splat(11.0), Vec2::splat(11.0));
                        let hr = ui.interact(h, fid.with("rs"), Sense::drag());
                        ui.painter().rect_filled(h, Rounding::same(2.0), accent);
                        if hr.drag_started() {
                            begin = Some((
                                Sel::Frame(fi),
                                true,
                                hr.interact_pointer_pos().unwrap_or(fr.max),
                            ));
                        }
                        if hr.dragged() {
                            pointer = hr.interact_pointer_pos();
                        }
                        if hr.drag_stopped() {
                            stop = true;
                        }
                    }
                }

                // tabs widgets: chrome + clickable bar (children render as placements below)
                for tb in &tabbars {
                    tb.style.paint_rect(
                        ui,
                        tb.rect,
                        ui.visuals().widgets.noninteractive.bg_fill,
                        border,
                    );
                    let mut bar =
                        ui.new_child(egui::UiBuilder::new().max_rect(tb.bar_rect.shrink(4.0)));
                    bar.set_clip_rect(tb.bar_rect);
                    bar.horizontal_wrapped(|ui| {
                        for (id, label) in &tb.tabs {
                            if ui.selectable_label(&tb.active == id, label).clicked() {
                                tab_switch = Some((tb.key.clone(), id.clone()));
                            }
                        }
                    });
                    ui.painter().line_segment(
                        [tb.bar_rect.left_bottom(), tb.bar_rect.right_bottom()],
                        Stroke::new(1.0, border),
                    );
                }

                // widget placements: WYSIWYG render + glass-pane select/drag/resize
                for (i, rect, _parent) in placements.iter().copied() {
                    let w = &widgets[i];
                    let is_sel = selected.contains(&Sel::Widget(i));
                    let id = egui::Id::new(("ed", i));
                    let (ty, name, label, val, dbg, dfg) = display_of(w, &wstate);
                    let noop = |_v: Value| {};
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    child.set_clip_rect(rect);
                    draw_interact_widget(
                        &mut child, rect, &ty, &name, &label, &val, dbg, dfg, &noop, w, self,
                    );

                    // locked: not selectable/draggable on the canvas (still in the layer tree).
                    if w.get("locked").and_then(|l| l.as_bool()).unwrap_or(false) {
                        let stroke = if is_sel {
                            Stroke::new(2.0, accent)
                        } else {
                            Stroke::new(1.0, border.gamma_multiply(0.3))
                        };
                        ui.painter().rect_stroke(rect, Rounding::same(5.0), stroke);
                        continue;
                    }

                    let resp = ui.interact(rect, id, Sense::click_and_drag());
                    resp.context_menu(|ui| {
                        if ui.button("Group into Frame").clicked() {
                            do_group = true;
                            ui.close_menu();
                        }
                    });
                    // a selected widget interacts directly (precise edit after a layers
                    // select); otherwise a click/drag grabs the top-most unlocked ancestor.
                    let target = if is_sel {
                        Sel::Widget(i)
                    } else {
                        click_target(i, &widgets, &frames, &frame_by_id)
                    };
                    if resp.clicked() {
                        click_item = Some((target, shift));
                    }
                    if resp.drag_started() {
                        begin = Some((
                            target,
                            false,
                            resp.interact_pointer_pos().unwrap_or(rect.min),
                        ));
                    }
                    if resp.dragged() {
                        pointer = resp.interact_pointer_pos();
                    }
                    if resp.drag_stopped() {
                        stop = true;
                    }
                    let stroke = if is_sel {
                        Stroke::new(2.0, accent)
                    } else if resp.hovered() {
                        Stroke::new(1.0, accent.gamma_multiply(0.5))
                    } else {
                        Stroke::new(1.0, border.gamma_multiply(0.5))
                    };
                    ui.painter().rect_stroke(rect, Rounding::same(5.0), stroke);
                    if only(Sel::Widget(i)) {
                        let h =
                            Rect::from_min_size(rect.max - Vec2::splat(11.0), Vec2::splat(11.0));
                        let hr = ui.interact(h, id.with("rs"), Sense::drag());
                        ui.painter().rect_filled(h, Rounding::same(2.0), accent);
                        if hr.drag_started() {
                            begin = Some((
                                Sel::Widget(i),
                                true,
                                hr.interact_pointer_pos().unwrap_or(rect.max),
                            ));
                        }
                        if hr.dragged() {
                            pointer = hr.interact_pointer_pos();
                        }
                        if hr.drag_stopped() {
                            stop = true;
                        }
                    }
                }
                if let Some((k, v)) = tab_switch {
                    self.tabs.insert(k, v);
                }
                let clicked_empty = bg_resp.clicked() && click_item.is_none() && begin.is_none();

                // marquee rubber-band (drag started on empty canvas)
                let mut marquee_rect: Option<Rect> = None;
                let mq_start = marquee0.or_else(|| {
                    if bg_resp.drag_started() {
                        bg_resp.interact_pointer_pos()
                    } else {
                        None
                    }
                });
                if let (Some(start), Some(cur)) = (mq_start, bg_resp.interact_pointer_pos()) {
                    if bg_resp.dragged() || bg_resp.drag_stopped() {
                        let r = Rect::from_two_pos(start, cur);
                        ui.painter().rect(
                            r,
                            Rounding::same(0.0),
                            accent.gamma_multiply(0.10),
                            Stroke::new(1.0, accent),
                        );
                        marquee_rect = Some(r);
                    }
                }

                let mut sh = self.shared.borrow_mut();
                if bg_resp.drag_started() {
                    sh.marquee = bg_resp.interact_pointer_pos();
                }
                if bg_resp.drag_stopped() {
                    if let Some(r) = marquee_rect {
                        // resolve each intersected widget to its top-most unlocked ancestor,
                        // matching click behavior: framed widgets pick up their frame.
                        let mut sel: Vec<Sel> = if shift { sh.selected.clone() } else { vec![] };
                        for (i, wr, _p) in &placements {
                            let locked = widgets
                                .get(*i)
                                .and_then(|w| w.get("locked"))
                                .and_then(|l| l.as_bool())
                                .unwrap_or(false);
                            if locked || !r.intersects(*wr) {
                                continue;
                            }
                            let t = click_target(*i, &widgets, &frames, &frame_by_id);
                            if !sel.contains(&t) {
                                sel.push(t);
                            }
                        }
                        sh.selected = sel;
                    }
                    sh.marquee = None;
                }
                if let Some((item, sh_held)) = click_item {
                    if sh_held {
                        if let Some(p) = sh.selected.iter().position(|x| *x == item) {
                            sh.selected.remove(p); // shift-click selected → deselect
                        } else {
                            sh.selected.push(item); // shift-click → add
                        }
                    } else {
                        sh.selected = vec![item]; // plain click → just this one
                    }
                } else if clicked_empty {
                    sh.selected.clear();
                }
                if let Some((item, resize, start)) = begin {
                    // dragging an unselected item selects it first (shift extends)
                    if !resize && !sh.selected.contains(&item) {
                        if shift {
                            sh.selected.push(item);
                        } else {
                            sh.selected = vec![item];
                        }
                    }
                    push_undo(&mut sh);
                    let items: Vec<Sel> = if resize {
                        vec![item]
                    } else {
                        sh.selected.clone()
                    };
                    sh.drag = Some(capture_drag(
                        resize,
                        start,
                        &sh.spec,
                        &items,
                        &parent_of,
                        &frame_parent_of,
                    ));
                }
                if let (Some(pos), Some(drag)) = (pointer, sh.drag.clone()) {
                    let total = pos - drag.start;
                    apply_drag(&mut sh.spec, &drag, total, shift);
                }
                if stop {
                    sh.drag = None;
                }
                // Group into Frame: wrap the selected widgets (uses the just-walked abs rects).
                // do_group = toolbar/canvas this pass; pending_group = layers context menu.
                let do_group = do_group || sh.pending_group;
                sh.pending_group = false;
                if do_group {
                    let wsel = sel_widgets(&sh.selected);
                    if !wsel.is_empty() {
                        let abs: HashMap<usize, Rect> =
                            placements.iter().map(|(i, r, _)| (*i, *r)).collect();
                        push_undo(&mut sh);
                        if let Some(fi) = group_into_frame(&mut sh.spec, &wsel, &abs, canvas) {
                            sh.selected = vec![Sel::Frame(fi)];
                        }
                    }
                }
            });
    }
}
