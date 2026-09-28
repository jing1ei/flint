/**
 * The one and only module allowed to import `@tauri-apps/*`.
 *
 * Everything above this file talks to a plain `Backend` interface, which has exactly two
 * implementations: the real Tauri IPC below, and the browser mock in `./mock`. The mock is loaded
 * through a dynamic `import()` that only runs when `__TAURI_INTERNALS__` is absent, so inside the
 * app the mock chunk is never fetched, never parsed and never able to answer a call.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  Activity,
  BatchEvent,
  BatchItemArg,
  BrowserPresence,
  CatalogView,
  CookieTest,
  Inspection,
  InstallEvent,
  LinkInspection,
  LinkSupport,
  PackageInstallPlan,
  PresetId,
  SafariAccess,
  Settings,
  ToolStatus,
} from "./types";

/** What every subscription hands back: call it to stop listening. Matches Tauri's `UnlistenFn`. */
export type Unlisten = () => void;

/** Native drag & drop, reduced to what the UI actually reacts to. */
export type DropEvent =
  | { kind: "hover" }
  | { kind: "leave" }
  | { kind: "drop"; paths: string[] };

/**
 * Every command the native menu bar can send. The Rust side (`src-tauri/src/menu.rs`) emits these
 * ids as a plain string payload; this union is the only place the frontend agrees to know them.
 */
export type MenuAction =
  | "settings"
  | "skin"
  | "open_files"
  | "open_folder"
  | "paste_links"
  | "convert"
  | "stop"
  | "clear";

const MENU_ACTIONS: readonly MenuAction[] = [
  "settings",
  "skin",
  "open_files",
  "open_folder",
  "paste_links",
  "convert",
  "stop",
  "clear",
];

/**
 * A payload that crossed the IPC boundary is just a string. Validate rather than cast: a menu id
 * renamed in Rust must surface as "nothing happened", never as a call on an undefined action.
 */
function toMenuAction(raw: string): MenuAction | null {
  return MENU_ACTIONS.includes(raw as MenuAction) ? (raw as MenuAction) : null;
}

export interface Backend {
  getCatalog(): Promise<CatalogView>;
  getSettings(): Promise<Settings>;
  saveSettings(settings: Settings): Promise<Settings>;
  applyPreset(presetId: PresetId): Promise<Settings>;
  inspectFiles(paths: string[]): Promise<Inspection>;
  /**
   * Judge a paste of video links, line by line, without touching the network. Rejects the *whole*
   * paste when it is over the cap — twenty-one links is a mistake to correct, not twenty-one rows.
   */
  inspectLinks(links: string[]): Promise<LinkInspection>;
  /** The cap, the accepted hosts, whether yt-dlp is here, and where a link's output will land. */
  getLinkSupport(settings: Settings): Promise<LinkSupport>;
  /**
   * Every allowlisted browser, in allowlist order, with the ones this machine actually has marked.
   *
   * The whole list rather than only what is installed: "you have Chrome" and "there is nothing here
   * to borrow from" are different answers, and a filtered list makes them look the same.
   */
  listCookieBrowsers(): Promise<BrowserPresence[]>;
  /**
   * Try the configured sign-in against one public video and say what happened.
   *
   * Takes the *current* settings rather than what is on disk, because the point is to check the
   * choice the user just made before anything depends on it. Rejects with `settings_store`'s own
   * refusal when the source is half-made — that is a sentence about the settings, not a verdict
   * about a sign-in, and it is shown where the check was asked for.
   */
  testCookieSource(settings: Settings, url?: string): Promise<CookieTest>;
  /**
   * Can this app read Safari's cookie jar right now? One `open`, no bytes, no network.
   *
   * Separate from `testCookieSource` because it is a different question with a different cost.
   * Safari's jar is behind Full Disk Access, and asking a twenty-second probe of a public video
   * whether a macOS permission is in place is twenty seconds spent on an answer `open(2)` gives
   * instantly — and gives without a network at all.
   */
  checkSafariCookieAccess(): Promise<SafariAccess>;
  estimateOutputPath(path: string, targetId: string, settings: Settings): Promise<string>;
  startBatch(items: BatchItemArg[], settings: Settings): Promise<void>;
  cancelBatch(): Promise<void>;
  /** What the shell is busy with. The one thing a freshly loaded window cannot work out itself. */
  getActivity(): Promise<Activity>;
  refreshTools(): Promise<ToolStatus[]>;
  getInstallPlans(): Promise<PackageInstallPlan[]>;
  /**
   * Install one *package* (`poppler`), never one binary. Resolves once the installer is
   * *running*; the outcome arrives on `install://event`.
   */
  installPackage(packageId: string): Promise<void>;
  onInstallEvent(handler: (event: InstallEvent) => void): Promise<Unlisten>;
  revealInFinder(path: string): Promise<void>;
  openPath(path: string): Promise<void>;
  /**
   * Open System Settings on Full Disk Access — the permission Safari's cookie jar sits behind.
   *
   * No argument, on purpose. `openPath` is already restricted to what this app converts because its
   * path comes from the webview; a URL scheme cannot be restricted the same way, so there is nothing
   * to restrict: the address lives in Rust as a constant and this call is the whole of the request.
   */
  openFullDiskAccessSettings(): Promise<void>;
  onBatchEvent(handler: (event: BatchEvent) => void): Promise<Unlisten>;
  onDragDrop(handler: (event: DropEvent) => void): Promise<Unlisten>;
  onMenuAction(handler: (action: MenuAction) => void): Promise<Unlisten>;
  pickFiles(): Promise<string[]>;
  pickDirectory(): Promise<string | null>;
}

