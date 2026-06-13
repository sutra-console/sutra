// Front-end mirror of the skrit CMD protocol. See protocol/PROTOCOL.md
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import invocableCatalog from "./invocables.json";

export const MSG = {
  PING: 0x01,
  INFO: 0x02,
  DEVICE_NAME: 0x03,
  REBOOT: 0x04,
  DATA_DESC: 0x07,
  OUTPUT_SET: 0x10,
  OUTPUT_GET: 0x11,
  OUTPUT_TOGGLE: 0x12,
  OUTPUT_DESC: 0x13,
  INPUT_DESC: 0x14,
  INPUT_GET: 0x15,
  OUTPUT_PULSE: 0x16,
  PROTO_GET: 0x17,
  PROTO_SET: 0x18,
  SERIAL_SIGNAL: 0x19,
  OUTPUT_PWM: 0x1a,
  OUTPUT_RGB: 0x1b,
  PWM_CONFIG: 0x1c,
  PIN_CAPS: 0x1d,
  CONFIG_GET: 0x1e,
  CONFIG_SET: 0x1f,
  MACRO_LIST: 0x20,
  MACRO_META: 0x21,
  MACRO_READ: 0x22,
  MACRO_WRITE_BEGIN: 0x23,
  MACRO_WRITE_DATA: 0x24,
  MACRO_WRITE_END: 0x25,
  MACRO_DELETE: 0x26,
  MACRO_RUN: 0x27,
  CFG_GET: 0x40,
  CFG_SET: 0x41,
  I2C_SCAN: 0x60,
  I2C_XFER: 0x61,
  INVOKE_DESC: 0x70,
  INVOKE: 0x71,
  EVENT_LOG: 0x50,
  EVENT_INPUT: 0x51,
} as const;

export const OUTPUT = { R1: 0, R2: 1, LED: 2 } as const;
/** Output control types (OUTPUT_DESC type byte), by behavior, not fixture. */
export const CTRL = { IO: 0, PWM: 1, RGB: 2 } as const;
/** duta_io row flag: output is active-low (driven LOW = on). */
export const DUTA_ACTIVE_LOW = 0x01;
export const RESP_FLAG = 0x80;
/** SERIAL_SIGNAL line bits (mask/value). */
export const SIG = { DTR: 0x01, RTS: 0x02, BREAK: 0x04 } as const;
/** SERIAL_GET/SET parity byte. */
export const PARITY = { NONE: 0, ODD: 1, EVEN: 2 } as const;
/** REBOOT modes. */
export const REBOOT = { APP: 0, BOOTLOADER: 1 } as const;

export interface PortDesc {
  name: string;
  vid: number | null;
  pid: number | null;
  product: string | null;
  manufacturer: string | null;
  serial_number: string | null;
  is_duta: boolean;
}

export interface RespFrame {
  typ: number;
  seq: number;
  status: number | null;
  body: number[];
}

export interface DetectResult {
  data: string;
  cmd: string;
}

export const listPorts = () => invoke<PortDesc[]>("list_ports");
export const autodetect = () => invoke<DetectResult>("autodetect");
/** Find a single-port muxed Duta: probes candidate ports with a skrit-mux PING. */
export const autodetectMux = () => invoke<string>("autodetect_mux");
/** Connect a DATA port. Pass cmdPort for a Duta, or null/omit for any generic serial port. */
export const connect = (dataPort: string, cmdPort?: string | null) =>
  invoke<void>("connect", { dataPort, cmdPort: cmdPort ?? null });
/** Connect a single-port muxed Duta (ESP32 / Pico / nRF52840): DATA + CMD over one port. */
export const connectMuxed = (port: string) => invoke<void>("connect_muxed", { port });
export const disconnect = () => invoke<void>("disconnect");

export interface BleDevice {
  id: string;
  name: string;
  rssi: number | null; // advertised signal (dBm); list is sorted strongest-first
}
/** Scan for Duta peripherals over Bluetooth LE (~3s). */
export const bleScan = () => invoke<BleDevice[]>("ble_scan");
/** Connect a Duta over BLE by scanned device id. Returns the device name. */
export const bleConnect = (id: string) => invoke<string>("ble_connect", { id });

export interface WsConnectResult {
  name: string;
  default_cred: boolean;
}
/** Connect a Duta over the network (WebSocket), authenticating with `password`. */
export const wsConnect = (url: string, password: string) =>
  invoke<WsConnectResult>("ws_connect", { url, password });
export interface DiscoveredDuta {
  name: string;
  vendor: string;
  host: string;
  ip: string;
  port: number;
  url: string; // ready-to-connect ws://ip:port/
}
/** Browse the LAN for Dutas advertising `_skrit._tcp` (mDNS); ~2.5s scan. */
export const wsDiscover = (timeoutMs?: number) =>
  invoke<DiscoveredDuta[]>("ws_discover", { timeoutMs });

// ---- workspace (a folder with a .sutra/ for macros + captures) ----
/** The current workspace folder, or null if none is selected. */
export const getWorkspace = () => invoke<string | null>("get_workspace");
/** Open a folder picker and adopt it as the workspace; returns the path or null. */
export const pickWorkspace = () => invoke<string | null>("pick_workspace");
/** Save ble-sniff records as a pcap (workspace captures/ or a save dialog). */
export const saveBlePcap = (name: string, records: number[][]) =>
  invoke<string>("save_ble_pcap", { name, records });
export const saveIeee154Pcap = (name: string, records: number[][]) =>
  invoke<string>("save_ieee154_pcap", { name, records });

export interface DecodedRow {
  num: number; // 1-based, maps to record index num-1
  protocol: string; // friendly upper-layer protocol (ZigBee NWK, Thread, …)
  summary: string; // one-line src → dst
  fields: [string, string][]; // dissected field tree (name, value) — the macro hook
}
/** Is Wireshark's tshark available for in-app decoding? (optional path override) */
export const tsharkAvailable = (tsharkPath?: string) =>
  invoke<boolean>("tshark_available", { tsharkPath: tsharkPath || null });
/** Dissect raw ieee802154 records with tshark (rtshark) → per-packet decode rows.
 *  Uses the workspace's saved Zigbee keys for decryption. */
export const dissectIeee154 = (records: number[][], tsharkPath?: string) =>
  invoke<DecodedRow[]>("dissect_ieee154", { records, tsharkPath: tsharkPath || null });

