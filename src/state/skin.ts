import { create } from "zustand";
import { DEFAULT_SKIN, parseSkin, renderSkin, SKIN_STORAGE_KEY, skinCode } from "../lib/skin";
import type { Scheme, Skin } from "../lib/skin";

interface SkinState {
  saved: Skin | null;
  preview: Skin | null;
  deadline: number | null;
  error: string | null;
  notice: string | null;
  beginPreview: (code: string) => boolean;
  keep: () => void;
  revert: () => void;
  reset: () => void;
  clearMessage: () => void;
}

export const PREVIEW_MS = 20_000;
let timer: ReturnType<typeof setTimeout> | null = null;
const cancelTimer = (): void => {
  if (timer !== null) clearTimeout(timer);
  timer = null;
};

// Separate from conversion settings and IPC: a skin cannot trigger or configure a conversion.
export const useSkin = create<SkinState>()((set, get) => ({
  saved: null, preview: null, deadline: null, error: null, notice: null,
  beginPreview: (code) => {
    try {
      const preview = parseSkin(code);
      cancelTimer();
      const deadline = Date.now() + PREVIEW_MS;
      set({ preview, deadline, error: null, notice: "Preview active. Keep this skin?" });
      timer = setTimeout(() => {
        timer = null;
        set({ preview: null, deadline: null, notice: "Preview expired. Previous skin restored." });
      }, PREVIEW_MS);
      return true;
    } catch (error) {
      set({ error: error instanceof Error ? error.message : "Could not read skin code.", notice: null });
      return false;
    }
  },
  keep: () => {
    const { preview, deadline } = get();
    if (preview === null || deadline === null) return;
    if (Date.now() >= deadline) {
      get().revert();
      return;
    }
    try {
      window.localStorage.setItem(SKIN_STORAGE_KEY, skinCode(preview));
      cancelTimer();
      set({ saved: preview, preview: null, deadline: null, error: null, notice: "Skin applied." });
    } catch {
      set({ error: "Could not save this skin. Copy the code and check local storage permissions.", notice: null });
    }
  },
  revert: () => {
    if (get().preview === null) return;
    cancelTimer();
    set({ preview: null, deadline: null, error: null, notice: "Previous skin restored." });
  },
  reset: () => {
    cancelTimer();
    try {
      window.localStorage.removeItem(SKIN_STORAGE_KEY);
      set({ saved: null, preview: null, deadline: null, error: null, notice: "Original skin restored." });
    } catch {
      // Always recover readable colors, even when persistent storage is unavailable.
      set({ saved: null, preview: null, deadline: null,
        error: "Original skin restored for this session. Could not remove the saved skin.", notice: null });
    }
  },
  clearMessage: () => set({ error: null, notice: null }),
}));

/** Call before React paints. A corrupt saved skin is ignored, never partially applied. */
export function initializeSkin(): () => void {
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  let scheme: Scheme = media.matches ? "dark" : "light";
  try {
    const code = window.localStorage.getItem(SKIN_STORAGE_KEY);
    useSkin.setState({ saved: code === null ? null : parseSkin(code) });
  } catch {
    useSkin.setState({ saved: null, error: "Saved skin could not be loaded. Original skin is active." });
  }
  const render = (): void => {
    const state = useSkin.getState();
    renderSkin(state.preview ?? state.saved, scheme, document.documentElement.style);
  };
  const onScheme = (event: MediaQueryListEvent): void => {
    scheme = event.matches ? "dark" : "light";
    render();
  };
  // Recheck the clock after a suspended/backgrounded window resumes.
  const onVisible = (): void => {
    const { deadline } = useSkin.getState();
    if (deadline !== null && Date.now() >= deadline) useSkin.getState().revert();
  };
  const unsubscribe = useSkin.subscribe(render);
  media.addEventListener("change", onScheme);
  document.addEventListener("visibilitychange", onVisible);
  render();
  return () => {
    cancelTimer();
    unsubscribe();
    media.removeEventListener("change", onScheme);
    document.removeEventListener("visibilitychange", onVisible);
  };
}

export const currentSkinCode = (): string => skinCode(useSkin.getState().saved ?? DEFAULT_SKIN);
