# Duta in Wireshark — extcap

Capture a Duta's DATA stream **live in Wireshark**. The one thing you need is the
**extcap** binary:

| File | What it is | Installs to |
|------|-----------|-------------|
| `sutra-extcap` (built from [`src-tauri/src/bin/sutra_extcap.rs`](../src-tauri/src/bin/sutra_extcap.rs)) | a Wireshark **extcap** — makes each Duta a live capture interface | Wireshark *Personal Extcap path* |

The extcap probes what the device bridges and picks the link type to match. For a
**BLE sniffer** it emits `LINKTYPE_BLUETOOTH_LE_LL_WITH_PHDR`, so **Wireshark's
own `btle` dissector** decodes everything natively — PDU types, advertising
addresses with vendor names, AD structures, CRC status. **No plugin required.**
Other (untyped) streams come through as raw `USER0`. Background: [`../EXTCAP.md`](../EXTCAP.md).

> The [`skrit-ble-sniff.lua`](skrit-ble-sniff.lua) dissector in this folder is
> **optional** — it predates the native BTLE path and is now kept as the reference
> parser for Sutra's own decoder layer ([`../DECODERS.md`](../DECODERS.md)). You do
> *not* need it for live BLE capture anymore.

---

## 1. Build the extcap binary

On Windows, build from **PowerShell** (a Developer/MSVC environment — a plain
`bash` shell won't have `msvcrt.lib` on its `LIB` path and linking fails):

```powershell
cd sutra\src-tauri
cargo build --release --bin sutra-extcap
# -> target\release\sutra-extcap.exe
```

## 2. Install the binary

Find the exact folder in **Wireshark ▸ Help ▸ About Wireshark ▸ Folders**
("Personal Extcap path"). On Windows:

```powershell
copy sutra\src-tauri\target\release\sutra-extcap.exe "$env:APPDATA\Wireshark\extcap\"
```

(macOS/Linux: `~/.local/lib/wireshark/extcap/`, or the path the Folders dialog shows.)

That's it — BLE decode is native, no Lua plugin to install.

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

For a BLE sniffer the **Protocol** column reads **`LE LL`** (Wireshark's native
dissector). Source = advertising address (with the vendor name resolved),
Destination = `Broadcast`; the Info column shows `ADV_IND` / `ADV_NONCONN_IND` /
`SCAN_REQ`. Expand a packet for the pseudo-header (RF channel → advertising
channel, signal dBm, CRC valid) and the full LL PDU with parsed AD structures.

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
- **`LE LL` shows `Unknown` PDU types?** That would mean the RF-channel mapping is
  off (the pseudo-header needs the *physical* RF channel, not the BLE channel
  index) — the binary handles 37/38/39 → RF 0/12/39. Rebuild if you've patched it.
- **CRC shows `0x000000`:** expected — the radio CRC-checks on-device and discards
  the bytes, so the pseudo-header marks CRC-valid and ships a placeholder.

---

## Testing the extcap from the CLI (no Wireshark)

Wireshark drives the binary through four calls — you can run them by hand to
debug. (`--fifo` accepts a plain file path, so a capture lands in a normal pcap.)

```powershell
$bin = "sutra\src-tauri\target\release\sutra-extcap.exe"

& $bin --extcap-interfaces                                   # list Dutas
& $bin --extcap-dlts   --extcap-interface COM34              # -> BTLE (256) for a sniffer
& $bin --extcap-config --extcap-interface COM34              # -> password, max-time
& $bin --capture --extcap-interface COM34 --fifo cap.pcap --max-time 4

# verify with tshark — native btle, no plugin:
& "C:\Program Files\Wireshark\tshark.exe" -r cap.pcap -c 10
```

## The `.lua` is a preview of Sutra's own decoders

[`skrit-ble-sniff.lua`](skrit-ble-sniff.lua) parses one DATA record into fields.
It's no longer needed for capture (native BTLE supersedes it), but it stays as the
reference for Sutra's `mlua` decoder layer: the **same parsing logic** will power
the in-app viewer and filters — Lua is the shared language between the two tools.
The contract is sketched in [`../DECODERS.md`](../DECODERS.md); the
record-layout/AD-walk logic in this file ports almost directly.
