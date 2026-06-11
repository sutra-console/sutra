// BLE sniffer view — the typed viewer for DATA kind ble-sniff. Two modes:
// a "Devices" table (grouped by advertiser address — what's nearby) and a live
// "Packets" stream. Fed by decoded sniff records from the DATA channel.
import { Download, Radio, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";

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
  onClear,
  onSavePcap,
}: {
  packets: BleSniffPacket[];
  onClear: () => void;
  onSavePcap: () => void;
}) {
  const [mode, setMode] = useState<"devices" | "packets">("devices");
  const logEnd = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (mode === "packets") logEnd.current?.scrollIntoView({ block: "nearest" });
  }, [packets.length, mode]);

  // group into devices (newest RSSI/name win; keep packet count + channels)
  const devices = new Map<string, Device>();
  packets.forEach((p, i) => {
    const key = p.addr || `(no addr) ${p.type}`;
    const d = devices.get(key);
    if (d) {
      d.count++;
      d.rssi = p.rssi;
      d.last = i;
      d.channels.add(p.channel);
      if (p.name) d.name = p.name;
      if (p.company) d.company = p.company;
    } else {
      devices.set(key, {
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
  const devList = [...devices.values()].sort((a, b) => b.last - a.last);

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
        <span className="text-xs text-muted-foreground">{packets.length} packets</span>
        <Button variant="outline" size="sm" className="ml-auto h-7 gap-1" disabled={!packets.length}
          title="Save the capture as a pcap (opens in Wireshark)" onClick={onSavePcap}>
          <Download className="size-3" /> Save .pcap
        </Button>
        <Button variant="ghost" size="sm" className="h-7 gap-1 text-muted-foreground" onClick={onClear}>
          <Trash2 className="size-3" /> Clear
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto rounded border">
        {mode === "devices" ? (
          <table className="w-full text-left font-mono text-xs">
            <thead className="sticky top-0 bg-background text-muted-foreground">
              <tr>
                <th className="px-2 py-1 font-normal">Address</th>
                <th className="px-2 py-1 font-normal">Name</th>
                <th className="px-2 py-1 font-normal">Company</th>
                <th className="px-2 py-1 font-normal">Type</th>
                <th className="px-2 py-1 font-normal">RSSI</th>
                <th className="px-2 py-1 font-normal">Ch</th>
                <th className="px-2 py-1 font-normal">#</th>
              </tr>
            </thead>
            <tbody>
              {devList.map((d) => (
                <tr key={d.addr || d.type} className="border-t">
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
                <th className="px-2 py-1 font-normal">Ch</th>
                <th className="px-2 py-1 font-normal">RSSI</th>
                <th className="px-2 py-1 font-normal">Type</th>
                <th className="px-2 py-1 font-normal">Address</th>
                <th className="px-2 py-1 font-normal">AdvData</th>
              </tr>
            </thead>
            <tbody>
              {packets.slice(-300).map((p, i) => (
                <tr key={i} className="border-t">
                  <td className="px-2 py-0.5 text-muted-foreground">{p.channel}</td>
                  <td className="px-2 py-0.5 tabular-nums">{p.rssi}</td>
                  <td className="px-2 py-0.5 text-muted-foreground">{p.type}</td>
                  <td className="px-2 py-0.5">{p.addr || "—"}</td>
                  <td className="px-2 py-0.5 text-muted-foreground">{p.payloadHex}</td>
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
        <div ref={logEnd} />
      </div>
    </div>
  );
}
