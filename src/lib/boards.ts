// App-side board database — pin maps the Board view renders, mirroring the Duta
// firmware headers (platforms/<plat>/src/{mcu,boards,targets}).
//
// Layered like the headers: an `mcu` silicon table (reused across boards, kept
// here in code) + a board `def` overlay (broken-out + onboard uses + our role
// pins + I²C default). Built-in defs ship below; extra/override defs are loaded
// at runtime from local JSON files (<app_data>/boards/*.json) — drop a file to
// add a board without a rebuild, and a future board-defs server can pull into
// the same folder. Matched to a device by its self-described name (getDeviceName).
import { PINCAP, listBoards } from "@/lib/skrit";

const FREE = PINCAP.DIGITAL | PINCAP.ADC | PINCAP.PWM | PINCAP.I2C; // ADC + PWM + matrix I²C
const DIG = PINCAP.DIGITAL | PINCAP.PWM | PINCAP.I2C; // no ADC
const ADCONLY = PINCAP.ADC; // input-only ADC (classic ESP32 GP34-39)
const DACP = FREE | PINCAP.DAC; // ADC2 pin that also drives a DAC (classic ESP32 GP25/26)
const RP = PINCAP.DIGITAL | PINCAP.PWM | PINCAP.I2C | PINCAP.SPI; // RP2040/RP2350 (no ADC)
const RPA = RP | PINCAP.ADC; // RP2 ADC-capable (GP26-28)

export type PinStatus = "free" | "caution" | "forbidden";

export interface McuPin {
  pin: number;
  caps: number; // PINCAP.* bitfield
  status: PinStatus; // silicon hazard
  note?: string; // why caution/forbidden
}
export interface PinUse {
  pin: number;
  use: "fixed" | "dual"; // wired onboard: fixed = hidden, dual = offered+warned
  what: string;
}
export interface RolePin {
  pin: number;
  role: string; // our target wiring
}
/** A board definition (the file/JSON shape): references an mcu table by name. */
export interface BoardDef {
  id: string; // matches the device's self-described name (BOARD_NAME)
  name: string;
  vendor: string;
  model: string;
  mcu: string; // key into MCUS
  brokenOut?: number[]; // GPIOs on a header/pad
  brokenOutAll?: boolean; // devkit: every mcu pin is broken out
  uses: PinUse[]; // onboard hardware commitments
  roles: RolePin[]; // DATA bridge / RGB / relays we wire
  i2c?: { sda: number; scl: number }; // active I²C bridge default (duta_i2c.h)
}
/** A resolved board: its def plus the mcu's silicon pin table. */
export interface Board extends BoardDef {
  mcuPins: McuPin[];
}

// ---- mcu silicon tables (mcu/<chip>.h) -------------------------------------
const ESP32S3_PINS: McuPin[] = [
  { pin: 0, caps: FREE, status: "caution", note: "strapping (boot)" },
  { pin: 1, caps: FREE, status: "free" },
  { pin: 2, caps: FREE, status: "free" },
  { pin: 3, caps: FREE, status: "caution", note: "strapping (JTAG source)" },
  { pin: 4, caps: FREE, status: "free" },
  { pin: 5, caps: FREE, status: "free" },
  { pin: 6, caps: FREE, status: "free" },
  { pin: 7, caps: FREE, status: "free" },
  { pin: 8, caps: FREE, status: "free" },
  { pin: 9, caps: FREE, status: "free" },
  { pin: 10, caps: FREE, status: "free" },
  { pin: 11, caps: FREE, status: "free" },
  { pin: 12, caps: FREE, status: "free" },
  { pin: 13, caps: FREE, status: "free" },
  { pin: 14, caps: FREE, status: "free" },
  { pin: 15, caps: FREE, status: "free" },
  { pin: 16, caps: FREE, status: "free" },
  { pin: 17, caps: FREE, status: "free" },
  { pin: 18, caps: FREE, status: "free" },
  { pin: 19, caps: FREE, status: "forbidden", note: "USB D−" },
  { pin: 20, caps: FREE, status: "forbidden", note: "USB D+" },
  { pin: 21, caps: DIG, status: "free" },
  { pin: 26, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 27, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 28, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 29, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 30, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 31, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 32, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 33, caps: DIG, status: "caution", note: "octal PSRAM (-R8)" },
  { pin: 34, caps: DIG, status: "caution", note: "octal PSRAM (-R8)" },
  { pin: 35, caps: DIG, status: "caution", note: "octal PSRAM (-R8)" },
  { pin: 36, caps: DIG, status: "caution", note: "octal PSRAM (-R8)" },
  { pin: 37, caps: DIG, status: "caution", note: "octal PSRAM (-R8)" },
  { pin: 38, caps: DIG, status: "free" },
  { pin: 39, caps: DIG, status: "free" },
  { pin: 40, caps: DIG, status: "free" },
  { pin: 41, caps: DIG, status: "free" },
  { pin: 42, caps: DIG, status: "free" },
  { pin: 43, caps: DIG, status: "free", note: "U0TXD (console)" },
  { pin: 44, caps: DIG, status: "free", note: "U0RXD (console)" },
  { pin: 45, caps: DIG, status: "caution", note: "strapping" },
  { pin: 46, caps: DIG, status: "caution", note: "strapping" },
  { pin: 47, caps: DIG, status: "free" },
  { pin: 48, caps: DIG, status: "free" },
];

