import type { CategoryId, CropSettings, Settings } from "./types";

export function cropError(crop: CropSettings): string | null {
  if (!crop.image && !crop.media && !crop.document) return "Choose at least one crop range.";
  if (crop.image) {
    const { x, y, width, height } = crop.image;
    if (![x, y, width, height].every((v) => Number.isInteger(v) && v >= 0 && v <= 100000)
      || width === 0 || height === 0) return "Enter whole pixel values; width and height must be positive (maximum 100000).";
  }
  if (crop.media) {
    const { start_secs: start, length_secs: length } = crop.media;
    if (![start, length].every(Number.isFinite) || start < 0 || length <= 0 || start + length > 86400)
      return "Enter a positive media range ending within 24 hours.";
  }
  if (crop.document) {
    const { start, end } = crop.document;
    if (![start, end].every(Number.isInteger) || start < 1 || end < start || end > 10000000)
      return "Document ranges start at 1 and include both endpoints (maximum 10000000).";
  }
  return null;
}

export function cropApplies(crop: CropSettings, category: CategoryId | null): boolean {
  if (category === "image") return crop.image !== null;
  if (category === "document") return crop.document !== null;
  return category === "audio" || category === "video" || category === "flash" ? crop.media !== null : false;
}

export function cropRunSettings(settings: Settings, crop: CropSettings | null): Settings {
  if (crop === null) {
    const { crop: _ignored, ...regular } = settings;
    return regular;
  }
  return {
    ...settings, crop,
    trim: { enabled: false, start_secs: 0, length_secs: 0 },
  };
}

/** Mirrors the engine's per-input time range for browser demo conversions. */
export function cropMediaSettings(settings: Settings, category: CategoryId | null): Settings {
  if (!settings.crop || category === null || !["video", "audio", "flash"].includes(category) || !settings.crop.media) return settings;
  return { ...settings, trim: { enabled: true, ...settings.crop.media } };
}