// The workspace network model (.sutra/networks.json): the unit everything hangs
// off. Its decryption key lives here (not on the device), next to the nodes we
// discover passively from sniffed traffic.
export interface NetEndpoint {
  id: number;
  clusters: string[]; // input cluster ids "0x0006"
}
export interface NetNode {
  addr: string; // short address "0x1234"
  role: string; // Coordinator / Router / End Device / Node
  channels: number[];
  count: number;
  lastSeen: string;
  ieee: string; // filled by active discovery (Phase B+)
  manufacturer: string;
  endpoints: NetEndpoint[];
}
export interface Network {
  label: string;
  pan: string; // "0x39fd" or ""
  channel: number; // 0 = unknown
  key: string; // network/TC key, 32 hex (decryption)
  nodes: NetNode[];
}
export interface Networks {
  networks: Network[];
  /** label of the active network {$…} macro vars resolve against (else first keyed). */
  active?: string;
}
/** Read the workspace network model from .sutra/networks.json (migrates keys.json). */
export const getNetworks = () => invoke<Networks>("get_networks");
/** Persist the workspace network model. Needs a workspace selected. */
export const setNetworks = (networks: Networks) =>
  invoke<void>("set_networks", { networks });
export const dataWrite = (bytes: number[]) => invoke<void>("data_write", { bytes });
export const sendCmd = (typ: number, body: number[] = []) =>
  invoke<RespFrame>("send_cmd", { typ, body });

/** Subscribe to raw DATA-port bytes (target console). */
export async function onData(cb: (bytes: Uint8Array) => void): Promise<UnlistenFn> {
  return listen<number[]>("sutra://data", (e) => cb(Uint8Array.from(e.payload)));
}

/** Target link state: fires false when the DATA port drops (unplug / device
 *  reset) and true when it auto-recovers. The connection stays open throughout. */
export async function onLink(cb: (online: boolean) => void): Promise<UnlistenFn> {
  return listen<boolean>("sutra://link", (e) => cb(e.payload));
}

/** Fires when the backend establishes a connection — including one initiated
 *  over MCP (not the UI). Lets the UI re-sync instead of showing a stale/errored
 *  state while the backend actually holds the port. */
export async function onConnected(cb: () => void): Promise<UnlistenFn> {
  return listen("sutra://connected", () => cb());
}

// ---- high-level helpers ----
export const ping = () => sendCmd(MSG.PING);
export const outputSet = (index: number, on: boolean) =>
  sendCmd(MSG.OUTPUT_SET, [index, on ? 1 : 0]);
export const outputToggle = (index: number) => sendCmd(MSG.OUTPUT_TOGGLE, [index]);

export async function outputGet(): Promise<{ r1: boolean; r2: boolean; led: boolean }> {
  const r = await sendCmd(MSG.OUTPUT_GET);
  const bm = r.body[r.status === null ? 0 : 1] ?? 0; // skip STATUS byte on responses
  return { r1: !!(bm & 1), r2: !!(bm & 2), led: !!(bm & 4) };
}

export const deviceRunMacro = (id: number) => sendCmd(MSG.MACRO_RUN, [id]);

/** Momentary pulse: flip an output for `ms`, then restore (reset/power button). */
export const outputPulse = (index: number, ms: number) =>
  sendCmd(MSG.OUTPUT_PULSE, [index, ms & 0xff, (ms >> 8) & 0xff]);

/** Set a PWM output's duty (0-1023). Needs CAP.PWM and a pwm-type output. */
export const outputPwm = (index: number, duty: number) =>
  sendCmd(MSG.OUTPUT_PWM, [index, duty & 0xff, (duty >> 8) & 0xff]);

/** Read a PWM output's current duty (0-1023). */
export async function outputPwmGet(index: number): Promise<number> {
  const b = (await sendCmd(MSG.OUTPUT_PWM, [index])).body; // [status, index, lo, hi]
  return (b[2] ?? 0) | ((b[3] ?? 0) << 8);
}

export interface PwmConfig {
  freq: number; // Hz
  res: number; // bits
}
const parsePwmConfig = (b: number[]): PwmConfig => ({
  // body: [status, index, freq(4 LE), res]
  freq: ((b[2] ?? 0) | ((b[3] ?? 0) << 8) | ((b[4] ?? 0) << 16) | ((b[5] ?? 0) << 24)) >>> 0,
  res: b[6] ?? 0,
});
/** Read a PWM output's frequency (Hz) + resolution (bits). Reports defaults even
 *  on a device that can't change them. */
export async function pwmConfigGet(index: number): Promise<PwmConfig> {
  return parsePwmConfig((await sendCmd(MSG.PWM_CONFIG, [index])).body);
}
/** Set a PWM output's frequency/resolution (0 = leave unchanged). Returns actuals. */
export async function pwmConfigSet(index: number, freq: number, res: number): Promise<PwmConfig> {
  const body = [index, freq & 0xff, (freq >> 8) & 0xff, (freq >> 16) & 0xff, (freq >>> 24) & 0xff, res & 0xff];
  return parsePwmConfig((await sendCmd(MSG.PWM_CONFIG, body)).body);
}

export interface Rgb {
  r: number;
  g: number;
  b: number;
}
/** Set an addressable-LED output's color. Omit `pixel` to fill the whole strip,
 *  or pass a pixel index to set just that LED. */
export const outputRgb = (index: number, { r, g, b }: Rgb, pixel?: number) =>
  sendCmd(
    MSG.OUTPUT_RGB,
    pixel === undefined
      ? [index, r & 0xff, g & 0xff, b & 0xff]
      : [index, pixel & 0xff, r & 0xff, g & 0xff, b & 0xff],
  );

/** Read an RGB output: its pixel `count` and pixel 0's color. */
export async function outputRgbGet(index: number): Promise<{ count: number } & Rgb> {
  const b = (await sendCmd(MSG.OUTPUT_RGB, [index])).body; // [status, index, count, r, g, b]
  return { count: b[2] ?? 1, r: b[3] ?? 0, g: b[4] ?? 0, b: b[5] ?? 0 };
}

/** "#rrggbb" <-> {r,g,b} helpers for the color UI. */
export const rgbToHex = ({ r, g, b }: Rgb) =>
  "#" + [r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("");
export function hexToRgb(hex: string): Rgb | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return { r: (n >> 16) & 0xff, g: (n >> 8) & 0xff, b: n & 0xff };
}

/** PROTO flags byte (PROTO_GET/SET). */
export const PROTO = { FWD: 0x01 } as const;

export interface ProtoParams {
  idx: number;
  flags: number; // bit0 = forward this interface's RX to the host
  value: number; // medium's primary param (uart baud · i2c clock · 802.15.4 channel)
  opt0: number; // uart: data_bits
  opt1: number; // uart: parity
  opt2: number; // uart: stop_bits
}

