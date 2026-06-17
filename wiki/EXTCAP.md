# sutra-extcap — Dutas as live Wireshark interfaces

`sutra-extcap` is a Wireshark **extcap** program (the same mechanism Nordic's BLE
sniffer uses). With it installed, every Duta on your LAN appears on Wireshark's
capture screen as a live interface — pick **"Duta: Duta S3-Zero — 10.0.0.29"**,
hit start, and the device's DATA stream lands in Wireshark in real time.

Wireshark is where you *understand* a bus; Sutra is where you *interact* with it —
this is the bridge between the two (see the org ROADMAP's *Wireshark interop*).

## Install

```sh
cargo build --release --bin sutra-extcap   # in src-tauri/
```

Copy the binary into Wireshark's **Personal Extcap path**
(Wireshark ▸ Help ▸ About ▸ Folders ▸ Personal Extcap path — e.g.
`%APPDATA%\Wireshark\extcap\` on Windows), then restart Wireshark.

## Use

Interfaces come from **both transports**:

- **USB** — plugged-in Dutas are probed (mux PING + name) and listed as
  `Duta: <name> — COMxx (USB)`. No password needed (USB isn't session-gated).
  Serial is exclusive: a port Sutra currently holds is skipped — disconnect in
  Sutra first, or capture that device over WiFi instead.
- **WiFi** — Dutas advertising `_skrit._tcp` (captive portal or Sutra's
  **Network** button gets them online) are listed as `Duta: <name> — <ip>
  (WiFi)`. The gear icon sets the **device password** (default `duta`).

Start the capture and packets flow live. The **link type follows what the device
bridges** (`sutra-extcap` probes `DATA_DESC` first):

- **ble-sniff** → `LINKTYPE_BLUETOOTH_LE_LL_WITH_PHDR` — each record is reframed
  into a BLE LL packet with a 10-byte pseudo-header, so **Wireshark's native
  `btle` dissector** decodes it in full (PDU types, advertising addresses with
  OUI names, AD structures, CRC status). No plugin needed.
- **anything else** (UART console today) → `LINKTYPE_USER0`, one packet per
  transport chunk (raw bytes; boundaries are reads, not protocol framing).

The same physical board can appear twice — once per transport; the WiFi one
coexists with a Sutra USB session (the console is teed to every link), so you can
drive the target from Sutra while Wireshark captures.

## How it works

```
Wireshark ── runs ──> sutra-extcap ──┬── COMxx (USB, DTR-only discipline) ──> Duta
     ^                      │        └── ws://<duta>:9555/ (AUTH) ──────────> Duta
     └── pcap over fifo <───┘   (DATA mux channel -> one packet per chunk)
```

- `--extcap-interfaces` → 2.5 s mDNS browse, one interface per Duta.
- `--extcap-dlts` → probes the device's `DATA_DESC` kind and reports the matching
  link type (BTLE for a sniffer, USER0 otherwise).
- `--capture --fifo <pipe>` → connect (+ `AUTH` over WS), probe the kind, write
  the pcap header for that link type, then stream reframed records until
  Wireshark stops the capture (broken pipe) or the link closes.
- The capture session coexists with USB: a Duta serves USB and WebSocket
  simultaneously, with the console teed to both — so you can drive the target
  from Sutra over USB *while* Wireshark captures the same traffic over WiFi.

## Scope & the path forward

- **ble-sniff → native BTLE** (`LINKTYPE_BLUETOOTH_LE_LL_WITH_PHDR`): full decode
  by Wireshark's own dissector. The radio CRC-checks on-device, so the pseudo-
  header sets CRC-valid and carries a placeholder (the real 3-byte CRC is dropped
  by the firmware). Advertising PDUs today (the sniffer's scope).
- **ieee802154 → native 802.15.4** (`LINKTYPE_IEEE802_15_4_TAP`): each record is
  wrapped in a TAP pseudo-header (FCS type, RSS, channel, LQI) + the MAC frame, so
  Wireshark's own stack decodes **Zigbee and Thread** (and 6LoWPAN/Matter). The
  nRF52840 radio FCS-checks in hardware (bad frames dropped), and the TAP header
  declares "no FCS" so frames dissect clean. Hardware-verified against a live
  Zigbee network (NWK commands, data-request/ack polling).
- **everything else → `USER0`**: the raw stream (UART console), one packet per
  chunk. The optional [`wireshark/skrit-ble-sniff.lua`](wireshark/skrit-ble-sniff.lua)
  dissector predates the native path — kept as the decoder reference (see
  [`DECODERS.md`](DECODERS.md)); the native BTLE link type supersedes it for BLE.
- **Next**: more typed link types per kind (SocketCAN for `can`, I²C transaction
  records), connection-following in the sniffer, and a move to **pcapng** so one
  capture carries several streams (console + I²C) as separate interfaces.
