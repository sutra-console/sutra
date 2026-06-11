// IEEE 802.15.4 sniffer view — the typed viewer for DATA kind ieee802154
// (Zigbee/Thread). Two modes: a "Nodes" table (grouped by source short/ext
// address — who's on the network) and a live "Packets" stream. Fed by decoded
// 802.15.4 frame records from the DATA channel.
import { Download, Radio, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { type Ieee154Frame } from "@/lib/skrit";

interface Node {
  addr: string;
  rssi: number;
  count: number;
  channels: Set<number>;
  types: Set<string>;
  last: number;
}

export function Ieee154Panel({
  frames,
  total,
  onClear,
  onSavePcap,
}: {
  frames: Ieee154Frame[];
  total: number; // total received (the frames buffer is capped)
  onClear: () => void;
  onSavePcap: () => void;
}) {
  const [mode, setMode] = useState<"nodes" | "packets">("nodes");
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
      if (!f.src) return;
      const d = m.get(f.src);
      if (d) {
        d.count++;
        d.rssi = f.rssi;
        d.last = i;
        d.channels.add(f.channel);
        d.types.add(f.type);
      } else {
        m.set(f.src, {
          addr: f.src,
          rssi: f.rssi,
          count: 1,
          channels: new Set([f.channel]),
          types: new Set([f.type]),
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
  const shown = selected.size
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

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 p-2 text-foreground">
      <div className="flex items-center gap-2">
        <div className="flex overflow-hidden rounded border text-xs">
          {(["nodes", "packets"] as const).map((m) => (
            <button key={m} type="button"
              className={`px-2.5 py-1 capitalize ${mode === m ? "bg-accent" : "hover:bg-accent/50"}`}
              onClick={() => setMode(m)}>
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
        {selected.size > 0 && (
          <button type="button"
            className="inline-flex items-center gap-1 rounded border border-primary/60 px-1.5 py-0.5 text-[11px] text-primary hover:bg-accent"
            title="Frames are filtered to the selected nodes — click to clear"
            onClick={() => { setSelected(new Set()); setAnchor(null); }}>
            filter: {selected.size} · clear <X className="size-3" />
          </button>
        )}
        <Button variant="outline" size="sm" className="ml-auto h-7 gap-1" disabled={!frames.length}
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
                  <td className="px-2 py-0.5 text-muted-foreground">{[...d.types].join(", ")}</td>
                  <td className="px-2 py-0.5 tabular-nums">{d.rssi} dBm</td>
                  <td className="px-2 py-0.5 text-muted-foreground">{[...d.channels].sort((a, b) => a - b).join(",")}</td>
                  <td className="px-2 py-0.5 tabular-nums">{d.count}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
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
                <tr key={group ? `${f.src}|${f.dst}|${f.type}|${f.payloadHex}` : i} className="border-t">
                  {group && <td className="px-2 py-0.5 tabular-nums text-muted-foreground">{n}</td>}
                  <td className="px-2 py-0.5 text-muted-foreground">{f.channel}</td>
                  <td className="px-2 py-0.5 tabular-nums">{f.rssi}</td>
                  <td className="px-2 py-0.5 text-muted-foreground">{f.type}</td>
                  <td className="px-2 py-0.5">{f.src || "—"}</td>
                  <td className="px-2 py-0.5">{f.dst || "—"}</td>
                  <td className="px-2 py-0.5 text-muted-foreground">
                    {/* 802.15.4 payloads run long — cap the column so the table
                        can't grow wider than the viewport (which breaks scroll). */}
                    <div className="max-w-[24rem] truncate" title={f.payloadHex}>{f.payloadHex}</div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {frames.length === 0 && (
          <p className="px-2 py-6 text-center text-xs text-muted-foreground">
            Listening for IEEE 802.15.4 frames… Zigbee/Thread traffic on channels 11–26 will appear here.
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
