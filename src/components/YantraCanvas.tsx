// Renders a .yantra control surface: widgets on a grid, each wired to a
// transport-agnostic action — a raw DATA write (UART/console, NOT the macro
// player so a leading "$" in NMEA isn't a $macro call), an I²C transfer, a
// device INVOKE command, or a CFG set. Readouts watch the live console stream
// and surface a regex capture. v1 = render + interact; scripts/plugins/visual-
// editor come later.
import { type CSSProperties, useEffect, useState } from "react";

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

  // active tab per `tabs` widget (keyed by its index); default = first pane
  const [activeTabs, setActiveTabs] = useState<Record<number, string>>({});
  const activeTabOf = (i: number) => activeTabs[i] ?? widgets[i].tabs?.[0]?.id;
  const tabVisible = (tabId: string): boolean => {
    const owner = widgets.findIndex((w) => w.type === "tabs" && (w.tabs ?? []).some((t) => t.id === tabId));
    return owner < 0 || activeTabOf(owner) === tabId; // orphan tab → always shown
  };

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

  // Coordinates are grid units (x/w in columns, y/h in rows). We position absolutely
  // (column % wide × ROW_H tall) so it matches the editor pixel-for-pixel. ROW_H must
  // equal the editor's. Anchors (responsive) resolve against the design size; CSS
  // positioning (left/right/top/bottom in px) then keeps fixed edges fixed as the
  // window resizes — no measurement needed. Default H=scale (today's %), V=top (fixed).
  const ROW_H = 56;
  const design = spec.design;
  const widgetStyle = (w: YantraWidget): CSSProperties => {
    const x = w.x ?? 0, y = w.y ?? 0, wc = w.w ?? 1, hc = w.h ?? 1;
    const aH = design ? w.anchorH ?? "scale" : "scale";
    const aV = design ? w.anchorV ?? "top" : "top";
    const dW = design?.w ?? 0;
    const dH = design?.h ?? 0;
    const cw = cols > 0 ? dW / cols : 0;
    const dx = x * cw, dw = wc * cw, dy = y * ROW_H, dh = hc * ROW_H;
    const s: CSSProperties = {};
    if (aH === "scale") { s.left = `${(x / cols) * 100}%`; s.width = `${(wc / cols) * 100}%`; }
    else if (aH === "left") { s.left = dx; s.width = dw; }
    else if (aH === "right") { s.right = dW - (dx + dw); s.width = dw; }
    else if (aH === "center") { s.left = `calc(${((dx + dw / 2) / dW) * 100}% - ${dw / 2}px)`; s.width = dw; }
    else { s.left = dx; s.right = dW - (dx + dw); } // stretch
    if (aV === "top") { s.top = dy; s.height = dh; }
    else if (aV === "scale") { s.top = `${(dy / dH) * 100}%`; s.height = `${(dh / dH) * 100}%`; }
    else if (aV === "bottom") { s.bottom = dH - (dy + dh); s.height = dh; }
    else if (aV === "middle") { s.top = `calc(${((dy + dh / 2) / dH) * 100}% - ${dh / 2}px)`; s.height = dh; }
    else { s.top = dy; s.bottom = dH - (dy + dh); } // stretch
    return s;
  };
  return (
    <div className="relative h-full overflow-auto p-1">
      {widgets.map((w, i) => {
        if (w.hidden) return null;
        if (w.tab && !tabVisible(w.tab)) return null;
        return (
          <div key={i} className="absolute" style={widgetStyle(w)}>
            {w.type === "tabs" ? (
              <div className="flex flex-wrap gap-1 border-b p-1">
                {(w.tabs ?? []).map((t) => (
                  <button key={t.id} type="button"
                    className={`rounded px-2 py-0.5 text-xs ${activeTabOf(i) === t.id ? "bg-primary text-primary-foreground" : "bg-muted/40 hover:bg-muted"}`}
                    onClick={() => setActiveTabs((m) => ({ ...m, [i]: t.id }))}>
                    {t.label}
                  </button>
                ))}
              </div>
            ) : (
              <Widget w={w} disabled={disabled} fire={fire} readout={readout} />
            )}
          </div>
        );
      })}
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
