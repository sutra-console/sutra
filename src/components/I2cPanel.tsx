// I²C viewer — replaces the terminal while the DATA kind is i2c. Three modes:
//   Controls — a masonry of device cards built from workspace defs (.sutra/i2c),
//              each register rendered as a live control (number/toggle/slider/
//              enum/button) that reads & writes over i2c_xfer.
//   Log      — the transaction log fed by the DATA records, annotated with def
//              register names where the address + register match.
//   Manual   — address scanner + a raw write/read transfer form.
import { RefreshCw, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Slider } from "@/components/ui/slider";
import {
  type I2cDef,
  type I2cReg,
  type I2cRecord,
  i2cReadReg,
  i2cScan,
  i2cWriteReg,
  i2cXfer,
} from "@/lib/skrit";

const hex = (n: number, w = 2) => n.toString(16).toUpperCase().padStart(w, "0");
const hexList = (b: number[]) => b.map((x) => hex(x)).join(" ");

function parseHexBytes(s: string): number[] | null {
  const t = s.trim();
  if (!t) return [];
  const out: number[] = [];
  for (const p of t.split(/[\s,]+/)) {
    const v = parseInt(p.replace(/^0x/i, ""), 16);
    if (Number.isNaN(v) || v < 0 || v > 255) return null;
    out.push(v);
  }
  return out;
}