const ESP32C6_PINS: McuPin[] = [
  { pin: 0, caps: FREE, status: "free" },
  { pin: 1, caps: FREE, status: "free" },
  { pin: 2, caps: FREE, status: "free" },
  { pin: 3, caps: FREE, status: "free" },
  { pin: 4, caps: FREE, status: "caution", note: "strapping (MTMS / JTAG)" },
  { pin: 5, caps: FREE, status: "caution", note: "strapping (MTDI / JTAG)" },
  { pin: 6, caps: FREE, status: "free" },
  { pin: 7, caps: DIG, status: "free" },
  { pin: 8, caps: DIG, status: "caution", note: "strapping; onboard WS2812" },
  { pin: 9, caps: DIG, status: "caution", note: "strapping (boot button)" },
  { pin: 10, caps: DIG, status: "free" },
  { pin: 11, caps: DIG, status: "free" },
  { pin: 12, caps: DIG, status: "forbidden", note: "USB D−" },
  { pin: 13, caps: DIG, status: "forbidden", note: "USB D+" },
  { pin: 14, caps: DIG, status: "free" },
  { pin: 15, caps: DIG, status: "caution", note: "strapping" },
  { pin: 16, caps: DIG, status: "free", note: "U0TXD → CH343" },
  { pin: 17, caps: DIG, status: "free", note: "U0RXD → CH343" },
  { pin: 18, caps: DIG, status: "free" },
  { pin: 19, caps: DIG, status: "free" },
  { pin: 20, caps: DIG, status: "free" },
  { pin: 21, caps: DIG, status: "free" },
  { pin: 22, caps: DIG, status: "free" },
  { pin: 23, caps: DIG, status: "free" },
  { pin: 24, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 25, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 26, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 27, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 28, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 29, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 30, caps: DIG, status: "forbidden", note: "SPI flash" },
];

// ESP32-C3: GP0-5 have ADC, GP6+ don't; GP11-19 internal (flash/USB).
const ESP32C3_PINS: McuPin[] = [
  { pin: 0, caps: FREE, status: "free" },
  { pin: 1, caps: FREE, status: "free" },
  { pin: 2, caps: FREE, status: "caution", note: "strapping" },
  { pin: 3, caps: FREE, status: "free" },
  { pin: 4, caps: FREE, status: "free" },
  { pin: 5, caps: FREE, status: "free" },
  { pin: 6, caps: DIG, status: "free" },
  { pin: 7, caps: DIG, status: "free" },
  { pin: 8, caps: DIG, status: "caution", note: "strapping; onboard WS2812" },
  { pin: 9, caps: DIG, status: "caution", note: "strapping (boot)" },
  { pin: 10, caps: DIG, status: "free" },
  { pin: 11, caps: DIG, status: "forbidden", note: "VDD_SPI" },
  { pin: 12, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 13, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 14, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 15, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 16, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 17, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 18, caps: DIG, status: "forbidden", note: "USB D−" },
  { pin: 19, caps: DIG, status: "forbidden", note: "USB D+" },
  { pin: 20, caps: DIG, status: "free", note: "U0RXD" },
  { pin: 21, caps: DIG, status: "free", note: "U0TXD" },
];

