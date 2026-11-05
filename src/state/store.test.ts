import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as ipc from "../lib/ipc";
import { MOCK_CATALOG } from "../lib/mock-catalog";
import type { BrowserPresence, LinkRow, Settings } from "../lib/types";

vi.mock("../lib/ipc", () => ({
  saveSettings: vi.fn(), applyPreset: vi.fn(), startBatch: vi.fn(), inspectFiles: vi.fn(),
  estimateOutputPath: vi.fn(), getLinkSupport: vi.fn(), inspectLinks: vi.fn(),
  testCookieSource: vi.fn(), pickDirectory: vi.fn(), pickFiles: vi.fn(),
  onBatchEvent: vi.fn(), onInstallEvent: vi.fn(), onDragDrop: vi.fn(), onMenuAction: vi.fn(),
  getCatalog: vi.fn(), getSettings: vi.fn(), getInstallPlans: vi.fn(),
  listCookieBrowsers: vi.fn(), getActivity: vi.fn(),
  errorMessage: (e: unknown) => e instanceof Error ? e.message : String(e),
}));

const defaults = (): Settings => ({
  preset: "web_and_demo",
  video: { codec: "auto", quality: "balanced", max_height: 1080, fps_cap: 60,
    bitrate_kbps: null, faststart: true, strip_metadata: true, hardware_accel: "auto" },
  audio: { codec: "auto", bitrate_kbps: 192, sample_rate: null, channels: null, normalize_loudness: false },
  image: { quality: 85, max_dimension: 2560, strip_metadata: true,
    flatten_background: "#ffffff", lossless: false, frame_extract_fps: 1 },
  gif: { fps: 12, width: 480, optimize_palette: true, loop_count: 0 },
  document: { raster_dpi: 150, first_page_only: false },
  output: { location: "subfolder", custom_dir: null, subfolder_name: "Converted",
    on_conflict: "rename", parallel_jobs: 0, preserve_timestamps: true },
  trim: { enabled: false, start_secs: 0, length_secs: 10 },
  link: { cookies: "none", cookie_browser: "", cookie_file: null },
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

const link = (id: string): LinkRow => ({
  id, url: `https://youtu.be/${id}`, supported: true, site: "youtube",
  site_label: "YouTube", category: "video", default_target: "mp4", suggested_targets: ["mp4"], note: null,
});

describe("music source classification", () => {
  it("keeps music in the audio group with audio defaults", () => {
    const item: LinkRow = { ...link("music"), site: "soundcloud", category: "audio",
      site_label: "SoundCloud", default_target: "mp3", suggested_targets: ["mp3", "flac"] };
    const result = row(item);
    expect(result.info.category).toBe("audio");
    expect(result.target).toBe("mp3");
    expect(result.info.suggested_targets).toEqual(["mp3", "flac"]);
  });
});

let store: typeof import("./store")["useStore"];
let row: typeof import("./store")["linkRow"];
beforeEach(async () => {
  vi.useFakeTimers();
  vi.resetAllMocks();
  vi.resetModules();
  ({ useStore: store, linkRow: row } = await import("./store"));
  store.setState({ settings: defaults(), catalog: MOCK_CATALOG });
  vi.mocked(ipc.saveSettings).mockImplementation(async (settings) => settings);
  vi.mocked(ipc.getLinkSupport).mockResolvedValue({
    max_links: 20, accepted_hosts: ["youtu.be"], destination: "/Downloads",
    tool_installed: true, package_id: "yt-dlp",
  });
});
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); });

