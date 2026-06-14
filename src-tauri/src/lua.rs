// lua.rs — sandboxed Lua scripting for .yantra control surfaces.
// ============================================================================
// The yantra reactive bus lives in the frontend (console buffer + control
// state). Lua runs here. So the model is frontend-driven: each tick the canvas
// ships a `vars` snapshot, we run a PER-SURFACE persistent Lua VM, and return
// { sets, sends, logs } for the frontend to apply. VM globals persist between
// ticks (that's the whole point — running averages, debounce, etc.).
//
// Script API (see PREAMBLE):
//   vars            the bus snapshot (name → value; plus t = ms, dt = ms since last tick)
//   set(name, v)    set a widget's value (v table ⇒ merge attrs: value/color/fg/label/image/hidden/disabled)
//   attr(name,k,v)  set one presentation attribute
//   send(action)    dispatch a YantraAction (string | {out=…}|{invoke=…}|{i2c=…}|{cfg=…})
//   log(msg)        debug line
//   state           persists across ticks (`state = state or {}`)
// A surface script conventionally defines `function update(vars) … end`; a widget
// script is a transform `(v, vars) -> value` whose result is published under its name.
use std::collections::HashMap;
use std::sync::Mutex;

use mlua::{Function, Lua, LuaSerdeExt, Value as LuaValue};
use serde_json::Value;

/// One widget's script, as sent from the frontend.
#[derive(serde::Deserialize)]
pub struct WidgetScript {
    pub name: String,
    pub script: String,
}

/// What a tick produces for the frontend to apply.
#[derive(serde::Serialize, Default)]
pub struct EvalOut {
    pub sets: Value,   // { name → { value?, color?, fg?, label?, image?, hidden?, disabled? } }
    pub frames: Value, // { id → { hidden? } } — container overrides (selector → show/hide panes)
    pub sends: Value,  // [ action, … ]
    pub logs: Vec<String>,
}

const PREAMBLE: &str = r#"
__sets = {}; __frames = {}; __sends = {}; __logs = {}
function set(n, v)
  local t = __sets[n]; if t == nil then t = {}; __sets[n] = t end
  if type(v) == 'table' then for k, val in pairs(v) do t[k] = val end
  else t.value = v end
end
function attr(n, k, v)
  local t = __sets[n]; if t == nil then t = {}; __sets[n] = t end
  t[k] = v
end
-- frame(id, {hidden=…}) merges container overrides; frame(id, bool) sets hidden.
function frame(id, v)
  local t = __frames[id]; if t == nil then t = {}; __frames[id] = t end
  if type(v) == 'table' then for k, val in pairs(v) do t[k] = val end
  else t.hidden = v end
end
-- show one frame of a group, hide the rest (the common tab pattern):
--   tabs('panelA', { 'panelA', 'panelB', 'panelC' })
function tabs(active, ids)
  for _, id in ipairs(ids) do frame(id, { hidden = id ~= active }) end