// Classic ESP32: ADC2 (GP0,2,4,12-15,25-27) + ADC1 (GP32-39); GP34-39 input-only;
// GP25/26 add a DAC; GP6-11 are the SPI flash.
const ESP32_PINS: McuPin[] = [
  { pin: 0, caps: FREE, status: "caution", note: "strapping (boot)" },
  { pin: 1, caps: DIG, status: "free", note: "U0TXD" },
  { pin: 2, caps: FREE, status: "caution", note: "strapping" },
  { pin: 3, caps: DIG, status: "free", note: "U0RXD" },
  { pin: 4, caps: FREE, status: "free" },
  { pin: 5, caps: DIG, status: "caution", note: "strapping" },
  { pin: 6, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 7, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 8, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 9, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 10, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 11, caps: DIG, status: "forbidden", note: "SPI flash" },
  { pin: 12, caps: FREE, status: "caution", note: "strapping" },
  { pin: 13, caps: FREE, status: "free" },
  { pin: 14, caps: FREE, status: "free" },
  { pin: 15, caps: FREE, status: "caution", note: "strapping" },
  { pin: 16, caps: FREE, status: "free" },
  { pin: 17, caps: FREE, status: "free" },
  { pin: 18, caps: FREE, status: "free" },
  { pin: 19, caps: FREE, status: "free" },
  { pin: 21, caps: FREE, status: "free" },
  { pin: 22, caps: FREE, status: "free" },
  { pin: 23, caps: FREE, status: "free" },
  { pin: 25, caps: DACP, status: "free" },
  { pin: 26, caps: DACP, status: "free" },
  { pin: 27, caps: FREE, status: "free" },
  { pin: 32, caps: FREE, status: "free" },
  { pin: 33, caps: FREE, status: "free" },
  { pin: 34, caps: ADCONLY, status: "free", note: "input-only" },
  { pin: 35, caps: ADCONLY, status: "free", note: "input-only" },
  { pin: 36, caps: ADCONLY, status: "free", note: "input-only" },
  { pin: 39, caps: ADCONLY, status: "free", note: "input-only" },
];

// RP2040 / RP2350 (shared): GP0-22 digital, GP26-28 add ADC. No strapping/flash in
// the user GPIO range; I²C/SPI route flexibly.
const RP2_PINS: McuPin[] = [
  ...Array.from({ length: 23 }, (_, pin) => ({ pin, caps: RP, status: "free" as PinStatus })),
  { pin: 26, caps: RPA, status: "free" },
  { pin: 27, caps: RPA, status: "free" },
  { pin: 28, caps: RPA, status: "free" },
];

/** Silicon pin tables by mcu name (board defs reference these). */
export const MCUS: Record<string, McuPin[]> = {
  "ESP32-S3": ESP32S3_PINS,
  "ESP32-C6": ESP32C6_PINS,
  "ESP32-C3": ESP32C3_PINS,
  ESP32: ESP32_PINS,
  RP2040: RP2_PINS,
  RP2350: RP2_PINS,
};

