import { describe, expect, it } from "vitest";
import { cropApplies, cropError, cropRunSettings } from "./crop";
import type { CropSettings, Settings } from "./types";

const selection = (): CropSettings => ({
  image: { x: 0, y: 0, width: 100, height: 100 },
  media: { start_secs: 10, length_secs: 40 },
  document: { unit: "words", start: 2000, end: 10000 },
});
describe("batch crop values", () => {
  it("accepts the requested numeric examples", () => {
    expect(cropError(selection())).toBeNull();
  });
  it.each([NaN, Infinity, -1, 1.5, 100001])("rejects invalid image width %s", (width) => {
    const crop = selection();
    crop.image!.width = width;
    expect(cropError(crop)).not.toBeNull();
  });
  it.each([0, -1, NaN, Infinity, 86400])("rejects invalid media duration %s", (length_secs) => {
    const crop = selection();
    crop.media!.length_secs = length_secs;
    expect(cropError(crop)).not.toBeNull();
  });
  it("refuses reversed or zero-based document ranges", () => {
    const crop = selection();
    crop.document!.start = 10001;
    expect(cropError(crop)).not.toBeNull();
    crop.document!.start = 0;
    expect(cropError(crop)).not.toBeNull();
  });
  it("filters categories and requires a selection", () => {
    const crop = selection();
    expect(cropApplies(crop, "subtitle")).toBe(false);
    expect(cropApplies(crop, "image")).toBe(true);
    crop.image = null;
    expect(cropApplies(crop, "image")).toBe(false);
    expect(cropError({ image: null, media: null, document: null })).not.toBeNull();
  });
  it("leaves saved settings and ordinary conversion unchanged", () => {
    const original = { trim: { enabled: true, start_secs: 5, length_secs: 2 } } as Settings;
    const batch = cropRunSettings(original, selection());
    expect(batch.trim.enabled).toBe(false);
    expect(original.trim.enabled).toBe(true);
    expect(cropRunSettings(original, null)).toEqual(original);
    expect(cropRunSettings(batch, null)).not.toHaveProperty("crop");
  });
});
