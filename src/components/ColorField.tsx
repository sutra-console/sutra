// A color editor tuned for LEDs: a rainbow hue bar plus an equally-tall
// brightness bar, and a hex field for precise entry. The sample swatch shows
// hue only — on-screen brightness isn't a faithful preview of the LED. Internally
// it tracks HSV so hue and brightness move independently; it emits a concrete
// {r,g,b}.
import { type CSSProperties, useEffect, useRef, useState } from "react";

import { Input } from "@/components/ui/input";
import { type Rgb, hexToRgb, rgbToHex } from "@/lib/skrit";

interface Hsv {
  h: number; // 0..360
  s: number; // 0..100
  v: number; // 0..100
}

function rgbToHsv({ r, g, b }: Rgb): Hsv {
  const rn = r / 255;
  const gn = g / 255;
  const bn = b / 255;
  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  const d = max - min;
  let h = 0;
  if (d !== 0) {
    if (max === rn) h = ((gn - bn) / d) % 6;
    else if (max === gn) h = (bn - rn) / d + 2;
    else h = (rn - gn) / d + 4;
    h *= 60;
    if (h < 0) h += 360;
  }
  const s = max === 0 ? 0 : d / max;
  return { h: Math.round(h), s: Math.round(s * 100), v: Math.round(max * 100) };
}

function hsvToRgb({ h, s, v }: Hsv): Rgb {
  const sn = s / 100;
  const vn = v / 100;
  const c = vn * sn;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = vn - c;
  let r = 0;
  let g = 0;
  let b = 0;
  if (h < 60) [r, g, b] = [c, x, 0];
  else if (h < 120) [r, g, b] = [x, c, 0];
  else if (h < 180) [r, g, b] = [0, c, x];
  else if (h < 240) [r, g, b] = [0, x, c];
  else if (h < 300) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  return {
    r: Math.round((r + m) * 255),
    g: Math.round((g + m) * 255),
    b: Math.round((b + m) * 255),
  };
}

// The hue bar reserves a white block at its left edge: positions below
// WHITE_FRAC select pure white (saturation 0); the rest maps to a full-
// saturation hue. The gradient remaps the rainbow stops into [WHITE_FRAC, 1].
const WHITE_FRAC = 0.14;
const RAINBOW: [number, string][] = [
  [0, "#f00"],
  [17, "#ff0"],
  [33, "#0f0"],
  [50, "#0ff"],
  [67, "#00f"],
  [83, "#f0f"],
  [100, "#f00"],
];
const W = WHITE_FRAC * 100;
// Style a mini swatch so it reads like a lit LED rather than a flat dimmed hex:
// the core shows the pure hue (or white) and casts a colored glow scaled by
// brightness; an off pixel (v=0) is dark with no glow. This is a better at-a-
// glance preview than rgbToHex, which just looks muddy at low brightness.
export function ledSwatchStyle(color: Rgb): CSSProperties {
  const { h, s, v } = rgbToHsv(color);
  if (v === 0) return { backgroundColor: "#111" };
  const frac = v / 100;
  const white = s === 0;
  const fill = white ? `hsl(0, 0%, ${20 + 80 * frac}%)` : `hsl(${h}, 100%, ${15 + 35 * frac}%)`;
  const glow = white ? `hsla(0, 0%, 100%, ${0.6 * frac})` : `hsla(${h}, 100%, 60%, ${0.7 * frac})`;
  return {
    backgroundColor: fill,
    boxShadow: `0 0 ${2 + 5 * frac}px ${0.5 + 1.5 * frac}px ${glow}`,
  };
}

const HUE_GRADIENT =
  `linear-gradient(to right, #fff 0%, #fff ${W}%, ` +
  RAINBOW.map(([s, c]) => `${c} ${W + (s * (100 - W)) / 100}%`).join(", ") +
  ")";