// ---- built-in board defs (targets/*.h) — seedable to localfiles later ------
const BUILTIN_DEFS: BoardDef[] = [
  {
    id: "Duta S3-Zero",
    name: "Duta S3-Zero",
    vendor: "Waveshare",
    model: "ESP32-S3-Zero",
    mcu: "ESP32-S3",
    brokenOut: [
      1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
      38, 39, 40, 41, 42, 43, 44, 45,
    ],
    uses: [
      { pin: 21, use: "fixed", what: "onboard WS2812" },
      { pin: 0, use: "fixed", what: "BOOT button" },
    ],
    roles: [
      { pin: 43, role: "DATA UART TX" },
      { pin: 44, role: "DATA UART RX" },
      { pin: 21, role: "RGB LED" },
    ],
    i2c: { sda: 2, scl: 1 }, // target override (duta_s3_zero.h): SCL=GP1, SDA=GP2
  },
  {
    id: "Duta ESP32-C6",
    name: "Duta ESP32-C6",
    vendor: "Espressif",
    model: "ESP32-C6-DevKitC-1",
    mcu: "ESP32-C6",
    brokenOutAll: true, // devkit: every GPIO on a header
    uses: [{ pin: 8, use: "dual", what: "onboard WS2812" }],
    roles: [
      { pin: 16, role: "DATA UART TX" },
      { pin: 17, role: "DATA UART RX" },
      { pin: 8, role: "RGB LED" },
    ],
    i2c: { sda: 8, scl: 9 },
  },
  {
    id: "Duta ESP32-S3",
    name: "Duta ESP32-S3",
    vendor: "Espressif",
    model: "ESP32-S3-DevKitC-1",
    mcu: "ESP32-S3",
    brokenOutAll: true,
    uses: [{ pin: 48, use: "dual", what: "onboard WS2812" }],
    roles: [
      { pin: 17, role: "DATA UART TX" },
      { pin: 18, role: "DATA UART RX" },
      { pin: 48, role: "RGB LED" },
    ],
    i2c: { sda: 8, scl: 9 },
  },
  {
    id: "Duta ESP32-C3",
    name: "Duta ESP32-C3",
    vendor: "Espressif",
    model: "ESP32-C3-DevKitM-1",
    mcu: "ESP32-C3",
    brokenOutAll: true,
    uses: [{ pin: 8, use: "dual", what: "onboard WS2812" }],
    roles: [
      { pin: 21, role: "DATA UART TX" },
      { pin: 20, role: "DATA UART RX" },
      { pin: 8, role: "RGB LED" },
    ],
    i2c: { sda: 8, scl: 9 },
  },
  {
    id: "Duta ESP32",
    name: "Duta ESP32",
    vendor: "Espressif",
    model: "ESP32 DevKit",
    mcu: "ESP32",
    brokenOutAll: true,
    uses: [{ pin: 2, use: "dual", what: "onboard LED" }],
    roles: [
      { pin: 17, role: "DATA UART TX" },
      { pin: 16, role: "DATA UART RX" },
      { pin: 2, role: "Status LED" },
    ],
    i2c: { sda: 8, scl: 9 },
  },
  {
    id: "Duta XIAO ESP32-S3",
    name: "Duta XIAO ESP32-S3",
    vendor: "Seeed Studio",
    model: "XIAO ESP32-S3",
    mcu: "ESP32-S3",
    brokenOut: [1, 2, 3, 4, 5, 6, 7, 8, 9, 43, 44],
    uses: [
      { pin: 21, use: "fixed", what: "onboard LED" },
      { pin: 0, use: "fixed", what: "BOOT button" },
    ],
    roles: [
      { pin: 43, role: "DATA UART TX" },
      { pin: 44, role: "DATA UART RX" },
      { pin: 21, role: "Status LED" },
    ],
    i2c: { sda: 8, scl: 9 },
  },
  {
    id: "Duta XIAO ESP32-C3",
    name: "Duta XIAO ESP32-C3",
    vendor: "Seeed Studio",
    model: "XIAO ESP32-C3",
    mcu: "ESP32-C3",
    brokenOut: [2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 21],
    uses: [{ pin: 9, use: "dual", what: "BOOT button" }],
    roles: [
      { pin: 21, role: "DATA UART TX" },
      { pin: 20, role: "DATA UART RX" },
    ],
    i2c: { sda: 8, scl: 9 },
  },
  {
    id: "Duta Pico",
    name: "Duta Pico",
    vendor: "Raspberry Pi",
    model: "Pico",
    mcu: "RP2040",
    brokenOut: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 26, 27, 28],
    uses: [{ pin: 25, use: "fixed", what: "onboard LED" }],
    roles: [
      { pin: 0, role: "DATA UART TX" },
      { pin: 1, role: "DATA UART RX" },
    ],
  },
  {
    id: "Duta Pico 2",
    name: "Duta Pico 2",
    vendor: "Raspberry Pi",
    model: "Pico 2",
    mcu: "RP2350",
    brokenOut: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 26, 27, 28],
    uses: [{ pin: 25, use: "fixed", what: "onboard LED" }],
    roles: [
      { pin: 0, role: "DATA UART TX" },
      { pin: 1, role: "DATA UART RX" },
    ],
  },
  {
    id: "Duta XIAO RP2040",
    name: "Duta XIAO RP2040",
    vendor: "Seeed Studio",
    model: "XIAO RP2040",
    mcu: "RP2040",
    brokenOut: [0, 1, 2, 3, 4, 6, 7, 26, 27, 28, 29],
    uses: [
      { pin: 12, use: "fixed", what: "onboard WS2812" },
      { pin: 11, use: "fixed", what: "WS2812 power enable" },
      { pin: 17, use: "fixed", what: "user LED (red)" },
      { pin: 16, use: "fixed", what: "user LED (green)" },
      { pin: 25, use: "fixed", what: "user LED (blue)" },
    ],
    roles: [
      { pin: 0, role: "DATA UART TX" },
      { pin: 1, role: "DATA UART RX" },
    ],
  },
];

