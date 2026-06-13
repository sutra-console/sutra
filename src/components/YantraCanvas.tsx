// Renders a .yantra control surface: widgets laid out on a grid, each wired to
// send raw text to the device (data_write — NOT the macro player, so a leading
// "$" in e.g. NMEA isn't mistaken for a $macro call). Readout widgets watch the
// live console stream and surface a regex capture. v1 = render + interact;
// scripts/plugins/visual-editor come later.
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { onData, type YantraSpec, type YantraWidget } from "@/lib/skrit";

const dec = new TextDecoder();

export function YantraCanvas({
  spec,
  disabled,
  onSend,
}: {
  spec: YantraSpec;
  disabled?: boolean;
  onSend: (text: string) => void;
}) {
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

  return (
    <div
      className="grid h-full content-start gap-2 overflow-auto p-1"
      style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}
    >
      {widgets.map((w, i) => (
        <div
          key={i}
          style={{
            gridColumn: `${(w.x ?? 0) + 1} / span ${w.w ?? 1}`,
            gridRow: `${(w.y ?? 0) + 1} / span ${w.h ?? 1}`,
          }}
        >
          <Widget w={w} disabled={disabled} onSend={onSend} readout={readout} />
        </div>
      ))}
    </div>
  );
}

function Widget({
  w,
  disabled,
  onSend,
  readout,
}: {
  w: YantraWidget;
  disabled?: boolean;
  onSend: (text: string) => void;
  readout: (re?: string) => string;
}) {
  const [on, setOn] = useState(false);
  const [val, setVal] = useState((w.min ?? 0).toString());

  switch (w.type) {
    case "button":
      return (
        <Button className="h-full w-full" size="sm" disabled={disabled}
          title={w.help} onClick={() => w.send && onSend(w.send)}>
          {w.label}
        </Button>
      );

    case "toggle":
      return (
        <Button className="h-full w-full" size="sm" variant={on ? "default" : "outline"}
          disabled={disabled} title={w.help}
          onClick={() => { const next = !on; setOn(next); onSend((next ? w.on : w.off) ?? ""); }}>
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
            onPointerUp={() => w.send && onSend(w.send.replace("{value}", val))}
            onKeyUp={() => w.send && onSend(w.send.replace("{value}", val))} />
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
                onClick={() => onSend(o.send)}>
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
