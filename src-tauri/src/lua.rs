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
//   set(name, v)    set a widget's value (v table ⇒ merge attrs: value/fill/fg/label/image/hidden/disabled)
//   attr(name,k,v)  set one presentation attribute
//   send(action)    dispatch a YantraAction (string | {out=…}|{invoke=…}|{i2c=…}|{cfg=…})
//   log(msg)        debug line
//   state           persists across ticks (`state = state or {}`)
// A surface script conventionally defines `function update(vars) … end`; a widget
// script is a transform `(v, vars) -> value` whose result is published under its name.
//
// Helper stdlib (in every script — see PREAMBLE): math `clamp/map/lerp/round/
// approach/ema/choose` and color `rgb/gray/mix/heat` (all return "#rrggbb"). These
// cover the moves control surfaces actually make — scale a sensor to a bar, smooth
// a noisy reading, pick a zone label, paint a green→red gradient.
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
    pub sets: Value, // { name → { value?, fill?, fg?, label?, image?, hidden?, disabled? } }
    pub frames: Value, // { id → { hidden? } } — container overrides (selector → show/hide panes)
    pub sends: Value, // [ action, … ]
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

-- ── helper stdlib (available to every surface + widget script) ───────────────
-- math
function clamp(x, lo, hi)
  x = tonumber(x); if x == nil then return lo end
  if x < lo then return lo elseif x > hi then return hi else return x end
end
function lerp(a, b, t) return a + (b - a) * t end
-- map x from [in0,in1] onto [out0,out1], clamped to the output range.
function map(x, in0, in1, out0, out1)
  x = tonumber(x); if x == nil or in1 == in0 then return out0 end
  local v = out0 + (out1 - out0) * ((x - in0) / (in1 - in0))
  local lo, hi = out0, out1
  if lo > hi then lo, hi = hi, lo end
  if v < lo then return lo elseif v > hi then return hi else return v end
end
-- round x to `dp` decimal places (default 0).
function round(x, dp)
  x = tonumber(x); if x == nil then return 0 end
  local m = 10 ^ (dp or 0)
  return math.floor(x * m + 0.5) / m
end
-- move `cur` toward `target` by at most `step` (rate-limit / one-tick smoothing).
function approach(cur, target, step)
  cur = tonumber(cur) or 0; target = tonumber(target) or 0; step = math.abs(tonumber(step) or 0)
  if cur < target then return math.min(cur + step, target)
  elseif cur > target then return math.max(cur - step, target)
  else return target end
end
-- exponential moving average; first call (prev nil) seeds with x. Pair with `state`.
function ema(prev, x, alpha)
  x = tonumber(x); if x == nil then return prev end
  prev = tonumber(prev); if prev == nil then return x end
  return prev + (tonumber(alpha) or 0.2) * (x - prev)
end
-- pick the i-th item (1-based, clamped to the list) — e.g. a zone label.
function choose(i, list)
  i = math.floor(tonumber(i) or 1)
  if i < 1 then i = 1 elseif i > #list then i = #list end
  return list[i]
end

-- color (all return a '#rrggbb' string)
local function _b(v) v = math.floor((tonumber(v) or 0) + 0.5); if v < 0 then v = 0 elseif v > 255 then v = 255 end return v end
function rgb(r, g, b) return string.format('#%02x%02x%02x', _b(r), _b(g), _b(b)) end
function gray(v) local n = _b(v); return string.format('#%02x%02x%02x', n, n, n) end
local function _parse(c)
  if type(c) ~= 'string' then return 0, 0, 0 end
  c = string.gsub(c, '#', '')
  return tonumber(string.sub(c, 1, 2), 16) or 0,
         tonumber(string.sub(c, 3, 4), 16) or 0,
         tonumber(string.sub(c, 5, 6), 16) or 0
end
-- blend two hex colors; t=0 → c1, t=1 → c2.
function mix(c1, c2, t)
  t = clamp(t, 0, 1)
  local r1, g1, b1 = _parse(c1); local r2, g2, b2 = _parse(c2)
  return rgb(lerp(r1, r2, t), lerp(g1, g2, t), lerp(b1, b2, t))
end
-- heat gradient green→yellow→red for t in [0,1] (0 = calm green, 1 = hot red).
function heat(t)
  t = clamp(t, 0, 1)
  if t < 0.5 then return mix('#22c55e', '#eab308', t * 2)
  else return mix('#eab308', '#ef4444', (t - 0.5) * 2) end
