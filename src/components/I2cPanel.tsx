// I²C viewer — replaces the terminal while the DATA kind is i2c.
// Address scanner (chip grid), a master write/read form, and the live
// transaction log fed by the DATA-channel records (one mux frame = one record).
import { RefreshCw, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { i2cScan, i2cXfer, type I2cRecord } from "@/lib/skrit";

const hex = (n: number, w = 2) => n.toString(16).toUpperCase().padStart(w, "0");
const hexList = (b: number[]) => b.map((x) => hex(x)).join(" ");

function parseHexBytes(s: string): number[] | null {
  const t = s.trim();
  if (!t) return [];
  const parts = t.split(/[\s,]+/);
  const out: number[] = [];
  for (const p of parts) {
    const v = parseInt(p.replace(/^0x/i, ""), 16);
    if (Number.isNaN(v) || v < 0 || v > 255) return null;
    out.push(v);
  }
  return out;
}

export function I2cPanel({
  records,
  disabled,
  onClear,
}: {
  records: I2cRecord[];
  disabled?: boolean;
  onClear: () => void;
}) {
  const [found, setFound] = useState<number[] | null>(null);
  const [scanning, setScanning] = useState(false);
  const [addr, setAddr] = useState("3C");
  const [writeHex, setWriteHex] = useState("");
  const [readLen, setReadLen] = useState("0");
  const [err, setErr] = useState<string | null>(null);
  const logEnd = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    logEnd.current?.scrollIntoView({ block: "nearest" });
  }, [records.length]);

  async function scan() {
    setScanning(true);
    setErr(null);
    try {
      setFound(await i2cScan());
    } catch (e) {
      setErr(String(e));
    } finally {
      setScanning(false);
    }
  }

  async function transfer() {
    setErr(null);
    const a = parseInt(addr.replace(/^0x/i, ""), 16);
    const w = parseHexBytes(writeHex);
    const rl = parseInt(readLen, 10) || 0;
    if (Number.isNaN(a) || a > 0x7f || w === null || rl < 0 || rl > 200) {
      setErr("bad address / bytes / read length");
      return;
    }
    try {
      await i2cXfer(a, w, rl); // the reply also arrives as a DATA record below
    } catch (e) {
      setErr(String(e)); // NAK -> status 0x5
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 p-2">
      {/* scanner */}
      <div className="flex flex-wrap items-center gap-1.5">
        <Button size="sm" variant="outline" className="h-7 gap-1.5" disabled={disabled || scanning} onClick={scan}>
          <RefreshCw className={`size-3 ${scanning ? "animate-spin" : ""}`} />
          {scanning ? "Scanning…" : "Scan bus"}
        </Button>
        {found !== null &&
          (found.length ? (
            found.map((a) => (
              <button key={a} type="button" title={`use 0x${hex(a)}`}
                className="rounded border px-1.5 py-0.5 font-mono text-xs hover:bg-accent"
                onClick={() => setAddr(hex(a))}>
                0x{hex(a)}
              </button>
            ))
          ) : (
            <span className="text-xs text-muted-foreground">no devices ACKed</span>
          ))}
      </div>

      {/* master transfer */}
      <div className="flex flex-wrap items-end gap-2">
        <label className="text-xs">
          Addr (hex)
          <Input className="mt-1 h-7 w-20 font-mono" value={addr} onChange={(e) => setAddr(e.target.value)} />
        </label>
        <label className="text-xs">
          Write bytes (hex)
          <Input className="mt-1 h-7 w-48 font-mono" placeholder="00 FF …" value={writeHex}
            onChange={(e) => setWriteHex(e.target.value)} />
        </label>
        <label className="text-xs">
          Read len
          <Input className="mt-1 h-7 w-16 font-mono" value={readLen} onChange={(e) => setReadLen(e.target.value)} />
        </label>
        <Button size="sm" className="h-7" disabled={disabled} onClick={transfer}>
          Transfer
        </Button>
        <Button size="sm" variant="ghost" className="h-7 gap-1 text-muted-foreground" onClick={onClear}>
          <Trash2 className="size-3" /> Clear log
        </Button>
        {err && <span className="text-xs text-destructive">{err}</span>}
      </div>

      {/* transaction log (the DATA records) */}
      <div className="min-h-0 flex-1 overflow-auto rounded border">
        <table className="w-full text-left font-mono text-xs">
          <thead className="sticky top-0 bg-background text-muted-foreground">
            <tr>
              <th className="px-2 py-1 font-normal">t (ms)</th>
              <th className="px-2 py-1 font-normal">addr</th>
              <th className="px-2 py-1 font-normal">dir</th>
              <th className="px-2 py-1 font-normal">write</th>
              <th className="px-2 py-1 font-normal">read</th>
            </tr>
          </thead>
          <tbody>
            {records.map((r, i) => (
              <tr key={i} className="border-t">
                <td className="px-2 py-0.5 text-muted-foreground">{r.ts}</td>
                <td className="px-2 py-0.5">0x{hex(r.addr)}</td>
                <td className="px-2 py-0.5">
                  {r.nak ? (
                    <Badge variant="destructive" className="px-1 py-0 text-[10px]">NAK</Badge>
                  ) : r.read && r.w.length ? "W+R" : r.read ? "R" : "W"}
                </td>
                <td className="px-2 py-0.5">{hexList(r.w) || "—"}</td>
                <td className="px-2 py-0.5">{hexList(r.r) || "—"}</td>
              </tr>
            ))}
            {records.length === 0 && (
              <tr>
                <td colSpan={5} className="px-2 py-6 text-center text-muted-foreground">
                  No transactions yet — scan the bus or run a transfer. Every master
                  transfer (from any link) lands here.
                </td>
              </tr>
            )}
          </tbody>
        </table>
        <div ref={logEnd} />
      </div>
    </div>
  );
}
