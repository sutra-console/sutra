//! skrit CMD-port wire protocol. See protocol/PROTOCOL.md
//!
//! Binary frame on the wire:  COBS( TYPE SEQ LEN BODY[LEN] CRC8 ) 0x00
//! Multi-byte integers are little-endian. BODY is capped at 64 bytes.

#[allow(dead_code)] // reported via INFO; not referenced internally yet
pub const PROTO_VER: u8 = 1;
pub const MAX_BODY: usize = 64;
pub const RESP_FLAG: u8 = 0x80;

// Message types (request side). Response = request | RESP_FLAG.
#[allow(dead_code)] // full protocol vocabulary; not all used by the app yet
pub mod msg {
    pub const PING: u8 = 0x01;
    pub const INFO: u8 = 0x02;
    pub const DEVICE_NAME: u8 = 0x03;
    pub const REBOOT: u8 = 0x04;
    pub const AUTH: u8 = 0x05;
    pub const AUTH_SET: u8 = 0x06;
    pub const DATA_DESC: u8 = 0x07;
    pub const OUTPUT_SET: u8 = 0x10;
    pub const OUTPUT_GET: u8 = 0x11;
    pub const OUTPUT_TOGGLE: u8 = 0x12;
    pub const OUTPUT_DESC: u8 = 0x13;
    pub const INPUT_DESC: u8 = 0x14;
    pub const INPUT_GET: u8 = 0x15;
    pub const OUTPUT_PULSE: u8 = 0x16;
    pub const SERIAL_GET: u8 = 0x17;
    pub const SERIAL_SET: u8 = 0x18;
    pub const SERIAL_SIGNAL: u8 = 0x19;
    pub const OUTPUT_PWM: u8 = 0x1A;
    pub const OUTPUT_RGB: u8 = 0x1B;
    pub const PWM_CONFIG: u8 = 0x1C;
    pub const PIN_CAPS: u8 = 0x1D;
    pub const CONFIG_GET: u8 = 0x1E;
    pub const CONFIG_SET: u8 = 0x1F;
    pub const MACRO_LIST: u8 = 0x20;
    pub const MACRO_META: u8 = 0x21;
    pub const MACRO_READ: u8 = 0x22;
    pub const MACRO_WRITE_BEGIN: u8 = 0x23;
    pub const MACRO_WRITE_DATA: u8 = 0x24;
    pub const MACRO_WRITE_END: u8 = 0x25;
    pub const MACRO_DELETE: u8 = 0x26;
    pub const MACRO_RUN: u8 = 0x27;
    pub const EE_READ: u8 = 0x30;
    pub const EE_WRITE: u8 = 0x31;
    pub const CFG_GET: u8 = 0x40;
    pub const CFG_SET: u8 = 0x41;
    pub const I2C_SCAN: u8 = 0x60;
    pub const I2C_XFER: u8 = 0x61;
    // Async device->host events (0x50..0x5F): RESP bit clear, SEQ=0.
    pub const EVENT_LOG: u8 = 0x50;
    pub const EVENT_INPUT: u8 = 0x51;
    pub const EVENT_LO: u8 = 0x50;
    pub const EVENT_HI: u8 = 0x5F;
}

/// True for a TYPE in the async-event range (a device-pushed frame, not a reply).
pub fn is_event(typ: u8) -> bool {
    typ & RESP_FLAG == 0 && (msg::EVENT_LO..=msg::EVENT_HI).contains(&typ)
}

// INFO capability bits (INFO body[3]).
#[allow(dead_code)]
pub mod cap {
    pub const STORE: u8 = 0x01;
    pub const OLED: u8 = 0x02;
    pub const SPI: u8 = 0x04;
    pub const PARITY: u8 = 0x08;
    pub const MUX: u8 = 0x10; // single endpoint carries both channels (skrit-mux)
    pub const SERIAL: u8 = 0x20; // honors SERIAL_GET/SET/SIGNAL
    pub const REBOOT: u8 = 0x40; // honors REBOOT
    pub const PWM: u8 = 0x80; // honors OUTPUT_PWM on at least one output
}

// SERIAL_SIGNAL line bits (mask/value).
#[allow(dead_code)]
pub mod sig {
    pub const DTR: u8 = 0x01;
    pub const RTS: u8 = 0x02;
    pub const BREAK: u8 = 0x04;
}

// SERIAL_GET/SET parity byte.
#[allow(dead_code)]
pub mod parity {
    pub const NONE: u8 = 0;
    pub const ODD: u8 = 1;
    pub const EVEN: u8 = 2;
}

