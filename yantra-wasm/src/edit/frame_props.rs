use eframe::egui;
use egui::RichText;
use serde_json::{json, Value};

use crate::geometry::{anchor, num, r2};
use crate::state::Shared;

use super::state_ops::push_undo;
pub(super) fn edit_frame_props(ui: &mut egui::Ui, sh: &mut Shared, fi: usize, f: &Value) {
    let mut name = f
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut id = f
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let (mut x, mut y, mut ww, mut hh) = (
        num(f, "x", 0.0),
        num(f, "y", 0.0),
        num(f, "w", 40.0),
        num(f, "h", 40.0),
    );
    let mut ah = anchor(f, "anchorH", "scale");
    let mut av = anchor(f, "anchorV", "scale");
    let mut clip = f.get("clip").and_then(|c| c.as_bool()).unwrap_or(true);
    let mut changed = false;
    let mut snapshot = false;

    ui.add_space(2.0);
    ui.label(RichText::new("frame").weak());
    egui::Grid::new("fprops")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            ui.label("id");
            let r = ui.text_edit_singleline(&mut id);
            if r.gained_focus() {
                snapshot = true;
            }
            if r.changed() {
                changed = true;
            }
            ui.end_row();
            ui.label("name");
            let r = ui.text_edit_singleline(&mut name);
            if r.gained_focus() {
                snapshot = true;
            }
            if r.changed() {
                changed = true;
            }
            ui.end_row();
            for (lbl, v) in [("x", &mut x), ("y", &mut y), ("w", &mut ww), ("h", &mut hh)] {
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
            for (lbl, cur, gid) in [("anchor H", &mut ah, "fah"), ("anchor V", &mut av, "fav")] {
                ui.label(lbl);
                let before = cur.clone();
                egui::ComboBox::from_id_salt(gid)
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
            ui.label("clip");
            if ui.checkbox(&mut clip, "").changed() {
                snapshot = true;
                changed = true;
            }
            ui.end_row();
        });
    if changed {
        if snapshot {
            push_undo(sh);
        }
        let fm = &mut sh.spec["frames"][fi];
        fm["id"] = json!(id);
        fm["name"] = json!(name);
        fm["x"] = json!(r2(x));
        fm["y"] = json!(r2(y));
        fm["w"] = json!(r2(ww));
        fm["h"] = json!(r2(hh));
        fm["anchorH"] = json!(ah);
        fm["anchorV"] = json!(av);
        fm["clip"] = json!(clip);
    }
}