const BATCH_EVENT = "batch://event";
const INSTALL_EVENT = "install://event";
const MENU_EVENT = "menu://action";

/**
 * True inside the Tauri webview, false in a plain browser tab (`npm run dev` / `vite preview`).
 *
 * Exported because the native menu bar owns ⌘, ⌘O ⇧⌘O ⌘↩ ⌘. ⇧⌘⌫ on macOS: the webview must not
 * bind them a second time, or every one of them would fire twice.
 */
export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

const asArray = (value: string | string[] | null): string[] =>
  value === null ? [] : Array.isArray(value) ? value : [value];

const tauriBackend: Backend = {
  getCatalog: () => invoke<CatalogView>("get_catalog"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings) => invoke<Settings>("save_settings", { settings }),
  applyPreset: (preset_id) => invoke<Settings>("apply_preset", { preset_id }),
  inspectFiles: (paths) => invoke<Inspection>("inspect_files", { paths }),
  inspectLinks: (links) => invoke<LinkInspection>("inspect_links", { links }),
  getLinkSupport: (settings) => invoke<LinkSupport>("get_link_support", { settings }),
  listCookieBrowsers: () => invoke<BrowserPresence[]>("list_cookie_browsers"),
  testCookieSource: (settings, url) => invoke<CookieTest>("test_cookie_source", { settings, url: url ?? null }),
  // Zero arguments: the jar's location is Rust's to work out, and a webview never names a path.
  checkSafariCookieAccess: () => invoke<SafariAccess>("check_safari_cookie_access"),
  estimateOutputPath: (path, target_id, settings) =>
    invoke<string>("estimate_output_path", { path, target_id, settings }),
  startBatch: (items, settings) => invoke<void>("start_batch", { items, settings }),
  cancelBatch: () => invoke<void>("cancel_batch"),
  getActivity: () => invoke<Activity>("get_activity"),
  refreshTools: () => invoke<ToolStatus[]>("refresh_tools"),
  getInstallPlans: () => invoke<PackageInstallPlan[]>("get_install_plans"),
  // Rust takes a package *id* and looks it up in its own allowlist: the webview never names a
  // command, and never a binary either - one click is one `brew install`. The command is still
  // called `install_tool`, which is the name the app shipped with.
  installPackage: (package_id) => invoke<void>("install_tool", { package_id }),
  onInstallEvent: (handler) => listen<InstallEvent>(INSTALL_EVENT, (e) => handler(e.payload)),
  revealInFinder: (path) => invoke<void>("reveal_in_finder", { path }),
  openPath: (path) => invoke<void>("open_path", { path }),
  // Zero arguments, so there is no URL for a compromised webview to substitute.
  openFullDiskAccessSettings: () => invoke<void>("open_full_disk_access_settings"),
  onBatchEvent: (handler) => listen<BatchEvent>(BATCH_EVENT, (e) => handler(e.payload)),
  onDragDrop: (handler) =>
    getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload;
      if (payload.type === "drop") handler({ kind: "drop", paths: payload.paths });
      else if (payload.type === "leave") handler({ kind: "leave" });
      else handler({ kind: "hover" });
    }),
  onMenuAction: (handler) =>
    listen<string>(MENU_EVENT, (event) => {
      const action = toMenuAction(event.payload);
      if (action !== null) handler(action);
    }),
  pickFiles: async () => asArray(await open({ multiple: true, directory: false })),
  pickDirectory: async () => {
    const picked = await open({ multiple: false, directory: true });
    return typeof picked === "string" ? picked : null;
  },
};

