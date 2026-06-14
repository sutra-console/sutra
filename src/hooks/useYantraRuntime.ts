// The yantra runtime data-flow, extracted so BOTH renderers (the React
// YantraCanvas and the egui WasmYantraCanvas) share one source of truth: the
// console buffer + reactive value bus, bind/emit data flow, the per-surface Lua
// tick (native mlua via yantra_eval), presentation overrides, and device-action
// dispatch. The renderers only draw + report input; this owns the logic.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  MSG, dataWrite, i2cXfer, invokeCommand, sendCmd, outputRgb, outputPwm, outputSet,
  onData, yantraEval, bindOf, computeBus, evalArray, evalBind, needsConsole, CURRENT_CONN,
  type YantraAction, type YantraFrame, type YantraSpec, type YantraWidget,
} from "@/lib/skrit";

const enc = new TextEncoder();
const dec = new TextDecoder();

/** Dispatch a widget action over the right transport. `value` (slider/emit) is
 *  substituted into a string action's {value} or carried by an `out` action. */
export async function runAction(a: YantraAction | undefined, value?: string): Promise<void> {
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
  if ("out" in a) {
    const lvl = Math.max(0, Math.min(255, Math.round(Number(a.out.value ?? value ?? 0))));
    const idx = a.out.index ?? 0;
    if (a.out.kind === "pwm") await outputPwm(idx, lvl);
    else if (a.out.kind === "set") await outputSet(idx, lvl > 0);
    else await outputRgb(idx, { r: lvl, g: lvl, b: lvl });
    return;
  }
}

export interface YantraRuntime {
  widgets: YantraWidget[];
  frames: YantraFrame[];
  vars: Record<string, unknown>;
  activeTabs: Record<number, string>;
  setActiveTabs: (f: (m: Record<number, string>) => Record<number, string>) => void;
  activeTabOf: (i: number) => string | undefined;
  valueOf: (w: YantraWidget) => unknown;
  rowsOf: (w: YantraWidget) => unknown[];
  ovOf: (w: YantraWidget) => Record<string, unknown> | undefined;
  publish: (name: string, v: unknown) => void;
  fire: (a: YantraAction | undefined, value?: string) => void;
  hasScripts: boolean;
  scriptLog: string[];
  clearLog: () => void;
}

export function useYantraRuntime(spec: YantraSpec, disabled?: boolean): YantraRuntime {
  const fire = useCallback((a: YantraAction | undefined, value?: string) => {
    runAction(a, value).catch(() => {});
  }, []);
  const widgets = spec.widgets ?? [];
  const frames = spec.frames ?? [];
  const wantsConsole = needsConsole(widgets);

  const [activeTabs, setActiveTabs] = useState<Record<number, string>>({});
  const activeTabOf = (i: number) => activeTabs[i] ?? widgets[i].tabs?.[0]?.id;

  // Rolling per-connection console buffers (only the current connection populated today).
  const [bufs, setBufs] = useState<Record<string, string>>({});
  useEffect(() => {
    if (!wantsConsole) return;
    let un: (() => void) | undefined;
    onData((bytes) => {
      const t = dec.decode(Uint8Array.from(bytes));
      setBufs((b) => ({ ...b, [CURRENT_CONN]: ((b[CURRENT_CONN] ?? "") + t).slice(-4000) }));
    }).then((u) => (un = u));
    return () => un?.();
  }, [wantsConsole]);

  // Live control state (slider/toggle/color) published under each widget's name.
  const [controls, setControls] = useState<Record<string, unknown>>({});
  const publish = useCallback((name: string, v: unknown) => {
    setControls((c) => (c[name] === v ? c : { ...c, [name]: v }));
  }, []);

  const consoleBus = useMemo(() => computeBus(widgets, bufs), [widgets, bufs]);
  const vars = useMemo(() => ({ ...consoleBus, ...controls }), [consoleBus, controls]);
  const valueOf = (w: YantraWidget): unknown => evalBind(bindOf(w), vars, bufs);
  const rowsOf = (w: YantraWidget): unknown[] => evalArray(w, vars, bufs);

  // Consume-output: when a widget's `emit` value changes, fire its action.
  const lastEmit = useRef<Record<string, string>>({});
  useEffect(() => {
    if (disabled) return;
    widgets.forEach((w, i) => {
      if (!w.emit) return;
      const val = evalBind(
        { source: w.emit!.source, match: w.emit!.match, field: w.emit!.field, expr: w.emit!.expr },
        vars, bufs,
      );
      if (val === undefined || val === null || (typeof val === "number" && Number.isNaN(val))) return;
      const key = w.name ?? `#${i}`;
      const s = String(val);
      if (lastEmit.current[key] === s) return;
      lastEmit.current[key] = s;
      runAction(w.emit!.send, s).catch(() => {});
    });
  }, [vars, bufs, disabled, widgets]);

  // Per-surface Lua tick (native mlua via yantra_eval): writes overrides + sends + logs.
  const [overrides, setOverrides] = useState<Record<string, Record<string, unknown>>>({});
  const [scriptLog, setScriptLog] = useState<string[]>([]);
  const hasScripts = !!spec.script || widgets.some((w) => w.script);
  const varsRef = useRef(vars); varsRef.current = vars;
  const specRef = useRef(spec); specRef.current = spec;
  const lastTick = useRef(0);
  useEffect(() => {
    if (!hasScripts || disabled) return;
    const key = spec.name || "yantra";
    let alive = true;
    const id = setInterval(async () => {
      const s = specRef.current;
      const ws = (s.widgets ?? []).filter((w) => w.name && w.script).map((w) => ({ name: w.name!, script: w.script! }));
      const now = Date.now();
      const dt = lastTick.current ? now - lastTick.current : 0;
      lastTick.current = now;
      try {
        const out = await yantraEval(key, s.script ?? "", ws, { ...varsRef.current, t: now, dt });
        if (!alive) return;
        const ov: Record<string, Record<string, unknown>> = {};
        for (const [name, attrs] of Object.entries(out.sets ?? {})) {
          if (!attrs || typeof attrs !== "object") continue;
          if ((attrs as { value?: unknown }).value !== undefined) publish(name, (attrs as { value?: unknown }).value);
          ov[name] = attrs as Record<string, unknown>;
        }
        setOverrides(ov);
        for (const a of out.sends ?? []) runAction(a).catch(() => {});
        if (out.logs?.length) setScriptLog((l) => [...l, ...out.logs].slice(-50));
      } catch (e) {
        setScriptLog((l) => [...l, `! ${e}`].slice(-50));
      }
    }, 100);
    return () => { alive = false; clearInterval(id); };
  }, [hasScripts, disabled, spec.name]); // eslint-disable-line react-hooks/exhaustive-deps
  const ovOf = (w: YantraWidget): Record<string, unknown> | undefined => (w.name ? overrides[w.name] : undefined);

  return {
    widgets, frames, vars, activeTabs, setActiveTabs, activeTabOf,
    valueOf, rowsOf, ovOf, publish, fire, hasScripts, scriptLog,
    clearLog: useCallback(() => setScriptLog([]), []),
  };
}
