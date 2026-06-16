use std::collections::HashMap;

use serde_json::Value;

use crate::state::Sel;
/// Layer-list drag payload (which list + source index).
#[derive(Clone, Copy)]
pub(super) enum LayerDrag {
    Widget(usize),
    Frame(usize),
}

/// A selected item — a widget or a frame (selection mixes both freely).

pub(super) fn sel_widgets(sel: &[Sel]) -> Vec<usize> {
    sel.iter()
        .filter_map(|s| {
            if let Sel::Widget(i) = s {
                Some(*i)
            } else {
                None
            }
        })
        .collect()
}

/// Resolve a canvas click on a widget to the frame it should grab. An *unlocked*
/// frame is a solid group; a *locked* frame is an edit boundary — transparent to
/// selection so its children can be picked individually. Walk up the run of
/// *consecutive unlocked* frames and return the outermost; a locked frame (or no
/// frame) stops the walk and the widget itself is selected.
pub(super) fn click_target(
    i: usize,
    widgets: &[Value],
    frames: &[Value],
    frame_by_id: &HashMap<String, usize>,
) -> Sel {
    let mut fid = widgets
        .get(i)
        .and_then(|w| w.get("frame"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let mut outermost_unlocked: Option<usize> = None;
    let mut guard = 0;
    while let Some(id) = fid {
        guard += 1;
        if guard > 64 {
            break; // cycle guard
        }
        let Some(&fi) = frame_by_id.get(&id) else {
            break;
        };
        let locked = frames
            .get(fi)
            .and_then(|f| f.get("locked"))
            .and_then(|l| l.as_bool())
            .unwrap_or(false);
        if locked {
            break; // boundary: select inside it (the widget / inner unlocked run)
        }
        outermost_unlocked = Some(fi);
        fid = frames
            .get(fi)
            .and_then(|f| f.get("parent"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    match outermost_unlocked {
        Some(fi) => Sel::Frame(fi),
        None => Sel::Widget(i),
    }
}

/// Wrap the selected widgets in a new frame sized to their bounding box, reparenting
/// each into it (coords made relative to the frame). `abs` = each widget's absolute

pub(super) fn move_in_array(spec: &mut Value, key: &str, from: usize, to: usize) -> usize {
    let Some(arr) = spec.get_mut(key).and_then(|v| v.as_array_mut()) else {
        return from;
    };
    if from >= arr.len() {
        return from;
    }
    let item = arr.remove(from);
    let insert_at = if to > from { to - 1 } else { to }.min(arr.len());
    arr.insert(insert_at, item);
    insert_at
}

/// Flatten the frame/widget tree into display rows (depth, item), honoring each
/// frame's collapsed state. `container` = None for root (frame.parent / widget.frame absent).
pub(super) fn build_layer_rows(
    container: Option<&str>,
    depth: usize,
    frames_meta: &[(usize, String, Option<String>, bool)], // (idx, id, parent, collapsed)
    widget_frame: &[(usize, Option<String>)],              // (idx, frame membership)
    rows: &mut Vec<(usize, LayerDrag)>,
) {
    for (fi, id, parent, collapsed) in frames_meta {
        if parent.as_deref() == container {
            rows.push((depth, LayerDrag::Frame(*fi)));
            if !collapsed {
                build_layer_rows(Some(id), depth + 1, frames_meta, widget_frame, rows);
            }
        }
    }
    for (i, frame) in widget_frame {
        if frame.as_deref() == container {
            rows.push((depth, LayerDrag::Widget(*i)));
        }
    }
}

/// Move several elements (preserving their order) to before `to`. Returns the new
/// start index of the moved block.
pub(super) fn move_many_in_array(
    spec: &mut Value,
    key: &str,
    mut idxs: Vec<usize>,
    to: usize,
) -> usize {
    let Some(arr) = spec.get_mut(key).and_then(|v| v.as_array_mut()) else {
        return to;
    };
    idxs.sort_unstable();
    idxs.dedup();
    idxs.retain(|&i| i < arr.len());
    if idxs.is_empty() {
        return to;
    }
    let mut removed: Vec<Value> = idxs.iter().rev().map(|&i| arr.remove(i)).collect();
    removed.reverse(); // back to ascending original order
    let before = idxs.iter().filter(|&&i| i < to).count();
    let insert_at = to.saturating_sub(before).min(arr.len());
    for (k, item) in removed.into_iter().enumerate() {
        arr.insert(insert_at + k, item);
    }
    insert_at
}
