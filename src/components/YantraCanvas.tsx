// Renders a .yantra control surface: widgets on a grid, each wired to a
// transport-agnostic action — a raw DATA write (UART/console, NOT the macro
// player so a leading "$" in NMEA isn't a $macro call), an I²C transfer, a
// device INVOKE command, or a CFG set. Readouts watch the live console stream
// and surface a regex capture. v1 = render + interact; scripts/plugins/visual-
// editor come later.
import { type CSSProperties, useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  axisStyle, evalCell,
  type AnchorMode, type YantraAction, type YantraFrame, type YantraSpec, type YantraWidget,
} from "@/lib/skrit";
import { useYantraRuntime } from "@/hooks/useYantraRuntime";

export function YantraCanvas({ spec, disabled }: { spec: YantraSpec; disabled?: boolean }) {
  const rt = useYantraRuntime(spec, disabled);
  return (
    <div className="flex h-full flex-col">
      <div className="scroll-stable min-h-0 flex-1 overflow-auto">
        {/* inner surface = the content area (scrollbar gutter excluded); widgets'
            % resolves against this, matching the editor's measured surface. */}
        <div className="relative h-full">
          <CanvasNodes
            container="root" widgets={rt.widgets} frames={rt.frames}
            activeTabOf={rt.activeTabOf} setActiveTabs={rt.setActiveTabs}
            disabled={disabled} fire={rt.fire} valueOf={rt.valueOf} rowsOf={rt.rowsOf} publish={rt.publish} ovOf={rt.ovOf}
          />
        </div>
      </div>
      {rt.hasScripts && (
        <div className="shrink-0 border-t bg-muted/20 px-2 py-1">
          <div className="mb-0.5 flex items-center justify-between text-[10px] text-muted-foreground">
            <span>console — script print() / log()</span>
            <button type="button" className="hover:text-foreground" onClick={rt.clearLog}>clear</button>
          </div>
          <div className="max-h-24 overflow-auto font-mono text-[10px] leading-tight">
            {rt.scriptLog.length === 0 ? (
              <div className="italic text-muted-foreground/60">— no output yet —</div>
            ) : (
              rt.scriptLog.map((l, i) => (
                <div key={i} className={l.startsWith("!") ? "text-destructive" : "text-muted-foreground"}>{l}</div>
              ))
            )}
          </div>
        </div>
      )}
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
  container, widgets, frames, activeTabOf, setActiveTabs, disabled, fire, valueOf, rowsOf, publish, ovOf,
}: {
  container: string; // "root" | frame id | pane id
  widgets: YantraWidget[];
  frames: YantraFrame[];
  activeTabOf: (i: number) => string | undefined;
  setActiveTabs: (f: (m: Record<number, string>) => Record<number, string>) => void;
  disabled?: boolean;
  fire: (a: YantraAction | undefined, value?: string) => void;
  valueOf: (w: YantraWidget) => unknown;
  rowsOf: (w: YantraWidget) => unknown[];
  publish: (name: string, v: unknown) => void;
  ovOf: (w: YantraWidget) => Record<string, unknown> | undefined;
}) {
  const isRoot = container === "root";
  const childFrames = frames.filter((f) =>
    isRoot ? !f.parent && !f.tab : f.tab === container || (f.parent === container && !f.tab),
  );
  const childWidgetIdx = widgets
    .map((_, i) => i)
    .filter((i) => {
      const w = widgets[i];
      if (w.hidden || ovOf(w)?.hidden) return false; // static or script-hidden
      return isRoot ? !w.frame && !w.tab : w.tab === container || (w.frame === container && !w.tab);
    });

  const sub = (c: string) => (
    <CanvasNodes container={c} widgets={widgets} frames={frames} activeTabOf={activeTabOf}
      setActiveTabs={setActiveTabs} disabled={disabled} fire={fire} valueOf={valueOf} rowsOf={rowsOf} publish={publish} ovOf={ovOf} />
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
        const ov = ovOf(w);
        const bg = (ov?.color as string) ?? w.color;
        return (
          <div key={i} style={{ ...nodeStyle(w), ...(bg ? { background: bg } : {}) }}>
            <Widget w={w} disabled={disabled || !!ov?.disabled} fire={fire} value={ov?.value ?? valueOf(w)} rows={rowsOf(w)} publish={publish} ov={ov} />
          </div>
        );
      })}
    </>
  );
}

/** Render any bound value to display text ("—" for empty). */
const asText = (v: unknown): string =>
  v === undefined || v === null || v === "" ? "—" : typeof v === "string" ? v : String(v);
/** Coerce a bound value to on/off. */
const truthy = (v: unknown): boolean =>
  v === true || v === 1 || /^(1|on|true|yes|high)$/i.test(String(v ?? ""));