/** Configure a bridged interface's link params (PROTO_SET). idx selects the
 *  interface; the rest is interpreted by the DATA kind. */
export const protoSet = (
  idx: number,
  value: number,
  { flags = PROTO.FWD, opt0 = 0, opt1 = 0, opt2 = 0 } = {},
) =>
  sendCmd(MSG.PROTO_SET, [
    idx & 0xff, flags & 0xff,
    value & 0xff, (value >> 8) & 0xff, (value >> 16) & 0xff, (value >> 24) & 0xff,
    opt0 & 0xff, opt1 & 0xff, opt2 & 0xff,
  ]);

/** Read a bridged interface's link params (PROTO_GET). */
export async function protoGet(idx = 0): Promise<ProtoParams> {
  const b = (await sendCmd(MSG.PROTO_GET, [idx])).body; // [st, idx, flags, value(4), opt0, opt1, opt2]
  return {
    idx: b[1] ?? 0,
    flags: b[2] ?? 0,
    value: (b[3] | (b[4] << 8) | (b[5] << 16) | (b[6] << 24)) >>> 0,
    opt0: b[7] ?? 0,
    opt1: b[8] ?? 0,
    opt2: b[9] ?? 0,
  };
}

/** Reconfigure the target DATA UART over the wire (PROTO_SET; keeps forwarding on). */
export const serialSet = (baud: number, dataBits = 8, parity = PARITY.NONE, stopBits = 1) =>
  protoSet(0, baud, { opt0: dataBits, opt1: parity, opt2: stopBits });

/** Pin the 802.15.4 sniffer's channel (11–26; 0 = promiscuous/auto-hop). */
export const setIeee154Channel = (channel: number) => protoSet(0, channel);
/** Read the 802.15.4 sniffer's current channel. */
export const getIeee154Channel = async () => (await protoGet(0)).value;

/** Drive DATA modem/break lines. mask/value are OR-combinations of SIG.*. */
export const serialSignal = (mask: number, value: number) =>
  sendCmd(MSG.SERIAL_SIGNAL, [mask & 0xff, value & 0xff]);

/** Reboot the device: REBOOT.APP (reset) or REBOOT.BOOTLOADER (DFU). */
export const reboot = (mode: number = REBOOT.APP) => sendCmd(MSG.REBOOT, [mode]);

/** Subscribe to async device events (EVENT_LOG / EVENT_INPUT). */
export async function onEvent(
  cb: (typ: number, body: number[]) => void,
): Promise<UnlistenFn> {
  return listen<[number, number[]]>("sutra://event", (e) => cb(e.payload[0], e.payload[1]));
}

// device capability bits (INFO body[3])
export const CAP = {
  STORE: 0x01,
  OLED: 0x02,
  SPI: 0x04,
  PARITY: 0x08,
  MUX: 0x10,
  SERIAL: 0x20,
  REBOOT: 0x40,
  PWM: 0x80,
} as const;
// INFO flags byte (body[9])
export const FLAG = { AUTH_REQUIRED: 0x01, DEFAULT_CRED: 0x02, PROVISION: 0x04, INVOKE: 0x08 } as const;

export interface DeviceInfo {
  fwVer: number;
  caps: number;
  nOutputs: number;
  storeKb: number;
  protoVer: number;
  nInputs: number;
  macroTier: number; // highest skrit-mc tier the device VM runs (0 = no VM)
  flags: number; // FLAG.* (auth-required / default-cred / provision)
}
export async function getInfo(): Promise<DeviceInfo> {
  const b = (await sendCmd(MSG.INFO)).body; // [status, fwlo, fwhi, caps, nout, eekb, ver, nin?, tier?, flags?]
  return {
    fwVer: ((b[2] ?? 0) << 8) | (b[1] ?? 0),
    caps: b[3] ?? 0,
    nOutputs: b[4] ?? 0,
    storeKb: b[5] ?? 0,
    protoVer: b[6] ?? 0,
    nInputs: b[7] ?? 0,
    macroTier: b[8] ?? 0,
    flags: b[9] ?? 0,
  };
}

// ---- runtime provisioning (PIN_CAPS / CONFIG_GET / CONFIG_SET) ----
/** Pin-capability bits (PIN_CAPS `caps` byte) + provisioning sentinels. */
export const PINCAP = {
  DIGITAL: 0x01, ADC: 0x02, PWM: 0x04, DAC: 0x08, I2C: 0x10, SPI: 0x20, TOUCH: 0x40,
  WARN: 1, NO_BUS: 0xff, CONFIG_RESET: 0xff,
} as const;

/** One offerable pin from the provisioning menu (mcu ∩ board). */
export interface PinCap {
  pin: number;
  caps: number; // PINCAP.* bitfield: which roles are valid
  warn: boolean; // offer, but show `note`
  bus: number; // I²C/SPI bus index, or PINCAP.NO_BUS
  note: string; // warning reason when `warn` (strapping / dual-use label)
}
/** The provisioning menu: every pin that can be assigned an IO role. */
export async function pinCaps(): Promise<PinCap[]> {
  const first = await sendCmd(MSG.PIN_CAPS, [0]);
  if (first.status !== 0) return []; // not a provisioning device
  const total = first.body[2] ?? 0;
  const out: PinCap[] = [];
  for (let i = 0; i < total; i++) {
    const b = (await sendCmd(MSG.PIN_CAPS, [i])).body; // [st,idx,total,pinlo,pinhi,caps,warn,bus,name...]
    out.push({
      pin: (b[3] ?? 0) | ((b[4] ?? 0) << 8),
      caps: b[5] ?? 0,
      warn: (b[6] ?? 0) === PINCAP.WARN,
      bus: b[7] ?? PINCAP.NO_BUS,
      note: dec.decode(Uint8Array.from(b.slice(8))),
    });
  }
  return out;
}

/** One row of the device's IO table (an output: role + pin + name). */
export interface IoRow {
  type: number; // CTRL.IO / PWM / RGB
  pin: number;
  flags: number; // bit0 = active-low
  arg: number; // RGB pixel count
  name: string;
}
/** Read the device's current IO table (the editable Configure-device view). */
export async function getIoConfig(): Promise<IoRow[]> {
  const first = await sendCmd(MSG.CONFIG_GET, [0]);
  if (first.status !== 0) return [];
  const n = first.body[2] ?? 0;
  const rows: IoRow[] = [];
  for (let i = 0; i < n; i++) {
    const b = (await sendCmd(MSG.CONFIG_GET, [i])).body; // [st,idx,n,type,pinlo,pinhi,flags,arglo,arghi,name...]
    rows.push({
      type: b[3] ?? 0,
      pin: (b[4] ?? 0) | ((b[5] ?? 0) << 8),
      flags: b[6] ?? 0,
      arg: (b[7] ?? 0) | ((b[8] ?? 0) << 8),
      name: dec.decode(Uint8Array.from(b.slice(9))),
    });
  }
  return rows;
}
/** Provision a new IO table. Validated per-row by the device; persists; applies
 *  after reboot. Resolves to a bad-row index on rejection, or null on success. */