let cached: Promise<Backend> | null = null;

function backend(): Promise<Backend> {
  if (cached === null) {
    cached = isTauriRuntime()
      ? Promise.resolve(tauriBackend)
      : import("./mock").then((m) => m.createMockBackend());
  }
  return cached;
}

// The exported surface: thin, typed, awaits the right implementation. Callers never branch.
export const getCatalog = async (): Promise<CatalogView> => (await backend()).getCatalog();
export const getSettings = async (): Promise<Settings> => (await backend()).getSettings();
export const saveSettings = async (settings: Settings): Promise<Settings> =>
  (await backend()).saveSettings(settings);
export const applyPreset = async (presetId: PresetId): Promise<Settings> =>
  (await backend()).applyPreset(presetId);
export const inspectFiles = async (paths: string[]): Promise<Inspection> =>
  (await backend()).inspectFiles(paths);
export const inspectLinks = async (links: string[]): Promise<LinkInspection> =>
  (await backend()).inspectLinks(links);
export const getLinkSupport = async (settings: Settings): Promise<LinkSupport> =>
  (await backend()).getLinkSupport(settings);
export const listCookieBrowsers = async (): Promise<BrowserPresence[]> =>
  (await backend()).listCookieBrowsers();
export const testCookieSource = async (settings: Settings, url?: string): Promise<CookieTest> =>
  (await backend()).testCookieSource(settings, url);
export const checkSafariCookieAccess = async (): Promise<SafariAccess> =>
  (await backend()).checkSafariCookieAccess();
export const estimateOutputPath = async (
  path: string,
  targetId: string,
  settings: Settings,
): Promise<string> => (await backend()).estimateOutputPath(path, targetId, settings);
export const startBatch = async (items: BatchItemArg[], settings: Settings): Promise<void> =>
  (await backend()).startBatch(items, settings);
export const cancelBatch = async (): Promise<void> => (await backend()).cancelBatch();
export const getActivity = async (): Promise<Activity> => (await backend()).getActivity();
export const refreshTools = async (): Promise<ToolStatus[]> => (await backend()).refreshTools();
export const getInstallPlans = async (): Promise<PackageInstallPlan[]> =>
  (await backend()).getInstallPlans();
export const installPackage = async (packageId: string): Promise<void> =>
  (await backend()).installPackage(packageId);
export const onInstallEvent = async (
  handler: (event: InstallEvent) => void,
): Promise<Unlisten> => (await backend()).onInstallEvent(handler);
export const revealInFinder = async (path: string): Promise<void> =>
  (await backend()).revealInFinder(path);
export const openPath = async (path: string): Promise<void> => (await backend()).openPath(path);
export const openFullDiskAccessSettings = async (): Promise<void> =>
  (await backend()).openFullDiskAccessSettings();
export const onBatchEvent = async (
  handler: (event: BatchEvent) => void,
): Promise<Unlisten> => (await backend()).onBatchEvent(handler);
export const onDragDrop = async (handler: (event: DropEvent) => void): Promise<Unlisten> =>
  (await backend()).onDragDrop(handler);
export const onMenuAction = async (handler: (action: MenuAction) => void): Promise<Unlisten> =>
  (await backend()).onMenuAction(handler);
export const pickFiles = async (): Promise<string[]> => (await backend()).pickFiles();
export const pickDirectory = async (): Promise<string | null> => (await backend()).pickDirectory();

/** Every command rejects with a plain string; normalise it so the UI can always render something. */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Something went wrong";
}
