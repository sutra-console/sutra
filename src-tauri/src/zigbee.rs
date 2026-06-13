//! Zigbee NWK/APS security — host-side AES-128-CCM* (network-model phase B).
//!
//! The firmware stays a dumb radio; ALL crypto lives here. This is the gate that
//! unlocks active discovery (ZDP interview) and control: with the network key we
//! can build + authenticate-encrypt outbound frames and decrypt inbound ones.
//!
//! Zigbee uses **CCM*** (CCM with the M=0 case allowed) over AES-128. For the
//! usual security level 5 (ENC-MIC-32) it's plain CCM with a 4-byte MIC, L=2, and
//! a 13-byte nonce. The nonce is:
//!     source EUI-64 (8, as on the wire) · frame counter (4, LE) · security control (1)
//! The AAD (authenticated, not encrypted) is the cleartext header — the NWK header
//! plus the auxiliary security header up to the encrypted payload.
//!
//! GOTCHA (not yet exercised here, but the reason `sec_control` is a parameter):
//! over the air the aux-header security-control byte often carries level 0, but the
//! nonce + AAD must use the *actual* level. Callers normalize before calling.

use aes::Aes128;
use ccm::{
    aead::{generic_array::GenericArray, AeadInPlace, KeyInit},
    consts::{U13, U4},
    Ccm,
};

/// Zigbee security level 5 (ENC-MIC-32): payload encrypted + 4-byte MIC.
/// `Ccm<cipher, tag, nonce>` → AES-128, 4-byte MIC, 13-byte nonce (so L = 15-13 = 2).
type ZbCcm = Ccm<Aes128, U4, U13>;

/// 13-byte CCM* nonce: source EUI-64 (8) · frame counter (4 LE) · security control (1).
fn nonce(src_eui64: &[u8; 8], frame_counter: u32, sec_control: u8) -> [u8; 13] {
    let mut n = [0u8; 13];
    n[..8].copy_from_slice(src_eui64);
    n[8..12].copy_from_slice(&frame_counter.to_le_bytes());
    n[12] = sec_control;
    n
}

/// Encrypt + authenticate `payload` in place (security level 5). `aad` is the
/// cleartext header authenticated alongside it. Returns the 4-byte MIC to append.
pub fn ccm_encrypt(
    key: &[u8; 16],
    src_eui64: &[u8; 8],
    frame_counter: u32,
    sec_control: u8,
    aad: &[u8],
    payload: &mut [u8],
) -> Result<[u8; 4], String> {
    let cipher = ZbCcm::new(GenericArray::from_slice(key));
    let n = nonce(src_eui64, frame_counter, sec_control);
    let tag = cipher
        .encrypt_in_place_detached(GenericArray::from_slice(&n), aad, payload)
        .map_err(|_| "ccm* encrypt failed".to_string())?;
    let mut mic = [0u8; 4];
    mic.copy_from_slice(&tag);
    Ok(mic)
}

/// Verify the 4-byte `mic` and decrypt `ciphertext` in place. Err on a bad MIC.
pub fn ccm_decrypt(
    key: &[u8; 16],
    src_eui64: &[u8; 8],
    frame_counter: u32,
    sec_control: u8,
    aad: &[u8],
    ciphertext: &mut [u8],
    mic: &[u8; 4],
) -> Result<(), String> {
    let cipher = ZbCcm::new(GenericArray::from_slice(key));
    let n = nonce(src_eui64, frame_counter, sec_control);
    cipher
        .decrypt_in_place_detached(
            GenericArray::from_slice(&n),
            aad,
            ciphertext,
            GenericArray::from_slice(mic),
        )
        .map_err(|_| "ccm* MIC verify / decrypt failed".to_string())
}

// ---- NWK-layer security wrapper -------------------------------------------
// The auxiliary security header (network key + extended nonce):
//   sec_control(1) · frame_counter(4 LE) · source EUI-64(8) · key seq(1)   = 14 bytes
// sec_control = level(bits0-2) | key_id<<3 | ext_nonce<<5. Network key + ext
// nonce + level L  ->  L | 0x28.
pub const SEC_LEVEL_ENC_MIC32: u8 = 5; // the usual Zigbee NWK level
const KEYID_NWK: u8 = 1;

fn sec_control(level: u8) -> u8 {
    (level & 0x07) | (KEYID_NWK << 3) | (1 << 5) // ext-nonce always present here
}

/// Wrap a cleartext NWK header + payload into a secured NWK frame:
///   header · aux-sec-header · ciphertext · MIC(4).
/// `header` is the cleartext NWK header (authenticated, not encrypted).
pub fn secure_nwk(
    key: &[u8; 16],
    header: &[u8],
    payload: &[u8],
    src_eui64: &[u8; 8],
    frame_counter: u32,
    key_seq: u8,
    level: u8,
) -> Result<Vec<u8>, String> {
    let sc = sec_control(level);
    let mut aux = Vec::with_capacity(14);
    aux.push(sc);
    aux.extend_from_slice(&frame_counter.to_le_bytes());
    aux.extend_from_slice(src_eui64);
    aux.push(key_seq);

    let mut aad = Vec::with_capacity(header.len() + aux.len());
    aad.extend_from_slice(header);
    aad.extend_from_slice(&aux);

    let mut ct = payload.to_vec();
    let mic = ccm_encrypt(key, src_eui64, frame_counter, sc, &aad, &mut ct)?;

    let mut out = aad; // header · aux …
    out.extend_from_slice(&ct);
    out.extend_from_slice(&mic);
    Ok(out)
}