/** Attach the mcu pin table to a def; undefined if the mcu isn't known here. */
function resolveBoard(def: BoardDef): Board | undefined {
  const mcuPins = MCUS[def.mcu];
  return mcuPins ? { ...def, mcuPins } : undefined;
}

/** Load all known boards: built-ins merged with local JSON files (which win by
 *  id). Local files come from <app_data>/boards/ via the backend. */
export async function loadBoards(): Promise<Board[]> {
  let local: BoardDef[] = [];
  try {
    local = (await listBoards()) as BoardDef[];
  } catch {
    /* not running under Tauri / no folder — built-ins only */
  }
  const byId = new Map<string, BoardDef>();
  for (const d of BUILTIN_DEFS) byId.set(d.id.toLowerCase(), d);
  for (const d of local) if (d?.id) byId.set(d.id.toLowerCase(), d); // local overrides built-in
  return [...byId.values()].map(resolveBoard).filter((b): b is Board => !!b);
}

/** Find a loaded board by the device's self-described name (case-insensitive). */
export function findBoard(boards: Board[], name: string | undefined | null): Board | undefined {
  const n = (name ?? "").trim().toLowerCase();
  if (!n) return undefined;
  return boards.find((b) => b.id.toLowerCase() === n || b.name.toLowerCase() === n);
}

/** A board pin resolved for display: silicon caps + the board/target overlay. */
export interface BoardPinRow {
  pin: number;
  caps: number;
  status: PinStatus;
  note?: string;
  brokenOut: boolean;
  use?: PinUse; // onboard commitment
  role?: string; // our wiring
  i2c?: "sda" | "scl"; // part of the active I²C bridge default
}

/** Compose a board's full pin map (sorted by GPIO) from its layers. */
export function boardPinRows(board: Board): BoardPinRow[] {
  const useByPin = new Map(board.uses.map((u) => [u.pin, u]));
  const roleByPin = new Map(board.roles.map((r) => [r.pin, r.role]));
  const broken = new Set(board.brokenOut);
  return board.mcuPins
    .map((p) => ({
      pin: p.pin,
      caps: p.caps,
      status: p.status,
      note: p.note,
      brokenOut: board.brokenOutAll === true || broken.has(p.pin),
      use: useByPin.get(p.pin),
      role: roleByPin.get(p.pin),
      i2c:
        board.i2c?.sda === p.pin ? ("sda" as const) : board.i2c?.scl === p.pin ? ("scl" as const) : undefined,
    }))
    .sort((a, b) => a.pin - b.pin);
}

/** Human labels for the PINCAP capability bits (in display order). */
export const CAP_LABELS: { bit: number; label: string }[] = [
  { bit: PINCAP.DIGITAL, label: "DIO" },
  { bit: PINCAP.ADC, label: "ADC" },
  { bit: PINCAP.PWM, label: "PWM" },
  { bit: PINCAP.DAC, label: "DAC" },
  { bit: PINCAP.I2C, label: "I²C" },
  { bit: PINCAP.SPI, label: "SPI" },
  { bit: PINCAP.TOUCH, label: "Touch" },
];
