import { type PointerEvent as ReactPointerEvent, useEffect, useRef, useState } from "react";
import {
  Usb, Plug, PlugZap, Play, Plus, Trash2, Settings2, Bot, Database, Copy, Lock, LockOpen, Pencil, GripVertical, Cog, CircleHelp, Bookmark, X, Download, Upload, Bluetooth, Globe, ChevronDown, PanelRight,
  Radio, Activity, Terminal as TerminalIcon, FolderOpen, LayoutGrid,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { CodeTextarea } from "@/components/CodeTextarea";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import { Popover, PopoverTrigger, PopoverContent } from "@/components/ui/popover";
import {
  DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem,
  DropdownMenuCheckboxItem, DropdownMenuSeparator, DropdownMenuSub,
  DropdownMenuSubTrigger, DropdownMenuSubContent,
} from "@/components/ui/dropdown-menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { Terminal, type TerminalHandle } from "@/components/Terminal";
import { MacroHelp } from "@/components/MacroHelp";
import { MacroVars } from "@/components/MacroVars";
import { clusterName } from "@/lib/zcl";
import { RgbControl } from "@/components/RgbControl";
import { WindowControls } from "@/components/WindowControls";
import logoUrl from "../assets/logo.png";
import { MacroColorStrip } from "@/components/MacroColorStrip";
import { PwmConfigBadge } from "@/components/PwmConfigBadge";
import { BleSnifferPanel } from "@/components/BleSnifferPanel";
import { Ieee154Panel, type NodeSnapshot } from "@/components/Ieee154Panel";
import { YantraCanvas } from "@/components/YantraCanvas";
import { ConfigureDevice } from "@/components/ConfigureDevice";
import { I2cPanel } from "@/components/I2cPanel";
import { NetworkConfig } from "@/components/NetworkConfig";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Slider } from "@/components/ui/slider";
import { save, open } from "@tauri-apps/plugin-dialog";
import {
  autodetect,
  autodetectMux,
  connect as serialConnect,
  connectMuxed,
  bleScan,
  bleConnect,
  type BleDevice,
  wsConnect,
  disconnect as serialDisconnect,
  listPorts,
  connState,
  outputToggle,
  outputsBitmap,
  CFG,
  cfgGet,
  dataPins,
  decodeBleSniff,
  decodeI2cRecord,
  decodeIeee154,
  decodeIeee154Tx,
  onTx,
  getInfo,
  getIoConfig,
  onData,
  setDataKind,
  type BleSniffPacket,
  type Ieee154Frame,
  type I2cRecord,
  getDeviceName,
  getControls,
  getDataDesc,
  DATA_KIND,
  type DataDesc,
  CAP,
  FLAG,
  CTRL,
  outputPwm,
  outputPwmGet,
  pwmConfigGet,
  pwmConfigSet,
  getWorkspace,
  type I2cDef,
  listI2cDefs,
  listYantras,
  type YantraDoc,
  pickWorkspace,
  setWorkspace as adoptWorkspace,
  closeWorkspace as closeWorkspaceApi,
  exportNetworks,
  rgbToHex,
  saveBlePcap,
  saveIeee154Pcap,
  setIeee154Channel,
  getIeee154Channel,
  tsharkAvailable,
  dissectIeee154,
  dataWrite,
  getNetworks,
  setNetworks as saveNetworksApi,
  setNodeName,
  observeFrames,
  type AttrObs,
  type Network,
  type NetNode,
  wifiStatus,
  wsDiscover,
  type DiscoveredDuta,
  type PwmConfig,
  outputRgb,
  outputRgbGet,
  type Rgb,
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
  onConnected,
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
  TIER_INFO,
} from "@/lib/skrit";

const BAUDS = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600, 1500000];

// tint a tier badge: 1=replay (green), 2=interactive (amber), 3=app-only (muted)
const TIER_COLOR: Record<number, string> = {
  1: "border-emerald-500/40 text-emerald-600 dark:text-emerald-400",
  2: "border-amber-500/40 text-amber-600 dark:text-amber-400",
  3: "border-muted-foreground/40 text-muted-foreground",
};