// REBOOT modes.
#[allow(dead_code)]
pub mod reboot {
    pub const APP: u8 = 0;
    pub const BOOTLOADER: u8 = 1;
}

// skrit-mux channel tags (see protocol/PROTOCOL.md "Transports").
#[allow(dead_code)]
pub mod mux {
    pub const DATA: u8 = 0x00;
    pub const CMD: u8 = 0x01;
}

// Response STATUS codes (BODY[0] of a response).
#[allow(dead_code)] // full protocol vocabulary; not all used by the app yet
pub mod status {
    pub const OK: u8 = 0x00;
    pub const BAD_CRC: u8 = 0x01;
    pub const UNKNOWN_TYPE: u8 = 0x02;
    pub const BAD_ARGS: u8 = 0x03;
    pub const STORAGE_ERR: u8 = 0x04;
    pub const NOT_FOUND: u8 = 0x05;
    pub const BUSY: u8 = 0x06;
    pub const UNSUPPORTED: u8 = 0x07;
    pub const UNAUTH: u8 = 0x08;
}

// INFO flags byte (trailing). Network transports gate behind AUTH.
#[allow(dead_code)]
pub mod flag {
    pub const AUTH_REQUIRED: u8 = 0x01;
    pub const DEFAULT_CRED: u8 = 0x02;
    pub const PROVISION: u8 = 0x04; // accepts runtime IO provisioning (PIN_CAPS/CONFIG_*)
}

// Pin-capability bits (PIN_CAPS `caps` byte) + provisioning sentinels.
#[allow(dead_code)]
pub mod pincap {
    pub const DIGITAL: u8 = 0x01;
    pub const ADC: u8 = 0x02;
    pub const PWM: u8 = 0x04;
    pub const DAC: u8 = 0x08;
    pub const I2C: u8 = 0x10;
    pub const SPI: u8 = 0x20;
    pub const TOUCH: u8 = 0x40;
    pub const WARN: u8 = 1; // PIN_CAPS `warn` byte: offer but show the reason name
    pub const NO_BUS: u8 = 0xFF; // PIN_CAPS `bus`: not a bus pin / matrix-routable
    pub const CONFIG_RESET: u8 = 0xFF; // CONFIG_SET `n`: revert to the compiled default
}

/// CRC-8/ATM (poly 0x07, init 0x00) over the given bytes.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0x00;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

/// COBS-encode `data` (does NOT append the 0x00 delimiter).
pub fn cobs_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 254 + 2);
    let mut code_idx = 0usize;
    out.push(0); // placeholder for first code byte
    let mut code: u8 = 1;
    for &b in data {
        if b == 0 {
            out[code_idx] = code;
            code_idx = out.len();
            out.push(0);
            code = 1;
        } else {
            out.push(b);
            code += 1;
            if code == 0xFF {
                out[code_idx] = code;
                code_idx = out.len();
                out.push(0);
                code = 1;
            }
        }
    }
    out[code_idx] = code;
    out
}

/// COBS-decode a frame payload (without the trailing 0x00 delimiter).
pub fn cobs_decode(data: &[u8]) -> Result<Vec<u8>, FrameError> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0usize;
    while i < data.len() {
        let code = data[i] as usize;
        if code == 0 {
            return Err(FrameError::Cobs);
        }
        i += 1;
        for _ in 1..code {
            match data.get(i) {
                Some(&b) => out.push(b),
                None => return Err(FrameError::Cobs),
            }
            i += 1;
        }
        if code < 0xFF && i < data.len() {
            out.push(0);
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    Cobs,
    TooShort,
    BadCrc,
    BodyTooLong,
    LenMismatch,
}

/// A decoded protocol frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub typ: u8,
    pub seq: u8,
    pub body: Vec<u8>,
}

impl Frame {
    pub fn new(typ: u8, seq: u8, body: Vec<u8>) -> Self {
        Frame { typ, seq, body }
    }

    pub fn is_response(&self) -> bool {
        self.typ & RESP_FLAG != 0
    }

    /// Response STATUS byte, if this is a response with a body.
    pub fn status(&self) -> Option<u8> {
        if self.is_response() {
            self.body.first().copied()
        } else {
            None
        }
    }