// A draggable horizontal bar with a round pointer. `pos` is 0..1; `onPos` reports
// the new fraction. Used for both the hue bar and the brightness bar so they
// share the same height and look.
function Bar({
  pos,
  background,
  pointerColor,
  onPos,
}: {
  pos: number;
  background: string;
  pointerColor: string;
  onPos: (pos: number) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);

  const update = (clientX: number) => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    onPos(Math.min(Math.max(clientX - rect.left, 0), rect.width) / rect.width);
  };

  return (
    <div
      ref={ref}
      className="relative h-6 cursor-pointer touch-none rounded border"
      style={{ background }}
      onPointerDown={(e) => {
        dragging.current = true;
        e.currentTarget.setPointerCapture(e.pointerId);
        update(e.clientX);
      }}
      onPointerMove={(e) => {
        if (dragging.current) update(e.clientX);
      }}
      onPointerUp={(e) => {
        dragging.current = false;
        e.currentTarget.releasePointerCapture(e.pointerId);
      }}
    >
      <div
        className="pointer-events-none absolute top-1/2 size-5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white shadow"
        style={{ left: `${pos * 100}%`, backgroundColor: pointerColor }}
      />
    </div>
  );
}

export function ColorField({
  value,
  onChange,
}: {
  value: Rgb;
  onChange: (rgb: Rgb) => void;
}) {
  // Track HSV internally so hue and brightness are independent. Re-sync when the
  // incoming color differs from what our current HSV represents (e.g. hex entry
  // or an external update), but otherwise keep our state to avoid round-trip drift.
  const [hsv, setHsv] = useState<Hsv>(() => rgbToHsv(value));
  const hex = rgbToHex(value);
  useEffect(() => {
    if (rgbToHex(hsvToRgb(hsv)) !== hex) setHsv(rgbToHsv(value));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hex]);

  const emit = (next: Hsv) => {
    setHsv(next);
    onChange(hsvToRgb(next));
  };

  // The sample shows hue (or white) at full brightness — screen brightness isn't
  // a faithful preview of the LED's actual brightness, so we don't dim it here.
  const isWhite = hsv.s === 0;
  const fullColor = isWhite ? "#fff" : `hsl(${hsv.h}, 100%, 50%)`;
  // Pointer at the middle of the white block when white, else mapped into the hue range.
  const huePos = isWhite ? WHITE_FRAC / 2 : WHITE_FRAC + (hsv.h / 360) * (1 - WHITE_FRAC);

  return (
    <div className="flex flex-col gap-3">
      <div className="h-6 rounded border" style={{ backgroundColor: fullColor }} title="Color" />
      <div className="flex flex-col gap-1">
        <span className="text-[10px] uppercase tracking-wide text-muted-foreground">hue</span>
        <Bar
          pos={huePos}
          background={HUE_GRADIENT}
          pointerColor={fullColor}
          onPos={(p) =>
            p < WHITE_FRAC
              ? emit({ ...hsv, s: 0 })
              : emit({
                  ...hsv,
                  h: Math.round(((p - WHITE_FRAC) / (1 - WHITE_FRAC)) * 360),
                  s: 100,
                })
          }
        />
      </div>
      <div className="flex flex-col gap-1">
        <span className="text-[10px] uppercase tracking-wide text-muted-foreground">brightness</span>
        <Bar
          pos={hsv.v / 100}
          background={`linear-gradient(to right, #000, ${fullColor})`}
          pointerColor={
            isWhite
              ? `hsl(0, 0%, ${100 * (hsv.v / 100)}%)`
              : `hsl(${hsv.h}, 100%, ${50 * (hsv.v / 100)}%)`
          }
          onPos={(p) => emit({ ...hsv, v: Math.round(p * 100) })}
        />
      </div>
      <Input
        value={hex}
        spellCheck={false}
        onChange={(e) => {
          const rgb = hexToRgb(e.target.value);
          if (rgb) emit(rgbToHsv(rgb));
        }}
        className="h-7 font-mono text-xs uppercase"
      />
    </div>
  );
}
