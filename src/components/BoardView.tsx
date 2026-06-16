// Board view: the connected device's full pinout, from the app-side board
// database (src/lib/boards.ts). Shows each GPIO's silicon capabilities (incl.
// I²C), hazard status, whether it's broken out, onboard/role commitments, and
// the active I²C bridge pins — overlaid with the device's current IO table.
import { useEffect, useMemo, useState } from "react";

import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import { CAP_LABELS, boardPinRows, findBoard, loadBoards, type Board } from "@/lib/boards";
import { PINCAP, getIoConfig, type IoRow } from "@/lib/skrit";

const STATUS_CLASS: Record<string, string> = {
  free: "text-emerald-500",
  caution: "text-amber-500",
  forbidden: "text-destructive",
};

export function BoardView({
  open,
  onOpenChange,
  deviceName,
  connected,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  deviceName: string;
  connected: boolean;
}) {
  const [boards, setBoards] = useState<Board[]>([]);
  const board = findBoard(boards, deviceName);
  const rows = useMemo(() => (board ? boardPinRows(board) : []), [board]);
  const [io, setIo] = useState<IoRow[]>([]);
  const [showAll, setShowAll] = useState(false);

  // Load the board database (built-ins + local JSON files) when the dialog opens.
  useEffect(() => {
    if (open) loadBoards().then(setBoards).catch(() => setBoards([]));
  }, [open]);

  // Overlay the device's live IO table so assigned pins show their current role.
  useEffect(() => {
    if (!open || !connected) {
      setIo([]);
      return;
    }
    getIoConfig().then(setIo).catch(() => setIo([]));
  }, [open, connected]);
  const ioByPin = new Map(io.map((r) => [r.pin, r]));

  // By default hide internal/unexposed pins (flash, PSRAM, USB); keep anything
  // that's broken out, wired to a role, or onboard hardware.
  const shown = showAll ? rows : rows.filter((r) => r.brokenOut || r.role || r.use);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="w-[92vw] sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{board ? board.name : "Board"}</DialogTitle>
        </DialogHeader>

        {!board ? (
          <p className="text-xs text-muted-foreground">
            No board definition for {deviceName ? <code>{deviceName}</code> : "this device"}.
            Add it to <code>src/lib/boards.ts</code>.
          </p>
        ) : (
          <div className="flex min-h-0 flex-col gap-3">
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
              <span>{board.vendor} · {board.model}</span>
              <span>MCU: <span className="text-foreground">{board.mcu}</span></span>
              {board.i2c && (
                <span>
                  I²C bridge: <span className="text-foreground">SDA GP{board.i2c.sda} · SCL GP{board.i2c.scl}</span>
                </span>
              )}
              <label className="ml-auto flex items-center gap-1.5">
                Show all pins
                <Switch checked={showAll} onCheckedChange={setShowAll} />
              </label>
            </div>

            <div className="max-h-[60vh] overflow-auto rounded border">
              <table className="w-full text-left font-mono text-xs">
                <thead className="sticky top-0 bg-background text-muted-foreground">
                  <tr>
                    <th className="px-2 py-1 font-normal">GPIO</th>
                    <th className="px-2 py-1 font-normal">Capabilities</th>
                    <th className="px-2 py-1 font-normal">Status</th>
                    <th className="px-2 py-1 font-normal">Exposed</th>
                    <th className="px-2 py-1 font-normal">Assignment</th>
                  </tr>
                </thead>
                <tbody>
                  {shown.map((r) => {
                    const liveIo = ioByPin.get(r.pin);
                    return (
                      <tr key={r.pin} className="border-t hover:bg-accent/40">
                        <td className="px-2 py-0.5 tabular-nums">
                          GP{r.pin}
                          {r.i2c && (
                            <span className="ml-1 rounded bg-sky-500/20 px-1 text-[9px] uppercase text-sky-500">
                              {r.i2c}
                            </span>
                          )}
                        </td>
                        <td className="px-2 py-0.5">
                          <span className="flex flex-wrap gap-1">
                            {CAP_LABELS.filter((c) => r.caps & c.bit).map((c) => (
                              <span
                                key={c.label}
                                className={
                                  c.bit === PINCAP.I2C
                                    ? "rounded bg-sky-500/15 px-1 text-[10px] text-sky-500"
                                    : "rounded bg-muted px-1 text-[10px] text-muted-foreground"
                                }
                              >
                                {c.label}
                              </span>
                            ))}
                          </span>
                        </td>
                        <td className="px-2 py-0.5">
                          <span className={STATUS_CLASS[r.status]} title={r.note}>
                            {r.status}
                          </span>
                          {r.note && <span className="ml-1 text-muted-foreground">· {r.note}</span>}
                        </td>
                        <td className="px-2 py-0.5 text-muted-foreground">
                          {r.brokenOut ? "header" : "internal"}
                        </td>
                        <td className="px-2 py-0.5">
                          {liveIo ? (
                            <span className="text-emerald-500" title="current device IO table">● {liveIo.name}</span>
                          ) : r.role ? (
                            <span className="text-primary">{r.role}</span>
                          ) : r.use ? (
                            <span className="text-muted-foreground">
                              {r.use.what}
                              {r.use.use === "dual" && <span className="text-amber-500"> (dual)</span>}
                            </span>
                          ) : (
                            <span className="text-muted-foreground/40">—</span>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>

            <p className="text-[10px] leading-tight text-muted-foreground">
              On the {board.mcu} the I²C controller is matrix-routed, so any free pin can be SDA/SCL;
              the bridge defaults to the pins above. <span className="text-primary">Blue</span> = our
              wiring, <span className="text-emerald-500">green ●</span> = live device IO,
              <span className="text-amber-500"> amber</span> = caution/dual-use.
            </p>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