    /// The undelimited frame bytes: TYPE SEQ LEN BODY CRC8 (no COBS, no 0x00).
    /// This is the CMD payload on both a dual link and a skrit-mux CMD channel.
    pub fn to_raw(&self) -> Result<Vec<u8>, FrameError> {
        if self.body.len() > MAX_BODY {
            return Err(FrameError::BodyTooLong);
        }
        let mut raw = Vec::with_capacity(self.body.len() + 4);
        raw.push(self.typ);
        raw.push(self.seq);
        raw.push(self.body.len() as u8);
        raw.extend_from_slice(&self.body);
        raw.push(crc8(&raw));
        Ok(raw)
    }

    /// Serialize to wire bytes: COBS(header+body+crc) + 0x00 delimiter.
    pub fn to_wire(&self) -> Result<Vec<u8>, FrameError> {
        let raw = self.to_raw()?;
        // Wire = 0x00 (SOF) + COBS(raw) + 0x00 (EOF). The leading 0x00 lets the
        // 8051 unambiguously enter binary mode (vs an ASCII line). Consecutive
        // frames just share delimiters; empty segments are ignored by readers.
        let mut wire = Vec::with_capacity(raw.len() + 4);
        wire.push(0x00);
        wire.extend(cobs_encode(&raw));
        wire.push(0x00);
        Ok(wire)
    }

    /// Serialize for a skrit-mux link: the CMD frame on channel `mux::CMD`.
    pub fn to_mux_wire(&self) -> Result<Vec<u8>, FrameError> {
        Ok(mux_wrap(mux::CMD, &self.to_raw()?))
    }

    /// Parse from a single COBS-decoded payload (header+body+crc, no delimiter).
    pub fn from_raw(raw: &[u8]) -> Result<Frame, FrameError> {
        if raw.len() < 4 {
            return Err(FrameError::TooShort);
        }
        let (frame_part, crc) = raw.split_at(raw.len() - 1);
        if crc8(frame_part) != crc[0] {
            return Err(FrameError::BadCrc);
        }
        let typ = raw[0];
        let seq = raw[1];
        let len = raw[2] as usize;
        let body = &raw[3..raw.len() - 1];
        if body.len() != len {
            return Err(FrameError::LenMismatch);
        }
        Ok(Frame { typ, seq, body: body.to_vec() })
    }

    /// Parse a complete on-wire frame (COBS bytes, delimiter already stripped).
    pub fn from_wire(cobs_bytes: &[u8]) -> Result<Frame, FrameError> {
        let raw = cobs_decode(cobs_bytes)?;
        Frame::from_raw(&raw)
    }
}

/// Incremental reassembler: feed bytes from the CMD port, get back complete frames
/// split on the 0x00 delimiter.
#[derive(Default)]
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    pub fn new() -> Self {
        FrameReader { buf: Vec::new() }
    }

    /// Feed received bytes; returns every complete frame that became available.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<Frame, FrameError>> {
        let mut frames = Vec::new();
        for &b in bytes {
            if b == 0x00 {
                if !self.buf.is_empty() {
                    frames.push(Frame::from_wire(&self.buf));
                    self.buf.clear();
                }
            } else {
                self.buf.push(b);
            }
        }
        frames
    }
}

/// Wrap a payload onto a skrit-mux channel: `0x00 + COBS(channel ++ payload) + 0x00`.
/// COBS removes interior zeros, so the delimiters stay unambiguous.
pub fn mux_wrap(channel: u8, payload: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(payload.len() + 1);
    raw.push(channel);
    raw.extend_from_slice(payload);
    let mut wire = Vec::with_capacity(raw.len() + 4);
    wire.push(0x00);
    wire.extend(cobs_encode(&raw));
    wire.push(0x00);
    wire
}

/// Incremental skrit-mux demuxer: feed bytes from a single muxed link, get back
/// `(channel, payload)` for every complete frame (channel 0 = DATA, 1 = CMD).
#[derive(Default)]
pub struct MuxReader {
    buf: Vec<u8>,
}

impl MuxReader {
    pub fn new() -> Self {
        MuxReader { buf: Vec::new() }
    }

