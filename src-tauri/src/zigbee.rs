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

/// The level-override (validated against a real Silicon Labs frame): Zigbee
/// transmits the aux security-control byte with the level bits ZEROED, but both
/// the CCM* nonce and the AAD must use the network's *actual* security level.
/// So: write `sec_control(0)` on the wire, compute with `sec_control(level)`.
fn sc_with_level(wire_sc: u8, level: u8) -> u8 {
    (wire_sc & !0x07) | (level & 0x07)
}

/// Wrap a cleartext NWK header + payload into a secured NWK frame:
///   header · aux-sec-header · ciphertext · MIC(4).
/// `header` is the cleartext NWK header (authenticated, not encrypted). The
/// transmitted aux sec-control carries level 0 (Zigbee convention); the crypto
/// uses the real `level`.
pub fn secure_nwk(
    key: &[u8; 16],
    header: &[u8],
    payload: &[u8],
    src_eui64: &[u8; 8],
    frame_counter: u32,
    key_seq: u8,
    level: u8,
) -> Result<Vec<u8>, String> {
    let sc_wire = sec_control(0); // level bits zeroed for transmission
    let sc_real = sec_control(level); // real level for nonce + AAD
    let mut aux = Vec::with_capacity(14);
    aux.push(sc_wire);
    aux.extend_from_slice(&frame_counter.to_le_bytes());
    aux.extend_from_slice(src_eui64);
    aux.push(key_seq);

    // AAD = header · aux, but with the real-level sec-control byte.
    let mut aad = Vec::with_capacity(header.len() + aux.len());
    aad.extend_from_slice(header);
    aad.extend_from_slice(&aux);
    aad[header.len()] = sc_real;

    let mut ct = payload.to_vec();
    let mic = ccm_encrypt(key, src_eui64, frame_counter, sc_real, &aad, &mut ct)?;

    let mut out = Vec::with_capacity(header.len() + aux.len() + ct.len() + 4);
    out.extend_from_slice(header);
    out.extend_from_slice(&aux); // wire bytes: sc_wire (level 0)
    out.extend_from_slice(&ct);
    out.extend_from_slice(&mic);
    Ok(out)
}

/// Inverse of `secure_nwk`. `header_len` is the cleartext NWK header length (a
/// NWK-header parser supplies it for real frames; tests pass the known length).
/// `level` is the network's configured security level (override the wire byte,
/// which carries 0). Returns the decrypted NWK payload, or Err on a bad MIC.
pub fn unsecure_nwk(
    key: &[u8; 16],
    frame: &[u8],
    header_len: usize,
    level: u8,
) -> Result<Vec<u8>, String> {
    if frame.len() < header_len + 14 + 4 {
        return Err("frame too short for aux header + MIC".into());
    }
    let aux_off = header_len;
    let sc = sc_with_level(frame[aux_off], level); // override wire level -> real
    let fc = u32::from_le_bytes(frame[aux_off + 1..aux_off + 5].try_into().unwrap());
    let mut eui = [0u8; 8];
    eui.copy_from_slice(&frame[aux_off + 5..aux_off + 13]);
    // key_seq at aux_off+13; aux header is 14 bytes total here (ext nonce + nwk key).
    let aux_end = aux_off + 14;
    let mic_off = frame.len() - 4;
    // AAD = header · aux with the real-level sec-control byte (not the wire's 0).
    let mut aad = frame[..aux_end].to_vec();
    aad[aux_off] = sc;
    let mut ct = frame[aux_end..mic_off].to_vec();
    let mut mic = [0u8; 4];
    mic.copy_from_slice(&frame[mic_off..]);
    ccm_decrypt(key, &eui, fc, sc, &aad, &mut ct, &mic)?;
    Ok(ct)
}

// ---- frame assembly: ZDP → APS → NWK → MAC --------------------------------
// Builds a complete injectable 802.15.4 MAC frame carrying a NWK-secured ZDP
// request. The sniffer's `tx` takes exactly these bytes (the radio appends the
// 2-byte FCS). Layout, outer → inner:
//
//   MAC header(9) · [ NWK header(8) · aux-sec(14) · ENC{ APS header(8) · ZDP } · MIC(4) ]
//
// All short-address, single-hop, NWK-secured-with-network-key. Multi-hop routing
// (source-route subframe) and APS-layer security are out of scope here.

/// ZDP request cluster ids (the APS cluster == the ZDP command). The three we
/// use to interview a node.
pub const ZDP_NODE_DESC_REQ: u16 = 0x0002;
pub const ZDP_SIMPLE_DESC_REQ: u16 = 0x0004;
pub const ZDP_ACTIVE_EP_REQ: u16 = 0x0005;

