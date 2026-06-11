// BLE sniffer view — the typed viewer for DATA kind ble-sniff. Two modes:
// a "Devices" table (grouped by advertiser address — what's nearby) and a live
// "Packets" stream. Fed by decoded sniff records from the DATA channel.
import { Download, Radio, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { type BleSniffPacket } from "@/lib/skrit";

function rssiBars(rssi: number) {
  // -50 strong … -90 weak
  const pct = Math.max(0, Math.min(100, ((rssi + 100) / 60) * 100));
  const color = rssi > -60 ? "bg-green-500" : rssi > -75 ? "bg-yellow-500" : "bg-red-500";
  return (
    <span className="inline-flex items-center gap-1">
      <span className="h-1.5 w-10 overflow-hidden rounded bg-muted">
        <span className={`block h-full ${color}`} style={{ width: `${pct}%` }} />
      </span>
      <span className="tabular-nums text-muted-foreground">{rssi}</span>
    </span>
  );
}

interface Device {
  addr: string;
  name: string;
  company: string;
  type: string;
  rssi: number;
  count: number;
  channels: Set<number>;
  last: number; // arrival index (recency)
}

export function BleSnifferPanel({
  packets,
  total,
  onClear,
  onSavePcap,
}: {
  packets: BleSniffPacket[];
  total: number; // total received (the packets buffer is capped at ~2000)
  onClear: () => void;
  onSavePcap: () => void;
}) {
  const [mode, setMode] = useState<"devices" | "packets">("devices");
  const [group, setGroup] = useState(true); // collapse identical packets to the latest
  // sort the devices table by a stable key (default: address) so rows don't jump
  // around as packets stream in. Clicking a header toggles the key/direction.
  const [sort, setSort] = useState<{ key: "addr" | "name" | "rssi" | "count"; dir: 1 | -1 }>({
    key: "addr",
    dir: 1,
  });
  const [selected, setSelected] = useState<Set<string>>(new Set()); // device addrs filtering Packets
  const [anchor, setAnchor] = useState<string | null>(null); // shift-range anchor (by addr)
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // Sticky-follow: keep the packet stream pinned to the bottom ONLY while the
  // user is already near the bottom, so scrolling up to inspect stays put.
  // (scrollIntoView would yank every ancestor and pin the whole layout.)
  useEffect(() => {
    if (mode !== "packets" || group) return; // grouped rows update in place — don't chase
    const el = scrollRef.current;
    if (!el) return;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 60) el.scrollTop = el.scrollHeight;
  }, [packets.length, mode, group]);

  // group into devices (newest RSSI/name win; keep packet count + channels) —
  // only recompute when packets change, not on every sort/select re-render.
  const devices = useMemo(() => {
    const m = new Map<string, Device>();
    packets.forEach((p, i) => {
      const key = p.addr || `(no addr) ${p.type}`;
      const d = m.get(key);
      if (d) {
        d.count++;
        d.rssi = p.rssi;
        d.last = i;
        d.channels.add(p.channel);
        if (p.name) d.name = p.name;
        if (p.company) d.company = p.company;
      } else {
        m.set(key, {
          addr: p.addr,
          name: p.name,
          company: p.company,
          type: p.type,
          rssi: p.rssi,
          count: 1,
          channels: new Set([p.channel]),
          last: i,
        });
      }
    });
    return m;
  }, [packets]);
  const devList = [...devices.values()].sort((a, b) => {
    let c: number;
    switch (sort.key) {
      case "name": c = (a.name || a.addr).localeCompare(b.name || b.addr); break;
      case "rssi": c = a.rssi - b.rssi; break;
      case "count": c = a.count - b.count; break;
      default: c = a.addr.localeCompare(b.addr); break; // stable
    }
    return (c || a.addr.localeCompare(b.addr)) * sort.dir; // addr tiebreak keeps order stable
  });
  const toggleSort = (key: typeof sort.key) =>
    setSort((s) => (s.key === key ? { key, dir: (s.dir * -1) as 1 | -1 } : { key, dir: 1 }));
  const arrow = (key: typeof sort.key) => (sort.key === key ? (sort.dir === 1 ? " ▲" : " ▼") : "");

  // select devices (in the Devices table) to filter the Packets stream to them.
  // Plain click = just this row · Ctrl/⌘ = toggle one · Shift = range from anchor
  // (looked up by address, so it survives the live re-sort).
  function clickDevice(e: React.MouseEvent, addr: string) {
    if (!addr) return;
    if (e.shiftKey && anchor) {
      const ai = devList.findIndex((d) => d.addr === anchor);
      const ci = devList.findIndex((d) => d.addr === addr);
      if (ai >= 0 && ci >= 0) {
        const [lo, hi] = ai < ci ? [ai, ci] : [ci, ai];
        setSelected(new Set(devList.slice(lo, hi + 1).map((d) => d.addr).filter(Boolean)));
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
  const shownPackets = selected.size
    ? packets.filter((p) => selected.has(p.addr))
    : packets;
  // Packets view rows: collapse identical content (addr·type·AdvData) to the
  // latest occurrence with a count, or the raw tail.
  const packetRows: { p: BleSniffPacket; n: number }[] = group
    ? (() => {
        const m = new Map<string, { p: BleSniffPacket; n: number }>();
        for (const p of shownPackets) {
          const k = `${p.addr}|${p.type}|${p.payloadHex}`;
          const e = m.get(k);
          if (e) {
            e.n++;
            e.p = p;
          } else {
            m.set(k, { p, n: 1 });
          }
        }
        return [...m.values()];
      })()
    : shownPackets.slice(-300).map((p) => ({ p, n: 1 }));

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 p-2 text-foreground">
      <div className="flex items-center gap-2">
        <div className="flex overflow-hidden rounded border text-xs">
          {(["devices", "packets"] as const).map((m) => (
            <button key={m} type="button"
              className={`px-2.5 py-1 capitalize ${mode === m ? "bg-accent" : "hover:bg-accent/50"}`}
              onClick={() => setMode(m)}>
              {m}
            </button>
          ))}
        </div>
        <Badge variant="secondary" className="gap-1"><Radio className="size-3" /> {devList.length} devices</Badge>
        <span className="text-xs text-muted-foreground">{total.toLocaleString()} packets</span>
        {mode === "packets" && (
          <button type="button"
            className={`rounded border px-2 py-0.5 text-[11px] ${group ? "bg-accent" : "hover:bg-accent/50"}`}
            title="Collapse identical packets to the latest, with a count"
            onClick={() => setGroup((g) => !g)}>
            group identical
          </button>
        )}
        {selected.size > 0 && (
          <button type="button"
            className="inline-flex items-center gap-1 rounded border border-primary/60 px-1.5 py-0.5 text-[11px] text-primary hover:bg-accent"
            title="Packets are filtered to the selected devices — click to clear"
            onClick={() => { setSelected(new Set()); setAnchor(null); }}>
            filter: {selected.size} · clear <X className="size-3" />
          </button>
        )}
        <Button variant="outline" size="sm" className="ml-auto h-7 gap-1" disabled={!packets.length}
          title="Save the capture as a pcap (opens in Wireshark)" onClick={onSavePcap}>
          <Download className="size-3" /> Save .pcap
        </Button>
        <Button variant="ghost" size="sm" className="h-7 gap-1 text-muted-foreground" onClick={onClear}>
          <Trash2 className="size-3" /> Clear
        </Button>
      </div>

      <div ref={scrollRef} className="min-h-0 min-w-0 flex-1 overflow-auto rounded border">
        {mode === "devices" ? (
          <table className="w-full text-left font-mono text-xs">
            <thead className="sticky top-0 bg-background text-muted-foreground">
              <tr>
                <th className="cursor-pointer px-2 py-1 font-normal hover:text-foreground" onClick={() => toggleSort("addr")}>Address{arrow("addr")}</th>
                <th className="cursor-pointer px-2 py-1 font-normal hover:text-foreground" onClick={() => toggleSort("name")}>Name{arrow("name")}</th>
                <th className="px-2 py-1 font-normal">Company</th>
                <th className="px-2 py-1 font-normal">Type</th>
                <th className="cursor-pointer px-2 py-1 font-normal hover:text-foreground" onClick={() => toggleSort("rssi")}>RSSI{arrow("rssi")}</th>
                <th className="px-2 py-1 font-normal">Ch</th>
                <th className="cursor-pointer px-2 py-1 font-normal hover:text-foreground" onClick={() => toggleSort("count")}>#{arrow("count")}</th>
              </tr>
            </thead>
            <tbody>
              {devList.map((d) => (
                <tr key={d.addr || d.type}
                  className={`cursor-pointer select-none border-t ${selected.has(d.addr) ? "bg-primary/15" : "hover:bg-accent/40"}`}
                  title="Filter the Packets view: click = this device · Ctrl/⌘+click = toggle · Shift+click = range"
                  onClick={(e) => clickDevice(e, d.addr)}>
                  <td className="px-2 py-0.5">{d.addr || <span className="text-muted-foreground">—</span>}</td>
                  <td className="px-2 py-0.5 font-sans">{d.name || <span className="text-muted-foreground">·</span>}</td>
                  <td className="px-2 py-0.5 font-sans">{d.company || <span className="text-muted-foreground">·</span>}</td>
                  <td className="px-2 py-0.5 text-muted-foreground">{d.type}</td>
                  <td className="px-2 py-0.5">{rssiBars(d.rssi)}</td>
                  <td className="px-2 py-0.5 text-muted-foreground">{[...d.channels].sort().join(",")}</td>
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
                <th className="px-2 py-1 font-normal">Address</th>
                <th className="px-2 py-1 font-normal">AdvData</th>
              </tr>
            </thead>
            <tbody>
              {packetRows.map(({ p, n }, i) => (
                <tr key={group ? `${p.addr}|${p.type}|${p.payloadHex}` : i} className="border-t">
                  {group && <td className="px-2 py-0.5 tabular-nums text-muted-foreground">{n}</td>}
                  <td className="px-2 py-0.5 text-muted-foreground">{p.channel}</td>
                  <td className="px-2 py-0.5 tabular-nums">{p.rssi}</td>
                  <td className="px-2 py-0.5 text-muted-foreground">{p.type}</td>
                  <td className="px-2 py-0.5">{p.addr || "—"}</td>
                  <td className="whitespace-nowrap px-2 py-0.5 text-muted-foreground">{p.payloadHex}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {packets.length === 0 && (
          <p className="px-2 py-6 text-center text-xs text-muted-foreground">
            Listening for BLE advertising… nearby devices will appear here.
          </p>
        )}
        {packets.length > 0 && mode === "packets" && shownPackets.length === 0 && (
          <p className="px-2 py-6 text-center text-xs text-muted-foreground">
            No packets from the selected device(s) yet.
          </p>
        )}
      </div>
    </div>
  );
}
