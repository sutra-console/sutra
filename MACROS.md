# Macros

Macros are Sutra's automation unit: a **line-based script** (Bash Bunny / DuckyScript
style, plus an expect engine) that types into the target console, waits on its output,
and drives the Duta's IO. You author them in the **Macros** card, the LLM can author
and run them over MCP (never read them, see *Secrets*), and every macro carries a
**tier** that says where it can execute (the app, or a device VM down to a $2 CH552).

## Language

One command per line. A line with no command keyword is **typed verbatim + Enter**.

### Typing & timing

| Command | Effect |
|---------|--------|
| `STRING <text>` | type text (no newline) |
| `STRINGLN <text>` | type text + Enter |
| `ENTER` / `CR` / `LF` / `CRLF` | newline variants |
| `TAB` `ESC` `SPACE` | those keys |
| `DELAY <ms>` / `WAIT <ms>` | pause (capped 60 s) |
| `CTRL <c>` | control byte (`CTRL c` → 0x03) |
| `HEX <hh hh ..>` | raw bytes |
| `REPEAT <n>` | repeat the previous line n times |
| `REM <text>` | comment |
| `Q <cmd>` / `QUACK <cmd>` | Bash Bunny prefix (`Q STRING foo`, `Q ENTER`) |
| *(bare line)* | typed verbatim + Enter |

`STRING` and bare lines honor `\n \r \t \0 \xHH \\` escapes.

### Expect & control flow (synchronise to console output)

| Command | Effect |
|---------|--------|
| `WAITFOR <text>` / `EXPECT <text>` | block until `<text>` appears on the console |
| `RUN <cmd>` (`SMARTWAIT`/`DO`) | run `<cmd>`, **wait for it to finish**, capture its exit code |
| `WAITOK` | abort the macro if the last `RUN`/`WAITFOR` failed |
| `IF OK` / `IF FAIL` … `ELSE` … `END` | branch on the last outcome |
| `TIMEOUT <ms>` | wait timeout for `WAITFOR`/`RUN` (default 10000) |
| `SET <name\|index> <0\|1>` | drive an output by name (`SET Relay1 0`); needs a CMD link |
| `WAITIO <name> <op> <value>` | wait until an input passes (`WAITIO LDR > 124`); ops `> < >= <= == !=` |
| `$Name` | run another macro inline (e.g. `$Login`); nesting capped at depth 8 |

`RUN` captures `$?` by appending a split-marker `echo` (`echo "sut""ra_N_:$?"`) so the
echoed command can't false-match: it needs a **POSIX shell** on the target.
Control-flow ops are *transparent*: they ride whatever read set the OK/FAIL outcome,
so a bare `WAITFOR` timeout also trips `WAITOK`/`IF FAIL`.

### Example

```
REM log in, check a service, restart it if it's down
WAITFOR login:
admin
WAITFOR Password:
$Creds                    REM secret macro holds the password
RUN systemctl is-active myapp
IF FAIL
  RUN systemctl restart myapp
  WAITOK
END
SET Relay1 1              REM power the fixture once we're healthy
```

## Tiers: where a macro can run

Every macro is **classified automatically** from its steps (never hand-tagged); the
badge shows in the Macros card, with a tier filter beside the set filter.

| Tier | Name | Steps | Runs on |
|------|------|-------|---------|
| **1** | Replay | `STRING`/keys/`HEX`, `DELAY`, `SET` | any device VM (open-loop) |
| **2** | Interactive | + `WAITFOR`, `WAITIO`, `WAITOK` | capable device VMs (closed-loop) |
| **3** | App-only | `RUN` (+ `IF` riding its exit code) | the Sutra player only |

A device advertises the highest tier its VM executes in `INFO.macro_tier`. Tier-1/2
macros compile to **skrit-mc bytecode** for the on-device VM (the compiler is in
progress, see [ROADMAP.md](ROADMAP.md)); today all macros run in the app's player,
and the badge tells you where each one *will* be able to live.

## Running

Run a macro from its row (or `run_macro` over MCP). The **Running** card shows
in-flight macros (e.g. one blocked on a `WAITFOR`) and lets you cancel them
mid-flight. `$call`s show as one run.

## Secrets

Mark a macro **secret** (the lock icon, or `secret: true` via MCP) for passwords and
keys:

- The LLM can **list names, run by name, and create** macros, but it can **never read a
  macro's text**, secret or not.
- Echoed secrets are **redacted from MCP console reads**: the typed literals of every
  secret macro (bare lines + `STRING` args, ≥3 chars) appear as `<REDACTED>` in
  `read_console` output. Your terminal is unaffected.

So `$Creds` in the example applies a real password without the model ever seeing it.

## Storage

Macros are a backend-owned store (`macros.json` in the app data dir), shared live
between the app UI and the MCP server: a macro the LLM creates appears in the card
immediately, and vice versa.
