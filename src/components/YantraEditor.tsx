// Visual editor for a .yantra control surface: drag/resize widgets on a grid,
// edit their properties (incl. the transport-agnostic action), add/remove widgets,
// and save back to the .yantra file. Pairs with YantraCanvas (the read-only
// renderer); App switches between them with an "Edit" toggle.
import { useEffect, useRef, useState } from "react";
import { Plus, Save, Undo2, Trash2 } from "lucide-react";

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
  const [sel, setSel] = useState<number | null>(null);
  const gridRef = useRef<HTMLDivElement>(null);
  const cols = draft.cols ?? 6;
  const widgets = draft.widgets ?? [];
  const dirty = JSON.stringify(draft) !== JSON.stringify(spec);

  // re-seed when the file prop changes (App passes key=file too, but be safe)
  useEffect(() => { setDraft(clone(spec)); setSel(null); }, [file]); // eslint-disable-line react-hooks/exhaustive-deps

  const setWidget = (i: number, patch: Partial<YantraWidget>) =>
    setDraft((d) => {
      const ws = [...(d.widgets ?? [])];
      ws[i] = { ...ws[i], ...patch };
      return { ...d, widgets: ws };
    });
  const addWidget = (type: string) =>
    setDraft((d) => {
      const ws = d.widgets ?? [];
      const maxY = ws.reduce((m, w) => Math.max(m, (w.y ?? 0) + (w.h ?? 1)), 0);
      return { ...d, widgets: [...ws, defaultWidget(type, maxY)] };
    });
  const removeWidget = (i: number) =>
    setDraft((d) => ({ ...d, widgets: (d.widgets ?? []).filter((_, j) => j !== i) }));

  // --- drag / resize -------------------------------------------------------
  const drag = useRef<null | {
    mode: "move" | "resize"; idx: number;
    px: number; py: number; x: number; y: number; w: number; h: number;
  }>(null);

  const cellSize = () => {
    const el = gridRef.current;
    const cw = el ? el.clientWidth / cols : 80;
    return { cw, ch: ROW_H };
  };

  useEffect(() => {
    const move = (e: PointerEvent) => {
      const g = drag.current;
      if (!g) return;
      const { cw, ch } = cellSize();
      const dx = Math.round((e.clientX - g.px) / cw);
      const dy = Math.round((e.clientY - g.py) / ch);
      const w0 = widgets[g.idx];
      if (!w0) return;
      if (g.mode === "move") {
        const w = w0.w ?? 1;
        setWidget(g.idx, {
          x: Math.max(0, Math.min(cols - w, g.x + dx)),
          y: Math.max(0, g.y + dy),
        });
      } else {
        const x = w0.x ?? 0;
        setWidget(g.idx, {
          w: Math.max(1, Math.min(cols - x, g.w + dx)),
          h: Math.max(1, g.h + dy),
        });
      }
    };
    const up = () => { drag.current = null; };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  }, [widgets, cols]); // eslint-disable-line react-hooks/exhaustive-deps

  const startDrag = (mode: "move" | "resize", i: number, e: React.PointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const w = widgets[i];
    setSel(i);
    drag.current = {
      mode, idx: i, px: e.clientX, py: e.clientY,
      x: w.x ?? 0, y: w.y ?? 0, w: w.w ?? 1, h: w.h ?? 1,
    };
  };

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
          <div className="ml-auto flex items-center gap-1.5">
            {dirty && <span className="text-[11px] text-amber-600 dark:text-amber-400">unsaved</span>}
            <Button size="sm" variant="outline" className="h-7 gap-1 px-2 text-[11px]"
              disabled={!dirty} onClick={() => { setDraft(clone(spec)); setSel(null); }}>
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
          className="relative flex-1 overflow-auto rounded border bg-muted/10"
          style={{
            backgroundSize: `calc(100% / ${cols}) ${ROW_H}px`,
            backgroundImage:
              "linear-gradient(to right, hsl(var(--border)/0.5) 1px, transparent 1px), linear-gradient(to bottom, hsl(var(--border)/0.5) 1px, transparent 1px)",
            minHeight: rows * ROW_H,
          }}
          onPointerDown={() => setSel(null)}
        >
          {widgets.map((w, i) => {
            const cw = `calc(${((w.w ?? 1) / cols) * 100}% )`;
            return (
              <div
                key={i}
                onPointerDown={(e) => startDrag("move", i, e)}
                className={`absolute cursor-move select-none rounded border bg-card p-1 text-[11px] shadow-sm ${
                  sel === i ? "ring-2 ring-primary" : ""
                }`}
                style={{
                  left: `calc(${((w.x ?? 0) / cols) * 100}%)`,
                  top: (w.y ?? 0) * ROW_H,
                  width: cw,
                  height: (w.h ?? 1) * ROW_H,
                  padding: 4,
                }}
              >
                <div className="flex h-full flex-col overflow-hidden">
                  <span className="truncate font-medium">{w.label || w.type}</span>
                  <span className="truncate text-[10px] text-muted-foreground">{w.type}</span>
                </div>
                {/* resize handle */}
                <div
                  onPointerDown={(e) => startDrag("resize", i, e)}
                  className="absolute bottom-0 right-0 size-3 cursor-se-resize rounded-tl bg-primary/40"
                />
              </div>
            );
          })}
        </div>
      </div>

      {/* property panel */}
      <div className="w-64 shrink-0 overflow-auto rounded border bg-muted/10 p-3">
        {sel == null || !widgets[sel] ? (
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
            <p className="text-[10px] text-muted-foreground">Select a widget to edit it, or drag one on the canvas.</p>
          </div>
        ) : (
          <WidgetProps
            w={widgets[sel]}
            onChange={(p) => setWidget(sel, p)}
            onDelete={() => { removeWidget(sel); setSel(null); }}
          />
        )}
      </div>
    </div>
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
