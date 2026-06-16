// egui yantra runtime (Path B): renders a YantraSpec in the webview; the React host
// owns the data-flow (bus + native mlua) and is authoritative. Two modes:
//  - interact: draw host-pushed values/overrides; report input via on_event.
//  - edit: an egui visual editor (multi-select, drag, resize, snap, align, undo) that
//    saves the spec back via on_save.
mod app;
mod edit;
mod geometry;
mod interact;
mod state;
mod theme;
mod widgets;

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::Value;
use wasm_bindgen::prelude::*;

use crate::app::YantraApp;
use crate::state::Shared;

thread_local! {
    static SHARED: Rc<RefCell<Shared>> = Rc::new(RefCell::new(Shared::default()));
}

#[wasm_bindgen]
pub fn start(
    canvas: web_sys::HtmlCanvasElement,
    spec_json: String,
    on_event: js_sys::Function,
    on_save: js_sys::Function,
) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let shared = SHARED.with(|s| s.clone());
    {
        let mut sh = shared.borrow_mut();
        sh.spec = serde_json::from_str(&spec_json).unwrap_or(Value::Null);
        sh.on_event = Some(on_event);
        sh.on_save = Some(on_save);
        sh.theme_dirty = true;
    }
    let app_shared = shared.clone();
    wasm_bindgen_futures::spawn_local(async move {
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |_cc| Ok(Box::new(YantraApp::new(app_shared)))),
            )
            .await
            .expect("eframe start failed");
    });
    Ok(())
}

#[wasm_bindgen]
pub fn set_state(state_json: String) {
    let v: Value = serde_json::from_str(&state_json).unwrap_or(Value::Null);
    SHARED.with(|s| s.borrow_mut().state = v);
}

#[wasm_bindgen]
pub fn set_theme(theme_json: String) {
    let v: Value = serde_json::from_str(&theme_json).unwrap_or(Value::Null);
    SHARED.with(|s| {
        let mut sh = s.borrow_mut();
        sh.theme = v;
        sh.theme_dirty = true;
    });
}

#[wasm_bindgen]
pub fn set_edit(editing: bool) {
    SHARED.with(|s| {
        let mut sh = s.borrow_mut();
        sh.editing = editing;
        if !editing {
            sh.selected.clear();
        }
    });
}
