# Sutra MCP server

The desktop app embeds an **MCP server** so an LLM can read the target device's
serial console and drive it. It's **off by default** — enable it in the app's
**MCP server** card (sidebar), pick a port (default **8765**), click **Start**.

Transport: **streamable HTTP** at `http://127.0.0.1:<port>/mcp` (localhost only).

## Tools exposed

| Tool | Args | Does |
|------|------|------|
| `read_console` | `max_bytes?` | returns recent target-console output (DATA port) |
| `write_console` | `text`, `newline?` | sends text/keystrokes to the target (DATA port) |
| `device_info` | — | firmware version, caps, output count |
| `get_outputs` | — | relay/LED bitmap (bit0=R1, bit1=R2, bit2=AuxLED) |
| `set_output` | `index`, `on` | 0=Relay1, 1=Relay2, 2=Aux LED |
| `list_snippets` | — | snippet **names only** (never contents) |
| `run_snippet` | `name` | runs a stored snippet by name (sends its text); returns `applied`, not the content |
| `create_snippet` | `name`, `text`, `secret?` | author/overwrite a reusable snippet |
| `list_serial_ports` | — | enumerate serial ports (Duta tagged) |
| `connect_buddy` | — | auto-detect + connect a Duta (DATA+CMD) |
| `connect_port` | `port`, `baud?`, `parity?`, `stop_bits?` | connect any serial port as a console |
| `disconnect_port` | — | disconnect |
| `set_serial` | `baud`, `parity?`, `stop_bits?` | change DATA serial params |
| `connection_status` | — | current port/baud/buddy status |

### Tool toggles (Settings ▸ MCP tools)

Each group is individually switchable in the app's **Settings**. A disabled
group is **removed from the router** — it doesn't appear in `tools/list` and any
call is rejected, so the model can't even see it. Groups:
`console_read`, `console_write`, `outputs`, `snippets_run`, `snippets_create`,
`connection`. All on by default. Toggling restarts a running server so the change
takes effect on the client's next list.

### Snippets & secrets (by design)

Snippets are a **backend-owned store** shared between the app UI and the MCP
server (persisted to `snippets.json` in the app data dir). The LLM can **list
names**, **run by name**, and **create** snippets — but **can never read a
snippet's text**. So you can keep a `prod-login` snippet holding a password: the
model applies it (`run_snippet "prod-login"`) without ever seeing the secret.
Snippets the LLM creates appear live in the app.

> Echo redaction: if the target **echoes** what a secret snippet typed (some
> consoles echo passwords), `read_console` would otherwise leak it. So the typed
> literals of every `secret` snippet (bare lines + `STRING` args, ≥3 chars) are
> replaced with `<REDACTED>` in `read_console` output. The human terminal is
> unaffected. Caveat: a transformed echo (e.g. masked `****`) won't match, and a
> very short secret over-redacts — over-redaction is the safe failure mode.

### Snippet macros (Bash Bunny / DuckyScript style)

Snippet text is a line-based payload — **one command per line**. A line with no
command keyword is **typed verbatim + Enter**.

```
REM log in and look around
admin
DELAY 1000
STRING whoami
ENTER
CTRL c
```

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

**Expect / control flow** (synchronise to console output):

| Command | Effect |
|---------|--------|
| `WAITFOR <text>` / `EXPECT <text>` | block until `<text>` appears on the console |
| `RUN <cmd>` (`SMARTWAIT`/`DO`) | run `<cmd>`, **wait for it to finish**, capture exit code (sentinel) |
| `WAITOK` | abort the macro if the last `RUN` exited non-zero |
| `IF OK` / `IF FAIL` … `ELSE` … `END` | branch on the last `RUN`'s exit code |
| `TIMEOUT <ms>` | wait timeout for `WAITFOR`/`RUN` (default 10000) |
| `SET <name\|index> <0\|1>` | drive an output by name (e.g. `SET Relay1 0`) — needs a CMD link |
| `WAITIO <name> <op> <value>` | wait until an input passes (`WAITIO LDR > 124`); ops `> < >= <= == !=` |
| `$Name` | run another snippet inline (e.g. `$Login`); nesting capped at depth 8 |

`RUN` captures `$?` by appending a split-marker `echo` (`echo "ttlb""uddy_N:$?"`) so
the echoed command can't false-match — it needs a **POSIX shell** on the target.

```
WAITFOR login:
admin
WAITFOR Password:
hunter2                 # bare line = typed + Enter (use a secret snippet for real creds)
RUN systemctl is-active myapp
IF FAIL
  RUN systemctl restart myapp
END
WAITOK
```

`STRING` and bare lines honor `\n \r \t \0 \xHH \\` escapes.

## Connect an MCP client

**Claude Code:**
```bash
claude mcp add --transport http sutra http://127.0.0.1:8551/mcp
```

**Claude Desktop** (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "sutra": { "type": "streamable-http", "url": "http://127.0.0.1:8551/mcp" }
  }
}
```

(Other clients: add a streamable-HTTP / HTTP MCP server pointing at the same URL.)

## ⚠️ Safety

This gives the connected LLM **read/write access to whatever is on the DATA
port** — it can run arbitrary commands on the target (e.g. a root console). It's
bound to **localhost** and **disabled until you start it**, but treat enabling it
as handing the model a live shell. Stop the server when you're done.