export async function setIoConfig(rows: IoRow[]): Promise<number | null> {
  const body = [rows.length & 0xff];
  for (const r of rows) {
    const nm = Array.from(enc.encode(r.name)).slice(0, 31);
    body.push(r.type & 0xff, r.pin & 0xff, (r.pin >> 8) & 0xff, r.flags & 0xff,
      r.arg & 0xff, (r.arg >> 8) & 0xff, nm.length, ...nm);
  }
  const resp = await sendCmd(MSG.CONFIG_SET, body);
  if (resp.status === 0) return null;
  if (resp.status === 0x03) return resp.body[1] ?? 0; // BADARGS -> bad row index
  throw new Error(`device returned status 0x${(resp.status ?? 0).toString(16)}`);
}
/** Revert the device to its compiled-default IO table (applies after reboot). */
export async function resetIoConfig(): Promise<void> {
  const resp = await sendCmd(MSG.CONFIG_SET, [PINCAP.CONFIG_RESET]);
  if (resp.status !== 0) throw new Error(`reset failed (status 0x${(resp.status ?? 0).toString(16)})`);
}

// ---- INVOKE: user-defined commands (INVOKE_DESC / INVOKE) ----
// A device advertises its own command vocabulary; we forward a high-level intent
// to its handler. The device's list is authoritative for *what exists*; the
// catalog (invocables.json) enriches *recognized* ids with labels + widgets and
// every unknown (vendor) id still works from its raw arg signature.

/** INVOKE arg-type codes (INVOKE_DESC argtype bytes). */
export const ARG = { U8: 0, U16: 1, U32: 2, I16: 3, I32: 4, BYTES: 5, STR: 6 } as const;
export type ArgName = "u8" | "u16" | "u32" | "i16" | "i32" | "bytes" | "str";
const ARG_NAMES: ArgName[] = ["u8", "u16", "u32", "i16", "i32", "bytes", "str"];
const ARG_CODE: Record<ArgName, number> = { u8: 0, u16: 1, u32: 2, i16: 3, i32: 4, bytes: 5, str: 6 };
/** ids >= this are vendor / user-defined (the device owns the meaning). */
export const INVOKE_VENDOR_BASE = 0x8000;
/** INVOKE_DESC per-command flags. */
export const INVOKE_REPLY = 0x01;

/** A catalogued argument: a label + UI widget hint over the wire type. */
export interface InvocableArg {
  name: string;
  type: ArgName;
  widget?: string; // "number" | "slider" | "hex" | "text" | ...
  min?: number;
  max?: number;
  unit?: string;
}
/** One command the device exposes, merged with the client catalog. */
export interface Invocable {
  id: number;
  name: string; // device's own label (or the catalog name)
  label: string; // catalog label, else the device name
  description?: string;
  args: InvocableArg[]; // catalogued args if known, else generic from the signature
  argCodes: number[]; // the raw ARG codes the device advertised
  hasReply: boolean;
  vendor: boolean; // id >= INVOKE_VENDOR_BASE
  known: boolean; // matched a catalog entry
}

interface CatalogArg { name: string; type: ArgName; widget?: string; min?: number; max?: number; unit?: string }
interface CatalogCommand { id: number; name: string; label?: string; description?: string; args: CatalogArg[] }
const CATALOG: CatalogCommand[] = (invocableCatalog as { commands: CatalogCommand[] }).commands;

/** Generic arg name + type from a wire code, when the catalog doesn't know the id. */
function genericArg(code: number, i: number): InvocableArg {
  return { name: `arg${i}`, type: ARG_NAMES[code] ?? "u8" };
}

/** Merge a device-advertised command with the catalog (catalog enriches, never overrides existence). */
function mergeInvocable(id: number, name: string, argCodes: number[], flags: number): Invocable {
  const vendor = id >= INVOKE_VENDOR_BASE;
  const hit = CATALOG.find((c) => c.id === id);
  // Use the catalog's typed args only if its signature matches what the device reports.
  const matches =
    hit && hit.args.length === argCodes.length && hit.args.every((a, i) => ARG_CODE[a.type] === argCodes[i]);
  return {
    id,
    name: name || hit?.name || `cmd_0x${id.toString(16)}`,
    label: hit?.label ?? name ?? `0x${id.toString(16)}`,
    description: hit?.description,
    args: matches ? hit!.args.map((a) => ({ ...a })) : argCodes.map(genericArg),
    argCodes,
    hasReply: (flags & INVOKE_REPLY) !== 0,
    vendor,
    known: !!hit,
  };
}

/** The device's command menu, enriched by the client catalog. Empty if it exposes none. */
export async function invocables(): Promise<Invocable[]> {
  const first = await sendCmd(MSG.INVOKE_DESC, [0]);
  if (first.status !== 0) return []; // device exposes no INVOKE commands
  const total = first.body[2] ?? 0;
  const out: Invocable[] = [];
  for (let i = 0; i < total; i++) {
    const b = (await sendCmd(MSG.INVOKE_DESC, [i])).body; // [st,idx,total,idlo,idhi,nargs,at..,flags,name..]
    const id = (b[3] ?? 0) | ((b[4] ?? 0) << 8);
    const nargs = b[5] ?? 0;
    const argCodes = b.slice(6, 6 + nargs);
    const flags = b[6 + nargs] ?? 0;
    const name = dec.decode(Uint8Array.from(b.slice(7 + nargs)));
    out.push(mergeInvocable(id, name, argCodes, flags));
  }
  return out;
}

