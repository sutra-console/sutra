# skrit — CMD-port wire protocol

The device exposes the two roles below. How they map onto the wire depends on the
**transport** (see *Transports*): a dual-CDC device gives them two USB interfaces; a
single-channel device (single USB-CDC, TCP, BLE) carries both over one stream via
*skrit-mux*.

| Role | Job | Framing |
|------|-----|---------|
| **DATA** | transparent USB↔UART bridge — the target console | **raw bytes**, no framing |
| **CMD** | control / macros / config | this protocol |

The **DATA** role is never framed — it carries the target's serial console verbatim
and is what the terminal widget renders. Everything below is the **CMD** role only.

The CMD port speaks **two interchangeable modes** so it stays debuggable by hand while
being efficient for the app:

- **ASCII line mode** — human-typeable (`R1 ON\n`, `STATUS\n`, `PING\n`). Any line that
  starts with a printable byte (`>= 0x20`) and ends in `\n`/`\r` is parsed as text.
- **Binary frame mode** — COBS-framed packets for the desktop app and bulk transfers.
  Recognised by the `0x00` frame delimiter.

A receiver disambiguates per message: a `0x00` terminator ⇒ decode the preceding bytes
as a COBS frame; a printable-led run ending in newline ⇒ ASCII line.

---

## Binary frame

On the wire, every binary message is delimited by `0x00` on **both** sides:

```
0x00 , COBS( payload ) , 0x00
```

