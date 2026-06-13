// Visual editor for a .yantra control surface: drag/resize widgets on a grid,
// edit their properties (incl. the transport-agnostic action), add/remove widgets,
// and save back to the .yantra file. Pairs with YantraCanvas (the read-only
// renderer); App switches between them with an "Edit" toggle.
import { useEffect, useMemo, useRef, useState } from "react";
import {
  Plus, Save, Undo2, Trash2, Move,
  AlignStartVertical, AlignCenterVertical, AlignEndVertical,
  AlignStartHorizontal, AlignCenterHorizontal, AlignEndHorizontal,
  AlignHorizontalDistributeCenter, AlignVerticalDistributeCenter,
} from "lucide-react";
import Moveable from "react-moveable";
import Selecto from "react-selecto";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import type { YantraAction, YantraSpec, YantraWidget } from "@/lib/skrit";

const ROW_H = 56; // px per grid row in the editor (renderer auto-sizes rows)
const WIDGET_TYPES = ["button", "toggle", "slider", "select", "readout", "label"] as const;

// ---- small helpers ----------------------------------------------------------

const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v));
const csvToBytes = (s: string): number[] =>
  s.split(/[\s,]+/).filter(Boolean).map((t) => parseInt(t, t.startsWith("0x") ? 16 : 10) || 0);
const bytesToCsv = (b?: number[]): string =>
  (b ?? []).map((n) => `0x${n.toString(16).padStart(2, "0")}`).join(" ");

function defaultWidget(type: string, y: number): YantraWidget {
  const base: YantraWidget = { type, label: type, x: 0, y, w: 2, h: 1 };
  switch (type) {
    case "button": return { ...base, send: "" };
    case "toggle": return { ...base, on: "", off: "" };
    case "slider": return { ...base, min: 0, max: 100, step: 1, send: "{value}" };
    case "select": return { ...base, w: 3, options: [{ label: "Option", send: "" }] };
    case "readout": return { ...base, w: 3, match: "" };
    default: return base;
  }
}

// ---- action editor (string | {i2c} | {invoke} | {cfg}) ----------------------

type ActionKind = "text" | "i2c" | "invoke" | "cfg";
const actionKind = (a: YantraAction | undefined): ActionKind => {
  if (a == null || typeof a === "string") return "text";
  if ("send" in a) return "text";
  if ("i2c" in a) return "i2c";
  if ("invoke" in a) return "invoke";
  return "cfg";
};
const actionText = (a: YantraAction | undefined): string =>
  a == null ? "" : typeof a === "string" ? a : "send" in a ? a.send : "";