/** Pack argument values into an INVOKE payload per the wire codec (LE; bytes/str length-prefixed). */
export function packArgs(argCodes: number[], values: Array<number | number[] | string>): number[] {
  const out: number[] = [];
  argCodes.forEach((t, i) => {
    const v = values[i];
    if (t === ARG.U8) out.push((v as number) & 0xff);
    else if (t === ARG.U16) { const n = v as number; out.push(n & 0xff, (n >> 8) & 0xff); }
    else if (t === ARG.U32 || t === ARG.I32) { const n = (v as number) >>> 0; out.push(n & 0xff, (n >> 8) & 0xff, (n >> 16) & 0xff, (n >> 24) & 0xff); }
    else if (t === ARG.I16) { const n = (v as number) & 0xffff; out.push(n & 0xff, (n >> 8) & 0xff); }
    else if (t === ARG.BYTES) { const a = (v as number[]) ?? []; out.push(a.length & 0xff, ...a); }
    else if (t === ARG.STR) { const a = Array.from(new TextEncoder().encode((v as string) ?? "")); out.push(a.length & 0xff, ...a); }
  });
  return out;
}

/** Call a device command by id with a packed payload. Returns the echoed id + any reply bytes. */
export async function invokeCommand(id: number, payload: number[] = []): Promise<{ id: number; reply: number[] }> {
  const resp = await sendCmd(MSG.INVOKE, [id & 0xff, (id >> 8) & 0xff, ...payload]);
  if (resp.status !== 0) throw new Error(`invoke 0x${id.toString(16)}: status 0x${(resp.status ?? 0).toString(16)}`);
  return { id: (resp.body[1] ?? 0) | ((resp.body[2] ?? 0) << 8), reply: resp.body.slice(3) };
}

// ---- key-value config (CFG_GET/CFG_SET) + WiFi provisioning ----
export const CFG = {
  WIFI_SSID: 0x10,
  WIFI_PASS: 0x11,
  WIFI_STATUS: 0x12,
  DATA_PINS: 0x13,
  DATA_KIND: 0x14,
} as const;
/** WIFI_STATUS state byte. */
export const WIFI = { OFF: 0, CONNECTING: 1, CONNECTED: 2, PORTAL: 3, FAILED: 4 } as const;
export const WS_PORT = 9555;

/** Read a config key's raw value bytes (throws on unsupported/not-found). */
export async function cfgGet(key: number): Promise<number[]> {
  const resp = await sendCmd(MSG.CFG_GET, [key]);
  if (resp.status !== 0) throw new Error(`cfg 0x${key.toString(16)}: status 0x${(resp.status ?? 0).toString(16)}`);
  return resp.body.slice(2); // [status, key, value...]
}
/** Set a config key to a UTF-8 string value. */
export async function cfgSetStr(key: number, value: string): Promise<void> {
  const resp = await sendCmd(MSG.CFG_SET, [key, ...Array.from(new TextEncoder().encode(value))]);
  if (resp.status !== 0) throw new Error(`cfg set 0x${key.toString(16)}: status 0x${(resp.status ?? 0).toString(16)}`);
}

export interface WifiStatus {
  state: number; // WIFI.*
  detail: string; // IP when connected · AP name in portal · SSID otherwise
}
/** Read the device's WiFi state (state byte + detail string). */
export async function wifiStatus(): Promise<WifiStatus> {
  const v = await cfgGet(CFG.WIFI_STATUS);
  return { state: v[0] ?? 0, detail: dec.decode(Uint8Array.from(v.slice(1))) };
}
/** The DATA bridge UART's pins (tx/rx; -1 = none). Throws if the device has no CFG. */
export async function dataPins(): Promise<{ tx: number; rx: number }> {
  const v = await cfgGet(CFG.DATA_PINS);
  const s16 = (lo: number, hi: number) => {
    const n = lo | (hi << 8);
    return n >= 0x8000 ? n - 0x10000 : n;
  };
  return { tx: s16(v[0] ?? 0xff, v[1] ?? 0xff), rx: s16(v[2] ?? 0xff, v[3] ?? 0xff) };
}

// ---- I2C (DATA kind i2c) ----
/** Switch the bridged medium (DATA_KIND.UART <-> DATA_KIND.I2C); persisted on-device. */
export async function setDataKind(kind: number): Promise<void> {
  const resp = await sendCmd(MSG.CFG_SET, [CFG.DATA_KIND, kind]);
  if (resp.status !== 0) throw new Error(`kind switch: status 0x${(resp.status ?? 0).toString(16)}`);
}

/** Probe all 7-bit addresses; returns the ones that ACKed. */
export async function i2cScan(): Promise<number[]> {
  const resp = await sendCmd(MSG.I2C_SCAN);
  if (resp.status !== 0) throw new Error(`scan: status 0x${(resp.status ?? 0).toString(16)}`);
  const bitmap = resp.body.slice(1); // [status, bitmap(16)]
  const found: number[] = [];
  for (let a = 0; a < 128; a++) if ((bitmap[a >> 3] ?? 0) & (1 << (a & 7))) found.push(a);
  return found;
}

// ---- I2C device definitions (workspace .sutra/i2c/*.json) ----
export interface I2cReg {
  name: string;
  reg: number; // register / command pointer byte
  bytes?: number; // value width: 0 = command (no value), 1, 2 (default 1)
  access?: "r" | "rw" | "w"; // default rw
  control?: "number" | "toggle" | "slider" | "enum" | "button";
  min?: number;
  max?: number;
  options?: { label: string; value: number }[];
  desc?: string;
}
export interface I2cDef {
  name: string;
  addr: number; // 7-bit address
  registers: I2cReg[];
}
/** Load the I2C device definitions from the workspace's .sutra/i2c/. */
export const listI2cDefs = () => invoke<I2cDef[]>("list_i2c_defs");

/** Read a def register's value (handles the byte width). */
export async function i2cReadReg(addr: number, r: I2cReg): Promise<number> {
  const n = r.bytes ?? 1;
  const v = await i2cXfer(addr, [r.reg], n);
  return n === 2 ? (v[0] ?? 0) | ((v[1] ?? 0) << 8) : (v[0] ?? 0);
}
/** Write a def register (or send a command when bytes=0). */
export async function i2cWriteReg(addr: number, r: I2cReg, value: number): Promise<void> {
  const n = r.bytes ?? 1;
  const bytes = n === 0 ? [] : n === 2 ? [value & 0xff, (value >> 8) & 0xff] : [value & 0xff];
  await i2cXfer(addr, [r.reg, ...bytes], 0);
}

/** Master transfer: write `w`, then read `rlen` bytes (either may be empty). */
export async function i2cXfer(addr: number, w: number[], rlen: number): Promise<number[]> {
  const resp = await sendCmd(MSG.I2C_XFER, [addr, w.length, ...w, rlen]);
  if (resp.status !== 0) throw new Error(`xfer: status 0x${(resp.status ?? 0).toString(16)}`);
  return resp.body.slice(2); // [status, addr, r...]
}

