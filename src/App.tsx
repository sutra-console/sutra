import { useEffect, useState } from "react";
import {
  Usb, Plug, PlugZap, Play, Plus, Trash2, Cpu, Settings2, Bot, Database, Copy, Lock, LockOpen,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import { Terminal } from "@/components/Terminal";
import {
  autodetect,
  connect as ttlConnect,
  disconnect as ttlDisconnect,
  listPorts,
  outputToggle,
  outputGet,
  runText,
  setDataParams,
  saveSnippetToDevice,
  snippetsGet,
  snippetUpsert,
  snippetDelete,
  onSnippets,
  mcpStart,
  mcpStop,
  mcpStatus,
  OUTPUT,
  type PortDesc,
  type McpStatus,
  type SnippetRec,
} from "@/lib/ttl";

const BAUDS = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600, 1500000];

export default function App() {
  const [ports, setPorts] = useState<PortDesc[]>([]);
  const [connected, setConnected] = useState(false);
  const [hasCmd, setHasCmd] = useState(false);
  const [selectedPort, setSelectedPort] = useState("auto");
  const [status, setStatus] = useState("disconnected");
  const [outputs, setOutputs] = useState({ r1: false, r2: false, led: false });
  const [snippets, setSnippets] = useState<SnippetRec[]>([]);
  const [newName, setNewName] = useState("");
  const [newText, setNewText] = useState("");

  const [baud, setBaud] = useState(115200);
  const [parity, setParity] = useState<"none" | "odd" | "even">("none");
  const [stopBits, setStopBits] = useState(1);

  const [mcp, setMcp] = useState<McpStatus>({ running: false, url: null });
  const [mcpPort, setMcpPort] = useState(8765);

  useEffect(() => {
    document.documentElement.classList.add("dark");
    refreshPorts();
    snippetsGet().then(setSnippets).catch(() => {});
    mcpStatus().then(setMcp).catch(() => {});
    const un = onSnippets(setSnippets);
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, []);

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
  }

  async function addSnippet() {
    if (!newName.trim() || !newText) return;
    await snippetUpsert(newName.trim(), newText, false);
    setNewName("");
    setNewText("");
  }

  async function saveToDevice(s: SnippetRec, index: number) {
    setStatus(`saving "${s.name}" to buddi…`);
    try {
      const res = await saveSnippetToDevice(index, s.name, s.text);
      setStatus(`"${s.name}" → ${res}`);
    } catch (e) {
      setStatus(`save failed: ${e}`);
    }
  }

  async function toggleMcp() {
    try {
      setMcp(mcp.running ? await mcpStop() : await mcpStart(mcpPort));
    } catch (e) {
      setStatus(`mcp failed: ${e}`);
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
        {mcp.running && (
          <Badge variant="outline" className="gap-1">
            <Bot className="size-3" /> MCP
          </Badge>
        )}
        <span className="ml-2 truncate text-xs text-muted-foreground">{status}</span>
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
            <Button
              size="sm"
              onClick={handleConnect}
              disabled={selectedPort === "auto" && ttlPorts.length < 2}
            >
              <Plug /> Connect
            </Button>
          )}
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
            <Terminal connected={connected} />
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
                    {BAUDS.map((b) => (
                      <SelectItem key={b} value={String(b)}>{b}</SelectItem>
                    ))}
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

          {/* MCP */}
          <Card>
            <CardHeader className="flex-row items-center gap-2 py-3">
              <Bot className="size-4" />
              <CardTitle>MCP server</CardTitle>
              <Badge variant={mcp.running ? "success" : "secondary"} className="ml-auto">
                {mcp.running ? "on" : "off"}
              </Badge>
            </CardHeader>
            <CardContent className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <span className="text-xs text-muted-foreground">Port</span>
                <Input
                  type="number"
                  className="h-8 w-24"
                  value={mcpPort}
                  disabled={mcp.running}
                  onChange={(e) => setMcpPort(+e.target.value)}
                />
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
            </CardContent>
          </Card>

          {/* snippets */}
          <Card className="flex min-h-0 flex-1 flex-col">
            <CardHeader className="py-3">
              <CardTitle>Snippets</CardTitle>
            </CardHeader>
            <CardContent className="flex min-h-0 flex-1 flex-col gap-2">
              <div className="flex flex-col gap-1.5 overflow-y-auto">
                {snippets.length === 0 && (
                  <p className="py-4 text-center text-xs text-muted-foreground">No snippets yet.</p>
                )}
                {snippets.map((s, i) => (
                  <div key={s.name} className="flex items-center gap-1 rounded-md border px-2 py-1.5">
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-1">
                        {s.secret && <Lock className="size-3 shrink-0 text-muted-foreground" />}
                        <span className="truncate text-sm">{s.name}</span>
                      </div>
                      <div className="truncate font-mono text-[11px] text-muted-foreground">
                        {s.secret ? "••••••••" : s.text}
                      </div>
                    </div>
                    <Button variant="ghost" size="icon" className="size-7" disabled={!connected} title="Run on target" onClick={() => runText(s.text)}>
                      <Play />
                    </Button>
                    <Button variant="ghost" size="icon" className="size-7" disabled={!connected} title="Save to buddi (EEPROM)" onClick={() => saveToDevice(s, i)}>
                      <Database />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="size-7 text-muted-foreground"
                      title={s.secret ? "Unmark secret" : "Mark secret"}
                      onClick={() => snippetUpsert(s.name, s.text, !s.secret)}
                    >
                      {s.secret ? <Lock /> : <LockOpen />}
                    </Button>
                    <Button variant="ghost" size="icon" className="size-7 text-muted-foreground" title="Delete" onClick={() => snippetDelete(s.name)}>
                      <Trash2 />
                    </Button>
                  </div>
                ))}
              </div>
              <div className="mt-auto flex flex-col gap-1.5 border-t pt-2">
                <Input placeholder="name" value={newName} onChange={(e) => setNewName(e.target.value)} />
                <Textarea
                  placeholder={"text / macro…\nlogin\n+++DELAY 1000+++\nwhoami +++ENTER+++"}
                  value={newText}
                  onChange={(e) => setNewText(e.target.value)}
                  className="h-20 resize-none font-mono text-xs"
                />
                <p className="text-[10px] leading-tight text-muted-foreground">
                  Newlines send as typed. Directives: <code>+++DELAY 3000+++</code>,{" "}
                  <code>+++ENTER+++</code>, <code>+++CTRL C+++</code>, <code>+++HEX 1b 5b 41+++</code>.
                </p>
                <Button size="sm" variant="secondary" onClick={addSnippet}>
                  <Plus /> Add snippet
                </Button>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