/// ZDP request payload (= the APS payload): txn seq · target short(2 LE) ·
/// [endpoint] (only Simple_Desc_req carries the endpoint).
pub fn zdp_request(txn_seq: u8, target: u16, endpoint: Option<u8>) -> Vec<u8> {
    let mut p = Vec::with_capacity(4);
    p.push(txn_seq);
    p.extend_from_slice(&target.to_le_bytes());
    if let Some(ep) = endpoint {
        p.push(ep);
    }
    p
}

/// APS data header for a ZDP request: endpoint 0, profile 0x0000 (ZDP), the
/// cluster carries the ZDP command. Unicast, no APS security, no ack request.
pub fn aps_zdp_header(cluster: u16, aps_counter: u8) -> Vec<u8> {
    vec![
        0x00, // frame control: data · unicast · no security · no ack
        0x00, // destination endpoint 0 (ZDP)
        (cluster & 0xff) as u8,
        (cluster >> 8) as u8,
        0x00,
        0x00, // profile 0x0000 (ZDP)
        0x00, // source endpoint 0
        aps_counter,
    ]
}

/// Cleartext (authenticated) NWK data header: FCF · dst · src · radius · seq.
/// FCF 0x0208 = data (type 0) · protocol version 2 (Zigbee PRO) · security on.
/// (Frame-type bits 0-1 = 00 = data; 0x09 would be a NWK *command*.)
pub fn nwk_header(dst: u16, src: u16, radius: u8, seq: u8) -> Vec<u8> {
    let fcf: u16 = 0x0208;
    let mut h = Vec::with_capacity(8);
    h.extend_from_slice(&fcf.to_le_bytes());
    h.extend_from_slice(&dst.to_le_bytes());
    h.extend_from_slice(&src.to_le_bytes());
    h.push(radius);
    h.push(seq);
    h
}

/// 802.15.4 data MAC header: FCF 0x8861 = data · ack request · PAN-ID
/// compression · short dst · short src. Then seq · dst PAN · dst · src.
pub fn mac_header(seq: u8, pan: u16, dst: u16, src: u16) -> Vec<u8> {
    let fcf: u16 = 0x8861;
    let mut h = Vec::with_capacity(9);
    h.extend_from_slice(&fcf.to_le_bytes());
    h.push(seq);
    h.extend_from_slice(&pan.to_le_bytes());
    h.extend_from_slice(&dst.to_le_bytes());
    h.extend_from_slice(&src.to_le_bytes());
    h
}

/// Everything needed to assemble one injectable ZDP request frame. Sequence
/// counters (`mac_seq`/`nwk_seq`/`aps_counter`/`zdp_seq`/`frame_counter`) are
/// caller-managed and must advance per frame; `src_short`/`src_eui64` are OUR
/// injector identity and must NOT collide with a real node (coordinator-safety).
pub struct ZdpInject<'a> {
    pub key: &'a [u8; 16],
    pub src_eui64: &'a [u8; 8],
    pub pan: u16,
    /// target node short address — MAC dst, NWK dst, and ZDP NWKAddrOfInterest.
    pub target: u16,
    pub src_short: u16,
    pub frame_counter: u32,
    pub mac_seq: u8,
    pub nwk_seq: u8,
    pub aps_counter: u8,
    pub zdp_seq: u8,
    pub radius: u8,
    pub key_seq: u8,
    pub cluster: u16,
    /// Some(endpoint) for Simple_Desc_req; None for Node_Desc/Active_EP.
    pub endpoint: Option<u8>,
}

/// Assemble the full injectable MAC frame (without FCS — the radio appends it).
pub fn build_zdp_inject(p: &ZdpInject) -> Result<Vec<u8>, String> {
    // APS frame = APS header · ZDP payload  → this becomes the encrypted NWK payload.
    let mut aps = aps_zdp_header(p.cluster, p.aps_counter);
    aps.extend_from_slice(&zdp_request(p.zdp_seq, p.target, p.endpoint));

    let nwkh = nwk_header(p.target, p.src_short, p.radius, p.nwk_seq);
    let nwk = secure_nwk(
        p.key,
        &nwkh,
        &aps,
        p.src_eui64,
        p.frame_counter,
        p.key_seq,
        SEC_LEVEL_ENC_MIC32,
    )?;

    let mut frame = mac_header(p.mac_seq, p.pan, p.target, p.src_short);
    frame.extend_from_slice(&nwk);
    Ok(frame)
}

// ---- ZCL command injection: control a peer (the "it's a light" path) --------
// A ZCL command is APS data on the functional cluster + HA profile, carrying a
// ZCL header (frame control · txn seq · command id · payload). Unlike a ZDP
// interview it expects no routed reply — with the default-response bit set the
// target just acts on it — so injecting one actually controls the device.

pub const ZCL_PROFILE_HA: u16 = 0x0104; // Home Automation application profile
pub const CLUSTER_ON_OFF: u16 = 0x0006;
pub const CLUSTER_LEVEL: u16 = 0x0008;
pub const CLUSTER_COLOR: u16 = 0x0300;
// On/Off cluster-specific command ids
pub const ONOFF_OFF: u8 = 0x00;
pub const ONOFF_ON: u8 = 0x01;
pub const ONOFF_TOGGLE: u8 = 0x02;