/** One decoded i2c DATA record (one mux DATA frame = one record). */
export interface I2cRecord {
  ts: number; // device millis
  addr: number;
  read: boolean;
  nak: boolean;
  w: number[];
  r: number[];
}
/** Decode an i2c DATA-channel record; null if the payload is malformed. */
export function decodeI2cRecord(p: number[] | Uint8Array): I2cRecord | null {
  const b = Array.from(p);
  if (b.length < 8) return null;
  const wlen = b[6] ?? 0;
  const rlenAt = 7 + wlen;
  if (b.length < rlenAt + 1) return null;
  const rlen = b[rlenAt] ?? 0;
  if (b.length < rlenAt + 1 + rlen) return null;
  return {
    ts: (b[0] | (b[1] << 8) | (b[2] << 16) | (b[3] << 24)) >>> 0,
    addr: b[4],
    read: !!(b[5] & 1),
    nak: !!(b[5] & 2),
    w: b.slice(7, 7 + wlen),
    r: b.slice(rlenAt + 1, rlenAt + 1 + rlen),
  };
}

// ---- BLE sniffer (DATA kind ble-sniff) ----
const BLE_PDU_TYPE: Record<number, string> = {
  0: "ADV_IND",
  1: "ADV_DIRECT_IND",
  2: "ADV_NONCONN_IND",
  3: "SCAN_REQ",
  4: "SCAN_RSP",
  5: "CONNECT_IND",
  6: "ADV_SCAN_IND",
  7: "ADV_EXT_IND",
};
// A few common Bluetooth SIG company IDs (manufacturer-data, little-endian).
const BLE_COMPANY: Record<number, string> = {
  0x004c: "Apple",
  0x0006: "Microsoft",
  0x00e0: "Google",
  0x0075: "Samsung",
  0x0059: "Nordic",
  0x000f: "Broadcom",
  0x0001: "Ericsson",
  0x00d2: "Bose",
  0x0157: "Anhui Huami",
  0x0171: "Amazon",
};

export interface BleSniffPacket {
  ts: number; // device millis
  channel: number; // 37/38/39
  rssi: number; // negative dBm
  type: string; // PDU type name
  addr: string; // AdvA as a MAC string (or "" if none)
  name: string; // local name from AD, if any
  company: string; // manufacturer-data company, if any
  payloadHex: string; // raw AdvData (after AdvA) as hex
  raw: number[]; // the original record bytes (for pcap export)
}

const macStr = (b: number[]) =>
  b.map((x) => x.toString(16).padStart(2, "0").toUpperCase()).reverse().join(":");

/** Decode one ble-sniff DATA record: ts·ch·rssi·aa·len·pdu. Null if malformed. */
export function decodeBleSniff(p: number[] | Uint8Array): BleSniffPacket | null {
  const b = Array.from(p);
  if (b.length < 11) return null;
  const ts = (b[0] | (b[1] << 8) | (b[2] << 16) | (b[3] << 24)) >>> 0;
  const channel = b[4];
  const rssi = -b[5];
  const pduLen = b[10];
  const pdu = b.slice(11, 11 + pduLen);
  if (pdu.length < 2) return null;
  const type = BLE_PDU_TYPE[pdu[0] & 0x0f] ?? `0x${(pdu[0] & 0x0f).toString(16)}`;
  let addr = "";
  let payload: number[] = [];
  // PDU types that carry AdvA(6) + AdvData
  if ([0, 2, 4, 6, 7].includes(pdu[0] & 0x0f) && pdu.length >= 8) {
    addr = macStr(pdu.slice(2, 8));
    payload = pdu.slice(8);
  } else {
    payload = pdu.slice(2);
  }
  // walk the AD structures
  let name = "";
  let company = "";
  for (let i = 0; i + 1 < payload.length; ) {
    const len = payload[i];
    if (len === 0 || i + 1 + len > payload.length + 1) break;
    const adType = payload[i + 1];
    const data = payload.slice(i + 2, i + 1 + len);
    if (adType === 0x08 || adType === 0x09) {
      name = new TextDecoder().decode(Uint8Array.from(data));
    } else if (adType === 0xff && data.length >= 2) {
      const cid = data[0] | (data[1] << 8);
      company = BLE_COMPANY[cid] ?? `0x${cid.toString(16).padStart(4, "0")}`;
    }
    i += 1 + len;
  }
  const payloadHex = payload.map((x) => x.toString(16).padStart(2, "0").toUpperCase()).join(" ");
  return { ts, channel, rssi, type, addr, name, company, payloadHex, raw: b };
}

export interface Ieee154Frame {
  ts: number; // device millis
  channel: number; // 11..26
  rssi: number; // signed dBm
  lqi: number;
  type: string; // MAC frame-type name (Beacon/Data/Ack/Command, refined for commands)
  seq: number | null; // sequence number (null if suppressed)
  dstPan: string; // "0xABCD" or ""
  dst: string; // short "0x1234" / ext MAC / "Broadcast" / ""
  src: string; // short / ext / ""
  payloadHex: string; // MAC payload (after addressing, before FCS) as hex
  raw: number[]; // original record bytes (for pcap export)
  mac: number[]; // the MAC frame (no FCS) — feeds zdpIngest for the live interview
  tx?: boolean; // true = a frame WE injected (echoed back; the radio can't hear its own TX)
}

const IEEE154_FTYPE = ["Beacon", "Data", "Ack", "MAC Command", "0x4", "0x5", "0x6", "0x7"];
// a few common MAC command frame ids (the first payload byte of a command frame)
const IEEE154_CMD: Record<number, string> = {
  0x01: "Assoc Request",
  0x02: "Assoc Response",
  0x03: "Disassoc Notify",
  0x04: "Data Request",
  0x05: "PAN ID Conflict",
  0x06: "Orphan Notify",
  0x07: "Beacon Request",
  0x08: "Coordinator Realign",
  0x09: "GTS Request",
};
const hex16 = (lo: number, hi: number) =>
  `0x${(((hi << 8) | lo) >>> 0).toString(16).padStart(4, "0")}`;

/** Decode one ieee802154 DATA record: ts·ch·rssi·lqi·flags·len·psdu. Parses the
 *  MAC header (FCF addressing, PAN-ID compression) for a Devices/Packets view. */
