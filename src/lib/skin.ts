/** Skin data can name only existing color tokens and local font families. Never executable CSS. */
export const COLOR_TOKENS = [
  "bg", "bg-2", "ink", "ink-1", "ink-2", "ink-3", "ink-4", "ink-control", "ink-quiet",
  "hairline", "hairline-2", "wash", "surface", "scrim", "scrim-modal", "veil", "focus",
  "accent", "accent-line", "accent-ink", "danger", "mat-line", "mat-line-hover", "mat-line-drag",
] as const;
export const FONT_TOKENS = ["sans", "serif", "mono"] as const;
export type Palette = Record<typeof COLOR_TOKENS[number], string>;
export type SkinFonts = Record<typeof FONT_TOKENS[number], string[]>;
export interface Skin {
  version: 1;
  name: string;
  colors: { light: Palette; dark: Palette };
  fonts: SkinFonts;
}
export type Scheme = "light" | "dark";
export const SKIN_STORAGE_KEY = "cross-converter.skin.v1";
export const MAX_SKIN_LENGTH = 16_384;

export const DEFAULT_SKIN: Skin = {
  version: 1,
  name: "Original",
  colors: {
    light: {
      "bg": "#f7f5fb",
      "bg-2": "#edf6ff",
      "ink": "#17161a",
      "ink-1": "rgba(23, 22, 26, 0.9)",
      "ink-2": "rgba(23, 22, 26, 0.58)",
      "ink-3": "rgba(23, 22, 26, 0.34)",
      "ink-4": "rgba(23, 22, 26, 0.2)",
      "ink-control": "rgba(23, 22, 26, 0.68)",
      "ink-quiet": "rgba(23, 22, 26, 0.54)",
      "hairline": "rgba(43, 61, 93, 0.10)",
      "hairline-2": "rgba(43, 61, 93, 0.22)",
      "wash": "rgba(108, 156, 222, 0.08)",
      "surface": "rgba(255, 255, 255, 0.88)",
      "scrim": "rgba(50, 65, 93, 0.12)",
      "scrim-modal": "rgba(50, 65, 93, 0.24)",
      "veil": "rgba(239, 245, 255, 0.82)",
      "focus": "rgba(58, 91, 166, 0.8)",
      "accent": "#456bb2",
      "accent-line": "rgba(69, 107, 178, 0.3)",
      "accent-ink": "#ffffff",
      "danger": "#a13d64",
      "mat-line": "rgba(43, 61, 93, 0.18)",
      "mat-line-hover": "rgba(43, 61, 93, 0.3)",
      "mat-line-drag": "rgba(69, 107, 178, 0.65)",
    },
    dark: {
      "bg": "#f7f5fb",
      "bg-2": "#edf6ff",
      "ink": "#17161a",
      "ink-1": "rgba(23, 22, 26, 0.9)",
      "ink-2": "rgba(23, 22, 26, 0.58)",
      "ink-3": "rgba(23, 22, 26, 0.34)",
      "ink-4": "rgba(23, 22, 26, 0.2)",
      "ink-control": "rgba(23, 22, 26, 0.68)",
      "ink-quiet": "rgba(23, 22, 26, 0.54)",
      "hairline": "rgba(43, 61, 93, 0.10)",
      "hairline-2": "rgba(43, 61, 93, 0.22)",
      "wash": "rgba(108, 156, 222, 0.08)",
      "surface": "rgba(255, 255, 255, 0.88)",
      "scrim": "rgba(50, 65, 93, 0.12)",
      "scrim-modal": "rgba(50, 65, 93, 0.24)",
      "veil": "rgba(239, 245, 255, 0.82)",
      "focus": "rgba(58, 91, 166, 0.8)",
      "accent": "#456bb2",
      "accent-line": "rgba(69, 107, 178, 0.3)",
      "accent-ink": "#ffffff",
      "danger": "#a13d64",
      "mat-line": "rgba(43, 61, 93, 0.18)",
      "mat-line-hover": "rgba(43, 61, 93, 0.3)",
      "mat-line-drag": "rgba(69, 107, 178, 0.65)",
    },
  },
  fonts: {
    sans: ["-apple-system", "BlinkMacSystemFont", "SF Pro Text", "Helvetica Neue", "Helvetica", "Arial", "sans-serif"],
    serif: ["ui-serif", "New York", "Iowan Old Style", "Palatino", "Georgia", "serif"],
    mono: ["ui-monospace", "SFMono-Regular", "SF Mono", "Menlo", "monospace"],
  },
};

