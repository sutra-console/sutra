// A color editor: react-colorful's RGB picker (a saturation/brightness plane +
// hue bar) plus a hex field for precise entry. Emits a concrete {r,g,b}.
import { RgbColorPicker } from "react-colorful";

import { Input } from "@/components/ui/input";
import { type Rgb, hexToRgb, rgbToHex } from "@/lib/skrit";

export function ColorField({
  value,
  onChange,
}: {
  value: Rgb;
  onChange: (rgb: Rgb) => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <RgbColorPicker color={value} onChange={onChange} />
      <Input
        value={rgbToHex(value)}
        spellCheck={false}
        onChange={(e) => {
          const rgb = hexToRgb(e.target.value);
          if (rgb) onChange(rgb);
        }}
        className="h-7 font-mono text-xs uppercase"
      />
    </div>
  );
}
