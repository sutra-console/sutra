//! Macro variables: `{$name}` / `{$name arg arg}` substitution for the macro VM.
//!
//! Expansion is purely textual and happens per-line at play time, left-to-right,
//! with side effects (a `{$fc}` pulls + increments the frame counter). The result
//! is spliced back into the line, so the existing macro parser handles it — e.g.
//! `HEX {$zdp active_ep abcd}` expands to space-separated hex the `HEX` keyword
//! then consumes. This is the bridge between the verified Zigbee crypto
//! (`crate::zigbee`) and the dumb-radio inject path: a macro composes an
//! encrypted frame without any one-off command.
//!
//! v1 vocabulary:
//!   {$key} {$pan} {$channel} {$src} {$eui}   network/injector context
//!   {$fc}                                     NWK frame counter (pull + increment)
//!   {$seq}                                    per-run sequence byte
//!   {$zdp <cmd> <target> [endpoint]}          a full injectable ZDP request frame
//!   {$NAME}                                   a user variable (set via `VAR NAME …`)
//!
//! The context here is data-only; the caller fills it from the workspace network
//! model and writes the advanced counter back (persistence lives in workspace.rs).

use std::collections::HashMap;

use crate::zigbee::{
    build_zcl_inject, build_zdp_inject, ZclInject, ZdpInject, CLUSTER_COLOR, CLUSTER_LEVEL,
    CLUSTER_ON_OFF, ONOFF_OFF, ONOFF_ON, ONOFF_TOGGLE, ZCL_PROFILE_HA, ZDP_ACTIVE_EP_REQ,
    ZDP_NODE_DESC_REQ, ZDP_SIMPLE_DESC_REQ,
};

/// Everything `{$…}` resolves against. Scalars are supplied by the caller; the
/// counters (`frame_counter`, `seq`) advance as tokens consume them.
#[derive(Clone, Default)]
pub struct VarContext {
    pub key: Option<[u8; 16]>,
    pub pan: u16,
    pub channel: u8,
    pub src_short: u16,
    pub src_eui64: [u8; 8],
    pub frame_counter: u32,
    pub seq: u8,
    pub vars: HashMap<String, String>,
}

impl VarContext {
    /// Pull the current NWK frame counter and advance it (the anti-replay value).
    fn take_fc(&mut self) -> u32 {
        let v = self.frame_counter;
        self.frame_counter = self.frame_counter.wrapping_add(1);
        v
    }

    fn take_seq(&mut self) -> u8 {
        let v = self.seq;
        self.seq = self.seq.wrapping_add(1);
        v
    }

    /// Resolve one `{$…}` token: `name` + whitespace-split `args`.
    fn eval(&mut self, name: &str, args: &[&str]) -> Result<String, String> {
        match name {
            "key" => self
                .key
                .map(hex_str)
                .ok_or_else(|| "no network key set (use NET <label>)".to_string()),
            "pan" => Ok(format!("{:04x}", self.pan)),
            "channel" => Ok(self.channel.to_string()),
            "src" => Ok(format!("{:04x}", self.src_short)),
            "eui" => Ok(hex_str(self.src_eui64)),
            "fc" => Ok(self.take_fc().to_string()),
            "seq" => Ok(format!("{:02x}", self.take_seq())),
            "zdp" => self.eval_zdp(args),
            "zcl" => self.eval_zcl(args),
            other => self
                .vars
                .get(other)
                .cloned()
                .ok_or_else(|| format!("unknown variable {{${other}}}")),
        }
    }