describe("settings persistence", () => {
  it.each(["row", "category"])("retires the previous tally after a %s format change", (kind) => {
    const first = { ...row(link("first")), status: "done" as const };
    store.setState({ files: { first }, order: ["first"], phase: "finished",
      summary: { ok: 1, failed: 0, skipped: 0 } });
    if (kind === "row") store.getState().setTarget("first", "mp3");
    else store.getState().setCategoryTarget("video", "mp3");
    expect(store.getState().phase).toBe("idle");
    expect(store.getState().summary).toBeNull();
    expect(store.getState().files.first?.status).toBe("queued");
  });

  it("starts a transient crop batch, preserves its ranges on retry and keeps regular Convert unchanged", async () => {
    const first = row(link("first"));
    store.setState({ files: { first }, order: ["first"] });
    const crop = { image: null, document: null, media: { start_secs: 10, length_secs: 40 } };
    await store.getState().startCrop(crop);
    expect(ipc.startBatch).toHaveBeenLastCalledWith(
      [{ id: "first", url: first.link!.url, target_id: first.target }],
      { ...defaults(), trim: { enabled: false, start_secs: 0, length_secs: 0 }, crop },
    );
    expect(ipc.saveSettings).not.toHaveBeenCalled();
    expect(store.getState().settings).toEqual(defaults());
    store.setState({ phase: "finished" });
    await store.getState().retryRows(["first"]);
    expect(vi.mocked(ipc.startBatch).mock.calls.at(-1)?.[1].crop).toEqual(crop);
    store.setState({ phase: "idle", files: { first: { ...store.getState().files.first!, status: "queued" } } });
    await store.getState().start();
    expect(vi.mocked(ipc.startBatch).mock.calls.at(-1)?.[1].crop).toBeUndefined();
  });

  it("does not start an ordinary conversion behind the crop dialog", async () => {
    const first = row(link("first"));
    store.setState({ files: { first }, order: ["first"], cropOpen: true });
    await store.getState().start();
    expect(ipc.startBatch).not.toHaveBeenCalled();
  });

  it.each(["output", "trim", "link"] as const)("persists independent edits with unfinished %s", async (group) => {
    const settings = defaults();
    settings.image.quality = 42;
    if (group === "output") settings.output.location = "custom";
    if (group === "trim") settings.trim = { enabled: true, length_secs: 0, start_secs: 0 };
    if (group === "link") settings.link.cookies = "file";
    store.getState().patchSettings(settings);
    await vi.advanceTimersByTimeAsync(301);
    expect(ipc.saveSettings).toHaveBeenCalledWith(settings);
  });

  it("flushes trim and cookies before applying a preset and never replays the old save", async () => {
    const settings = defaults();
    settings.trim.enabled = true;
    settings.link = { cookies: "browser", cookie_browser: "firefox", cookie_file: null };
    let saved = defaults();
    vi.mocked(ipc.saveSettings).mockImplementation(async (next) => { saved = next; return next; });
    vi.mocked(ipc.applyPreset).mockImplementation(async (preset) => {
      saved = { ...defaults(), preset, trim: saved.trim, link: saved.link };
      return saved;
    });
    store.getState().patchSettings(settings);
    await store.getState().choosePreset("smallest");
    await vi.advanceTimersByTimeAsync(500);
    expect(saved.preset).toBe("smallest");
    expect(saved.trim.enabled).toBe(true);
    expect(saved.link.cookie_browser).toBe("firefox");
    expect(store.getState().settings).toEqual(saved);
    expect(ipc.saveSettings).toHaveBeenCalledTimes(1);
  });

  it("serializes presets behind in-flight saves", async () => {
    const pending = deferred<Settings>();
    vi.mocked(ipc.saveSettings).mockReturnValueOnce(pending.promise);
    vi.mocked(ipc.applyPreset).mockResolvedValue({ ...defaults(), preset: "archive" });
    store.getState().patchSettings(defaults());
    await vi.advanceTimersByTimeAsync(301);
    const chosen = store.getState().choosePreset("archive");
    await Promise.resolve();
    expect(ipc.applyPreset).not.toHaveBeenCalled();
    pending.resolve(defaults());
    await chosen;
    expect(store.getState().settings?.preset).toBe("archive");
  });

  it("keeps the last of rapid preset selections in sync", async () => {
    vi.mocked(ipc.applyPreset).mockImplementation(async (preset) => ({ ...defaults(), preset }));
    await Promise.all([
      store.getState().choosePreset("smallest"), store.getState().choosePreset("archive"),
    ]);
    expect(store.getState().settings?.preset).toBe("archive");
  });

  it("does not let a preset overtake an edit queued while a previous save is pending", async () => {
    const pending = deferred<Settings>();
    const order: string[] = [];
    vi.mocked(ipc.saveSettings)
      .mockReturnValueOnce(pending.promise)
      .mockImplementation(async (next) => { order.push("edit"); return next; });
    vi.mocked(ipc.applyPreset).mockImplementation(async (preset) => {
      order.push("preset");
      return { ...defaults(), preset };
    });
    store.getState().patchSettings(defaults());
    await vi.advanceTimersByTimeAsync(301);
    const choosing = store.getState().choosePreset("archive");
    const next = defaults();
    next.image.quality = 42;
    store.getState().patchSettings(next);
    await vi.advanceTimersByTimeAsync(301);
    pending.resolve(defaults());
    await choosing;
    await vi.advanceTimersByTimeAsync(1);
    expect(order).toEqual(["preset", "edit"]);
    expect(store.getState().settings?.image.quality).toBe(42);
  });

  it("does not overwrite an edit made while a folder picker was open", async () => {
    const pending = deferred<string | null>();
    vi.mocked(ipc.pickDirectory).mockReturnValue(pending.promise);
    const choosing = store.getState().pickOutputDir();
    const settings = defaults();
    settings.image.quality = 42;
    store.getState().patchSettings(settings);
    pending.resolve("/new");
    await choosing;
    expect(store.getState().settings?.image.quality).toBe(42);
    expect(store.getState().settings?.output.custom_dir).toBe("/new");
  });
});

