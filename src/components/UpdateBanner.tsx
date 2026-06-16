import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { Button } from "@/components/ui/button";
import { Download, X } from "lucide-react";

// On launch, checks the configured GitHub-releases endpoint for a newer signed
// build. If one exists, shows a one-line banner to download + install + relaunch.
// Silent on any failure (no endpoint configured, offline, or a dev build).
export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [busy, setBusy] = useState(false);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    check()
      .then((u) => setUpdate(u))
      .catch(() => {});
  }, []);

  if (!update || dismissed) return null;

  const install = async () => {
    setBusy(true);
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch {
      setBusy(false); // re-enable so the user can retry or dismiss
    }
  };

  return (
    <div className="flex items-center gap-2 border-b bg-primary/10 px-3 py-1.5 text-sm">
      <Download className="size-4 shrink-0 text-primary" />
      <span>
        A new version <b>v{update.version}</b> of Sutra is available.
      </span>
      <div className="ml-auto flex items-center gap-1.5">
        <Button size="sm" onClick={install} disabled={busy}>
          {busy ? "Updating…" : "Update & Restart"}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => setDismissed(true)}
          disabled={busy}
          title="Dismiss"
        >
          <X className="size-3.5" />
        </Button>
      </div>
    </div>
  );
}
