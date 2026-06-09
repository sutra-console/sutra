//! skrit CMD-port wire protocol — see protocol/PROTOCOL.md
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
    pub const OUTPUT_SET: u8 = 0x10;
    pub const OUTPUT_GET: u8 = 0x11;
    pub const OUTPUT_TOGGLE: u8 = 0x12;
    pub const OUTPUT_DESC: u8 = 0x13;
    pub const INPUT_DESC: u8 = 0x14;
    pub const INPUT_GET: u8 = 0x15;
    pub const SNIP_LIST: u8 = 0x20;
    pub const SNIP_META: u8 = 0x21;
    pub const SNIP_READ: u8 = 0x22;
    pub const SNIP_WRITE_BEGIN: u8 = 0x23;
    pub const SNIP_WRITE_DATA: u8 = 0x24;
    pub const SNIP_WRITE_END: u8 = 0x25;
    pub const SNIP_DELETE: u8 = 0x26;
    pub const SNIP_RUN: u8 = 0x27;
    pub const EE_READ: u8 = 0x30;
    pub const EE_WRITE: u8 = 0x31;
    pub const CFG_GET: u8 = 0x40;
    pub const CFG_SET: u8 = 0x41;
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

    /// Serialize to wire bytes: COBS(header+body+crc) + 0x00 delimiter.
    pub fn to_wire(&self) -> Result<Vec<u8>, FrameError> {
        if self.body.len() > MAX_BODY {
            return Err(FrameError::BodyTooLong);
        }
        let mut raw = Vec::with_capacity(self.body.len() + 4);
        raw.push(self.typ);
        raw.push(self.seq);
        raw.push(self.body.len() as u8);
        raw.extend_from_slice(&self.body);
        raw.push(crc8(&raw));
        // Wire = 0x00 (SOF) + COBS(raw) + 0x00 (EOF). The leading 0x00 lets the
        // 8051 unambiguously enter binary mode (vs an ASCII line). Consecutive
        // frames just share delimiters; empty segments are ignored by readers.
        let mut wire = Vec::with_capacity(raw.len() + 4);
        wire.push(0x00);
        wire.extend(cobs_encode(&raw));
        wire.push(0x00);
        Ok(wire)
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
