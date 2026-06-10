# Roadmap — skrit-mc tiered macro execution

Macros are authored as text, parsed to a shared IR (`Step`), and either interpreted by
the **Sutra** player or **compiled** to **skrit-mc** bytecode that a device VM runs. The
same front-end feeds both, so on-device runs match the app exactly.

Spans three repos: **sutra** (app + compiler + player), **duta** (firmware + VM),
**skrit** (the wire/bytecode contract).

## Tier model

A macro's capability **tier** is the max tier of its steps — derived, never hand-tagged.

| Tier | Name | Steps | Execution model | Runs on |
|------|------|-------|-----------------|---------|
| **1** | Replay | `EMIT` (STRING/keys/HEX), `DELAY`, `SETOUT` | open-loop — output is a fixed function of time | any device VM |
| **2** | Interactive | + `EXPECT`/`WAITFOR`, `WAITIO`, `WAITOK` | closed-loop — blocks/branches on a read | capable device VMs |
| **3** | App-only | `RUN` + the `WAITOK`/`IF` riding its exit code | host orchestration | Sutra player only |

Control-flow ops (`WAITOK`/`IF`/`ELSE`/`END`) are **tier-transparent** — they ride
whatever read set the OK/FAIL outcome, so a bare `WAITFOR` timeout also trips them.
A device advertises the highest tier its VM runs in `INFO.macro_tier` (`0` = no VM).

## Status

### Done
- [x] **skrit-mc ISA** — opcodes, tiers, outcome flag, `INFO.macro_tier`, scratch
      push-and-run, `STATUS 0x07` (over-tier). `PROTOCOL.md` + `protocol.h`, mirrored
      skrit ↔ duta.
- [x] **Tier classifier** (`serial.rs`) — `$call` inlined, control-flow transparent,
      cycle-guarded; recomputed on every read so editing a callee re-tiers its callers.
- [x] **UI** — per-row tier badge (Replay / Interactive / App-only) + tier filter beside
      the set filter; `INFO.macroTier` mirrored into the TS `DeviceInfo`.

### In progress
- [ ] **Compiler** `Vec<Step>` → skrit-mc bytecode (sutra)
  - resolve `TIMEOUT` into each `EXPECT`/`WAITIO`; inline `$call`; resolve
    `SETOUT`/`WAITIO` names → indices against the connected device
  - reject Tier-3 (`RUN`) for a device target; split `EMIT > 255` / `DELAY > 65535`
  - round-trip unit test (text → bytecode → decode), no hardware needed

### Planned
- [ ] **CH552 VM** (duta) — `EMIT`/`DELAY`/`SETOUT` + `EXPECT`/`WAITIO`/`WAITOK`
  - streaming substring matcher (one index, no buffer); cooperative wait loop that
    keeps USB + the host console alive; tee RX to console while matching
  - behind a build flag (trades against OLED on flash-tight boards); measure flash
  - target: **Tier 2** on the CH552
- [x] **INFO `macro_tier` byte** (duta) — advertised by every shared-core port (ESP32 /
      Pico / nRF / host report tier 2); the CH552 still needs it once its VM lands
- [x] **Scratch push-and-run** — write bytecode to reserved id `0xFF`, `MACRO_RUN(0xFF)`;
      works with zero persistence (RAM-bound) on every shared-core port
- [ ] **Flip Save-to-Duta gate** — gate on `macro.tier <= device.macroTier` once the
      firmware reports it (kept on the old raw path until then to avoid a regression)
- [ ] **Persisted programs** — store bytecode via `MACRO_WRITE_*`; VM streams opcodes
      from storage page-by-page (not RAM-bound) on `CAP_STORE` devices

### Later / v2
- [ ] **Branching opcodes** `IF`/`ELSE`/`ENDIF` (reserved `0x20`–`0x22`) — forward-scan
      jumps in the VM; lifts on-device conditionals past linear expect
- [x] **Tier-1 / Tier-2 VMs on bigger MCUs** — ESP32 / nRF52 (Zephyr) / RP2040 — done
      via the shared core (`duta/platforms/common/skrit_device.h`), Tier 2 on all three

## Invariants
- The device never sees `STRING`/`WAITFOR` text — only resolved opcodes.
- Secret macros stay unreadable to the MCP/LLM (names + run-by-name + create only).
- One front-end (`parse_macro` → `Step`) feeds both the player and the compiler.