describe("batch rollback", () => {
  it.each(["start", "retry", "retryRows"] as const)("keeps new rows after a refused %s", async (action) => {
    const original = { ...row(link("original")), status: "failed" as const, message: "Old failure" };
    store.setState({ files: { original }, order: ["original"] });
    const pending = deferred<void>();
    vi.mocked(ipc.startBatch).mockReturnValue(pending.promise);
    const running = action === "start" ? store.getState().start()
      : action === "retry" ? store.getState().retry("original")
      : store.getState().retryRows(["original"]);
    store.getState().addLinks([link("new")]);
    pending.reject(new Error("Batch refused"));
    await running;
    expect(store.getState().files.original).toEqual(original);
    expect(store.getState().files.new).toBeDefined();
    expect(store.getState().order).toEqual(["original", "new"]);
    expect(store.getState().phase).toBe("idle");
    expect(store.getState().error).toBe("Batch refused");
  });

  it("deduplicates retry IDs", async () => {
    store.setState({ files: { a: row(link("a")) }, order: ["a"] });
    await store.getState().retryRows(["a", "a"]);
    expect(vi.mocked(ipc.startBatch).mock.calls[0]?.[0]).toHaveLength(1);
  });

  it("does not publish a new files object for an ignored duplicate event", () => {
    const files = { a: { ...row(link("a")), status: "done" as const } };
    store.setState({ files, phase: "running", batchIds: new Set(["a"]) });
    store.getState().handleEvent({ type: "failed", id: "a", message: "late" });
    expect(store.getState().files).toBe(files);
  });
});

describe("stale asynchronous results", () => {
  it.each(["clear", "close"] as const)("discards link errors after %s", async (action) => {
    const pending = deferred<never>();
    vi.mocked(ipc.inspectLinks).mockReturnValue(pending.promise);
    const checking = store.getState().checkLinks(["old"]);
    if (action === "clear") store.getState().clearLinksError();
    else store.getState().closeLinks();
    pending.reject(new Error("Old refusal"));
    await checking;
    expect(store.getState().linksError).toBeNull();
  });

  it("ignores an old destination after clearing the queue", async () => {
    const pending = deferred<Awaited<ReturnType<typeof ipc.getLinkSupport>>>();
    vi.mocked(ipc.getLinkSupport).mockReturnValue(pending.promise);
    store.getState().addLinks([link("a")]);
    await vi.advanceTimersByTimeAsync(201);
    store.getState().clearAll();
    pending.resolve({ max_links: 20, accepted_hosts: [], destination: "/old",
      tool_installed: true, package_id: "yt-dlp" });
    await Promise.resolve();
    expect(store.getState().destination).toBeNull();
  });

  it("ignores a destination response overtaken by newer settings", async () => {
    const first = deferred<Awaited<ReturnType<typeof ipc.getLinkSupport>>>();
    const second = deferred<Awaited<ReturnType<typeof ipc.getLinkSupport>>>();
    vi.mocked(ipc.getLinkSupport).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    store.getState().addLinks([link("a")]);
    await vi.advanceTimersByTimeAsync(201);
    const next = defaults();
    next.output.custom_dir = "/new";
    store.getState().patchSettings(next);
    await vi.advanceTimersByTimeAsync(201);
    const support = { max_links: 20, accepted_hosts: [], tool_installed: true, package_id: "yt-dlp" };
    second.resolve({ ...support, destination: "/new" });
    await Promise.resolve();
    first.resolve({ ...support, destination: "/old" });
    await Promise.resolve();
    expect(store.getState().destination).toBe("/new");
  });

  it("ignores a sign-in result when the user changes the source", async () => {
    const pending = deferred<Awaited<ReturnType<typeof ipc.testCookieSource>>>();
    vi.mocked(ipc.testCookieSource).mockReturnValue(pending.promise);
    const checking = store.getState().checkSignIn();
    const next = defaults();
    next.link = { cookies: "browser", cookie_browser: "firefox", cookie_file: null };
    store.getState().patchSettings(next);
    pending.resolve({ ok: true, result: "working", message: "Old source works", tested_url: "https://youtu.be/test" });
    expect(await checking).toBe(false);
    expect(store.getState().signInCheck).toBeNull();
  });

  it("checks the failed music source rather than an unrelated video site", async () => {
    const source: LinkRow = { ...link("song"), url: "https://soundcloud.com/artist/track",
      category: "audio", site: "soundcloud", site_label: "SoundCloud", default_target: "mp3" };
    store.setState({ order: ["song"], files: { song: { ...row(source), status: "failed",
      message: "The site wants a sign-in before it will hand this video over." } } });
    vi.mocked(ipc.testCookieSource).mockResolvedValue({
      ok: true, result: "working", message: "Track accessible", tested_url: source.url,
    });
    expect(await store.getState().checkSignIn()).toBe(true);
    expect(ipc.testCookieSource).toHaveBeenCalledWith(store.getState().settings, source.url);
  });
});

