import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { init, Terminal as Ghostty, FitAddon } from "ghostty-web";
import { onData, dataWrite, readConsole } from "@/lib/ttl";

// WASM init is shared across all Terminal instances; run it once.
let initialized: Promise<void> | null = null;
const ensureInit = () => (initialized ??= init());

export interface TerminalHandle {
  focus: () => void;
}

export const Terminal = forwardRef<TerminalHandle, { connected: boolean }>(function Terminal(
  { connected },
  ref
) {
  const hostRef = useRef<HTMLDivElement>(null);
  // ghostty-web's Terminal is xterm.js-API-compatible
  const termRef = useRef<any>(null);
  const prevConnected = useRef(connected);

  useImperativeHandle(ref, () => ({
    focus: () => {
      try {
        termRef.current?.focus?.();
      } catch {
        /* ignore */
      }
      // fallback: focus the input element ghostty/xterm mounts in the host
      const el = hostRef.current?.querySelector(
        "textarea, canvas, [tabindex]"
      ) as HTMLElement | null;
      el?.focus?.();
    },
  }), []);

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

      // seed with whatever the backend already buffered (survives webview reloads)
      readConsole(32000)
        .then((hist) => {
          if (hist) term.write(hist);
          else term.write("\x1b[90mSutra — connect a port to begin.\x1b[0m\r\n");
        })
        .catch(() => {});
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
    const t = termRef.current;
    if (t) {
      if (connected && !prevConnected.current) t.write("\r\n\x1b[32m● connected\x1b[0m\r\n");
      else if (!connected && prevConnected.current) t.write("\r\n\x1b[31m● disconnected\x1b[0m\r\n");
    }
    prevConnected.current = connected;
  }, [connected]);

  return <div ref={hostRef} className="h-full w-full" />;
});