// ---- a single register's live control ----
function RegControl({ addr, reg, disabled }: { addr: number; reg: I2cReg; disabled?: boolean }) {
  const [val, setVal] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const readable = (reg.access ?? "rw") !== "w";
  const writable = (reg.access ?? "rw") !== "r";

  async function read() {
    setBusy(true);
    try {
      setVal(await i2cReadReg(addr, reg));
    } catch {
      setVal(null);
    } finally {
      setBusy(false);
    }
  }
  async function write(v: number) {
    setBusy(true);
    try {
      await i2cWriteReg(addr, reg, v);
      setVal(v);
    } catch {
      /* surfaced in the log */
    } finally {
      setBusy(false);
    }
  }
  useEffect(() => {
    if (readable && reg.control !== "button" && !disabled) read();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [disabled]);

  const ctl = reg.control ?? "number";
  return (
    <div className="flex flex-col gap-1 border-t py-1.5 first:border-t-0">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium" title={reg.desc}>{reg.name}</span>
        <span className="font-mono text-[10px] text-muted-foreground">
          reg 0x{hex(reg.reg)}{val != null && ctl !== "button" ? ` = ${val}` : ""}
        </span>
      </div>
      {ctl === "toggle" ? (
        <Button size="sm" variant={val ? "default" : "outline"} className="h-7" disabled={disabled || busy || !writable}
          onClick={() => write(val ? 0 : 1)}>
          {val ? "On" : "Off"}
        </Button>
      ) : ctl === "slider" ? (
        <Slider min={reg.min ?? 0} max={reg.max ?? 255} value={[val ?? reg.min ?? 0]}
          disabled={disabled || !writable} onValueChange={([v]) => write(v)} />
      ) : ctl === "enum" ? (
        <div className="flex flex-wrap gap-1">
          {(reg.options ?? []).map((o) => (
            <Button key={o.value} size="sm" variant={val === o.value ? "default" : "outline"}
              className="h-6 px-2 text-[11px]" disabled={disabled || busy || !writable} onClick={() => write(o.value)}>
              {o.label}
            </Button>
          ))}
        </div>
      ) : ctl === "button" ? (
        <Button size="sm" variant="outline" className="h-7" disabled={disabled || busy || !writable}
          onClick={() => write(0)}>
          Send
        </Button>
      ) : (
        <div className="flex items-center gap-1">
          <Input className="h-7 w-24 font-mono" type="number" value={val ?? ""}
            onChange={(e) => setVal(e.target.value === "" ? null : Number(e.target.value))}
            disabled={disabled || !writable} />
          {writable && (
            <Button size="sm" variant="outline" className="h-7" disabled={disabled || busy || val == null}
              onClick={() => val != null && write(val)}>Write</Button>
          )}
          {readable && (
            <Button size="sm" variant="ghost" className="h-7 px-2 text-muted-foreground" disabled={disabled || busy}
              onClick={read}>Read</Button>
          )}
        </div>
      )}
    </div>
  );
}

export function I2cPanel({
  records,
  defs,
  present,
  disabled,
  onClear,
  onScan,
}: {
  records: I2cRecord[];
  defs: I2cDef[];
  present: Set<number>; // addresses seen in the last scan
  disabled?: boolean;
  onClear: () => void;
  onScan: (found: number[]) => void;
}) {
  const [mode, setMode] = useState<"controls" | "log" | "manual">(defs.length ? "controls" : "manual");
  const [scanning, setScanning] = useState(false);
  const [addr, setAddr] = useState("3C");
  const [writeHex, setWriteHex] = useState("");
  const [readLen, setReadLen] = useState("0");
  const [err, setErr] = useState<string | null>(null);
  const logEnd = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (mode === "log") logEnd.current?.scrollIntoView({ block: "nearest" });
  }, [records.length, mode]);

  async function scan() {
    setScanning(true);
    setErr(null);
    try {
      onScan(await i2cScan());
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
      await i2cXfer(a, w, rl);
    } catch (e) {
      setErr(String(e));
    }
  }

  // label a transaction with its def register, if any
  const labelOf = (rec: I2cRecord) => {
    const d = defs.find((x) => x.addr === rec.addr);
    const r = d?.registers.find((rg) => rg.reg === rec.w[0]);
    return d && r ? `${d.name}·${r.name}` : "";
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 p-2 text-foreground">
      <div className="flex items-center gap-2">
        <div className="flex overflow-hidden rounded border text-xs">
          {(["controls", "log", "manual"] as const).map((m) => (
            <button key={m} type="button"
              className={`px-2.5 py-1 capitalize ${mode === m ? "bg-accent" : "hover:bg-accent/50"}`}
              onClick={() => setMode(m)}>
              {m}
            </button>
          ))}
        </div>
        <Button size="sm" variant="outline" className="h-7 gap-1.5" disabled={disabled || scanning} onClick={scan}>
          <RefreshCw className={`size-3 ${scanning ? "animate-spin" : ""}`} /> Scan
        </Button>
        {present.size > 0 && <Badge variant="secondary">{present.size} on bus</Badge>}
        <span className="text-xs text-muted-foreground">{records.length} transactions</span>
        <Button variant="ghost" size="sm" className="ml-auto h-7 gap-1 text-muted-foreground" onClick={onClear}>
          <Trash2 className="size-3" /> Clear
        </Button>
      </div>
      {err && <span className="text-xs text-destructive">{err}</span>}

      {mode === "controls" ? (
        defs.length === 0 ? (
          <p className="px-2 py-6 text-center text-xs text-muted-foreground">
            No device definitions. Add JSON files to <code>.sutra/i2c/</code> in your workspace
            (one per device: address + named registers) — they render as controls here.
          </p>
        ) : (
          <div className="min-h-0 flex-1 overflow-auto">
            <div className="columns-1 gap-2 [column-fill:_balance] sm:columns-2">
              {defs.map((d) => (
                <div key={`${d.name}-${d.addr}`} className="mb-2 break-inside-avoid rounded border p-2">
                  <div className="mb-1 flex items-center justify-between">
                    <span className="text-sm font-semibold">{d.name}</span>
                    <span className="flex items-center gap-1.5 font-mono text-[10px] text-muted-foreground">
                      {present.has(d.addr) && <span className="size-1.5 rounded-full bg-green-500" title="ACKed on the bus" />}
                      0x{hex(d.addr)}
                    </span>
                  </div>
                  {d.registers.map((r) => (
                    <RegControl key={r.name} addr={d.addr} reg={r} disabled={disabled} />
                  ))}
                </div>
              ))}
            </div>
          </div>
        )
      ) : mode === "manual" ? (
        <div className="flex min-h-0 flex-1 flex-col gap-2">
          <div className="flex flex-wrap items-center gap-1.5">
            {[...present].sort((a, b) => a - b).map((a) => (
              <button key={a} type="button" className="rounded border px-1.5 py-0.5 font-mono text-xs hover:bg-accent"
                onClick={() => setAddr(hex(a))}>0x{hex(a)}</button>
            ))}
            {present.size === 0 && <span className="text-xs text-muted-foreground">Scan to list devices.</span>}
          </div>
          <div className="flex flex-wrap items-end gap-2">
            <label className="text-xs">Addr (hex)
              <Input className="mt-1 h-7 w-20 font-mono" value={addr} onChange={(e) => setAddr(e.target.value)} /></label>
            <label className="text-xs">Write bytes (hex)
              <Input className="mt-1 h-7 w-48 font-mono" placeholder="00 FF …" value={writeHex}
                onChange={(e) => setWriteHex(e.target.value)} /></label>
            <label className="text-xs">Read len
              <Input className="mt-1 h-7 w-16 font-mono" value={readLen} onChange={(e) => setReadLen(e.target.value)} /></label>
            <Button size="sm" className="h-7" disabled={disabled} onClick={transfer}>Transfer</Button>
          </div>
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto rounded border">
          <table className="w-full text-left font-mono text-xs">
            <thead className="sticky top-0 bg-background text-muted-foreground">
              <tr>
                <th className="px-2 py-1 font-normal">t</th>
                <th className="px-2 py-1 font-normal">addr</th>
                <th className="px-2 py-1 font-normal">dir</th>
                <th className="px-2 py-1 font-normal">write</th>
                <th className="px-2 py-1 font-normal">read</th>
                <th className="px-2 py-1 font-normal">label</th>
              </tr>
            </thead>
            <tbody>
              {records.map((r, i) => (
                <tr key={i} className="border-t">
                  <td className="px-2 py-0.5 text-muted-foreground">{r.ts}</td>
                  <td className="px-2 py-0.5">0x{hex(r.addr)}</td>
                  <td className="px-2 py-0.5">
                    {r.nak ? <Badge variant="destructive" className="px-1 py-0 text-[10px]">NAK</Badge>
                      : r.read && r.w.length ? "W+R" : r.read ? "R" : "W"}
                  </td>
                  <td className="px-2 py-0.5">{hexList(r.w) || "—"}</td>
                  <td className="px-2 py-0.5">{hexList(r.r) || "—"}</td>
                  <td className="px-2 py-0.5 font-sans text-muted-foreground">{labelOf(r)}</td>
                </tr>
              ))}
              {records.length === 0 && (
                <tr><td colSpan={6} className="px-2 py-6 text-center text-muted-foreground">
                  No transactions yet — use the controls or run a manual transfer.
                </td></tr>
              )}
            </tbody>
          </table>
          <div ref={logEnd} />
        </div>
      )}
    </div>
  );
}