    /// `{$zdp <cmd> <target-hex> [endpoint]}` → the full injectable MAC frame as
    /// space-separated hex. cmd ∈ active_ep | node_desc | simple_desc.
    /// `{$zcl <target> <endpoint> <cluster> <cmd> [payload hex…]}` → an injectable
    /// ZCL command frame. cluster/cmd accept names (onoff/level/color, on/off/toggle)
    /// or hex. The "type a command at a peer" path — e.g. turn a light on.
    fn eval_zcl(&mut self, args: &[&str]) -> Result<String, String> {
        let target = parse_u16_hex(args.first().ok_or("zcl: missing target")?)?;
        let endpoint = parse_u8(args.get(1).ok_or("zcl: missing endpoint")?)?;
        let cluster = match args
            .get(2)
            .ok_or("zcl: missing cluster")?
            .to_ascii_lowercase()
            .as_str()
        {
            "onoff" | "on_off" => CLUSTER_ON_OFF,
            "level" => CLUSTER_LEVEL,
            "color" => CLUSTER_COLOR,
            h => parse_u16_hex(h)?,
        };
        let cmd = match args
            .get(3)
            .ok_or("zcl: missing command")?
            .to_ascii_lowercase()
            .as_str()
        {
            "off" => ONOFF_OFF,
            "on" => ONOFF_ON,
            "toggle" => ONOFF_TOGGLE,
            h => parse_u8(h)?,
        };
        let payload: Vec<u8> = args
            .get(4..)
            .unwrap_or(&[])
            .iter()
            .map(|a| parse_u8(a))
            .collect::<Result<_, _>>()?;
        let key = self
            .key
            .ok_or("zcl: no network key set (use NET <label>)")?;
        // Derive EVERY sequence/counter from the persistent, monotonic frame
        // counter — not the per-run seq (which resets to 0 each macro, so the APS
        // sublayer's (source, APS counter) duplicate table dropped repeats).
        let fc = self.take_fc();
        let frame = build_zcl_inject(&ZclInject {
            key: &key,
            src_eui64: &self.src_eui64,
            pan: self.pan,
            target,
            src_short: self.src_short,
            frame_counter: fc,
            mac_seq: fc as u8,
            nwk_seq: (fc >> 8) as u8,
            aps_counter: fc as u8,
            zcl_seq: fc as u8,
            radius: 30,
            key_seq: 0,
            profile: ZCL_PROFILE_HA,
            cluster,
            src_endpoint: 1,
            dst_endpoint: endpoint,
            cmd,
            cluster_specific: true,
            payload: &payload,
        })?;
        Ok(hex_bytes(&frame))
    }

    fn eval_zdp(&mut self, args: &[&str]) -> Result<String, String> {
        let cmd = args.first().ok_or("zdp: missing command")?;
        let target = parse_u16_hex(args.get(1).ok_or("zdp: missing target address")?)?;
        let (cluster, endpoint) = match cmd.to_ascii_lowercase().as_str() {
            "active_ep" | "activeep" => (ZDP_ACTIVE_EP_REQ, None),
            "node_desc" | "nodedesc" => (ZDP_NODE_DESC_REQ, None),
            "simple_desc" | "simpledesc" => {
                let ep = parse_u8(args.get(2).ok_or("zdp simple_desc: missing endpoint")?)?;
                (ZDP_SIMPLE_DESC_REQ, Some(ep))
            }
            other => return Err(format!("zdp: unknown command '{other}'")),
        };
        let key = self
            .key
            .ok_or("zdp: no network key set (use NET <label>)")?;
        // All counters from the persistent monotonic frame counter (see eval_zcl).
        let fc = self.take_fc();
        let frame = build_zdp_inject(&ZdpInject {
            key: &key,
            src_eui64: &self.src_eui64,
            pan: self.pan,
            target,
            src_short: self.src_short,
            frame_counter: fc,
            mac_seq: fc as u8,
            nwk_seq: (fc >> 8) as u8,
            aps_counter: fc as u8,
            zdp_seq: fc as u8,
            radius: 30,
            key_seq: 0,
            cluster,
            endpoint,
        })?;
        Ok(hex_bytes(&frame))
    }
}

/// Resolve every `{$…}` in `line`, left-to-right, with side effects. Non-token
/// text is copied verbatim; a `{` not starting a `{$` token is literal. No
/// nesting in v1. Returns the resolved line or the first eval error.
pub fn resolve_line(ctx: &mut VarContext, line: &str) -> Result<String, String> {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            // find the matching '}', depth-aware so nested {$…} can be an argument
            let mut depth = 0usize;
            let mut close = None;
            for (j, &b) in bytes.iter().enumerate().skip(i + 2) {
                match b {
                    b'{' => depth += 1,
                    b'}' if depth == 0 => {
                        close = Some(j);
                        break;
                    }
                    b'}' => depth -= 1,
                    _ => {}
                }
            }
            let close = close.ok_or("unterminated {$…} (missing '}')")?;
            // resolve any nested {$…} in the args first, then split name + args
            let inner = resolve_line(ctx, &line[i + 2..close])?;
            let mut toks = inner.split_whitespace();
            let name = toks.next().unwrap_or("");
            let args: Vec<&str> = toks.collect();
            out.push_str(&ctx.eval(name, &args)?);
            i = close + 1;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Ok(out)
}