    /// Feed received bytes; returns each complete `(channel, payload)` that became
    /// available. A malformed (un-decodable) frame is dropped silently: the link
    /// resyncs on the next 0x00 delimiter.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut out = Vec::new();
        for &b in bytes {
            if b == 0x00 {
                if !self.buf.is_empty() {
                    if let Ok(dec) = cobs_decode(&self.buf) {
                        if !dec.is_empty() {
                            out.push((dec[0], dec[1..].to_vec()));
                        }
                    }
                    self.buf.clear();
                }
            } else {
                self.buf.push(b);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_known() {
        // CRC-8/ATM of "123456789" is 0xF4
        assert_eq!(crc8(b"123456789"), 0xF4);
    }

    #[test]
    fn cobs_roundtrip() {
        for case in [
            vec![],
            vec![1, 2, 3],
            vec![0],
            vec![0, 0, 0],
            vec![1, 0, 2, 0, 3],
            (0u8..=255).collect::<Vec<_>>(),
            vec![5u8; 600],
        ] {
            let enc = cobs_encode(&case);
            assert!(!enc.contains(&0), "encoded must contain no zero");
            let dec = cobs_decode(&enc).unwrap();
            assert_eq!(dec, case);
        }
    }

    #[test]
    fn frame_roundtrip() {
        let f = Frame::new(msg::OUTPUT_SET, 7, vec![0, 1]);
        let wire = f.to_wire().unwrap();
        assert_eq!(wire[0], 0x00, "SOF delimiter present");
        assert_eq!(*wire.last().unwrap(), 0x00, "EOF delimiter present");
        let mut r = FrameReader::new();
        let frames = r.push(&wire);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].clone().unwrap(), f);
    }

    #[test]
    fn frame_reader_splits() {
        let a = Frame::new(msg::PING, 1, vec![]).to_wire().unwrap();
        let b = Frame::new(msg::OUTPUT_GET, 2, vec![]).to_wire().unwrap();
        let mut stream = a.clone();
        stream.extend_from_slice(&b);
        let mut r = FrameReader::new();
        // feed in two awkward chunks to exercise reassembly
        let mut got = r.push(&stream[..3]);
        got.extend(r.push(&stream[3..]));
        let frames: Vec<_> = got.into_iter().map(|f| f.unwrap()).collect();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].typ, msg::PING);
        assert_eq!(frames[1].typ, msg::OUTPUT_GET);
    }

    #[test]
    fn mux_cmd_roundtrip() {
        // A CMD frame wrapped on the mux link demuxes back to the same frame.
        let f = Frame::new(msg::OUTPUT_SET, 9, vec![1, 1]);
        let wire = f.to_mux_wire().unwrap();
        assert!(!wire[1..wire.len() - 1].contains(&0), "no interior zeros");
        let mut r = MuxReader::new();
        let got = r.push(&wire);
        assert_eq!(got.len(), 1);
        let (ch, payload) = &got[0];
        assert_eq!(*ch, mux::CMD);
        assert_eq!(Frame::from_raw(payload).unwrap(), f);
    }

    #[test]
    fn mux_data_roundtrip() {
        // Raw console bytes (incl. a zero) survive the DATA channel intact.
        let console = b"login: \x00ready>";
        let wire = mux_wrap(mux::DATA, console);
        let mut r = MuxReader::new();
        let got = r.push(&wire);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, mux::DATA);
        assert_eq!(got[0].1, console);
    }

    #[test]
    fn mux_interleaved_chunks() {
        // DATA and CMD frames interleave on one stream, fed in awkward splits.
        let mut stream = mux_wrap(mux::DATA, b"hello");
        stream.extend(Frame::new(msg::PING, 1, vec![]).to_mux_wire().unwrap());
        stream.extend(mux_wrap(mux::DATA, b"world"));
        let mut r = MuxReader::new();
        let mut got = r.push(&stream[..4]);
        got.extend(r.push(&stream[4..]));
        let chans: Vec<u8> = got.iter().map(|(c, _)| *c).collect();
        assert_eq!(chans, vec![mux::DATA, mux::CMD, mux::DATA]);
        assert_eq!(got[0].1, b"hello");
        assert_eq!(got[2].1, b"world");
    }

    #[test]
    fn event_classified() {
        assert!(is_event(msg::EVENT_LOG));
        assert!(is_event(msg::EVENT_INPUT));
        assert!(!is_event(msg::PING));
        assert!(!is_event(msg::INFO | RESP_FLAG));
    }

    #[test]
    fn bad_crc_detected() {
        let mut wire = Frame::new(msg::PING, 1, vec![1, 2, 3]).to_wire().unwrap();
        // corrupt a COBS byte (between the SOF and EOF delimiters)
        wire[2] ^= 0xFF;
        let mut r = FrameReader::new();
        let frames = r.push(&wire);
        assert_eq!(frames.len(), 1);
        assert!(matches!(
            frames[0],
            Err(FrameError::BadCrc) | Err(FrameError::Cobs) | Err(FrameError::LenMismatch)
        ));
    }
}
