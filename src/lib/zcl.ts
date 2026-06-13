// ZCL cluster/command dictionary + device typing — the "what can I do with this
// peer" knowledge. Cluster ids come off the node model (passive discovery); we
// map them to human names, a device type ("it's a light"), and the commands the
// UI offers (each issued as HEX {$zcl <addr> <ep> <cluster> <cmd>}).

export interface ZclCommand {
  cmd: number; // ZCL command id
  label: string;
  payloadHex?: string; // optional command payload (space-separated hex)
}
export interface ClusterDef {
  name: string;
  commands?: ZclCommand[];
}

// A pragmatic subset of the HA clusters — names for everything we commonly see,
// commands for the ones worth a button.
export const CLUSTERS: Record<number, ClusterDef> = {
  0x0000: { name: "Basic" },
  0x0001: { name: "Power Config" },
  0x0003: { name: "Identify", commands: [{ cmd: 0x00, label: "Identify", payloadHex: "0a 00" }] },
  0x0004: { name: "Groups" },
  0x0005: { name: "Scenes" },
  0x0006: {
    name: "On/Off",
    commands: [
      { cmd: 0x00, label: "Off" },
      { cmd: 0x01, label: "On" },
      { cmd: 0x02, label: "Toggle" },
    ],
  },
  0x0008: {
    name: "Level",
    commands: [
      // Move to level (with On/Off): level, transition time (0.1s) LE
      { cmd: 0x04, label: "25%", payloadHex: "40 0a 00" },
      { cmd: 0x04, label: "50%", payloadHex: "80 0a 00" },
      { cmd: 0x04, label: "100%", payloadHex: "fe 0a 00" },
    ],
  },
  0x000a: { name: "Time" },
  0x0019: { name: "OTA Upgrade" },
  0x0101: { name: "Door Lock", commands: [{ cmd: 0x00, label: "Lock" }, { cmd: 0x01, label: "Unlock" }] },
  0x0102: { name: "Window Covering", commands: [{ cmd: 0x00, label: "Up" }, { cmd: 0x01, label: "Down" }, { cmd: 0x02, label: "Stop" }] },
  0x0201: { name: "Thermostat" },
  0x0300: { name: "Color" },
  0x0400: { name: "Illuminance" },
  0x0402: { name: "Temperature" },
  0x0405: { name: "Humidity" },
  0x0406: { name: "Occupancy" },
  0x0500: { name: "IAS Zone (security)" },
  0x0b04: { name: "Electrical" },
  0x0702: { name: "Metering" },
};

export const clusterName = (id: number): string =>
  CLUSTERS[id]?.name ?? `0x${id.toString(16).padStart(4, "0")}`;

export const clusterCommands = (id: number): ZclCommand[] => CLUSTERS[id]?.commands ?? [];

/** Parse a "0x0006" cluster string to a number (NetNode stores them as strings). */
export const parseCluster = (s: string): number => parseInt(s.replace(/^0x/i, ""), 16);

/** Infer a device type from the clusters a node exposes — "it's a light", etc. */
export function deviceType(clusterIds: number[]): string {
  const has = (c: number) => clusterIds.includes(c);
  if (has(0x0101)) return "Door Lock";
  if (has(0x0102)) return "Window Covering";
  if (has(0x0201)) return "Thermostat";
  if (has(0x0500)) return "Security Sensor";
  if (has(0x0006) && (has(0x0008) || has(0x0300))) return has(0x0300) ? "Color Light" : "Dimmable Light";
  if (has(0x0006)) return "Light / Switch";
  if (has(0x0402) || has(0x0405) || has(0x0400) || has(0x0406)) return "Sensor";
  if (has(0x0b04) || has(0x0702)) return "Energy Meter";
  return "Device";
}