function ActionEditor({
  label, value, onChange, hint,
}: {
  label: string;
  value: YantraAction | undefined;
  onChange: (a: YantraAction) => void;
  hint?: string;
}) {
  const kind = actionKind(value);
  const i2c = value && typeof value === "object" && "i2c" in value ? value.i2c : undefined;
  const inv = value && typeof value === "object" && "invoke" in value ? value.invoke : undefined;
  const cfg = value && typeof value === "object" && "cfg" in value ? value.cfg : undefined;

  return (
    <div className="flex flex-col gap-1.5 rounded border bg-muted/20 p-2">
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-medium text-muted-foreground">{label}</span>
        <Select value={kind} onValueChange={(k) => {
          if (k === "text") onChange("");
          else if (k === "i2c") onChange({ i2c: { addr: 0, write: [], read: 0 } });
          else if (k === "invoke") onChange({ invoke: { id: 0, args: [] } });
          else onChange({ cfg: { key: 0, str: "" } });
        }}>
          <SelectTrigger className="h-6 w-24 text-[11px]"><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem value="text">Text / UART</SelectItem>
            <SelectItem value="i2c">I²C</SelectItem>
            <SelectItem value="invoke">Invoke</SelectItem>
            <SelectItem value="cfg">CFG set</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {kind === "text" && (
        <Input className="h-7 font-mono text-[11px]" placeholder={hint ?? "bytes to send (\\r\\n allowed)"}
          value={actionText(value)} onChange={(e) => onChange(e.target.value)} />
      )}
      {kind === "i2c" && (
        <div className="flex gap-1">
          <Input className="h-7 w-20 text-[11px]" type="number" placeholder="addr"
            value={i2c?.addr ?? 0}
            onChange={(e) => onChange({ i2c: { addr: +e.target.value, write: i2c?.write ?? [], read: i2c?.read ?? 0 } })} />
          <Input className="h-7 flex-1 font-mono text-[11px]" placeholder="write bytes"
            value={bytesToCsv(i2c?.write)}
            onChange={(e) => onChange({ i2c: { addr: i2c?.addr ?? 0, write: csvToBytes(e.target.value), read: i2c?.read ?? 0 } })} />
          <Input className="h-7 w-16 text-[11px]" type="number" placeholder="read"
            value={i2c?.read ?? 0}
            onChange={(e) => onChange({ i2c: { addr: i2c?.addr ?? 0, write: i2c?.write ?? [], read: +e.target.value } })} />
        </div>
      )}
      {kind === "invoke" && (
        <div className="flex gap-1">
          <Input className="h-7 w-20 text-[11px]" type="number" placeholder="id"
            value={inv?.id ?? 0}
            onChange={(e) => onChange({ invoke: { id: +e.target.value, args: inv?.args ?? [] } })} />
          <Input className="h-7 flex-1 font-mono text-[11px]" placeholder="args"
            value={bytesToCsv(inv?.args)}
            onChange={(e) => onChange({ invoke: { id: inv?.id ?? 0, args: csvToBytes(e.target.value) } })} />
        </div>
      )}
      {kind === "cfg" && (
        <div className="flex gap-1">
          <Input className="h-7 w-20 text-[11px]" type="number" placeholder="key"
            value={cfg?.key ?? 0}
            onChange={(e) => onChange({ cfg: { key: +e.target.value, str: cfg?.str ?? "" } })} />
          <Input className="h-7 flex-1 text-[11px]" placeholder="value (string)"
            value={cfg?.str ?? ""}
            onChange={(e) => onChange({ cfg: { key: cfg?.key ?? 0, str: e.target.value } })} />
        </div>
      )}
    </div>
  );
}

// ---- the editor -------------------------------------------------------------