/// Everything to assemble one injectable ZCL command frame. Mirrors `ZdpInject`
/// but for an application cluster: control a peer directly.
pub struct ZclInject<'a> {
    pub key: &'a [u8; 16],
    pub src_eui64: &'a [u8; 8],
    pub pan: u16,
    pub target: u16,
    pub src_short: u16,
    pub frame_counter: u32,
    pub mac_seq: u8,
    pub nwk_seq: u8,
    pub aps_counter: u8,
    pub zcl_seq: u8,
    pub radius: u8,
    pub key_seq: u8,
    pub profile: u16,
    pub cluster: u16,
    pub src_endpoint: u8,
    pub dst_endpoint: u8,
    pub cmd: u8,
    /// true = cluster-specific (On/Off, Move-to-level…); false = global (read attr…).
    pub cluster_specific: bool,
    pub payload: &'a [u8],
}

/// Assemble the full injectable MAC frame for a ZCL command (no FCS — the radio
/// appends it). Layers: ZCL · APS data header · NWK-secured · MAC.
pub fn build_zcl_inject(p: &ZclInject) -> Result<Vec<u8>, String> {
    // ZCL header: frame control · txn seq · command id · payload.
    // FC: frame type (01 cluster-specific / 00 global) | disable-default-response (0x10).
    let zcl_fc = if p.cluster_specific { 0x01u8 } else { 0x00 } | 0x10;
    let mut zcl = Vec::with_capacity(3 + p.payload.len());
    zcl.push(zcl_fc);
    zcl.push(p.zcl_seq);
    zcl.push(p.cmd);
    zcl.extend_from_slice(p.payload);

    // APS data header: fc · dst ep · cluster(2 LE) · profile(2 LE) · src ep · counter.
    let mut aps = vec![
        0x00,
        p.dst_endpoint,
        (p.cluster & 0xff) as u8,
        (p.cluster >> 8) as u8,
        (p.profile & 0xff) as u8,
        (p.profile >> 8) as u8,
        p.src_endpoint,
        p.aps_counter,
    ];
    aps.extend_from_slice(&zcl);

    let nwkh = nwk_header(p.target, p.src_short, p.radius, p.nwk_seq);
    let nwk = secure_nwk(
        p.key,
        &nwkh,
        &aps,
        p.src_eui64,
        p.frame_counter,
        p.key_seq,
        SEC_LEVEL_ENC_MIC32,
    )?;
    let mut frame = mac_header(p.mac_seq, p.pan, p.target, p.src_short);
    frame.extend_from_slice(&nwk);
    Ok(frame)
}

// ---- receive side: decrypt an arbitrary sniffed frame + parse ZDP replies ---
// Injection (above) builds a fixed-shape header; sniffed frames are arbitrary, so
// to decrypt them we must compute the NWK header length from the FCF (optional
// IEEE addresses / multicast / source-route all change it) before handing the
// aux-header offset to unsecure_nwk. This is the half that turns an interview
// *reply* into data, and lets Sutra decrypt the live sniffer stream host-side.

/// Parse an 802.15.4 **data** MAC header → (header length, source short addr).
/// Handles the short/long-address + PAN-compression cases real Zigbee traffic
/// uses; None for non-data frames or addressing modes we don't decode. The NWK
/// frame starts at `header length`.
pub fn mac_data_header(frame: &[u8]) -> Option<(usize, u16)> {
    if frame.len() < 3 || frame[0] & 0x07 != 0x01 {
        return None; // frame type bits 001 = data
    }
    let fcf = u16::from_le_bytes([frame[0], frame[1]]);
    let pan_compress = fcf & (1 << 6) != 0;
    let dst_mode = (fcf >> 10) & 0x3;
    let src_mode = (fcf >> 14) & 0x3;
    let mut off = 3usize; // FCF(2) · sequence(1)
    if dst_mode != 0 {
        off += 2; // destination PAN
    }
    off += match dst_mode {
        0 => 0,
        2 => 2, // short
        3 => 8, // long
        _ => return None,
    };
    if src_mode != 0 && !pan_compress {
        off += 2; // source PAN
    }
    let mut src_short = 0u16;
    match src_mode {
        0 => {}
        2 => {
            let b = frame.get(off..off + 2)?;
            src_short = u16::from_le_bytes([b[0], b[1]]);
            off += 2;
        }
        3 => off += 8, // long source; short addr not available
        _ => return None,
    }
    (frame.len() >= off).then_some((off, src_short))
}

