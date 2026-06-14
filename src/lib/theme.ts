// App-wide theming: light/dark/system mode + an accent preset. The base neutral
// (zinc) tokens live in index.css (:root / .dark); an accent is a `[data-accent]`
// overlay that re-points --primary/--ring. Everything shadcn-tokened re-themes for
// free. Persisted in the app Settings (localStorage). The egui yantra surface reads
// the *resolved* colors via resolveTokens() so it matches.

export type Mode = "light" | "dark" | "system";

export interface Accent {
  id: string;
  label: string;
  swatch: string; // a CSS color for the picker dot (the dark-mode primary)
}

/** Accent presets. `zinc` = the stock shadcn neutral (no --primary override). */
export const ACCENTS: Accent[] = [
  { id: "zinc", label: "Zinc", swatch: "oklch(0.92 0.004 286.32)" },
  { id: "blue", label: "Blue", swatch: "oklch(0.62 0.19 259)" },
  { id: "green", label: "Green", swatch: "oklch(0.7 0.17 162)" },
  { id: "violet", label: "Violet", swatch: "oklch(0.61 0.22 293)" },
  { id: "rose", label: "Rose", swatch: "oklch(0.64 0.23 16)" },
  { id: "amber", label: "Amber", swatch: "oklch(0.77 0.16 70)" },
];

export interface Theme {
  mode: Mode;
  accent: string; // an ACCENTS id
}

export const DEFAULT_THEME: Theme = { mode: "dark", accent: "zinc" };

const prefersDark = () =>
  typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;

/** Resolve a mode to an actual dark/light boolean. */
export function isDark(mode: Mode): boolean {
  return mode === "dark" || (mode === "system" && prefersDark());
}

/** Apply a theme: toggle `.dark`, set the accent overlay + color-scheme on <html>. */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  const dark = isDark(theme.mode);
  root.classList.toggle("dark", dark);
  root.dataset.accent = theme.accent;
  root.style.colorScheme = dark ? "dark" : "light";
}

/** Re-apply on OS scheme change while in `system` mode. Returns an unsubscribe. */
export function watchSystemTheme(get: () => Theme): () => void {
  if (typeof matchMedia !== "function") return () => {};
  const mq = matchMedia("(prefers-color-scheme: dark)");
  const on = () => {
    if (get().mode === "system") applyTheme(get());
  };
  mq.addEventListener("change", on);
  return () => mq.removeEventListener("change", on);
}

/** The resolved (rgb) token colors, for syncing the egui surface. The browser
 *  resolves the OKLCH `var(--…)` for us via getComputedStyle on a temp element. */
export type ThemeTokens = Record<string, string>; // name → "rgb(r, g, b)"
const TOKEN_KEYS = [
  "background", "foreground", "card", "card-foreground",
  "primary", "primary-foreground", "border", "muted-foreground", "accent",
] as const;

export function resolveTokens(): ThemeTokens {
  const probe = document.createElement("span");
  probe.style.position = "absolute";
  probe.style.visibility = "hidden";
  document.body.appendChild(probe);
  const out: ThemeTokens = {};
  try {
    for (const k of TOKEN_KEYS) {
      probe.style.color = `var(--${k})`;
      out[k] = getComputedStyle(probe).color; // "rgb(r, g, b)" / "rgba(...)"
    }
  } finally {
    probe.remove();
  }
  return out;
}