end
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
                "io",
                "os",
                "package",
                "require",
                "dofile",
                "loadfile",
                "load",
                "loadstring",
                "collectgarbage",
                "debug",
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
            let src = format!(
                "return function(v, vars)\nlocal n = tonumber(v)\n{}\nend",
                w.script
            );
            let f: Function = lua.load(src).eval()?;
            compiled.push((w.name.clone(), f));
        }
        Ok(Surface {
            lua,
            update,
            widgets: compiled,
            hash,
        })
    }

    // Fresh per-call accumulators; user state (globals) persists across calls.
    fn reset_accumulators(&self) -> mlua::Result<()> {
        let lua = &self.lua;
        let g = lua.globals();
        g.set("__sets", lua.create_table()?)?;
        g.set("__frames", lua.create_table()?)?;
        g.set("__sends", lua.create_table()?)?;
        g.set("__logs", lua.create_table()?)?;
        Ok(())
    }

    // Drain the accumulators into an EvalOut, appending any caller-side errors.
    fn collect(&self, mut errs: Vec<String>) -> mlua::Result<EvalOut> {
        let lua = &self.lua;
        let g = lua.globals();
        let sets: Value = lua.from_value(g.get("__sets")?)?;
        let frames: Value = lua.from_value(g.get("__frames")?)?;
        // an empty Lua table serializes as {} (object); force the actions list to an array
        let sends_v: Value = lua.from_value(g.get("__sends")?)?;
        let sends = if sends_v.is_array() {
            sends_v
        } else {
            Value::Array(Vec::new())
        };
        let mut logs: Vec<String> = lua.from_value(g.get("__logs")?).unwrap_or_default();
        logs.append(&mut errs);
        Ok(EvalOut {
            sets,
            frames,
            sends,
            logs,
        })
    }

    fn run(&self, vars: Value) -> mlua::Result<EvalOut> {
        let lua = &self.lua;
        let g = lua.globals();
        self.reset_accumulators()?;
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
            let v = vars_t
                .get::<LuaValue>(name.as_str())
                .unwrap_or(LuaValue::Nil);
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
        self.collect(errs)
    }

    // Call a named global handler — the event-wiring path (a widget's on:{press:fn}
    // etc.). `vars` is published so the handler can read the current bus; `args` is
    // the event payload table (widget name, value, x/y, …). Same set/frame/send/log
    // collectors as a tick, so a handler shows/hides frames and drives outputs the
    // same way update() does. A missing handler is logged, not fatal.
    fn call(&self, fn_name: &str, vars: Value, args: Value) -> mlua::Result<EvalOut> {
        let lua = &self.lua;
        let g = lua.globals();
        self.reset_accumulators()?;
        g.set("vars", lua.to_value(&vars)?)?;
        let mut errs: Vec<String> = Vec::new();
        match g.get::<Option<Function>>(fn_name)? {
            Some(f) => {
                if let Err(e) = f.call::<()>(lua.to_value(&args)?) {
                    errs.push(format!("{fn_name}: {e}"));
                }
            }
            None => errs.push(format!("{fn_name}: not a function")),
        }
        self.collect(errs)
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

    /// Call a named handler in surface `key` (event wiring). Uses the same
    /// persistent VM as eval() — compiling it first if this is the first touch or
    /// the source changed — so state is shared with the tick. Returns the
    /// handler's { sets, frames, sends, logs } for the frontend to apply.
    pub fn call(
        &self,
        key: &str,
        surface_script: &str,
        widgets: &[WidgetScript],
        fn_name: &str,
        vars: Value,
        args: Value,
    ) -> Result<EvalOut, String> {
        let mut vms = self.vms.lock().unwrap();
        let hash = hash_scripts(surface_script, widgets);
        let stale = vms.get(key).map(|s| s.hash != hash).unwrap_or(true);
        if stale {
            let s = Surface::new(surface_script, widgets, hash).map_err(|e| e.to_string())?;
            vms.insert(key.to_string(), s);
        }
        vms.get(key)
            .unwrap()
            .call(fn_name, vars, args)
            .map_err(|e| e.to_string())
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
              set('out', { value = state.n, fill = vars.dist == '50' and '#0f0' or '#f00' })
              attr('lbl', 'label', 'hi')
              if state.n == 2 then send({ out = { index = 0, kind = 'rgb' } }) end
              log('tick ' .. state.n)
            end
        "#;
        let widgets: Vec<WidgetScript> = vec![];

        let a = eng
            .eval("k", script, &widgets, json!({ "dist": "50" }))
            .unwrap();
        assert_eq!(a.sets["out"]["value"], json!(1)); // state persisted from 0→1
        assert_eq!(a.sets["out"]["fill"], json!("#0f0"));
        assert_eq!(a.sets["lbl"]["label"], json!("hi"));
        assert!(a.sends.as_array().unwrap().is_empty());

        let b = eng
            .eval("k", script, &widgets, json!({ "dist": "999" }))
            .unwrap();
        assert_eq!(b.sets["out"]["value"], json!(2)); // VM reused → state == 2
        assert_eq!(b.sets["out"]["fill"], json!("#f00"));
        assert_eq!(b.sends.as_array().unwrap().len(), 1); // send fired on tick 2
        assert_eq!(b.sends[0]["out"]["index"], json!(0));
    }

    #[test]
    fn call_named_handler_event_wiring() {
        let eng = LuaEngine::default();
        let script = r#"
            state = state or { hits = 0 }
            function on_press(a)
              state.hits = state.hits + 1
              set(a.name, { label = 'hit ' .. tostring(state.hits) })
              frame(a.target, { hidden = a.value == 'hide' })
              log('press from ' .. a.name)
            end
        "#;
        let widgets: Vec<WidgetScript> = vec![];
        let a = eng
            .call("k", script, &widgets, "on_press", json!({ "t": 1 }),
                  json!({ "name": "btn1", "target": "panelA", "value": "hide" }))
            .unwrap();
        assert_eq!(a.sets["btn1"]["label"], json!("hit 1"));
        assert_eq!(a.frames["panelA"]["hidden"], json!(true));
        assert!(a.logs.iter().any(|l| l.contains("press from btn1")));

        // same VM → state persists across calls
        let b = eng
            .call("k", script, &widgets, "on_press", json!({}),
                  json!({ "name": "btn1", "target": "panelA", "value": "show" }))
            .unwrap();
        assert_eq!(b.sets["btn1"]["label"], json!("hit 2"));
        assert_eq!(b.frames["panelA"]["hidden"], json!(false));

        // unknown handler → logged, no panic
        let c = eng.call("k", script, &widgets, "nope", json!({}), json!({})).unwrap();
        assert!(c.logs.iter().any(|l| l.contains("nope")));
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
        let out = eng
            .eval("f", script, &[], json!({ "sel": "panelB" }))
            .unwrap();
        assert_eq!(out.frames["panelA"]["hidden"], json!(true));
        assert_eq!(out.frames["panelB"]["hidden"], json!(false));
        assert_eq!(out.frames["extra"]["hidden"], json!(true));
    }

    #[test]
    fn widget_transform_and_error_isolation() {
        let eng = LuaEngine::default();
        let widgets = vec![
            WidgetScript {
                name: "ok".into(),
                script: "return (n or 0) * 2".into(),
            },
            WidgetScript {
                name: "bad".into(),
                script: "return nope()".into(),
            }, // runtime error
        ];
        let out = eng.eval("w", "", &widgets, json!({ "ok": "21" })).unwrap();
        assert_eq!(out.sets["ok"]["value"], json!(42)); // transform ran (Lua 5.4 integer)
        assert!(out.logs.iter().any(|l| l.contains("bad"))); // error logged, didn't abort
    }

    #[test]
    fn sandbox_blocks_io() {
        let eng = LuaEngine::default();
        // os is nil'd → calling it errors, surfaced in logs, no panic
        let out = eng
            .eval("s", "function update(v) os.exit(0) end", &[], json!({}))
            .unwrap();
        assert!(out.logs.iter().any(|l| l.contains("update")));
    }

    #[test]
    fn stdlib_math_helpers() {
        let eng = LuaEngine::default();
        let script = r#"
            function update(vars)
              set('clamp_hi', clamp(150, 0, 100))     -- 100 (above range)
              set('clamp_lo', clamp(-5, 0, 100))       -- 0   (below range)
              set('clamp_str', clamp('80', 0, 100))    -- 80  (string coerced)
              set('map', map(50, 0, 100, 0, 10))       -- 5.0 (linear)
              set('map_clamp', map(200, 0, 100, 0, 100))  -- 100 (clamped to out range)
              set('map_inv', map(0, 30, 1200, 100, 0))    -- 100 (inverted range ok)
              set('lerp', lerp(0, 10, 0.5))            -- 5.0
              set('round2', round(3.14159, 2))         -- 3.14
              set('round0', round(2.4))                -- 2.0
              set('approach_up', approach(0, 10, 3))   -- 3
              set('approach_cap', approach(9, 10, 3))  -- 10 (no overshoot)
              set('approach_dn', approach(2, 0, 3))    -- 0  (no undershoot)
              set('ema_seed', ema(nil, 5, 0.5))        -- 5  (first call seeds)
              set('ema_step', ema(0, 10, 0.5))         -- 5.0
              set('choose', choose(2, {'a','b','c'}))  -- 'b'
              set('choose_hi', choose(9, {'a','b'}))   -- 'b' (clamped)
              set('choose_lo', choose(0, {'a','b'}))   -- 'a' (clamped)
            end
        "#;
        let o = eng.eval("m", script, &[], json!({})).unwrap();
        assert_eq!(o.sets["clamp_hi"]["value"], json!(100));
        assert_eq!(o.sets["clamp_lo"]["value"], json!(0));
        assert_eq!(o.sets["clamp_str"]["value"], json!(80));
        assert_eq!(o.sets["map"]["value"], json!(5.0));
        assert_eq!(o.sets["map_clamp"]["value"], json!(100));
        assert_eq!(o.sets["map_inv"]["value"], json!(100));
        assert_eq!(o.sets["lerp"]["value"], json!(5.0));
        assert_eq!(o.sets["round2"]["value"], json!(3.14));
        assert_eq!(o.sets["round0"]["value"], json!(2.0));
        assert_eq!(o.sets["approach_up"]["value"], json!(3));
        assert_eq!(o.sets["approach_cap"]["value"], json!(10));
        assert_eq!(o.sets["approach_dn"]["value"], json!(0));
        assert_eq!(o.sets["ema_seed"]["value"], json!(5));
        assert_eq!(o.sets["ema_step"]["value"], json!(5.0));
        assert_eq!(o.sets["choose"]["value"], json!("b"));
        assert_eq!(o.sets["choose_hi"]["value"], json!("b"));
        assert_eq!(o.sets["choose_lo"]["value"], json!("a"));
        assert!(o.logs.is_empty(), "no errors expected: {:?}", o.logs);
    }

    #[test]
    fn stdlib_color_helpers() {
        let eng = LuaEngine::default();
        let script = r#"
            function update(vars)
              set('rgb', rgb(255, 0, 128))             -- #ff0080
              set('rgb_clamp', rgb(300, -5, 10))       -- #ff000a (channels clamped)
              set('gray', gray(16))                    -- #101010
              set('mix_mid', mix('#000000', '#ffffff', 0.5))  -- #808080
              set('mix_c1', mix('#ff0000', '#00ff00', 0))     -- #ff0000
              set('mix_c2', mix('#ff0000', '#00ff00', 1))     -- #00ff00
              set('heat_lo', heat(0))                  -- #22c55e (green)
              set('heat_mid', heat(0.5))               -- #eab308 (amber)
              set('heat_hi', heat(1))                  -- #ef4444 (red)
            end
        "#;
        let o = eng.eval("c", script, &[], json!({})).unwrap();
        assert_eq!(o.sets["rgb"]["value"], json!("#ff0080"));
        assert_eq!(o.sets["rgb_clamp"]["value"], json!("#ff000a"));
        assert_eq!(o.sets["gray"]["value"], json!("#101010"));
        assert_eq!(o.sets["mix_mid"]["value"], json!("#808080"));
        assert_eq!(o.sets["mix_c1"]["value"], json!("#ff0000"));
        assert_eq!(o.sets["mix_c2"]["value"], json!("#00ff00"));
        assert_eq!(o.sets["heat_lo"]["value"], json!("#22c55e"));
        assert_eq!(o.sets["heat_mid"]["value"], json!("#eab308"));
        assert_eq!(o.sets["heat_hi"]["value"], json!("#ef4444"));
        assert!(o.logs.is_empty(), "no errors expected: {:?}", o.logs);
    }

    #[test]
    fn stdlib_helpers_in_widget_transform() {
        // helpers are visible inside the per-widget transform chunk too
        let eng = LuaEngine::default();
        let widgets = vec![WidgetScript {
            name: "bar".into(),
            script: "return { value = clamp(map(n, 30, 1200, 100, 0), 0, 100), fill = heat((n or 0)/1200) }".into(),
        }];
        let o = eng.eval("w", "", &widgets, json!({ "bar": "30" })).unwrap();
        assert_eq!(o.sets["bar"]["value"], json!(100.0)); // closest → full bar (linear path → float)
        assert_eq!(o.sets["bar"]["fill"], json!("#2cc45a")); // heat(30/1200) → near-green
    }
}