export function decodeIeee154(p: number[] | Uint8Array): Ieee154Frame | null {
  const b = Array.from(p);
  if (b.length < 9) return null;
  const ts = (b[0] | (b[1] << 8) | (b[2] << 16) | (b[3] << 24)) >>> 0;
  const channel = b[4];
  const rssi = (b[5] << 24) >> 24; // signed
  const lqi = b[6];
  const plen = b[8];
  const mac = b.slice(9, 9 + plen - 2); // drop trailing FCS field
  if (mac.length < 3) return null;

  const fcf = mac[0] | (mac[1] << 8);
  const ftype = fcf & 0x07;
  const panComp = (fcf >> 6) & 1;
  const dstMode = (fcf >> 10) & 0x03; // 0 none, 2 short, 3 ext
  const frameVer = (fcf >> 12) & 0x03;
  const srcMode = (fcf >> 14) & 0x03;
  const seqSuppressed = frameVer === 2 && (fcf >> 8) & 1;

  let i = 2;
  const seq = seqSuppressed ? null : mac[i++] ?? null;

  let dstPan = "";
  let dst = "";
  let src = "";
  if (dstMode !== 0) {
    dstPan = hex16(mac[i], mac[i + 1]);
    i += 2;
    if (dstMode === 2) {
      dst = mac[i] === 0xff && mac[i + 1] === 0xff ? "Broadcast" : hex16(mac[i], mac[i + 1]);
      i += 2;
    } else if (dstMode === 3) {
      dst = macStr(mac.slice(i, i + 8));
      i += 8;
    }
  }
  if (srcMode !== 0) {
    if (!panComp) {
      i += 2; // src PAN (same as dst when compressed)
    }
    if (srcMode === 2) {
      src = hex16(mac[i], mac[i + 1]);
      i += 2;
    } else if (srcMode === 3) {
      src = macStr(mac.slice(i, i + 8));
      i += 8;
    }
  }

  const payload = mac.slice(i);
  let type = IEEE154_FTYPE[ftype] ?? `0x${ftype.toString(16)}`;
  if (ftype === 3 && payload.length >= 1) {
    type = IEEE154_CMD[payload[0]] ?? `Command 0x${payload[0].toString(16)}`;
  }
  const payloadHex = payload.map((x) => x.toString(16).padStart(2, "0").toUpperCase()).join(" ");
  return { ts, channel, rssi, lqi, type, seq, dstPan, dst, src, payloadHex, raw: b, mac };
}

/** Decode a raw injected MAC frame (no record header, no FCS) into the same
 *  shape as a captured frame, flagged `tx`. We synthesize a capture record
 *  (ts·ch·rssi·lqi·flags·len·psdu) so the normal decoder does the MAC parse —
 *  this is the "assumed send" we show because the radio can't hear its own TX. */
export function decodeIeee154Tx(mac: number[], channel: number): Ieee154Frame | null {
  // record = ts(4)·ch(1)·rssi(1)·lqi(1)·flags(1, fcs-ok)·len(1)·psdu(mac + 2 FCS)
  const rec = [0, 0, 0, 0, channel & 0xff, 0, 0xff, 0x01, mac.length + 2, ...mac, 0, 0];
  const f = decodeIeee154(rec);
  if (f) f.tx = true;
  return f;
}

/** Frames WE inject over the DATA channel, echoed back so the UI can show them
 *  (the sniffer radio can't capture its own transmissions). Raw MAC bytes. */
export async function onTx(cb: (mac: number[]) => void): Promise<UnlistenFn> {
  return listen<number[]>("sutra://tx", (e) => cb(e.payload));
}

/** A node fact recovered from a decrypted ZDP reply (see zdpIngest). */
export interface ZdpDiscovery {
  addr: string; // "0xabcd"
  kind: string; // active_ep | simple_desc | node_desc
  endpoints: number[];
  endpoint: number | null;
  in_clusters: string[];
  out_clusters: string[];
  manufacturer: string | null;
}

/** Try to decode a sniffed MAC frame (no FCS) as a ZDP reply against the active
 *  network; on success the backend merges it into the node model. Returns the
 *  discovery, or null if the frame isn't a decryptable ZDP reply. */
export const zdpIngest = (mac: number[]) =>
  invoke<ZdpDiscovery | null>("zdp_ingest", { frame: mac });

/** Batch-observe sniffed MAC frames against the active network: ZDP replies feed
 *  active discovery, other application frames passively record endpoints/clusters.
 *  Returns how many frames changed the node model. */
export const observeFrames = (frames: number[][]) =>
  invoke<number>("observe_frames", { frames });

/** Provision WiFi over the CMD link: password first, then SSID (SSID triggers the join). */
export async function wifiConfigure(ssid: string, password: string): Promise<void> {
  await cfgSetStr(CFG.WIFI_PASS, password);
  await cfgSetStr(CFG.WIFI_SSID, ssid);
}

// ---- self-describe ----
const dec = new TextDecoder();
export interface ControlDesc {
  index: number;
  type: number; // outputs: 0 = io, 1 = pwm, 2 = rgb · inputs: 0 = digital, 1 = analog
  name: string;
}
export async function getDeviceName(): Promise<string> {
  const b = (await sendCmd(MSG.DEVICE_NAME)).body; // [status, name...]
  return dec.decode(Uint8Array.from(b.slice(1)));
}

/** What the device's DATA channel carries (so the app can pick a viewer). */
export const DATA_KIND = { UART: 0, CAN: 1, RS485: 2, SPI: 3, BLE_SNIFF: 4, LOGIC: 5, I2C: 6, IEEE802154: 7 } as const;
export interface DataDesc {
  kind: number;
  name: string;
}
export async function getDataDesc(): Promise<DataDesc> {
  const b = (await sendCmd(MSG.DATA_DESC)).body; // [status, kind, name...]
  return { kind: b[1] ?? 0, name: dec.decode(Uint8Array.from(b.slice(2))) || "UART" };
}
export async function getOutputDesc(index: number): Promise<ControlDesc> {
  const b = (await sendCmd(MSG.OUTPUT_DESC, [index])).body; // [status, index, type, name...]
  return { index: b[1] ?? index, type: b[2] ?? 0, name: dec.decode(Uint8Array.from(b.slice(3))) };
}
/** Enumerate the device's named controls (count from INFO, label/type per index). */
export async function getControls(): Promise<ControlDesc[]> {
  const info = await getInfo();
  const out: ControlDesc[] = [];
  for (let i = 0; i < info.nOutputs; i++) {
    try {
      out.push(await getOutputDesc(i));
    } catch {
      /* skip a control that fails to describe */
    }
  }
  return out;
}
/** Output states as a bitmap (bit i = control index i). */
export const outputsBitmap = async () => (await sendCmd(MSG.OUTPUT_GET)).body[1] ?? 0;

