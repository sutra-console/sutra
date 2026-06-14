// Path-B host: the egui/WASM yantra renderer. Shares the data-flow with the React
// renderer via useYantraRuntime — egui only draws + reports input. The host pushes
// per-widget render-state (values + presentation overrides) and the resolved theme
// into the wasm, and routes egui input events back through the runtime (publish +
// runAction). So the device/Lua side is unchanged; egui is render + input only.
import { useEffect, useMemo, useRef } from "react";

import init, { start, set_state, set_theme } from "../../yantra-wasm/pkg/yantra_wasm";
import wasmUrl from "../../yantra-wasm/pkg/yantra_wasm_bg.wasm?url";
import { useYantraRuntime } from "@/hooks/useYantraRuntime";
import { resolveTokens, THEME_EVENT } from "@/lib/theme";
import type { YantraSpec, YantraWidget } from "@/lib/skrit";

export function WasmYantraCanvas({ spec, disabled }: { spec: YantraSpec; disabled?: boolean }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const rt = useYantraRuntime(spec, disabled);
  const ready = useRef(false);

  const widgetByName = useMemo(() => {
    const m: Record<string, YantraWidget> = {};
    for (const w of rt.widgets) if (w.name) m[w.name] = w;
    return m;
  }, [rt.widgets]);

  // egui input → the runtime. Kept in a ref so the wasm trampoline always sees the
  // latest closure without re-mounting.
  const onEventRef = useRef<(json: string) => void>(() => {});
  onEventRef.current = (json: string) => {
    let ev: { kind?: string; name?: string; value?: unknown };
    try { ev = JSON.parse(json); } catch { return; }
    const w = ev.name ? widgetByName[ev.name] : undefined;
    if (!w || !ev.name) return;
    if (ev.kind === "value") {
      rt.publish(ev.name, ev.value); // slider/toggle/color → the bus
      if (w.send) rt.fire(w.send, String(ev.value));
    } else if (ev.kind === "press") {
      rt.fire(w.send); // button / select option
    }
  };

  // mount once per spec: init the wasm, start with the spec + a stable event trampoline.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      await init(wasmUrl);
      if (cancelled || !ref.current) return;
      start(ref.current, JSON.stringify(spec), (s: string) => onEventRef.current(s));
      ready.current = true;
      set_theme(JSON.stringify(resolveTokens()));
    })().catch((e) => console.error("yantra-wasm mount failed", e));
    return () => { cancelled = true; ready.current = false; };
  }, [spec]);

  // push the latest render-state every render (host is authoritative for display).
  useEffect(() => {
    if (!ready.current) return;
    const widgets: Record<string, Record<string, unknown>> = {};
    for (const w of rt.widgets) {
      if (!w.name) continue;
      const ov = rt.ovOf(w) ?? {};
      widgets[w.name] = {
        value: ov.value ?? rt.valueOf(w),
        color: ov.color, fg: ov.fg, label: ov.label, hidden: ov.hidden, disabled: ov.disabled,
      };
    }
    set_state(JSON.stringify({ widgets }));
  });

  // re-sync egui visuals when the app theme changes.
  useEffect(() => {
    const on = () => { if (ready.current) set_theme(JSON.stringify(resolveTokens())); };
    window.addEventListener(THEME_EVENT, on);
    return () => window.removeEventListener(THEME_EVENT, on);
  }, []);

  return <canvas ref={ref} className="h-full w-full" />;
}
