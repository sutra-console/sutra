# Yantra — control surfaces

A **`.yantra`** is a declarative control surface for a device: a YAML file describing widgets
(buttons, sliders, readouts, tables, …), how they're laid out, what they **send** to the device, what
they're **filled** from (the live console / other widgets), and — optionally — **Lua** that scripts
the whole thing. Sutra renders it as that device's config/dashboard panel. Ship a `.yantra` and Sutra
becomes that device's app.

> *yantrá* (Sanskrit यन्त्र): *machine, contraption; a diagram used as an instrument.*

Files live in the workspace at `<ws>/.sutra/yantra/*.yantra`. In the app's **Controls** tab: pick a
surface, **Edit** to lay it out, **Open** to import one, **New** to start blank. They're plain YAML
(`serde_yaml` ⇄ JSON passthrough) — hand-editable, shareable, no secrets.

The runtime contract is mirrored in [`src/lib/skrit.ts`](src/lib/skrit.ts) (`YantraSpec`,
`YantraWidget`, `YantraAction`, …) and rendered by `YantraCanvas.tsx`; the visual editor is
`YantraEditor.tsx`; Lua runs in [`src-tauri/src/lua.rs`](src-tauri/src/lua.rs).

---

## Anatomy

```yaml
name: My Device
description: …
coordV: 2          # coordinate model (2 = container-relative; written by the editor)
cols: 6            # editor snap grid only
script: |          # optional surface Lua (see Scripting)
  function update(vars) … end
frames: []         # optional nested containers (editor-managed)
widgets:
  - { type: button, name: hot, label: "Hot start", x: 4, y: 8, w: 40, h: 40,
      send: "$PMTK101*32\r\n" }
```

### Widgets

| type | renders | key fields |
|------|---------|-----------|
| `button` | a button | `send` |
| `toggle` | on/off button | `on`, `off`, `value` |
| `slider` | range input | `min`, `max`, `step`, `value`, `send` (`{value}` ← position) |
| `select` | option buttons | `options: [{label, send}]` |
| `readout` | a value display | `bind` / `match` |
| `label` | text (optionally styled with `renderers`) | `label` |
| `table` | a repeating row template | `source`, `match`, `all`, `columns` |
| `image` | an `<img>` | `image` (or bound `value`) |
| `tabs` | tabbed panes | `tabs: [{id, label}]`; members set `tab` |

Every widget has a stable **`name`** — the addressable id used by `var:<name>` sources and by Lua
`set`/`attr`. Common fields: `label`, `help` (tooltip), `hidden`, `locked` (editor), `frame`/`tab`
(membership), and the layout + presentation fields below.

### Layout (container-relative)

`x`/`y`/`w`/`h` are relative to the parent container's content box. Each axis has an **anchor** preset
(`anchorH`/`anchorV`) deciding how they read:

- `scale` → `x`,`w` are **%** of the parent (responsive). *(default H)*
- `start` → px from the left/top, `w`/`h` px. *(default V)*
- `center` → px offset from centre · `end` → px gap from right/bottom · `stretch` → near/far margins.

`frames` are nestable containers (clip + move together); `tabs` widgets hold panes. The editor manages
both via the **Layers** tree. The on-disk model is `coordV: 2`; older grid files migrate on load.

---

## Data flow

Controls aren't just write-only — they can be **filled** from live data, and values can be **piped**
back out to the device.

### Sources

A source is a string id (loose/extensible):

- `"uart"` — the current connection's **console** stream (default).
- `"var:<name>"` — another widget's value from the reactive **bus** (widget→widget piping).
- `"com:<id>"` — a specific connection's console (multi-device-ready; resolves to the current one today).
- `"nodes"` — *reserved* (active network node list); type slot only.

### The value bus (`vars`)

A reactive map keyed by widget `name`. Console-bound widgets publish their evaluated value; sliders and
toggles publish their **live control state**. Anything can read another widget as `vars.<name>` (in
`expr`/Lua). Plus `vars.t` (ms clock) and `vars.dt` (ms since last tick) in scripts.

### `bind` — fill a display control