export function YantraEditor({
  file,
  spec,
  onSave,
  saving,
}: {
  file: string;
  spec: YantraSpec;
  onSave: (spec: YantraSpec) => void;
  saving?: boolean;
}) {
  const [draft, setDraft] = useState<YantraSpec>(() => clone(spec));
  const [selected, setSelected] = useState<number[]>([]);
  const [containerW, setContainerW] = useState(0);
  const [ready, setReady] = useState(false);
  const gridRef = useRef<HTMLDivElement>(null);
  const moveableRef = useRef<Moveable>(null);
  const widgetRefs = useRef<(HTMLDivElement | null)[]>([]);

  const cols = draft.cols ?? 6;
  const free = draft.layout === "free";
  const widgets = draft.widgets ?? [];
  widgetRefs.current.length = widgets.length;
  const cw = containerW > 0 ? containerW / cols : 80;
  const dirty = JSON.stringify(draft) !== JSON.stringify(spec);
  const sel = selected.length ? selected[0] : null;

  // px geometry of a widget (grid cells × cell size, or absolute pixels in free mode)
  const geom = (w: YantraWidget) =>
    free
      ? { left: w.x ?? 0, top: w.y ?? 0, width: w.w ?? 140, height: w.h ?? 48 }
      : { left: (w.x ?? 0) * cw, top: (w.y ?? 0) * ROW_H, width: (w.w ?? 1) * cw, height: (w.h ?? 1) * ROW_H };

  // re-seed when the file prop changes (App also remounts via key, but be safe)
  useEffect(() => { setDraft(clone(spec)); setSelected([]); }, [file]); // eslint-disable-line react-hooks/exhaustive-deps

  // measure the canvas so grid cells map to pixels (Moveable works in px)
  useEffect(() => {
    setReady(true);
    const el = gridRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setContainerW(el.clientWidth));
    ro.observe(el);
    setContainerW(el.clientWidth);
    return () => ro.disconnect();
  }, []);

  // keep Moveable's box synced when geometry changes from outside a gesture
  useEffect(() => { moveableRef.current?.updateRect(); }, [draft, containerW, free, selected]);

  // Delete/Backspace removes the selection (unless a text field is focused)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Delete" && e.key !== "Backspace") return;
      const ae = document.activeElement;
      if (ae && (ae.tagName === "INPUT" || ae.tagName === "TEXTAREA")) return;
      if (selected.length) {
        e.preventDefault();
        removeMany(selected);
        setSelected([]);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selected]); // eslint-disable-line react-hooks/exhaustive-deps

  const setWidget = (i: number, patch: Partial<YantraWidget>) =>
    setDraft((d) => {
      const ws = [...(d.widgets ?? [])];
      ws[i] = { ...ws[i], ...patch };
      return { ...d, widgets: ws };
    });
  const removeMany = (indices: number[]) =>
    setDraft((d) => ({ ...d, widgets: (d.widgets ?? []).filter((_, j) => !indices.includes(j)) }));
  const addWidget = (type: string) => {
    const newIdx = widgets.length;
    setDraft((d) => {
      const ws = d.widgets ?? [];
      const w = defaultWidget(type, 0);
      if (d.layout === "free") {
        const bottom = ws.reduce((m, x) => Math.max(m, (x.y ?? 0) + (x.h ?? 0)), 0);
        Object.assign(w, { x: 8, y: bottom + 8, w: type === "select" || type === "readout" ? 200 : 140, h: 48 });
      } else {
        w.y = ws.reduce((m, x) => Math.max(m, (x.y ?? 0) + (x.h ?? 1)), 0);
      }
      return { ...d, widgets: [...ws, w] };
    });
    setSelected([newIdx]);
  };

  // Flip grid⇄free, converting coordinates through the current cell size so
  // widgets keep their on-screen position instead of jumping.
  const toggleFree = () =>
    setDraft((d) => {
      const toFree = d.layout !== "free";
      const cwNow = containerW > 0 ? containerW / (d.cols ?? 6) : 80;
      const ws = (d.widgets ?? []).map((w) =>
        toFree
          ? {
              ...w,
              x: Math.round((w.x ?? 0) * cwNow), y: Math.round((w.y ?? 0) * ROW_H),
              w: Math.round((w.w ?? 1) * cwNow), h: Math.round((w.h ?? 1) * ROW_H),
            }
          : {
              ...w,
              x: Math.round((w.x ?? 0) / cwNow), y: Math.round((w.y ?? 0) / ROW_H),
              w: Math.max(1, Math.round((w.w ?? cwNow) / cwNow)), h: Math.max(1, Math.round((w.h ?? ROW_H) / ROW_H)),
            },
      );
      return { ...d, layout: toFree ? "free" : "grid", widgets: ws };
    });

  // Read the final DOM geometry of the dragged/resized widgets back into the spec
  // (cells in grid mode, pixels in free mode), then clear the gesture transforms.
  const commit = (indices: number[]) => {
    const cont = gridRef.current;
    if (!cont) return;
    const cr = cont.getBoundingClientRect();
    const cwNow = containerW > 0 ? containerW / cols : 80;
    setDraft((d) => {
      const ws = [...(d.widgets ?? [])];
      for (const i of indices) {
        const el = widgetRefs.current[i];
        if (!el) continue;
        const r = el.getBoundingClientRect();
        const left = r.left - cr.left + cont.scrollLeft;
        const top = r.top - cr.top + cont.scrollTop;
        ws[i] = d.layout === "free"
          ? { ...ws[i], x: Math.max(0, Math.round(left)), y: Math.max(0, Math.round(top)), w: Math.round(r.width), h: Math.round(r.height) }
          : {
              ...ws[i],
              x: Math.max(0, Math.round(left / cwNow)),
              y: Math.max(0, Math.round(top / ROW_H)),
              w: Math.max(1, Math.round(r.width / cwNow)),
              h: Math.max(1, Math.round(r.height / ROW_H)),
            };
      }
      return { ...d, widgets: ws };
    });
    for (const i of indices) {
      const el = widgetRefs.current[i];
      if (el) el.style.transform = "";
    }
  };

  // --- align / distribute the multi-selection (operates on spec units —
  //     cells in grid mode, pixels in free; the math is unit-agnostic) ---------
  type Box = { minX: number; minY: number; maxR: number; maxB: number; cx: number; cy: number };
  const alignSelected = (fn: (w: YantraWidget, b: Box) => Partial<YantraWidget>) =>
    setDraft((d) => {
      const ws = [...(d.widgets ?? [])];
      const picks = selected.map((i) => ws[i]).filter(Boolean);
      if (picks.length < 2) return d;
      const minX = Math.min(...picks.map((w) => w.x ?? 0));
      const minY = Math.min(...picks.map((w) => w.y ?? 0));
      const maxR = Math.max(...picks.map((w) => (w.x ?? 0) + (w.w ?? 1)));
      const maxB = Math.max(...picks.map((w) => (w.y ?? 0) + (w.h ?? 1)));
      const b: Box = { minX, minY, maxR, maxB, cx: (minX + maxR) / 2, cy: (minY + maxB) / 2 };
      for (const i of selected) {
        const w = ws[i];
        if (w) ws[i] = { ...w, ...fn(w, b) };
      }
      return { ...d, widgets: ws };
    });
  const distribute = (axis: "h" | "v") =>
    setDraft((d) => {
      const ws = [...(d.widgets ?? [])];
      const idxs = selected.filter((i) => ws[i]);
      if (idxs.length < 3) return d;
      const pos = (w: YantraWidget) => (axis === "h" ? w.x ?? 0 : w.y ?? 0);
      const size = (w: YantraWidget) => (axis === "h" ? w.w ?? 1 : w.h ?? 1);
      const sorted = [...idxs].sort((a, c) => pos(ws[a]) - pos(ws[c]));
      const start = pos(ws[sorted[0]]);
      const last = ws[sorted[sorted.length - 1]];
      const end = pos(last) + size(last);
      const totalSize = sorted.reduce((s, i) => s + size(ws[i]), 0);
      const gap = (end - start - totalSize) / (sorted.length - 1);
      let cursor = start;
      for (const i of sorted) {
        const w = ws[i];
        const np = Math.max(0, Math.round(cursor));
        ws[i] = axis === "h" ? { ...w, x: np } : { ...w, y: np };
        cursor += size(w) + gap;
      }
      return { ...d, widgets: ws };
    });

  // shared align/distribute actions (used by both the floating toolbar and the panel)
  const alignActions = [
    { key: "l", label: "Align left", Icon: AlignStartVertical, run: () => alignSelected((_, b) => ({ x: Math.round(b.minX) })) },
    { key: "c", label: "Align center", Icon: AlignCenterVertical, run: () => alignSelected((w, b) => ({ x: Math.round(b.cx - (w.w ?? 1) / 2) })) },
    { key: "r", label: "Align right", Icon: AlignEndVertical, run: () => alignSelected((w, b) => ({ x: Math.round(b.maxR - (w.w ?? 1)) })) },
    { key: "t", label: "Align top", Icon: AlignStartHorizontal, run: () => alignSelected((_, b) => ({ y: Math.round(b.minY) })) },
    { key: "m", label: "Align middle", Icon: AlignCenterHorizontal, run: () => alignSelected((w, b) => ({ y: Math.round(b.cy - (w.h ?? 1) / 2) })) },
    { key: "b", label: "Align bottom", Icon: AlignEndHorizontal, run: () => alignSelected((w, b) => ({ y: Math.round(b.maxB - (w.h ?? 1)) })) },
  ];
  const distActions = [
    { key: "dh", label: "Distribute horizontally", Icon: AlignHorizontalDistributeCenter, run: () => distribute("h") },
    { key: "dv", label: "Distribute vertically", Icon: AlignVerticalDistributeCenter, run: () => distribute("v") },
  ];

  // top-left corner (px, canvas coords) of the multi-selection's bounding box,
  // so a mini align/distribute toolbar can pin to the group outline.
  const groupBox = useMemo(() => {
    if (selected.length < 2) return null;
    const gs = selected.map((i) => widgets[i]).filter(Boolean).map(geom);
    if (!gs.length) return null;
    return { left: Math.min(...gs.map((g) => g.left)), top: Math.min(...gs.map((g) => g.top)) };
  }, [selected, draft, cw, free]); // eslint-disable-line react-hooks/exhaustive-deps

  // sibling elements (for alignment/snap guidelines)
  const guidelines = useMemo(
    () => widgetRefs.current.filter((el, i) => !!el && !selected.includes(i)) as HTMLElement[],
    [selected, draft, containerW], // eslint-disable-line react-hooks/exhaustive-deps
  );
  const targets = useMemo(
    () => selected.map((i) => widgetRefs.current[i]).filter(Boolean) as HTMLElement[],
    [selected, draft, containerW], // eslint-disable-line react-hooks/exhaustive-deps
  );

  const rows = Math.max(4, widgets.reduce((m, w) => Math.max(m, (w.y ?? 0) + (w.h ?? 1)), 0) + 1);

  return (
    <div className="flex h-full min-h-0 gap-3">
      {/* canvas */}
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <div className="flex items-center gap-1.5">
          <span className="mr-1 text-[11px] text-muted-foreground">Add:</span>
          {WIDGET_TYPES.map((t) => (
            <Button key={t} size="sm" variant="outline" className="h-7 gap-1 px-2 text-[11px]"
              onClick={() => addWidget(t)}>
              <Plus className="size-3" /> {t}
            </Button>
          ))}
          <Button size="sm" variant={free ? "default" : "outline"} className="h-7 gap-1 px-2 text-[11px]"
            title="Freeflow: place widgets anywhere (pixels) instead of snapping to the grid"
            onClick={toggleFree}>
            <Move className="size-3" /> Freeflow
          </Button>
          <div className="ml-auto flex items-center gap-1.5">
            {dirty && <span className="text-[11px] text-amber-600 dark:text-amber-400">unsaved</span>}
            <Button size="sm" variant="outline" className="h-7 gap-1 px-2 text-[11px]"
              disabled={!dirty} onClick={() => { setDraft(clone(spec)); setSelected([]); }}>
              <Undo2 className="size-3" /> Revert
            </Button>
            <Button size="sm" className="h-7 gap-1 px-2 text-[11px]"
              disabled={!dirty || saving} onClick={() => onSave(draft)}>
              <Save className="size-3" /> Save
            </Button>
          </div>
        </div>

        <div
          ref={gridRef}
          className="yantra-canvas relative flex-1 overflow-auto rounded border bg-muted/10"
          style={{
            backgroundSize: free ? undefined : `${cw}px ${ROW_H}px`,
            backgroundImage: free
              ? undefined
              : "linear-gradient(to right, hsl(var(--border)/0.5) 1px, transparent 1px), linear-gradient(to bottom, hsl(var(--border)/0.5) 1px, transparent 1px)",
            minHeight: free ? undefined : rows * ROW_H,
          }}
        >
          {widgets.map((w, i) => (
            <div
              key={i}
              data-idx={i}
              ref={(el) => { widgetRefs.current[i] = el; }}
              className={`yantra-widget absolute select-none rounded border bg-card text-[11px] shadow-sm ${
                selected.includes(i) ? "ring-2 ring-primary" : ""
              }`}
              style={{ ...geom(w), padding: 4 }}
            >
              <div className="flex h-full flex-col overflow-hidden">
                <span className="truncate font-medium">{w.label || w.type}</span>
                <span className="truncate text-[10px] text-muted-foreground">{w.type}</span>
              </div>
            </div>
          ))}

          {/* mini align/distribute toolbar pinned to the group's outline */}
          {groupBox && (
            <div
              className="absolute z-[60] flex flex-col gap-0.5 rounded-md border bg-popover/95 p-0.5 shadow-md backdrop-blur"
              style={{ left: Math.max(2, groupBox.left - 30), top: Math.max(0, groupBox.top) }}
              onPointerDown={(e) => e.stopPropagation()}
            >
              {alignActions.map((a) => (
                <button key={a.key} type="button" title={a.label}
                  className="flex size-6 items-center justify-center rounded hover:bg-accent"
                  onClick={a.run}>
                  <a.Icon className="size-3.5" />
                </button>
              ))}
              <div className="mx-auto my-0.5 h-px w-4 bg-border" />
              {distActions.map((a) => (
                <button key={a.key} type="button" title={a.label} disabled={selected.length < 3}
                  className="flex size-6 items-center justify-center rounded hover:bg-accent disabled:opacity-30"
                  onClick={a.run}>
                  <a.Icon className="size-3.5" />
                </button>
              ))}
            </div>
          )}

          {ready && (
            <Moveable
              ref={moveableRef}
              target={targets}
              draggable
              resizable
              rotatable={false}
              snappable
              elementGuidelines={guidelines}
              snapGridWidth={free ? undefined : cw}
              snapGridHeight={free ? undefined : ROW_H}
              bounds={{ left: 0, top: 0, position: "css" }}
              throttleDrag={0}
              throttleResize={0}
              onDrag={({ target, transform }) => { (target as HTMLElement).style.transform = transform; }}
              onDragEnd={() => commit(selected)}
              onDragGroup={({ events }) => events.forEach((ev) => { (ev.target as HTMLElement).style.transform = ev.transform; })}
              onDragGroupEnd={() => commit(selected)}
              onResize={({ target, width, height, drag }) => {
                const el = target as HTMLElement;
                el.style.width = `${width}px`;
                el.style.height = `${height}px`;
                el.style.transform = drag.transform;
              }}
              onResizeEnd={() => commit(selected)}
              onResizeGroup={({ events }) => events.forEach((ev) => {
                const el = ev.target as HTMLElement;
                el.style.width = `${ev.width}px`;
                el.style.height = `${ev.height}px`;
                el.style.transform = ev.drag.transform;
              })}
              onResizeGroupEnd={() => commit(selected)}
            />
          )}
          {ready && (
            <Selecto
              dragContainer={gridRef.current}
              selectableTargets={[".yantra-widget"]}
              hitRate={0}
              selectByClick
              selectFromInside={false}
              toggleContinueSelect={["shift"]}
              onDragStart={(e) => {
                const t = e.inputEvent.target as HTMLElement;
                const mv = moveableRef.current;
                if (mv?.isMoveableElement(t)) { e.stop(); return; }
                if (selected.some((i) => { const el = widgetRefs.current[i]; return !!el && (el === t || el.contains(t)); })) {
                  e.stop();
                }
              }}
              onSelectEnd={(e) => {
                const idxs = e.selected
                  .map((el) => Number((el as HTMLElement).dataset.idx))
                  .filter((n) => !Number.isNaN(n));
                setSelected(idxs);
              }}
            />
          )}
        </div>
      </div>

      {/* property panel */}
      <div className="w-64 shrink-0 overflow-auto rounded border bg-muted/10 p-3">
        {selected.length > 1 ? (
          <div className="flex flex-col gap-3">
            <div className="flex items-center justify-between">
              <div className="text-sm font-medium">{selected.length} selected</div>
              <Button size="sm" variant="ghost" className="h-6 px-1 text-destructive"
                title="Delete selected" onClick={() => { removeMany(selected); setSelected([]); }}>
                <Trash2 className="size-3.5" />
              </Button>
            </div>
            <div>
              <div className="mb-1 text-[11px] text-muted-foreground">Align</div>
              <div className="grid grid-cols-3 gap-1">
                {alignActions.map((a) => (
                  <AlignBtn key={a.key} label={a.label} icon={a.Icon} onClick={a.run} />
                ))}
              </div>
            </div>
            <div>
              <div className="mb-1 text-[11px] text-muted-foreground">Distribute</div>
              <div className="grid grid-cols-2 gap-1">
                {distActions.map((a) => (
                  <AlignBtn key={a.key} label={a.label} icon={a.Icon}
                    disabled={selected.length < 3} onClick={a.run} />
                ))}
              </div>
            </div>
            <p className="text-[10px] text-muted-foreground">
              Distribute needs 3+ widgets. Shift-click to add/remove from the selection.
            </p>
          </div>
        ) : sel == null || !widgets[sel] ? (
          <div className="flex flex-col gap-2">
            <div className="text-sm font-medium">Surface</div>
            <Field label="Name">
              <Input className="h-7 text-xs" value={draft.name ?? ""}
                onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))} />
            </Field>
            <Field label="Description">
              <Textarea className="min-h-12 text-xs" value={draft.description ?? ""}
                onChange={(e) => setDraft((d) => ({ ...d, description: e.target.value }))} />
            </Field>
            <Field label="Columns">
              <Input className="h-7 w-20 text-xs" type="number" min={1} max={24} value={cols}
                onChange={(e) => setDraft((d) => ({ ...d, cols: Math.max(1, +e.target.value) }))} />
            </Field>
            <p className="text-[10px] text-muted-foreground">
              Click a widget to edit it (shift-click or marquee for many); drag to move, handles to resize.
            </p>
          </div>
        ) : (
          <WidgetProps
            w={widgets[sel]}
            onChange={(p) => setWidget(sel, p)}
            onDelete={() => { removeMany([sel]); setSelected([]); }}
          />
        )}
      </div>
    </div>
  );
}

