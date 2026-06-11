// Configure-device screen: runtime IO provisioning. Reads the device's pin menu
// (PIN_CAPS, mcu ∩ board) and current table (CONFIG_GET), lets the user assign a
// role + name to each offerable pin, writes it back (CONFIG_SET), and reboots to
// apply. The pin picker is constrained to what each pin supports, no hardcoded
// chip knowledge. See PROTOCOL.md "Provisioning".
import { AlertTriangle, Plus, RotateCcw, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  CTRL, DUTA_ACTIVE_LOW, type IoRow, type PinCap, PINCAP,
  dataPins, getIoConfig, pinCaps, reboot, resetIoConfig, setIoConfig,
} from "@/lib/skrit";

const ROLE_LABEL: Record<number, string> = { [CTRL.IO]: "IO", [CTRL.PWM]: "PWM", [CTRL.RGB]: "RGB" };

// Which roles a pin's capability bits allow.
function rolesFor(caps: number): number[] {
  const r: number[] = [];
  if (caps & PINCAP.DIGITAL) r.push(CTRL.IO);
  if (caps & PINCAP.PWM) r.push(CTRL.PWM);
  return r;
}

export function ConfigureDevice({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [pins, setPins] = useState<PinCap[]>([]);
  const [rows, setRows] = useState<IoRow[]>([]);
  const [data, setData] = useState<{ tx: number; rx: number } | null>(null); // DATA UART pins (fixed)
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [badRow, setBadRow] = useState<number | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (!open) return;
    setSaved(false);
    setBadRow(null);
    setNote(null);
    setLoading(true);
    Promise.all([pinCaps(), getIoConfig()])
      .then(([p, r]) => {
        setPins(p);
        setRows(r);
      })
      .catch((e) => setNote(String(e)))
      .finally(() => setLoading(false));
    dataPins().then(setData).catch(() => setData(null)); // older firmware: no key
  }, [open]);

  const capOf = (pin: number) => pins.find((p) => p.pin === pin);
  const usedPins = (exceptIdx: number) =>
    new Set(rows.filter((_, i) => i !== exceptIdx).map((r) => r.pin));

  function update(i: number, patch: Partial<IoRow>) {
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
    setSaved(false);
    setBadRow(null);
  }
  function addRow() {
    // first free, offerable pin that isn't already taken
    const taken = new Set(rows.map((r) => r.pin));
    const free = pins.find((p) => !taken.has(p.pin) && rolesFor(p.caps).length);
    if (!free) return;
    setRows((rs) => [...rs, { type: rolesFor(free.caps)[0], pin: free.pin, flags: 0, arg: 0, name: "" }]);
    setSaved(false);
  }
  const removeRow = (i: number) => {
    setRows((rs) => rs.filter((_, j) => j !== i));
    setSaved(false);
    setBadRow(null);
  };

  async function save() {
    setBusy(true);
    setNote(null);
    try {
      const bad = await setIoConfig(rows);
      if (bad === null) {
        setSaved(true);
        setBadRow(null);
      } else {
        setBadRow(bad);
        setNote(`Pin not allowed for that role in row ${bad + 1}.`);
      }
    } catch (e) {
      setNote(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function doReset() {
    setBusy(true);
    setNote(null);
    try {
      await resetIoConfig();
      setSaved(true);
    } catch (e) {
      setNote(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function doReboot() {
    setBusy(true);
    try {
      await reboot();
    } catch {
      /* the link drops as the device resets, expected */
    }
    setBusy(false);
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Configure device IO</DialogTitle>
        </DialogHeader>

        {loading ? (
          <p className="py-6 text-center text-sm text-muted-foreground">Reading pin map…</p>
        ) : (
          <div className="flex flex-col gap-2">
            <p className="text-xs text-muted-foreground">
              Assign a role and name to each pin. Only pins this board breaks out are listed;
              ⚠ marks a strapping pin or one that shares onboard hardware. Changes apply after a reboot.
            </p>

            {data && (
              <div className="flex items-center gap-2 rounded-md border border-dashed px-2 py-1.5 text-xs text-muted-foreground">
                <span className="font-medium text-foreground">DATA UART</span>
                <span className="font-mono">
                  TX {data.tx >= 0 ? `GPIO${data.tx}` : "—"} · RX {data.rx >= 0 ? `GPIO${data.rx}` : "—"}
                </span>
                <span className="ml-auto">fixed — the bridged console</span>
              </div>
            )}

            <div className="flex flex-col gap-2">
              {rows.map((row, i) => {
                const cap = capOf(row.pin);
                const used = usedPins(i);
                const roleOpts = cap ? rolesFor(cap.caps) : [];
                // off-menu (fixed) pins and RGB rows keep their current role only
                if (!roleOpts.includes(row.type)) roleOpts.push(row.type);
                const roleLocked = row.type === CTRL.RGB || !cap;
                const isBad = badRow === i;
                return (
                  <div
                    key={i}
                    className={`flex items-center gap-2 rounded border p-2 ${isBad ? "border-destructive" : ""}`}
                  >
                    {/* role */}
                    <Select
                      value={String(row.type)}
                      onValueChange={(v) => update(i, { type: Number(v) })}
                      disabled={roleLocked}
                    >
                      <SelectTrigger className="h-8 w-24 shrink-0">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {roleOpts.map((t) => (
                          <SelectItem key={t} value={String(t)}>
                            {ROLE_LABEL[t]}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>

                    {/* pin: a pin outside the menu (e.g. a fixed onboard LED) can
                        be kept in its compiled role but not moved or repurposed */}
                    <Select value={String(row.pin)} onValueChange={(v) => update(i, { pin: Number(v) })}>
                      <SelectTrigger className="h-8 w-32 shrink-0">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {!cap && (
                          <SelectItem value={String(row.pin)} disabled>
                            GPIO{row.pin} (fixed)
                          </SelectItem>
                        )}
                        {pins
                          .filter((p) => p.pin === row.pin || (!used.has(p.pin) && rolesFor(p.caps).includes(row.type)))
                          .map((p) => (
                            <SelectItem key={p.pin} value={String(p.pin)}>
                              GPIO{p.pin}
                              {p.warn ? " ⚠" : ""}
                            </SelectItem>
                          ))}
                      </SelectContent>
                    </Select>

                    {/* name */}
                    <Input
                      className="h-8 flex-1"
                      placeholder="name (e.g. Power relay)"
                      value={row.name}
                      maxLength={31}
                      onChange={(e) => update(i, { name: e.target.value })}
                    />

                    {/* active-low */}
                    <label className="flex shrink-0 items-center gap-1 text-[10px] text-muted-foreground">
                      <Switch
                        checked={(row.flags & DUTA_ACTIVE_LOW) !== 0}
                        onCheckedChange={(on) =>
                          update(i, { flags: on ? row.flags | DUTA_ACTIVE_LOW : row.flags & ~DUTA_ACTIVE_LOW })
                        }
                      />
                      low
                    </label>

                    <Button size="icon" variant="ghost" className="h-8 w-8 shrink-0" onClick={() => removeRow(i)}>
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                );
              })}
            </div>

            {/* per-pin caution */}
            {rows.some((r) => capOf(r.pin)?.warn) && (
              <div className="flex items-start gap-1.5 text-[11px] text-amber-600 dark:text-amber-500">
                <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" />
                <span>
                  {rows
                    .map((r) => capOf(r.pin))
                    .filter((c): c is PinCap => !!c?.warn)
                    .map((c) => `GPIO${c.pin}: ${c.note || "use with caution"}`)
                    .join(" · ")}
                </span>
              </div>
            )}

            <Button size="sm" variant="outline" className="w-fit" onClick={addRow}>
              <Plus className="mr-1 h-3.5 w-3.5" /> Add output
            </Button>

            {note && <p className="text-xs text-destructive">{note}</p>}
            {saved && (
              <p className="text-xs text-emerald-600 dark:text-emerald-500">
                Saved. Reboot the device to apply.
              </p>
            )}
          </div>
        )}

        <DialogFooter className="flex-row justify-between sm:justify-between">
          <Button size="sm" variant="ghost" onClick={doReset} disabled={busy || loading}>
            <RotateCcw className="mr-1 h-3.5 w-3.5" /> Reset to default
          </Button>
          <div className="flex gap-2">
            {saved ? (
              <Button size="sm" onClick={doReboot} disabled={busy}>
                Reboot to apply
              </Button>
            ) : (
              <Button size="sm" onClick={save} disabled={busy || loading}>
                Save
              </Button>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
