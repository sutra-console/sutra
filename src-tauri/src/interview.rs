//! Live ZDP interview: turn a sniffed 802.15.4 frame into node knowledge.
//!
//! When Sutra injects a ZDP request (`HEX {$zdp …}`), the node's reply comes
//! back as just another sniffed frame. This module decrypts it against the
//! active network, parses the ZDP response, and merges the discovery
//! (endpoints / clusters / manufacturer) into the workspace NetNode — the fields
//! the model has reserved for active discovery since phase A.
//!
//! The decrypt/parse pipeline (`parse_zdp_response`) is pure + unit-tested; only
//! `ingest_mac_frame` touches the workspace.

use serde::Serialize;
use tauri::AppHandle;

use crate::workspace::{self, NetEndpoint, NetNode, Network};
use crate::zigbee::{
    decrypt_nwk, mac_data_header, parse_active_ep_rsp, parse_aps_data, parse_node_desc_rsp,
    parse_simple_desc_rsp, SEC_LEVEL_ENC_MIC32, ZDP_ACTIVE_EP_RSP, ZDP_NODE_DESC_RSP,
    ZDP_SIMPLE_DESC_RSP,
};

/// What one ZDP response told us about a node. Emitted to the UI and merged into
/// the NetNode model.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct ZdpDiscovery {
    pub addr: String, // "0xabcd"
    pub kind: String, // active_ep | simple_desc | node_desc
    #[serde(default)]
    pub endpoints: Vec<u8>,
    #[serde(default)]
    pub endpoint: Option<u8>, // simple_desc: which endpoint
    #[serde(default)]
    pub in_clusters: Vec<String>,
    #[serde(default)]
    pub out_clusters: Vec<String>,
    #[serde(default)]
    pub manufacturer: Option<String>,
}