describe("modal ownership", () => {
  it("Settings answers a sign-in prompt instead of stacking over it", () => {
    store.setState({ signInPrompt: { links: 1, ids: ["a"], remedy: { kind: "guide" } } });
    store.getState().setDrawer(true);
    expect(store.getState().signInPrompt).toBeNull();
    expect(store.getState().drawerOpen).toBe(true);
    expect(store.getState().askedAboutSignIn).toBe(true);
  });

  it.each(["linksOpen", "signInOpen"] as const)("does not raise a completion prompt over %s", (sheet) => {
    const failed = { ...row(link("a")), status: "failed" as const, message: "This site wants a sign-in" };
    store.setState({
      [sheet]: true, phase: "running", batchIds: new Set(["a"]), files: { a: failed },
      cookieBrowsers: [{ recommended: true, id: "firefox", label: "Firefox",
        needs_full_disk_access: false } as BrowserPresence],
    });
    store.getState().handleEvent({ type: "batch_finished", ok: 0, failed: 1, skipped: 0 });
    expect(store.getState().signInPrompt).toBeNull();
    expect(store.getState().installPrompt).toBeNull();
  });

  it.each(["linksOpen", "signInOpen"] as const)("does not raise a helper prompt over %s", (sheet) => {
    store.setState({
      [sheet]: true, phase: "running", batchIds: new Set(["a"]),
      files: { a: { ...row(link("a")), status: "failed", message: "yt-dlp missing" } },
      catalog: { ...MOCK_CATALOG, tools: MOCK_CATALOG.tools.map((tool) =>
        tool.id === "yt-dlp" ? { ...tool, available: false } : tool) },
      installPlans: [{ package_id: "yt-dlp", name: "yt-dlp", tool_ids: ["yt-dlp"] } as
        Awaited<ReturnType<typeof ipc.getInstallPlans>>[number]],
    });
    store.getState().handleEvent({ type: "batch_finished", ok: 0, failed: 1, skipped: 0 });
    expect(store.getState().installPrompt).toBeNull();
  });
});

describe("initialization recovery", () => {
  it("cleans partial listeners and allows a failed initialization to be retried", async () => {
    const unlisten = vi.fn();
    vi.mocked(ipc.onBatchEvent).mockResolvedValue(unlisten);
    vi.mocked(ipc.onInstallEvent).mockRejectedValueOnce(new Error("Listener failed")).mockResolvedValue(vi.fn());
    vi.mocked(ipc.onDragDrop).mockResolvedValue(vi.fn());
    vi.mocked(ipc.onMenuAction).mockResolvedValue(vi.fn());
    vi.mocked(ipc.getCatalog).mockResolvedValue(MOCK_CATALOG);
    vi.mocked(ipc.getSettings).mockResolvedValue(defaults());
    vi.mocked(ipc.getInstallPlans).mockResolvedValue([]);
    vi.mocked(ipc.listCookieBrowsers).mockResolvedValue([]);
    vi.mocked(ipc.getActivity).mockResolvedValue({ converting: false, installing: false });
    await store.getState().init();
    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(store.getState().initializationFailed).toBe(true);
    await store.getState().init();
    expect(store.getState().initializationFailed).toBe(false);
    expect(store.getState().error).toBeNull();
  });
});
