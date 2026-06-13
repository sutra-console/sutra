// Renders a .yantra control surface: widgets on a grid, each wired to a
// transport-agnostic action — a raw DATA write (UART/console, NOT the macro
// player so a leading "$" in NMEA isn't a $macro call), an I²C transfer, a
// device INVOKE command, or a CFG set. Readouts watch the live console stream
// and surface a regex capture. v1 = render + interact; scripts/plugins/visual-
// editor come later.
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  MSG, dataWrite, i2cXfer, invokeCommand, onData, sendCmd,
  type YantraAction, type YantraSpec, type YantraWidget,
} from "@/lib/skrit";

const enc = new TextEncoder();
const dec = new TextDecoder();

/** Dispatch a widget action over the right transport. `value` (slider) is
 *  substituted into a string action's {value}. */
async function runAction(a: YantraAction | undefined, value?: string): Promise<void> {
  if (a == null) return;
  const sub = (s: string) => (value === undefined ? s : s.replace(/\{value\}/g, value));
  if (typeof a === "string") return dataWrite(Array.from(enc.encode(sub(a))));
  if ("send" in a) return dataWrite(Array.from(enc.encode(sub(a.send))));
  if ("i2c" in a) {
    await i2cXfer(a.i2c.addr, a.i2c.write ?? [], a.i2c.read ?? 0);
    return;
  }
  if ("invoke" in a) {
    await invokeCommand(a.invoke.id, a.invoke.args ?? []);
    return;
  }
  if ("cfg" in a) {
    const bytes = a.cfg.bytes ?? Array.from(enc.encode(a.cfg.str ?? ""));
    await sendCmd(MSG.CFG_SET, [a.cfg.key, ...bytes]);
    return;
  }
}

export function YantraCanvas({
  spec,
  disabled,
}: {
  spec: YantraSpec;
  disabled?: boolean;
}) {
  const fire = (a: YantraAction | undefined, value?: string) => {
    runAction(a, value).catch(() => {});
  };
  const cols = spec.cols ?? 6;
  const widgets = spec.widgets ?? [];
  const hasReadout = widgets.some((w) => w.type === "readout");

  // Rolling console buffer for readout regex matches (only while readouts exist).
  const [consoleText, setConsoleText] = useState("");
  useEffect(() => {
    if (!hasReadout) return;
    let un: (() => void) | undefined;
    onData((bytes) => {
      const t = dec.decode(Uint8Array.from(bytes));
      setConsoleText((c) => (c + t).slice(-4000));
    }).then((u) => (un = u));
    return () => un?.();
  }, [hasReadout]);

  const readout = (re?: string): string => {
    if (!re) return "—";
    try {
      return consoleText.match(new RegExp(re))?.[1] ?? "—";
    } catch {
      return "bad regex";
    }
  };

  // Coordinates are grid units (x/w in columns, y/h in rows) for BOTH layouts —
  // grid mode keeps them whole, free mode allows fractions. We position absolutely
  // (column % wide × ROW_H tall) so it matches the editor pixel-for-pixel. ROW_H
  // must equal the editor's.
  const ROW_H = 56;
  return (
    <div className="relative h-full overflow-auto p-1">
      {widgets.map((w, i) => (
        <div
          key={i}
          className="absolute"
          style={{
            left: `${((w.x ?? 0) / cols) * 100}%`,
            top: (w.y ?? 0) * ROW_H,
            width: `${((w.w ?? 1) / cols) * 100}%`,
            height: (w.h ?? 1) * ROW_H,
          }}
        >
          <Widget w={w} disabled={disabled} fire={fire} readout={readout} />
        </div>
      ))}
    </div>
  );
}

function Widget({
  w,
  disabled,
  fire,
  readout,
}: {
  w: YantraWidget;
  disabled?: boolean;
  fire: (a: YantraAction | undefined, value?: string) => void;
  readout: (re?: string) => string;
}) {
  const [on, setOn] = useState(false);
  const [val, setVal] = useState((w.min ?? 0).toString());

  switch (w.type) {
    case "button":
      return (
        <Button className="h-full w-full" size="sm" disabled={disabled}
          title={w.help} onClick={() => fire(w.send)}>
          {w.label}
        </Button>
      );

    case "toggle":
      return (
        <Button className="h-full w-full" size="sm" variant={on ? "default" : "outline"}
          disabled={disabled} title={w.help}
          onClick={() => { const next = !on; setOn(next); fire(next ? w.on : w.off); }}>
          {w.label}: {on ? "on" : "off"}
        </Button>
      );

    case "slider":
      return (
        <div className="flex h-full flex-col justify-center rounded border bg-muted/20 px-2 py-1">
          <div className="flex justify-between text-[11px]">
            <span className="text-muted-foreground">{w.label}</span>
            <span className="font-mono tabular-nums">{val}</span>
          </div>
          <input type="range" disabled={disabled}
            min={w.min ?? 0} max={w.max ?? 100} step={w.step ?? 1} value={val}
            onChange={(e) => setVal(e.target.value)}
            onPointerUp={() => fire(w.send, val)}
            onKeyUp={() => fire(w.send, val)} />
        </div>
      );

    case "select":
      return (
        <div className="flex h-full flex-col justify-center rounded border bg-muted/20 px-2 py-1">
          <div className="text-[11px] text-muted-foreground">{w.label}</div>
          <div className="flex flex-wrap gap-1">
            {(w.options ?? []).map((o, i) => (
              <button key={i} type="button" disabled={disabled}
                className="rounded bg-primary/15 px-1.5 py-0.5 text-[11px] text-primary hover:bg-primary/25 disabled:opacity-40"
                onClick={() => fire(o.send)}>
                {o.label}
              </button>
            ))}
          </div>
        </div>
      );

    case "readout":
      return (
        <div className="flex h-full flex-col justify-center rounded border bg-muted/20 px-2 py-1" title={w.help}>
          <div className="text-[11px] text-muted-foreground">{w.label}</div>
          <div className="truncate font-mono text-lg tabular-nums">{readout(w.match)}</div>
        </div>
      );

    case "label":
      return <div className="flex h-full items-center text-xs font-medium">{w.label}</div>;

    default:
      return (
        <div className="flex h-full items-center justify-center rounded border border-dashed text-[10px] text-muted-foreground">
          {w.type}?
        </div>
      );
  }
}
