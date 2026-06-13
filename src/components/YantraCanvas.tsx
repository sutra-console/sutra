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
  axisStyle, type AnchorMode, type YantraAction, type YantraFrame, type YantraSpec, type YantraWidget,
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
  const widgets = spec.widgets ?? [];
  const frames = spec.frames ?? [];
  const hasReadout = widgets.some((w) => w.type === "readout");

  // active tab per `tabs` widget (keyed by its index); default = first pane
  const [activeTabs, setActiveTabs] = useState<Record<number, string>>({});
  const activeTabOf = (i: number) => activeTabs[i] ?? widgets[i].tabs?.[0]?.id;

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
    <div className="scroll-stable relative h-full overflow-auto p-1">
      <CanvasNodes
        container="root" widgets={widgets} frames={frames}
        activeTabOf={activeTabOf} setActiveTabs={setActiveTabs}
        disabled={disabled} fire={fire} readout={readout}
      />
    </div>
  );
}

// x/y/w/h are relative to the parent container's content box; the per-axis anchor
// (scale/start/center/end/stretch) decides how they resolve. Defaults: H=scale, V=start.
function nodeStyle(n: YantraWidget | YantraFrame): CSSProperties {
  const aH = (n.anchorH ?? "scale") as AnchorMode;
  const aV = (n.anchorV ?? "start") as AnchorMode;
  return {
    position: "absolute",
    ...axisStyle(aH, n.x ?? 0, n.w ?? (aH === "scale" ? 25 : 100), "h"),
    ...axisStyle(aV, n.y ?? 0, n.h ?? (aV === "scale" ? 25 : 48), "v"),
  } as CSSProperties;
}

// Recursively render the children of one container (root | a frame id | a tab-pane id).
function CanvasNodes({
  container, widgets, frames, activeTabOf, setActiveTabs, disabled, fire, readout,
}: {
  container: string; // "root" | frame id | pane id
  widgets: YantraWidget[];
  frames: YantraFrame[];
  activeTabOf: (i: number) => string | undefined;
  setActiveTabs: (f: (m: Record<number, string>) => Record<number, string>) => void;
  disabled?: boolean;
  fire: (a: YantraAction | undefined, value?: string) => void;
  readout: (re?: string) => string;
}) {
  const isRoot = container === "root";
  const childFrames = frames.filter((f) =>
    isRoot ? !f.parent && !f.tab : f.tab === container || (f.parent === container && !f.tab),
  );
  const childWidgetIdx = widgets
    .map((_, i) => i)
    .filter((i) => {
      const w = widgets[i];
      if (w.hidden) return false;
      return isRoot ? !w.frame && !w.tab : w.tab === container || (w.frame === container && !w.tab);
    });

  const sub = (c: string) => (
    <CanvasNodes container={c} widgets={widgets} frames={frames} activeTabOf={activeTabOf}
      setActiveTabs={setActiveTabs} disabled={disabled} fire={fire} readout={readout} />
  );

  return (
    <>
      {childFrames.map((f) => (
        <div key={f.id} style={nodeStyle(f)} className={`rounded ${f.clip === false ? "" : "overflow-hidden"}`}>
          {sub(f.id)}
        </div>
      ))}
      {childWidgetIdx.map((i) => {
        const w = widgets[i];
        if (w.type === "tabs") {
          const active = activeTabOf(i);
          return (
            <div key={i} style={nodeStyle(w)} className="flex flex-col overflow-hidden rounded border bg-card">
              <div className="flex flex-wrap gap-1 border-b p-1">
                {(w.tabs ?? []).map((t) => (
                  <button key={t.id} type="button"
                    className={`rounded px-2 py-0.5 text-xs ${active === t.id ? "bg-primary text-primary-foreground" : "bg-muted/40 hover:bg-muted"}`}
                    onClick={() => setActiveTabs((m) => ({ ...m, [i]: t.id }))}>
                    {t.label}
                  </button>
                ))}
              </div>
              <div className="relative flex-1">{active && sub(active)}</div>
            </div>
          );
        }
        return (
          <div key={i} style={nodeStyle(w)}>
            <Widget w={w} disabled={disabled} fire={fire} readout={readout} />
          </div>
        );
      })}
    </>
  );
}

export function Widget({
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