```yaml
bind:
  source: uart                 # default
  match: 'temp=(\d+)'          # text source: regex; capture group 1 → v  (LAST/newest match wins)
  field: addr                  # bus/object value: dotted property path → v
  expr: "n * 0.1"              # JS transform: v (string), n (Number), vars → display value
```

Fills `readout`/`label` (text), `toggle` (truthiness), `slider` (position). No `bind` ⇒ behaves as a
plain control. Legacy: a top-level `match` is read as `{ source: uart, match }`.

### `emit` — consume-output (drive a device output on change)

```yaml
emit:
  source: var:dist
  expr: "vars.dmax and n < vars.dmax and 255 or 0"   # undefined ⇒ HOLD (don't fire)
  send: { out: { index: 0, kind: rgb } }
```

When the computed value changes, fire `send` with the value substituted (deduped; skipped while
disconnected). Returning `undefined`/`NaN` **holds** (no fire) — handy for "no reading → keep last".

### Actions (`send` / `on` / `off` / option `send` / `emit.send`)

A `YantraAction` is one of:

| form | does |
|------|------|
| `"text"` | raw DATA write (UART/console); `{value}` substitutes the slider/emit value |
| `{ i2c: { addr, write, read } }` | an I²C transfer |
| `{ invoke: { id, args } }` | a device **INVOKE** command (see [INVOKE.md](INVOKE.md)) |
| `{ cfg: { key, str \| bytes } }` | a CFG set |
| `{ out: { index, kind, value? } }` | drive output `index`: `rgb` (grey level) · `pwm` (duty) · `set` (on/off); `value` carries the level inline (Lua) |

### `table` — a repeating row template

```yaml
type: table
source: uart
match: 'node (\w+) lqi (\d+)'
all: true                       # matchAll → one row per match
columns:
  - { label: addr, field: "1" }      # capture group / object field
  - { label: lqi,  expr: "Number(item[2])" }   # item, i in scope
```

The array comes from a `var:<name>` (a bus array) or a text source with `all`. Generic, so a future
`nodes` source slots in unchanged.

### Presentation

`fg` (text) and `image` (src/data-URI) are static fields **and** script-overridable
(see `attr`). The `image` widget shows `image` or its bound `value`.

Stage, frame, and widget nodes share a stacked renderer component. Each pass can paint and then inset
the content box with `padding`; multiple passes compose like a scene-graph renderer stack:

```yaml
stage:
  renderers:
    - { fill: "#101820", stroke: "#345", strokeWidth: 1, radius: 10, padding: 12 }

frames:
  - id: panel
    renderers:
      - { fill: "rgba(255,255,255,0.08)", radius: 8, padding: { x: 10, y: 8 } }
      - { stroke: "#475569", strokeWidth: 1, radius: 8, padding: 2 }

widgets:
  - type: readout
    name: volts
    fg: "#f8fafc"
    renderers:
      - { fill: "#1f2937", stroke: "#475569", radius: 6 }
```

Renderer keys are exactly `fill`, `stroke`, `strokeWidth`, `radius`, and `padding`. `padding` may be a
number or `{ x, y }` / `{ horizontal, vertical }`; on frames and `stage`, padding changes the child
content box. Deprecated top-level chrome keys are rejected at load/save time instead of being aliased.

---

## Scripting (Lua)

For logic beyond one-line `expr` — persistent state (smoothing, debounce), orchestration across
widgets, dynamic presentation. Runs **sandboxed** in the Rust backend (`mlua`), one persistent VM per
surface, ticked ~every 100 ms while the surface is shown. (Frontend-driven: the bus lives in the UI.)

Two attach points:

- **Surface** `spec.script` — loaded once; define `function update(vars) … end` (called each tick) plus
  any helpers. Open the full-size editor with the **Script** button in the editor toolbar.
- **Widget** `widget.script` — a transform `(v, vars) → value`; its return is published under the
  widget's `name`. Wins over `bind.expr` when both are set.

### API (globals each tick)

