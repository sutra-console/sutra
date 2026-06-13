// Per-peer command surface: the discovered nodes of a network, typed ("it's a
// light") from their clusters, with buttons that inject the ZCL command. Pairs
// with src-tauri/src/zigbee.rs build_zcl_inject via the {$zcl} macro var.
import type { AttrObs, Network } from "@/lib/skrit";
import { clusterCommands, clusterName, deviceType, parseCluster } from "@/lib/zcl";

export function NetworkDevices({
  net,
  disabled,
  onCommand,
  attrs,
}: {
  net: Network;
  /** true when not connected — buttons can't inject. */
  disabled?: boolean;
  onCommand: (addr: string, endpoint: number, cluster: number, cmd: number, payloadHex?: string) => void;
  /** live ZCL attribute values, keyed addr|ep|cluster|attr. */
  attrs?: Record<string, AttrObs>;
}) {
  // attribute values seen for a given node/endpoint/cluster (passive reads)
  const attrsFor = (addr: string, ep: number, cluster: string): AttrObs[] =>
    attrs ? Object.values(attrs).filter((a) => a.addr === addr && a.endpoint === ep && a.cluster === cluster) : [];
  const nodes = net.nodes.filter((n) => n.endpoints?.length);
  if (!nodes.length) {
    return (
      <p className="text-[11px] text-muted-foreground">
        No clusters discovered yet — sniff this network with the key set (ch pinned); each node fills
        in its endpoints/clusters as it talks.
      </p>
    );
  }
  return (
    <div className="flex flex-col gap-1.5">
      {nodes.map((n) => {
        const clusters = n.endpoints.flatMap((e) => e.clusters.map(parseCluster));
        return (
          <div key={n.addr} className="rounded border p-2 text-xs">
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-mono">{n.addr}</span>
              <span className="rounded bg-accent px-1.5 py-0.5 text-[10px] font-medium">{deviceType(clusters)}</span>
              {n.manufacturer && <span className="text-[10px] text-muted-foreground">mfr {n.manufacturer}</span>}
            </div>
            {n.endpoints.map((ep) => (
              <div key={ep.id} className="mt-1 flex flex-wrap items-center gap-1">
                <span className="w-8 shrink-0 text-[10px] text-muted-foreground">ep{ep.id}</span>
                {ep.clusters.map((cs) => {
                  const cid = parseCluster(cs);
                  const cmds = clusterCommands(cid);
                  const vals = attrsFor(n.addr, ep.id, cs);
                  return (
                    <span key={cs} className="flex items-center gap-0.5 rounded border bg-muted/30 px-1 py-0.5">
                      <span className="text-[10px] text-muted-foreground" title={cs}>{clusterName(cid)}</span>
                      {cmds.map((c, idx) => (
                        <button
                          key={idx}
                          type="button"
                          disabled={disabled}
                          title={disabled ? "connect to a Duta on this channel to send" : `send ${c.label}`}
                          onClick={() => onCommand(n.addr, ep.id, cid, c.cmd, c.payloadHex)}
                          className="rounded bg-primary/15 px-1 text-[10px] text-primary hover:bg-primary/25 disabled:opacity-40"
                        >
                          {c.label}
                        </button>
                      ))}
                      {vals.map((a) => (
                        <span key={a.attr} className="rounded bg-emerald-500/15 px-1 text-[10px] text-emerald-500"
                          title={`attribute ${a.attr} (read off the wire)`}>
                          {a.attr}={a.value}
                        </span>
                      ))}
                    </span>
                  );
                })}
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );
}