/// Length of the cleartext NWK header (FCF·dst·src·radius·seq + optional fields),
/// parsed from the FCF. None if the frame is too short to contain what it claims.
pub fn nwk_header_len(frame: &[u8]) -> Option<usize> {
    if frame.len() < 8 {
        return None;
    }
    let fcf = u16::from_le_bytes([frame[0], frame[1]]);
    let mut len = 8usize; // FCF(2)·dst(2)·src(2)·radius(1)·seq(1)
    if fcf & (1 << 11) != 0 {
        len += 8; // destination IEEE
    }
    if fcf & (1 << 12) != 0 {
        len += 8; // source IEEE
    }
    if fcf & (1 << 8) != 0 {
        len += 1; // multicast control
    }
    if fcf & (1 << 10) != 0 {
        // source route subframe: relay count(1)·relay index(1)·relay list(2·count)
        let relay_count = *frame.get(len)? as usize;
        len += 2 + 2 * relay_count;
    }
    (frame.len() >= len).then_some(len)
}

/// Decrypt a complete secured NWK frame (header length parsed from the FCF).
/// `level` is the network's configured security level (5 = ENC-MIC-32). Returns
/// the decrypted NWK payload (a NWK command, or an APS frame for data).
pub fn decrypt_nwk(key: &[u8; 16], frame: &[u8], level: u8) -> Result<Vec<u8>, String> {
    let hlen = nwk_header_len(frame).ok_or("cannot parse NWK header length")?;
    let fcf = u16::from_le_bytes([frame[0], frame[1]]);
    if fcf & (1 << 9) == 0 {
        return Err("NWK frame is not secured".into());
    }
    unsecure_nwk(key, frame, hlen, level)
}

/// A parsed APS data frame (the common unicast shape ZDP/ZCL use).
pub struct ApsData<'a> {
    pub dst_ep: u8,
    pub src_ep: u8,
    pub cluster: u16,
    pub profile: u16,
    pub payload: &'a [u8], // ZDP/ZCL payload (for ZDP: txn seq · response)
}

/// Parse a unicast APS **data** frame: fc·dst_ep·cluster·profile·src_ep·counter
/// then payload. None for non-data / group-addressed / too-short frames.
pub fn parse_aps_data(aps: &[u8]) -> Option<ApsData<'_>> {
    if aps.len() < 8 {
        return None;
    }
    let fc = aps[0];
    // Only the standard layout: data frame (00), UNICAST delivery (bits 2-3 = 00),
    // no extended header (bit 7). Group/broadcast/indirect or an extended header
    // shift the bytes — parsing those as fc·dst_ep·cluster·profile yields garbage
    // (e.g. a node address read as a "cluster"), which pollutes the model.
    if fc & 0x03 != 0 || (fc >> 2) & 0x03 != 0 || fc & 0x80 != 0 {
        return None;
    }
    Some(ApsData {
        dst_ep: aps[1],
        cluster: u16::from_le_bytes([aps[2], aps[3]]),
        profile: u16::from_le_bytes([aps[4], aps[5]]),
        src_ep: aps[6],
        payload: &aps[8..],
    })
}

// ZDP response cluster ids = request | 0x8000.
pub const ZDP_NODE_DESC_RSP: u16 = 0x8002;
pub const ZDP_SIMPLE_DESC_RSP: u16 = 0x8004;
pub const ZDP_ACTIVE_EP_RSP: u16 = 0x8005;

/// Active_EP_rsp: the endpoints a node hosts. `zdp` is the APS payload
/// (txn seq · status · addr · count · endpoints…).
pub struct ActiveEpRsp {
    pub status: u8,
    pub addr: u16,
    pub endpoints: Vec<u8>,
}
pub fn parse_active_ep_rsp(zdp: &[u8]) -> Option<ActiveEpRsp> {
    if zdp.len() < 5 {
        return None;
    }
    let count = zdp[4] as usize;
    Some(ActiveEpRsp {
        status: zdp[1],
        addr: u16::from_le_bytes([zdp[2], zdp[3]]),
        endpoints: zdp.get(5..5 + count)?.to_vec(),
    })
}

/// Simple_Desc_rsp: one endpoint's profile/device + input/output clusters.
pub struct SimpleDescRsp {
    pub status: u8,
    pub addr: u16,
    pub endpoint: u8,
    pub profile: u16,
    pub device: u16,
    pub in_clusters: Vec<u16>,
    pub out_clusters: Vec<u16>,
}
pub fn parse_simple_desc_rsp(zdp: &[u8]) -> Option<SimpleDescRsp> {
    // txn(1)·status(1)·addr(2)·len(1)·[endpoint(1)·profile(2)·device(2)·ver(1)·
    //   in_count(1)·in(2·n)·out_count(1)·out(2·m)]
    if zdp.len() < 5 {
        return None;
    }
    let d = &zdp[5..]; // the simple descriptor body
    if d.len() < 8 {
        return None;
    }
    let endpoint = d[0];
    let profile = u16::from_le_bytes([d[1], d[2]]);
    let device = u16::from_le_bytes([d[3], d[4]]);
    let in_count = d[6] as usize;
    let in_clusters = read_clusters(d, 7, in_count)?;
    let out_off = 7 + in_count * 2;
    let out_count = *d.get(out_off)? as usize;
    let out_clusters = read_clusters(d, out_off + 1, out_count)?;
    Some(SimpleDescRsp {
        status: zdp[1],
        addr: u16::from_le_bytes([zdp[2], zdp[3]]),
        endpoint,
        profile,
        device,
        in_clusters,
        out_clusters,
    })
}