| | |
|---|---|
| `vars` | the bus snapshot (`vars.<name>`, `vars.t`, `vars.dt`) |
| `set(name, v)` | set a widget's **value** (`v` table ⇒ merge attrs) |
| `attr(name, key, value)` | set a presentation attribute: `value`, `fill`, `fg`, `label`, `image`, `hidden`, `disabled` |
| `send(action)` | dispatch a `YantraAction` (Lua table): `"raw"` · `{out={index=0,kind='rgb',value=lvl}}` · `{invoke={id=…,args={…}}}` · `{i2c=…}` · `{cfg=…}` |
| `log(msg)` / `print(msg)` | write to the **console** strip in the Controls view |
| `state` | a normal global — persists across ticks (`state = state or {}`) |

Sandboxed: no `io`/`os`/`package`/`require`/`debug`. Errors are caught and shown in the console
(prefixed `!`); the surface keeps running.

### Helper library (always in scope)

Beyond the bare API, every surface and widget script gets a small stdlib so the moves a control
surface actually makes are one-liners — scale a sensor to a bar, smooth a reading, paint a gradient:

| | |
|---|---|
| `clamp(x, lo, hi)` · `lerp(a, b, t)` · `round(x, dp?)` | basic math |
| `map(x, in0, in1, out0, out1)` | rescale `x` between ranges, **clamped to the output** |
| `approach(cur, target, step)` | move `cur` toward `target` by ≤ `step` (rate-limit / one-tick smoothing) |
| `ema(prev, x, alpha)` | exponential moving average; first call (`prev` nil) seeds with `x` |
| `choose(i, list)` | the i-th item (1-based, clamped) — e.g. a zone label |
| `rgb(r, g, b)` · `gray(v)` | build `"#rrggbb"` (channels clamped 0–255) |
| `mix(c1, c2, t)` | blend two hex colors (`t` 0 → c1, 1 → c2) |
| `heat(t)` | green→amber→red gradient for `t` in 0…1 |

```lua
state.avg = ema(state.avg, tonumber(vars.dist), 0.2)              -- smoothed
set('bar', { value = map(state.avg, 30, 1200, 100, 0), fill = heat(state.avg/1200) })
```

### Test it without a device

The full-size **Script** dialog has a **Test** runner: drop a `vars` snapshot in as JSON (or **Live
bus** to fill it from the connected device), hit **Test**, and it runs one real backend tick
off-device — reporting the `set`/`send`/`frame` it produced plus any `log`/`print` lines and errors, so
a syntax error shows up immediately instead of at runtime. Each press advances `t`, so `state`-based
scripts (`ema`, `approach`) visibly converge. Uses a separate VM key, so it never disturbs a live surface.

### Notes

- Scripts run only in the **Controls** (render) view, not while editing.
- `send` is **not** auto-deduped — at ~10 Hz that's ~10 writes/s; dedupe in Lua via `state` for heavier
  outputs.
- Adding a new backend command (e.g. `yantra_eval`) means the app must be **rebuilt + relaunched** to
  pick it up.

### Example (excerpt)

```lua
state = state or {}
function update(vars)
  local raw = tonumber(vars.raw)                       -- hidden readout bound to the console
  if raw and raw < 8000 then                           -- "none"/garbage → nil → hold
    state.s = state.s and (state.s*0.7 + raw*0.3) or raw   -- smoothing
  end
  if not state.s then set('dist','no target'); return end
  local lvl = math.max(0, math.min(255, math.floor(255*(vars.dmax-state.s)/(vars.dmax-vars.dmin))))
  set('dist', math.floor(state.s) .. ' mm')
  send({ out = { index = 0, kind = 'rgb', value = lvl } })   -- closer = brighter
  attr('dot', 'fill', state.s < vars.dmin and '#22c55e' or '#ef4444')
  if state.s % 1 == 0 then print('d=' .. math.floor(state.s)) end
end
```

A full version (smoothing + status dot + LED + console `print`) ships as `VL53L0X-Lua.yantra`.

---

## Editor

**Edit** (in the Controls tab) swaps the renderer for the visual editor: an **Add** palette, drag /
8-way resize (hold **⇧** to snap to the grid, **Ctrl** to mirror-resize), align/spacing tools,
undo/redo, a **Layers** tree (frames, tabs, z-order, lock/hide), a per-widget **property panel**
(type, name, label, geometry, anchors, the action editor, the **Data** binding, presentation, and a
per-widget script), and the **Script** button for the full surface Lua. **Save** writes YAML back.
