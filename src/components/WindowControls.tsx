// Custom min/maximize/close buttons for the frameless window (decorations are
// off in tauri.conf.json, so we draw our own). Lives at the right edge of the
// title bar. Needs core:window allow-minimize / -toggle-maximize / -close /
// -is-maximized / -start-dragging in the capability file.
import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, Copy, X } from "lucide-react";

const appWindow = getCurrentWindow();

export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    appWindow.isMaximized().then(setMaximized).catch(() => {});
    // keep the maximize/restore glyph in sync when the OS resizes the window
    appWindow
      .onResized(() => appWindow.isMaximized().then(setMaximized).catch(() => {}))
      .then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  const btn =
    "flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground";

  return (
    <div className="flex h-full items-stretch">
      <button type="button" className={btn} title="Minimize" onClick={() => appWindow.minimize()}>
        <Minus className="size-4" />
      </button>
      <button
        type="button"
        className={btn}
        title={maximized ? "Restore" : "Maximize"}
        onClick={() => appWindow.toggleMaximize()}
      >
        {maximized ? <Copy className="size-3.5" /> : <Square className="size-3.5" />}
      </button>
      <button
        type="button"
        className="flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-destructive hover:text-white"
        title="Close"
        onClick={() => appWindow.close()}
      >
        <X className="size-4" />
      </button>
    </div>
  );
}
