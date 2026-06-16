use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use eframe::egui;

use crate::state::Shared;
use crate::theme::apply_visuals;

pub(crate) struct YantraApp {
    pub(crate) shared: Rc<RefCell<Shared>>,
    pub(crate) sliders: HashMap<String, f32>,
    pub(crate) toggles: HashMap<String, bool>,
    pub(crate) colors: HashMap<String, [u8; 3]>,
    pub(crate) tabs: HashMap<String, String>, // tabs-widget key → active tab id (legacy widget)
    pub(crate) selects: HashMap<String, usize>, // select-widget key → active option index
}

impl YantraApp {
    pub(crate) fn new(shared: Rc<RefCell<Shared>>) -> Self {
        Self {
            shared,
            sliders: HashMap::new(),
            toggles: HashMap::new(),
            colors: HashMap::new(),
            tabs: HashMap::new(),
            selects: HashMap::new(),
        }
    }
}

impl eframe::App for YantraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let (theme, theme_dirty, editing) = {
            let mut sh = self.shared.borrow_mut();
            let td = sh.theme_dirty;
            sh.theme_dirty = false;
            (sh.theme.clone(), td, sh.editing)
        };
        if theme_dirty {
            apply_visuals(ctx, &theme);
        }
        ctx.request_repaint_after(Duration::from_millis(50));
        if editing {
            self.edit_ui(ctx);
        } else {
            self.interact_ui(ctx);
        }
    }
}
