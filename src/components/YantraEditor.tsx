// Visual editor for a .yantra control surface: drag/resize widgets on a grid,
// edit their properties (incl. the transport-agnostic action), add/remove widgets,
// and save back to the .yantra file. Pairs with YantraCanvas (the read-only
// renderer); App switches between them with an "Edit" toggle.
import { useEffect, useMemo, useRef, useState } from "react";
import {
  Plus, Save, Undo2, Redo2, RotateCcw, Trash2, Move,
  Layers, Eye, EyeOff, Lock, LockOpen, ChevronUp, ChevronDown, ChevronRight,
  AlignStartVertical, AlignCenterVertical, AlignEndVertical,
  AlignStartHorizontal, AlignCenterHorizontal, AlignEndHorizontal,
  AlignHorizontalSpaceBetween, AlignVerticalSpaceBetween,
} from "lucide-react";
import Moveable from "react-moveable";
import Selecto from "react-selecto";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import {
  ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  resolveAxis, storeAxis,
  type AnchorMode, type YantraAction, type YantraFrame, type YantraSpec, type YantraWidget,
} from "@/lib/skrit";
import { Widget } from "./YantraCanvas";

const ANCHORS: AnchorMode[] = ["scale", "start", "center", "end", "stretch"];
const r2 = (n: number) => Math.round(n * 100) / 100;

const ROW_H = 56; // px per grid row in the editor (renderer auto-sizes rows)
const WIDGET_TYPES = ["button", "toggle", "slider", "select", "readout", "label", "tabs"] as const;
const tid = () => `t${crypto.randomUUID().slice(0, 5)}`;

// ---- small helpers ----------------------------------------------------------

const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v));

// Migrate the legacy flat `group` tag to the frame model, synthesizing frame
// entries for any referenced ids. Idempotent.
function migrateFrames(spec: YantraSpec): YantraSpec {
  const s = clone(spec);
  const frames = s.frames ?? [];
  const known = new Set(frames.map((f) => f.id));
  for (const w of s.widgets ?? []) {
    if (w.group && !w.frame) { w.frame = w.group; delete w.group; }
    if (w.frame && !known.has(w.frame)) { frames.push({ id: w.frame, name: "Frame" }); known.add(w.frame); }
  }
  if (frames.length) s.frames = frames;

  // Phase C: convert the old flat grid coords (x/w in cols, y/h in rows, absolute to
  // canvas) into container-relative units. Frames become full-canvas (so children's
  // canvas-relative coords stay correct as frame-relative), then the user can resize them.
  if (s.coordV !== 2) {
    const cols = s.cols ?? 6;
    for (const w of s.widgets ?? []) {
      w.x = ((w.x ?? 0) / cols) * 100;
      w.w = ((w.w ?? 1) / cols) * 100;
      w.y = (w.y ?? 0) * ROW_H;
      w.h = (w.h ?? 1) * ROW_H;
      w.anchorH = "scale";
      w.anchorV = "start";
    }
    for (const f of s.frames ?? []) {
      f.x = 0; f.y = 0; f.w = 100; f.h = 100;
      f.anchorH = "scale"; f.anchorV = "scale";
      if (f.clip === undefined) f.clip = false; // migrated full-canvas frames don't clip
    }
    s.coordV = 2;
  }
  // interim Phase-C2 files stored unitH/unitV — convert to the anchor model
  const u2a = (n: { anchorH?: AnchorMode; anchorV?: AnchorMode; unitH?: string; unitV?: string }) => {
    if (!n.anchorH && n.unitH) n.anchorH = n.unitH === "pct" ? "scale" : "start";
    if (!n.anchorV && n.unitV) n.anchorV = n.unitV === "pct" ? "scale" : "start";
    delete n.unitH; delete n.unitV;
  };
  for (const w of s.widgets ?? []) u2a(w);
  for (const f of s.frames ?? []) u2a(f);
  return s;
}
const csvToBytes = (s: string): number[] =>
  s.split(/[\s,]+/).filter(Boolean).map((t) => parseInt(t, t.startsWith("0x") ? 16 : 10) || 0);
const bytesToCsv = (b?: number[]): string =>
  (b ?? []).map((n) => `0x${n.toString(16).padStart(2, "0")}`).join(" ");