end
function send(a) __sends[#__sends + 1] = a end
function log(m) __logs[#__logs + 1] = tostring(m) end
print = log
"#;

/// A live per-surface VM: the Lua state + the compiled update/widget functions
/// and a hash of the source they came from (recompiled when the source changes).
struct Surface {
    lua: Lua,
    update: Option<Function>,
    widgets: Vec<(String, Function)>,
    hash: u64,
}

fn hash_scripts(surface: &str, widgets: &[WidgetScript]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    surface.hash(&mut h);
    for w in widgets {
        w.name.hash(&mut h);
        w.script.hash(&mut h);
    }
    h.finish()
}

impl Surface {
    fn new(surface_script: &str, widgets: &[WidgetScript], hash: u64) -> mlua::Result<Surface> {
        let lua = Lua::new();
        // sandbox: drop the libs a shared .yantra script should never reach
        {
            let g = lua.globals();
            for k in [
                "io", "os", "package", "require", "dofile", "loadfile", "load", "loadstring",
                "collectgarbage", "debug",
            ] {
                g.set(k, LuaValue::Nil)?;
            }
        }
        lua.load(PREAMBLE).exec()?;
        if !surface_script.trim().is_empty() {
            lua.load(surface_script).exec()?; // defines update / helpers / initial state
        }
        let update: Option<Function> = lua.globals().get("update")?;
        let mut compiled = Vec::new();
        for w in widgets {
            if w.script.trim().is_empty() {
                continue;
            }
            // wrap as a transform; `n` = tonumber(v) for convenience
            let src = format!("return function(v, vars)\nlocal n = tonumber(v)\n{}\nend", w.script);
            let f: Function = lua.load(src).eval()?;
            compiled.push((w.name.clone(), f));
        }
        Ok(Surface { lua, update, widgets: compiled, hash })
    }

    fn run(&self, vars: Value) -> mlua::Result<EvalOut> {
        let lua = &self.lua;
        let g = lua.globals();
        // fresh accumulators each tick; user state (globals) persists
        g.set("__sets", lua.create_table()?)?;
        g.set("__frames", lua.create_table()?)?;
        g.set("__sends", lua.create_table()?)?;
        g.set("__logs", lua.create_table()?)?;
        let vars_lua = lua.to_value(&vars)?;
        g.set("vars", vars_lua.clone())?;
        let vars_t = match &vars_lua {
            LuaValue::Table(t) => t.clone(),
            _ => lua.create_table()?,
        };

        let set_fn: Function = g.get("set")?;
        let mut errs: Vec<String> = Vec::new();

        // widget transforms first (compute values), then the surface orchestrator
        for (name, f) in &self.widgets {
            let v = vars_t.get::<LuaValue>(name.as_str()).unwrap_or(LuaValue::Nil);
            match f.call::<LuaValue>((v, vars_lua.clone())) {
                Ok(res) => {
                    if let Err(e) = set_fn.call::<()>((name.as_str(), res)) {
                        errs.push(format!("{name}: {e}"));
                    }
                }
                Err(e) => errs.push(format!("{name}: {e}")),
            }
        }
        if let Some(update) = &self.update {
            if let Err(e) = update.call::<()>(vars_lua.clone()) {
                errs.push(format!("update: {e}"));
            }
        }

        let sets: Value = lua.from_value(g.get("__sets")?)?;
        let frames: Value = lua.from_value(g.get("__frames")?)?;
        // an empty Lua table serializes as {} (object); force the actions list to an array
        let sends_v: Value = lua.from_value(g.get("__sends")?)?;
        let sends = if sends_v.is_array() { sends_v } else { Value::Array(Vec::new()) };
        let mut logs: Vec<String> = lua.from_value(g.get("__logs")?).unwrap_or_default();
        logs.extend(errs);
        Ok(EvalOut { sets, frames, sends, logs })
    }
}

/// Registry of per-surface VMs, held in app state. `Lua` is `Send` (the `send`
/// feature) so this lives behind a Mutex in Tauri-managed state.
#[derive(Default)]
pub struct LuaEngine {
    vms: Mutex<HashMap<String, Surface>>,
}

impl LuaEngine {
    /// Run one tick for surface `key`. Recompiles the VM when the script source
    /// changes (which resets its state — expected on edit).
    pub fn eval(
        &self,
        key: &str,
        surface_script: &str,
        widgets: &[WidgetScript],
        vars: Value,
    ) -> Result<EvalOut, String> {
        let mut vms = self.vms.lock().unwrap();
        let hash = hash_scripts(surface_script, widgets);
        let stale = vms.get(key).map(|s| s.hash != hash).unwrap_or(true);
        if stale {
            let s = Surface::new(surface_script, widgets, hash).map_err(|e| e.to_string())?;
            vms.insert(key.to_string(), s);
        }
        vms.get(key).unwrap().run(vars).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn surface_set_attr_send_and_state() {
        let eng = LuaEngine::default();
        let script = r#"
            state = state or { n = 0 }
            function update(vars)
              state.n = state.n + 1
              set('out', { value = state.n, color = vars.dist == '50' and '#0f0' or '#f00' })
              attr('lbl', 'label', 'hi')
              if state.n == 2 then send({ out = { index = 0, kind = 'rgb' } }) end
              log('tick ' .. state.n)
            end
        "#;
        let widgets: Vec<WidgetScript> = vec![];

        let a = eng.eval("k", script, &widgets, json!({ "dist": "50" })).unwrap();
        assert_eq!(a.sets["out"]["value"], json!(1)); // state persisted from 0→1
        assert_eq!(a.sets["out"]["color"], json!("#0f0"));
        assert_eq!(a.sets["lbl"]["label"], json!("hi"));
        assert!(a.sends.as_array().unwrap().is_empty());

        let b = eng.eval("k", script, &widgets, json!({ "dist": "999" })).unwrap();
        assert_eq!(b.sets["out"]["value"], json!(2)); // VM reused → state == 2
        assert_eq!(b.sets["out"]["color"], json!("#f00"));
        assert_eq!(b.sends.as_array().unwrap().len(), 1); // send fired on tick 2
        assert_eq!(b.sends[0]["out"]["index"], json!(0));
    }

    #[test]
    fn frame_overrides_and_tabs_helper() {
        let eng = LuaEngine::default();
        let script = r#"
            function update(vars)
              tabs(vars.sel, { 'panelA', 'panelB' })  -- show the selected, hide the rest
              frame('extra', { hidden = true })        -- explicit single override
            end
        "#;
        let out = eng.eval("f", script, &[], json!({ "sel": "panelB" })).unwrap();
        assert_eq!(out.frames["panelA"]["hidden"], json!(true));
        assert_eq!(out.frames["panelB"]["hidden"], json!(false));
        assert_eq!(out.frames["extra"]["hidden"], json!(true));
    }

    #[test]
    fn widget_transform_and_error_isolation() {
        let eng = LuaEngine::default();
        let widgets = vec![
            WidgetScript { name: "ok".into(), script: "return (n or 0) * 2".into() },
            WidgetScript { name: "bad".into(), script: "return nope()".into() }, // runtime error
        ];
        let out = eng.eval("w", "", &widgets, json!({ "ok": "21" })).unwrap();
        assert_eq!(out.sets["ok"]["value"], json!(42)); // transform ran (Lua 5.4 integer)
        assert!(out.logs.iter().any(|l| l.contains("bad"))); // error logged, didn't abort
    }

    #[test]
    fn sandbox_blocks_io() {
        let eng = LuaEngine::default();
        // os is nil'd → calling it errors, surfaced in logs, no panic
        let out = eng.eval("s", "function update(v) os.exit(0) end", &[], json!({})).unwrap();
        assert!(out.logs.iter().any(|l| l.contains("update")));
    }
}
