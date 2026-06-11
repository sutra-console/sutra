# Duta in Wireshark — extcap + dissector

Capture a Duta's DATA stream **live in Wireshark**. Two pieces work together:

| File | What it is | Installs to |
|------|-----------|-------------|
| `sutra-extcap` (binary, built from [`src-tauri/src/bin/sutra_extcap.rs`](../src-tauri/src/bin/sutra_extcap.rs)) | a Wireshark **extcap** — makes each Duta a live capture interface | Wireshark *Personal Extcap path* |
| [`skrit-ble-sniff.lua`](skrit-ble-sniff.lua) | a **dissector** — decodes the captured `USER0` bytes into BLE-sniff fields | Wireshark *Personal Lua Plugins* |

The extcap gets the bytes in; the dissector makes them readable. The extcap alone
works (you'll just see raw `USER0` hex); add the dissector for a real **BLE-Sniff**
column. Background and design: [`../EXTCAP.md`](../EXTCAP.md).

---

## 1. Build the extcap binary

On Windows, build from **PowerShell** (a Developer/MSVC environment — a plain
`bash` shell won't have `msvcrt.lib` on its `LIB` path and linking fails):

```powershell
cd sutra\src-tauri
cargo build --release --bin sutra-extcap
# -> target\release\sutra-extcap.exe
```

## 2. Install both pieces

Find the exact folders in **Wireshark ▸ Help ▸ About Wireshark ▸ Folders**
("Personal Extcap path" and "Personal Lua Plugins"). On Windows they are:

```powershell
# extcap binary
copy sutra\src-tauri\target\release\sutra-extcap.exe "$env:APPDATA\Wireshark\extcap\"
# dissector
copy sutra\wireshark\skrit-ble-sniff.lua            "$env:APPDATA\Wireshark\plugins\"
```

(macOS/Linux: `~/.local/lib/wireshark/extcap/` and `~/.local/lib/wireshark/plugins/`,
or the per-user paths the Folders dialog shows.)

## 3. Run Wireshark

1. **Restart Wireshark** — it scans the extcap folder *at startup*. If the folder
   was just created, a running instance won't see the new interface.
2. On the welcome screen, find the interface:
   **`Duta: <name> — COM## (USB, skrit DATA)`** (USB), or
   **`Duta: <name> — <ip> (WiFi)`** for a Duta on `_skrit._tcp`.
3. *(WiFi only)* click the **gear ⚙** next to the interface to set the device
   **password** (default `duta`) and an optional **stop-after** timer. USB needs
   nothing — it isn't session-gated.
4. **Double-click** the interface to start. Packets stream in live.

With the dissector loaded, the **Protocol** column reads `BLE-Sniff` and the
**Info** column reads like `ch37  -61 dBm  ADV_IND — <name>`. Expand a packet for
channel, RSSI, access address, PDU type, advertising address, and parsed AD
structures (flags, local name, manufacturer data…).

> Edited the `.lua` and don't want to restart? **Analyze ▸ Reload Lua Plugins**
> (`Ctrl+Shift+L`) reloads dissectors without dropping Wireshark.

---

## Gotchas

- **Serial is exclusive.** A COM port that Sutra (or anything) currently holds is
  skipped by the extcap, and vice-versa — disconnect in Sutra before capturing
  that board over USB. **WiFi has no such limit:** the console is teed to every
  link, so you can drive the target from Sutra over USB *while* Wireshark captures
  the same board over WiFi.
- **Interface missing?** Confirm `sutra-extcap.exe` is in the *Personal Extcap
  path* the Folders dialog names, then restart. Sanity-check from a shell:
  `sutra-extcap.exe --extcap-interfaces` should print an `interface {…}` line per
  Duta.
- **Raw hex instead of BLE-Sniff?** The dissector isn't loaded — check it's in
  *Personal Lua Plugins* and hit `Ctrl+Shift+L`. It auto-binds to `USER0`; no
  `DLT_USER` preference setup is needed.
- **Today's scope:** advertising PDUs as `LINKTYPE_USER0` (one record per packet).
  Real `LINKTYPE_NORDIC_BLE` + `pcapng` multi-stream is the next step — see
  [`../EXTCAP.md`](../EXTCAP.md).

---

## Testing the extcap from the CLI (no Wireshark)

Wireshark drives the binary through four calls — you can run them by hand to
debug. (`--fifo` accepts a plain file path, so a capture lands in a normal pcap.)

```powershell
$bin = "sutra\src-tauri\target\release\sutra-extcap.exe"

& $bin --extcap-interfaces                                   # list Dutas
& $bin --extcap-dlts   --extcap-interface COM34              # -> USER0 (147)
& $bin --extcap-config --extcap-interface COM34              # -> password, max-time
& $bin --capture --extcap-interface COM34 --fifo cap.pcap --max-time 4

# then verify with tshark + our dissector:
& "C:\Program Files\Wireshark\tshark.exe" -r cap.pcap `
    -X "lua_script:sutra\wireshark\skrit-ble-sniff.lua" `
    -T fields -e skrit_blesniff.channel -e skrit_blesniff.rssi `
    -e skrit_blesniff.aa -e skrit_blesniff.pdu_type -e skrit_blesniff.adv_addr -E header=y
```

## The dissector is a preview of Sutra's own decoders

`skrit-ble-sniff.lua` parses one DATA record into fields. Sutra will embed the
same idea (via `mlua`) so the **same parsing logic** powers the in-app viewer and
filters — Lua is the shared language between the two tools. The contract for that
is sketched in [`../DECODERS.md`](../DECODERS.md); the record-layout/AD-walk logic
in this file ports almost directly.
