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

## What you can do with it

A Duta and Sutra together are a general-purpose bridge between you (or an LLM) and
whatever hardware is in front of you. A few of the things people reach for it to do:

- **A real KVM for the rack.** Drop a Duta on your homelab's console ports (a
  headless box's serial header, a switch/router/PDU console) and reach them
  remotely over WiFi (mDNS-discovered) or Bluetooth. It's out-of-band serial
  access, the KVM-over-IP equivalent, with no OpenGear/Avocent appliance and no
  trudging over to the rack with a USB-serial dongle. An LLM can hold the console too.
- **Turn an MCU into a Wireshark capture board.** Port a Duta build to your own
  radio/bus hardware and it becomes a live, *named* Wireshark interface (BLE,
  802.15.4/Zigbee/Thread, I²C, raw serial), dissected in real time over
  `sutra-extcap`. No proprietary sniffer app.
- **Automate arduous hardware testing.** Script the tedious bring-up and QA loops
  with macros (`WAITFOR`, `RUN` with exit-code capture, `IF/ELSE`, `SET`,
  `WAITIO`), watch the run queue, record results to pcap, and hand the whole thing
  to an LLM over MCP to run unattended.
- **Replace sketchy manufacturer apps.** Instead of a vendor's one-off
  configurator, describe a board's panel once as a `.yantra` (buttons, sliders,
  and readouts, scripted in Lua) and get a clean, reusable control surface that
  travels with your project.
- **…or just use it as a serial console.** Point it at any plain COM port and
  you've got a fast, modern terminal. No Duta required.

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
  redacted from console reads. Full language + tier reference: **[MACROS.md](wiki/MACROS.md)**.
- **Device-driven controls**: the UI renders what the device *self-describes*, typed
  by behavior: a toggle for digital IO, a 0-1023 slider + frequency/resolution badge
  for PWM, a per-pixel color picker for addressable RGB, live input readouts, all by
  name, with no per-board UI.
- **Yantra control surfaces**: ship a `.yantra` (YAML) describing a device's panel
  of buttons/sliders/readouts/tables laid out on a responsive canvas, each wired to what it
  *sends* (UART/I²C/INVOKE/CFG/outputs) and *filled* from (the live console or other widgets).
  A visual editor builds them; readouts/emitters give a reactive data flow; and a sandboxed
  **Lua** layer scripts logic, orchestration, and presentation. Full reference: **[YANTRA.md](wiki/YANTRA.md)**.
- **Configure device (runtime provisioning)**: on firmware that advertises the
  `provision` flag, re-pin the device's IO from the app: a per-pin role/name picker
  constrained to what each pin supports (strapping/dual-use pins warned, fixed pins
  locked), persisted on-device, applied on reboot. No reflashing.
- **MCP server**: per-tool toggles; lets an LLM read the console, run/author
  macros (run-by-name only), drive outputs (incl. PWM config + RGB), provision IO,
  and manage the connection.
- **Workspace**: pick a project folder (header, top-left) and Sutra keeps a
  `.sutra/` directory inside it: macros (`macros.json`) and capture exports
  (`captures/*.pcap`) live there, so a project's automation and recordings travel
  with the folder. Without one, macros fall back to the app data dir.
- **Wireshark integration**: the bundled `sutra-extcap` binary turns every Duta
  into a live Wireshark capture interface, whether USB-attached (probed and listed
  by name) or network (mDNS-discovered, auth-gated), with the DATA stream delivered
  as pcap packets. A WiFi capture coexists with a Sutra USB session, so you can
  drive the target while Wireshark watches. See [EXTCAP.md](wiki/EXTCAP.md).

<div align="center">
  <img src="assets/zigbeeDecode.png" alt="Live 802.15.4 / Zigbee capture from a Duta, dissected in Wireshark via sutra-extcap" width="640">
  <br><sub>Live 802.15.4 / Zigbee capture from a Duta, dissected in Wireshark via <code>sutra-extcap</code>.</sub>
</div>

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
usage is in [MCP.md](wiki/MCP.md).
