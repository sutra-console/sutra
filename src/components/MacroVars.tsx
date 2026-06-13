// Macro variable palette: click to insert a {$…} token at the cursor, and show
// what the active network resolves them against. Pairs with the {$name}
// substitution engine (src-tauri/src/macrovars.rs).
import type { Network } from "@/lib/skrit";

type Chip = { token: string; label: string; desc: string };

const CONTEXT: Chip[] = [
  { token: "{$key}", label: "{$key}", desc: "network key" },
  { token: "{$pan}", label: "{$pan}", desc: "PAN id" },
  { token: "{$channel}", label: "{$channel}", desc: "channel" },
  { token: "{$src}", label: "{$src}", desc: "our short addr" },
  { token: "{$eui}", label: "{$eui}", desc: "our EUI-64" },
];
const COUNTERS: Chip[] = [
  { token: "{$fc}", label: "{$fc}", desc: "frame counter (auto-increments)" },
  { token: "{$seq}", label: "{$seq}", desc: "sequence byte" },
];
const ZIGBEE: Chip[] = [
  { token: "HEX {$zdp active_ep 0000}", label: "active_ep", desc: "interview: a node's endpoints" },
  { token: "HEX {$zdp node_desc 0000}", label: "node_desc", desc: "interview: manufacturer" },
  { token: "HEX {$zdp simple_desc 0000 1}", label: "simple_desc", desc: "interview: clusters on an endpoint" },
];

function Row({ title, chips, onInsert }: { title: string; chips: Chip[]; onInsert: (t: string) => void }) {
  return (
    <div className="flex flex-wrap items-center gap-1">
      <span className="w-14 shrink-0 text-[10px] uppercase tracking-wide text-muted-foreground">{title}</span>
      {chips.map((c) => (
        <button
          key={c.token}
          type="button"
          title={`${c.desc} — insert "${c.token}"`}
          onClick={() => onInsert(c.token)}
          className="rounded border bg-muted/40 px-1.5 py-0.5 font-mono text-[10px] text-foreground hover:bg-muted"
        >
          {c.label}
        </button>
      ))}
    </div>
  );
}

export function MacroVars({ onInsert, active }: { onInsert: (token: string) => void; active?: Network }) {
  return (
    <div className="flex flex-col gap-1.5 rounded-md border bg-muted/20 p-2">
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">Variables</span>
        {active ? (
          <span className="truncate text-[10px] text-muted-foreground">
            resolves against <span className="font-medium text-foreground">{active.label || "network"}</span> · PAN{" "}
            {active.pan || "?"} · ch {active.channel || "?"} · key {active.key ? "set" : "—"}
          </span>
        ) : (
          <span className="text-[10px] text-amber-500">no active network — set one in Networks</span>
        )}
      </div>
      <Row title="Context" chips={CONTEXT} onInsert={onInsert} />
      <Row title="Counters" chips={COUNTERS} onInsert={onInsert} />
      <Row title="Zigbee" chips={ZIGBEE} onInsert={onInsert} />
    </div>
  );
}