function object(value: unknown, path: string, keys: readonly string[]): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${path} must be an object.`);
  }
  const record = value as Record<string, unknown>;
  const extra = Object.keys(record).find((key) => !keys.includes(key));
  if (extra !== undefined) throw new Error(`${path}: unknown field "${extra.slice(0, 60)}".`);
  const missing = keys.find((key) => !Object.prototype.hasOwnProperty.call(record, key));
  if (missing !== undefined) throw new Error(`${path}.${missing} is required.`);
  return record;
}

function color(value: unknown, path: string): string {
  if (typeof value !== "string" || value.length > 64) {
    throw new Error(`${path} must be a hex or rgb/rgba color.`);
  }
  if (/^#(?:[0-9a-f]{6}|[0-9a-f]{8})$/i.test(value)) return value;
  const match = /^(rgb|rgba)\(\s*(\d{1,3}),\s*(\d{1,3}),\s*(\d{1,3})(?:,\s*(0(?:\.\d+)?|1(?:\.0+)?))?\s*\)$/.exec(value);
  if (match !== null && [match[2], match[3], match[4]].every((part) => Number(part) <= 255)
    && (match[1] === "rgba") === (match[5] !== undefined)) return value;
  throw new Error(`${path}: use #RRGGBB, #RRGGBBAA, rgb(0, 0, 0), or rgba(0, 0, 0, 0.5).`);
}

function palette(value: unknown, path: string): Palette {
  const record = object(value, path, COLOR_TOKENS);
  return Object.fromEntries(COLOR_TOKENS.map((key) => [key, color(record[key], `${path}.${key}`)])) as Palette;
}

export function parseSkin(code: string): Skin {
  if (code.length > MAX_SKIN_LENGTH) throw new Error("Skin code is too large (maximum 16 KB).");
  const trimmed = code.trim();
  const fenced = /^```(?:json)?\s*\n([\s\S]*?)\n```$/i.exec(trimmed);
  let raw: unknown;
  try {
    raw = JSON.parse(fenced?.[1] ?? trimmed);
  } catch {
    throw new Error("Invalid JSON. Paste one complete skin object, without comments or trailing commas.");
  }
  const root = object(raw, "skin", ["version", "name", "colors", "fonts"]);
  if (root.version !== 1) throw new Error("Unsupported skin version. Use version 1.");
  if (typeof root.name !== "string" || root.name.trim() === "" || root.name.length > 80
    || /[\u0000-\u001f\u007f]/.test(root.name)) {
    throw new Error("Skin name must contain 1 to 80 printable characters.");
  }
  const colors = object(root.colors, "colors", ["light", "dark"]);
  const fonts = object(root.fonts, "fonts", FONT_TOKENS);
  const parsedFonts = Object.fromEntries(FONT_TOKENS.map((key) => {
    const families = fonts[key];
    if (!Array.isArray(families) || families.length < 1 || families.length > 8
      || !families.every((name: unknown) => typeof name === "string" && name.length <= 64
        && /^[A-Za-z0-9][A-Za-z0-9 _-]*$|^-apple-system$/.test(name) && name.trim() === name)) {
      throw new Error(`fonts.${key}: use 1 to 8 local font-family names (letters, numbers, spaces, hyphens).`);
    }
    return [key, [...families]];
  })) as SkinFonts;
  return {
    version: 1, name: root.name.trim(),
    colors: { light: palette(colors.light, "colors.light"), dark: palette(colors.dark, "colors.dark") },
    fonts: parsedFonts,
  };
}

const GENERIC_FONTS = new Set([
  "serif", "sans-serif", "monospace", "system-ui", "ui-serif", "ui-sans-serif", "ui-monospace",
  "cursive", "fantasy", "-apple-system", "BlinkMacSystemFont",
]);

/** Fixed keys only: layout variables, selectors, URLs and arbitrary declarations never enter CSS. */
export function skinVariables(skin: Skin, scheme: Scheme): Record<string, string> {
  return Object.fromEntries([
    ...COLOR_TOKENS.map((key) => [`--${key}`, skin.colors[scheme][key]]),
    ...FONT_TOKENS.map((key) => [`--${key}`, skin.fonts[key].map((name) =>
      GENERIC_FONTS.has(name) ? name : JSON.stringify(name)).join(", ")]),
  ]);
}

export function renderSkin(skin: Skin | null, scheme: Scheme, style: CSSStyleDeclaration): void {
  if (skin === null) {
    for (const key of [...COLOR_TOKENS, ...FONT_TOKENS]) style.removeProperty(`--${key}`);
    return;
  }
  for (const [key, value] of Object.entries(skinVariables(skin, scheme))) style.setProperty(key, value);
}

export const skinCode = (skin: Skin): string => JSON.stringify(skin, null, 2);

export function skinPrompt(skin: Skin): string {
  return `Customize this Flint skin for my preferences: [describe my preferred colors and fonts].

Return only the complete JSON object, retaining version 1 and exactly the existing keys.
Only change name, color values, and font-family arrays. Do not add CSS, JavaScript, HTML,
URLs, imports, font files, sizes, weights, spacing, layout, animation, or conversion settings.
Colors must be #RRGGBB, #RRGGBBAA, rgb(R, G, B), or rgba(R, G, B, A).
Each font array contains 1-8 local font-family names: letters, numbers, spaces, underscores,
or hyphens, with a generic fallback. Fonts unavailable on my machine use the next fallback.
Keep both light and dark palettes readable: body/control text at least 4.5:1 against their
surfaces, visible focus and error colors, and contrasting accent-ink against accent.
Keep the name within 80 characters. The settings skin editor retains its original appearance.

${skinCode(skin)}`;
}