interface Settings {
  autoStartMcp: boolean;
  mcpPort: number;
  rememberLastPort: boolean;
  lastPort: string;
  tsharkPath: string; // optional override for Wireshark's tshark (empty = autodetect)
  autoSave: boolean; // placeholder preference (not yet wired to a behavior)
  mcpTools: McpToolFlags;
}
const DEFAULT_SETTINGS: Settings = {
  autoStartMcp: false,
  autoSave: false,
  mcpPort: 8551,
  rememberLastPort: true,
  lastPort: "auto",
  tsharkPath: "",
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

// Sidebar sections for the settings view (replaces the old single-scroll modal).
type SettingsTab = "general" | "mcp" | "connection" | "decode";
const SETTINGS_SECTIONS: { id: SettingsTab; label: string; icon: typeof Bot }[] = [
  { id: "general", label: "General", icon: Settings2 },
  { id: "mcp", label: "MCP", icon: Bot },
  { id: "connection", label: "Connection", icon: Plug },
  { id: "decode", label: "Packet decode", icon: Radio },
];

export default function App() {
  const [ports, setPorts] = useState<PortDesc[]>([]);
  const [connected, setConnected] = useState(false);
  const [linkOnline, setLinkOnline] = useState(true); // target present on the wire
  const [dataPort, setDataPort] = useState<string | null>(null); // connected DATA port name
  const [bleOpen, setBleOpen] = useState(false); // BLE scan dialog
  const [bleScanning, setBleScanning] = useState(false);
  const [bleDevices, setBleDevices] = useState<BleDevice[]>([]);
  const [wsOpen, setWsOpen] = useState(false); // network (WebSocket) dialog
  const [wsUrl, setWsUrl] = useState("ws://127.0.0.1:9555/");
  const [wsPassword, setWsPassword] = useState("duta");
  const [wsFound, setWsFound] = useState<DiscoveredDuta[]>([]); // mDNS scan results
  const [wsScanning, setWsScanning] = useState(false);
  const [wsConnecting, setWsConnecting] = useState(false);
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
  const [mainView, setMainView] = useState<"data" | "controls">("data"); // card view: data stream vs .yantra controls
  const [yantras, setYantras] = useState<YantraDoc[]>([]); // workspace .yantra control surfaces
  const [yantraSel, setYantraSel] = useState(0); // which .yantra is shown
  const [outBitmap, setOutBitmap] = useState(0); // device output states (bit i = control i)
  const [controls, setControls] = useState<ControlDesc[]>([]); // self-described controls
  const [pwmVals, setPwmVals] = useState<Record<number, number>>({}); // index -> duty 0..1023
  const [pwmCfg, setPwmCfg] = useState<Record<number, PwmConfig>>({}); // index -> {freq, res}
  const [rgbVals, setRgbVals] = useState<Record<number, Rgb[]>>({}); // index -> per-pixel colors
  const [dataDesc, setDataDesc] = useState<DataDesc | null>(null); // what the DATA channel carries
  const [deviceName, setDeviceName] = useState("");
  const [caps, setCaps] = useState(0); // device capability bits (Duta only)
  const [provision, setProvision] = useState(false); // device accepts runtime IO provisioning
  const [configOpen, setConfigOpen] = useState(false);
  const [hasWifi, setHasWifi] = useState(false); // device answers the WiFi CFG keys
  const [ioPins, setIoPins] = useState<Record<number, number>>({}); // output index -> GPIO (tooltips)
  const [dataSrcPins, setDataSrcPins] = useState<{ tx: number; rx: number } | null>(null); // Duta pins the bridged UART enters on
  const [hasKindSwitch, setHasKindSwitch] = useState(false); // device supports CFG DATA_KIND
  const [i2cRecords, setI2cRecords] = useState<I2cRecord[]>([]); // decoded i2c DATA records
  const [blePackets, setBlePackets] = useState<BleSniffPacket[]>([]); // decoded ble-sniff records (last 2000)
  const [bleTotal, setBleTotal] = useState(0); // total received (the buffer is capped)
  const [ieee154Frames, setIeee154Frames] = useState<Ieee154Frame[]>([]); // decoded 802.15.4 frames (last 2000)
  const [ieee154Total, setIeee154Total] = useState(0); // total received (the buffer is capped)
  const [ch154, setCh154] = useState(0); // 802.15.4 sniffer channel (0 = auto-hop, 11..26 = pinned)
  const ch154Ref = useRef(0); // latest channel for the (non-re-subscribing) TX echo listener
  ch154Ref.current = ch154;
  const [tsharkOk, setTsharkOk] = useState(false); // Wireshark tshark present (enables in-app decode)
  const [networks, setNetworks] = useState<Network[]>([]); // workspace network model (keys + nodes)
  const [networksActive, setNetworksActive] = useState(""); // active-network label (preserved on save)
  const [attrs, setAttrs] = useState<Record<string, AttrObs>>({}); // live ZCL attribute values, keyed addr|ep|cluster|attr
  const [draftKey, setDraftKey] = useState("");
  const [draftKeyLabel, setDraftKeyLabel] = useState("");
  const [draftProtocol, setDraftProtocol] = useState(""); // "" = Zigbee, "thread" = Thread/Matter
  const [workspace, setWorkspace] = useState<string | null>(null); // the .sutra workspace folder
  const [recents, setRecents] = useState<string[]>(() => {
    try { return JSON.parse(localStorage.getItem("sutra.recentWorkspaces") || "[]"); } catch { return []; }
  });
  const [i2cDefs, setI2cDefs] = useState<I2cDef[]>([]); // i2c device definitions from .sutra/i2c
  const [i2cPresent, setI2cPresent] = useState<Set<number>>(new Set()); // addresses seen in the last scan
  const [macros, setMacros] = useState<MacroRec[]>([]);
  const [runs, setRuns] = useState<MacroRunInfo[]>([]); // in-flight macro runs

  const [baud, setBaud] = useState(115200);
  const [parity, setParity] = useState<"none" | "odd" | "even">("none");
  const [stopBits, setStopBits] = useState(1);

  const [mcp, setMcp] = useState<McpStatus>({ running: false, url: null });
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("general");
  const [rightBarOpen, setRightBarOpen] = useState(() => localStorage.getItem("sutra.rightBar") !== "0");
  const [rightBarWidth, setRightBarWidth] = useState(() => +(localStorage.getItem("sutra.rightBarWidth") || 320));
  const resizingBar = useRef(false);
  function toggleRightBar() {
    setRightBarOpen((v) => {
      const next = !v;
      localStorage.setItem("sutra.rightBar", next ? "1" : "0");
      return next;
    });
  }
  // Drag the panel's left edge to resize. Width = panel's right inner edge
  // (viewport minus the p-3 padding) minus the cursor x. Clamped + persisted.
  function startBarResize(e: ReactPointerEvent) {
    resizingBar.current = true;
    e.currentTarget.setPointerCapture(e.pointerId);
  }
  function onBarResize(e: ReactPointerEvent) {
    if (!resizingBar.current) return;
    const w = Math.min(640, Math.max(240, window.innerWidth - 12 - e.clientX));
    setRightBarWidth(w);
  }
  function endBarResize(e: ReactPointerEvent) {
    resizingBar.current = false;
    try { e.currentTarget.releasePointerCapture(e.pointerId); } catch { /* not captured */ }
    localStorage.setItem("sutra.rightBarWidth", String(rightBarWidth));
  }
  const setSetting = <K extends keyof Settings>(k: K, v: Settings[K]) =>
    setSettings((s) => ({ ...s, [k]: v }));

  const terminalRef = useRef<TerminalHandle>(null);
  const focusTerm = () => setTimeout(() => terminalRef.current?.focus(), 0);

  // macro add/edit dialog
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editOrig, setEditOrig] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [draftText, setDraftText] = useState("");
  const macroTextRef = useRef<HTMLTextAreaElement>(null);
  // Insert a {$…} variable token at the cursor (or replace the selection).
  function insertMacroVar(token: string) {
    const ta = macroTextRef.current;
    if (!ta) {
      setDraftText((t) => (t ? `${t}\n${token}` : token));
      return;
    }
    const start = ta.selectionStart ?? draftText.length;
    const end = ta.selectionEnd ?? start;
    setDraftText(draftText.slice(0, start) + token + draftText.slice(end));
    requestAnimationFrame(() => {
      ta.focus();
      const pos = start + token.length;
      ta.setSelectionRange(pos, pos);
    });
  }
  const [draftSecret, setDraftSecret] = useState(false);
  const [draftSet, setDraftSet] = useState("");
  const [showHelp, setShowHelp] = useState(false);
  const [setFilter, setSetFilter] = useState(""); // active project/set ("" = all)
  const [tierFilter, setTierFilter] = useState(0); // active tier (0 = all)

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
      setMacros(orderedMacros); // optimistic; backend confirms via sutra://macros
      macrosSet(orderedMacros).catch(() => {});
    }
    setDragName(null);
    setOverName(null);
  }

  useEffect(() => {
    document.documentElement.classList.add("dark");
    refreshPorts();
    getWorkspace().then(setWorkspace).catch(() => {});
    listI2cDefs().then(setI2cDefs).catch(() => {});
    listYantras().then(setYantras).catch(() => {});
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
      setStatus(online ? "target online" : "target offline: link lost, retrying…");
    });
    // A connect initiated over MCP (or any backend-side connect) → re-sync the UI
    // so it never shows a stale/errored state while the backend holds the port.
    const unConn = onConnected(() => syncConnState());
    return () => {
      un.then((f) => f()).catch(() => {});
      unRuns.then((f) => f()).catch(() => {});
      unLink.then((f) => f()).catch(() => {});
      unConn.then((f) => f()).catch(() => {});
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
          ? `Duta: DATA ${cs.data_port} · CMD ${cs.cmd_port}`
          : `serial: ${cs.data_port} @ ${cs.params.baud}`
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
        try {
          // Two Duta ports → dual-CDC.
          const { data, cmd } = await autodetect();
          await serialConnect(data, cmd);
          setHasCmd(true);
          setDataPort(data);
          setStatus(`Duta: DATA ${data} · CMD ${cmd}`);
        } catch {
          // No dual-CDC pair → probe candidates for a single-port muxed board
          // (ESP32 / Pico / nRF52840); the backend confirms with a mux PING.
          const muxPort = await autodetectMux();
          await connectMuxed(muxPort);
          setHasCmd(true);
          setDataPort(muxPort);
          setStatus(`Duta (muxed): ${muxPort}`);
        }
        loadDevice();
      } else if (ports.some((p) => p.name === selectedPort && p.is_duta)) {
        // A manually-picked Duta: connect MUXED so CMD + the proper viewer
        // (802.15.4 / I²C / BLE) light up — don't treat it as a dumb serial port.
        // Fall back to raw serial if the mux PING fails (not actually muxed).
        try {
          await connectMuxed(selectedPort);
          setHasCmd(true);
          setDataPort(selectedPort);
          setStatus(`Duta (muxed): ${selectedPort}`);
          loadDevice();
        } catch {
          await serialConnect(selectedPort, null);
          setHasCmd(false);
          setDataPort(selectedPort);
          clearDevice();
          setStatus(`serial: ${selectedPort} @ ${baud}`);
        }
      } else {
        await serialConnect(selectedPort, null);
        setHasCmd(false);
        setDataPort(selectedPort);
        clearDevice();
        setStatus(`serial: ${selectedPort} @ ${baud}`);
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

  async function handleBleScan() {
    setBleScanning(true);
    setBleDevices([]);
    try {
      setBleDevices(await bleScan());
    } catch (e) {
      setStatus(`BLE scan failed: ${e}`);
    } finally {
      setBleScanning(false);
    }
  }

  async function handleBleConnect(d: BleDevice) {
    setStatus(`connecting BLE ${d.name || d.id}…`);
    try {
      const name = await bleConnect(d.id);
      setBleOpen(false);
      setHasCmd(true);
      setDataPort(`BLE: ${name}`);
      setConnected(true);
      setLinkOnline(true);
      setStatus(`Duta (BLE): ${name}`);
      loadDevice();
      focusTerm();
    } catch (e) {
      setStatus(`BLE connect failed: ${e}`);
    }
  }

  async function handleWsConnect() {
    setWsConnecting(true);
    setStatus(`connecting ${wsUrl}…`);
    try {
      const res = await wsConnect(wsUrl, wsPassword);
      setWsOpen(false);
      setHasCmd(true);
      setDataPort("WebSocket");
      setConnected(true);
      setLinkOnline(true);
      loadDevice();
      focusTerm();
      setStatus(
        res.default_cred
          ? `Duta (network): ${res.name} ⚠ default password, change it`
          : `Duta (network): ${res.name}`,
      );
    } catch (e) {
      setStatus(`network connect failed: ${e}`);
    } finally {
      setWsConnecting(false);
    }
  }

  async function handleDisconnect() {
    await serialDisconnect().catch(() => {});
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
    // Fresh session: clear any capture left over from the previous connection
    // (we keep it on screen across a disconnect; a new connect starts clean).
    setBlePackets([]);
    setBleTotal(0);
    setIeee154Frames([]);
    setIeee154Total(0);
    setI2cRecords([]);
    setI2cPresent(new Set());
    getDeviceName().then(setDeviceName).catch(() => {});
    getInfo()
      .then((i) => {
        setCaps(i.caps);
        setProvision((i.flags & FLAG.PROVISION) !== 0);
        if (i.flags & FLAG.PROVISION) {
          // the IO table carries each output's pin — surface it in tooltips
          getIoConfig()
            .then((rows) => setIoPins(Object.fromEntries(rows.map((r, idx) => [idx, r.pin]))))
            .catch(() => setIoPins({}));
        } else {
          setIoPins({});
        }
      })
      .catch(() => {});
    getDataDesc().then(setDataDesc).catch(() => setDataDesc(null)); // UART if unsupported
    dataPins().then(setDataSrcPins).catch(() => setDataSrcPins(null)); // which Duta pins the source rides
    cfgGet(CFG.DATA_KIND).then(() => setHasKindSwitch(true)).catch(() => setHasKindSwitch(false));
    wifiStatus().then(() => setHasWifi(true)).catch(() => setHasWifi(false));
    try {
      const cs = await getControls();
      setControls(cs);
      // Pull current values for the analog (pwm) and color (rgb) controls.
      const pwm: Record<number, number> = {};
      const cfg: Record<number, PwmConfig> = {};
      const rgb: Record<number, Rgb[]> = {};
      for (const c of cs) {
        if (c.type === CTRL.PWM) {
          pwm[c.index] = await outputPwmGet(c.index).catch(() => 0);
          cfg[c.index] = await pwmConfigGet(c.index).catch(() => ({ freq: 0, res: 0 }));
        } else if (c.type === CTRL.RGB) {
          const v = await outputRgbGet(c.index).catch(() => ({ count: 1, r: 0, g: 0, b: 0 }));
          // The read reports pixel 0; seed every pixel with it until each is set.
          rgb[c.index] = Array.from({ length: Math.max(1, v.count) }, () => ({ r: v.r, g: v.g, b: v.b }));
        }
      }
      setPwmVals(pwm);
      setPwmCfg(cfg);
      setRgbVals(rgb);
    } catch {
      /* device may not self-describe */
    }
    refreshOutputs();
  }

  // Clear live *device* state on disconnect. Deliberately KEEPS the DATA view
  // (dataDesc) and any captured records (ble/i2c) so a sniff/I²C session stays
  // on screen to review and save — and, crucially, so the Terminal does NOT
  // remount and render the tail of an in-flight sniff stream as UTF-8 garbage.
  // The capture buffers are freshened at connect time (loadDevice) instead.
  function clearDevice() {
    setDeviceName("");
    setControls([]);
    setOutBitmap(0);
    setPwmVals({});
    setPwmCfg({});
    setRgbVals({});
    setDataSrcPins(null);
    setIoPins({});
    setCaps(0);
  }

  // For typed DATA kinds the Terminal is unmounted, so App is the only DATA
  // consumer. The sniffer streams ~70 records/s; decode into a buffer and flush
  // to React on an interval (not per-record) so a high-rate stream can't
  // saturate the renderer (which would starve the disconnect click → "freeze").
  useEffect(() => {
    const kind = dataDesc?.kind;
    if (kind !== DATA_KIND.I2C && kind !== DATA_KIND.BLE_SNIFF && kind !== DATA_KIND.IEEE802154) return;
    const bleBuf: BleSniffPacket[] = [];
    const i2cBuf: I2cRecord[] = [];
    const ieeeBuf: Ieee154Frame[] = [];
    let unlisten: (() => void) | undefined;
    onData((bytes) => {
      if (kind === DATA_KIND.I2C) {
        const rec = decodeI2cRecord(bytes);
        if (rec) i2cBuf.push(rec);
      } else if (kind === DATA_KIND.IEEE802154) {
        const f = decodeIeee154(bytes);
        if (f) ieeeBuf.push(f);
      } else {
        const pkt = decodeBleSniff(bytes);
        if (pkt) bleBuf.push(pkt);
      }
    }).then((u) => (unlisten = u));
    // Echo our own injected frames into the 802.15.4 view as TX ("assumed sends")
    // — the radio can't capture its own transmissions, so without this an inject
    // produces no visible feedback.
    let unTx: (() => void) | undefined;
    if (kind === DATA_KIND.IEEE802154) {
      onTx((mac) => {
        const f = decodeIeee154Tx(mac, ch154Ref.current);
        if (f) setIeee154Frames((fs) => [...fs, f].slice(-2000));
      }).then((u) => (unTx = u));
    }
    const flush = window.setInterval(() => {
      if (kind === DATA_KIND.BLE_SNIFF && bleBuf.length) {
        const chunk = bleBuf.splice(0);
        setBlePackets((ps) => [...ps, ...chunk].slice(-2000));
        setBleTotal((t) => t + chunk.length);
      } else if (kind === DATA_KIND.IEEE802154 && ieeeBuf.length) {
        const chunk = ieeeBuf.splice(0);
        setIeee154Frames((fs) => [...fs, ...chunk].slice(-2000));
        setIeee154Total((t) => t + chunk.length);
        // Interview: hand the whole batch to the backend, which decrypts each
        // frame against the active network — ZDP replies feed active discovery,
        // and any other application frame passively records its node's endpoints
        // and clusters. Refresh the node model when something changed.
        if (workspace) {
          observeFrames(chunk.map((f) => f.mac))
            .then((res) => {
              if (res.changed > 0)
                getNetworks().then((n) => { setNetworks(n.networks ?? []); setNetworksActive(n.active ?? ""); }).catch(() => {});
              if (res.attrs.length)
                setAttrs((prev) => {
                  const next = { ...prev };
                  for (const a of res.attrs) next[`${a.addr}|${a.endpoint}|${a.cluster}|${a.attr}`] = a;
                  return next;
                });
            })
            .catch(() => {});
        }
      } else if (kind === DATA_KIND.I2C && i2cBuf.length) {
        const chunk = i2cBuf.splice(0);
        setI2cRecords((rs) => [...rs, ...chunk].slice(-500));
      }
    }, 150);
    return () => {
      unlisten?.();
      unTx?.();
      window.clearInterval(flush);
    };
  }, [dataDesc?.kind]);

  // Read the 802.15.4 sniffer's current channel when its viewer becomes active.
  useEffect(() => {
    if (dataDesc?.kind === DATA_KIND.IEEE802154 && connected) {
      getIeee154Channel().then(setCh154).catch(() => {});
    }
  }, [dataDesc?.kind, connected]);

  // Is Wireshark's tshark available? (enables in-app Zigbee/Thread/Matter decode)
  // Re-checks when the manual path setting changes.
  useEffect(() => {
    tsharkAvailable(settings.tsharkPath).then(setTsharkOk).catch(() => setTsharkOk(false));
  }, [settings.tsharkPath]);

  // Load the workspace's network model — keys + discovered nodes (reload on change).
  useEffect(() => {
    getNetworks().then((n) => { setNetworks(n.networks ?? []); setNetworksActive(n.active ?? ""); }).catch(() => setNetworks([]));
  }, [workspace]);

  // Inject a ZCL command at a peer via the {$zcl} macro var (build + send).
  function runZcl(addr: string, endpoint: number, cluster: number, cmd: number, payloadHex?: string) {
    const a = addr.replace(/^0x/i, "");
    const cl = cluster.toString(16).padStart(4, "0");
    const cm = cmd.toString(16).padStart(2, "0");
    const pl = payloadHex ? ` ${payloadHex}` : "";
    runText(`HEX {$zcl ${a} ${endpoint} ${cl} ${cm}${pl}}`, "zcl").catch((e) => setStatus(`zcl failed: ${e}`));
    setStatus(`sent ${clusterName(cluster)} cmd 0x${cm} → ${addr}`);
  }

  async function saveNetworks(next: Network[]) {
    setNetworks(next);
    try {
      await saveNetworksApi({ networks: next, active: networksActive });
    } catch (e) {
      setStatus(`save network failed: ${e}`);
    }
  }

  // Merge passively-discovered nodes into the workspace network model, grouped by
  // PAN: each PAN becomes (or updates) a network you can then drop a key onto.
  function saveDiscoveredNodes(snaps: NodeSnapshot[]) {
    if (!workspace) {
      setStatus("Select a workspace first to save the network");
      return;
    }
    const next = networks.map((n) => ({ ...n, nodes: [...n.nodes] }));
    const stamp = new Date().toISOString();
    const byPan = new Map<string, NodeSnapshot[]>();
    for (const s of snaps) {
      const list = byPan.get(s.pan) ?? [];
      list.push(s);
      byPan.set(s.pan, list);
    }
    for (const [pan, list] of byPan) {
      let net = next.find((n) => pan && n.pan === pan);
      if (!net) {
        net = { label: pan ? `PAN ${pan}` : "Unknown PAN", pan, channel: list[0]?.channels[0] ?? 0, key: "", nodes: [] };
        next.push(net);
      }
      for (const s of list) {
        const node: NetNode = {
          addr: s.addr, role: s.role, channels: s.channels, count: s.count, lastSeen: stamp,
          ieee: "", manufacturer: "", endpoints: [],
        };
        const existing = net.nodes.find((nd) => nd.addr === s.addr);
        if (existing) Object.assign(existing, { ...node, ieee: existing.ieee, manufacturer: existing.manufacturer, endpoints: existing.endpoints });
        else net.nodes.push(node);
      }
    }
    saveNetworks(next);
    setStatus(`saved ${snaps.length} node(s) across ${byPan.size} network(s)`);
  }
  // Accept what HA/Z2M actually hand you: 32 hex chars (any separators / 0x), or a
  // 16-byte decimal array like [1, 3, 5, …] (the configuration.yaml / diagnostics form).
  function parseZbKey(input: string): string | null {
    const s = input.trim();
    const tokens = s.replace(/[[\]]/g, "").split(/[\s,]+/).filter(Boolean);
    if (tokens.length === 16 && tokens.every((t) => /^\d+$/.test(t) && +t <= 255)) {
      return tokens.map((t) => (+t).toString(16).padStart(2, "0")).join("");
    }
    const hex = s.replace(/0x/gi, "").replace(/[^0-9a-fA-F]/g, "");
    return hex.length === 32 ? hex.toLowerCase() : null;
  }
  function addNetwork() {
    const key = parseZbKey(draftKey);
    if (!key) {
      setStatus("Key must be 32 hex chars, or a 16-byte array (HA/Z2M form)");
      return;
    }
    // If we already discovered a network passively (a keyless PAN entry), drop the
    // key onto it — sniff → Save nodes → paste key lights up decryption in place.
    const label = draftKeyLabel.trim();
    // A Thread key starts a fresh network; a Zigbee key can drop onto a keyless
    // PAN we discovered passively (sniff → Save nodes → paste key lights it up).
    const keyless = draftProtocol === "thread" ? -1 : networks.findIndex((n) => !n.key.trim());
    if (keyless >= 0) {
      saveNetworks(networks.map((n, i) => (i === keyless ? { ...n, key, label: label || n.label } : n)));
    } else {
      saveNetworks([
        ...networks,
        {
          label: label || (draftProtocol === "thread" ? "thread" : `network ${networks.length + 1}`),
          key, pan: "", channel: 0, protocol: draftProtocol, nodes: [],
        },
      ]);
    }
    setDraftKey("");
    setDraftKeyLabel("");
  }

  /** Dissect the current 802.15.4 capture with tshark (Zigbee/Thread/Matter). */
  const decodeIeee154Capture = () =>
    dissectIeee154(ieee154Frames.map((f) => f.raw), settings.tsharkPath);

  /** Inject an 802.15.4 MAC frame (no FCS — the radio appends it) over the air. */
  async function injectIeee154(mac: number[]) {
    try {
      await dataWrite(mac);
      setStatus(`injected ${mac.length}-byte frame`);
    } catch (e) {
      setStatus(`inject failed: ${e}`);
    }
  }

  /** Pin the 802.15.4 sniffer to a channel (0 = auto-hop). */
  async function applyChannel(ch: number) {
    try {
      await setIeee154Channel(ch);
      setCh154(ch);
      setStatus(ch ? `802.15.4 channel ${ch}` : "802.15.4: auto-hop (11–26)");
    } catch (e) {
      setStatus(`channel set failed: ${e}`);
    }
  }

  function rememberRecent(path: string) {
    setRecents((r) => {
      const next = [path, ...r.filter((p) => p !== path)].slice(0, 6);
      localStorage.setItem("sutra.recentWorkspaces", JSON.stringify(next));
      return next;
    });
  }
  function forgetRecent(path: string) {
    setRecents((r) => {
      const next = r.filter((p) => p !== path);
      localStorage.setItem("sutra.recentWorkspaces", JSON.stringify(next));
      return next;
    });
  }
  // Adopt a workspace (or clear it with null) and re-point everything that reads
  // from .sutra/ (macros, i2c defs, yantra; the network model reloads via effect).
  function applyWorkspace(path: string | null) {
    setWorkspace(path);
    if (path) rememberRecent(path);
    macrosGet().then(setMacros).catch(() => {});
    listI2cDefs().then(setI2cDefs).catch(() => {});
    listYantras().then(setYantras).catch(() => {});
  }

  /** Choose the workspace folder (.sutra/ for macros + captures). */
  async function chooseWorkspace() {
    try {
      const path = await pickWorkspace();
      if (path) {
        applyWorkspace(path);
        setStatus(`workspace: ${path}`);
      }
    } catch (e) {
      setStatus(`workspace failed: ${e}`);
    }
  }

  /** Open a previously-used workspace by path (Open Recent). */
  async function openRecent(path: string) {
    try {
      const set = await adoptWorkspace(path);
      applyWorkspace(set);
      setStatus(`workspace: ${set}`);
    } catch (e) {
      setStatus(`open recent failed: ${e}`);
      forgetRecent(path); // path likely gone — drop it from the list
    }
  }

  /** Forget the workspace; macros fall back to the app data dir. */
  async function doCloseWorkspace() {
    try {
      await closeWorkspaceApi();
      applyWorkspace(null);
      setStatus("workspace closed");
    } catch (e) {
      setStatus(`close workspace failed: ${e}`);
    }
  }

  /** Export the current capture as a pcap — whichever medium has frames. */
  function exportCapturePcap() {
    if (ieee154Frames.length) return saveIeee154();
    if (blePackets.length) return saveSniffPcap();
  }

  /** Export the discovered-nodes network model to a JSON file. */
  async function exportNodes() {
    const path = await save({
      defaultPath: "networks.json",
      filters: [{ name: "Network model", extensions: ["json"] }],
    });
    if (!path) return;
    try {
      await exportNetworks(path);
      setStatus(`exported network model → ${path}`);
    } catch (e) {
      setStatus(`export nodes failed: ${e}`);
    }
  }

  /** Save the current BLE capture as a pcap (into the workspace, else a dialog). */
  async function saveSniffPcap() {
    const records = blePackets.map((p) => p.raw);
    if (!records.length) return;
    const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
    try {
      const path = await saveBlePcap(`ble-${stamp}`, records);
      setStatus(`saved ${records.length} packets → ${path}`);
    } catch (e) {
      setStatus(`pcap save failed: ${e}`);
    }
  }

  /** Save the current 802.15.4 capture as a pcap (workspace, else a dialog). */
  async function saveIeee154() {
    const records = ieee154Frames.map((f) => f.raw);
    if (!records.length) return;
    const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
    try {
      const path = await saveIeee154Pcap(`802154-${stamp}`, records);
      setStatus(`saved ${records.length} frames → ${path}`);
    } catch (e) {
      setStatus(`pcap save failed: ${e}`);
    }
  }

  /** Switch the bridged medium and refresh what the device reports. */
  async function switchDataKind(kind: number) {
    try {
      await setDataKind(kind);
      setI2cRecords([]);
      getDataDesc().then(setDataDesc).catch(() => {});
      setStatus(kind === DATA_KIND.I2C ? "DATA: I2C master" : "DATA: UART console");
    } catch (e) {
      setStatus(`kind switch failed: ${e}`);
    }
  }

  /** Right-click → copy a control's state as a runnable macro command. */
  function copyMacroCmd(cmd: string) {
    navigator.clipboard.writeText(cmd).then(
      () => setStatus(`copied: ${cmd}`),
      () => setStatus("copy failed"),
    );
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

  // PWM frequency/resolution change: device returns the actual applied values.
  async function setPwmConfig(index: number, freq: number, res: number) {
    try {
      const actual = await pwmConfigSet(index, freq, res);
      setPwmCfg((p) => ({ ...p, [index]: actual }));
    } catch (e) {
      setStatus(`cmd failed: ${e}`);
    }
  }

  // PWM duty change (analog output). Optimistic local update + fire to device.
  async function setPwm(index: number, duty: number) {
    setPwmVals((p) => ({ ...p, [index]: duty }));
    setOutBitmap((bm) => (duty > 0 ? bm | (1 << index) : bm & ~(1 << index)));
    try {
      await outputPwm(index, duty);
    } catch (e) {
      setStatus(`cmd failed: ${e}`);
    }
  }

  // RGB color change (addressable LED). pixel = undefined fills the whole strip.
  async function setRgb(index: number, color: Rgb, pixel?: number) {
    const next = (rgbVals[index] ?? [{ r: 0, g: 0, b: 0 }]).slice();
    if (pixel === undefined) next.fill(color);
    else next[pixel] = color;
    setRgbVals((p) => ({ ...p, [index]: next }));
    const lit = next.some((c) => c.r || c.g || c.b);
    setOutBitmap((bm) => (lit ? bm | (1 << index) : bm & ~(1 << index)));
    try {
      await outputRgb(index, color, pixel);
    } catch (e) {
      setStatus(`cmd failed: ${e}`);
    }
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
  const shownMacros = orderedMacros.filter(
    (s) => (!setFilter || s.set === setFilter) && (!tierFilter || s.tier === tierFilter)
  );

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

  const dutaPorts = ports.filter((p) => p.is_duta);
  // On a Duta the firmware UART is 8N1 (1 stop, no parity) unless built with
  // PARITY_SUPPORT. On a generic adapter parity/stop are real hardware settings.
  const parityLocked = connected && hasCmd && !(caps & CAP.PARITY);
  const stopLocked = connected && hasCmd;

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      {/* title bar: app identity + status on the left, our own window controls on
          the right (the window is frameless). The strip is a drag region. */}
      <div data-tauri-drag-region className="relative flex h-9 items-center gap-2.5 border-b px-3">
        {/* workspace name, centered — display only (open/close live in the Sutra menu);
            hover shows the full path. Keep it a drag region so the center still drags. */}
        <span
          data-tauri-drag-region
          title={workspace ?? "No workspace selected"}
          className="absolute left-1/2 max-w-[40%] -translate-x-1/2 cursor-default truncate text-xs text-muted-foreground"
        >
          {workspace ? workspace.split(/[/\\]/).pop() : "No workspace"}
        </span>
        {/* app name → Sutra menu (workspace, import/export, preferences, exit) */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="-mx-1 flex items-center gap-2 rounded px-1 py-0.5 hover:bg-accent"
              title="Sutra menu"
            >
              <img src={logoUrl} alt="Sutra" className="size-5 object-contain" draggable={false} />
              <span className="font-semibold tracking-tight">Sutra</span>
              <ChevronDown className="size-3 text-muted-foreground" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-56">
            <DropdownMenuItem onSelect={chooseWorkspace}>
              <FolderOpen className="size-3.5" /> Open workspace…
            </DropdownMenuItem>
            <DropdownMenuSub>
              <DropdownMenuSubTrigger disabled={recents.length === 0}>
                <FolderOpen className="size-3.5" /> Open Recent
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                {recents.map((p) => (
                  <DropdownMenuItem key={p} onSelect={() => openRecent(p)} title={p}>
                    <span className="truncate">{p.split(/[/\\]/).pop()}</span>
                  </DropdownMenuItem>
                ))}
              </DropdownMenuSubContent>
            </DropdownMenuSub>
            <DropdownMenuItem disabled={!workspace} onSelect={doCloseWorkspace}>
              <X className="size-3.5" /> Close workspace
            </DropdownMenuItem>

            <DropdownMenuSeparator />

            {/* Import — only what's actually importable today (macro sets) */}
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <Upload className="size-3.5" /> Import
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                <DropdownMenuItem onSelect={doImport}>Macros…</DropdownMenuItem>
              </DropdownMenuSubContent>
            </DropdownMenuSub>
            {/* Export — show only the kinds we currently have data for */}
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <Download className="size-3.5" /> Export
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                <DropdownMenuItem onSelect={doExport}>Macros…</DropdownMenuItem>
                {(ieee154Frames.length > 0 || blePackets.length > 0) && (
                  <DropdownMenuItem onSelect={exportCapturePcap}>Capture (.pcap)…</DropdownMenuItem>
                )}
                {ieee154Frames.length > 0 && (
                  <DropdownMenuItem onSelect={exportNodes}>Nodes…</DropdownMenuItem>
                )}
              </DropdownMenuSubContent>
            </DropdownMenuSub>

            <DropdownMenuSeparator />

            <DropdownMenuCheckboxItem
              checked={settings.autoSave}
              onCheckedChange={(v) => setSetting("autoSave", v)}
            >
              Auto-save
            </DropdownMenuCheckboxItem>
            <DropdownMenuItem onSelect={() => setSettingsOpen(true)}>
              <Cog className="size-3.5" /> Preferences
            </DropdownMenuItem>

            <DropdownMenuSeparator />

            <DropdownMenuItem onSelect={() => getCurrentWindow().close()}>
              <X className="size-3.5" /> Exit
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <Badge
          variant={!connected ? "secondary" : linkOnline ? "success" : "destructive"}
          className="ml-1"
        >
          {!connected ? "offline" : linkOnline ? "online" : "target offline"}
        </Badge>
        {deviceName && <span className="text-xs text-muted-foreground">· {deviceName}</span>}
        <div className="ml-auto -mr-3 flex h-full items-stretch">
          <WindowControls />
        </div>
      </div>

      {/* toolbar strip: connection + tool controls, beneath the title bar */}
      <header className="flex items-center gap-3 border-b px-4 py-2">
        {/* DATA serial settings popover */}
        <Popover>
          <PopoverTrigger asChild>
            <Button variant="outline" size="sm" className="gap-1.5">
              <Settings2 className="size-3.5" />
              {!connected
                ? "serial"
                : dataDesc?.kind === DATA_KIND.IEEE802154
                  ? `802.15.4 · ${ch154 ? `ch ${ch154}` : "hop"}`
                  : `${dataPort ?? selectedPort} @ ${baud}`}
              {connected && (
                <span className={cn("size-1.5 rounded-full", linkOnline ? "bg-success" : "bg-destructive")} />
              )}
            </Button>
          </PopoverTrigger>
          <PopoverContent align="start" className={dataDesc?.kind === DATA_KIND.IEEE802154 ? "w-80" : "w-72"}>
            <div className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <Settings2 className="size-4" />
                <span className="text-sm font-semibold">
                  {dataDesc?.kind === DATA_KIND.IEEE802154 ? "802.15.4 radio" : "Serial: DATA"}
                </span>
                {dataPort && (
                  <Badge variant="secondary" className="ml-auto">{dataPort}</Badge>
                )}
              </div>
              {dataDesc?.kind === DATA_KIND.IEEE802154 ? (
                <>
                  {/* The bridged medium is a radio, not a UART — so this slot configures
                      the 802.15.4 channel (PROTO_SET) instead of baud/parity/stop. */}
                  <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                    Channel
                    <Select value={String(ch154)} onValueChange={(v) => applyChannel(+v)} disabled={!connected}>
                      <SelectTrigger className="w-40"><SelectValue /></SelectTrigger>
                      <SelectContent>
                        <SelectItem value="0">Auto-hop (11–26)</SelectItem>
                        {Array.from({ length: 16 }, (_, i) => 11 + i).map((c) => (
                          <SelectItem key={c} value={String(c)}>
                            {c} · {2405 + (c - 11) * 5} MHz
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <p className="text-[10px] leading-tight text-muted-foreground">
                    Pin a channel to follow one network, or Auto-hop to scan 11–26 for traffic
                    (sticking where it appears). Applies immediately.
                  </p>

                  {/* Zigbee decryption keys, right where you configure the radio.
                      Saved in the workspace; feeds tshark so frames decode to ZCL. */}
                  {tsharkOk && (
                    <div className="mt-1 flex flex-col gap-1 border-t pt-2">
                      <div className="flex items-center justify-between">
                        <span className="text-xs text-muted-foreground">Zigbee keys</span>
                        <span
                          className="cursor-help text-[10px] text-muted-foreground underline decoration-dotted"
                          title={
                            "Where to find your network key:\n" +
                            "• Home Assistant (ZHA): Settings ▸ Devices & Services ▸ Zigbee Home Automation ▸ ⋮ ▸ Download diagnostics — the JSON has network_info.network_key.\n" +
                            "• Zigbee2MQTT: data/configuration.yaml ▸ advanced ▸ network_key (a 16-byte array).\n" +
                            "Paste hex (32 chars) or the 16-byte array; it's saved to the workspace."
                          }
                        >
                          where?
                        </span>
                      </div>
                      {networks.map((net, i) => (
                        <div key={i} className="flex items-center gap-1 text-[11px]">
                          <span className="min-w-0 flex-1 truncate">
                            <span className="text-muted-foreground">{net.label}</span>{" "}
                            {net.key
                              ? <span className="font-mono">{net.key.slice(0, 8)}…</span>
                              : <span className="text-amber-500">no key</span>}
                            {net.nodes.length > 0 && (
                              <span className="text-muted-foreground"> · {net.nodes.length} nodes</span>
                            )}
                          </span>
                          <button type="button" className="text-muted-foreground hover:text-destructive"
                            title="Remove network" onClick={() => saveNetworks(networks.filter((_, j) => j !== i))}>
                            <X className="size-3" />
                          </button>
                        </div>
                      ))}
                      <div className="flex items-center gap-1">
                        <Input className="h-7 min-w-0 flex-1 font-mono text-[11px]"
                          placeholder="network key (hex or 16-byte array)"
                          value={draftKey} spellCheck={false}
                          onChange={(e) => setDraftKey(e.target.value)}
                          onKeyDown={(e) => e.key === "Enter" && addNetwork()} />
                        <Button size="sm" className="h-7 px-2 text-xs" disabled={!workspace} onClick={addNetwork}>
                          Add
                        </Button>
                      </div>
                      {!workspace && (
                        <p className="text-[10px] text-destructive">Select a workspace to save the network.</p>
                      )}
                    </div>
                  )}
                </>
              ) : (
                <>
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
                          : "Duta is 8N1, so only baud reaches the wire (build firmware with PARITY_SUPPORT for parity)."
                        : "Applied to the serial adapter (real baud/parity/stop)."}
                  </p>
                </>
              )}
            </div>
          </PopoverContent>
        </Popover>

        <div className="ml-auto flex items-center gap-2">
          <Select value={selectedPort} onValueChange={setSelectedPort} disabled={connected}>
            <SelectTrigger className="w-44" title="Port to connect">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="auto">
                {connected && deviceName
                  ? `Duta (${deviceName})`
                  : dutaPorts.length > 0
                    ? "Duta (auto)"
                    : "Duta (none)"}
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
                  <p className="text-[11px] text-muted-foreground">None yet. Save one below.</p>
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
          {!connected && (
            <Button
              variant="outline"
              size="sm"
              title="Connect over Bluetooth LE"
              onClick={() => {
                setBleOpen(true);
                handleBleScan();
              }}
            >
              <Bluetooth />
            </Button>
          )}
          {!connected && (
            <Button variant="outline" size="sm" title="Connect over the network (WebSocket)" onClick={() => setWsOpen(true)}>
              <Globe />
            </Button>
          )}
          {connected ? (
            <Button variant="destructive" size="sm" onClick={handleDisconnect}>
              <PlugZap /> Disconnect
            </Button>
          ) : (
            <Button size="sm" onClick={handleConnect} disabled={selectedPort === "auto" && dutaPorts.length < 1}>
              <Plug /> Connect
            </Button>
          )}
          <Button
            variant="ghost"
            size="icon"
            className={cn("size-8", rightBarOpen && "text-primary")}
            title={rightBarOpen ? "Hide panel" : "Show panel"}
            onClick={toggleRightBar}
          >
            <PanelRight />
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1 gap-3 p-3">
        <Card className="flex min-w-0 flex-1 flex-col overflow-hidden">
          {/* tab strip above the viewport: one tab per main view (the active
              DATA stream, typed by kind). Ready for more streams/captures. */}
          <CardHeader className="flex-row items-center gap-2 border-b py-0 pl-2 pr-3">
            <div className="flex items-center self-end">
              {(() => {
                const kind = dataDesc?.kind ?? DATA_KIND.UART;
                const Icon =
                  kind === DATA_KIND.BLE_SNIFF || kind === DATA_KIND.IEEE802154
                    ? Radio
                    : kind === DATA_KIND.I2C
                      ? Activity
                      : TerminalIcon;
                const label =
                  kind === DATA_KIND.BLE_SNIFF
                    ? "BLE sniffer"
                    : kind === DATA_KIND.IEEE802154
                      ? "802.15.4"
                      : kind === DATA_KIND.I2C
                        ? "I²C"
                        : "Console";
                const tab = (active: boolean) =>
                  `flex items-center gap-1.5 border-b-2 px-2 py-2 text-sm font-medium ${active ? "border-primary" : "border-transparent text-muted-foreground hover:text-foreground"}`;
                return (
                  <>
                    <button type="button" className={tab(mainView === "data")} onClick={() => setMainView("data")}>
                      <Icon className="size-3.5" /> {label}
                    </button>
                    {yantras.length > 0 && (
                      <button type="button" className={tab(mainView === "controls")} onClick={() => setMainView("controls")}>
                        <LayoutGrid className="size-3.5" /> Controls
                      </button>
                    )}
                  </>
                );
              })()}
            </div>
            <div className="ml-auto flex items-center gap-2 self-center">
              {mainView === "controls" && yantras.length > 1 && (
                <select className="h-7 rounded border bg-background px-1 text-xs"
                  value={yantraSel} onChange={(e) => setYantraSel(Number(e.target.value))}>
                  {yantras.map((y, i) => (
                    <option key={y.file} value={i}>{y.doc.name || y.file}</option>
                  ))}
                </select>
              )}
              {mainView === "data" && dataSrcPins && dataDesc?.kind === DATA_KIND.UART && (
                <span className="font-mono text-[10px] text-muted-foreground"
                  title={`bridged into the Duta on TX GPIO${dataSrcPins.tx} · RX GPIO${dataSrcPins.rx}`}>
                  TX {dataSrcPins.tx >= 0 ? `GPIO${dataSrcPins.tx}` : "—"} · RX {dataSrcPins.rx >= 0 ? `GPIO${dataSrcPins.rx}` : "—"}
                </span>
              )}
              {/* Bridge-medium selector — only for UART/I²C bridges. A sniffer's
                  medium is the radio (not switchable); never offer it there, or
                  switching to I²C strands the view with no way back. */}
              {mainView === "data" && hasKindSwitch && connected &&
                (dataDesc?.kind === DATA_KIND.UART || dataDesc?.kind === DATA_KIND.I2C) && (
                  <div className="flex items-center overflow-hidden rounded border text-[10px]">
                    {[
                      { k: DATA_KIND.UART, l: "Console" },
                      { k: DATA_KIND.I2C, l: "I²C" },
                    ].map(({ k, l }) => (
                      <button key={k} type="button"
                        className={`px-2 py-0.5 ${dataDesc?.kind === k ? "bg-accent font-medium" : "text-muted-foreground hover:bg-accent/50"}`}
                        title="Switch the bridged medium"
                        onClick={() => dataDesc?.kind !== k && switchDataKind(k)}>
                        {l}
                      </button>
                    ))}
                  </div>
                )}
              {mainView === "data" && dataDesc?.kind === DATA_KIND.UART && (
                <span className="text-xs text-muted-foreground">
                  {baud} 8{parity[0].toUpperCase()}{stopBits}
                </span>
              )}
            </div>
          </CardHeader>
          {/* flex flex-col so the panel child's flex-1/min-h-0 actually
              constrains its height — otherwise a tall list (802.15.4 grouped)
              overflows with no scroll instead of scrolling internally. */}
          <CardContent className="flex min-h-0 flex-1 flex-col bg-[#0a0a0b] p-2">
            {mainView === "controls" ? (
              yantras[yantraSel] ? (
                <YantraCanvas spec={yantras[yantraSel].doc} disabled={!connected} />
              ) : (
                <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
                  Drop a .yantra in the workspace's .sutra/yantra/ to add a control surface.
                </div>
              )
            ) : dataDesc?.kind === DATA_KIND.I2C ? (
              <I2cPanel
                records={i2cRecords}
                defs={i2cDefs}
                present={i2cPresent}
                disabled={!connected || !hasCmd}
                onClear={() => setI2cRecords([])}
                onScan={(found) => setI2cPresent(new Set(found))}
              />
            ) : dataDesc?.kind === DATA_KIND.BLE_SNIFF ? (
              <BleSnifferPanel
                packets={blePackets}
                total={bleTotal}
                onClear={() => { setBlePackets([]); setBleTotal(0); }}
                onSavePcap={saveSniffPcap}
              />
            ) : dataDesc?.kind === DATA_KIND.IEEE802154 ? (
              <Ieee154Panel
                frames={ieee154Frames}
                total={ieee154Total}
                onClear={() => { setIeee154Frames([]); setIeee154Total(0); }}
                onSavePcap={saveIeee154}
                canDecode={tsharkOk}
                onDecode={decodeIeee154Capture}
                onInject={connected ? injectIeee154 : undefined}
                onSaveNodes={saveDiscoveredNodes}
                activeNet={networks.find((n) => n.key.trim())}
                onZclCommand={connected ? runZcl : undefined}
                onRenameNode={(addr, name) =>
                  setNodeName(addr, name)
                    .then(() => getNetworks().then((n) => setNetworks(n.networks ?? [])))
                    .catch((e) => setStatus(`rename failed: ${e}`))
                }
                attrs={attrs}
              />
            ) : (
              <Terminal ref={terminalRef} connected={connected} />
            )}
          </CardContent>
        </Card>

        {rightBarOpen && (
        <>
        {/* drag the panel's left edge to resize */}
        <div
          onPointerDown={startBarResize}
          onPointerMove={onBarResize}
          onPointerUp={endBarResize}
          title="Drag to resize"
          className="-mx-1 w-1.5 shrink-0 cursor-col-resize rounded bg-transparent hover:bg-border"
        />
        <div style={{ width: rightBarWidth }} className="flex shrink-0 flex-col gap-3 overflow-y-auto">
          {/* controls: self-described by the device */}
          <Card>
            <CardHeader className="flex-row flex-wrap items-center gap-y-1.5 py-3">
              <CardTitle>Controls</CardTitle>
              <div className="ml-auto flex min-w-0 flex-wrap items-center justify-end gap-2">
                {hasWifi && connected && (
                  <NetworkConfig
                    onConnectWs={(url) => {
                      setWsUrl(url);
                      setWsOpen(true);
                    }}
                  />
                )}
                {provision && connected && (
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7 gap-1.5"
                    onClick={() => setConfigOpen(true)}
                    title="Provision the device's IO at runtime"
                  >
                    <Cog className="size-3.5" /> Configure
                  </Button>
                )}
                {deviceName && (
                  <Badge variant="secondary" className="min-w-0 max-w-[9rem]" title={deviceName}>
                    <span className="truncate">{deviceName}</span>
                  </Badge>
                )}
              </div>
            </CardHeader>
            <CardContent className="flex flex-col gap-2">
              {controls.length > 0 ? (
                <div className="flex flex-col gap-2">
                  {/* digital on/off controls (io): compact toggle grid */}
                  {controls.some((c) => c.type === CTRL.IO) && (
                    <div className="grid grid-cols-3 gap-2">
                      {controls
                        .filter((c) => c.type === CTRL.IO)
                        .map((c) => {
                          const on = !!(outBitmap & (1 << c.index));
                          return (
                            <ContextMenu key={c.index}>
                              <ContextMenuTrigger asChild>
                                <Button
                                  variant={on ? "default" : "outline"}
                                  size="sm"
                                  disabled={!connected || !hasCmd}
                                  onClick={() => toggle(c.index)}
                                  className="flex h-auto flex-col py-2"
                                  title={ioPins[c.index] != null ? `${c.name} · GPIO${ioPins[c.index]}` : c.name}
                                >
                                  <span className={on ? "" : "text-muted-foreground"}>{c.name}</span>
                                  <span className="text-[10px]">{on ? "ON" : "OFF"}</span>
                                </Button>
                              </ContextMenuTrigger>
                              <ContextMenuContent>
                                <ContextMenuItem onSelect={() => copyMacroCmd(`SET ${c.name} ${on ? 1 : 0}`)}>
                                  <Copy className="size-3" /> Copy as macro command
                                </ContextMenuItem>
                              </ContextMenuContent>
                            </ContextMenu>
                          );
                        })}
                    </div>
                  )}

                  {/* analog (pwm) + color (rgb) controls: one labeled row each */}
                  {controls
                    .filter((c) => c.type === CTRL.PWM || c.type === CTRL.RGB)
                    .map((c) => (
                      <ContextMenu key={c.index}>
                      <ContextMenuTrigger asChild>
                      <div className="flex flex-col gap-1">
                        <div className="flex items-center justify-between">
                          <span
                            className="text-xs font-medium"
                            title={ioPins[c.index] != null ? `${c.name} · GPIO${ioPins[c.index]}` : c.name}
                          >
                            {c.name}
                          </span>
                          {c.type === CTRL.PWM ? (
                            <div className="flex items-center gap-1.5">
                              <span className="text-[10px] text-muted-foreground">
                                {Math.round(((pwmVals[c.index] ?? 0) / 1023) * 100)}%
                              </span>
                              <PwmConfigBadge
                                cfg={pwmCfg[c.index] ?? { freq: 0, res: 0 }}
                                disabled={!connected || !hasCmd}
                                onSet={(f, r) => setPwmConfig(c.index, f, r)}
                              />
                            </div>
                          ) : (
                            <span className="text-[10px] text-muted-foreground">RGB</span>
                          )}
                        </div>
                        {c.type === CTRL.PWM ? (
                          <Slider
                            min={0}
                            max={1023}
                            value={[pwmVals[c.index] ?? 0]}
                            disabled={!connected || !hasCmd}
                            onValueChange={([v]) => setPwm(c.index, v)}
                          />
                        ) : (
                          <RgbControl
                            pixels={rgbVals[c.index] ?? [{ r: 0, g: 0, b: 0 }]}
                            disabled={!connected || !hasCmd}
                            onChange={(pixel, rgb) => setRgb(c.index, rgb, pixel)}
                          />
                        )}
                      </div>
                      </ContextMenuTrigger>
                      <ContextMenuContent>
                        <ContextMenuItem
                          onSelect={() =>
                            copyMacroCmd(
                              c.type === CTRL.PWM
                                ? `SET ${c.name} ${pwmVals[c.index] ?? 0}`
                                : `RGB ${c.name} ${rgbToHex(rgbVals[c.index]?.[0] ?? { r: 0, g: 0, b: 0 })}`,
                            )
                          }
                        >
                          <Copy className="size-3" /> Copy as macro command
                        </ContextMenuItem>
                      </ContextMenuContent>
                      </ContextMenu>
                    ))}
                </div>
              ) : (
                <p className="text-[10px] text-muted-foreground">
                  {connected && !hasCmd
                    ? "Generic serial port. No device controls."
                    : "No controls reported."}
                </p>
              )}
            </CardContent>
          </Card>

          {/* run queue: in-flight macros (cancellable) */}
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
                <Button size="icon" variant="ghost" className="ml-auto size-7" title="New macro" onClick={openAdd}>
                  <Plus />
                </Button>
              </div>
              <div className="flex gap-1">
                <Select value={setFilter || "__all"} onValueChange={(v) => setSetFilter(v === "__all" ? "" : v)}>
                  <SelectTrigger className="h-7 flex-1 text-xs" title="Project / set">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="__all">All sets</SelectItem>
                    {sets.map((s) => (
                      <SelectItem key={s} value={s}>{s}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Select value={tierFilter ? String(tierFilter) : "__all"} onValueChange={(v) => setTierFilter(v === "__all" ? 0 : Number(v))}>
                  <SelectTrigger className="h-7 flex-1 text-xs" title="Capability tier">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="__all">All tiers</SelectItem>
                    {[1, 2, 3].map((t) => (
                      <SelectItem key={t} value={String(t)} title={TIER_INFO[t]?.title}>
                        {TIER_INFO[t]?.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
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
                      <span className="ml-auto flex shrink-0 items-center gap-1">
                        <Badge
                          variant="outline"
                          className={cn("px-1 py-0 text-[9px]", TIER_COLOR[s.tier])}
                          title={TIER_INFO[s.tier]?.title}
                        >
                          {TIER_INFO[s.tier]?.label ?? `T${s.tier}`}
                        </Badge>
                        {!setFilter && s.set && (
                          <Badge variant="secondary" className="px-1 py-0 text-[9px]">
                            {s.set}
                          </Badge>
                        )}
                      </span>
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
        </>
        )}
      </div>

      {/* status bar (VS Code-style): connection + data info on the left, the MCP
          server control + macro count on the right. */}
      <footer className="flex h-6 shrink-0 items-center gap-3 border-t bg-card px-3 text-[11px] text-muted-foreground">
        <span className="flex items-center gap-1.5">
          <span className={cn("size-1.5 rounded-full", !connected ? "bg-muted-foreground/40" : linkOnline ? "bg-success" : "bg-destructive")} />
          {!connected ? "Disconnected" : linkOnline ? "Online" : "Target offline"}
        </span>
        {connected && dataPort && <span className="truncate">{dataPort}</span>}
        {connected && dataDesc?.kind === DATA_KIND.UART && (
          <span>{baud} 8{parity[0].toUpperCase()}{stopBits}</span>
        )}
        {connected && dataDesc?.kind === DATA_KIND.IEEE802154 && (
          <span>802.15.4 · {ch154 ? `ch ${ch154}` : "hop"}</span>
        )}
        <span className="truncate">{status}</span>

        {/* MCP server — relocated here as a clickable status item */}
        <Popover>
          <PopoverTrigger asChild>
            <button type="button" className="ml-auto flex items-center gap-1.5 rounded px-1.5 hover:bg-accent hover:text-foreground" title="MCP server">
              <Bot className="size-3" /> MCP
              <span className={cn("size-1.5 rounded-full", mcp.running ? "bg-success" : "bg-muted-foreground/40")} />
            </button>
          </PopoverTrigger>
          <PopoverContent align="end" side="top" className="w-80">
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
                contents (secrets) are never exposed; it can only run them by name.
              </p>
            </div>
          </PopoverContent>
        </Popover>
        <span>{macros.length} macro{macros.length === 1 ? "" : "s"}</span>
      </footer>

      {/* Configure-device (runtime IO provisioning) modal */}
      <ConfigureDevice
        open={configOpen}
        onOpenChange={(o) => {
          setConfigOpen(o);
          if (!o) loadDevice(); // refresh controls after a reboot/edit
        }}
      />

      {/* BLE scan / connect modal */}
      <Dialog open={bleOpen} onOpenChange={setBleOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Bluetooth className="size-4" /> Bluetooth devices
            </DialogTitle>
          </DialogHeader>
          <p className="-mt-2 text-xs text-muted-foreground">
            The link is encrypted — the first connection pairs the device (accept the
            Windows pairing prompt if it appears). Strongest signal is listed first.
          </p>
          <div className="flex flex-col gap-2 py-1">
            {bleScanning ? (
              <p className="text-xs text-muted-foreground">Scanning…</p>
            ) : bleDevices.length === 0 ? (
              <p className="text-xs text-muted-foreground">No Duta devices found.</p>
            ) : (
              bleDevices.map((d) => (
                <Button
                  key={d.id}
                  variant="outline"
                  size="sm"
                  className="justify-start gap-2"
                  onClick={() => handleBleConnect(d)}
                >
                  <Bluetooth className="size-3.5" />
                  <span className="min-w-0 flex-1 truncate text-left">{d.name || d.id}</span>
                  {d.rssi != null && (
                    <span
                      className="shrink-0 font-mono text-[11px] text-muted-foreground"
                      title={`signal ${d.rssi} dBm`}
                    >
                      {d.rssi >= -60 ? "▮▮▮" : d.rssi >= -75 ? "▮▮▯" : d.rssi >= -88 ? "▮▯▯" : "▯▯▯"} {d.rssi}
                    </span>
                  )}
                </Button>
              ))
            )}
          </div>
          <DialogFooter>
            <Button variant="ghost" size="sm" onClick={handleBleScan} disabled={bleScanning}>
              {bleScanning ? "Scanning…" : "Rescan"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* network (WebSocket) connect modal */}
      <Dialog open={wsOpen} onOpenChange={setWsOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Globe className="size-4" /> Connect over the network
            </DialogTitle>
          </DialogHeader>
          <div className="flex flex-col gap-2 py-1">
            <div className="flex items-center justify-between">
              <span className="text-xs text-muted-foreground">
                {wsScanning
                  ? "scanning the LAN…"
                  : wsFound.length
                    ? "Dutas on your network:"
                    : "no Dutas discovered (mDNS)"}
              </span>
              <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" disabled={wsScanning}
                onClick={async () => {
                  setWsScanning(true);
                  try { setWsFound(await wsDiscover()); } catch { setWsFound([]); }
                  setWsScanning(false);
                }}>
                Scan
              </Button>
            </div>
            {wsFound.map((d) => (
              <button key={d.url} type="button"
                className={cn(
                  "flex items-center justify-between rounded border px-2 py-1.5 text-left text-xs hover:bg-accent",
                  wsUrl === d.url && "border-primary",
                )}
                onClick={() => setWsUrl(d.url)}>
                <span className="font-medium">{d.name}</span>
                <span className="font-mono text-muted-foreground">{d.ip}:{d.port}</span>
              </button>
            ))}
            <Input
              placeholder="ws://host:port/"
              value={wsUrl}
              onChange={(e) => setWsUrl(e.target.value)}
              spellCheck={false}
            />
            <Input
              type="password"
              placeholder="password"
              value={wsPassword}
              onChange={(e) => setWsPassword(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleWsConnect()}
            />
            <p className="text-[10px] text-muted-foreground">
              The device authenticates with this password (default <code>duta</code>). Use
              <code>wss://</code> for TLS over an untrusted network.
            </p>
          </div>
          <DialogFooter>
            <Button size="sm" onClick={handleWsConnect} disabled={wsConnecting || !wsUrl}>
              {wsConnecting ? "Connecting…" : "Connect"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* add / edit macro modal */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className={cn("w-[92vw] sm:max-w-2xl lg:max-w-4xl", showHelp && "lg:max-w-6xl")}>
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
              <CodeTextarea
                ref={macroTextRef}
                placeholder={"login\nDELAY 1000\nSTRING whoami\nENTER"}
                value={draftText}
                onChange={(e) => setDraftText(e.target.value)}
                className="h-44"
              />
              <MacroColorStrip text={draftText} onChange={setDraftText} />
              <MacroVars onInsert={insertMacroVar} active={networks.find((n) => n.key.trim()) ?? networks[0]} />
              {!showHelp && (
                <p className="text-[10px] leading-tight text-muted-foreground">
                  One command per line. <code>STRING</code> <code>ENTER</code> <code>DELAY ms</code>{" "}
                  <code>WAITFOR text</code> <code>RUN cmd</code> <code>IF OK…END</code>, or tap{" "}
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

      {/* settings view — a full-screen pane with a section sidebar (not a modal) */}
      {settingsOpen && (
        // start below the title bar (h-9) so the window controls stay reachable
        <div className="fixed inset-x-0 bottom-0 top-9 z-50 flex flex-col bg-background text-foreground">
          <header className="flex items-center gap-3 border-b px-4 py-2.5">
            <Cog className="size-5 text-primary" />
            <span className="font-semibold tracking-tight">Settings</span>
            <Button
              variant="ghost"
              size="icon"
              className="ml-auto size-8"
              title="Close settings"
              onClick={() => setSettingsOpen(false)}
            >
              <X />
            </Button>
          </header>

          <div className="flex min-h-0 flex-1">
            {/* sidebar menu */}
            <nav className="flex w-52 shrink-0 flex-col gap-0.5 border-r p-2">
              {SETTINGS_SECTIONS.map((s) => (
                <button
                  key={s.id}
                  type="button"
                  onClick={() => setSettingsTab(s.id)}
                  className={cn(
                    "flex items-center gap-2 rounded-md px-3 py-2 text-sm",
                    settingsTab === s.id
                      ? "bg-accent font-medium text-foreground"
                      : "text-muted-foreground hover:bg-accent/50",
                  )}
                >
                  <s.icon className="size-4 shrink-0" /> {s.label}
                </button>
              ))}
            </nav>

            {/* section content */}
            <div className="min-w-0 flex-1 overflow-auto p-6">
              <div className="mx-auto flex max-w-2xl flex-col gap-4">
                {settingsTab === "general" && (
                  <Card>
                    <CardHeader className="pb-3">
                      <CardTitle className="text-sm">Workspace</CardTitle>
                    </CardHeader>
                    <CardContent className="flex flex-col gap-3">
                      <div className="flex items-center justify-between gap-3">
                        <div className="min-w-0">
                          <div className="text-sm">Workspace folder</div>
                          <div className="truncate text-[11px] text-muted-foreground">
                            {workspace ?? "None selected"}
                          </div>
                        </div>
                        <Button variant="outline" size="sm" className="shrink-0 gap-1.5" onClick={chooseWorkspace}>
                          <FolderOpen className="size-3.5" /> Change…
                        </Button>
                      </div>
                      <p className="text-[10px] leading-tight text-muted-foreground">
                        Macros, captures, and network keys are saved under the workspace's{" "}
                        <code>.sutra/</code> folder.
                      </p>
                    </CardContent>
                  </Card>
                )}

                {settingsTab === "mcp" && (
                  <>
                    <Card>
                      <CardHeader className="pb-3">
                        <CardTitle className="text-sm">MCP server</CardTitle>
                      </CardHeader>
                      <CardContent className="flex flex-col gap-3">
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
                      </CardContent>
                    </Card>

                    <Card>
                      <CardHeader className="pb-3">
                        <CardTitle className="text-sm">Tools exposed to the LLM</CardTitle>
                      </CardHeader>
                      <CardContent className="flex flex-col gap-3">
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
                          Disabled tools are hidden from the model entirely. Changing these restarts a
                          running MCP server.
                        </p>
                      </CardContent>
                    </Card>
                  </>
                )}

                {settingsTab === "connection" && (
                  <div className="flex flex-col gap-3">
                    <div className="text-sm font-semibold">Connection</div>
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
                )}

                {settingsTab === "decode" && (
                  <div className="flex flex-col gap-3">
                    <div className="text-sm font-semibold">Packet decode (Wireshark)</div>
                    <div className="flex items-center justify-between gap-3">
                      <div className="min-w-0">
                        <div className="text-sm">tshark path</div>
                        <div className="text-[11px] text-muted-foreground">
                          For decoding Zigbee/Thread captures. Leave blank to autodetect;{" "}
                          {tsharkOk ? (
                            <span className="text-success">found ✓</span>
                          ) : (
                            <span className="text-destructive">not found</span>
                          )}
                          .
                        </div>
                      </div>
                      <Input
                        className="h-8 w-44 font-mono text-xs"
                        placeholder="autodetect"
                        value={settings.tsharkPath}
                        spellCheck={false}
                        onChange={(e) => setSetting("tsharkPath", e.target.value)}
                      />
                    </div>

                    {/* Networks — the workspace network model (.sutra/networks.json):
                        each network's decryption key (fed to tshark to decode NWK/APS to
                        real ZCL commands) plus the nodes discovered passively from sniffing. */}
                    <div className="mt-1">
                      <div className="flex items-center gap-2">
                        <div className="text-sm">Networks</div>
                        <span
                          className="cursor-help text-[10px] text-muted-foreground underline decoration-dotted"
                          title={
                            "Where to find your network key:\n" +
                            "• Home Assistant (ZHA): Settings ▸ Devices & Services ▸ Zigbee Home Automation ▸ ⋮ ▸ Download diagnostics — the JSON has network_info.network_key.\n" +
                            "• Zigbee2MQTT: data/configuration.yaml ▸ advanced ▸ network_key (a 16-byte array)."
                          }
                        >
                          where?
                        </span>
                      </div>
                      <div className="text-[11px] text-muted-foreground">
                        Saved in the workspace. The key decrypts your network so frames show the
                        actual command, not just "Command"; nodes are discovered by sniffing
                        (802.15.4 ▸ Nodes ▸ Save nodes).
                        {!workspace && <span className="text-destructive"> Select a workspace to save.</span>}
                      </div>
                    </div>
                    {networks.map((net, i) => (
                      <div key={i} className="flex items-center gap-2">
                        <span className="min-w-0 flex-1 truncate text-xs">
                          {net.protocol === "thread" && (
                            <span className="mr-1 rounded bg-accent px-1 text-[10px]">Thread</span>
                          )}
                          <span className="text-muted-foreground">{net.label}</span>{" "}
                          {net.key
                            ? <span className="font-mono">{net.key.slice(0, 8)}…</span>
                            : <span className="text-amber-500">no key</span>}
                          {net.nodes.length > 0 && (
                            <span className="text-muted-foreground"> · {net.nodes.length} nodes</span>
                          )}
                        </span>
                        <Button variant="ghost" size="icon" className="size-7 text-muted-foreground"
                          title="Remove network" onClick={() => saveNetworks(networks.filter((_, j) => j !== i))}>
                          <Trash2 />
                        </Button>
                      </div>
                    ))}
                    <div className="flex items-center gap-1">
                      <button type="button"
                        className="h-8 shrink-0 rounded border px-2 text-[11px] hover:bg-accent"
                        title="Which protocol this key decrypts (Thread = Matter-over-Thread)"
                        onClick={() => setDraftProtocol(draftProtocol === "thread" ? "" : "thread")}>
                        {draftProtocol === "thread" ? "Thread" : "Zigbee"}
                      </button>
                      <Input className="h-8 w-20 text-xs" placeholder="label"
                        value={draftKeyLabel} onChange={(e) => setDraftKeyLabel(e.target.value)} />
                      <Input className="h-8 min-w-0 flex-1 font-mono text-xs"
                        placeholder={draftProtocol === "thread" ? "Thread network key (32 hex)" : "key: hex or 16-byte array"}
                        value={draftKey} spellCheck={false} onChange={(e) => setDraftKey(e.target.value)}
                        onKeyDown={(e) => e.key === "Enter" && addNetwork()} />
                      <Button size="sm" className="h-8" disabled={!workspace} onClick={addNetwork} title="Add network key">
                        <Plus />
                      </Button>
                    </div>
                  </div>
                )}

              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
