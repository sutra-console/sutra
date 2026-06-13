// IEEE 802.15.4 sniffer view — the typed viewer for DATA kind ieee802154
// (Zigbee/Thread). Two modes: a "Nodes" table (grouped by source short/ext
// address — who's on the network) and a live "Packets" stream. Fed by decoded
// 802.15.4 frame records from the DATA channel.
import { Download, Radio, Trash2, X } from "lucide-react";
import { Fragment, useEffect, useMemo, useRef, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { type DecodedRow, type Ieee154Frame } from "@/lib/skrit";

const copyText = (s: string) => navigator.clipboard?.writeText(s).catch(() => {});
// The on-air MAC frame from a captured record (ts·ch·rssi·lqi·flags·len·psdu),
// minus the FCS — what you'd inject to replay it (the radio re-appends the FCS).
const frameMac = (f: Ieee154Frame): number[] => {
  const len = f.raw[8] ?? 0;
  return f.raw.slice(9, 9 + Math.max(0, len - 2));
};
const parseHex = (s: string): number[] =>
  (s.replace(/0x/gi, "").match(/[0-9a-fA-F]{2}/g) ?? []).map((b) => parseInt(b, 16));
const roleClass = (role: string): string =>
  role === "Coordinator" ? "text-amber-400"
  : role === "Router" ? "text-sky-400"
  : role === "End Device" ? "text-emerald-400"
  : "text-muted-foreground";
const fieldsText = (d: DecodedRow) => d.fields.map(([k, v]) => `${k}=${v}`).join("\n");
const rowText = (d: DecodedRow) => {
  const f = d.fields.map(([k, v]) => `${k}=${v}`).join(" ");
  return `#${d.num} ${d.protocol} | ${d.summary}${f ? " | " + f : ""}`;
};

// Color the protocol badge by upper-layer stack (from Wireshark's Protocol column).
function protoColor(p: string): string {
  if (/zigbee|zbee/i.test(p)) return "text-amber-400";
  if (/thread|6lowpan|mle|coap|matter/i.test(p)) return "text-sky-400";
  if (/ack|802\.15\.4|wpan/i.test(p)) return "text-muted-foreground";
  return "text-emerald-400";
}

interface Node {
  addr: string;
  rssi: number;
  count: number;
  channels: Set<number>;
  types: Set<string>;
  pans: Set<string>; // PAN ids this node appears on
  broadcasts: boolean; // ever sent a broadcast (Link Status etc. ⇒ a router)
  last: number;
}

// A passively-discovered node, flattened for persistence into the workspace
// network model (App merges these into networks[].nodes, grouped by PAN).
export interface NodeSnapshot {
  addr: string;
  role: string;
  pan: string;
  channels: number[];
  count: number;
}

// Infer the node's role from what we've *passively* seen it transmit — no key
// needed. 0x0000 is always the coordinator; a node that polls its parent
// (Data Request) is a sleepy end-device; one that broadcasts / beacons routes.
export function nodeRole(n: Pick<Node, "addr" | "types" | "broadcasts">): string {
  if (n.addr === "0x0000") return "Coordinator";
  if (n.types.has("Data Request")) return "End Device";
  if (n.broadcasts || n.types.has("Beacon")) return "Router";
  return "Node";
}

export function Ieee154Panel({
  frames,
  total,
  onClear,
  onSavePcap,
  canDecode,
  onDecode,
  onInject,
  onSaveNodes,
}: {
  frames: Ieee154Frame[];
  total: number; // total received (the frames buffer is capped)
  onClear: () => void;
  onSavePcap: () => void;
  canDecode?: boolean; // Wireshark/tshark present
  onDecode?: () => Promise<DecodedRow[]>; // dissect the current capture
  onInject?: (mac: number[]) => void; // transmit a MAC frame (no FCS); undefined if not connected
  onSaveNodes?: (nodes: NodeSnapshot[]) => void; // persist discovered nodes to the workspace network model
}) {
  const [mode, setMode] = useState<"nodes" | "packets" | "decoded">("nodes");
  const [decoded, setDecoded] = useState<DecodedRow[]>([]);
  const [decoding, setDecoding] = useState(false);
  const [expanded, setExpanded] = useState<Set<number>>(new Set()); // decoded rows showing their field tree
  const [decFilter, setDecFilter] = useState(""); // free-text filter on protocol/summary
  const [hideNoise, setHideNoise] = useState(true); // hide Ack / Data Request / Link Status chatter
  const [live, setLive] = useState(false); // auto re-decode as frames arrive
  const [decErr, setDecErr] = useState(""); // last decode error (shown, not thrown)
  const [ctxRow, setCtxRow] = useState<DecodedRow | null>(null); // right-clicked decoded row
  const [injectHex, setInjectHex] = useState(""); // a MAC frame to transmit, as hex
  // keep the latest onDecode + frame count in refs so the live interval (set up
  // once) always decodes the CURRENT capture, not a stale closure.
  const onDecodeRef = useRef(onDecode);
  onDecodeRef.current = onDecode;
  const framesCount = useRef(frames.length);
  framesCount.current = frames.length;
  const lastDecodedAt = useRef(0);

  async function runDecode() {
    if (!onDecodeRef.current || decoding) return;
    setDecoding(true);
    try {
      lastDecodedAt.current = framesCount.current;
      setDecoded(await onDecodeRef.current());
      setDecErr("");
    } catch (e) {
      setDecErr(String(e)); // surface, never throw (an uncaught reject under live = noise)
    } finally {
      setDecoding(false);
    }
  }

  // Live mode: re-dissect the current buffer every couple seconds when new frames
  // have arrived. (A pragmatic "streaming" decode — re-runs tshark on the capped
  // buffer; a true incremental pipeline would feed one long-lived tshark.)
  useEffect(() => {
    if (!live || mode !== "decoded" || !canDecode) return;
    const id = window.setInterval(() => {
      if (framesCount.current !== lastDecodedAt.current && !decoding) runDecode();
    }, 2000);
    return () => window.clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [live, mode, canDecode, decoding]);
  const [group, setGroup] = useState(true); // collapse identical packets to the latest
  const [sort, setSort] = useState<{ key: "addr" | "rssi" | "count"; dir: 1 | -1 }>({
    key: "addr",
    dir: 1,
  });
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [anchor, setAnchor] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (mode !== "packets" || group) return;
    const el = scrollRef.current;
    if (!el) return;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 60) el.scrollTop = el.scrollHeight;
  }, [frames.length, mode, group]);

  // group into nodes by source address (frames with no src — e.g. Acks — skip).
  const nodes = useMemo(() => {
    const m = new Map<string, Node>();
    frames.forEach((f, i) => {
      if (!f.src || f.tx) return; // skip our own injected frames — not discovered nodes
      const d = m.get(f.src);
      if (d) {
        d.count++;
        d.rssi = f.rssi;
        d.last = i;
        d.channels.add(f.channel);
        d.types.add(f.type);
        if (f.dstPan) d.pans.add(f.dstPan);
        if (f.dst === "Broadcast") d.broadcasts = true;
      } else {
        m.set(f.src, {
          addr: f.src,
          rssi: f.rssi,
          count: 1,
          channels: new Set([f.channel]),
          types: new Set([f.type]),
          pans: new Set(f.dstPan ? [f.dstPan] : []),
          broadcasts: f.dst === "Broadcast",
          last: i,
        });
      }
    });
    return m;
  }, [frames]);
  const nodeList = [...nodes.values()].sort((a, b) => {
    let c: number;
    switch (sort.key) {
      case "rssi": c = a.rssi - b.rssi; break;
      case "count": c = a.count - b.count; break;
      default: c = a.addr.localeCompare(b.addr); break;
    }
    return (c || a.addr.localeCompare(b.addr)) * sort.dir;
  });
  const saveNodes = () => {
    if (!onSaveNodes) return;
    onSaveNodes(
      nodeList.map((n) => ({
        addr: n.addr,
        role: nodeRole(n),
        pan: [...n.pans][0] ?? "",
        channels: [...n.channels].sort((a, b) => a - b),
        count: n.count,
      })),
    );
  };
  const toggleSort = (key: typeof sort.key) =>
    setSort((s) => (s.key === key ? { key, dir: (s.dir * -1) as 1 | -1 } : { key, dir: 1 }));
  const arrow = (key: typeof sort.key) => (sort.key === key ? (sort.dir === 1 ? " ▲" : " ▼") : "");

  function clickNode(e: React.MouseEvent, addr: string) {
    if (!addr) return;
    if (e.shiftKey && anchor) {
      const ai = nodeList.findIndex((d) => d.addr === anchor);
      const ci = nodeList.findIndex((d) => d.addr === addr);
      if (ai >= 0 && ci >= 0) {
        const [lo, hi] = ai < ci ? [ai, ci] : [ci, ai];
        setSelected(new Set(nodeList.slice(lo, hi + 1).map((d) => d.addr)));
      }
    } else if (e.ctrlKey || e.metaKey) {
      setSelected((s) => {
        const n = new Set(s);
        n.has(addr) ? n.delete(addr) : n.add(addr);
        return n;
      });
      setAnchor(addr);
    } else {
      setSelected(new Set([addr]));
      setAnchor(addr);
    }
  }
  // selected nodes filter the Packets stream to frames with that src OR dst.
  // Only compute in Packets mode — this group-map over the whole buffer ran on
  // every render (incl. live re-decodes + 150ms frame flushes), which is what
  // froze the UI under live decode.
  const shown =
    mode !== "packets" ? [] : selected.size
      ? frames.filter((f) => selected.has(f.src) || selected.has(f.dst))
      : frames;
  const packetRows: { f: Ieee154Frame; n: number }[] = group
    ? (() => {
        const m = new Map<string, { f: Ieee154Frame; n: number }>();
        for (const f of shown) {
          const k = `${f.src}|${f.dst}|${f.type}|${f.payloadHex}`;
          const e = m.get(k);
          if (e) {
            e.n++;
            e.f = f;
          } else {
            m.set(k, { f, n: 1 });
          }
        }
        // 802.15.4 frames rarely repeat byte-for-byte, so grouping collapses
        // little — cap the rendered rows so a busy network can't balloon the DOM.
        return [...m.values()].slice(-400);
      })()
    : shown.slice(-300).map((f) => ({ f, n: 1 }));

  // Decoded view: hide mesh/MAC chatter and/or free-text match on protocol/info.
  const noiseRe = /\back\b|data request|link status|beacon request|poll/i;
  const shownDecoded = decoded
    .filter((d) => {
      if (hideNoise && noiseRe.test(d.summary)) return false;
      if (decFilter && !`${d.protocol} ${d.summary}`.toLowerCase().includes(decFilter.toLowerCase()))
        return false;
      return true;
    })
    .slice(-600); // cap rendered rows so a big buffer (esp. under live) can't freeze the UI

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 p-2 text-foreground">
      <div className="flex items-center gap-2">
        <div className="flex overflow-hidden rounded border text-xs">
          {(["nodes", "packets", "decoded"] as const).map((m) => (
            <button key={m} type="button"
              className={`px-2.5 py-1 capitalize ${mode === m ? "bg-accent" : "hover:bg-accent/50"}`}
              onClick={() => { setMode(m); if (m === "decoded" && canDecode && !decoded.length) runDecode(); }}>
              {m}
            </button>
          ))}
        </div>
        <Badge variant="secondary" className="gap-1"><Radio className="size-3" /> {nodeList.length} nodes</Badge>
        <span className="text-xs text-muted-foreground">{total.toLocaleString()} frames</span>
        {mode === "packets" && (
          <button type="button"
            className={`rounded border px-2 py-0.5 text-[11px] ${group ? "bg-accent" : "hover:bg-accent/50"}`}
            title="Collapse identical frames to the latest, with a count"
            onClick={() => setGroup((g) => !g)}>
            group identical
          </button>
        )}
        {mode === "decoded" && canDecode && (
          <>
            <button type="button"
              className="rounded border px-2 py-0.5 text-[11px] hover:bg-accent/50 disabled:opacity-50"
              title="Re-run Wireshark dissection on the current capture"
              disabled={decoding || !frames.length}
              onClick={runDecode}>
              {decoding ? "decoding…" : "↻ decode"}
            </button>
            <button type="button"
              className={`rounded border px-2 py-0.5 text-[11px] ${live ? "bg-accent" : "hover:bg-accent/50"}`}
              title="Auto re-decode as new frames arrive"
              onClick={() => setLive((v) => !v)}>
              live
            </button>
            <button type="button"
              className={`rounded border px-2 py-0.5 text-[11px] ${hideNoise ? "bg-accent" : "hover:bg-accent/50"}`}
              title="Hide Ack / Data Request / Link Status mesh + MAC chatter"
              onClick={() => setHideNoise((v) => !v)}>
              hide noise
            </button>
            <input
              className="h-6 w-40 rounded border bg-transparent px-2 text-[11px] outline-none focus:border-primary"
              placeholder="filter protocol / info…"
              value={decFilter}
              spellCheck={false}
              onChange={(e) => setDecFilter(e.target.value)}
            />
          </>
        )}
        {selected.size > 0 && (
          <button type="button"
            className="inline-flex items-center gap-1 rounded border border-primary/60 px-1.5 py-0.5 text-[11px] text-primary hover:bg-accent"
            title="Frames are filtered to the selected nodes — click to clear"
            onClick={() => { setSelected(new Set()); setAnchor(null); }}>
            filter: {selected.size} · clear <X className="size-3" />
          </button>
        )}
        {onInject && (
          <span className="flex items-center gap-1" title="Transmit a MAC frame (hex, no FCS) on the current channel">
            <input
              className="h-6 w-44 rounded border bg-transparent px-2 font-mono text-[11px] outline-none focus:border-primary"
              placeholder="inject hex…"
              value={injectHex}
              spellCheck={false}
              onChange={(e) => setInjectHex(e.target.value)}
              onKeyDown={(e) => {
                if (e.key !== "Enter") return;
                const b = parseHex(injectHex);
                if (b.length) onInject(b);
              }}
            />
            <button type="button"
              className="rounded border px-2 py-0.5 text-[11px] hover:bg-accent/50 disabled:opacity-50"
              disabled={!parseHex(injectHex).length}
              onClick={() => { const b = parseHex(injectHex); if (b.length) onInject(b); }}>
              inject
            </button>
          </span>
        )}
        {mode === "nodes" && onSaveNodes && (
          <Button variant="outline" size="sm" className="ml-auto h-7 gap-1" disabled={!nodeList.length}
            title="Save these discovered nodes into the workspace network model (grouped by PAN)"
            onClick={saveNodes}>
            <Radio className="size-3" /> Save nodes
          </Button>
        )}
        <Button variant="outline" size="sm" className={`h-7 gap-1 ${mode === "nodes" && onSaveNodes ? "" : "ml-auto"}`} disabled={!frames.length}
          title="Save the capture as a pcap (opens in Wireshark)" onClick={onSavePcap}>
          <Download className="size-3" /> Save .pcap
        </Button>
        <Button variant="ghost" size="sm" className="h-7 gap-1 text-muted-foreground" onClick={onClear}>
          <Trash2 className="size-3" /> Clear
        </Button>
      </div>

      <div ref={scrollRef} className="min-h-0 min-w-0 flex-1 overflow-auto rounded border">
        {mode === "nodes" ? (
          <table className="w-full text-left font-mono text-xs">
            <thead className="sticky top-0 bg-background text-muted-foreground">
              <tr>
                <th className="cursor-pointer px-2 py-1 font-normal hover:text-foreground" onClick={() => toggleSort("addr")}>Address{arrow("addr")}</th>
                <th className="px-2 py-1 font-normal">Role</th>
                <th className="px-2 py-1 font-normal">PAN</th>
                <th className="px-2 py-1 font-normal">Frame types</th>
                <th className="cursor-pointer px-2 py-1 font-normal hover:text-foreground" onClick={() => toggleSort("rssi")}>RSSI{arrow("rssi")}</th>
                <th className="px-2 py-1 font-normal">Ch</th>
                <th className="cursor-pointer px-2 py-1 font-normal hover:text-foreground" onClick={() => toggleSort("count")}>#{arrow("count")}</th>
              </tr>
            </thead>
            <tbody>
              {nodeList.map((d) => (
                <tr key={d.addr}
                  className={`cursor-pointer select-none border-t ${selected.has(d.addr) ? "bg-primary/15" : "hover:bg-accent/40"}`}
                  title="Filter the Packets view: click = this node · Ctrl/⌘+click = toggle · Shift+click = range"
                  onClick={(e) => clickNode(e, d.addr)}>
                  <td className="px-2 py-0.5">{d.addr}</td>
                  <td className="px-2 py-0.5"><span className={roleClass(nodeRole(d))}>{nodeRole(d)}</span></td>
                  <td className="px-2 py-0.5 text-muted-foreground">{[...d.pans].join(",") || "—"}</td>
                  <td className="px-2 py-0.5 text-muted-foreground">{[...d.types].join(", ")}</td>
                  <td className="px-2 py-0.5 tabular-nums">{d.rssi} dBm</td>
                  <td className="px-2 py-0.5 text-muted-foreground">{[...d.channels].sort((a, b) => a - b).join(",")}</td>
                  <td className="px-2 py-0.5 tabular-nums">{d.count}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : mode === "packets" ? (
          <table className="w-full text-left font-mono text-xs">
            <thead className="sticky top-0 bg-background text-muted-foreground">
              <tr>
                {group && <th className="px-2 py-1 font-normal">#</th>}
                <th className="px-2 py-1 font-normal">Ch</th>
                <th className="px-2 py-1 font-normal">RSSI</th>
                <th className="px-2 py-1 font-normal">Type</th>
                <th className="px-2 py-1 font-normal">Src</th>
                <th className="px-2 py-1 font-normal">Dst</th>
                <th className="px-2 py-1 font-normal">Payload</th>
              </tr>
            </thead>
            <tbody>
              {packetRows.map(({ f, n }, i) => (
                <tr key={group ? `${f.src}|${f.dst}|${f.type}|${f.payloadHex}` : i}
                  className={`border-t ${f.tx ? "bg-primary/10" : ""}`}>
                  {group && <td className="px-2 py-0.5 tabular-nums text-muted-foreground">{n}</td>}
                  <td className="px-2 py-0.5 text-muted-foreground">{f.channel}</td>
                  <td className="px-2 py-0.5 tabular-nums">{f.tx ? "TX" : f.rssi}</td>
                  <td className="px-2 py-0.5 text-muted-foreground">
                    {f.tx && <span className="mr-1 rounded bg-primary/20 px-1 text-[10px] text-primary">→ sent</span>}
                    {f.type}
                  </td>
                  <td className="px-2 py-0.5">{f.src || "—"}</td>
                  <td className="px-2 py-0.5">{f.dst || "—"}</td>
                  <td className="whitespace-nowrap px-2 py-0.5 text-muted-foreground">{f.payloadHex}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          // Decoded: Wireshark (tshark) dissection of the capture — the real
          // upper-layer protocol (ZigBee/Thread/6LoWPAN/Matter) and a summary.
          // Right-click a row to copy; click a field value (when expanded) to copy it.
          <ContextMenu>
            <ContextMenuTrigger asChild>
              <table className="w-full text-left font-mono text-xs">
                <thead className="sticky top-0 bg-background text-muted-foreground">
                  <tr>
                    <th className="px-2 py-1 font-normal">#</th>
                    <th className="px-2 py-1 font-normal">Ch</th>
                    <th className="px-2 py-1 font-normal">Protocol</th>
                    <th className="px-2 py-1 font-normal">Summary</th>
                  </tr>
                </thead>
                <tbody>
                  {shownDecoded.map((d) => {
                    const f = frames[d.num - 1];
                    const open = expanded.has(d.num);
                    return (
                      <Fragment key={d.num}>
                        <tr
                          className="cursor-pointer select-none border-t hover:bg-accent/40"
                          title={d.fields.length ? "Click to show fields · right-click to copy" : "Right-click to copy"}
                          onContextMenu={() => setCtxRow(d)}
                          onClick={() => setExpanded((s) => {
                            const n = new Set(s);
                            n.has(d.num) ? n.delete(d.num) : n.add(d.num);
                            return n;
                          })}>
                          <td className="px-2 py-0.5 tabular-nums text-muted-foreground">
                            {d.fields.length ? (open ? "▾ " : "▸ ") : ""}{d.num}
                          </td>
                          <td className="px-2 py-0.5 text-muted-foreground">{f?.channel ?? "—"}</td>
                          <td className={`px-2 py-0.5 ${protoColor(d.protocol)}`}>{d.protocol}</td>
                          <td className="whitespace-nowrap px-2 py-0.5">{d.summary || "—"}</td>
                        </tr>
                        {open && d.fields.length > 0 && (
                          <tr className="bg-muted/30">
                            <td />
                            <td colSpan={3} className="px-2 py-1">
                              <div className="flex flex-wrap gap-x-4 gap-y-0.5">
                                {d.fields.map(([k, v]) => (
                                  <button key={k} type="button"
                                    className="text-left text-[11px] hover:text-primary"
                                    title="Click to copy value"
                                    onClick={() => copyText(v)}>
                                    <span className="text-muted-foreground">{k}</span>={v}
                                  </button>
                                ))}
                              </div>
                            </td>
                          </tr>
                        )}
                      </Fragment>
                    );
                  })}
                </tbody>
              </table>
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem disabled={!ctxRow} onSelect={() => ctxRow && copyText(ctxRow.summary)}>
                Copy summary
              </ContextMenuItem>
              <ContextMenuItem disabled={!ctxRow} onSelect={() => ctxRow && copyText(ctxRow.protocol)}>
                Copy protocol
              </ContextMenuItem>
              <ContextMenuItem disabled={!ctxRow?.fields.length} onSelect={() => ctxRow && copyText(fieldsText(ctxRow))}>
                Copy fields
              </ContextMenuItem>
              <ContextMenuItem disabled={!ctxRow} onSelect={() => ctxRow && copyText(rowText(ctxRow))}>
                Copy row
              </ContextMenuItem>
              <ContextMenuItem disabled={!ctxRow} onSelect={() => ctxRow && setDecFilter(ctxRow.protocol)}>
                Filter to “{ctxRow?.protocol}”
              </ContextMenuItem>
              {onInject && (
                <ContextMenuItem
                  disabled={!ctxRow}
                  onSelect={() => {
                    const f = ctxRow && frames[ctxRow.num - 1];
                    if (f) onInject(frameMac(f));
                  }}>
                  Replay frame (TX)
                </ContextMenuItem>
              )}
            </ContextMenuContent>
          </ContextMenu>
        )}
        {frames.length === 0 && mode !== "decoded" && (
          <p className="px-2 py-6 text-center text-xs text-muted-foreground">
            Listening for IEEE 802.15.4 frames… Zigbee/Thread traffic on channels 11–26 will appear here.
          </p>
        )}
        {mode === "decoded" && !canDecode && (
          <div className="px-4 py-8 text-center text-xs text-muted-foreground">
            <p className="font-medium text-foreground">Packet decoding needs Wireshark</p>
            <p className="mt-1">
              Zigbee / Thread / Matter dissection is handled by Wireshark's <code>tshark</code>.
              Install Wireshark, then (if it isn't auto-detected) set its path in{" "}
              <span className="text-foreground">Settings ▸ Packet decode</span>.
            </p>
            <p className="mt-1">
              Capture, Save&nbsp;.pcap, the Nodes/Packets views, and the{" "}
              <code>sutra-extcap</code> → Wireshark workflow all work without it.
            </p>
          </div>
        )}
        {mode === "decoded" && decErr && (
          <p className="px-2 py-2 text-center text-[11px] text-destructive">decode error: {decErr}</p>
        )}
        {mode === "decoded" && canDecode && !decoded.length && !decErr && (
          <p className="px-2 py-6 text-center text-xs text-muted-foreground">
            {decoding
              ? "Dissecting with Wireshark…"
              : frames.length === 0
                ? "Capture some 802.15.4 frames, then ↻ decode."
                : "Hit ↻ decode to dissect the capture, or turn on live."}
          </p>
        )}
        {mode === "decoded" && canDecode && decoded.length > 0 && shownDecoded.length === 0 && (
          <p className="px-2 py-6 text-center text-xs text-muted-foreground">
            {decoded.length} frames, all hidden by the filter
            {hideNoise && " / hide-noise"}.
          </p>
        )}
        {frames.length > 0 && mode === "packets" && shown.length === 0 && (
          <p className="px-2 py-6 text-center text-xs text-muted-foreground">
            No frames from the selected node(s) yet.
          </p>
        )}
      </div>
    </div>
  );
}
