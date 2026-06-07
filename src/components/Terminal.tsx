import { useEffect, useRef } from "react";
import { init, Terminal as Ghostty, FitAddon } from "ghostty-web";
import { onData, dataWrite } from "@/lib/ttl";

// WASM init is shared across all Terminal instances; run it once.
let initialized: Promise<void> | null = null;
const ensureInit = () => (initialized ??= init());

export function Terminal({ connected }: { connected: boolean }) {
  const hostRef = useRef<HTMLDivElement>(null);
  // ghostty-web's Terminal is xterm.js-API-compatible
  const termRef = useRef<any>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let ro: ResizeObserver | undefined;
    let sub: { dispose?: () => void } | undefined;

    ensureInit().then(() => {
      if (disposed || !hostRef.current) return;
      const term = new Ghostty({
        fontFamily: 'ui-monospace, "Cascadia Code", "JetBrains Mono", Menlo, monospace',
        fontSize: 13,
        cursorBlink: true,
        scrollback: 5000,
        theme: { background: "#0a0a0b", foreground: "#e4e4e7", cursor: "#a1a1aa" },
      } as any);
      const fit = new FitAddon();
      term.loadAddon(fit);
      term.open(hostRef.current);
      try { fit.fit(); } catch { /* not laid out yet */ }
      termRef.current = term;

      ro = new ResizeObserver(() => {
        try { fit.fit(); } catch { /* ignore */ }
      });
      ro.observe(hostRef.current);

      // target console bytes -> terminal
      onData((bytes) => term.write(bytes)).then((u) => (unlisten = u));
      // keystrokes -> DATA/UART
      sub = term.onData((d: string) => {
        dataWrite(Array.from(new TextEncoder().encode(d))).catch(() => {});
      });

      term.write("\x1b[90msutra — connect a port to begin.\x1b[0m\r\n");
    });

    return () => {
      disposed = true;
      sub?.dispose?.();
      unlisten?.();
      ro?.disconnect();
      termRef.current?.dispose?.();
      termRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (connected) termRef.current?.write("\x1b[32m● connected\x1b[0m\r\n");
  }, [connected]);

  return <div ref={hostRef} className="h-full w-full" />;
}