// inputs (digital/analog): mirror of the output self-describe
export async function getInputDesc(index: number): Promise<ControlDesc> {
  const b = (await sendCmd(MSG.INPUT_DESC, [index])).body; // [status, index, type, name...]
  return { index: b[1] ?? index, type: b[2] ?? 0, name: dec.decode(Uint8Array.from(b.slice(3))) };
}
export async function getInputs(): Promise<ControlDesc[]> {
  const info = await getInfo();
  const out: ControlDesc[] = [];
  for (let i = 0; i < info.nInputs; i++) {
    try {
      out.push(await getInputDesc(i));
    } catch {
      /* skip */
    }
  }
  return out;
}
/** Read an input's current value (digital 0/1, analog 0-1023). */
export async function readInput(index: number): Promise<number> {
  const b = (await sendCmd(MSG.INPUT_GET, [index])).body; // [status, index, lo, hi]
  return ((b[3] ?? 0) << 8) | (b[2] ?? 0);
}

const enc = new TextEncoder();
/** Send a macro's text straight out the DATA/UART now (raw, no macros). */
export const runTextNow = (text: string) => dataWrite(Array.from(enc.encode(text)));
/** Run macro text through the macro player; `name` labels it in the run queue. */
export const runText = (text: string, name?: string) =>
  invoke<void>("run_text", { text, name });

// ---- run queue (in-flight macros; cancellable) ----
export interface MacroRunInfo {
  id: number;
  name: string;
  status: string;
}
export const macroRuns = () => invoke<MacroRunInfo[]>("macro_runs");
export const cancelRun = (id: number) => invoke<void>("cancel_run", { id });
/** Fires whenever the set of in-flight runs changes (start / status / finish). */
export async function onRuns(cb: (runs: MacroRunInfo[]) => void): Promise<UnlistenFn> {
  return listen<MacroRunInfo[]>("sutra://runs", (e) => cb(e.payload));
}

// ---- DATA serial params ----
export interface SerialParams {
  baud: number;
  data_bits: number;
  parity: "none" | "odd" | "even";
  stop_bits: number;
}
export interface ConnStateT {
  connected: boolean;
  data_port: string | null;
  cmd_port: string | null;
  has_cmd: boolean;
  params: SerialParams;
}
export const connState = () => invoke<ConnStateT>("conn_state");
export const setDataParams = (params: SerialParams) =>
  invoke<void>("set_data_params", { params });
export const reconnectData = () => invoke<void>("reconnect_data");
/** Last `max` bytes of the DATA console buffer (lossy UTF-8). */
export const readConsole = (max: number) => invoke<string>("read_console", { max });

// ---- MCP server ----
export interface McpStatus {
  running: boolean;
  url: string | null;
}
export const mcpStart = (port: number) => invoke<McpStatus>("mcp_start", { port });
export const mcpStop = () => invoke<McpStatus>("mcp_stop");
export const mcpStatus = () => invoke<McpStatus>("mcp_status");

/** Which MCP tool groups are exposed to the LLM. */
export interface McpToolFlags {
  consoleRead: boolean;
  consoleWrite: boolean;
  outputs: boolean;
  macrosRun: boolean;
  macrosCreate: boolean;
  connection: boolean;
}
export const setMcpTools = (flags: McpToolFlags) => invoke<void>("set_mcp_tools", { flags });

// ---- macro store (backend-owned; shared with the MCP server) ----
export interface MacroRec {
  name: string;
  text: string;
  secret: boolean;
  set: string; // project/collection ("" = default)
  tier: number; // derived skrit-mc tier: 1=replay, 2=interactive, 3=app-only
}

/** Label + short title for a skrit-mc tier. */
export const TIER_INFO: Record<number, { label: string; title: string }> = {
  1: { label: "Replay", title: "open-loop: emit / delay / set output, runs on any device" },
  2: { label: "Interactive", title: "closed-loop: waits on / branches on a read (expect, input)" },
  3: { label: "App-only", title: "host orchestration (RUN exit codes), Sutra player only" },
};
export const macrosGet = () => invoke<MacroRec[]>("macros_get");
export const macroUpsert = (name: string, text: string, secret: boolean, set: string) =>
  invoke<void>("macro_upsert", { name, text, secret, set });
/** Export a set (or all macros if set is omitted) to a JSON file at `path`. */
export const exportSet = (path: string, set?: string) =>
  invoke<void>("export_set", { path, set: set ?? null });
/** Import a macro-set JSON file; returns how many were merged. */
export const importSet = (path: string) => invoke<number>("import_set", { path });
export const macroDelete = (name: string) => invoke<void>("macro_delete", { name });
/** Replace the whole store (used to persist a reorder). */
export const macrosSet = (macros: MacroRec[]) =>
  invoke<void>("macros_set", { macros });
/** Fires when the store changes (incl. macros the LLM creates via MCP). */
export async function onMacros(cb: (list: MacroRec[]) => void): Promise<UnlistenFn> {
  return listen<MacroRec[]>("sutra://macros", (e) => cb(e.payload));
}

// ---- macro -> device EEPROM (Save to buddi) ----
export function statusText(s: number | null): string {
  switch (s) {
    case 0: return "ok";
    case 4: return "device has no macro storage";
    case 5: return "not found";
    default: return `error (status ${s})`;
  }
}

function crc16(data: number[]): number {
  let crc = 0xffff;
  for (const b of data) {
    crc ^= b << 8;
    for (let i = 0; i < 8; i++) crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
  }
  return crc & 0xffff;
}

const DEV_CHUNK = 60; // MACRO_WRITE_DATA body = id+off(2)+bytes <= 64

/** Push a macro into the device's EEPROM store (graceful STORAGE_ERR until fitted). */
export async function saveMacroToDevice(
  id: number,
  name: string,
  text: string
): Promise<string> {
  const nameB = Array.from(enc.encode(name)).slice(0, 16);
  const data = Array.from(enc.encode(text));
  const total = data.length;
  let r = await sendCmd(MSG.MACRO_WRITE_BEGIN, [
    id, total & 0xff, (total >> 8) & 0xff, nameB.length, ...nameB,
  ]);
  if (r.status !== 0) return statusText(r.status);
  for (let off = 0; off < total; off += DEV_CHUNK) {
    const chunk = data.slice(off, off + DEV_CHUNK);
    r = await sendCmd(MSG.MACRO_WRITE_DATA, [id, off & 0xff, (off >> 8) & 0xff, ...chunk]);
    if (r.status !== 0) return statusText(r.status);
  }
  const c = crc16(data);
  r = await sendCmd(MSG.MACRO_WRITE_END, [id, c & 0xff, (c >> 8) & 0xff]);
  return statusText(r.status);
}
