// VS Code-style color decorations for a macro: every #RRGGBB token in the text
// gets a clickable swatch chip. Clicking opens a color picker + brightness; the
// edit rewrites that exact occurrence in place. Replacements are the same length
// (#RRGGBB is always 7 chars), so other tokens' offsets stay valid.
import { useMemo } from "react";

import { ColorField, ledSwatchStyle } from "@/components/ColorField";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { type Rgb, hexToRgb, rgbToHex } from "@/lib/skrit";

const COLOR_RE = /#[0-9a-fA-F]{6}(?![0-9a-fA-F])/g;

export function MacroColorStrip({
  text,
  onChange,
}: {
  text: string;
  onChange: (next: string) => void;
}) {
  const matches = useMemo(() => {
    const out: { start: number; end: number; hex: string }[] = [];
    COLOR_RE.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = COLOR_RE.exec(text))) {
      out.push({ start: m.index, end: m.index + m[0].length, hex: m[0] });
    }
    return out;
  }, [text]);

  if (matches.length === 0) return null;

  function replace(start: number, end: number, rgb: Rgb) {
    onChange(text.slice(0, start) + rgbToHex(rgb) + text.slice(end));
  }

  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <span className="text-[10px] uppercase tracking-wide text-muted-foreground">colors</span>
      {matches.map((c, i) => (
        <Popover key={`${c.start}-${i}`}>
          <PopoverTrigger asChild>
            <button
              type="button"
              className="flex items-center gap-1 rounded border px-1 py-0.5 font-mono text-[10px] hover:bg-accent"
              title="Edit color"
            >
              <span
                className="size-3 rounded-sm border"
                style={ledSwatchStyle(hexToRgb(c.hex) ?? { r: 0, g: 0, b: 0 })}
              />
              {c.hex}
            </button>
          </PopoverTrigger>
          <PopoverContent className="w-64" align="start">
            <ColorField
              value={hexToRgb(c.hex) ?? { r: 0, g: 0, b: 0 }}
              onChange={(rgb) => replace(c.start, c.end, rgb)}
            />
          </PopoverContent>
        </Popover>
      ))}
    </div>
  );
}