/// Node_Desc_rsp: the node descriptor's manufacturer code is the headline field.
pub struct NodeDescRsp {
    pub status: u8,
    pub addr: u16,
    pub manufacturer: u16,
}
pub fn parse_node_desc_rsp(zdp: &[u8]) -> Option<NodeDescRsp> {
    // txn(1)·status(1)·addr(2)·node descriptor(13): the manufacturer code is at
    // descriptor bytes 3-4 (after the 3 flag/capability bytes).
    if zdp.len() < 4 + 5 {
        return None;
    }
    let desc = &zdp[4..];
    Some(NodeDescRsp {
        status: zdp[1],
        addr: u16::from_le_bytes([zdp[2], zdp[3]]),
        manufacturer: u16::from_le_bytes([desc[3], desc[4]]),
    })
}

// ---- ZCL attribute reports: read device state passively off the wire --------
// Nodes emit Report Attributes (global cmd 0x0a) and Read Attributes Response
// (0x01) constantly; we decrypt them, so the attribute values are free to
// harvest — no active read (which would need a routed reply to our ghost addr).

/// Decode one ZCL typed value → (bytes consumed, formatted string). None for a
/// type whose length we don't know (so the caller stops walking that record set).
fn zcl_value(ty: u8, data: &[u8]) -> Option<(usize, String)> {
    let u = |n: usize| -> Option<u64> {
        let b = data.get(..n)?;
        let mut v = 0u64;
        for (i, x) in b.iter().enumerate() {
            v |= (*x as u64) << (8 * i);
        }
        Some(v)
    };
    match ty {
        0x00 => Some((0, String::new())),                                  // no data
        0x10 => Some((1, (*data.first()? != 0).to_string())),             // bool
        0x08 | 0x18 | 0x20 | 0x30 => Some((1, u(1)?.to_string())),        // data8/map8/uint8/enum8
        0x28 => Some((1, (*data.first()? as i8).to_string())),            // int8
        0x21 | 0x31 => Some((2, u(2)?.to_string())),                      // uint16/enum16
        0x29 => Some((2, (i16::from_le_bytes([data[0], data[1]])).to_string())), // int16
        0x22 => Some((3, u(3)?.to_string())),                            // uint24
        0x23 => Some((4, u(4)?.to_string())),                            // uint32
        0x2b => Some((4, (i32::from_le_bytes(data.get(..4)?.try_into().ok()?)).to_string())), // int32
        0x39 => Some((4, f32::from_le_bytes(data.get(..4)?.try_into().ok()?).to_string())),   // float
        0x41 | 0x42 => {
            let n = *data.first()? as usize; // length-prefixed octet/char string
            let s = data.get(1..1 + n)?;
            Some((1 + n, String::from_utf8_lossy(s).into_owned()))
        }
        _ => None,
    }
}

/// Parse attribute records from a ZCL Report Attributes / Read Attributes
/// Response payload (the bytes AFTER the ZCL header). `read_response` adds the
/// per-record status byte. Returns (attribute id, formatted value).
pub fn parse_zcl_attr_reports(rec: &[u8], read_response: bool) -> Vec<(u16, String)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 2 <= rec.len() {
        let attr = u16::from_le_bytes([rec[i], rec[i + 1]]);
        i += 2;
        if read_response {
            let Some(&st) = rec.get(i) else { break };
            i += 1;
            if st != 0 {
                continue; // failed read — no type/value follows
            }
        }
        let Some(&ty) = rec.get(i) else { break };
        i += 1;
        match zcl_value(ty, rec.get(i..).unwrap_or(&[])) {
            Some((n, v)) => {
                out.push((attr, v));
                i += n;
            }
            None => break, // unknown type — can't size the remaining records
        }
    }
    out
}

