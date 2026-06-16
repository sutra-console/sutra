use egui::Pos2;
use serde_json::Value;

#[derive(Default)]
pub(crate) struct Shared {
    pub(crate) spec: Value,
    pub(crate) state: Value,
    pub(crate) theme: Value,
    pub(crate) on_event: Option<js_sys::Function>,
    pub(crate) on_save: Option<js_sys::Function>,
    pub(crate) theme_dirty: bool,
    pub(crate) editing: bool,
    pub(crate) selected: Vec<Sel>, // unified: widgets and frames, multi-select
    pub(crate) undo: Vec<Value>,
    pub(crate) redo: Vec<Value>,
    pub(crate) drag: Option<Drag>,
    pub(crate) marquee: Option<Pos2>, // rubber-band select origin (canvas px)
    pub(crate) layer_anchor_row: Option<usize>,
    pub(crate) pending_group: bool, // layers context-menu "Group into Frame" -> applied in the canvas pass
}

/// An in-progress move/resize. Captured once at drag-start so we map the *absolute*
/// pointer delta onto the original geometry - pointer-following, never accumulating.
#[derive(Clone)]
pub(crate) struct Drag {
    pub(crate) resize: bool, // false = move whole selection, true = resize the single item
    pub(crate) start: Pos2,  // pointer pos at drag start
    pub(crate) items: Vec<DragItem>,
}

#[derive(Clone)]
pub(crate) struct DragItem {
    pub(crate) idx: usize,
    pub(crate) frames: bool, // this item targets spec.frames (else spec.widgets)
    pub(crate) ah: String,
    pub(crate) av: String,
    pub(crate) sx: f32, // original resolved absolute px geometry
    pub(crate) sy: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
    pub(crate) px: f32, // parent container origin + size (for nested store_axis)
    pub(crate) py: f32,
    pub(crate) pw: f32,
    pub(crate) ph: f32,
}

/// A selected item: a widget or a frame. Selection mixes both freely.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sel {
    Widget(usize),
    Frame(usize),
}
