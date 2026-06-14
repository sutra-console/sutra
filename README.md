<div align="center">
  <img src="assets/logo.png" alt="Sutra" width="160">
</div>

# Sutra

**Sutra** is the *thread*: a desktop app (Tauri + React) that connects you, and
an LLM, to a device's serial console.

> *sūtra* (Sanskrit सूत्र): *"thread"; that which threads things together.*

It pairs with **[Duta](https://github.com/sutra-console/duta)** firmware over the
shared **[skrit](https://github.com/sutra-console/skrit)** protocol, but also
drives any plain COM port.

## Features

- **Universal serial console** (ghostty-web): connect a Duta device *or* any
  COM port; baud/parity/stop, named connection profiles, link online/offline.
- **Bluetooth LE**: the toolbar's Bluetooth button scans for a Duta and connects
  over BLE (two skrit GATT services: DATA console + CMD); from there the console, controls,
  and macros all work identically to a serial link. *(Scaffold, see duta/zephyr.)*
- **Network (WebSocket)**: the globe button connects a Duta over `ws://`/`wss://`
  (the skrit-mux stream over WS binary frames), authenticating with the device
  password (default `duta`, prompts to change). The `host` reference is a runnable
  WS server.
- **Macros**: saved, reorderable command scripts (line-based Bash Bunny /
  DuckyScript) plus an expect engine: `WAITFOR`, `RUN` (with exit-code capture),
  `IF OK…ELSE…END`, `SET` an output, `WAITIO` on an input, `$call` another macro.
  A **run queue** shows in-flight macros (e.g. blocked on `WAITFOR`) and lets you
  cancel them. Macros flagged secret are never readable by the LLM and are
  redacted from console reads. Full language + tier reference: **[MACROS.md](MACROS.md)**.
- **Device-driven controls**: the UI renders what the device *self-describes*, typed
  by behavior: a toggle for digital IO, a 0-1023 slider + frequency/resolution badge
  for PWM, a per-pixel color picker for addressable RGB, live input readouts, all by
  name, with no per-board UI.
- **Yantra control surfaces**: ship a `.yantra` (YAML) describing a device's panel —
  buttons/sliders/readouts/tables laid out on a responsive canvas, each wired to what it
  *sends* (UART/I²C/INVOKE/CFG/outputs) and *filled* from (the live console or other widgets).
  A visual editor builds them; readouts/emitters give a reactive data flow; and a sandboxed
  **Lua** layer scripts logic, orchestration, and presentation. Full reference: **[YANTRA.md](YANTRA.md)**.
- **Configure device (runtime provisioning)**: on firmware that advertises the
  `provision` flag, re-pin the device's IO from the app: a per-pin role/name picker
  constrained to what each pin supports (strapping/dual-use pins warned, fixed pins
  locked), persisted on-device, applied on reboot. No reflashing.
- **MCP server**: per-tool toggles; lets an LLM read the console, run/author
  macros (run-by-name only), drive outputs (incl. PWM config + RGB), provision IO,
  and manage the connection.
- **Workspace** — pick a project folder (header, top-left) and Sutra keeps a
  `.sutra/` directory inside it: macros (`macros.json`) and capture exports
  (`captures/*.pcap`) live there, so a project's automation and recordings travel
  with the folder. Without one, macros fall back to the app data dir.
- **Wireshark integration**: the bundled `sutra-extcap` binary turns every Duta
  into a live Wireshark capture interface — USB-attached ones (probed + listed by
  name) and network ones (mDNS-discovered, auth-gated) alike, with the DATA stream
  delivered as pcap packets. A WiFi capture coexists with a Sutra USB session, so
  you can drive the target while Wireshark watches. See [EXTCAP.md](EXTCAP.md).

## Run

```sh
bun install
bun run tauri dev
```

## Protocol & MCP

The app mirrors the skrit contract in
[`src-tauri/src/protocol.rs`](src-tauri/src/protocol.rs) and
[`src/lib/skrit.ts`](src/lib/skrit.ts); the spec is vendored in [PROTOCOL.md](PROTOCOL.md)
(canonical home: the [skrit](https://github.com/sutra-console/skrit) repo). MCP
usage is in [MCP.md](MCP.md).
