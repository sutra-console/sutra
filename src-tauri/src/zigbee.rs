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
}