// Phase C coords: x/w in % of parent, y/h in px. y is the stacking offset (px).
function defaultWidget(type: string, y: number): YantraWidget {
  const base: YantraWidget = { type, label: type, x: 4, y, w: 30, h: 48, anchorH: "scale", anchorV: "start" };
  switch (type) {
    case "button": return { ...base, send: "" };
    case "toggle": return { ...base, on: "", off: "" };
    case "slider": return { ...base, w: 50, min: 0, max: 100, step: 1, send: "{value}" };
    case "select": return { ...base, w: 50, options: [{ label: "Option", send: "" }] };
    case "readout": return { ...base, w: 50, match: "" };
    case "tabs": return { ...base, w: 60, h: 220, label: "Tabs", tabs: [{ id: tid(), label: "Tab 1" }, { id: tid(), label: "Tab 2" }] };
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
  const [draft, setDraft] = useState<YantraSpec>(() => migrateFrames(spec));
  const [selected, setSelected] = useState<number[]>([]); // selected widget indices
  const [selectedFrames, setSelectedFrames] = useState<string[]>([]); // frames selected in the tree
  const [containerW, setContainerW] = useState(0);
  const [containerH, setContainerH] = useState(0);
  const [ready, setReady] = useState(false);
  const [toolMenu, setToolMenu] = useState<"a" | "s" | null>(null); // open align/spacing submenu
  const [showLayers, setShowLayers] = useState(false); // layers panel visible
  const [activeTab, setActiveTab] = useState<Record<number, string>>({}); // editor preview: active pane per tabs widget
  const [past, setPast] = useState<YantraSpec[]>([]); // undo stack (checkpoints before each change)
  const [future, setFuture] = useState<YantraSpec[]>([]); // redo stack
  const committed = useRef<YantraSpec>(migrateFrames(spec)); // last history checkpoint
  const gridRef = useRef<HTMLDivElement>(null);
  const moveableRef = useRef<Moveable>(null);
  const toolbarRef = useRef<HTMLDivElement>(null); // floating toolbar, repositioned live during a gesture
  const moved = useRef(false); // true once a gesture actually moves (so a plain click doesn't commit/drift)
  const widgetRefs = useRef<(HTMLDivElement | null)[]>([]);

  const cols = draft.cols ?? 6;
  const widgets = draft.widgets ?? [];
  const frames = draft.frames ?? [];
  widgetRefs.current.length = widgets.length;
  const cw = containerW > 0 ? containerW / cols : 80;
  const dirty = JSON.stringify(draft) !== JSON.stringify(spec);
  const sel = selected.length ? selected[0] : null;

  // Phase C: coords are relative to the parent container's content box, in unitH/unitV
  // (pct of parent | px). The editor is FLAT — we resolve each node's ABSOLUTE px rect
  // by walking the parent chain, so Moveable/Selecto keep working on absolute wrappers.
  type Rect = { x: number; y: number; w: number; h: number };
  type Node = { x?: number; y?: number; w?: number; h?: number; anchorH?: AnchorMode; anchorV?: AnchorMode };
  const TAB_BAR = 30; // approx tab-bar height (renderer uses flex; editor estimates)
  const parentKeyOf = (n: { tab?: string; frame?: string; parent?: string }) =>
    n.tab ?? n.frame ?? n.parent ?? "root";
  const relRect = (n: Node, pb: Rect): Rect => {
    const aH = n.anchorH ?? "scale", aV = n.anchorV ?? "start";
    const h = resolveAxis(aH, n.x ?? 0, n.w ?? (aH === "scale" ? 25 : 100), pb.w);
    const v = resolveAxis(aV, n.y ?? 0, n.h ?? (aV === "scale" ? 25 : 48), pb.h);
    return { x: pb.x + h.start, w: h.size, y: pb.y + v.start, h: v.size };
  };
  const contentBox = (key: string, seen = new Set<string>()): Rect => {
    if (key === "root" || seen.has(key)) return { x: 0, y: 0, w: containerW, h: containerH };
    seen.add(key);
    const f = frames.find((x) => x.id === key);
    if (f) return relRect(f, contentBox(parentKeyOf(f), seen));
    const owner = widgets.find((w) => w.type === "tabs" && (w.tabs ?? []).some((t) => t.id === key));
    if (owner) {
      const ob = relRect(owner, contentBox(parentKeyOf(owner), seen));
      return { x: ob.x, y: ob.y + TAB_BAR, w: ob.w, h: Math.max(0, ob.h - TAB_BAR) };
    }
    return { x: 0, y: 0, w: containerW, h: containerH };
  };
  const absRect = (n: Node & { tab?: string; frame?: string; parent?: string }): Rect =>
    relRect(n, contentBox(parentKeyOf(n)));

  // Which container a canvas point lands in (innermost wins) — for drag-to-reparent.
  // Candidates: root, every visible frame, and each visible tabs widget's ACTIVE pane.
  const dropTarget = (cx: number, cy: number, cur: string): string => {
    const cands: { key: string; box: Rect }[] = [{ key: "root", box: contentBox("root") }];
    for (const f of frames) if (!frameHidden(f)) cands.push({ key: f.id, box: contentBox(f.id) });
    widgets.forEach((w, i) => {
      if (w.type === "tabs" && !paneHidden(w)) {
        const a = activeTab[i] ?? w.tabs?.[0]?.id;
        if (a) cands.push({ key: a, box: contentBox(a) });
      }
    });
    const inside = cands.filter((c) => cx >= c.box.x && cx <= c.box.x + c.box.w && cy >= c.box.y && cy <= c.box.y + c.box.h);
    if (!inside.length) return "root";
    const area = (c: { box: Rect }) => c.box.w * c.box.h;
    inside.sort((a, b) => area(a) - area(b) || (a.key === "root" ? 1 : 0) - (b.key === "root" ? 1 : 0));
    const cc = inside.find((c) => c.key === cur);
    return cc && area(cc) <= area(inside[0]) + 4 ? cur : inside[0].key; // prefer staying on ~tie
  };
  const geom = (w: YantraWidget) => {
    const r = absRect(w);
    return { left: r.x, top: r.y, width: r.w, height: r.h };
  };

  // editor preview gating: show only the active pane's members (treat tabs like a
  // frame). Mirrors the renderer but against the editor's chosen active tab.
  const activeTabOf = (i: number) => activeTab[i] ?? widgets[i].tabs?.[0]?.id;
  const tabActive = (tabId: string) => {
    const owner = widgets.findIndex((x) => x.type === "tabs" && (x.tabs ?? []).some((t) => t.id === tabId));
    return owner < 0 || activeTabOf(owner) === tabId;
  };
  const panesFor = (w: YantraWidget): string[] => {
    const out: string[] = [];
    if (w.tab) out.push(w.tab);
    let fid = w.frame;
    const seen = new Set<string>();
    while (fid && !seen.has(fid)) {
      seen.add(fid);
      const f = frames.find((x) => x.id === fid);
      if (!f) break;
      if (f.tab) out.push(f.tab);
      fid = f.parent;
    }
    return out;
  };
  const paneHidden = (w: YantraWidget) => panesFor(w).some((p) => !tabActive(p));
  // a frame is hidden if any pane on its own tab/parent chain is inactive
  const frameHidden = (f: YantraFrame) => panesFor({ type: "", tab: f.tab, frame: f.parent } as YantraWidget).some((p) => !tabActive(p));

  // re-seed when the file prop changes (App also remounts via key, but be safe)
  useEffect(() => {
    setDraft(migrateFrames(spec)); setSelected([]); setSelectedFrames([]);
    committed.current = migrateFrames(spec); setPast([]); setFuture([]);
  }, [file]); // eslint-disable-line react-hooks/exhaustive-deps

  // Snapshot draft into the undo stack once edits settle (coalesces rapid changes
  // like dragging or typing into a single history step).
  useEffect(() => {
    const id = setTimeout(() => {
      if (JSON.stringify(draft) !== JSON.stringify(committed.current)) {
        setPast((p) => [...p, committed.current].slice(-100));
        committed.current = clone(draft);
        setFuture([]);
      }
    }, 350);
    return () => clearTimeout(id);
  }, [draft]);

  const undo = () => {
    if (!past.length) return;
    const prev = past[past.length - 1];
    setPast((p) => p.slice(0, -1));
    setFuture((f) => [committed.current, ...f]);
    committed.current = clone(prev);
    setDraft(clone(prev));
    setSelected([]);
  };
  const redo = () => {
    if (!future.length) return;
    const next = future[0];
    setFuture((f) => f.slice(1));
    setPast((p) => [...p, committed.current]);
    committed.current = clone(next);
    setDraft(clone(next));
    setSelected([]);
  };

  // measure the canvas so grid cells map to pixels (Moveable works in px)
  useEffect(() => {
    setReady(true);
    const el = gridRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => { setContainerW(el.clientWidth); setContainerH(el.clientHeight); });
    ro.observe(el);
    setContainerW(el.clientWidth);
    setContainerH(el.clientHeight);
    return () => ro.disconnect();
  }, []);

  // keep Moveable's box synced when geometry changes from outside a gesture
  useEffect(() => { moveableRef.current?.updateRect(); }, [draft, containerW, selected]);

  // Set a dimension from a typed string like "50", "50%", "120px". A "%" suffix
  // switches the axis anchor to scale; "px" switches a scale axis to start.
  const setDim = (field: "x" | "y" | "w" | "h", raw: string) => {
    if (sel == null) return;
    const w = widgets[sel];
    const axis: "H" | "V" = field === "x" || field === "w" ? "H" : "V";
    const m = raw.trim().match(/^(-?\d*\.?\d+)\s*(%|px)?$/i);
    if (!m) return;
    const val = parseFloat(m[1]);
    const suffix = m[2]?.toLowerCase();
    const cur = axis === "H" ? w.anchorH ?? "scale" : w.anchorV ?? "start";
    if (suffix === "%" && cur !== "scale") setAnchor(axis, "scale");
    else if (suffix === "px" && cur === "scale") setAnchor(axis, "start");
    setWidget(sel, { [field]: val });
  };

  // Switch a selected widget's axis anchor (Unity-style preset), converting the
  // stored values through the measured parent so it doesn't jump on screen.
  const setAnchor = (axis: "H" | "V", mode: AnchorMode) => {
    if (sel == null) return;
    const w = widgets[sel];
    const pb = contentBox(parentKeyOf(w));
    if (axis === "H") {
      const cur = w.anchorH ?? "scale";
      const abs = resolveAxis(cur, w.x ?? 0, w.w ?? (cur === "scale" ? 25 : 100), pb.w);
      const st = storeAxis(mode, abs.start, abs.size, pb.w);
      setWidget(sel, { anchorH: mode, x: r2(st.a), w: r2(st.b) });
    } else {
      const cur = w.anchorV ?? "start";
      const abs = resolveAxis(cur, w.y ?? 0, w.h ?? (cur === "scale" ? 25 : 48), pb.h);
      const st = storeAxis(mode, abs.start, abs.size, pb.h);
      setWidget(sel, { anchorV: mode, y: r2(st.a), h: r2(st.b) });
    }
  };

  // Delete/Backspace removes the selection (unless a text field is focused)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const ae = document.activeElement;
      const typing = ae && (ae.tagName === "INPUT" || ae.tagName === "TEXTAREA");
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
        if (typing) return; // let the text field handle its own undo
        e.preventDefault();
        if (e.shiftKey) redo(); else undo();
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "y") {
        if (typing) return;
        e.preventDefault();
        redo();
        return;
      }
      if (e.key === "Delete" || e.key === "Backspace") {
        if (typing) return;
        if (selected.length) {
          e.preventDefault();
          removeMany(selected);
          setSelected([]);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selected, past, future]); // eslint-disable-line react-hooks/exhaustive-deps

  const setWidget = (i: number, patch: Partial<YantraWidget>) =>
    setDraft((d) => {
      const ws = [...(d.widgets ?? [])];
      ws[i] = { ...ws[i], ...patch };
      return { ...d, widgets: ws };
    });
  const removeMany = (indices: number[]) =>
    setDraft((d) => ({ ...d, widgets: (d.widgets ?? []).filter((_, j) => !indices.includes(j)) }));
  // Reorder z-index by swapping neighbors (dir +1 = toward front / on top). Render
  // order is array order, so a later index draws on top. Remap the selection too.
  const moveLayer = (i: number, dir: 1 | -1) => {
    const j = i + dir;
    if (j < 0 || j >= widgets.length) return;
    setDraft((d) => {
      const ws = [...(d.widgets ?? [])];
      [ws[i], ws[j]] = [ws[j], ws[i]];
      return { ...d, widgets: ws };
    });
    setSelected((sel) => sel.map((s) => (s === i ? j : s === j ? i : s)));
  };
  const toggleHidden = (i: number) =>
    setDraft((d) => {
      const ws = [...(d.widgets ?? [])];
      ws[i] = { ...ws[i], hidden: !ws[i].hidden };
      return { ...d, widgets: ws };
    });
  const toggleLock = (i: number) =>
    setDraft((d) => {
      const ws = [...(d.widgets ?? [])];
      ws[i] = { ...ws[i], locked: !ws[i].locked };
      return { ...d, widgets: ws };
    });
  const toggleFrameLock = (id: string) =>
    setDraft((d) => ({ ...d, frames: (d.frames ?? []).map((f) => (f.id === id ? { ...f, locked: !f.locked } : f)) }));

  // --- frames: nestable editor-only containers --------------------------------
  // Frame ids in the subtree rooted at `id` (inclusive).
  const subtreeFrames = (id: string): string[] => {
    const out = [id];
    for (let n = 0; n < out.length; n++) {
      for (const f of frames) if (f.parent === out[n] && !out.includes(f.id)) out.push(f.id);
    }
    return out;
  };
  // Widget indices whose frame is in any of `frameIds`.
  const widgetsInFrames = (frameIds: string[]): number[] => {
    const set = new Set(frameIds);
    return widgets.map((w, i) => (w.frame && set.has(w.frame) ? i : -1)).filter((i) => i >= 0);
  };
  // (Canvas selection picks the individual widget; the layer tree's frame rows
  //  select a whole subtree via selectFrame.)

  // Group the current selection into a new frame (nests when items share a parent
  // frame, or when whole frames are selected via the tree).
  const groupSelected = () => {
    if (selected.length < 2 && selectedFrames.length < 1) return;
    const id = `f${crypto.randomUUID().slice(0, 6)}`;
    // common parent: the frame all selected items already sit under (else top level)
    const parents = new Set<string | undefined>([
      ...selected.map((i) => widgets[i]?.frame),
      ...selectedFrames.map((fid) => frames.find((f) => f.id === fid)?.parent),
    ]);
    const parent = parents.size === 1 ? [...parents][0] : undefined;
    setDraft((d) => {
      // New frame fills its parent (pct), so the children's existing relative coords
      // stay visually correct after reparenting; resize it afterwards to constrain.
      const newFrame: YantraFrame = { id, name: "Frame", parent, x: 0, y: 0, w: 100, h: 100, anchorH: "scale", anchorV: "scale", clip: false };
      const fr: YantraFrame[] = [...(d.frames ?? []), newFrame];
      // reparent selected frames under the new one (nesting)
      const fr2 = fr.map((f) => (selectedFrames.includes(f.id) ? { ...f, parent: id } : f));
      const ws = (d.widgets ?? []).map((w, i) => (selected.includes(i) ? { ...w, frame: id } : w));
      return { ...d, frames: fr2, widgets: ws };
    });
    setSelectedFrames([id]);
  };
  // Dissolve the selected frame(s): their direct children move up to the frame's parent.
  const ungroupFrames = (ids: string[]) => {
    if (!ids.length) return;
    setDraft((d) => {
      const fr = d.frames ?? [];
      const parentOf = (fid: string) => fr.find((f) => f.id === fid)?.parent;
      const ws = (d.widgets ?? []).map((w) =>
        w.frame && ids.includes(w.frame) ? { ...w, frame: parentOf(w.frame) } : w,
      );
      const fr2 = fr
        .filter((f) => !ids.includes(f.id))
        .map((f) => (f.parent && ids.includes(f.parent) ? { ...f, parent: parentOf(f.parent) } : f));
      return { ...d, frames: fr2, widgets: ws };
    });
    setSelectedFrames([]);
  };
  // Frames implied by the current selection (selected tree frames, or frames the
  // selected widgets belong to).
  const selectionFrames = (): string[] => {
    const out = new Set(selectedFrames);
    selected.forEach((i) => { const f = widgets[i]?.frame; if (f) out.add(f); });
    return [...out];
  };
  const ungroupSelected = () => ungroupFrames(selectionFrames());
  const selectionHasGroup = selectionFrames().length > 0;

  // anchor-aware size presets for the selection (% of parent / stretch / reset)
  const sizePreset = (patch: Partial<YantraWidget>) =>
    setDraft((d) => ({ ...d, widgets: (d.widgets ?? []).map((w, i) => (selected.includes(i) ? { ...w, ...patch } : w)) }));
  const SIZE_PRESETS: { label: string; patch: Partial<YantraWidget> }[] = [
    { label: "Fill width", patch: { anchorH: "scale", x: 0, w: 100 } },
    { label: "Fill height", patch: { anchorV: "scale", y: 0, h: 100 } },
    { label: "Fill parent", patch: { anchorH: "scale", anchorV: "scale", x: 0, y: 0, w: 100, h: 100 } },
    { label: "Stretch (margins)", patch: { anchorH: "stretch", anchorV: "stretch", x: 0, y: 0, w: 0, h: 0 } },
    { label: "Reset size", patch: { anchorH: "scale", anchorV: "start", w: 30, h: 48 } },
  ];
  // right-click selects the widget (and its frame) if it isn't already selected
  const ensureSelected = (i: number) => { if (!selected.includes(i)) { setSelectedFrames([]); setSelected([i]); } };
  // frame helpers (layer tree)
  const selectFrame = (id: string, additive: boolean) => {
    setSelectedFrames((sf) => (additive ? (sf.includes(id) ? sf.filter((x) => x !== id) : [...sf, id]) : [id]));
    const idxs = widgetsInFrames(subtreeFrames(id));
    setSelected((sel) => (additive ? [...new Set([...sel, ...idxs])] : idxs));
  };
  const renameFrame = (id: string, name: string) =>
    setDraft((d) => ({ ...d, frames: (d.frames ?? []).map((f) => (f.id === id ? { ...f, name } : f)) }));
  const toggleCollapse = (id: string) =>
    setDraft((d) => ({ ...d, frames: (d.frames ?? []).map((f) => (f.id === id ? { ...f, collapsed: !f.collapsed } : f)) }));
  const toggleFrameHidden = (id: string) => {
    const idxs = new Set(widgetsInFrames(subtreeFrames(id)));
    const anyVisible = [...idxs].some((i) => !widgets[i]?.hidden);
    setDraft((d) => ({
      ...d,
      widgets: (d.widgets ?? []).map((w, i) => (idxs.has(i) ? { ...w, hidden: anyVisible } : w)),
    }));
  };
  const addWidget = (type: string) => {
    const newIdx = widgets.length;
    setDraft((d) => {
      const ws = d.widgets ?? [];
      const y = ws.reduce((m, x) => Math.max(m, (x.y ?? 0) + (x.h ?? 1)), 0);
      const n = ws.filter((x) => x.type === type).length + 1;
      return { ...d, widgets: [...ws, { ...defaultWidget(type, y), name: `${type}${n}` }] };
    });
    setSelected([newIdx]);
  };


  // Read the dragged/resized widgets' final DOM rects back into the spec, converted
  // to coords RELATIVE to each widget's parent container, in its unit (pct|px).
  const commit = (indices: number[]) => {
    const cont = gridRef.current;
    if (!cont) return;
    const cr = cont.getBoundingClientRect();
    const r2 = (n: number) => Math.round(n * 100) / 100;
    setDraft((d) => {
      const ws = [...(d.widgets ?? [])];
      for (const i of indices) {
        const el = widgetRefs.current[i];
        if (!el) continue;
        const w0 = ws[i];
        const r = el.getBoundingClientRect();
        const absL = r.left - cr.left + cont.scrollLeft;
        const absT = r.top - cr.top + cont.scrollTop;
        // drag-to-reparent: which container does the widget's centre land in?
        const target = dropTarget(absL + r.width / 2, absT + r.height / 2, parentKeyOf(w0));
        const pb = contentBox(target);
        const ptr = target === "root"
          ? { frame: undefined, tab: undefined }
          : frames.some((f) => f.id === target)
            ? { frame: target, tab: undefined }
            : { tab: target, frame: undefined };
        const left = absL - pb.x, top = absT - pb.y;
        const aH = w0.anchorH ?? "scale", aV = w0.anchorV ?? "start";
        const H = storeAxis(aH, left, r.width, pb.w);
        const V = storeAxis(aV, top, r.height, pb.h);
        ws[i] = { ...w0, ...ptr, x: r2(H.a), w: r2(H.b), y: r2(V.a), h: r2(V.b) };
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
    { key: "dh", label: "Even spacing (horizontal)", Icon: AlignHorizontalSpaceBetween, run: () => distribute("h") },
    { key: "dv", label: "Even spacing (vertical)", Icon: AlignVerticalSpaceBetween, run: () => distribute("v") },
  ];

  // top-left corner (px, canvas coords) of the multi-selection's bounding box,
  // so a mini align/distribute toolbar can pin to the group outline.
  const groupBox = useMemo(() => {
    if (selected.length < 2) return null;
    const gs = selected.map((i) => widgets[i]).filter(Boolean).map(geom);
    if (!gs.length) return null;
    return { left: Math.min(...gs.map((g) => g.left)), top: Math.min(...gs.map((g) => g.top)) };
  }, [selected, draft, cw]); // eslint-disable-line react-hooks/exhaustive-deps

  // Move the floating toolbar to track the selection mid-gesture by writing to its
  // DOM node directly — no React state, so we never re-render (and abort) the drag.
  const syncLiveBox = () => {
    const cont = gridRef.current;
    const bar = toolbarRef.current;
    if (!cont || !bar) return;
    const cr = cont.getBoundingClientRect();
    let left = Infinity, top = Infinity;
    for (const i of selected) {
      const el = widgetRefs.current[i];
      if (!el) continue;
      const r = el.getBoundingClientRect();
      left = Math.min(left, r.left - cr.left + cont.scrollLeft);
      top = Math.min(top, r.top - cr.top + cont.scrollTop);
    }
    if (!Number.isFinite(left)) return;
    bar.style.left = `${Math.max(2, left - 34)}px`;
    bar.style.top = `${Math.max(0, top)}px`;
  };

  // Drag/resize apply Moveable's transform/size to the element. Holding Shift snaps
  // the result to the grid (cell size cw × ROW_H), relative to the element's base.
  const shiftOf = (e: unknown) => !!(e as { shiftKey?: boolean })?.shiftKey;
  const ctrlOf = (e: unknown) => !!(e as { ctrlKey?: boolean; metaKey?: boolean })?.ctrlKey || !!(e as { metaKey?: boolean })?.metaKey;
  const resizeCenters = useRef<Map<HTMLElement, { cx: number; cy: number }>>(new Map()); // center at resize-start
  const captureCenter = (el: HTMLElement) => {
    const l = parseFloat(el.style.left) || 0, t = parseFloat(el.style.top) || 0;
    resizeCenters.current.set(el, { cx: l + el.offsetWidth / 2, cy: t + el.offsetHeight / 2 });
  };
  const dragWidget = (el: HTMLElement, translate: number[], transform: string, snap: boolean) => {
    if (!snap) { el.style.transform = transform; return; }
    const baseLeft = parseFloat(el.style.left) || 0;
    const baseTop = parseFloat(el.style.top) || 0;
    const sx = Math.round((baseLeft + translate[0]) / cw) * cw - baseLeft;
    const sy = Math.round((baseTop + translate[1]) / ROW_H) * ROW_H - baseTop;
    el.style.transform = `translate(${sx}px, ${sy}px)`;
  };
  // snap = Shift (snap size to grid); mirror = Ctrl (keep centre fixed → symmetric resize).
  const resizeWidget = (el: HTMLElement, width: number, height: number, transform: string, snap: boolean, mirror: boolean) => {
    const w = snap ? Math.max(cw, Math.round(width / cw) * cw) : width;
    const h = snap ? Math.max(ROW_H, Math.round(height / ROW_H) * ROW_H) : height;
    el.style.width = `${w}px`;
    el.style.height = `${h}px`;
    const c = mirror ? resizeCenters.current.get(el) : undefined;
    if (c) {
      const baseLeft = parseFloat(el.style.left) || 0, baseTop = parseFloat(el.style.top) || 0;
      el.style.transform = `translate(${c.cx - w / 2 - baseLeft}px, ${c.cy - h / 2 - baseTop}px)`;
    } else {
      el.style.transform = transform;
    }
  };

  // sibling elements (for alignment/snap guidelines)
  const guidelines = useMemo(
    () => widgetRefs.current.filter((el, i) => !!el && !selected.includes(i)) as HTMLElement[],
    [selected, draft, containerW], // eslint-disable-line react-hooks/exhaustive-deps
  );
  const targets = useMemo(
    // locked widgets are selectable but get no move/resize handles
    () => selected.filter((i) => !widgets[i]?.locked).map((i) => widgetRefs.current[i]).filter(Boolean) as HTMLElement[],
    [selected, draft, containerW], // eslint-disable-line react-hooks/exhaustive-deps
  );


  // --- layer tree (frames + tabs panes; drag rows to reparent) ----------------
  const dragRef = useRef<{ kind: "w" | "f"; key: string } | null>(null);
  // Drop a dragged widget/frame into a container: {tab} (a pane), {frame}, or {} (root).
  const moveItem = (target: { tab?: string; frame?: string }) => {
    const d = dragRef.current;
    dragRef.current = null;
    if (!d) return;
    if (d.kind === "w") {
      setWidget(+d.key, { tab: target.tab, frame: target.frame });
    } else {
      if (target.frame && subtreeFrames(d.key).includes(target.frame)) return; // no cycles
      setDraft((dr) => ({
        ...dr,
        frames: (dr.frames ?? []).map((f) => (f.id === d.key ? { ...f, tab: target.tab, parent: target.frame } : f)),
      }));
    }
  };
  const dropProps = (target: { tab?: string; frame?: string }) => ({
    onDragOver: (e: React.DragEvent) => e.preventDefault(),
    onDrop: (e: React.DragEvent) => { e.preventDefault(); e.stopPropagation(); moveItem(target); },
  });

  const widgetRow = (i: number, depth: number): React.ReactNode => {
    const w = widgets[i];
    if (w.type === "tabs") return tabsNode(i, depth);
    return (
      <ContextMenu key={`w${i}`}>
        <ContextMenuTrigger asChild>
          <div draggable onDragStart={() => { dragRef.current = { kind: "w", key: String(i) }; }}
            style={{ paddingLeft: depth * 12 + 4 }}
            className={`flex items-center gap-1 rounded px-1 py-0.5 text-[11px] ${selected.includes(i) && !selectedFrames.length ? "bg-primary/15" : "hover:bg-accent/50"}`}
            onClick={(e) => { setSelectedFrames([]); setSelected((sel) => (e.shiftKey ? (sel.includes(i) ? sel.filter((s) => s !== i) : [...sel, i]) : [i])); }}>
            <button type="button" title={w.hidden ? "Show" : "Hide"} className="text-muted-foreground hover:text-foreground"
              onClick={(e) => { e.stopPropagation(); toggleHidden(i); }}>
              {w.hidden ? <EyeOff className="size-3" /> : <Eye className="size-3" />}
            </button>
            <span className="min-w-0 flex-1 truncate" title={w.label || w.type}>{w.label || w.type}</span>
            <button type="button" title={w.locked ? "Unlock" : "Lock"} className="text-muted-foreground hover:text-foreground"
              onClick={(e) => { e.stopPropagation(); toggleLock(i); }}>
              {w.locked ? <Lock className="size-3" /> : <LockOpen className="size-3 opacity-40" />}
            </button>
            <button type="button" title="Bring forward" disabled={i === widgets.length - 1}
              className="text-muted-foreground hover:text-foreground disabled:opacity-30"
              onClick={(e) => { e.stopPropagation(); moveLayer(i, 1); }}><ChevronUp className="size-3" /></button>
            <button type="button" title="Send backward" disabled={i === 0}
              className="text-muted-foreground hover:text-foreground disabled:opacity-30"
              onClick={(e) => { e.stopPropagation(); moveLayer(i, -1); }}><ChevronDown className="size-3" /></button>
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent className="w-40">
          <ContextMenuItem onSelect={() => { setSelectedFrames([]); setSelected([i]); }}>Select</ContextMenuItem>
          <ContextMenuItem onSelect={() => toggleLock(i)}>{w.locked ? "Unlock" : "Lock"}</ContextMenuItem>
          <ContextMenuItem onSelect={() => toggleHidden(i)}>{w.hidden ? "Show" : "Hide"}</ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem disabled={i === widgets.length - 1} onSelect={() => moveLayer(i, 1)}>Bring forward</ContextMenuItem>
          <ContextMenuItem disabled={i === 0} onSelect={() => moveLayer(i, -1)}>Send backward</ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem onSelect={() => { removeMany([i]); setSelected([]); }}>Delete</ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>
    );
  };
  const frameNode = (f: YantraFrame, depth: number): React.ReactNode => (
    <div key={f.id}>
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div draggable onDragStart={() => { dragRef.current = { kind: "f", key: f.id }; }}
            style={{ paddingLeft: depth * 12 }} {...dropProps({ frame: f.id })}
            className={`flex items-center gap-1 rounded px-1 py-0.5 text-[11px] ${selectedFrames.includes(f.id) ? "bg-primary/20" : "hover:bg-accent/50"}`}
            onClick={(e) => selectFrame(f.id, e.shiftKey)}>
            <button type="button" className="text-muted-foreground" title={f.collapsed ? "Expand" : "Collapse"}
              onClick={(e) => { e.stopPropagation(); toggleCollapse(f.id); }}>
              {f.collapsed ? <ChevronRight className="size-3" /> : <ChevronDown className="size-3" />}
            </button>
            <Layers className="size-3 text-muted-foreground" />
            <input value={f.name ?? "Frame"} onClick={(e) => e.stopPropagation()}
              onChange={(e) => renameFrame(f.id, e.target.value)}
              className="min-w-0 flex-1 bg-transparent outline-none focus:underline" />
            <button type="button" className="text-muted-foreground hover:text-foreground" title={f.locked ? "Unlock frame" : "Lock frame"}
              onClick={(e) => { e.stopPropagation(); toggleFrameLock(f.id); }}>
              {f.locked ? <Lock className="size-3" /> : <LockOpen className="size-3 opacity-40" />}
            </button>
            <button type="button" className="text-muted-foreground hover:text-foreground" title="Hide / show frame"
              onClick={(e) => { e.stopPropagation(); toggleFrameHidden(f.id); }}><Eye className="size-3" /></button>
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent className="w-40">
          <ContextMenuItem onSelect={() => selectFrame(f.id, false)}>Select contents</ContextMenuItem>
          <ContextMenuItem onSelect={() => toggleFrameLock(f.id)}>{f.locked ? "Unlock" : "Lock"}</ContextMenuItem>
          <ContextMenuItem onSelect={() => toggleFrameHidden(f.id)}>Hide / show</ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem onSelect={() => ungroupFrames([f.id])}>Ungroup (dissolve)</ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>
      {!f.collapsed && renderChildren(f.id, depth + 1)}
    </div>
  );
  const tabsNode = (i: number, depth: number): React.ReactNode => {
    const w = widgets[i];
    return (
      <div key={`w${i}`}>
        <div draggable onDragStart={() => { dragRef.current = { kind: "w", key: String(i) }; }}
          style={{ paddingLeft: depth * 12 }}
          className={`flex items-center gap-1 rounded px-1 py-0.5 text-[11px] ${selected.includes(i) && !selectedFrames.length ? "bg-primary/15" : "hover:bg-accent/50"}`}
          onClick={() => { setSelectedFrames([]); setSelected([i]); }}>
          <Layers className="size-3 text-primary" />
          <span className="min-w-0 flex-1 truncate font-medium">{w.name || w.label || "tabs"}</span>
          <button type="button" title="Bring forward" disabled={i === widgets.length - 1}
            className="text-muted-foreground hover:text-foreground disabled:opacity-30"
            onClick={(e) => { e.stopPropagation(); moveLayer(i, 1); }}><ChevronUp className="size-3" /></button>
          <button type="button" title="Send backward" disabled={i === 0}
            className="text-muted-foreground hover:text-foreground disabled:opacity-30"
            onClick={(e) => { e.stopPropagation(); moveLayer(i, -1); }}><ChevronDown className="size-3" /></button>
        </div>
        {(w.tabs ?? []).map((pane) => (
          <div key={pane.id}>
            <div style={{ paddingLeft: (depth + 1) * 12 }} {...dropProps({ tab: pane.id })}
              onClick={() => setActiveTab((m) => ({ ...m, [i]: pane.id }))}
              className={`flex items-center gap-1 rounded px-1 py-0.5 text-[10px] hover:bg-accent/40 ${activeTabOf(i) === pane.id ? "font-medium text-foreground" : "text-muted-foreground"}`}
              title="Click to edit this pane · drop a control/frame here to add it">
              <ChevronRight className="size-3" /> {pane.label}
            </div>
            {frames.filter((f) => f.tab === pane.id).map((f) => frameNode(f, depth + 2))}
            {widgets.map((cw, ci) => (cw.tab === pane.id ? ci : -1)).filter((ci) => ci >= 0).reverse().map((ci) => widgetRow(ci, depth + 2))}
          </div>
        ))}
      </div>
    );
  };
  const renderChildren = (parent: string | undefined, depth: number): React.ReactNode => {
    const childFrames = frames.filter((f) => f.parent === parent && !f.tab);
    const childWidgets = widgets.map((w, i) => (w.frame === parent && !w.tab ? i : -1)).filter((i) => i >= 0).reverse();
    return (
      <>
        {childFrames.map((f) => frameNode(f, depth))}
        {childWidgets.map((i) => widgetRow(i, depth))}
      </>
    );
  };

  return (
    <div className="flex h-full min-h-0 gap-3">
      {/* layers tree: frames (nestable) + widgets. Tree = hierarchy; z-order is
          still the flat widget array (the chevrons reorder it). */}
      {showLayers && (
        <div className="flex w-48 shrink-0 flex-col overflow-auto rounded border bg-muted/10 p-2"
          {...dropProps({})}>
          <div className="mb-1 text-[11px] font-medium text-muted-foreground">Layers</div>
          {widgets.length === 0 && frames.length === 0 && (
            <div className="text-[10px] text-muted-foreground">No widgets yet.</div>
          )}
          {renderChildren(undefined, 0)}
          <div className="mt-1 flex-1" {...dropProps({})} /> {/* drop here = move to top level */}
        </div>
      )}

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
          <Button size="sm" variant={showLayers ? "default" : "outline"} className="h-7 gap-1 px-2 text-[11px]"
            title="Layers" onClick={() => setShowLayers((v) => !v)}>
            <Layers className="size-3" /> Layers
          </Button>
          <span className="ml-1 text-[10px] text-muted-foreground">hold ⇧ to snap to grid</span>
          <div className="ml-auto flex items-center gap-1.5">
            {dirty && <span className="text-[11px] text-amber-600 dark:text-amber-400">unsaved</span>}
            <Button size="sm" variant="outline" className="h-7 px-1.5" title="Undo (Ctrl+Z)"
              disabled={!past.length} onClick={undo}>
              <Undo2 className="size-3.5" />
            </Button>
            <Button size="sm" variant="outline" className="h-7 px-1.5" title="Redo (Ctrl+Shift+Z)"
              disabled={!future.length} onClick={redo}>
              <Redo2 className="size-3.5" />
            </Button>
            <Button size="sm" variant="outline" className="h-7 gap-1 px-2 text-[11px]"
              disabled={!dirty} onClick={() => { setDraft(clone(spec)); setSelected([]); }} title="Revert to saved">
              <RotateCcw className="size-3" /> Revert
            </Button>
            <Button size="sm" className="h-7 gap-1 px-2 text-[11px]"
              disabled={!dirty || saving} onClick={() => onSave(draft)}>
              <Save className="size-3" /> Save
            </Button>
          </div>
        </div>

        <div
          ref={gridRef}
          className="yantra-canvas scroll-stable relative flex-1 overflow-auto rounded border bg-muted/10"
          style={{
            backgroundSize: `${cw}px ${ROW_H}px`,
            backgroundImage:
              "linear-gradient(to right, hsl(var(--border)/0.5) 1px, transparent 1px), linear-gradient(to bottom, hsl(var(--border)/0.5) 1px, transparent 1px)",
          }}
        >
          {/* container outlines (non-interactive) so frame bounds are visible */}
          {frames.map((f) => frameHidden(f) ? null : (() => {
            const r = absRect({ ...f });
            return (
              <div key={`f${f.id}`} className={`pointer-events-none absolute rounded border border-dashed ${selectedFrames.includes(f.id) ? "border-primary" : "border-muted-foreground/40"}`}
                style={{ left: r.x, top: r.y, width: r.w, height: r.h }}>
                <span className="absolute left-0 top-0 rounded-br bg-background/70 px-1 text-[9px] text-muted-foreground">{f.name || "frame"}</span>
              </div>
            );
          })())}
          {widgets.map((w, i) => paneHidden(w) ? null : (
            <ContextMenu key={i}>
              <ContextMenuTrigger asChild>
                <div
                  data-idx={i}
                  ref={(el) => { widgetRefs.current[i] = el; }}
                  onContextMenu={() => ensureSelected(i)}
                  className={`yantra-widget absolute overflow-hidden rounded ${
                    selected.includes(i) ? "ring-2 ring-primary" : ""
                  } ${w.hidden ? "opacity-40" : ""} ${w.locked ? "pointer-events-none" : ""}`}
                  style={geom(w)}
                >
                  {w.type === "tabs" ? (
                    // real tab bar; clicking a tab swaps the editor's active pane
                    <div className="flex flex-wrap gap-1 rounded border bg-card p-1">
                      {(w.tabs ?? []).map((t) => (
                        <button key={t.id} type="button"
                          className={`rounded px-2 py-0.5 text-xs ${activeTabOf(i) === t.id ? "bg-primary text-primary-foreground" : "bg-muted/40 hover:bg-muted"}`}
                          onClick={(e) => { e.stopPropagation(); setActiveTab((m) => ({ ...m, [i]: t.id })); }}>
                          {t.label}
                        </button>
                      ))}
                    </div>
                  ) : (
                    // the real control, non-interactive so gestures hit the wrapper (WYSIWYG)
                    <div className="pointer-events-none h-full w-full">
                      <Widget w={w} disabled fire={() => {}} readout={() => "—"} />
                    </div>
                  )}
                </div>
              </ContextMenuTrigger>
              <ContextMenuContent className="w-44">
                <ContextMenuItem onSelect={() => { removeMany(selected); setSelected([]); }}>
                  Delete{selected.length > 1 ? ` (${selected.length})` : ""}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem disabled={selected.length < 2} onSelect={groupSelected}>Group</ContextMenuItem>
                <ContextMenuItem disabled={!selectionHasGroup} onSelect={ungroupSelected}>Ungroup</ContextMenuItem>
                <ContextMenuItem onSelect={() => { const lock = selected.some((j) => !widgets[j]?.locked); setDraft((d) => ({ ...d, widgets: (d.widgets ?? []).map((x, j) => (selected.includes(j) ? { ...x, locked: lock } : x)) })); }}>
                  {selected.some((j) => !widgets[j]?.locked) ? "Lock" : "Unlock"}
                </ContextMenuItem>
                <ContextMenuSeparator />
                {SIZE_PRESETS.map((p) => (
                  <ContextMenuItem key={p.label} onSelect={() => sizePreset(p.patch)}>{p.label}</ContextMenuItem>
                ))}
              </ContextMenuContent>
            </ContextMenu>
          ))}

          {/* mini toolbar pinned to (and following) the group's outline: A = align,
              S = spacing sub-menus. Position is React-driven from the committed box;
              syncLiveBox() nudges it directly during a gesture. */}
          {groupBox && (
            <div
              ref={toolbarRef}
              className="yantra-tool absolute z-[60] flex flex-col gap-0.5 rounded-md border bg-popover/95 p-0.5 shadow-md backdrop-blur"
              style={{ left: Math.max(2, groupBox.left - 34), top: Math.max(0, groupBox.top) }}
              onPointerDown={(e) => e.stopPropagation()}
            >
              <button type="button" title="Align" onClick={() => setToolMenu((m) => (m === "a" ? null : "a"))}
                className={`flex size-6 items-center justify-center rounded text-[11px] font-semibold hover:bg-accent ${toolMenu === "a" ? "bg-accent" : ""}`}>
                A
              </button>
              {toolMenu === "a" && alignActions.map((a) => (
                <button key={a.key} type="button" title={a.label}
                  className="flex size-6 items-center justify-center rounded hover:bg-accent"
                  onClick={a.run}>
                  <a.Icon className="size-3.5" />
                </button>
              ))}
              <button type="button" title="Spacing" onClick={() => setToolMenu((m) => (m === "s" ? null : "s"))}
                className={`flex size-6 items-center justify-center rounded text-[11px] font-semibold hover:bg-accent ${toolMenu === "s" ? "bg-accent" : ""}`}>
                S
              </button>
              {toolMenu === "s" && distActions.map((a) => (
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
              bounds={{ left: 0, top: 0, position: "css" }}
              throttleDrag={0}
              throttleResize={0}
              onDrag={(e) => { moved.current = true; dragWidget(e.target as HTMLElement, e.translate, e.transform, shiftOf(e.inputEvent)); syncLiveBox(); }}
              onDragEnd={() => { if (moved.current) commit(selected); moved.current = false; }}
              onDragGroup={(e) => { moved.current = true; const s = shiftOf(e.inputEvent); e.events.forEach((ev) => dragWidget(ev.target as HTMLElement, ev.translate, ev.transform, s)); syncLiveBox(); }}
              onDragGroupEnd={() => { if (moved.current) commit(selected); moved.current = false; }}
              onResizeStart={(e) => captureCenter(e.target as HTMLElement)}
              onResize={(e) => { moved.current = true; resizeWidget(e.target as HTMLElement, e.width, e.height, e.drag.transform, shiftOf(e.inputEvent), ctrlOf(e.inputEvent)); syncLiveBox(); }}
              onResizeEnd={() => { if (moved.current) commit(selected); moved.current = false; }}
              onResizeGroupStart={(e) => e.events.forEach((ev) => captureCenter(ev.target as HTMLElement))}
              onResizeGroup={(e) => { moved.current = true; const s = shiftOf(e.inputEvent); const c = ctrlOf(e.inputEvent); e.events.forEach((ev) => resizeWidget(ev.target as HTMLElement, ev.width, ev.height, ev.drag.transform, s, c)); syncLiveBox(); }}
              onResizeGroupEnd={() => { if (moved.current) commit(selected); moved.current = false; }}
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
                if (t.closest(".yantra-tool")) { e.stop(); return; } // align toolbar clicks keep the selection
                if (mv?.isMoveableElement(t)) { e.stop(); return; }
                if (selected.some((i) => { const el = widgetRefs.current[i]; return !!el && (el === t || el.contains(t)); })) {
                  e.stop();
                }
              }}
              onSelectEnd={(e) => {
                const idxs = e.selected
                  .map((el) => Number((el as HTMLElement).dataset.idx))
                  .filter((n) => !Number.isNaN(n));
                setSelectedFrames([]);
                setSelected(idxs); // pick the individual widget(s) — frame select is via the layer tree
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
            <div className="flex gap-1">
              <Button size="sm" variant="outline" className="h-7 flex-1 gap-1 text-[11px]" onClick={groupSelected}>
                <Layers className="size-3" /> Group
              </Button>
              <Button size="sm" variant="outline" className="h-7 flex-1 text-[11px]"
                disabled={!selectionHasGroup} onClick={ungroupSelected}>
                Ungroup
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
              <div className="mb-1 text-[11px] text-muted-foreground">Spacing</div>
              <div className="grid grid-cols-2 gap-1">
                {distActions.map((a) => (
                  <AlignBtn key={a.key} label={a.label} icon={a.Icon}
                    disabled={selected.length < 3} onClick={a.run} />
                ))}
              </div>
            </div>
            <p className="text-[10px] text-muted-foreground">
              Even spacing needs 3+ widgets. Shift-click to add/remove from the selection.
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
            tabOptions={widgets.flatMap((x) => (x.tabs ?? []).map((t) => ({ id: t.id, label: `${x.name || x.label || "tabs"} · ${t.label}` })))}
            onChange={(p) => setWidget(sel, p)}
            onAnchor={setAnchor}
            onDim={setDim}
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

// Unit-aware dimension input: shows the value with its unit (px/%); typing a
// "%" or "px" suffix switches the axis unit. Commits on blur / Enter.
function DimInput({ value, mode, onCommit }: { value: number; mode: AnchorMode; onCommit: (raw: string) => void }) {
  const fmt = `${Math.round((value ?? 0) * 10) / 10}${mode === "scale" ? "%" : "px"}`;
  const [s, setS] = useState(fmt);
  useEffect(() => { setS(fmt); }, [fmt]);
  return (
    <Input className="h-7 px-1 text-xs" value={s}
      onChange={(e) => setS(e.target.value)}
      onBlur={() => onCommit(s)}
      onKeyDown={(e) => { if (e.key === "Enter") { onCommit(s); (e.target as HTMLInputElement).blur(); } }} />
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
  w, tabOptions, onChange, onAnchor, onDim, onDelete,
}: {
  w: YantraWidget;
  tabOptions: { id: string; label: string }[];
  onChange: (patch: Partial<YantraWidget>) => void;
  onAnchor: (axis: "H" | "V", mode: AnchorMode) => void;
  onDim: (field: "x" | "y" | "w" | "h", raw: string) => void;
  onDelete: () => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <div className="text-sm font-medium">Widget</div>
        <div className="flex items-center gap-0.5">
          <Button size="sm" variant="ghost" className="h-6 px-1" title={w.locked ? "Unlock" : "Lock"}
            onClick={() => onChange({ locked: !w.locked })}>
            {w.locked ? <Lock className="size-3.5" /> : <LockOpen className="size-3.5 opacity-50" />}
          </Button>
          <Button size="sm" variant="ghost" className="h-6 px-1 text-destructive" onClick={onDelete}>
            <Trash2 className="size-3.5" />
          </Button>
        </div>
      </div>

      <Field label="Type">
        <Select value={w.type} onValueChange={(t) => onChange({ type: t })}>
          <SelectTrigger className="h-7 text-xs"><SelectValue /></SelectTrigger>
          <SelectContent>
            {WIDGET_TYPES.map((t) => <SelectItem key={t} value={t}>{t}</SelectItem>)}
          </SelectContent>
        </Select>
      </Field>
      <Field label="Name (for scripts)">
        <Input className="h-7 font-mono text-xs" value={w.name ?? ""} placeholder="e.g. throttle"
          onChange={(e) => onChange({ name: e.target.value })} />
      </Field>
      <Field label="Label">
        <Input className="h-7 text-xs" value={w.label ?? ""} onChange={(e) => onChange({ label: e.target.value })} />
      </Field>

      {/* tab membership: which tab pane this widget belongs to (hidden unless active) */}
      {w.type !== "tabs" && tabOptions.length > 0 && (
        <Field label="In tab">
          <Select value={w.tab ?? "__none"} onValueChange={(v) => onChange({ tab: v === "__none" ? undefined : v })}>
            <SelectTrigger className="h-7 text-xs"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="__none">— always shown —</SelectItem>
              {tabOptions.map((t) => <SelectItem key={t.id} value={t.id}>{t.label}</SelectItem>)}
            </SelectContent>
          </Select>
        </Field>
      )}

      {/* tabs widget: edit the pane list */}
      {w.type === "tabs" && (
        <div className="flex flex-col gap-1.5">
          <span className="text-[11px] text-muted-foreground">Tabs</span>
          {(w.tabs ?? []).map((t, i) => (
            <div key={t.id} className="flex gap-1">
              <Input className="h-6 flex-1 text-[11px]" value={t.label}
                onChange={(e) => {
                  const tabs = [...(w.tabs ?? [])];
                  tabs[i] = { ...tabs[i], label: e.target.value };
                  onChange({ tabs });
                }} />
              <Button size="sm" variant="ghost" className="h-6 px-1 text-destructive" disabled={(w.tabs ?? []).length <= 1}
                onClick={() => onChange({ tabs: (w.tabs ?? []).filter((_, j) => j !== i) })}>
                <Trash2 className="size-3" />
              </Button>
            </div>
          ))}
          <Button size="sm" variant="outline" className="h-6 gap-1 text-[11px]"
            onClick={() => onChange({ tabs: [...(w.tabs ?? []), { id: tid(), label: `Tab ${(w.tabs ?? []).length + 1}` }] })}>
            <Plus className="size-3" /> tab
          </Button>
        </div>
      )}

      <div className="grid grid-cols-4 gap-1">
        {(["x", "y", "w", "h"] as const).map((k) => (
          <Field key={k} label={k.toUpperCase()}>
            <DimInput value={w[k] ?? 0} mode={(k === "x" || k === "w" ? w.anchorH : w.anchorV) ?? (k === "x" || k === "w" ? "scale" : "start")}
              onCommit={(raw) => onDim(k, raw)} />
          </Field>
        ))}
      </div>

      {/* anchors (per-axis, relative to the parent): scale=% · start/center/end=px · stretch=fill */}
      <div className="grid grid-cols-2 gap-1">
        <Field label="Anchor X">
          <Select value={w.anchorH ?? "scale"} onValueChange={(v) => onAnchor("H", v as AnchorMode)}>
            <SelectTrigger className="h-7 text-xs"><SelectValue /></SelectTrigger>
            <SelectContent>{ANCHORS.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}</SelectContent>
          </Select>
        </Field>
        <Field label="Anchor Y">
          <Select value={w.anchorV ?? "start"} onValueChange={(v) => onAnchor("V", v as AnchorMode)}>
            <SelectTrigger className="h-7 text-xs"><SelectValue /></SelectTrigger>
            <SelectContent>{ANCHORS.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}</SelectContent>
          </Select>
        </Field>
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
