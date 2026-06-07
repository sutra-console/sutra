import { useEffect, useRef, useState } from "react";
import {
  Usb, Plug, PlugZap, Play, Plus, Trash2, Cpu, Settings2, Bot, Database, Copy, Lock, LockOpen, Pencil, GripVertical, Cog, CircleHelp,
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
import {
  autodetect,
  connect as ttlConnect,
  disconnect as ttlDisconnect,
  listPorts,
  connState,
  outputToggle,
  outputGet,
  runText,
  setDataParams,
  saveSnippetToDevice,
  snippetsGet,
  snippetUpsert,
  snippetDelete,
  snippetsSet,
  onSnippets,
  mcpStart,
  mcpStop,
  mcpStatus,
  setMcpTools,
  OUTPUT,
  type PortDesc,
  type McpStatus,
  type McpToolFlags,
  type SnippetRec,
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
  mcpPort: 8765,
  rememberLastPort: true,
  lastPort: "auto",
  mcpTools: {
    consoleRead: true,
    consoleWrite: true,
    outputs: true,
    snippetsRun: true,
    snippetsCreate: true,
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

const MCP_TOOL_OPTIONS: { key: keyof McpToolFlags; label: string; hint: string }[] = [
  { key: "consoleRead", label: "Read console", hint: "read_console" },
  { key: "consoleWrite", label: "Write console", hint: "write_console" },
  { key: "outputs", label: "Outputs & device info", hint: "set/get_output, device_info" },
  { key: "snippetsRun", label: "List / run snippets", hint: "list_snippets, run_snippet" },
  { key: "snippetsCreate", label: "Create snippets", hint: "create_snippet" },
  { key: "connection", label: "Connection control", hint: "list ports, connect, set serial" },
];

export default function App() {
  const [ports, setPorts] = useState<PortDesc[]>([]);
  const [connected, setConnected] = useState(false);
  const [hasCmd, setHasCmd] = useState(false);
  const [selectedPort, setSelectedPort] = useState("auto");
  const [status, setStatus] = useState("disconnected");
  const [outputs, setOutputs] = useState({ r1: false, r2: false, led: false });
  const [snippets, setSnippets] = useState<SnippetRec[]>([]);

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

  // snippet add/edit dialog
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editOrig, setEditOrig] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [draftText, setDraftText] = useState("");
  const [draftSecret, setDraftSecret] = useState(false);
  const [showHelp, setShowHelp] = useState(false);

  // snippet drag-reorder (live projected order + ghost)
  const [dragName, setDragName] = useState<string | null>(null);
  const [overName, setOverName] = useState<string | null>(null);
  const [insertAfter, setInsertAfter] = useState(false);

  // the list as it would look if dropped right now
  const orderedSnippets = (() => {
    if (!dragName || !overName || dragName === overName) return snippets;
    const moved = snippets.find((s) => s.name === dragName);
    if (!moved) return snippets;
    const arr = snippets.filter((s) => s.name !== dragName);
    let idx = arr.findIndex((s) => s.name === overName);
    if (idx === -1) return snippets;
    if (insertAfter) idx += 1;
    arr.splice(idx, 0, moved);
    return arr;
  })();

  function commitReorder() {
    if (dragName && overName && dragName !== overName) {
      setSnippets(orderedSnippets); // optimistic; backend confirms via ttl://snippets
      snippetsSet(orderedSnippets).catch(() => {});
    }
    setDragName(null);
    setOverName(null);
  }

  useEffect(() => {
    document.documentElement.classList.add("dark");
    refreshPorts();
    snippetsGet().then(setSnippets).catch(() => {});
    syncConnState(); // adopt a connection the backend already holds (after a reload)

    // push saved MCP tool toggles to the backend
    setMcpTools(settings.mcpTools).catch(() => {});

    // remember-last-port: preselect it in the dropdown
    if (settings.rememberLastPort && settings.lastPort) setSelectedPort(settings.lastPort);

    // MCP: reflect current status, then auto-start on launch if enabled
    mcpStatus().then((st) => {
      setMcp(st);
      if (!st.running && settings.autoStartMcp) {
        mcpStart(settings.mcpPort).then(setMcp).catch(() => {});
      }
    }).catch(() => {});

    const un = onSnippets(setSnippets);
    return () => {
      un.then((f) => f()).catch(() => {});
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
      setSelectedPort(cs.has_cmd ? "auto" : cs.data_port ?? "auto");
      setStatus(
        cs.has_cmd
          ? `sutra — DATA ${cs.data_port} · CMD ${cs.cmd_port}`
          : `serial — ${cs.data_port} @ ${cs.params.baud}`
      );
      if (cs.has_cmd) refreshOutputs();
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
        setStatus(`sutra — DATA ${data} · CMD ${cmd}`);
        refreshOutputs();
      } else {
        await ttlConnect(selectedPort, null);
        setHasCmd(false);
        setStatus(`serial — ${selectedPort} @ ${baud}`);
      }
      setConnected(true);
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
      setOutputs(await outputGet());
    } catch {
      /* device may not implement OUTPUT_GET */
    }
  }

  async function toggle(index: number) {
    try {
      await outputToggle(index);
      refreshOutputs();
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
    setDialogOpen(true);
  }
  function openEdit(s: SnippetRec) {
    setEditOrig(s.name);
    setDraftName(s.name);
    setDraftText(s.text);
    setDraftSecret(s.secret);
    setDialogOpen(true);
  }
  async function saveSnippet() {
    const name = draftName.trim();
    if (!name || !draftText) return;
    if (editOrig && editOrig !== name) await snippetDelete(editOrig);
    await snippetUpsert(name, draftText, draftSecret);
    setDialogOpen(false);
  }
  async function deleteSnippet() {
    if (editOrig) await snippetDelete(editOrig);
    setDialogOpen(false);
  }

  async function saveToDevice(s: SnippetRec, index: number) {
    setStatus(`saving "${s.name}" to buddi…`);
    try {
      setStatus(`"${s.name}" → ${await saveSnippetToDevice(index, s.name, s.text)}`);
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

  const ttlPorts = ports.filter((p) => p.is_sutra);

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <header className="flex items-center gap-3 border-b px-4 py-2.5">
        <Cpu className="size-5 text-primary" />
        <span className="font-semibold tracking-tight">sutra</span>
        <Badge variant={connected ? "success" : "secondary"} className="ml-1">
          {connected ? "online" : "offline"}
        </Badge>

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
                Lets an LLM read the console, run/author snippets &amp; control outputs. Snippet
                contents (secrets) are never exposed — it can only run them by name.
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
                {ttlPorts.length >= 2 ? "sutra (auto)" : "sutra (none)"}
              </SelectItem>
              {ports.map((p) => (
                <SelectItem key={p.name} value={p.name}>
                  {p.name}
                  {p.is_sutra ? " · buddy" : p.product ? ` · ${p.product}` : ""}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
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
          {/* serial settings */}
          <Card>
            <CardHeader className="flex-row items-center gap-2 py-3">
              <Settings2 className="size-4" />
              <CardTitle>Serial — DATA</CardTitle>
            </CardHeader>
            <CardContent className="flex flex-col gap-2">
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
                <Select value={parity} onValueChange={(v) => setParity(v as any)}>
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
                <Select value={String(stopBits)} onValueChange={(v) => setStopBits(+v)}>
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
                Baud reaches the TTL UART (auto-followed). Parity/stop set the host CDC only until
                firmware UART support lands.
              </p>
            </CardContent>
          </Card>

          {/* outputs (sutra only) */}
          <Card>
            <CardHeader className="flex-row items-center py-3">
              <CardTitle>Outputs — CMD</CardTitle>
              <Badge variant="secondary" className="ml-auto">sutra</Badge>
            </CardHeader>
            <CardContent className="flex flex-col gap-2">
              <div className="grid grid-cols-3 gap-2">
                {[
                  { label: "Relay 1", on: outputs.r1, idx: OUTPUT.R1, hint: "P3.4" },
                  { label: "Relay 2", on: outputs.r2, idx: OUTPUT.R2, hint: "P3.3" },
                  { label: "Aux LED", on: outputs.led, idx: OUTPUT.LED, hint: "P1.4 ext" },
                ].map((o) => (
                  <Button
                    key={o.idx}
                    variant={o.on ? "default" : "outline"}
                    size="sm"
                    disabled={!connected || !hasCmd}
                    onClick={() => toggle(o.idx)}
                    title={o.hint}
                    className="flex h-auto flex-col py-2"
                  >
                    <span className={o.on ? "" : "text-muted-foreground"}>{o.label}</span>
                    <span className="text-[10px]">{o.on ? "ON" : "OFF"}</span>
                  </Button>
                ))}
              </div>
              {connected && !hasCmd && (
                <p className="text-[10px] text-muted-foreground">
                  Generic serial port — connect a sutra for relay/LED control.
                </p>
              )}
            </CardContent>
          </Card>

          {/* snippets */}
          <Card className="flex min-h-0 flex-1 flex-col">
            <CardHeader className="flex-row items-center py-3">
              <CardTitle>Snippets</CardTitle>
              <Button size="icon" variant="ghost" className="ml-auto size-7" title="New snippet" onClick={openAdd}>
                <Plus />
              </Button>
            </CardHeader>
            <CardContent className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto">
              {snippets.length === 0 && (
                <p className="py-4 text-center text-xs text-muted-foreground">No snippets yet.</p>
              )}
              {orderedSnippets.map((s, i) => (
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
                    </div>
                    <div className="truncate font-mono text-[11px] text-muted-foreground">
                      {s.secret ? "••••••••" : s.text.replace(/\n/g, " ⏎ ")}
                    </div>
                  </div>
                  <Button variant="ghost" size="icon" className="size-7" disabled={!connected} title="Run on target" onClick={() => { runText(s.text); focusTerm(); }}>
                    <Play />
                  </Button>
                  {hasCmd && (
                    <Button variant="ghost" size="icon" className="size-7" title="Save to buddi (EEPROM)" onClick={() => saveToDevice(s, i)}>
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

      {/* add / edit snippet modal */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className={cn(showHelp && "max-w-3xl")}>
          <DialogHeader>
            <div className="flex items-center gap-2">
              <DialogTitle>{editOrig ? "Edit snippet" : "New snippet"}</DialogTitle>
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
              <Input placeholder="name" value={draftName} onChange={(e) => setDraftName(e.target.value)} />
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
                  <Button variant="destructive" size="sm" className="mr-auto" onClick={deleteSnippet}>
                    <Trash2 /> Delete
                  </Button>
                )}
                <DialogClose asChild>
                  <Button variant="ghost" size="sm">Cancel</Button>
                </DialogClose>
                <Button size="sm" onClick={saveSnippet}>{editOrig ? "Save" : "Add"}</Button>
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
