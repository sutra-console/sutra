// Path-B host: the egui/WASM yantra renderer. Shares the data-flow with the React
// renderer via useYantraRuntime — egui only draws + reports input. The host pushes
// per-widget render-state (values + presentation overrides) and the resolved theme
// into the wasm, and routes egui input events back through the runtime (publish +
// runAction). So the device/Lua side is unchanged; egui is render + input only.
import { useEffect, useMemo, useRef } from "react";

import init, { start, set_state, set_theme, set_edit } from "../../yantra-wasm/pkg/yantra_wasm";
import wasmUrl from "../../yantra-wasm/pkg/yantra_wasm_bg.wasm?url";
import { useYantraRuntime } from "@/hooks/useYantraRuntime";
import { resolveTokens, THEME_EVENT } from "@/lib/theme";
import type { YantraSpec, YantraWidget } from "@/lib/skrit";

export function WasmYantraCanvas({
  spec, disabled, editing, onSave,
}: {
  spec: YantraSpec;
  disabled?: boolean;
  editing?: boolean; // edit mode (egui editable canvas) vs interact
  onSave?: (spec: YantraSpec) => void; // edit-mode Save → write the .yantra
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  const rt = useYantraRuntime(spec, disabled);
  const ready = useRef(false);
  const onSaveRef = useRef(onSave); onSaveRef.current = onSave;

  const widgetByName = useMemo(() => {
    const m: Record<string, YantraWidget> = {};
    for (const w of rt.widgets) if (w.name) m[w.name] = w;
    return m;
  }, [rt.widgets]);

  // egui input → the runtime. Kept in a ref so the wasm trampoline always sees the
  // latest closure without re-mounting.
  const onEventRef = useRef<(json: string) => void>(() => {});
  onEventRef.current = (json: string) => {
    let ev: { kind?: string; name?: string; value?: unknown; index?: number };
    try { ev = JSON.parse(json); } catch { return; }
    const w = ev.name ? widgetByName[ev.name] : undefined;
    if (!w || !ev.name) return;
    if (ev.kind === "value") {
      rt.publish(ev.name, ev.value); // slider/toggle/color → the bus
      if (w.send) rt.fire(w.send, String(ev.value));
      rt.fireWidgetEvent(w, "value", ev.value); // Lua handler (if w.handlers.value)
    } else if (ev.kind === "press") {
      rt.fire(w.send); // button
      rt.fireWidgetEvent(w, "press"); // Lua handler (if w.handlers.press)
    } else if (ev.kind === "select") {
      // selector: publish the chosen value (scripts read it to show/hide frames)
      // and fire the chosen option's own action.
      rt.publish(ev.name, ev.value);
      const opt = typeof ev.index === "number" ? w.options?.[ev.index] : undefined;
      if (opt?.send) rt.fire(opt.send, String(ev.value));
      rt.fireWidgetEvent(w, "select", ev.value); // Lua handler (if w.handlers.select)
    }
  };

  // mount once per spec: init the wasm, start with the spec + a stable event trampoline.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      await init(wasmUrl);
      if (cancelled || !ref.current) return;
      start(
        ref.current,
        JSON.stringify(spec),
        (s: string) => onEventRef.current(s),
        (specJson: string) => {
          try { onSaveRef.current?.(JSON.parse(specJson)); } catch { /* malformed */ }
        },
      );
      set_edit(!!editing);
      ready.current = true;
      set_theme(JSON.stringify(resolveTokens()));
    })().catch((e) => console.error("yantra-wasm mount failed", e));
    return () => { cancelled = true; ready.current = false; };
  }, [spec]);

  // toggle edit mode without remounting
  useEffect(() => {
    if (ready.current) set_edit(!!editing);
  }, [editing]);

  // push the latest render-state every render (host is authoritative for display).
  useEffect(() => {
    if (!ready.current) return;
    const widgets: Record<string, Record<string, unknown>> = {};
    for (const w of rt.widgets) {
      if (!w.name) continue;
      const ov = rt.ovOf(w) ?? {};
      widgets[w.name] = {
        value: ov.value ?? rt.valueOf(w),
        fill: ov.fill, fg: ov.fg, label: ov.label, hidden: ov.hidden, disabled: ov.disabled,
      };
    }
    // frame overrides (script-driven hide/show → selector composes tabs)
    const frames: Record<string, Record<string, unknown>> = {};
    for (const f of rt.frames) {
      const fo = rt.frameOvOf(f.id);
      if (fo) frames[f.id] = { hidden: fo.hidden };
    }
    set_state(JSON.stringify({ widgets, frames }));
  });

  // re-sync egui visuals when the app theme changes.
  useEffect(() => {
    const on = () => { if (ready.current) set_theme(JSON.stringify(resolveTokens())); };
    window.addEventListener(THEME_EVENT, on);
    return () => window.removeEventListener(THEME_EVENT, on);
  }, []);

  return <canvas ref={ref} className="h-full w-full" />;
}
