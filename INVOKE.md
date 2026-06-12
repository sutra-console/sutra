# INVOKE — user-defined device commands

Duta is a framework, not a fixed-function device (the QMK "just sends keycodes"
posture is the *default*, not the ceiling). `INVOKE` is the open extension point:
a module on the device registers its own command, advertises it, and Sutra
forwards a high-level intent — `send_touch(x, y)`, `zigbee_join(…)`, anything —
to that handler **without having to understand the implementation**.

See `PROTOCOL.md` → *INVOKE* for the wire format. This doc tracks the Sutra
surface + the plan for the visual layer.

## Status

| Layer | Where | State |
|-------|-------|-------|
| Wire spec (`INVOKE_DESC`/`INVOKE`, arg codec, id ranges, skrit-mc op `0x06`) | skrit `protocol.h` + `PROTOCOL.md` | ✅ |
| Device core dispatch + `cmd_desc`/`cmd_invoke` HAL | duta `skrit_device.h` | ✅ (host-tested) |
| Demo commands on real HW | duta nRF52840 (`send_touch`, `blink`, `echo`) | ✅ |
| Client catalog + helpers (`invocables()`, `packArgs`, `invokeCommand`) | sutra `src/lib/skrit.ts` + `invocables.json` | ✅ |
| MCP tools (`list_invocables`, `invoke`) | sutra `src-tauri/src/mcp.rs` | ✅ |
| **Generic invoke panel** (UI) | sutra `src/components/` | ⏳ planned |
| **Macro-editor `Invoke` op** | sutra macro editor | ⏳ planned |

The **two lists** contract holds everywhere: the device's `INVOKE_DESC` is the
source of truth for *what exists*; `invocables.json` (the catalog, shared by the
UI and the MCP server) enriches *recognized* ids with labels + widget hints, and
any unknown vendor id (≥ `0x8000`) still works from its raw arg signature.

## Gameplan: the visual surface

### 1. Generic invoke panel (`InvokePanel.tsx`)

A device-controls panel that appears when `getInfo().flags & FLAG.INVOKE`.

- **Data**: call `invocables()` on connect → `Invocable[]` (already merges device
  `INVOKE_DESC` + catalog). Re-fetch on reconnect, like `pinCaps()`/`outputs`.
- **Render**: one card per command — `label` (catalog or device name), a
  `[vendor]` chip for ≥ `0x8000`, and a typed form built from `args[]`:
  | arg `widget` / type | control |
  |---------------------|---------|
  | `number` / u8·u16·u32·i16·i32 | numeric input (clamp to `min`/`max`) |
  | `slider` | range slider with the value readout |
  | `xy` (a paired x/y, e.g. send_touch) | an xy-pad that writes both args |
  | `hex` / `bytes` | hex text field → `number[]` |
  | `text` / `str` | text field |
  | no catalog match | generic `arg0…argN` inputs typed by the wire code |
- **Invoke**: `invokeCommand(cmd.id, packArgs(cmd.argCodes, values))`. If
  `cmd.hasReply`, show the returned bytes (hex, + ASCII gutter).
- **Reuse**: the typed-control mapping is the same idea as the per-IO-type
  controls (task #9) — factor the widget switch so both can share it.
- Follow the shadcn/Radix component rule (no native form controls — see the
  user-prefs memory): `Input`, `Slider`, `Badge`, `Card`, `Button`.

### 2. Macro-editor `Invoke` op

Make `INVOKE` a first-class macro step so a stored program can drive a module's
own commands (compiles to skrit-mc `0x06`).

- **Grammar**: `Invoke <name|0xID> <arg> <arg> …` (e.g. `Invoke send_touch 100 200`).
  Add to the `parse_macro` front-end (`Step::Invoke { id, args }`).
- **Compile**: resolve `name → id` via the catalog; pack args with the same codec
  as `packArgs` → emit `0x06, id_lo, id_hi, n, payload…`. Tier 1 (no gate).
- **Decode/inline-decoration**: render `Invoke send_touch(100, 200)` with the
  catalog label, mirroring the RGB color decoration work (task #10).
- **Editor affordance**: an "Invoke" insert offering the connected device's
  `invocables()` as a dropdown + a typed arg form (reuse the panel's widgets).
- **Player**: the app-side player calls `invokeCommand` for a live run; the
  device target runs the bytecode op directly (one front-end, both paths — the
  macro invariant).

### Sequencing

Panel first (it's standalone and makes INVOKE usable in the GUI), then the macro
op (it depends on the same widget + catalog plumbing). Both are independent of
the firmware, which is already done.

## Catalog (`src/lib/invocables.json`)

The canonical well-known registry (ids `0x0000`–`0x7FFF`), mirrors the table in
`PROTOCOL.md`. Loaded by the UI (`skrit.ts`) and `include_str!`'d by the MCP
server, so both speak the same labels. To add a well-known command: add it to
`PROTOCOL.md`'s registry **and** this file (keep them in lockstep), then any
device advertising that id gets the rich form for free.
