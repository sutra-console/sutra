// Front-end mirror of the sutra CMD protocol — see docs/PROTOCOL.md
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const MSG = {
  PING: 0x01,
  INFO: 0x02,
  OUTPUT_SET: 0x10,
  OUTPUT_GET: 0x11,
  OUTPUT_TOGGLE: 0x12,
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
  is_sutra: boolean;
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
/** Connect a DATA port. Pass cmdPort for a sutra, or null/omit for any generic serial port. */
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

const enc = new TextEncoder();
/** Send a snippet's text straight out the DATA/UART now (raw, no macros). */
export const runTextNow = (text: string) => dataWrite(Array.from(enc.encode(text)));
/** Run snippet text through the macro player (escapes + `+++DELAY/ENTER/CTRL...+++`). */
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

// ---- MCP server ----
export interface McpStatus {
  running: boolean;
  url: string | null;
}
export const mcpStart = (port: number) => invoke<McpStatus>("mcp_start", { port });
export const mcpStop = () => invoke<McpStatus>("mcp_stop");
export const mcpStatus = () => invoke<McpStatus>("mcp_status");

// ---- snippet store (backend-owned; shared with the MCP server) ----
export interface SnippetRec {
  name: string;
  text: string;
  secret: boolean;
}
export const snippetsGet = () => invoke<SnippetRec[]>("snippets_get");
export const snippetUpsert = (name: string, text: string, secret: boolean) =>
  invoke<void>("snippet_upsert", { name, text, secret });
export const snippetDelete = (name: string) => invoke<void>("snippet_delete", { name });
/** Fires when the store changes (incl. snippets the LLM creates via MCP). */
export async function onSnippets(cb: (list: SnippetRec[]) => void): Promise<UnlistenFn> {
  return listen<SnippetRec[]>("ttl://snippets", (e) => cb(e.payload));
}

// ---- snippet -> device EEPROM (Save to buddi) ----
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

/** Push a snippet into the device's EEPROM store (graceful STORAGE_ERR until fitted). */
export async function saveSnippetToDevice(
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
