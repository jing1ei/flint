import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { COLOR_TOKENS, DEFAULT_SKIN, FONT_TOKENS, MAX_SKIN_LENGTH, parseSkin, renderSkin, skinCode, skinPrompt, skinVariables } from "./skin";

describe("skin code boundary", () => {
  it("keeps exported defaults exactly in sync with the existing stylesheet", () => {
    const css = readFileSync(new URL("../styles.css", import.meta.url), "utf8");
    const light = css.slice(css.indexOf(":root {"), css.indexOf("@media (prefers-color-scheme: dark)"));
    const dark = css.slice(css.indexOf("@media (prefers-color-scheme: dark)"), css.indexOf("* {"));
    for (const key of COLOR_TOKENS) {
      expect(light).toContain(`--${key}: ${DEFAULT_SKIN.colors.light[key]};`);
      expect(dark).toContain(`--${key}: ${DEFAULT_SKIN.colors.dark[key]};`);
    }
  });
  it("round-trips the original skin and accepts a single JSON fence", () => {
    expect(parseSkin(skinCode(DEFAULT_SKIN))).toEqual(DEFAULT_SKIN);
    expect(parseSkin(`\`\`\`json\n${skinCode(DEFAULT_SKIN)}\n\`\`\``)).toEqual(DEFAULT_SKIN);
  });
  it.each(["layout", "settings", "css", "__proto__", "fontSize"])("rejects extra %s fields", (key) => {
    const raw = skinCode(DEFAULT_SKIN).replace('"version": 1', `"${key}": {}, "version": 1`);
    expect(() => parseSkin(raw)).toThrow("unknown field");
  });
  it.each(["red; display:none", "url(https://example.com)", "var(--bg)", "rgb(256,0,0)", "rgba(0,0,0,2)", "transparent"])("rejects unsafe or invalid color %s", (value) => {
    const skin = parseSkin(skinCode(DEFAULT_SKIN));
    skin.colors.light.bg = value;
    expect(() => parseSkin(skinCode(skin))).toThrow("colors.light.bg");
  });
  it.each(["#123456", "#12345680", "rgb(255, 0, 12)", "rgba(1, 2, 3, 0.25)"])("accepts color %s", (value) => {
    const skin = parseSkin(skinCode(DEFAULT_SKIN));
    skin.colors.dark.accent = value;
    expect(parseSkin(skinCode(skin)).colors.dark.accent).toBe(value);
  });
  it.each(["Arial; width:999px", "url(x)", "</style>", "@font-face", "inherit", "initial", "unset"])("cannot inject CSS through font %s", (value) => {
    const skin = parseSkin(skinCode(DEFAULT_SKIN));
    skin.fonts.sans = [value];
    if (/^[a-z]+$/.test(value)) {
      // CSS-wide keywords are quoted family names, not declarations.
      expect(skinVariables(parseSkin(skinCode(skin)), "light")["--sans"]).toBe(`"${value}"`);
    } else expect(() => parseSkin(skinCode(skin))).toThrow("fonts.sans");
  });
  it("rejects missing fields, unknown palettes, versions and oversize inputs", () => {
    expect(() => parseSkin("{}")).toThrow("required");
    expect(() => parseSkin(skinCode({ ...DEFAULT_SKIN, version: 2 } as never))).toThrow("version");
    expect(() => parseSkin(" ".repeat(MAX_SKIN_LENGTH + 1))).toThrow("too large");
    const skin = JSON.parse(skinCode(DEFAULT_SKIN));
    skin.colors.light["gutter"] = "100px";
    expect(() => parseSkin(JSON.stringify(skin))).toThrow("unknown field");
  });
  it("generates only color and font variables, preserving existing layout declarations", () => {
    const properties = new Map([["--gutter", "26px"], ["--t", "320ms"]]);
    const style = {
      setProperty: (name: string, value: string) => properties.set(name, value),
      removeProperty: (name: string) => properties.delete(name),
    } as unknown as CSSStyleDeclaration;
    renderSkin(DEFAULT_SKIN, "dark", style);
    expect(properties.get("--bg")).toBe("#f7f5fb");
    expect(properties.get("--gutter")).toBe("26px");
    expect(Object.keys(skinVariables(DEFAULT_SKIN, "light")).sort()).toEqual(
      [...COLOR_TOKENS, ...FONT_TOKENS].map((key) => `--${key}`).sort(),
    );
    renderSkin(null, "light", style);
    expect([...properties]).toEqual([["--gutter", "26px"], ["--t", "320ms"]]);
  });
  it("exports an LLM prompt containing the complete schema and no app data", () => {
    const prompt = skinPrompt(DEFAULT_SKIN);
    expect(prompt).toContain(skinCode(DEFAULT_SKIN));
    expect(prompt).toContain("Do not add CSS");
    expect(prompt).toContain("conversion settings");
    expect(prompt).toContain("preferences:");
  });
});
