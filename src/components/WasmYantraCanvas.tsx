// Path-B host: mounts the egui WASM yantra runtime onto a React-owned <canvas>.
// SPIKE: this loads the egui spike module to prove the mount + Vite/wasm wiring.
// (Real version will feed the spec + bus state in, and route input/actions out to
// the app's existing runAction / mlua data flow.)
import { useEffect, useRef } from "react";

// wasm-pack `web` output (built from ../../yantra-wasm). `?url` hands Vite the
// .wasm as an asset URL so init() can fetch it.
import init, { start } from "../../yantra-wasm/pkg/yantra_wasm";
import wasmUrl from "../../yantra-wasm/pkg/yantra_wasm_bg.wasm?url";

export function WasmYantraCanvas() {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    let cancelled = false;
    (async () => {
      await init(wasmUrl);
      if (cancelled || !ref.current) return;
      start(ref.current);
    })().catch((e) => console.error("yantra-wasm mount failed", e));
    return () => { cancelled = true; };
  }, []);
  return <canvas ref={ref} className="h-full w-full" />;
}
