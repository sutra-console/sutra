import { useEffect, useRef, useState } from "react";
import {
  Usb, Plug, PlugZap, Play, Plus, Trash2, Cpu, Settings2, Bot, Database, Copy, Lock, LockOpen, Pencil, GripVertical, Cog, CircleHelp, Bookmark, X, Download, Upload, Bluetooth, Globe,
  Radio, Activity, Terminal as TerminalIcon, FolderOpen,
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
import { RgbControl } from "@/components/RgbControl";
import { MacroColorStrip } from "@/components/MacroColorStrip";
import { PwmConfigBadge } from "@/components/PwmConfigBadge";
import { BleSnifferPanel } from "@/components/BleSnifferPanel";
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
  getInfo,
  getIoConfig,
  onData,
  setDataKind,
  type BleSniffPacket,
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
  pickWorkspace,
  rgbToHex,
  saveBlePcap,
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
  const [blePackets, setBlePackets] = useState<BleSniffPacket[]>([]); // decoded ble-sniff records
  const [workspace, setWorkspace] = useState<string | null>(null); // the .sutra workspace folder
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

  function clearDevice() {
    setDeviceName("");
    setControls([]);
    setOutBitmap(0);
    setPwmVals({});
    setPwmCfg({});
    setRgbVals({});
    setDataDesc(null);
    setDataSrcPins(null);
    setIoPins({});
    setI2cRecords([]);
    setBlePackets([]);
    setCaps(0);
  }

  // For typed DATA kinds the Terminal is unmounted, so App is the only DATA
  // consumer — decode the records into the matching view's state.
  useEffect(() => {
    const kind = dataDesc?.kind;
    if (kind !== DATA_KIND.I2C && kind !== DATA_KIND.BLE_SNIFF) return;
    let unlisten: (() => void) | undefined;
    onData((bytes) => {
      if (kind === DATA_KIND.I2C) {
        const rec = decodeI2cRecord(bytes);
        if (rec) setI2cRecords((rs) => [...rs.slice(-499), rec]);
      } else {
        const pkt = decodeBleSniff(bytes);
        if (pkt) setBlePackets((ps) => [...ps.slice(-1999), pkt]);
      }
    }).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, [dataDesc?.kind]);

  /** Choose the workspace folder (.sutra/ for macros + captures). */
  async function chooseWorkspace() {
    try {
      const path = await pickWorkspace();
      if (path) {
        setWorkspace(path);
        macrosGet().then(setMacros).catch(() => {}); // store re-pointed into .sutra
        setStatus(`workspace: ${path}`);
      }
    } catch (e) {
      setStatus(`workspace failed: ${e}`);
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

        {/* workspace folder (macros + captures land in its .sutra/) */}
        <Button
          variant="ghost"
          size="sm"
          className="ml-1 h-7 max-w-[14rem] gap-1.5 text-muted-foreground"
          title={workspace ? `Workspace: ${workspace}\nMacros + captures save to .sutra/` : "Choose a workspace folder (.sutra/ for macros + captures)"}
          onClick={chooseWorkspace}
        >
          <FolderOpen className="size-3.5 shrink-0" />
          <span className="truncate">{workspace ? workspace.split(/[/\\]/).pop() : "No workspace"}</span>
        </Button>

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
                contents (secrets) are never exposed; it can only run them by name.
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
                <span className="text-sm font-semibold">Serial: DATA</span>
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
                      : "Duta is 8N1, so only baud reaches the wire (build firmware with PARITY_SUPPORT for parity)."
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
          <Button variant="ghost" size="icon" className="size-8" title="Settings" onClick={() => setSettingsOpen(true)}>
            <Cog />
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
                const Icon = kind === DATA_KIND.BLE_SNIFF ? Radio : kind === DATA_KIND.I2C ? Activity : TerminalIcon;
                const label = kind === DATA_KIND.BLE_SNIFF ? "BLE sniffer" : kind === DATA_KIND.I2C ? "I²C" : "Console";
                return (
                  <div className="flex items-center gap-1.5 border-b-2 border-primary px-2 py-2 text-sm font-medium">
                    <Icon className="size-3.5" /> {label}
                  </div>
                );
              })()}
            </div>
            <div className="ml-auto flex items-center gap-2 self-center">
              {dataSrcPins && dataDesc?.kind === DATA_KIND.UART && (
                <span className="font-mono text-[10px] text-muted-foreground"
                  title={`bridged into the Duta on TX GPIO${dataSrcPins.tx} · RX GPIO${dataSrcPins.rx}`}>
                  TX {dataSrcPins.tx >= 0 ? `GPIO${dataSrcPins.tx}` : "—"} · RX {dataSrcPins.rx >= 0 ? `GPIO${dataSrcPins.rx}` : "—"}
                </span>
              )}
              {hasKindSwitch && connected && (
                <Button variant="outline" size="sm" className="h-6 px-2 text-[10px]"
                  title="Switch the bridged medium"
                  onClick={() => switchDataKind(dataDesc?.kind === DATA_KIND.I2C ? DATA_KIND.UART : DATA_KIND.I2C)}>
                  {dataDesc?.kind === DATA_KIND.I2C ? "→ UART" : "→ I²C"}
                </Button>
              )}
              {dataDesc?.kind === DATA_KIND.UART && (
                <span className="text-xs text-muted-foreground">
                  {baud} 8{parity[0].toUpperCase()}{stopBits}
                </span>
              )}
            </div>
          </CardHeader>
          <CardContent className="min-h-0 flex-1 bg-[#0a0a0b] p-2">
            {dataDesc?.kind === DATA_KIND.I2C ? (
              <I2cPanel records={i2cRecords} disabled={!connected || !hasCmd} onClear={() => setI2cRecords([])} />
            ) : dataDesc?.kind === DATA_KIND.BLE_SNIFF ? (
              <BleSnifferPanel
                packets={blePackets}
                onClear={() => setBlePackets([])}
                onSavePcap={saveSniffPcap}
              />
            ) : (
              <Terminal ref={terminalRef} connected={connected} />
            )}
          </CardContent>
        </Card>

        <div className="flex w-80 shrink-0 flex-col gap-3 overflow-y-auto">
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
      </div>

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
                  className="justify-start"
                  onClick={() => handleBleConnect(d)}
                >
                  <Bluetooth className="size-3.5" />
                  <span className="truncate">{d.name || d.id}</span>
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
              <MacroColorStrip text={draftText} onChange={setDraftText} />
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