fn parse_key(s: &str) -> Option<[u8; 16]> {
    let t = s.trim().trim_start_matches("0x");
    if t.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn fmt_addr(a: u16) -> String {
    format!("0x{a:04x}")
}
fn fmt_id(id: u16) -> String {
    format!("0x{id:04x}")
}

/// Decrypt + parse a sniffed MAC frame as a ZDP response. Pure: no workspace.
/// Returns None for anything that isn't a successful ZDP reply we model.
pub fn parse_zdp_response(key: &[u8; 16], mac: &[u8], level: u8) -> Option<ZdpDiscovery> {
    let (mlen, _src) = mac_data_header(mac)?;
    let aps_bytes = decrypt_nwk(key, mac.get(mlen..)?, level).ok()?;
    let aps = parse_aps_data(&aps_bytes)?;
    if aps.profile != 0x0000 {
        return None; // ZDP lives on profile 0x0000
    }
    match aps.cluster {
        ZDP_ACTIVE_EP_RSP => {
            let r = parse_active_ep_rsp(aps.payload)?;
            (r.status == 0).then(|| ZdpDiscovery {
                addr: fmt_addr(r.addr),
                kind: "active_ep".into(),
                endpoints: r.endpoints,
                ..Default::default()
            })
        }
        ZDP_SIMPLE_DESC_RSP => {
            let r = parse_simple_desc_rsp(aps.payload)?;
            (r.status == 0).then(|| ZdpDiscovery {
                addr: fmt_addr(r.addr),
                kind: "simple_desc".into(),
                endpoint: Some(r.endpoint),
                in_clusters: r.in_clusters.iter().map(|c| fmt_id(*c)).collect(),
                out_clusters: r.out_clusters.iter().map(|c| fmt_id(*c)).collect(),
                ..Default::default()
            })
        }
        ZDP_NODE_DESC_RSP => {
            let r = parse_node_desc_rsp(aps.payload)?;
            (r.status == 0).then(|| ZdpDiscovery {
                addr: fmt_addr(r.addr),
                kind: "node_desc".into(),
                manufacturer: Some(fmt_id(r.manufacturer)),
                ..Default::default()
            })
        }
        _ => None,
    }
}

/// Merge a discovery into a node (created if new). Endpoints/clusters/
/// manufacturer accumulate; re-running an interview refreshes in place.
pub fn merge_into_node(net: &mut Network, d: &ZdpDiscovery) {
    if !net.nodes.iter().any(|n| n.addr == d.addr) {
        net.nodes.push(NetNode { addr: d.addr.clone(), ..Default::default() });
    }
    let node = net.nodes.iter_mut().find(|n| n.addr == d.addr).unwrap();
    match d.kind.as_str() {
        "active_ep" => {
            for &ep in &d.endpoints {
                if !node.endpoints.iter().any(|e| e.id == ep) {
                    node.endpoints.push(NetEndpoint { id: ep, clusters: vec![] });
                }
            }
        }
        "simple_desc" => {
            if let Some(ep) = d.endpoint {
                if !node.endpoints.iter().any(|e| e.id == ep) {
                    node.endpoints.push(NetEndpoint { id: ep, clusters: vec![] });
                }
                let e = node.endpoints.iter_mut().find(|e| e.id == ep).unwrap();
                let mut cl = d.in_clusters.clone();
                cl.extend(d.out_clusters.iter().cloned());
                e.clusters = cl;
            }
        }
        "node_desc" => {
            if let Some(m) = &d.manufacturer {
                node.manufacturer = m.clone();
            }
        }
        _ => {}
    }
}

/// Passive: record that node `addr` is associated with `cluster` on `endpoint`,
/// learned from normal decrypted traffic (no interrogation). Endpoints/clusters
/// accumulate; duplicates are ignored.
pub fn observe_into_node(net: &mut Network, addr: &str, endpoint: u8, cluster: u16) {
    if !net.nodes.iter().any(|n| n.addr == addr) {
        net.nodes.push(NetNode { addr: addr.to_string(), ..Default::default() });
    }
    let node = net.nodes.iter_mut().find(|n| n.addr == addr).unwrap();
    if !node.endpoints.iter().any(|e| e.id == endpoint) {
        node.endpoints.push(NetEndpoint { id: endpoint, clusters: vec![] });
    }
    let e = node.endpoints.iter_mut().find(|e| e.id == endpoint).unwrap();
    let c = fmt_id(cluster);
    if !e.clusters.contains(&c) {
        e.clusters.push(c);
    }
}

/// Pure: decrypt a sniffed application (non-ZDP) APS frame and return the
/// (node, endpoint, cluster) associations it reveals — source end always, and
/// the dest end when it's a unicast (not broadcast). Empty for anything that
/// isn't a decryptable application APS data frame.
pub fn observe_frame(key: &[u8; 16], mac: &[u8], level: u8) -> Vec<(u16, u8, u16)> {
    let mut out = Vec::new();
    let Some((mlen, _)) = mac_data_header(mac) else { return out };
    let Some(nwk) = mac.get(mlen..) else { return out };
    if nwk.len() < 6 {
        return out;
    }
    let nwk_dst = u16::from_le_bytes([nwk[2], nwk[3]]);
    let nwk_src = u16::from_le_bytes([nwk[4], nwk[5]]);
    let Ok(aps_bytes) = decrypt_nwk(key, nwk, level) else { return out };
    let Some(aps) = parse_aps_data(&aps_bytes) else { return out };
    if aps.profile == 0x0000 {
        return out; // ZDP / NWK-mgmt, not application clusters
    }
    out.push((nwk_src, aps.src_ep, aps.cluster));
    if nwk_dst != 0xfffc && nwk_dst != 0xffff {
        out.push((nwk_dst, aps.dst_ep, aps.cluster));
    }
    out
}

/// Batch-ingest sniffed MAC frames against the active network: decrypt each,
/// route ZDP replies through the active-discovery merge, and passively record
/// endpoints/clusters from any other application (APS) frame. One workspace
/// write for the whole batch. Returns how many frames yielded model changes.
pub fn ingest_frames(app: &AppHandle, frames: &[Vec<u8>]) -> usize {
    let mut nets = workspace::load_networks(app);
    let Some(idx) = workspace::active_network_index(&nets) else {
        return 0;
    };
    let Some(key) = parse_key(&nets.networks[idx].key) else {
        return 0;
    };
    let level = SEC_LEVEL_ENC_MIC32;
    let mut changed = 0usize;
    for mac in frames {
        // ZDP reply addressed to our injector → active-discovery merge.
        if let Some(d) = parse_zdp_response(&key, mac, level) {
            merge_into_node(&mut nets.networks[idx], &d);
            changed += 1;
            continue;
        }
        // Otherwise: passive endpoint/cluster observation from any APS data frame.
        let obs = observe_frame(&key, mac, level);
        for (addr, ep, cl) in &obs {
            observe_into_node(&mut nets.networks[idx], &fmt_addr(*addr), *ep, *cl);
        }
        if !obs.is_empty() {
            changed += 1;
        }
    }
    if changed > 0 {
        let _ = workspace::save_networks(app, &nets);
    }
    changed
}

/// Try to ingest a sniffed MAC frame as a ZDP reply against the active network,
/// persisting any discovery. Returns it (for the UI) or None if the frame isn't
/// a ZDP reply we can decrypt + model.
pub fn ingest_mac_frame(app: &AppHandle, mac: &[u8]) -> Option<ZdpDiscovery> {
    let mut nets = workspace::load_networks(app);
    let idx = workspace::active_network_index(&nets)?;
    let key = parse_key(&nets.networks[idx].key)?;
    let disc = parse_zdp_response(&key, mac, SEC_LEVEL_ENC_MIC32)?;
    merge_into_node(&mut nets.networks[idx], &disc);
    let _ = workspace::save_networks(app, &nets);
    Some(disc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zigbee::{aps_zdp_header, mac_header, nwk_header, secure_nwk};

    const KEY: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ];
    const EUI: [u8; 8] = [0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11];

    /// Build the full sniffed frame a node would send for an Active_EP_rsp.
    fn active_ep_reply_frame() -> Vec<u8> {
        let zdp = vec![0x40, 0x00, 0xcd, 0xab, 0x02, 0x01, 0x0a]; // txn·status·addr·count·eps
        let mut aps = aps_zdp_header(ZDP_ACTIVE_EP_RSP, 0x30);
        aps.extend_from_slice(&zdp);
        let nwk = secure_nwk(&KEY, &nwk_header(0x7fff, 0xabcd, 30, 0x20), &aps, &EUI, 5, 0, SEC_LEVEL_ENC_MIC32).unwrap();
        let mut frame = mac_header(0x11, 0x0c84, 0x7fff, 0xabcd); // node → us
        frame.extend_from_slice(&nwk);
        frame
    }

    #[test]
    fn parses_active_ep_reply() {
        let frame = active_ep_reply_frame();
        let d = parse_zdp_response(&KEY, &frame, SEC_LEVEL_ENC_MIC32).unwrap();
        assert_eq!(d.addr, "0xabcd");
        assert_eq!(d.kind, "active_ep");
        assert_eq!(d.endpoints, vec![1, 10]);
    }

    #[test]
    fn wrong_key_yields_nothing() {
        let frame = active_ep_reply_frame();
        let mut bad = KEY;
        bad[0] ^= 0xff;
        assert!(parse_zdp_response(&bad, &frame, SEC_LEVEL_ENC_MIC32).is_none());
    }

    #[test]
    fn observes_app_clusters_from_traffic() {
        // A node (0xabcd, endpoint 1) reports On/Off (cluster 0x0006, HA profile
        // 0x0104) to the coordinator — a normal frame we can decrypt + learn from.
        let zcl = vec![0x18, 0x4a, 0x0a, 0x00, 0x00, 0x10, 0x01]; // ZCL Report Attributes
        let mut aps = vec![0x00, 0x01, 0x06, 0x00, 0x04, 0x01, 0x01, 0x55]; // fc·dstEp·cluster·profile·srcEp·ctr
        aps.extend_from_slice(&zcl);
        let nwk = secure_nwk(&KEY, &nwk_header(0x0000, 0xabcd, 30, 0x20), &aps, &EUI, 7, 0, SEC_LEVEL_ENC_MIC32).unwrap();
        let mut frame = mac_header(0x11, 0x0c84, 0x0000, 0xabcd);
        frame.extend_from_slice(&nwk);

        let obs = observe_frame(&KEY, &frame, SEC_LEVEL_ENC_MIC32);
        // source (the reporting node) + dest (coordinator), both endpoint 1, cluster 0x0006
        assert!(obs.contains(&(0xabcd, 1, 0x0006)), "node's own cluster observed");
        assert!(obs.contains(&(0x0000, 1, 0x0006)), "coordinator end observed");

        let mut net = Network::default();
        for (a, e, c) in &obs {
            observe_into_node(&mut net, &fmt_addr(*a), *e, *c);
        }
        let node = net.nodes.iter().find(|n| n.addr == "0xabcd").unwrap();
        assert_eq!(node.endpoints[0].id, 1);
        assert_eq!(node.endpoints[0].clusters, vec!["0x0006"]);
        // idempotent: re-observing the same thing doesn't duplicate
        observe_into_node(&mut net, "0xabcd", 1, 0x0006);
        let node = net.nodes.iter().find(|n| n.addr == "0xabcd").unwrap();
        assert_eq!(node.endpoints[0].clusters.len(), 1);
    }

    #[test]
    fn merge_accumulates() {
        let mut net = Network::default();
        merge_into_node(&mut net, &ZdpDiscovery {
            addr: "0xabcd".into(),
            kind: "active_ep".into(),
            endpoints: vec![1, 10],
            ..Default::default()
        });
        merge_into_node(&mut net, &ZdpDiscovery {
            addr: "0xabcd".into(),
            kind: "simple_desc".into(),
            endpoint: Some(1),
            in_clusters: vec!["0x0006".into(), "0x0008".into()],
            ..Default::default()
        });
        merge_into_node(&mut net, &ZdpDiscovery {
            addr: "0xabcd".into(),
            kind: "node_desc".into(),
            manufacturer: Some("0x1037".into()),
            ..Default::default()
        });
        assert_eq!(net.nodes.len(), 1, "one node, merged in place");
        let n = &net.nodes[0];
        assert_eq!(n.endpoints.len(), 2);
        assert_eq!(n.endpoints.iter().find(|e| e.id == 1).unwrap().clusters, vec!["0x0006", "0x0008"]);
        assert_eq!(n.manufacturer, "0x1037");
    }
}
