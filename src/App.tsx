import { useEffect, useRef, useState } from "react";
import {
  Usb, Plug, PlugZap, Play, Plus, Trash2, Cpu, Settings2, Bot, Database, Copy, Lock, LockOpen, Pencil, GripVertical, Cog, CircleHelp, Bookmark, X, Download, Upload,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import { Popover, PopoverTrigger, PopoverContent } from "@/components/ui/popover";
import {
  Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { Terminal, type TerminalHandle } from "@/components/Terminal";
import { MacroHelp } from "@/components/MacroHelp";
import { save, open } from "@tauri-apps/plugin-dialog";
import {
  autodetect,
  connect as ttlConnect,
  disconnect as ttlDisconnect,
  listPorts,
  connState,
  outputToggle,
  outputsBitmap,
  getInfo,
  getDeviceName,
  getControls,
  CAP,
  runText,
  setDataParams,
  saveMacroToDevice,
  macrosGet,
  macroUpsert,
  macroDelete,
  macrosSet,
  exportSet,
  importSet,
  onMacros,
  macroRuns,
  cancelRun,
  onRuns,
  onLink,
  mcpStart,
  mcpStop,
  mcpStatus,
  setMcpTools,
  type PortDesc,
  type McpStatus,
  type McpToolFlags,
  type MacroRec,
  type MacroRunInfo,
  type ControlDesc,
} from "@/lib/ttl";

const BAUDS = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600, 1500000];

interface Settings {
  autoStartMcp: boolean;
  mcpPort: number;
  rememberLastPort: boolean;
  lastPort: string;
  mcpTools: McpToolFlags;
}
const DEFAULT_SETTINGS: Settings = {
  autoStartMcp: false,
  mcpPort: 8551,
  rememberLastPort: true,
  lastPort: "auto",
  mcpTools: {
    consoleRead: true,
    consoleWrite: true,
    outputs: true,
    macrosRun: true,
    macrosCreate: true,
    connection: true,
  },
};
const loadSettings = (): Settings => {
  try {
    const s = JSON.parse(localStorage.getItem("sutra.settings") || "{}");
    return {
      ...DEFAULT_SETTINGS,
      ...s,
      mcpTools: { ...DEFAULT_SETTINGS.mcpTools, ...(s.mcpTools || {}) },
    };
  } catch {
    return DEFAULT_SETTINGS;
  }
};

// A saved connection. `transport` is the abstraction point for TCP/BLE later;
// `target` is whatever that transport needs (serial port name, host:port, BLE id).
interface Profile {
  id: string;
  name: string;
  transport: "serial" | "tcp" | "ble";
  target: string;
}
const loadProfiles = (): Profile[] => {
  try {
    return JSON.parse(localStorage.getItem("sutra.profiles") || "[]");
  } catch {
    return [];
  }
};

const MCP_TOOL_OPTIONS: { key: keyof McpToolFlags; label: string; hint: string }[] = [
  { key: "consoleRead", label: "Read console", hint: "read_console" },
  { key: "consoleWrite", label: "Write console", hint: "write_console" },
  { key: "outputs", label: "Outputs & device info", hint: "set/get_output, device_info" },
  { key: "macrosRun", label: "List / run macros", hint: "list_macros, run_macro" },
  { key: "macrosCreate", label: "Create macros", hint: "create_macro" },
  { key: "connection", label: "Connection control", hint: "list ports, connect, set serial" },
];

export default function App() {
  const [ports, setPorts] = useState<PortDesc[]>([]);
  const [connected, setConnected] = useState(false);
  const [linkOnline, setLinkOnline] = useState(true); // target present on the wire
  const [dataPort, setDataPort] = useState<string | null>(null); // connected DATA port name
  const [hasCmd, setHasCmd] = useState(false);
  const [selectedPort, setSelectedPort] = useState("auto");
  const [profiles, setProfiles] = useState<Profile[]>(loadProfiles);
  const [profilesOpen, setProfilesOpen] = useState(false);
  const [profileName, setProfileName] = useState("");

  function saveProfiles(list: Profile[]) {
    setProfiles(list);
    localStorage.setItem("sutra.profiles", JSON.stringify(list));
  }
  function addProfile() {
    const target = selectedPort;
    const name = profileName.trim() || (target === "auto" ? "Duta (auto)" : target);
    const p: Profile = { id: crypto.randomUUID(), name, transport: "serial", target };
    saveProfiles([...profiles.filter((x) => x.name !== name), p]);
    setProfileName("");
  }
  function deleteProfile(id: string) {
    saveProfiles(profiles.filter((p) => p.id !== id));
  }
  function loadProfile(p: Profile) {
    setSelectedPort(p.target); // (serial only for now; transport routing comes with TCP/BLE)
    setProfilesOpen(false);
  }
  const [status, setStatus] = useState("disconnected");
  const [outBitmap, setOutBitmap] = useState(0); // device output states (bit i = control i)
  const [controls, setControls] = useState<ControlDesc[]>([]); // self-described controls
  const [deviceName, setDeviceName] = useState("");
  const [caps, setCaps] = useState(0); // device capability bits (buddy only)
  const [macros, setMacros] = useState<MacroRec[]>([]);
  const [runs, setRuns] = useState<MacroRunInfo[]>([]); // in-flight macro runs

  const [baud, setBaud] = useState(115200);
  const [parity, setParity] = useState<"none" | "odd" | "even">("none");
  const [stopBits, setStopBits] = useState(1);

  const [mcp, setMcp] = useState<McpStatus>({ running: false, url: null });
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const setSetting = <K extends keyof Settings>(k: K, v: Settings[K]) =>
    setSettings((s) => ({ ...s, [k]: v }));

  const terminalRef = useRef<TerminalHandle>(null);
  const focusTerm = () => setTimeout(() => terminalRef.current?.focus(), 0);

  // macro add/edit dialog
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editOrig, setEditOrig] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [draftText, setDraftText] = useState("");
  const [draftSecret, setDraftSecret] = useState(false);
  const [draftSet, setDraftSet] = useState("");
  const [showHelp, setShowHelp] = useState(false);
  const [setFilter, setSetFilter] = useState(""); // active project/set ("" = all)

  // macro drag-reorder (live projected order + ghost)
  const [dragName, setDragName] = useState<string | null>(null);
  const [overName, setOverName] = useState<string | null>(null);
  const [insertAfter, setInsertAfter] = useState(false);

  // the list as it would look if dropped right now
  const orderedMacros = (() => {
    if (!dragName || !overName || dragName === overName) return macros;
    const moved = macros.find((s) => s.name === dragName);
    if (!moved) return macros;
    const arr = macros.filter((s) => s.name !== dragName);
    let idx = arr.findIndex((s) => s.name === overName);
    if (idx === -1) return macros;
    if (insertAfter) idx += 1;
    arr.splice(idx, 0, moved);
    return arr;
  })();

  function commitReorder() {
    if (dragName && overName && dragName !== overName) {
      setMacros(orderedMacros); // optimistic; backend confirms via ttl://macros
      macrosSet(orderedMacros).catch(() => {});
    }
    setDragName(null);
    setOverName(null);
  }

  useEffect(() => {
    document.documentElement.classList.add("dark");
    refreshPorts();
    macrosGet().then(setMacros).catch(() => {});
    syncConnState(); // adopt a connection the backend already holds (after a reload)

    // push saved MCP tool toggles to the backend
    setMcpTools(settings.mcpTools).catch(() => {});

    // remember-last-port: preselect it in the dropdown
    if (settings.rememberLastPort && settings.lastPort) setSelectedPort(settings.lastPort);

    // MCP: reflect current status, then auto-start on launch if enabled
    mcpStatus().then((st) => {
      setMcp(st);
      if (!st.running && settings.autoStartMcp) {
        mcpStart(settings.mcpPort).then(setMcp).catch((e) => setStatus(`mcp: ${e}`));
      }
    }).catch(() => {});

    macroRuns().then(setRuns).catch(() => {});

    const un = onMacros(setMacros);
    const unRuns = onRuns(setRuns);
    const unLink = onLink((online) => {
      setLinkOnline(online);
      setStatus(online ? "target online" : "target offline — link lost, retrying…");
    });
    return () => {
      un.then((f) => f()).catch(() => {});
      unRuns.then((f) => f()).catch(() => {});
      unLink.then((f) => f()).catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // persist settings
  useEffect(() => {
    localStorage.setItem("sutra.settings", JSON.stringify(settings));
  }, [settings]);

  /// Re-sync the UI to the backend's actual connection (it survives webview reloads).
  async function syncConnState() {
    try {
      const cs = await connState();
      if (!cs.connected) return;
      setConnected(true);
      setHasCmd(cs.has_cmd);
      setBaud(cs.params.baud);
      setParity(cs.params.parity as any);
      setStopBits(cs.params.stop_bits);
      setDataPort(cs.data_port ?? null);
      setSelectedPort(cs.has_cmd ? "auto" : cs.data_port ?? "auto");
      setStatus(
        cs.has_cmd
          ? `Duta — DATA ${cs.data_port} · CMD ${cs.cmd_port}`
          : `serial — ${cs.data_port} @ ${cs.params.baud}`
      );
      if (cs.has_cmd) loadDevice();
    } catch {
      /* not connected / backend unavailable */
    }
  }

  async function refreshPorts() {
    try {
      setPorts(await listPorts());
    } catch (e) {
      setStatus(`port scan failed: ${e}`);
    }
  }

  async function handleConnect() {
    setStatus("connecting…");
    try {
      await setDataParams({ baud, data_bits: 8, parity, stop_bits: stopBits });
      if (selectedPort === "auto") {
        const { data, cmd } = await autodetect();
        await ttlConnect(data, cmd);
        setHasCmd(true);
        setDataPort(data);
        setStatus(`Duta — DATA ${data} · CMD ${cmd}`);
        loadDevice();
      } else {
        await ttlConnect(selectedPort, null);
        setHasCmd(false);
        setDataPort(selectedPort);
        clearDevice();
        setStatus(`serial — ${selectedPort} @ ${baud}`);
      }
      setConnected(true);
      setLinkOnline(true);
      if (settings.rememberLastPort) setSetting("lastPort", selectedPort);
      focusTerm();
    } catch (e) {
      setStatus(`connect failed: ${e}`);
      setConnected(false);
    }
  }

  async function handleDisconnect() {
    await ttlDisconnect().catch(() => {});
    setConnected(false);
    setLinkOnline(true);
    setDataPort(null);
    clearDevice();
    setStatus("disconnected");
  }

  async function applySerial() {
    try {
      await setDataParams({ baud, data_bits: 8, parity, stop_bits: stopBits });
      setStatus(`DATA set ${baud} 8${parity[0].toUpperCase()}${stopBits}`);
    } catch (e) {
      setStatus(`serial set failed: ${e}`);
    }
  }

  async function refreshOutputs() {
    try {
      setOutBitmap(await outputsBitmap());
    } catch {
      /* device may not implement OUTPUT_GET */
    }
  }

  async function loadDevice() {
    getDeviceName().then(setDeviceName).catch(() => {});
    getControls().then(setControls).catch(() => {});
    getInfo().then((i) => setCaps(i.caps)).catch(() => {});
    refreshOutputs();
  }

  function clearDevice() {
    setDeviceName("");
    setControls([]);
    setOutBitmap(0);
    setCaps(0);
  }

  async function toggle(index: number) {
    try {
      const r = await outputToggle(index); // resp body: [status, bitmap]
      setOutBitmap(r.body[1] ?? 0);
    } catch (e) {
      setStatus(`cmd failed: ${e}`);
    }
    focusTerm();
  }

  function openAdd() {
    setEditOrig(null);
    setDraftName("");
    setDraftText("");
    setDraftSecret(false);
    setDraftSet(setFilter); // default new macro to the active project
    setDialogOpen(true);
  }
  function openEdit(s: MacroRec) {
    setEditOrig(s.name);
    setDraftName(s.name);
    setDraftText(s.text);
    setDraftSecret(s.secret);
    setDraftSet(s.set);
    setDialogOpen(true);
  }
  async function saveMacro() {
    const name = draftName.trim();
    if (!name || !draftText) return;
    if (editOrig && editOrig !== name) await macroDelete(editOrig);
    await macroUpsert(name, draftText, draftSecret, draftSet.trim());
    setDialogOpen(false);
  }

  // distinct project/set names present in the store
  const sets = Array.from(new Set(macros.map((m) => m.set).filter(Boolean))).sort();
  const shownMacros = orderedMacros.filter((s) => !setFilter || s.set === setFilter);

  async function doExport() {
    const label = setFilter || "macros";
    const path = await save({
      defaultPath: `${label}.macroset.json`,
      filters: [{ name: "Macro set", extensions: ["json"] }],
    });
    if (!path) return;
    try {
      await exportSet(path, setFilter || undefined);
      setStatus(`exported ${setFilter ? `set "${setFilter}"` : "all macros"} → ${path}`);
    } catch (e) {
      setStatus(`export failed: ${e}`);
    }
  }

  async function doImport() {
    const path = await open({
      multiple: false,
      filters: [{ name: "Macro set", extensions: ["json"] }],
    });
    if (!path || typeof path !== "string") return;
    try {
      const n = await importSet(path);
      setStatus(`imported ${n} macro${n === 1 ? "" : "s"}`);
      macrosGet().then(setMacros).catch(() => {});
    } catch (e) {
      setStatus(`import failed: ${e}`);
    }
  }
  async function deleteMacro() {
    if (editOrig) await macroDelete(editOrig);
    setDialogOpen(false);
  }

  async function saveToDevice(s: MacroRec, index: number) {
    setStatus(`saving "${s.name}" to Duta…`);
    try {
      setStatus(`"${s.name}" → ${await saveMacroToDevice(index, s.name, s.text)}`);
    } catch (e) {
      setStatus(`save failed: ${e}`);
    }
  }

  async function toggleMcp() {
    try {
      setMcp(mcp.running ? await mcpStop() : await mcpStart(settings.mcpPort));
    } catch (e) {
      setStatus(`mcp failed: ${e}`);
    }
  }

  async function setToolFlag(k: keyof McpToolFlags, v: boolean) {
    const flags = { ...settings.mcpTools, [k]: v };
    setSetting("mcpTools", flags);
    await setMcpTools(flags).catch(() => {});
    // apply to a running server by restarting it (the client reconnects & re-lists)
    if (mcp.running) {
      try {
        await mcpStop();
        setMcp(await mcpStart(settings.mcpPort));
      } catch {
        /* ignore */
      }
    }
  }

  const ttlPorts = ports.filter((p) => p.is_duta);
  // On a Duta the firmware UART is 8N1 (1 stop, no parity) unless built with
  // PARITY_SUPPORT. On a generic adapter parity/stop are real hardware settings.
  const parityLocked = connected && hasCmd && !(caps & CAP.PARITY);
  const stopLocked = connected && hasCmd;

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <header className="flex items-center gap-3 border-b px-4 py-2.5">
        <Cpu className="size-5 text-primary" />
        <span className="font-semibold tracking-tight">Sutra</span>
        <Badge
          variant={!connected ? "secondary" : linkOnline ? "success" : "destructive"}
          className="ml-1"
        >
          {!connected ? "offline" : linkOnline ? "online" : "target offline"}
        </Badge>
        {deviceName && <span className="text-xs text-muted-foreground">· {deviceName}</span>}

        {/* MCP settings popover */}
        <Popover>
          <PopoverTrigger asChild>
            <Button variant="outline" size="sm" className="gap-1.5">
              <Bot className="size-3.5" /> MCP
              <span className={cn("size-1.5 rounded-full", mcp.running ? "bg-success" : "bg-muted-foreground/40")} />
            </Button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-80">
            <div className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <Bot className="size-4" />
                <span className="text-sm font-semibold">MCP server</span>
                <Badge variant={mcp.running ? "success" : "secondary"} className="ml-auto">
                  {mcp.running ? "on" : "off"}
                </Badge>
              </div>
              <div className="flex items-center gap-2">
                <span className="text-xs text-muted-foreground">Port</span>
                <Input type="number" className="h-8 w-24" value={settings.mcpPort} disabled={mcp.running} onChange={(e) => setSetting("mcpPort", +e.target.value)} />
                <Button size="sm" className="ml-auto" variant={mcp.running ? "destructive" : "default"} onClick={toggleMcp}>
                  {mcp.running ? "Stop" : "Start"}
                </Button>
              </div>
              {mcp.url && (
                <div className="flex items-center gap-1 rounded-md border px-2 py-1">
                  <code className="min-w-0 flex-1 truncate text-[11px]">{mcp.url}</code>
                  <Button variant="ghost" size="icon" className="size-6" title="Copy URL" onClick={() => mcp.url && navigator.clipboard.writeText(mcp.url)}>
                    <Copy />
                  </Button>
                </div>
              )}
              <p className="text-[10px] leading-tight text-muted-foreground">
                Lets an LLM read the console, run/author macros &amp; control outputs. Macro
                contents (secrets) are never exposed — it can only run them by name.
              </p>
            </div>
          </PopoverContent>
        </Popover>

        {/* DATA serial settings popover */}
        <Popover>
          <PopoverTrigger asChild>
            <Button variant="outline" size="sm" className="gap-1.5">
              <Settings2 className="size-3.5" />
              {connected ? `${dataPort ?? selectedPort} @ ${baud}` : "serial"}
              {connected && (
                <span className={cn("size-1.5 rounded-full", linkOnline ? "bg-success" : "bg-destructive")} />
              )}
            </Button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-72">
            <div className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <Settings2 className="size-4" />
                <span className="text-sm font-semibold">Serial — DATA</span>
                {dataPort && (
                  <Badge variant="secondary" className="ml-auto">{dataPort}</Badge>
                )}
              </div>
              <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                Baud
                <Select value={String(baud)} onValueChange={(v) => setBaud(+v)}>
                  <SelectTrigger className="w-32"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {BAUDS.map((b) => (<SelectItem key={b} value={String(b)}>{b}</SelectItem>))}
                  </SelectContent>
                </Select>
              </div>
              <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                Parity
                <Select value={parity} onValueChange={(v) => setParity(v as any)} disabled={parityLocked}>
                  <SelectTrigger className="w-32"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="none">none</SelectItem>
                    <SelectItem value="odd">odd</SelectItem>
                    <SelectItem value="even">even</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                Stop bits
                <Select value={String(stopBits)} onValueChange={(v) => setStopBits(+v)} disabled={stopLocked}>
                  <SelectTrigger className="w-32"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="1">1</SelectItem>
                    <SelectItem value="2">2</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <Button size="sm" variant="secondary" disabled={!connected} onClick={applySerial}>
                Apply &amp; reconnect DATA
              </Button>
              <p className="text-[10px] leading-tight text-muted-foreground">
                {!connected
                  ? "Applied on connect. On a Duta only baud reaches the UART (8N1); on a generic adapter all settings apply."
                  : hasCmd
                    ? caps & CAP.PARITY
                      ? "Baud + parity reach the UART; stop bits fixed at 1."
                      : "Duta is 8N1 — only baud reaches the wire (build firmware with PARITY_SUPPORT for parity)."
                    : "Applied to the serial adapter (real baud/parity/stop)."}
              </p>
            </div>
          </PopoverContent>
        </Popover>

        <span className="ml-1 truncate text-xs text-muted-foreground">{status}</span>

        <div className="ml-auto flex items-center gap-2">
          <Select value={selectedPort} onValueChange={setSelectedPort} disabled={connected}>
            <SelectTrigger className="w-44" title="Port to connect">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="auto">
                {ttlPorts.length >= 2 ? "Duta (auto)" : "Duta (none)"}
              </SelectItem>
              {ports.map((p) => (
                <SelectItem key={p.name} value={p.name}>
                  {p.name}
                  {p.is_duta ? " · Duta" : p.product ? ` · ${p.product}` : ""}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Popover open={profilesOpen} onOpenChange={setProfilesOpen}>
            <PopoverTrigger asChild>
              <Button variant="outline" size="sm" title="Saved connections">
                <Bookmark />
              </Button>
            </PopoverTrigger>
            <PopoverContent className="w-72" align="end">
              <div className="flex flex-col gap-2">
                <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                  Saved connections
                </div>
                {profiles.length === 0 && (
                  <p className="text-[11px] text-muted-foreground">None yet — save one below.</p>
                )}
                {profiles.map((p) => (
                  <div key={p.id} className="flex items-center gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 min-w-0 flex-1 justify-start gap-2"
                      onClick={() => loadProfile(p)}
                      title={p.target}
                    >
                      <span className="truncate">{p.name}</span>
                      <span className="ml-auto shrink-0 text-[10px] text-muted-foreground">
                        {p.transport}
                      </span>
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="size-7 text-muted-foreground"
                      title="Delete"
                      onClick={() => deleteProfile(p.id)}
                    >
                      <Trash2 />
                    </Button>
                  </div>
                ))}
                <div className="flex items-center gap-1 border-t pt-2">
                  <Input
                    className="h-8"
                    placeholder={selectedPort === "auto" ? "name (auto Duta)" : `name (${selectedPort})`}
                    value={profileName}
                    onChange={(e) => setProfileName(e.target.value)}
                  />
                  <Button size="sm" onClick={addProfile} title="Save current selection">
                    <Plus />
                  </Button>
                </div>
              </div>
            </PopoverContent>
          </Popover>
          <Button variant="outline" size="sm" onClick={refreshPorts} title="Rescan ports">
            <Usb />
          </Button>
          {connected ? (
            <Button variant="destructive" size="sm" onClick={handleDisconnect}>
              <PlugZap /> Disconnect
            </Button>
          ) : (
            <Button size="sm" onClick={handleConnect} disabled={selectedPort === "auto" && ttlPorts.length < 2}>
              <Plug /> Connect
            </Button>
          )}
          <Button variant="ghost" size="icon" className="size-8" title="Settings" onClick={() => setSettingsOpen(true)}>
            <Cog />
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1 gap-3 p-3">
        <Card className="flex min-w-0 flex-1 flex-col overflow-hidden">
          <CardHeader className="flex-row items-center justify-between border-b py-2">
            <CardTitle>Console — DATA</CardTitle>
            <span className="text-xs text-muted-foreground">
              {baud} 8{parity[0].toUpperCase()}
              {stopBits}
            </span>
          </CardHeader>
          <CardContent className="min-h-0 flex-1 bg-[#0a0a0b] p-2">
            <Terminal ref={terminalRef} connected={connected} />
          </CardContent>
        </Card>

        <div className="flex w-80 shrink-0 flex-col gap-3 overflow-y-auto">
          {/* controls — self-described by the device */}
          <Card>
            <CardHeader className="flex-row items-center py-3">
              <CardTitle>Controls</CardTitle>
              {deviceName && (
                <Badge variant="secondary" className="ml-auto max-w-[10rem] truncate">
                  {deviceName}
                </Badge>
              )}
            </CardHeader>
            <CardContent className="flex flex-col gap-2">
              {controls.length > 0 ? (
                <div className="grid grid-cols-3 gap-2">
                  {controls.map((c) => {
                    const on = !!(outBitmap & (1 << c.index));
                    return (
                      <Button
                        key={c.index}
                        variant={on ? "default" : "outline"}
                        size="sm"
                        disabled={!connected || !hasCmd}
                        onClick={() => toggle(c.index)}
                        className="flex h-auto flex-col py-2"
                      >
                        <span className={on ? "" : "text-muted-foreground"}>{c.name}</span>
                        <span className="text-[10px]">{on ? "ON" : "OFF"}</span>
                      </Button>
                    );
                  })}
                </div>
              ) : (
                <p className="text-[10px] text-muted-foreground">
                  {connected && !hasCmd
                    ? "Generic serial port — no device controls."
                    : "No controls reported."}
                </p>
              )}
            </CardContent>
          </Card>

          {/* run queue — in-flight macros (cancellable) */}
          {runs.length > 0 && (
            <Card>
              <CardHeader className="flex-row items-center py-3">
                <CardTitle>Running</CardTitle>
                <Badge variant="secondary" className="ml-auto">{runs.length}</Badge>
              </CardHeader>
              <CardContent className="flex flex-col gap-1.5">
                {runs.map((r) => (
                  <div key={r.id} className="flex items-center gap-2 rounded-md border px-2 py-1.5">
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-sm">{r.name}</div>
                      <div className="truncate font-mono text-[10px] text-muted-foreground">
                        {r.status}
                      </div>
                    </div>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="size-7 text-muted-foreground hover:text-destructive"
                      title="Cancel run"
                      onClick={() => cancelRun(r.id)}
                    >
                      <X />
                    </Button>
                  </div>
                ))}
              </CardContent>
            </Card>
          )}

          {/* macros */}
          <Card className="flex min-h-0 flex-1 flex-col">
            <CardHeader className="flex-col items-stretch gap-2 py-3">
              <div className="flex items-center gap-1">
                <CardTitle>Macros</CardTitle>
                <Button size="icon" variant="ghost" className="ml-auto size-7" title="Import a set (.json)" onClick={doImport}>
                  <Upload />
                </Button>
                <Button size="icon" variant="ghost" className="size-7" title="Export this set (.json)" onClick={doExport}>
                  <Download />
                </Button>
                <Button size="icon" variant="ghost" className="size-7" title="New macro" onClick={openAdd}>
                  <Plus />
                </Button>
              </div>
              <Select value={setFilter || "__all"} onValueChange={(v) => setSetFilter(v === "__all" ? "" : v)}>
                <SelectTrigger className="h-7 text-xs" title="Project / set">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__all">All sets</SelectItem>
                  {sets.map((s) => (
                    <SelectItem key={s} value={s}>{s}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </CardHeader>
            <CardContent className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto">
              {shownMacros.length === 0 && (
                <p className="py-4 text-center text-xs text-muted-foreground">
                  {macros.length === 0 ? "No macros yet." : "No macros in this set."}
                </p>
              )}
              {shownMacros.map((s, i) => (
                <div
                  key={s.name}
                  draggable
                  onDragStart={(e) => {
                    setDragName(s.name);
                    setOverName(s.name);
                    e.dataTransfer.effectAllowed = "move";
                    e.dataTransfer.setData("text/plain", s.name); // Firefox needs payload
                  }}
                  onDragOver={(e) => {
                    e.preventDefault();
                    e.dataTransfer.dropEffect = "move";
                    if (s.name === dragName) return;
                    const r = e.currentTarget.getBoundingClientRect();
                    const after = e.clientY > r.top + r.height / 2;
                    if (overName !== s.name || insertAfter !== after) {
                      setOverName(s.name);
                      setInsertAfter(after);
                    }
                  }}
                  onDrop={(e) => {
                    e.preventDefault();
                    commitReorder();
                  }}
                  onDragEnd={() => {
                    setDragName(null);
                    setOverName(null);
                  }}
                  className={cn(
                    "flex items-center gap-1 rounded-md border px-2 py-1.5",
                    dragName === s.name && "border-dashed border-primary/60 opacity-40"
                  )}
                >
                  <GripVertical className="size-3.5 shrink-0 cursor-grab text-muted-foreground/50" />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1">
                      {s.secret && <Lock className="size-3 shrink-0 text-muted-foreground" />}
                      <span className="truncate text-sm">{s.name}</span>
                      {!setFilter && s.set && (
                        <Badge variant="secondary" className="ml-auto shrink-0 px-1 py-0 text-[9px]">
                          {s.set}
                        </Badge>
                      )}
                    </div>
                    <div className="truncate font-mono text-[11px] text-muted-foreground">
                      {s.secret ? "••••••••" : s.text.replace(/\n/g, " ⏎ ")}
                    </div>
                  </div>
                  <Button variant="ghost" size="icon" className="size-7" disabled={!connected} title="Run on target" onClick={() => { runText(s.text, s.name); focusTerm(); }}>
                    <Play />
                  </Button>
                  {hasCmd && (caps & CAP.STORE) !== 0 && (
                    <Button variant="ghost" size="icon" className="size-7" title="Save to Duta" onClick={() => saveToDevice(s, i)}>
                      <Database />
                    </Button>
                  )}
                  <Button variant="ghost" size="icon" className="size-7 text-muted-foreground" title="Edit" onClick={() => openEdit(s)}>
                    <Pencil />
                  </Button>
                </div>
              ))}
            </CardContent>
          </Card>
        </div>
      </div>

      {/* add / edit macro modal */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className={cn(showHelp && "max-w-3xl")}>
          <DialogHeader>
            <div className="flex items-center gap-2">
              <DialogTitle>{editOrig ? "Edit macro" : "New macro"}</DialogTitle>
              <Button
                variant="ghost"
                size="icon"
                className={cn("size-7", showHelp && "text-primary")}
                title="Macro command reference"
                onClick={() => setShowHelp((v) => !v)}
              >
                <CircleHelp />
              </Button>
            </div>
          </DialogHeader>

          <div className="flex gap-4">
            <div className="flex min-w-0 flex-1 flex-col gap-3">
              <div className="flex gap-2">
                <Input placeholder="name" value={draftName} onChange={(e) => setDraftName(e.target.value)} />
                <Input
                  className="w-36"
                  placeholder="set (optional)"
                  value={draftSet}
                  onChange={(e) => setDraftSet(e.target.value)}
                  list="macro-sets"
                />
                <datalist id="macro-sets">
                  {sets.map((s) => (
                    <option key={s} value={s} />
                  ))}
                </datalist>
              </div>
              <Textarea
                placeholder={"login\nDELAY 1000\nSTRING whoami\nENTER"}
                value={draftText}
                onChange={(e) => setDraftText(e.target.value)}
                className="h-44 resize-none font-mono text-xs"
              />
              {!showHelp && (
                <p className="text-[10px] leading-tight text-muted-foreground">
                  One command per line. <code>STRING</code> <code>ENTER</code> <code>DELAY ms</code>{" "}
                  <code>WAITFOR text</code> <code>RUN cmd</code> <code>IF OK…END</code> — or tap{" "}
                  <CircleHelp className="inline size-3" /> for the full reference.
                </p>
              )}
              <Button variant="ghost" size="sm" className="w-fit gap-1.5" onClick={() => setDraftSecret(!draftSecret)}>
                {draftSecret ? <Lock className="size-3.5" /> : <LockOpen className="size-3.5" />}
                {draftSecret ? "Secret (hidden from MCP)" : "Not secret"}
              </Button>
              <DialogFooter>
                {editOrig && (
                  <Button variant="destructive" size="sm" className="mr-auto" onClick={deleteMacro}>
                    <Trash2 /> Delete
                  </Button>
                )}
                <DialogClose asChild>
                  <Button variant="ghost" size="sm">Cancel</Button>
                </DialogClose>
                <Button size="sm" onClick={saveMacro}>{editOrig ? "Save" : "Add"}</Button>
              </DialogFooter>
            </div>

            {showHelp && <MacroHelp />}
          </div>
        </DialogContent>
      </Dialog>

      {/* settings modal */}
      <Dialog open={settingsOpen} onOpenChange={setSettingsOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Settings</DialogTitle>
          </DialogHeader>

          <div className="flex flex-col gap-4 py-1">
            <div className="flex flex-col gap-2">
              <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                MCP server
              </div>
              <div className="flex items-center justify-between gap-3">
                <div>
                  <div className="text-sm">Auto-start on launch</div>
                  <div className="text-[11px] text-muted-foreground">
                    Start the MCP server automatically when the app opens.
                  </div>
                </div>
                <Switch
                  checked={settings.autoStartMcp}
                  onCheckedChange={(v) => setSetting("autoStartMcp", v)}
                />
              </div>
              <div className="flex items-center justify-between gap-3">
                <div className="text-sm">Port</div>
                <Input
                  type="number"
                  className="h-8 w-24"
                  value={settings.mcpPort}
                  onChange={(e) => setSetting("mcpPort", +e.target.value)}
                />
              </div>
            </div>

            <div className="flex flex-col gap-2 border-t pt-3">
              <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                Connection
              </div>
              <div className="flex items-center justify-between gap-3">
                <div>
                  <div className="text-sm">Remember last port</div>
                  <div className="text-[11px] text-muted-foreground">
                    Preselect the last connected port on launch.
                  </div>
                </div>
                <Switch
                  checked={settings.rememberLastPort}
                  onCheckedChange={(v) => setSetting("rememberLastPort", v)}
                />
              </div>
              <div className="text-[11px] text-muted-foreground">
                Last used: <code>{settings.lastPort}</code>
              </div>
            </div>

            <div className="flex flex-col gap-2 border-t pt-3">
              <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                MCP tools exposed to the LLM
              </div>
              {MCP_TOOL_OPTIONS.map((o) => (
                <div key={o.key} className="flex items-center justify-between gap-3">
                  <div>
                    <div className="text-sm">{o.label}</div>
                    <div className="font-mono text-[10px] text-muted-foreground">{o.hint}</div>
                  </div>
                  <Switch
                    checked={settings.mcpTools[o.key]}
                    onCheckedChange={(v) => setToolFlag(o.key, v)}
                  />
                </div>
              ))}
              <p className="text-[10px] leading-tight text-muted-foreground">
                Disabled tools are hidden from the model entirely. Changing these restarts a running
                MCP server.
              </p>
            </div>
          </div>

          <DialogFooter>
            <DialogClose asChild>
              <Button size="sm">Done</Button>
            </DialogClose>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
