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

1. Get a Duta on your network (captive portal or Sutra's **Network** button) —
   it advertises `_skrit._tcp` over mDNS.
2. Open Wireshark: each discovered Duta is listed as `Duta: <name> — <ip>`.
3. The interface's gear icon sets the **device password** (default `duta`) —
   the session is auth-gated like every skrit network transport.
4. Start the capture. Each chunk of console bytes arrives as one timestamped
   packet (classic pcap, `LINKTYPE_USER0`).

## How it works

```
Wireshark ── runs ──> sutra-extcap ── ws://<duta>:9555/ (AUTH) ──> Duta
     ^                      │
     └── pcap over fifo <───┘   (DATA mux channel -> one packet per chunk)
```

- `--extcap-interfaces` → 2.5 s mDNS browse, one interface per Duta.
- `--capture --fifo <pipe>` → WebSocket connect + `AUTH`, then the DATA channel
  is written as pcap records until Wireshark stops the capture (broken pipe) or
  the WS closes.
- The capture session coexists with USB: a Duta serves USB and WebSocket
  simultaneously, with the console teed to both — so you can drive the target
  from Sutra over USB *while* Wireshark captures the same traffic over WiFi.

## v1 scope & the path forward

- **Today**: WebSocket Dutas; the raw UART console as `USER0` packets
  (chunk-per-packet — boundaries are transport reads, not protocol framing).
- **Next** (with typed DATA streams): real link types per kind — SocketCAN for
  `can`, `LINKTYPE_NORDIC_BLE` for the sniffer, I²C transaction records — and a
  move to **pcapng** so one capture can carry several streams (console + I²C)
  as separate interfaces.