/// Resolve a whole macro: each line through `resolve_line`, plus the `VAR NAME
/// value` directive (sets a user variable, usable later as `{$NAME}`; the value
/// may itself contain `{$…}`). A consumed `VAR` line is replaced by a comment so
/// the macro parser skips it. Left-to-right with side effects, so a `{$fc}` /
/// `VAR` earlier in the macro is visible to lines below it.
pub fn resolve_text(ctx: &mut VarContext, text: &str) -> Result<String, String> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let mut w = line.trim_start().splitn(2, char::is_whitespace);
        let kw = w.next().unwrap_or("");
        if kw.eq_ignore_ascii_case("VAR") {
            let rest = w.next().unwrap_or("").trim();
            match rest.split_once(char::is_whitespace) {
                Some((name, val)) => {
                    let v = resolve_line(ctx, val.trim())?;
                    ctx.vars.insert(name.to_string(), v);
                }
                None if !rest.is_empty() => {
                    ctx.vars.insert(rest.to_string(), String::new());
                }
                None => {}
            }
            out.push("#".to_string()); // consumed — parser treats it as a comment
        } else {
            out.push(resolve_line(ctx, line)?);
        }
    }
    Ok(out.join("\n"))
}

fn hex_str<const N: usize>(b: [u8; N]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hex_bytes(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_u16_hex(s: &str) -> Result<u16, String> {
    let t = s.trim().trim_start_matches("0x");
    u16::from_str_radix(t, 16).map_err(|_| format!("bad 16-bit hex '{s}'"))
}

fn parse_u8(s: &str) -> Result<u8, String> {
    let t = s.trim();
    t.parse::<u8>()
        .or_else(|_| u8::from_str_radix(t.trim_start_matches("0x"), 16))
        .map_err(|_| format!("bad u8 '{s}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zigbee::{unsecure_nwk, SEC_LEVEL_ENC_MIC32};

    fn ctx() -> VarContext {
        VarContext {
            key: Some([
                0xa1, 0x40, 0x35, 0x57, 0x84, 0xcc, 0xa8, 0x94, 0xa1, 0x40, 0x35, 0x57, 0x84, 0xcc,
                0xa8, 0x94,
            ]),
            pan: 0x0c84,
            channel: 11,
            src_short: 0x7fff,
            src_eui64: [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
            frame_counter: 1000,
            seq: 0,
            vars: HashMap::new(),
        }
    }

    #[test]
    fn scalars_and_literals() {
        let mut c = ctx();
        assert_eq!(
            resolve_line(&mut c, "pan={$pan} ch={$channel}").unwrap(),
            "pan=0c84 ch=11"
        );
        assert_eq!(resolve_line(&mut c, "src={$src}").unwrap(), "src=7fff");
        // a brace that isn't a token is literal
        assert_eq!(resolve_line(&mut c, "a {b} c").unwrap(), "a {b} c");
        assert_eq!(resolve_line(&mut c, "{$key}").unwrap().len(), 32);
    }

    #[test]
    fn fc_increments_and_persists() {
        let mut c = ctx();
        assert_eq!(resolve_line(&mut c, "{$fc} {$fc}").unwrap(), "1000 1001");
        assert_eq!(c.frame_counter, 1002, "counter advanced for the next run");
    }

    #[test]
    fn user_var_and_unknown() {
        let mut c = ctx();
        c.vars.insert("TARGET".into(), "abcd".into());
        assert_eq!(resolve_line(&mut c, "n={$TARGET}").unwrap(), "n=abcd");
        assert!(resolve_line(&mut c, "{$nope}").is_err());
    }

    #[test]
    fn zdp_expands_to_a_decryptable_frame() {
        let mut c = ctx();
        // The macro a user would write: HEX {$zdp active_ep abcd}
        let line = resolve_line(&mut c, "HEX {$zdp active_ep abcd}").unwrap();
        assert!(line.starts_with("HEX "), "keyword preserved");
        // parse the hex back into bytes (same as the HEX keyword does)
        let frame: Vec<u8> = line["HEX ".len()..]
            .split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect();
        // MAC FCF + the injector/target addresses
        assert_eq!(&frame[0..2], &[0x61, 0x88], "MAC data frame");
        assert_eq!(&frame[5..7], &[0xcd, 0xab], "MAC dst = target abcd");
        // strip the 9-byte MAC header, decrypt the NWK payload, confirm the ZDP req
        let key = c.key.unwrap();
        let aps = unsecure_nwk(&key, &frame[9..], 8, SEC_LEVEL_ENC_MIC32).unwrap();
        assert_eq!(&aps[2..4], &[0x05, 0x00], "cluster = Active_EP_req");
        assert_eq!(&aps[9..11], &[0xcd, 0xab], "ZDP target = abcd");
        // the frame counter advanced
        assert_eq!(c.frame_counter, 1001);
    }

    /// Not a unit test — emits the exact frame Sutra would inject for the
    /// bench network so an external dissector (tshark) can validate it.
    /// Run: cargo test --lib macrovars::tests::emit_bench_inject -- --ignored --nocapture
    #[test]
    #[ignore]
    fn emit_bench_inject() {
        let mut c = VarContext {
            key: Some([
                0xa1, 0x40, 0x35, 0x57, 0x84, 0xcc, 0xa8, 0x94, 0xa1, 0x40, 0x35, 0x57, 0x84, 0xcc,
                0xa8, 0x94,
            ]),
            pan: 0x0c84,
            channel: 11,
            src_short: 0x7fff,
            src_eui64: [0x02, 0x53, 0x55, 0x54, 0x52, 0x41, 0x00, 0x01], // "0253555452410001"
            frame_counter: 1,
            seq: 0,
            vars: HashMap::new(),
        };
        let line = resolve_line(&mut c, "{$zdp active_ep 0000}").unwrap();
        println!("INJECT_FRAME={}", line.replace(' ', ""));
    }

    #[test]
    fn var_directive_sets_and_uses() {
        let mut c = ctx();
        let text =
            "VAR node abcd\nVAR label n-{$node}\nHEX {$zdp active_ep {$node}}\nSTRING {$label}";
        let out = resolve_text(&mut c, text).unwrap();
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(lines[0], "#", "VAR line consumed to a comment");
        assert_eq!(lines[1], "#");
        assert!(lines[2].starts_with("HEX 61 88"), "zdp used {{$node}}");
        assert_eq!(
            lines[3], "STRING n-abcd",
            "nested {{$node}} inside VAR value"
        );
    }

    #[test]
    fn zcl_turn_on_a_light() {
        let mut c = ctx();
        // "turn the light at 0xabcd, endpoint 1, on"
        let line = resolve_line(&mut c, "HEX {$zcl abcd 1 onoff on}").unwrap();
        let frame: Vec<u8> = line["HEX ".len()..]
            .split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect();
        assert_eq!(&frame[5..7], &[0xcd, 0xab], "MAC dst = the light");
        let aps = unsecure_nwk(&c.key.unwrap(), &frame[9..], 8, SEC_LEVEL_ENC_MIC32).unwrap();
        assert_eq!(&aps[2..4], &[0x06, 0x00], "cluster = On/Off");
        assert_eq!(&aps[4..6], &[0x04, 0x01], "profile = HA");
        assert_eq!(aps[10], 0x01, "ZCL command = On");
    }

    #[test]
    fn zdp_simple_desc_needs_endpoint() {
        let mut c = ctx();
        assert!(resolve_line(&mut c, "HEX {$zdp simple_desc abcd}").is_err());
        let ok = resolve_line(&mut c, "HEX {$zdp simple_desc abcd 1}").unwrap();
        let frame: Vec<u8> = ok["HEX ".len()..]
            .split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect();
        let aps = unsecure_nwk(&c.key.unwrap(), &frame[9..], 8, SEC_LEVEL_ENC_MIC32).unwrap();
        assert_eq!(&aps[2..4], &[0x04, 0x00], "cluster = Simple_Desc_req");
        assert_eq!(aps[11], 0x01, "endpoint trails the target");
    }
}
