# Sutra decoders — the `mlua` filter/decode layer (design sketch)

> Status: 🔭 designed, not built. This pins the **contract** before we wire it
> into the UI. See the org ROADMAP's *Filters / decoders* and *Wireshark interop*.

## Why Lua

A typed DATA stream is bytes until something gives them meaning. That "something"
is a small, sandboxed, **user-supplied** transform — and the natural language for
it is **Lua**, because Lua is already Wireshark's dissector language. We don't run
Wireshark `.lua` files verbatim (their `Proto`/`ProtoField`/`tvb` API is
Wireshark's), but the *parsing logic* is portable: the record walk you write to
dissect a bus in Wireshark moves into Sutra with only the host calls swapped. That
is the round trip the roadmap is built around — understand in Wireshark, *interact*
in Sutra — and Lua is what carries the knowledge across.

`mlua` (Rust) is the embed: mature, `Send`-able, and sandboxable — strip
`io`/`os`/`require`, and cap instructions + memory so a community decoder can't
wedge the UI (see *Sandbox*).

## The contract

A decoder is a Lua chunk that returns a table. Pure data + pure functions — no I/O,
no globals that persist across records.

```lua
-- decoders/ble-sniff.lua
return {
  name = "BLE advertising sniffer",
  kind = "ble-sniff",          -- the DATA_DESC kind this decoder claims (see skrit)
  columns = { "channel", "rssi", "type", "addr", "name" },  -- summary table headers

  -- bytes (one DATA record) -> structured result. `b` is the byte-cursor helper
  -- (below), NOT a raw Lua string — so reads are explicit and endian-safe.
  decode = function(b)
    local ch   = b:u8(4)
    local rssi = -b:u8(5)
    local plen = b:u8(10)
    local b0   = b:u8(11)
    local ptype = b0 % 16
    local PT = { [0]="ADV_IND",[2]="ADV_NONCONN_IND",[4]="SCAN_RSP",[6]="ADV_SCAN_IND" }
    local tname = PT[ptype] or string.format("0x%02x", ptype)

    local fields = {
      { name = "Channel",        value = ch },
      { name = "RSSI (dBm)",     value = rssi },
      { name = "Access Address", value = b:u32le(6), hex = true },
      { name = "PDU Type",       value = tname },
    }
    local addr, name
    if ptype == 0 or ptype == 2 or ptype == 6 or ptype == 4 then
      addr = b:mac_le(13)                 -- 6 bytes LE -> "aa:bb:.." MSB-first
      fields[#fields+1] = { name = "Advertising Address", value = addr }
      name = b:ad_name(19, 11 + plen)     -- walk AD structs, return local name
      if name then fields[#fields+1] = { name = "Local Name", value = name } end
    end

    return {
      summary = string.format("ch%d  %d dBm  %s%s", ch, rssi, tname,
                              name and ("  — "..name) or ""),
      row = { channel = ch, rssi = rssi, type = tname, addr = addr, name = name },
      fields = fields,          -- detail pane (a flat list, or nest with `children`)
    }
  end,

  -- optional include/highlight predicate over a decoded row. Absent => keep all.
  filter = function(row) return row.channel == 37 end,
}
```

### Return shape

`decode(b)` returns `{ summary, row, fields }`:

- **`summary`** — one-line string for the packet list (the Info column).
- **`row`** — flat `{ column = value }` keyed by the decoder's `columns`; drives
  the sortable/groupable summary table (the existing BLE/I²C panels generalize to
  this).
- **`fields`** — the detail tree: `{ name, value, hex?, raw?, children? }`. `raw`
  is an optional `{offset, len}` for byte-range highlighting; `children` nests.

A decode that can't parse a record returns `nil` (shown as raw/undecoded) — never
throws; an erroring decoder is disabled with its message surfaced, never silent.

### The byte cursor `b`

Raw Lua string indexing is error-prone, so the host hands `decode` a thin reader
(implemented in Rust over the record slice). One obvious API, no surprises:

| call | meaning |
|------|---------|
| `b:len()` | record length |
| `b:u8(i)` / `b:u16le(i)` / `b:u16be(i)` / `b:u32le(i)` / `b:u32be(i)` | fixed-width ints |
| `b:i8(i)` … | signed variants |
| `b:bytes(i, n)` | `n` bytes as a Lua string |
| `b:hex(i, n)` | `"a1b2c3"` |
| `b:ascii(i, n)` | printable-ASCII string |
| `b:mac_le(i)` / `b:mac_be(i)` | 6-byte address → `"aa:bb:cc:dd:ee:ff"` |

`ad_name` above is *not* a primitive — it's a helper a decoder defines or imports
from a shared `ble.lua`; the host only provides the byte cursor + stdlib-minus-IO.

## Where it plugs in

```
DATA record bytes ──> decode(b) ──> { summary, row, fields }
                                       │        │        └─ detail pane (tree)
                                       │        └─ summary table row (sort/group)
                                       └─ packet-list Info line
                          filter(row) ──> include / highlight in the list
                          row.<field> ──> macro EXPECT matches on FIELDS, not bytes
```

- **Viewer.** The per-kind panels (BLE, I²C) become one generic record view driven
  by `columns`/`row`/`fields`. A kind with no decoder falls back to the current
  raw/hex view — nothing regresses.
- **Filter.** `filter(row)` is the in-app equivalent of a Wireshark display filter:
  a predicate over decoded rows for include/highlight. (A richer expression language
  can compile *to* this later; the predicate is the primitive.)
- **Macros.** Because a record becomes named fields, a macro `EXPECT` can assert on
  `row.type == "ADV_IND"` instead of a byte pattern — the payoff of decoding.

## Loading & matching

- Decoders live in the workspace: `.sutra/decoders/*.lua` (alongside `macros.json`,
  `captures/`, `i2c/`). User-owned and shareable, like the i²c defs.
- Each declares the `kind` it claims; Sutra picks the decoder whose `kind` matches
  the active `DATA_DESC`. Multiple decoders per kind => user chooses (or `match`
  narrows by a header byte).
- Built-in decoders (ble-sniff, i²c) ship embedded; a workspace file of the same
  name overrides, so anyone can fork one.

## Sandbox

`mlua` with the guard rails on:

- **No ambient authority:** remove `io`, `os`, `package`/`require`, `dofile`,
  `load*`. A decoder gets `string`, `table`, `math`, and the byte cursor — nothing
  that touches the machine.
- **Bounded:** instruction-count hook (interrupt long loops) + memory limit; a
  decode that runs away is killed and the decoder disabled, not the app.
- **Pure:** `decode`/`filter` are called per record with no persistent state; no
  way to leak data between records or to disk.

## Relationship to the Wireshark dissector

[`wireshark/skrit-ble-sniff.lua`](wireshark/skrit-ble-sniff.lua) already does this
parse — just against Wireshark's host API. The mapping is mechanical:

| Wireshark dissector | Sutra decoder |
|--------------------|---------------|
| `tvb(6,4):le_uint()` | `b:u32le(6)` |
| `tree:add(field, range, val)` | append to `fields` |
| `pinfo.cols.info = …` | `summary = …` |
| `pinfo.cols.protocol` | the decoder's `name`/`kind` |
| display filter `skrit_blesniff.channel == 37` | `filter = function(r) return r.channel==37 end` |

Same language, same record-walk logic, two thin host APIs. Write the parser once;
read it in Wireshark, *act on it* in Sutra.