export function Widget({
  w,
  disabled,
  fire,
  value,
  rows,
  publish,
  ov,
}: {
  w: YantraWidget;
  disabled?: boolean;
  fire: (a: YantraAction | undefined, value?: string) => void;
  value?: unknown; // bound scalar (undefined = unbound → use internal state)
  rows?: unknown[]; // table row items
  publish?: (name: string, v: unknown) => void; // push live control state to the bus
  ov?: Record<string, unknown>; // script presentation overrides (label/fg/image/…)
}) {
  const [on, setOn] = useState(!!w.value);
  const [val, setVal] = useState((w.value ?? w.min ?? 0).toString());
  const bound = value !== undefined;
  // static field → script override for presentation
  const label = (ov?.label as string) ?? w.label;
  const fg = (ov?.fg as string) ?? w.fg;
  const img = (ov?.image as string) ?? w.image;
  const fgStyle = fg ? { color: fg } : undefined;
  // publish the initial control value once so emit exprs can reference it immediately
  useEffect(() => {
    if (!w.name || !publish) return;
    if (w.type === "slider") publish(w.name, Number(w.value ?? w.min ?? 0));
    else if (w.type === "toggle") publish(w.name, !!w.value);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  switch (w.type) {
    case "button":
      return (
        <Button className="h-full w-full" size="sm" disabled={disabled}
          title={w.help} onClick={() => fire(w.send)}>
          {label}
        </Button>
      );

    case "toggle": {
      const state = bound ? truthy(value) : on;
      return (
        <Button className="h-full w-full" size="sm" variant={state ? "default" : "outline"}
          disabled={disabled} title={w.help}
          onClick={() => {
            const next = !state;
            if (!bound) setOn(next);
            if (w.name && publish) publish(w.name, next);
            fire(next ? w.on : w.off);
          }}>
          {label}: {state ? "on" : "off"}
        </Button>
      );
    }

    case "slider": {
      const shown = bound ? asText(value) : val; // bound ⇒ display-driven (still draggable to fire)
      return (
        <div className="flex h-full flex-col justify-center rounded border bg-muted/20 px-2 py-1">
          <div className="flex justify-between text-[11px]">
            <span className="text-muted-foreground">{label}</span>
            <span className="font-mono tabular-nums">{shown}</span>
          </div>
          <input type="range" disabled={disabled}
            min={w.min ?? 0} max={w.max ?? 100} step={w.step ?? 1} value={shown}
            onChange={(e) => { setVal(e.target.value); if (w.name && publish) publish(w.name, Number(e.target.value)); }}
            onPointerUp={() => fire(w.send, shown)}
            onKeyUp={() => fire(w.send, shown)} />
        </div>
      );
    }

    case "table": {
      const cols = w.columns && w.columns.length ? w.columns : [{ label: "value" }];
      const list = rows ?? [];
      return (
        <div className="h-full overflow-auto rounded border bg-muted/20" title={w.help}>
          <table className="w-full border-collapse text-[11px]">
            <thead className="sticky top-0 bg-muted/60 backdrop-blur">
              <tr>
                {cols.map((c, ci) => (
                  <th key={ci} className="px-1.5 py-0.5 text-left font-medium text-muted-foreground">
                    {c.label ?? c.field ?? ci}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {list.map((item, ri) => (
                <tr key={ri} className="border-t border-border/60">
                  {cols.map((c, ci) => (
                    <td key={ci} className="truncate px-1.5 py-0.5 font-mono tabular-nums">
                      {asText(evalCell(c, item, ri))}
                    </td>
                  ))}
                </tr>
              ))}
              {list.length === 0 && (
                <tr><td className="px-1.5 py-1 text-muted-foreground" colSpan={cols.length}>—</td></tr>
              )}
            </tbody>
          </table>
        </div>
      );
    }

    case "select":
      return (
        <div className="flex h-full flex-col justify-center rounded border bg-muted/20 px-2 py-1">
          <div className="text-[11px] text-muted-foreground">{label}</div>
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
          <div className="text-[11px] text-muted-foreground">{label}</div>
          <div className="truncate font-mono text-lg tabular-nums" style={fgStyle}>{asText(value)}</div>
        </div>
      );

    case "label":
      return (
        <div className="flex h-full items-center text-xs font-medium" style={fgStyle}>
          {bound ? asText(value) : label}
        </div>
      );

    case "image":
      return (
        <img src={img ?? (typeof value === "string" ? value : undefined)} alt={label ?? ""}
          title={w.help} className="h-full w-full rounded object-contain" />
      );

    default:
      return (
        <div className="flex h-full items-center justify-center rounded border border-dashed text-[10px] text-muted-foreground">
          {w.type}?
        </div>
      );
  }
}
