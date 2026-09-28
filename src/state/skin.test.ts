import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_SKIN, SKIN_STORAGE_KEY, skinCode } from "../lib/skin";

let store: typeof import("./skin")["useSkin"];
let initialize: typeof import("./skin")["initializeSkin"];
let cleanup: (() => void) | undefined;
let storage: Map<string, string>;
let properties: Map<string, string>;
beforeEach(async () => {
  vi.useFakeTimers();
  vi.resetModules();
  storage = new Map();
  properties = new Map();
  vi.stubGlobal("window", {
    localStorage: {
      getItem: vi.fn((key: string) => storage.get(key) ?? null),
      setItem: vi.fn((key: string, value: string) => storage.set(key, value)),
      removeItem: vi.fn((key: string) => storage.delete(key)),
    },
    matchMedia: () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }),
  });
  vi.stubGlobal("document", {
    documentElement: { style: {
      setProperty: (key: string, value: string) => properties.set(key, value),
      removeProperty: (key: string) => properties.delete(key),
    } },
    addEventListener: vi.fn(), removeEventListener: vi.fn(),
  });
  ({ useSkin: store, initializeSkin: initialize } = await import("./skin"));
});
afterEach(() => {
  cleanup?.();
  cleanup = undefined;
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("isolated skin preferences", () => {
  it("previews without saving and automatically reverts", async () => {
    cleanup = initialize();
    expect(store.getState().beginPreview(skinCode(DEFAULT_SKIN))).toBe(true);
    expect(properties.get("--bg")).toBe("#f7f5fb");
    expect(storage.size).toBe(0);
    await vi.advanceTimersByTimeAsync(20_001);
    expect(store.getState().preview).toBeNull();
    expect(properties.has("--bg")).toBe(false);
  });
  it("persists only confirmed skins under their own key", async () => {
    cleanup = initialize();
    store.getState().beginPreview(skinCode(DEFAULT_SKIN));
    store.getState().keep();
    expect([...storage.keys()]).toEqual([SKIN_STORAGE_KEY]);
    await vi.advanceTimersByTimeAsync(21_000);
    expect(store.getState().saved).toEqual(DEFAULT_SKIN);
    expect(properties.get("--bg")).toBe(DEFAULT_SKIN.colors.light.bg);
  });
  it("loads saved skins and ignores corrupt data", () => {
    storage.set(SKIN_STORAGE_KEY, skinCode(DEFAULT_SKIN));
    cleanup = initialize();
    expect(store.getState().saved).toEqual(DEFAULT_SKIN);
    cleanup();
    storage.set(SKIN_STORAGE_KEY, "{bad");
    cleanup = initialize();
    expect(store.getState().saved).toBeNull();
    expect(properties.size).toBe(0);
    expect(store.getState().error).toContain("could not be loaded");
  });
  it("invalid import never changes the applied skin or conversion storage", () => {
    storage.set("conversion-settings", "untouched");
    cleanup = initialize();
    expect(store.getState().beginPreview('{"css":"body{display:none}"}')).toBe(false);
    expect(properties.size).toBe(0);
    expect(storage.get("conversion-settings")).toBe("untouched");
  });
  it("reverts to the previous saved skin and resets only the skin", () => {
    storage.set("conversion-settings", "untouched");
    storage.set(SKIN_STORAGE_KEY, skinCode(DEFAULT_SKIN));
    cleanup = initialize();
    const next = JSON.parse(skinCode(DEFAULT_SKIN));
    next.colors.light.bg = "#112233";
    store.getState().beginPreview(JSON.stringify(next));
    expect(properties.get("--bg")).toBe("#112233");
    store.getState().revert();
    expect(properties.get("--bg")).toBe(DEFAULT_SKIN.colors.light.bg);
    store.getState().reset();
    expect(properties.size).toBe(0);
    expect([...storage]).toEqual([["conversion-settings", "untouched"]]);
  });
  it("failed persistence does not claim a skin was saved", () => {
    cleanup = initialize();
    vi.mocked(window.localStorage.setItem).mockImplementation(() => { throw new Error("quota"); });
    store.getState().beginPreview(skinCode(DEFAULT_SKIN));
    store.getState().keep();
    expect(store.getState().saved).toBeNull();
    expect(store.getState().error).toContain("Could not save");
    expect(store.getState().preview).not.toBeNull();
  });
  it("switches palettes with the operating system without persisting a preview", () => {
    let change: ((event: MediaQueryListEvent) => void) | undefined;
    window.matchMedia = () => ({
      matches: false,
      addEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => { change = listener; },
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList);
    cleanup = initialize();
    store.getState().beginPreview(skinCode(DEFAULT_SKIN));
    change?.({ matches: true } as MediaQueryListEvent);
    expect(properties.get("--bg")).toBe(DEFAULT_SKIN.colors.dark.bg);
    expect(storage.size).toBe(0);
  });
});
