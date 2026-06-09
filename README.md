# Sutra

**Sutra** is the *thread* — a desktop app (Tauri + React) that connects you, and
an LLM, to a device's serial console.

> *sūtra* (Sanskrit सूत्र) — *"thread"; that which threads things together.*

It pairs with **[Duta](https://github.com/sutra-console/duta)** firmware over the
shared **[skrit](https://github.com/sutra-console/skrit)** protocol — but also
drives any plain COM port.

## Features

- **Universal serial console** (ghostty-web) — connect a Duta device *or* any
  COM port; baud/parity/stop, named connection profiles, link online/offline.
- **Macros** — saved, reorderable command scripts (line-based Bash Bunny /
  DuckyScript) plus an expect engine: `WAITFOR`, `RUN` (with exit-code capture),
  `IF OK…ELSE…END`, `SET` an output, `WAITIO` on an input, `$call` another macro.
  A **run queue** shows in-flight macros (e.g. blocked on `WAITFOR`) and lets you
  cancel them. Macros flagged secret are never readable by the LLM and are
  redacted from console reads.
- **Device-driven controls** — the UI renders the relays/LED/inputs the device
  *self-describes*, by name.
- **MCP server** — per-tool toggles; lets an LLM read the console, run/author
  macros (run-by-name only), drive outputs, and manage the connection.

## Run

```sh
bun install
bun run tauri dev
```

## Protocol & MCP

The app mirrors the skrit contract in
[`src-tauri/src/protocol.rs`](src-tauri/src/protocol.rs) and
[`src/lib/ttl.ts`](src/lib/ttl.ts); the spec is vendored in [PROTOCOL.md](PROTOCOL.md)
(canonical home: the [skrit](https://github.com/sutra-console/skrit) repo). MCP
usage is in [MCP.md](MCP.md).
