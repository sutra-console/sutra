# sutra CMD-port wire protocol

The device exposes **two USB-CDC interfaces**:

| Interface | Role | Framing |
|-----------|------|---------|
| **DATA** (MI_00) | transparent USB↔UART bridge — the target console | **raw bytes**, no framing |
| **CMD** (MI_02) | control / snippets / config | this protocol |

The **DATA** port is never framed — it carries the target's serial console verbatim
and is what the terminal widget renders. Everything below is the **CMD** port only.

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
| `0x04` | storage error (EEPROM) |
| `0x05` | not found (snippet id) |
| `0x06` | busy |

A malformed or unknown request still gets a response (`TYPE|0x80`, `SEQ` echoed,
`STATUS` set) so the app never hangs waiting.

---

## Message types

| TYPE | name | request body | response body (after STATUS) |
|------|------|--------------|------------------------------|
| `0x01` | `PING` | — | `"PONG"` |
| `0x02` | `INFO` | — | `fw_ver(2)`, `caps(1)`, `n_outputs(1)`, `eeprom_kb(1)`, `proto_ver(1)`, `n_inputs(1)` |
| `0x03` | `DEVICE_NAME` | — | device name string |
| `0x10` | `OUTPUT_SET` | `index(1)`, `value(1)` | — | sets relay/LED. index: 0=R1,1=R2,2=LED |
| `0x11` | `OUTPUT_GET` | — | `bitmap(1)` (bit0=R1,bit1=R2,bit2=LED) |
| `0x12` | `OUTPUT_TOGGLE` | `index(1)` | `bitmap(1)` |
| `0x13` | `OUTPUT_DESC` | `index(1)` | `index(1)`, `type(1)`, `name…` (type 0=relay, 1=led) |
| `0x14` | `INPUT_DESC` | `index(1)` | `index(1)`, `type(1)`, `name…` (type 0=digital, 1=analog) |
| `0x15` | `INPUT_GET` | `index(1)` | `index(1)`, `value(2)` (digital 0/1, analog 0-1023) |
| `0x20` | `SNIP_LIST` | `start(1)` | `count(1)`, then repeated `{id(1), name_len(1), name…}` until frame full; more via next `start`. |
| `0x21` | `SNIP_META` | `id(1)` | `id(1)`, `len(2)`, `name_len(1)`, `name…` |
| `0x22` | `SNIP_READ` | `id(1)`, `off(2)`, `n(1)` | `bytes…` (n ≤ 64) |
| `0x23` | `SNIP_WRITE_BEGIN` | `id(1)`, `total(2)`, `name_len(1)`, `name…` | — (allocates / truncates) |
| `0x24` | `SNIP_WRITE_DATA` | `id(1)`, `off(2)`, `bytes…` | — |
| `0x25` | `SNIP_WRITE_END` | `id(1)`, `crc16(2)` | — (commits; verifies) |
| `0x26` | `SNIP_DELETE` | `id(1)` | — |
| `0x27` | `SNIP_RUN` | `id(1)` | — | device streams the snippet out the **DATA/UART** |
| `0x30` | `EE_READ` | `addr(2)`, `n(1)` | `bytes…` |
| `0x31` | `EE_WRITE` | `addr(2)`, `bytes…` | — |
| `0x40` | `CFG_GET` | `key(1)` | `value…` (e.g. default DATA baud) |
| `0x41` | `CFG_SET` | `key(1)`, `value…` | — |

Multi-byte integers are **little-endian** (matches SDCC and `x86`/`arm` hosts).

`caps` bitfield (INFO): bit0=has-EEPROM, bit1=has-OLED, bit2=has-SPI-flash, bit3=parity.

---

## Bulk transfer (snippets / fonts)

Because `LEN ≤ 64`, anything large is sent as a sequence the firmware can stream
straight to EEPROM page-by-page without buffering the whole blob:

```
host → SNIP_WRITE_BEGIN(id, total, name)
host → SNIP_WRITE_DATA(id, off=0,  bytes[0..n])      ┐
host → SNIP_WRITE_DATA(id, off=n,  bytes[n..2n])     │ repeat, n ≤ 64
host → …                                             ┘
host → SNIP_WRITE_END(id, crc16)   ← device verifies & commits
```

Reads mirror it with `SNIP_READ(id, off, n)`. `SNIP_RUN(id)` tells the device to
replay the snippet out the target UART itself (so the on-device menu and the app share
one code path).

---

## ASCII mode (debug / on-device menu)

The same actions are reachable as plain lines, for a hand terminal and for the
on-device menu to reuse the vocabulary:

```
PING            -> PONG
ID              -> sutra v<n>
STATUS          -> R1=0 R2=0 LED=0
R1 ON|OFF|TOGGLE
R2 ON|OFF|TOGGLE
LED ON|OFF|TOGGLE
SNIP LIST       -> id: name        (one per line)
SNIP RUN <id>
HELP
```

Binary mode is preferred from the desktop app (typed, CRC-checked, supports bulk);
ASCII stays for humans.

---

## Versioning

`proto_ver` starts at **1**. Additive changes (new TYPEs) keep the same version;
breaking changes bump it. The app reads `INFO` on connect and refuses mismatched
major versions.
