// Controls panel widget for an rgb (addressable-LED) output. A single-pixel
// output shows one swatch; a strip shows a swatch per pixel plus a "fill all".
// Each swatch opens the color picker.
import { ColorField } from "@/components/ColorField";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { type Rgb, rgbToHex } from "@/lib/skrit";

function Swatch({
  color,
  label,
  disabled,
  onChange,
}: {
  color: Rgb;
  label?: string;
  disabled?: boolean;
  onChange: (rgb: Rgb) => void;
}) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          title={label ? `Pixel ${label}` : "Color"}
          className="flex items-center gap-1 rounded border px-1.5 py-1 hover:bg-accent disabled:opacity-50"
        >
          <span className="size-4 rounded-sm border" style={{ backgroundColor: rgbToHex(color) }} />
          {label !== undefined && <span className="text-[10px] text-muted-foreground">{label}</span>}
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-64" align="start">
        <ColorField value={color} onChange={onChange} />
      </PopoverContent>
    </Popover>
  );
}

export function RgbControl({
  pixels,
  disabled,
  onChange,
}: {
  pixels: Rgb[];
  disabled?: boolean;
  /** pixel = undefined fills the whole strip; a number sets that one pixel. */
  onChange: (pixel: number | undefined, rgb: Rgb) => void;
}) {
  const black = { r: 0, g: 0, b: 0 };
  if (pixels.length <= 1) {
    return <Swatch color={pixels[0] ?? black} disabled={disabled} onChange={(rgb) => onChange(undefined, rgb)} />;
  }
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex flex-wrap gap-1">
        {pixels.map((px, i) => (
          <Swatch key={i} color={px} label={String(i)} disabled={disabled} onChange={(rgb) => onChange(i, rgb)} />
        ))}
      </div>
      <Swatch color={pixels[0] ?? black} label="all" disabled={disabled} onChange={(rgb) => onChange(undefined, rgb)} />
    </div>
  );
}
