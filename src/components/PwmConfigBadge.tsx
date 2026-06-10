// A small badge showing a PWM output's frequency + resolution; click to edit.
// The device always reports its actuals, so a fixed-PWM board still shows its
// defaults here even though Apply is a no-op for it.
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { type PwmConfig } from "@/lib/skrit";

export function PwmConfigBadge({
  cfg,
  disabled,
  onSet,
}: {
  cfg: PwmConfig;
  disabled?: boolean;
  onSet: (freq: number, res: number) => void;
}) {
  const [freq, setFreq] = useState(String(cfg.freq));
  const [res, setRes] = useState(String(cfg.res));
  return (
    <Popover
      onOpenChange={(open) => {
        if (open) {
          setFreq(String(cfg.freq));
          setRes(String(cfg.res));
        }
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          title="PWM frequency / resolution"
          className="rounded border px-1.5 py-0.5 text-[10px] text-muted-foreground hover:bg-accent disabled:opacity-50"
        >
          {cfg.freq ? `${cfg.freq} Hz · ${cfg.res}-bit` : "PWM"}
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-56" align="end">
        <div className="flex flex-col gap-2">
          <label className="flex items-center justify-between gap-2 text-xs">
            Frequency (Hz)
            <Input
              className="h-7 w-24"
              type="number"
              value={freq}
              onChange={(e) => setFreq(e.target.value)}
            />
          </label>
          <label className="flex items-center justify-between gap-2 text-xs">
            Resolution (bits)
            <Input
              className="h-7 w-24"
              type="number"
              min={1}
              max={16}
              value={res}
              onChange={(e) => setRes(e.target.value)}
            />
          </label>
          <Button size="sm" onClick={() => onSet(Number(freq) || 0, Number(res) || 0)}>
            Apply
          </Button>
          <p className="text-[10px] text-muted-foreground">
            A device that can't change a value reports its default.
          </p>
        </div>
      </PopoverContent>
    </Popover>
  );
}