fn read_clusters(d: &[u8], off: usize, count: usize) -> Option<Vec<u16>> {
    let bytes = d.get(off..off + count * 2)?;
    Some(
        bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect(),
    )
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
        let back = unsecure_nwk(&KEY, &frame, header.len(), SEC_LEVEL_ENC_MIC32).unwrap();
        assert_eq!(back, payload, "unsecure recovers the NWK payload");
        // the wire aux sec-control byte must carry level 0 (Zigbee convention)
        assert_eq!(frame[header.len()] & 0x07, 0, "wire level is zeroed");

        // a flipped ciphertext byte must fail the MIC
        let mut bad = frame.clone();
        let ct0 = header.len() + 14;
        bad[ct0] ^= 0x01;
        assert!(unsecure_nwk(&KEY, &bad, header.len(), SEC_LEVEL_ENC_MIC32).is_err());
    }

    /// Regression vector from a REAL Silicon Labs frame captured on the bench
    /// (channel 11, key a140355784cca894…). tshark decrypts it to a Link Status
    /// command (cmd 0x08, options 0x6a). Proves the nonce/AAD construction +
    /// the wire-level override against silicon, not just our own round-trip.
    #[test]
    fn real_frame_decrypt() {
        // The NWK frame (MAC header stripped): NWK hdr(16, incl ext-source) ·
        // aux(14) · ciphertext(32) · MIC(4).
        let nwk: Vec<u8> = vec![
            0x09, 0x12, 0xfc, 0xff, 0x6e, 0xb5, 0x01, 0xac, // FCF·dst·src·radius·seq
            0x69, 0xa2, 0xc2, 0xfe, 0xff, 0x27, 0x87, 0x04, // NWK ext-source EUI64
            0x28, 0xe6, 0xe5, 0x58, 0x00, // aux: sec-control(level 0!)·frame counter
            0x69, 0xa2, 0xc2, 0xfe, 0xff, 0x27, 0x87, 0x04, // aux: source EUI64
            0x00, // aux: key sequence
            0x3e, 0xd1, 0x9d, 0x7e, 0x0e, 0xbd, 0xde, 0xd4, 0xbc, 0x7d, 0xaa, 0x4d, 0xdd, 0xc9,
            0x2f, 0xa6, 0x83, 0x72, 0x4e, 0x4c, 0x53, 0x83, 0x0f, 0x64, 0x3c, 0x6f, 0x64, 0xfa,
            0xbb, 0xc4, 0xe3, 0x38, // ciphertext (32)
            0x17, 0xde, 0xb0, 0x61, // MIC
        ];
        let key: [u8; 16] = [
            0xa1, 0x40, 0x35, 0x57, 0x84, 0xcc, 0xa8, 0x94, 0xa1, 0x40, 0x35, 0x57, 0x84, 0xcc,
            0xa8, 0x94,
        ];
        // wire sec-control is level 0; the real network level is 5.
        let pt = unsecure_nwk(&key, &nwk, 16, SEC_LEVEL_ENC_MIC32)
            .expect("real frame MIC must verify with the level override");
        assert_eq!(pt.len(), 32, "Link Status payload length");
        assert_eq!(pt[0], 0x08, "NWK command id = Link Status");
        assert_eq!(pt[1], 0x6a, "Link Status options (Last|First|count=10)");

        // sanity: WITHOUT the override (treat wire level 0 as real) the MIC fails.
        assert!(
            unsecure_nwk(&key, &nwk, 16, 0).is_err(),
            "level-0 (no override) must NOT verify — proves the override is load-bearing"
        );
    }

    #[test]
    fn zdp_active_ep_inject() {
        let p = ZdpInject {
            key: &KEY,
            src_eui64: &EUI,
            pan: 0x1234,
            target: 0xabcd,
            src_short: 0x7fff, // a high, unlikely-to-collide injector address
            frame_counter: 100,
            mac_seq: 0x10,
            nwk_seq: 0x20,
            aps_counter: 0x30,
            zdp_seq: 0x40,
            radius: 30,
            key_seq: 0,
            cluster: ZDP_ACTIVE_EP_REQ,
            endpoint: None,
        };
        let frame = build_zdp_inject(&p).unwrap();

        // MAC header: FCF 0x8861 · seq · dst PAN · dst · src
        assert_eq!(&frame[0..2], &[0x61, 0x88], "MAC FCF = data/ack/short");
        assert_eq!(frame[2], 0x10, "MAC seq");
        assert_eq!(&frame[3..5], &[0x34, 0x12], "dst PAN LE");
        assert_eq!(&frame[5..7], &[0xcd, 0xab], "dst short LE");
        assert_eq!(&frame[7..9], &[0xff, 0x7f], "src short LE");

        // NWK header (cleartext) starts at byte 9: FCF 0x0209 · dst · src · radius · seq
        assert_eq!(&frame[9..11], &[0x08, 0x02], "NWK FCF = data/v2/secured");
        assert_eq!(&frame[11..13], &[0xcd, 0xab], "NWK dst LE");
        assert_eq!(&frame[13..15], &[0xff, 0x7f], "NWK src LE");
        assert_eq!(frame[15], 30, "radius");
        assert_eq!(frame[16], 0x20, "NWK seq");

        // The secured NWK frame is everything after the 9-byte MAC header.
        // Decrypt it and confirm the APS+ZDP payload round-trips.
        let secured = &frame[9..];
        let aps = unsecure_nwk(&KEY, secured, 8, SEC_LEVEL_ENC_MIC32).unwrap();
        // APS header: fc=0 · dst ep 0 · cluster(2 LE) · profile(2 LE) · src ep 0 · counter
        assert_eq!(aps[0], 0x00, "APS frame control: data/unicast");
        assert_eq!(aps[1], 0x00, "APS dst endpoint 0");
        assert_eq!(&aps[2..4], &[0x05, 0x00], "cluster = Active_EP_req");
        assert_eq!(&aps[4..6], &[0x00, 0x00], "profile = ZDP");
        assert_eq!(aps[6], 0x00, "APS src endpoint 0");
        assert_eq!(aps[7], 0x30, "APS counter");
        // ZDP payload: txn seq · target(2 LE)
        assert_eq!(aps[8], 0x40, "ZDP txn seq");
        assert_eq!(&aps[9..11], &[0xcd, 0xab], "ZDP target = node short");
        assert_eq!(aps.len(), 11, "no trailing endpoint for Active_EP_req");
    }

    #[test]
    fn zdp_simple_desc_carries_endpoint() {
        let zdp = zdp_request(0x40, 0xabcd, Some(7));
        assert_eq!(zdp, vec![0x40, 0xcd, 0xab, 0x07]);
    }

    #[test]
    fn mac_data_header_real_frame() {
        // Real MAC frame: FCF 0x8841 (data·PANcompress·short dst·short src).
        let mac = [
            0x41, 0x88, 0x57, 0x84, 0x0c, 0xff, 0xff, 0x6e, 0xb5, 0x09, 0x12, // …NWK begins
        ];
        let (hlen, src) = mac_data_header(&mac).unwrap();
        assert_eq!(hlen, 9, "FCF·seq·dstPAN·dst·src = 9 (PAN compression)");
        assert_eq!(src, 0xb56e, "source short address");
        // a non-data frame (ack, type 010) is rejected
        assert!(mac_data_header(&[0x02, 0x00, 0x00]).is_none());
    }

    #[test]
    fn zcl_on_command_inject() {
        // "Turn on the light at 0xabcd, endpoint 1" — On/Off cluster, On command.
        let p = ZclInject {
            key: &KEY,
            src_eui64: &EUI,
            pan: 0x0c84,
            target: 0xabcd,
            src_short: 0x7fff,
            frame_counter: 50,
            mac_seq: 0x10,
            nwk_seq: 0x20,
            aps_counter: 0x30,
            zcl_seq: 0x40,
            radius: 30,
            key_seq: 0,
            profile: ZCL_PROFILE_HA,
            cluster: CLUSTER_ON_OFF,
            src_endpoint: 1,
            dst_endpoint: 1,
            cmd: ONOFF_ON,
            cluster_specific: true,
            payload: &[],
        };
        let frame = build_zcl_inject(&p).unwrap();
        assert_eq!(&frame[5..7], &[0xcd, 0xab], "MAC dst = the light");
        // decrypt the NWK payload (8-byte minimal header) → APS + ZCL
        let aps = unsecure_nwk(&KEY, &frame[9..], 8, SEC_LEVEL_ENC_MIC32).unwrap();
        assert_eq!(aps[1], 0x01, "APS dst endpoint 1");
        assert_eq!(&aps[2..4], &[0x06, 0x00], "cluster = On/Off");
        assert_eq!(&aps[4..6], &[0x04, 0x01], "profile = HA 0x0104");
        // ZCL header at aps[8..]: frame control · seq · command
        assert_eq!(aps[8], 0x11, "ZCL fc: cluster-specific + disable default response");
        assert_eq!(aps[9], 0x40, "ZCL txn seq");
        assert_eq!(aps[10], 0x01, "ZCL command = On");
        assert_eq!(aps.len(), 11, "no payload for a bare On");
    }

    #[test]
    fn zcl_attribute_reports_parse() {
        // Report Attributes: attr 0x0000 uint8=42, attr 0x0021 uint8=200 (battery %).
        let rec = [0x00, 0x00, 0x20, 42, 0x21, 0x00, 0x20, 200];
        let a = parse_zcl_attr_reports(&rec, false);
        assert_eq!(a, vec![(0x0000, "42".to_string()), (0x0021, "200".to_string())]);
        // Read Attributes Response: attr 0x0000 status OK bool=true; attr 0x0001 status FAIL.
        let rsp = [0x00, 0x00, 0x00, 0x10, 0x01, 0x01, 0x00, 0x86];
        let b = parse_zcl_attr_reports(&rsp, true);
        assert_eq!(b, vec![(0x0000, "true".to_string())], "failed read (status 0x86) skipped");
        // int16 temperature 0x0402/attr 0x0000 = 2350 (23.50 °C ×100)
        let t = [0x00, 0x00, 0x29, 0x2e, 0x09];
        assert_eq!(parse_zcl_attr_reports(&t, false), vec![(0x0000, "2350".to_string())]);
    }

    #[test]
    fn nwk_header_len_real_frame() {
        // The real Link Status frame's NWK header: FCF 0x1209 sets ext-source
        // (bit 12) → 8 base + 8 EUI = 16 bytes.
        let nwk = [
            0x09, 0x12, 0xfc, 0xff, 0x6e, 0xb5, 0x01, 0xac, 0x69, 0xa2, 0xc2, 0xfe, 0xff, 0x27,
            0x87, 0x04, 0x28, 0x00, // (rest doesn't matter for header-len)
        ];
        assert_eq!(nwk_header_len(&nwk), Some(16));
        // a minimal data header (no optional fields) is 8
        let min = [0x08, 0x02, 0x00, 0x00, 0x34, 0x12, 0x1e, 0x05, 0x28];
        assert_eq!(nwk_header_len(&min), Some(8));
    }

    #[test]
    fn decrypt_nwk_real_frame() {
        // Same real frame as real_frame_decrypt, but via the FCF-driven path.
        let nwk: Vec<u8> = vec![
            0x09, 0x12, 0xfc, 0xff, 0x6e, 0xb5, 0x01, 0xac, 0x69, 0xa2, 0xc2, 0xfe, 0xff, 0x27,
            0x87, 0x04, 0x28, 0xe6, 0xe5, 0x58, 0x00, 0x69, 0xa2, 0xc2, 0xfe, 0xff, 0x27, 0x87,
            0x04, 0x00, 0x3e, 0xd1, 0x9d, 0x7e, 0x0e, 0xbd, 0xde, 0xd4, 0xbc, 0x7d, 0xaa, 0x4d,
            0xdd, 0xc9, 0x2f, 0xa6, 0x83, 0x72, 0x4e, 0x4c, 0x53, 0x83, 0x0f, 0x64, 0x3c, 0x6f,
            0x64, 0xfa, 0xbb, 0xc4, 0xe3, 0x38, 0x17, 0xde, 0xb0, 0x61,
        ];
        let key: [u8; 16] = [
            0xa1, 0x40, 0x35, 0x57, 0x84, 0xcc, 0xa8, 0x94, 0xa1, 0x40, 0x35, 0x57, 0x84, 0xcc,
            0xa8, 0x94,
        ];
        let pt = decrypt_nwk(&key, &nwk, SEC_LEVEL_ENC_MIC32).unwrap();
        assert_eq!(&pt[..2], &[0x08, 0x6a], "Link Status, header len auto-parsed");
    }

    #[test]
    fn interview_reply_roundtrip() {
        // Build an Active_EP_rsp the way a node would (APS data · ZDP payload),
        // secure it as a NWK data frame, then run the full receive path.
        let zdp = vec![0x40, 0x00, 0xcd, 0xab, 0x02, 0x01, 0x0a]; // txn·status·addr·count·eps
        let mut aps = aps_zdp_header(ZDP_ACTIVE_EP_RSP, 0x30);
        aps.extend_from_slice(&zdp);
        let header = nwk_header(0x7fff, 0xabcd, 30, 0x20); // node → us
        let frame = secure_nwk(&KEY, &header, &aps, &EUI, 5, 0, SEC_LEVEL_ENC_MIC32).unwrap();

        let dec = decrypt_nwk(&KEY, &frame, SEC_LEVEL_ENC_MIC32).unwrap();
        let aps = parse_aps_data(&dec).unwrap();
        assert_eq!(aps.cluster, ZDP_ACTIVE_EP_RSP);
        assert_eq!(aps.profile, 0x0000);
        let rsp = parse_active_ep_rsp(aps.payload).unwrap();
        assert_eq!(rsp.status, 0);
        assert_eq!(rsp.addr, 0xabcd);
        assert_eq!(rsp.endpoints, vec![1, 10], "the node's two endpoints");
    }

    #[test]
    fn simple_desc_rsp_parses_clusters() {
        // descriptor: ep1 · profile 0x0104 · device 0x0100 · ver · in[0x0006,0x0008] · out[0x0019]
        let zdp = vec![
            0x40, 0x00, 0xcd, 0xab, 0x0e, // txn·status·addr·len
            0x01, 0x04, 0x01, 0x00, 0x01, 0x00, // ep·profile·device·ver
            0x02, 0x06, 0x00, 0x08, 0x00, // in_count·in clusters
            0x01, 0x19, 0x00, // out_count·out clusters
        ];
        let d = parse_simple_desc_rsp(&zdp).unwrap();
        assert_eq!(d.endpoint, 1);
        assert_eq!(d.profile, 0x0104);
        assert_eq!(d.in_clusters, vec![0x0006, 0x0008]);
        assert_eq!(d.out_clusters, vec![0x0019]);
    }

    #[test]
    fn node_desc_rsp_manufacturer() {
        // txn·status·addr · descriptor(flags 3 bytes · manufacturer 0x1037 · …)
        let zdp = vec![0x40, 0x00, 0xcd, 0xab, 0x00, 0x40, 0x8e, 0x37, 0x10, 0x52, 0x80];
        let n = parse_node_desc_rsp(&zdp).unwrap();
        assert_eq!(n.manufacturer, 0x1037, "manufacturer code");
    }
}