/// Inverse of `secure_nwk`. `header_len` is the cleartext NWK header length (a
/// NWK-header parser supplies it for real frames; the round-trip test passes the
/// known length). Returns the decrypted NWK payload, or Err on a bad MIC.
pub fn unsecure_nwk(key: &[u8; 16], frame: &[u8], header_len: usize) -> Result<Vec<u8>, String> {
    if frame.len() < header_len + 14 + 4 {
        return Err("frame too short for aux header + MIC".into());
    }
    let aux_off = header_len;
    let sc = frame[aux_off];
    let fc = u32::from_le_bytes(frame[aux_off + 1..aux_off + 5].try_into().unwrap());
    let mut eui = [0u8; 8];
    eui.copy_from_slice(&frame[aux_off + 5..aux_off + 13]);
    // key_seq at aux_off+13; aux header is 14 bytes total here (ext nonce + nwk key).
    let aux_end = aux_off + 14;
    let mic_off = frame.len() - 4;
    let aad = &frame[..aux_end];
    let mut ct = frame[aux_end..mic_off].to_vec();
    let mut mic = [0u8; 4];
    mic.copy_from_slice(&frame[mic_off..]);
    // GOTCHA: over the air some stacks zero the level bits in the wire sec_control;
    // the nonce + AAD must use the ACTUAL level. We pass `sc` as-is, which is correct
    // when the wire carries the real level — validate against tshark on a live frame.
    ccm_decrypt(key, &eui, fc, sc, aad, &mut ct, &mic)?;
    Ok(ct)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ];
    const EUI: [u8; 8] = [0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11];

    #[test]
    fn ccm_roundtrip() {
        let aad = [0x09, 0x12, 0xfd, 0xff, 0x00, 0x00]; // a plausible NWK header
        let plain: Vec<u8> = (0..40).collect();
        let mut buf = plain.clone();
        let mic = ccm_encrypt(&KEY, &EUI, 7, 0x28, &aad, &mut buf).unwrap();
        assert_ne!(buf, plain, "payload was actually encrypted");
        // decrypt the ciphertext back
        ccm_decrypt(&KEY, &EUI, 7, 0x28, &aad, &mut buf, &mic).unwrap();
        assert_eq!(buf, plain, "decrypt restores the plaintext");
    }

    #[test]
    fn ccm_rejects_tamper() {
        let aad = [0x09, 0x12, 0xfd, 0xff, 0x00, 0x00];
        let mut buf: Vec<u8> = (0..16).collect();
        let mic = ccm_encrypt(&KEY, &EUI, 7, 0x28, &aad, &mut buf).unwrap();
        // flip one ciphertext byte -> MIC must fail
        let mut bad = buf.clone();
        bad[0] ^= 0x01;
        assert!(ccm_decrypt(&KEY, &EUI, 7, 0x28, &aad, &mut bad, &mic).is_err());
        // wrong frame counter (nonce) -> MIC must fail
        let mut ok = buf.clone();
        assert!(ccm_decrypt(&KEY, &EUI, 8, 0x28, &aad, &mut ok, &mic).is_err());
        // tampered AAD -> MIC must fail
        let mut ok2 = buf.clone();
        let bad_aad = [0x09, 0x12, 0xfd, 0xff, 0x00, 0x01];
        assert!(ccm_decrypt(&KEY, &EUI, 7, 0x28, &bad_aad, &mut ok2, &mic).is_err());
    }

    #[test]
    fn nwk_secure_roundtrip() {
        // A plausible cleartext NWK header (frame control + dst/src short + radius + seq).
        let header = [0x48, 0x02, 0x00, 0x00, 0x34, 0x12, 0x1e, 0x05];
        let payload: Vec<u8> = (0..30).collect();
        let frame = secure_nwk(
            &KEY,
            &header,
            &payload,
            &EUI,
            0x12345678,
            0,
            SEC_LEVEL_ENC_MIC32,
        )
        .unwrap();
        // header · aux(14) · ct(30) · mic(4)
        assert_eq!(frame.len(), header.len() + 14 + payload.len() + 4);
        assert_eq!(&frame[..header.len()], &header, "header is cleartext");
        assert_ne!(
            &frame[header.len() + 14..header.len() + 14 + payload.len()],
            &payload[..],
            "payload is encrypted on the wire"
        );
        let back = unsecure_nwk(&KEY, &frame, header.len()).unwrap();
        assert_eq!(back, payload, "unsecure recovers the NWK payload");

        // a flipped ciphertext byte must fail the MIC
        let mut bad = frame.clone();
        let ct0 = header.len() + 14;
        bad[ct0] ^= 0x01;
        assert!(unsecure_nwk(&KEY, &bad, header.len()).is_err());
    }
}