function AlignBtn({
  label, icon: Icon, onClick, disabled,
}: {
  label: string;
  icon: typeof Move;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <Button size="sm" variant="outline" className="h-8 px-0" title={label} disabled={disabled} onClick={onClick}>
      <Icon className="size-4" />
    </Button>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-[11px] text-muted-foreground">{label}</span>
      {children}
    </label>
  );
}

function WidgetProps({
  w, onChange, onDelete,
}: {
  w: YantraWidget;
  onChange: (patch: Partial<YantraWidget>) => void;
  onDelete: () => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <div className="text-sm font-medium">Widget</div>
        <Button size="sm" variant="ghost" className="h-6 px-1 text-destructive" onClick={onDelete}>
          <Trash2 className="size-3.5" />
        </Button>
      </div>

      <Field label="Type">
        <Select value={w.type} onValueChange={(t) => onChange({ type: t })}>
          <SelectTrigger className="h-7 text-xs"><SelectValue /></SelectTrigger>
          <SelectContent>
            {WIDGET_TYPES.map((t) => <SelectItem key={t} value={t}>{t}</SelectItem>)}
          </SelectContent>
        </Select>
      </Field>
      <Field label="Label">
        <Input className="h-7 text-xs" value={w.label ?? ""} onChange={(e) => onChange({ label: e.target.value })} />
      </Field>

      <div className="grid grid-cols-4 gap-1">
        {(["x", "y", "w", "h"] as const).map((k) => (
          <Field key={k} label={k.toUpperCase()}>
            <Input className="h-7 px-1 text-xs" type="number" value={w[k] ?? (k === "w" || k === "h" ? 1 : 0)}
              onChange={(e) => onChange({ [k]: Math.max(k === "w" || k === "h" ? 1 : 0, +e.target.value) })} />
          </Field>
        ))}
      </div>

      {w.type === "button" && (
        <ActionEditor label="On click" value={w.send} onChange={(a) => onChange({ send: a })} />
      )}
      {w.type === "toggle" && (
        <>
          <ActionEditor label="On" value={w.on} onChange={(a) => onChange({ on: a })} />
          <ActionEditor label="Off" value={w.off} onChange={(a) => onChange({ off: a })} />
        </>
      )}
      {w.type === "slider" && (
        <>
          <div className="grid grid-cols-3 gap-1">
            <Field label="Min"><Input className="h-7 px-1 text-xs" type="number" value={w.min ?? 0}
              onChange={(e) => onChange({ min: +e.target.value })} /></Field>
            <Field label="Max"><Input className="h-7 px-1 text-xs" type="number" value={w.max ?? 100}
              onChange={(e) => onChange({ max: +e.target.value })} /></Field>
            <Field label="Step"><Input className="h-7 px-1 text-xs" type="number" value={w.step ?? 1}
              onChange={(e) => onChange({ step: +e.target.value })} /></Field>
          </div>
          <ActionEditor label="On change" value={w.send} hint="use {value}" onChange={(a) => onChange({ send: a })} />
        </>
      )}
      {w.type === "select" && (
        <div className="flex flex-col gap-1.5">
          <span className="text-[11px] text-muted-foreground">Options</span>
          {(w.options ?? []).map((o, i) => (
            <div key={i} className="flex flex-col gap-1 rounded border bg-muted/20 p-1.5">
              <div className="flex gap-1">
                <Input className="h-6 flex-1 text-[11px]" placeholder="label" value={o.label}
                  onChange={(e) => {
                    const opts = [...(w.options ?? [])];
                    opts[i] = { ...opts[i], label: e.target.value };
                    onChange({ options: opts });
                  }} />
                <Button size="sm" variant="ghost" className="h-6 px-1 text-destructive"
                  onClick={() => onChange({ options: (w.options ?? []).filter((_, j) => j !== i) })}>
                  <Trash2 className="size-3" />
                </Button>
              </div>
              <ActionEditor label="sends" value={o.send} onChange={(a) => {
                const opts = [...(w.options ?? [])];
                opts[i] = { ...opts[i], send: a };
                onChange({ options: opts });
              }} />
            </div>
          ))}
          <Button size="sm" variant="outline" className="h-6 gap-1 text-[11px]"
            onClick={() => onChange({ options: [...(w.options ?? []), { label: "Option", send: "" }] })}>
            <Plus className="size-3" /> option
          </Button>
        </div>
      )}
      {w.type === "readout" && (
        <Field label="Match (regex, group 1)">
          <Input className="h-7 font-mono text-[11px]" value={w.match ?? ""} onChange={(e) => onChange({ match: e.target.value })} />
        </Field>
      )}

      <Field label="Help (tooltip)">
        <Input className="h-7 text-xs" value={w.help ?? ""} onChange={(e) => onChange({ help: e.target.value })} />
      </Field>
    </div>
  );
}
