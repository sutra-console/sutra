// Network (WiFi) provisioning popover — shown for devices that answer the WiFi
// CFG keys. Reads the live status (off / connecting / connected+IP / portal /
// failed), takes SSID + password over the current CMD link, and once the device
// reports an IP offers a one-click jump to the WebSocket connect dialog.
import { Globe, RefreshCw, Wifi } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  WIFI, WS_PORT, type WifiAp, type WifiStatus,
  wifiConfigure, wifiScanResults, wifiScanStart, wifiStatus,
} from "@/lib/skrit";

const STATE_LABEL: Record<number, string> = {
  [WIFI.OFF]: "off — no network configured",
  [WIFI.CONNECTING]: "joining",
  [WIFI.CONNECTED]: "connected",
  [WIFI.PORTAL]: "setup portal active",
  [WIFI.FAILED]: "join failed",
};

export function NetworkConfig({
  disabled,
  onConnectWs,
}: {
  disabled?: boolean;
  /** Open the app's WebSocket dialog pre-filled with this device's URL. */
  onConnectWs: (url: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<WifiStatus | null>(null);
  const [ssid, setSsid] = useState("");
  const [pass, setPass] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [aps, setAps] = useState<WifiAp[] | null>(null); // scan results (null = not scanned)
  const [scanning, setScanning] = useState(false);
  const poll = useRef<number | null>(null);

  // Scan for nearby APs using the board's radio, then pick one to fill the SSID.
  async function scan() {
    setScanning(true);
    setErr(null);
    try {
      await wifiScanStart();
      await new Promise((r) => setTimeout(r, 2500)); // async scan ~1.5–2s
      const found = await wifiScanResults();
      found.sort((a, b) => b.rssi - a.rssi);
      setAps(found);
    } catch (e) {
      setErr(String(e));
    } finally {
      setScanning(false);
    }
  }

  async function refresh() {
    try {
      setStatus(await wifiStatus());
      setErr(null);
    } catch (e) {
      setStatus(null);
      setErr(String(e));
    }
  }

  // Poll while open (joins resolve within seconds).
  useEffect(() => {
    if (!open) {
      if (poll.current) window.clearInterval(poll.current);
      poll.current = null;
      return;
    }
    refresh();
    poll.current = window.setInterval(refresh, 2000);
    return () => {
      if (poll.current) window.clearInterval(poll.current);
      poll.current = null;
    };
  }, [open]);

  async function apply() {
    setBusy(true);
    setErr(null);
    try {
      await wifiConfigure(ssid.trim(), pass);
      await refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  const connectedIp = status?.state === WIFI.CONNECTED ? status.detail : null;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" size="sm" className="h-7 gap-1.5" disabled={disabled}
          title="WiFi setup (network bridge)">
          <Wifi className="size-3.5" /> Network
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-72" align="end">
        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between text-xs">
            <span className="font-medium">WiFi</span>
            <button type="button" className="text-muted-foreground hover:text-foreground"
              title="Refresh" onClick={refresh}>
              <RefreshCw className="size-3" />
            </button>
          </div>
          <p className="text-xs text-muted-foreground">
            {status
              ? <>
                  {STATE_LABEL[status.state] ?? `state ${status.state}`}
                  {status.detail && <> · <span className="font-mono">{status.detail}</span></>}
                </>
              : err
                ? "this device has no WiFi config"
                : "reading…"}
          </p>

          {connectedIp && (
            <Button size="sm" className="gap-1.5"
              onClick={() => { setOpen(false); onConnectWs(`ws://${connectedIp}:${WS_PORT}/`); }}>
              <Globe className="size-3.5" /> Connect over WebSocket
            </Button>
          )}

          <div className="flex items-end gap-1">
            <label className="flex-1 text-xs">
              SSID
              <Input className="mt-1 h-8" value={ssid} onChange={(e) => setSsid(e.target.value)}
                placeholder={status?.state === WIFI.CONNECTED ? "change network…" : "network name"} />
            </label>
            <Button variant="outline" size="sm" className="h-8 gap-1" onClick={scan} disabled={scanning}
              title="Scan for nearby networks using the board's radio">
              <RefreshCw className={`size-3.5 ${scanning ? "animate-spin" : ""}`} /> Scan
            </Button>
          </div>
          {aps && (
            <div className="max-h-32 overflow-y-auto rounded border text-xs">
              {aps.length === 0 ? (
                <div className="px-2 py-1 text-muted-foreground">no networks found</div>
              ) : (
                aps.map((ap, i) => (
                  <button key={`${ap.ssid}-${i}`} type="button"
                    className="flex w-full items-center justify-between gap-2 px-2 py-1 text-left hover:bg-accent"
                    onClick={() => setSsid(ap.ssid)}>
                    <span className="truncate">{ap.ssid || <span className="text-muted-foreground">(hidden)</span>}</span>
                    <span className="shrink-0 tabular-nums text-muted-foreground">ch{ap.channel} · {ap.rssi}dBm</span>
                  </button>
                ))
              )}
            </div>
          )}
          <label className="text-xs">
            Password
            <Input className="mt-1 h-8" type="password" value={pass}
              onChange={(e) => setPass(e.target.value)} />
          </label>
          <Button size="sm" onClick={apply} disabled={busy || !ssid.trim()}>
            {busy ? "Applying…" : "Join network"}
          </Button>
          {status?.state === WIFI.PORTAL && (
            <p className="text-[10px] text-muted-foreground">
              Or join the <span className="font-mono">{status.detail}</span> hotspot from a phone —
              the setup page opens automatically.
            </p>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
