# sutra MCP server

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

### Snippets & secrets (by design)

Snippets are a **backend-owned store** shared between the app UI and the MCP
server (persisted to `snippets.json` in the app data dir). The LLM can **list
names**, **run by name**, and **create** snippets — but **can never read a
snippet's text**. So you can keep a `prod-login` snippet holding a password: the
model applies it (`run_snippet "prod-login"`) without ever seeing the secret.
Snippets the LLM creates appear live in the app.

> Indirect-exposure caveat: if the target **echoes** what a snippet sends (some
> consoles echo typed passwords), a later `read_console` could reveal it. Mark
> sensitive snippets `secret` and be mindful of echoing targets.

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

`STRING` and bare lines honor `\n \r \t \0 \xHH \\` escapes.

## Connect an MCP client

**Claude Code:**
```bash
claude mcp add --transport http sutra http://127.0.0.1:8765/mcp
```

**Claude Desktop** (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "sutra": { "type": "streamable-http", "url": "http://127.0.0.1:8765/mcp" }
  }
}
```

(Other clients: add a streamable-HTTP / HTTP MCP server pointing at the same URL.)

## ⚠️ Safety

This gives the connected LLM **read/write access to whatever is on the DATA
port** — it can run arbitrary commands on the target (e.g. a root console). It's
bound to **localhost** and **disabled until you start it**, but treat enabling it
as handing the model a live shell. Stop the server when you're done.
