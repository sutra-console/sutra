// Front-end mirror of the skrit CMD protocol — see protocol/PROTOCOL.md
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const MSG = {
  PING: 0x01,
  INFO: 0x02,
  DEVICE_NAME: 0x03,
  OUTPUT_SET: 0x10,
  OUTPUT_GET: 0x11,
  OUTPUT_TOGGLE: 0x12,
  OUTPUT_DESC: 0x13,
  INPUT_DESC: 0x14,
  INPUT_GET: 0x15,
  SNIP_LIST: 0x20,
  SNIP_META: 0x21,
  SNIP_READ: 0x22,
  SNIP_WRITE_BEGIN: 0x23,
  SNIP_WRITE_DATA: 0x24,
  SNIP_WRITE_END: 0x25,
  SNIP_DELETE: 0x26,
  SNIP_RUN: 0x27,
} as const;

export const OUTPUT = { R1: 0, R2: 1, LED: 2 } as const;
export const RESP_FLAG = 0x80;

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
/** Connect a DATA port. Pass cmdPort for a Duta, or null/omit for any generic serial port. */
export const connect = (dataPort: string, cmdPort?: string | null) =>
  invoke<void>("connect", { dataPort, cmdPort: cmdPort ?? null });
export const disconnect = () => invoke<void>("disconnect");
export const dataWrite = (bytes: number[]) => invoke<void>("data_write", { bytes });
export const sendCmd = (typ: number, body: number[] = []) =>
  invoke<RespFrame>("send_cmd", { typ, body });

/** Subscribe to raw DATA-port bytes (target console). */
export async function onData(cb: (bytes: Uint8Array) => void): Promise<UnlistenFn> {
  return listen<number[]>("ttl://data", (e) => cb(Uint8Array.from(e.payload)));
}

/** Target link state: fires false when the DATA port drops (unplug / device
 *  reset) and true when it auto-recovers. The connection stays open throughout. */
export async function onLink(cb: (online: boolean) => void): Promise<UnlistenFn> {
  return listen<boolean>("ttl://link", (e) => cb(e.payload));
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

export const snipRun = (id: number) => sendCmd(MSG.SNIP_RUN, [id]);

// device capability bits (INFO body[3])
export const CAP = { EEPROM: 0x01, OLED: 0x02, SPI: 0x04, PARITY: 0x08 } as const;

export interface DeviceInfo {
  fwVer: number;
  caps: number;
  nOutputs: number;
  eepromKb: number;
  protoVer: number;
  nInputs: number;
}
export async function getInfo(): Promise<DeviceInfo> {
  const b = (await sendCmd(MSG.INFO)).body; // [status, fwlo, fwhi, caps, nout, eekb, ver, nin?]
  return {
    fwVer: ((b[2] ?? 0) << 8) | (b[1] ?? 0),
    caps: b[3] ?? 0,
    nOutputs: b[4] ?? 0,
    eepromKb: b[5] ?? 0,
    protoVer: b[6] ?? 0,
    nInputs: b[7] ?? 0,
  };
}

// ---- self-describe ----
const dec = new TextDecoder();
export interface ControlDesc {
  index: number;
  type: number; // 0 = relay, 1 = led
  name: string;
}
export async function getDeviceName(): Promise<string> {
  const b = (await sendCmd(MSG.DEVICE_NAME)).body; // [status, name...]
  return dec.decode(Uint8Array.from(b.slice(1)));
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

// inputs (digital/analog) — mirror of the output self-describe
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
/** Run macro text through the macro player (escapes + `+++DELAY/ENTER/CTRL...+++`). */
export const runText = (text: string) => invoke<void>("run_text", { text });

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
}
export const macrosGet = () => invoke<MacroRec[]>("macros_get");
export const macroUpsert = (name: string, text: string, secret: boolean) =>
  invoke<void>("macro_upsert", { name, text, secret });
export const macroDelete = (name: string) => invoke<void>("macro_delete", { name });
/** Replace the whole store (used to persist a reorder). */
export const macrosSet = (macros: MacroRec[]) =>
  invoke<void>("macros_set", { macros });
/** Fires when the store changes (incl. macros the LLM creates via MCP). */
export async function onMacros(cb: (list: MacroRec[]) => void): Promise<UnlistenFn> {
  return listen<MacroRec[]>("ttl://macros", (e) => cb(e.payload));
}

// ---- macro -> device EEPROM (Save to buddi) ----
export function statusText(s: number | null): string {
  switch (s) {
    case 0: return "ok";
    case 4: return "no EEPROM on device yet";
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

const DEV_CHUNK = 60; // SNIP_WRITE_DATA body = id+off(2)+bytes <= 64

/** Push a macro into the device's EEPROM store (graceful STORAGE_ERR until fitted). */
export async function saveMacroToDevice(
  id: number,
  name: string,
  text: string
): Promise<string> {
  const nameB = Array.from(enc.encode(name)).slice(0, 16);
  const data = Array.from(enc.encode(text));
  const total = data.length;
  let r = await sendCmd(MSG.SNIP_WRITE_BEGIN, [
    id, total & 0xff, (total >> 8) & 0xff, nameB.length, ...nameB,
  ]);
  if (r.status !== 0) return statusText(r.status);
  for (let off = 0; off < total; off += DEV_CHUNK) {
    const chunk = data.slice(off, off + DEV_CHUNK);
    r = await sendCmd(MSG.SNIP_WRITE_DATA, [id, off & 0xff, (off >> 8) & 0xff, ...chunk]);
    if (r.status !== 0) return statusText(r.status);
  }
  const c = crc16(data);
  r = await sendCmd(MSG.SNIP_WRITE_END, [id, c & 0xff, (c >> 8) & 0xff]);
  return statusText(r.status);
}