[COBS](https://en.wikipedia.org/wiki/Consistent_Overhead_Byte_Stuffing) removes all
`0x00` bytes from `payload`, so the `0x00` bytes are unambiguous frame delimiters and
the link can always resync after a glitch. The **leading** `0x00` is what lets the
firmware switch cleanly into binary mode (a `0x00` while idle ⇒ a binary frame is
starting; a printable byte ⇒ an ASCII line). Back-to-back frames simply share
delimiters; empty segments between two `0x00`s are ignored.

The **decoded `payload`** is a fixed little header + body + CRC:

```
 0      1      2      3 .. 3+LEN-1      last
+------+------+------+-------------------+------+
| TYPE | SEQ  | LEN  |     BODY[LEN]     | CRC8 |
+------+------+------+-------------------+------+
```

| Field | Size | Meaning |
|-------|------|---------|
| `TYPE` | 1 | message type (table below). Response = request `TYPE \| 0x80`. |
| `SEQ`  | 1 | rolling id chosen by the requester; echoed in the response so the app can match replies. |
| `LEN`  | 1 | body length, **0..64** (capped to fit the CH552's RAM). |
| `BODY` | LEN | type-specific payload. |
| `CRC8` | 1 | CRC-8/ATM (poly `0x07`, init `0x00`) over `TYPE..BODY` inclusive. |

`LEN` is bounded to **64** on purpose — the firmware has ~600 B of RAM, so anything
larger is **chunked** (see *Bulk transfer*). Max decoded frame = 68 B; after COBS +
delimiter, ≤ 70 B on the wire.

### Response status

Responses carry a 1-byte `STATUS` as `BODY[0]`, then type-specific data:

| STATUS | meaning |
|--------|---------|
| `0x00` | OK |
| `0x01` | bad CRC |
| `0x02` | unknown TYPE |
| `0x03` | bad args / length |
| `0x04` | storage error (storage) |
| `0x05` | not found (macro id) |
| `0x06` | busy |
| `0x07` | unsupported (skrit-mc tier/opcode above `macro_tier`) |

A malformed or unknown request still gets a response (`TYPE|0x80`, `SEQ` echoed,
`STATUS` set) so the app never hangs waiting.

---

## Message types

| TYPE | name | request body | response body (after STATUS) |
|------|------|--------------|------------------------------|
| `0x01` | `PING` | — | `"PONG"` |
| `0x02` | `INFO` | — | `fw_ver(2)`, `caps(1)`, `n_outputs(1)`, `store_kb(1)`, `proto_ver(1)`, `n_inputs(1)`, `macro_tier(1)` |
| `0x03` | `DEVICE_NAME` | — | device name string |
| `0x04` | `REBOOT` | `mode(1)` | — (replies OK, then reboots). `mode` 0=app reset, 1=bootloader/DFU. Needs `CAP_REBOOT`. |
| `0x10` | `OUTPUT_SET` | `index(1)`, `value(1)` | — | sets relay/LED. index: 0=R1,1=R2,2=LED |
| `0x11` | `OUTPUT_GET` | — | `bitmap(1)` (bit0=R1,bit1=R2,bit2=LED) |
| `0x12` | `OUTPUT_TOGGLE` | `index(1)` | `bitmap(1)` |
| `0x13` | `OUTPUT_DESC` | `index(1)` | `index(1)`, `type(1)`, `name…` — type is the *behavior*: 0=io (digital on/off), 1=pwm, 2=rgb; what the pin drives (relay, LED, reset…) is the descriptive `name`. |
| `0x14` | `INPUT_DESC` | `index(1)` | `index(1)`, `type(1)`, `name…` (type 0=digital, 1=analog) |
| `0x15` | `INPUT_GET` | `index(1)` | `index(1)`, `value(2)` (digital 0/1, analog 0-1023) |
| `0x16` | `OUTPUT_PULSE` | `index(1)`, `ms(2)` | — | drive output on, restore after `ms` (a momentary button — reset/power lines). |
| `0x17` | `SERIAL_GET` | — | `baud(4)`, `data_bits(1)`, `parity(1)`, `stop_bits(1)` — the DATA-UART config. |
| `0x18` | `SERIAL_SET` | `baud(4)`, `data_bits(1)`, `parity(1)`, `stop_bits(1)` | — | reconfigure DATA UART (works even on a muxed link where USB line-coding isn't available). Needs `CAP_SERIAL`. |
| `0x19` | `SERIAL_SIGNAL` | `mask(1)`, `value(1)` | — | drive DATA modem/break lines. bit0=DTR, bit1=RTS, bit2=BREAK. Lets a host sequence ESP32 / AVR bootloader entry. Needs `CAP_SERIAL`. |
| `0x1A` | `OUTPUT_PWM` | `index(1)`[, `duty(2)`] | `index(1)`, `duty(2)` | with `duty` (0–1023) = set the output's PWM duty; without = read it back. Needs `CAP_PWM`; non-PWM outputs answer `0x03`. A PWM output still honors `OUTPUT_SET` (0 = duty 0, 1 = full). |
| `0x1B` | `OUTPUT_RGB` | `index(1)`[, [`pixel(1)`,] `r(1)`, `g(1)`, `b(1)`] | `index(1)`, `count(1)`, `r(1)`, `g(1)`, `b(1)` | addressable-LED color. Body **1** = read; **4** = set all pixels; **5** = set one `pixel`. Response carries the strip's pixel `count` and pixel 0's color. Only `OUTPUT_DESC` type 4 (rgb); others answer `0x03`. Still honors `OUTPUT_SET` (0 = off, 1 = white). |
| `0x20` | `MACRO_LIST` | `start(1)` | `count(1)`, then repeated `{id(1), name_len(1), name…}` until frame full; more via next `start`. |
| `0x21` | `MACRO_META` | `id(1)` | `id(1)`, `len(2)`, `name_len(1)`, `name…` |
| `0x22` | `MACRO_READ` | `id(1)`, `off(2)`, `n(1)` | `bytes…` (n ≤ 64) |
| `0x23` | `MACRO_WRITE_BEGIN` | `id(1)`, `total(2)`, `name_len(1)`, `name…` | — (allocates / truncates) |
| `0x24` | `MACRO_WRITE_DATA` | `id(1)`, `off(2)`, `bytes…` | — |
| `0x25` | `MACRO_WRITE_END` | `id(1)`, `crc16(2)` | — (commits; verifies) |
| `0x26` | `MACRO_DELETE` | `id(1)` | — |
| `0x27` | `MACRO_RUN` | `id(1)` | — | device runs the stored **skrit-mc** program through its VM (see *Macro bytecode*) |
| `0x30` | `EE_READ` | `addr(2)`, `n(1)` | `bytes…` |
| `0x31` | `EE_WRITE` | `addr(2)`, `bytes…` | — |
| `0x40` | `CFG_GET` | `key(1)` | `value…` (e.g. default DATA baud) |
| `0x41` | `CFG_SET` | `key(1)`, `value…` | — |

Multi-byte integers are **little-endian** (matches SDCC and `x86`/`arm` hosts).

`caps` bitfield (INFO): bit0=persists-macros, bit1=has-OLED, bit2=has-SPI-flash,
bit3=parity, **bit4=muxed** (one endpoint carries both channels — see *skrit-mux*),
bit5=serial-control (`SERIAL_*`), bit6=reboot (`REBOOT`), bit7=pwm (`OUTPUT_PWM`).

`macro_tier` (INFO): the highest **skrit-mc** tier the device's VM executes — `0`=none (no
on-device macros), `1`=open-loop replay (`EMIT`/`DELAY`/`SETOUT`), `2`=+closed-loop
(`EXPECT`/`WAITIO`/`WAITOK`). A device returning a short INFO body (no `macro_tier` byte)
is treated as `0`. The app refuses to save/run a program whose tier exceeds this.

---

## Bulk transfer (macros / fonts)

Because `LEN ≤ 64`, anything large is sent as a sequence the firmware can stream
straight to storage page-by-page without buffering the whole blob:

```
host → MACRO_WRITE_BEGIN(id, total, name)
host → MACRO_WRITE_DATA(id, off=0,  bytes[0..n])      ┐
host → MACRO_WRITE_DATA(id, off=n,  bytes[n..2n])     │ repeat, n ≤ 64
host → …                                             ┘
host → MACRO_WRITE_END(id, crc16)   ← device verifies & commits
```

Reads mirror it with `MACRO_READ(id, off, n)`. The stored payload is a **skrit-mc
program** (below); `MACRO_RUN(id)` runs it through the device VM (so the on-device menu
and the app share one code path).

---

## Macro bytecode (skrit-mc v1)

A macro is authored as text (`STRING`, `WAITFOR`, `IF`…) and parsed to a shared IR. The
host **compiles** that IR to a compact byte program; the device runs the program in a
tiny VM. This is what makes on-device execution *correct* — the device never sees
`STRING`/`WAITFOR` text, only resolved opcodes.

A program is a 1-byte version followed by opcodes, terminated by `END`:

```
program := mc_ver(1) , op... , 0x00
mc_ver   = 0x01
```

| op | name | operands | tier | effect |
|----|------|----------|------|--------|
| `0x00` | `END`    | — | 1 | halt (success) |
| `0x01` | `EMIT`   | `n(1)`, `bytes[n]` | 1 | write `bytes` to DATA/UART |
| `0x02` | `DELAY`  | `ms(2)` | 1 | pause |
| `0x03` | `SETOUT` | `index(1)`, `val(1)` | 1 | drive output `index` (0/1) |
| `0x04` | `SETPWM` | `index(1)`, `duty(2)` | 1 | set output `index` PWM duty (0–1023); no-op on a non-PWM output |
| `0x05` | `SETRGB` | `index(1)`, `r(1)`, `g(1)`, `b(1)` | 1 | fill output `index`'s RGB strip with a color; no-op on a non-RGB output |
| `0x10` | `EXPECT` | `timeout(2)`, `n(1)`, `bytes[n]` | 2 | match `bytes` on incoming DATA; sets outcome (match=OK, timeout=FAIL) |
| `0x11` | `WAITIO` | `index(1)`, `cmp(1)`, `val(2)`, `timeout(2)` | 2 | poll input `index` until `value cmp val`; sets outcome (met=OK, timeout=FAIL) |
| `0x12` | `WAITOK` | — | 2 | if last outcome is FAIL, halt the run with `STATUS` failed |
| `0x20` | `IF`     | `cond(1)`, `skip(2)` | 2 | *reserved v2* — if outcome ≠ `cond`, jump `skip` bytes |
| `0x21` | `ELSE`   | `skip(2)` | 2 | *reserved v2* |
| `0x22` | `ENDIF`  | — | 2 | *reserved v2* |

- Multi-byte operands are **little-endian**. `EMIT n ≤ 255` and `DELAY ms ≤ 65535`; the
  compiler splits longer runs into multiple ops.
- `cmp` byte (`WAITIO`): `0=>`, `1=<`, `2=>=`, `3=<=`, `4===`, `5=!=`.
- **Outcome flag** — one boolean, init OK. `EXPECT`/`WAITIO` set it; `WAITOK` (and the
  reserved `IF`) read it. This decouples OK/FAIL from `RUN`, so a bare `WAITFOR` timeout
  also trips it.
- **Compile-time-only IR** (no opcode): `TIMEOUT` is folded into each `EXPECT`/`WAITIO`
  `timeout` field; `$Call` is inlined; `SETOUT`/`WAITIO` names are resolved to indices
  against the connected device. `RUN`/`WAITOK`-on-`RUN`/`IF`-on-`RUN` are **app-only**
  (tier 3) and are never compiled for a device target.

### Tiers

| tier | name | opcodes | model |
|------|------|---------|-------|
| **1** | replay | `EMIT` `DELAY` `SETOUT` `END` | open-loop — output is a fixed function of time |
| **2** | interactive | + `EXPECT` `WAITIO` `WAITOK` | closed-loop — blocks/branches on a read |
| **3** | app-only | `RUN` + the `WAITOK`/`IF` that ride it | host orchestration (exit codes); never on-device |

A program's tier is the max tier of its opcodes. The device advertises the highest tier
its VM runs in `INFO.macro_tier` (`0` = no VM). A device executes an op only if its tier
≤ `macro_tier`; an over-tier op is a backstop `STATUS 0x07` (the app pre-checks via INFO).

### Running a program

The same bulk path carries bytecode. To **persist**, `MACRO_WRITE_*` a program to an id
then `MACRO_RUN(id)` (requires caps bit0, *persists-macros*). For **push-and-run** with
no storage, write to the reserved **scratch id `0xFF`** (a volatile RAM slot) and
`MACRO_RUN(0xFF)` — this works even when the device persists nothing, bounded by RAM.
A `CAP_STORE` device streams stored opcodes page-by-page, so persisted programs aren't
RAM-bound.

---

## ASCII mode (debug / on-device menu)

The same actions are reachable as plain lines, for a hand terminal and for the
on-device menu to reuse the vocabulary:

```
PING            -> PONG
ID              -> Duta v<n>
STATUS          -> R1=0 R2=0 LED=0
R1 ON|OFF|TOGGLE
R2 ON|OFF|TOGGLE
LED ON|OFF|TOGGLE
MACRO LIST      -> id: name        (one per line)
MACRO RUN <id>
HELP
```

Binary mode is preferred from the desktop app (typed, CRC-checked, supports bulk);
ASCII stays for humans.

---

## Transports

skrit is transport-independent. A device picks **one** of these; the app discovers
which from `INFO.caps` (the `muxed` bit) and how it connected.

### Dual-CDC (e.g. CH552)

Two USB-CDC interfaces: **DATA** (MI_00) is the raw console; **CMD** (MI_02) speaks the
binary/ASCII protocol above. Nothing is multiplexed — each role owns a port. This is
the cheapest path on an MCU with composite-USB silicon.

### skrit-mux — one channel, both roles

Single-channel transports — a single USB-CDC (ESP32-S3, RP2040, nRF52840 over one CDC
ACM), a **TCP** socket (WiFi bridge), or **BLE** (a NUS GATT pipe) — carry **both**
roles over one byte stream. Every packet is a COBS frame with a 1-byte **channel** tag:

```
0x00 , COBS( CHANNEL , payload... ) , 0x00

CHANNEL 0x00  DATA  -> payload is raw target-console bytes
CHANNEL 0x01  CMD   -> payload is a CMD frame:  TYPE SEQ LEN BODY[LEN] CRC8
```

COBS removes every `0x00` from the body, so the delimiters stay unambiguous and the
link resyncs after a glitch — exactly like the CMD framing, with one extra tag byte.
The **CMD payload is byte-identical** to the dual-CDC CMD frame: a device that
implements the CMD dispatch already has the hard part; mux just prepends a channel
tag and routes DATA inline. A muxed device sets the **`muxed`** capability bit in
`INFO` so the app knows to wrap/unwrap rather than open a second port.

DATA payloads should stay ≤ 240 B so COBS overhead is a single byte; larger console
bursts are split across frames (order is preserved). There is no CRC on the DATA
channel — it mirrors the lossless, unframed nature of the raw console port.

## Async events

A device MAY push **unsolicited** frames in the `0x50..0x5F` range — these have the
`SKRIT_RESP` bit **clear** and `SEQ = 0`, so a host routes them to an event sink
instead of the reply-matcher (which only ever waits on a `TYPE | 0x80` with a matching
`SEQ`). Events are advisory; a host that ignores them loses nothing.

| TYPE | name | body | meaning |
|------|------|------|---------|
| `0x50` | `EVENT_LOG` | `text…` | a device log line (e.g. on-device macro progress) |
| `0x51` | `EVENT_INPUT` | `index(1)`, `value(2)` | an input crossed an edge / threshold |

Events ride the CMD channel (or, on a muxed link, `CHANNEL=CMD`). They carry no CRC
beyond the frame's own; a host treats a malformed event as a no-op.

## Versioning

`proto_ver` starts at **1**. Additive changes — new `TYPE`s (`REBOOT`, `OUTPUT_PULSE`,
`SERIAL_*`, the `0x50` events), new `caps` bits, and the *skrit-mux* channel tag — keep
the same version: an older app simply doesn't send or decode them and a newer device
still answers the v1 core. Breaking changes bump it. The app reads `INFO` on connect
and refuses mismatched major versions.
