/**
 * Browser mock backend.
 *
 * Loaded **only** through the dynamic `import()` in `./ipc`, which is guarded by
 * `"__TAURI_INTERNALS__" in window` — inside the app this module is never fetched. It exists so
 * `npm run dev` and `vite preview` show a working UI (real catalog data, faked probing, simulated
 * progress) on any machine, including CI and Linux.
 */
import type { Backend, DropEvent, MenuAction, Unlisten } from "./ipc";
import { cropError, cropMediaSettings } from "./crop";
import { EXTENSION_TO_FORMAT, FORMAT_HELPERS, FORMAT_META, MOCK_CATALOG } from "./mock-catalog";
import type {
  BatchEvent,
  BatchItemArg,
  BrowserPresence,
  CatalogView,
  CategoryId,
  CookieBrowser,
  CookieCheck,
  CookieTest,
  FileInfo,
  FormatView,
  InstallEvent,
  InstallManager,
  LinkInspection,
  LinkRow,
  LinkSettings,
  LinkSupport,
  PackageInstallPlan,
  PresetId,
  SafariAccess,
  Settings,
  ToolStatus,
} from "./types";

// ---------------------------------------------------------------------------------------------
// Settings — mirrors `Settings::default()` and `Preset::settings()` in convert-core.
// ---------------------------------------------------------------------------------------------

function defaultSettings(): Settings {
  return {
    preset: "web_and_demo",
    video: {
      codec: "auto",
      quality: "balanced",
      max_height: 1080,
      fps_cap: 60,
      bitrate_kbps: null,
      faststart: true,
      strip_metadata: true,
      hardware_accel: "auto",
    },
    audio: {
      codec: "auto",
      bitrate_kbps: 192,
      sample_rate: null,
      channels: null,
      normalize_loudness: false,
    },
    image: {
      quality: 85,
      max_dimension: 2560,
      strip_metadata: true,
      flatten_background: "#ffffff",
      lossless: false,
      frame_extract_fps: 1,
    },
    gif: { fps: 12, width: 480, optimize_palette: true, loop_count: 0 },
    document: { raster_dpi: 150, first_page_only: false },
    output: {
      location: "subfolder",
      custom_dir: null,
      subfolder_name: "Converted",
      on_conflict: "rename",
      parallel_jobs: 0,
      preserve_timestamps: true,
    },
    // `TrimSettings::default()`: off, and ten seconds ready for the moment it is switched on.
    trim: { enabled: false, start_secs: 0, length_secs: 10 },
    // `LinkSettings::default()`: no sign-in borrowed, and nothing chosen to borrow one from.
    link: { cookies: "none", cookie_browser: "", cookie_file: null },
  };
}

function presetSettings(preset: PresetId): Settings {
  const s = defaultSettings();
  switch (preset) {
    case "web_and_demo":
      break;
    case "smallest":
      s.video.codec = "h265";
      s.video.quality = "small";
      s.video.max_height = 720;
      s.video.fps_cap = 30;
      s.audio.codec = "opus";
      s.audio.bitrate_kbps = 96;
      s.image.quality = 72;
      s.image.max_dimension = 1600;
      s.gif.width = 400;
      s.gif.fps = 10;
      break;
    case "high_quality":
      s.video.quality = "high";
      s.video.max_height = null;
      s.video.fps_cap = null;
      s.audio.bitrate_kbps = 256;
      s.image.quality = 95;
      s.image.max_dimension = null;
      s.image.strip_metadata = false;
      s.gif.width = 720;
      s.gif.fps = 20;
      break;
    case "archive":
      s.video.codec = "pro_res";
      s.video.quality = "max";
      s.video.max_height = null;
      s.video.fps_cap = null;
      s.audio.codec = "flac";
      s.image.quality = 100;
      s.image.max_dimension = null;
      // "Keeps metadata" is what this preset promises, and Rust turns it off for video as well as
      // for images (`Preset::Archive` in convert-core/src/settings.rs).
      s.video.strip_metadata = false;
      s.image.strip_metadata = false;
      s.document.raster_dpi = 300;
      break;
  }
  s.preset = preset;
  return s;
}

/*
 * A preset does not carry a trim — `Preset::settings()` hands back `TrimSettings::default()`, and
 * `commands::apply_preset` copies the *current* trim onto it before it is stored. A preset is an
 * opinion about quality; which ten seconds of a clip the user wants is not one, and losing the cut
 * to a click on "Smallest" would be a setting silently turning itself off.
 *
 * The sign-in a pasted link borrows is carried across for the same reason and in the same place.
 * `LinkSettings` is not an opinion about quality either — it is which account is watching — and
 * "Smallest" signing the user out, so that the next age-restricted link fails where the last one
 * worked, would be the same setting turning itself off with no one told.
 */

// ---------------------------------------------------------------------------------------------
// Preview knobs — how the awkward states get seen
// ---------------------------------------------------------------------------------------------

/**
 * Most files one `inspect_files` may collect — `commands::MAX_INSPECTED_FILES`, to the digit.
 *
 * The number is part of what the backend answers with (`Inspection.limit`), so the copy the user
 * reads names this rather than a constant of its own. `?maxfiles=` lowers it, because the only other
 * way to see a truncated drop is to drop five thousand and one files.
 */
const MAX_INSPECTED_FILES = 5000;

/**
 * Most links one paste may carry — `link::MAX_LINKS_PER_BATCH`, to the digit.
 *
 * Like the file cap, the number is part of what the backend answers with (`LinkInspection.limit`,
 * `LinkSupport.max_links`), so every sentence the user reads names *that* rather than a constant of
 * its own — and `?maxlinks=` is how a suite proves it, by moving the number the backend says and
 * watching the copy move with it.
 */
const MAX_LINKS_PER_BATCH = 20;

/**
 * A helper that installs perfectly on the first click is the only state a mock gets for free, and
 * it is the least interesting one. These query parameters exist so the failure, the
 * "Homebrew is not installed" and the "this may ask for a password" paths can be looked at (and
 * asserted on) in a browser tab:
 *
 *   ?missing=libreoffice,pandoc   which helper *binaries* are absent — ids, so `?missing=pdftohtml`
 *                                 is a half-installed Poppler (`?missing=` → none, all installed)
 *   ?nobrew                       pretend Homebrew itself is not installed: no Install buttons
 *   ?installfail=pandoc           that install fails (`?installfail` alone → every install fails)
 *   ?installpartial=poppler       that install brings only one more of its binaries: the honest
 *                                 "some of it arrived" ending, which one binary cannot have
 *   ?installmissing=pandoc        that install exits 0 and the helper is still nowhere the app looks:
 *                                 `install::Outcome::NotDiscoverable`, the ending whose whole answer
 *                                 is "Use Re-check to look again" (`?installmissing` alone → every
 *                                 install ends that way)
 *   ?installms=60                 milliseconds between streamed log lines (default 320)
 *   ?activity=converting          the state a webview reload lands in: the shell is already busy
 *                                 with work this page never started (`installing`, or `both`)
 *   ?activity=stale               the same question answered a moment too late: the first
 *                                 `get_activity` reports work that has in fact just ended, and
 *                                 every one after it reports the truth
 *   ?maxfiles=4                   lower the enumeration cap `inspect_files` applies, so a drop can
 *                                 be truncated without dropping five thousand files (default 5000,
 *                                 the same number as `commands::MAX_INSPECTED_FILES`)
 *   ?linkms=20                    milliseconds between a pasted link's progress samples (default
 *                                 90). A link is fetched and *then* converted, so its row is the
 *                                 only one with two phases to watch — and at the real tick rate a
 *                                 download takes long enough to be tedious to look at twice
 *   ?maxlinks=3                   lower the link cap `inspect_links` and `get_link_support` report,
 *                                 so the copy that names it can be caught naming a number the
 *                                 backend did not say (default 20, `link::MAX_LINKS_PER_BATCH`).
 *                                 The refusal sentences still quote `LinkError`'s own wording, which
 *                                 in Rust spells the real cap out
 *   ?clipsecs=12                  every clip is exactly this many seconds long — the duration this
 *                                 mock invents per file (and per pasted video) is otherwise a hash
 *                                 of its name, which makes "shorter than the trim" and "the trim
 *                                 starts past the end" a matter of finding the right filename
 *   ?cookies=chrome               the sign-in the session starts in, so the Links group can be
 *                                 looked at in every state without clicking into it: a browser name
 *                                 (any string, so `?cookies=netscape` is the allowlist's refusal),
 *                                 `browser` or `file` for the two states held in silence,
 *                                 `file:/Users/you/Desktop/cookies.txt` for a chosen file, and
 *                                 `none` or nothing for `LinkSettings::default()`. A path is judged
 *                                 by its shape (see [`checkCookieFile`]), so `file:cookies.txt`,
 *                                 `file:/Users/you/Desktop` and `file:/tmp/missing-cookies.txt` are
 *                                 the other three refusals
 *   ?linkfail=jsruntime           every YouTube link fails the way a Mac with no JavaScript runtime
 *                                 does (`FetchFailure::NoJsRuntime`), which is the failure that used
 *                                 to read as a sign-in wall. Neither this nor the next one can be
 *                                 reached honestly by a mock — one needs a Mac without a runtime and
 *                                 the other a Mac without a permission — and both are the whole
 *                                 reason an affordance exists, so both are a knob. A Bilibili link
 *                                 still converts: YouTube's challenges are YouTube's. Deno and Node
 *                                 are absent in the default fixture, so the failed row resolves to
 *                                 the Deno helper by itself: microlink, question, Settings
 *   ?linkfail=safari              every link fails the way Safari's cookie jar does without Full Disk
 *                                 Access (`FetchFailure::SafariNeedsFullDiskAccess`), whatever
 *                                 `?cookies=` says — the row then offers the trip to System Settings
 *                                 that only that failure gets
 *   ?linkfail=signin              every YouTube link fails on the sign-in — and *stops* failing the
 *                                 moment a sign-in this machine can actually read is configured.
 *                                 Which of the three sign-in endings it is comes from the settings
 *                                 rather than from the knob (see [`signInState`]): nothing chosen is
 *                                 `LoginRequired`, a browser this machine does not have (or Safari,
 *                                 which needs a permission no browser tab can grant) is the jar that
 *                                 never opened, and anything readable simply works. That is the
 *                                 whole recovery in one knob: wall → question → borrow Chrome →
 *                                 check → automatic retry → converted
 *   ?linkfail=refused             every YouTube link fails with `FetchFailure::CookiesNotAccepted` —
 *                                 the jar opened, the cookies went out, and the site said no anyway.
 *                                 Unlike the one above this does not recover, because a stale
 *                                 account is not something the app can fix from here
 *   ?browsers=chrome,safari       which allowlisted browsers this machine has, as
 *                                 `list_cookie_browsers` reports them (ids, allowlist order kept).
 *                                 The default is Chrome and Safari — `?browsers=safari` is the
 *                                 machine where the only browser costs a permission, and
 *                                 `?browsers=` (empty) is the machine with no allowlisted browser at
 *                                 all, where the only honest offer is a cookies.txt
 *   ?mac=safari                   *whose* Mac this is — which browser macOS opens links with, and
 *                                 what `stat` says about each cookie store. `?browsers=` decides
 *                                 what is installed; this decides which of them the sign-in is
 *                                 actually in, which is what `settings::rank_browsers` orders by
 *                                 and what the app now offers. Three machines, because three is
 *                                 what the bug needed: `chrome` (the default) is Chrome-primary —
 *                                 Chrome the default browser with a jar written minutes ago and a
 *                                 Safari nobody has used in weeks, which is the one-click path;
 *                                 `safari` is the Mac the complaint came from — Safari the default,
 *                                 its jar 346 KB and written minutes ago, while Chrome's is an
 *                                 empty 65,536 bytes untouched for two days, so the browser worth
 *                                 borrowing from is the one that costs a permission; and `nostore`
 *                                 is the Mac where the browser the app would have picked has no
 *                                 cookie store on disk at all, so there is nothing to offer and the
 *                                 app must say so instead of borrowing an empty jar
 *   ?fda                          this pretend app holds Full Disk Access, so Safari's jar opens.
 *                                 Without it `check_safari_cookie_access` answers
 *                                 `needs_full_disk_access`, which is what that user's Mac says and
 *                                 the only state a browser tab could not otherwise reach
 *   ?signin=refused               force what `test_cookie_source` answers, whatever the settings
 *                                 say: `working`, `unreadable`, `refused`, `inconclusive` or
 *                                 `not_configured`. Without it the check is derived honestly from
 *                                 the configured source and `?browsers=`, which is what lets the
 *                                 fix-and-retry path actually succeed
 */
interface Knobs {
  /** Helper *binary* ids to treat as missing. `null` keeps whatever the generated catalog recorded. */
  missing: string[] | null;
  noManager: boolean;
  failing: string[];
  /** Package ids whose install lands only part of the package — see [`runInstall`]. */
  partial: string[];
  /** Package ids whose install lands nothing the app can find afterwards — see [`runInstall`]. */
  undiscoverable: string[];
  lineMs: number;
  /** Work the shell is already doing at page load — see [`startInheritedWork`]. */
  inherited: { converting: boolean; installing: boolean };
  /** Answer the first `get_activity` with work that has already finished. */
  staleActivity: boolean;
  /** Most files one `inspect_files` may collect — [`MAX_INSPECTED_FILES`]. */
  maxFiles: number;
  /** Milliseconds between a link row's download/convert samples — see [`runLink`]. */
  linkMs: number;
  /** Most links one paste may carry — [`MAX_LINKS_PER_BATCH`]. */
  maxLinks: number;
  /** The duration to give every clip, or null to keep the per-name hash — see [`clipSeconds`]. */
  clipSecs: number | null;
  /** Names whose length the probe cannot read, or `["all"]` for every clip — see [`fakeProbe`]. */
  noLength: string[];
  /** The sign-in the session starts in — see [`cookieKnob`]. */
  cookies: LinkSettings;
  /** The fetch failure every link row is to end in, or null for a fetch that works — see [`runLink`]. */
  linkFail: LinkFailKnob;
  /** Browser ids this machine has, or null for the measured default — see [`browsersKnob`]. */
  browsers: string[] | null;
  /** Which Mac this is: whose browser is default, and what is in each jar — see [`MACHINES`]. */
  machine: MachineKnob;
  /** Does this pretend app hold Full Disk Access? Safari's jar opens only when it does. */
  fullDiskAccess: boolean;
  /** The verdict `test_cookie_source` is to give whatever the settings say, or null to derive it. */
  signIn: CookieCheck | null;
}

/**
 * `?linkfail=` — which of `FetchFailure`'s actionable endings a run produces.
 *
 * Deliberately not "which stderr does yt-dlp print": the mock has no yt-dlp, and what the UI is
 * developed against is the classified failure, which is what a row is handed.
 */
type LinkFailKnob = "jsruntime" | "safari" | "signin" | "refused" | null;

const LINK_FAILS: ReadonlyArray<LinkFailKnob> = ["jsruntime", "safari", "signin", "refused"];

const linkFailKnob = (raw: string | null): LinkFailKnob =>
  LINK_FAILS.find((known) => known === raw) ?? null;

/**
 * `?nolength=` — the clips `ffprobe` cannot measure.
 *
 * Not every file has a duration to read: a stream captured to disk, a container written by a tool
 * that never filled the header in, a growing recording. `Engine::run` is handed `None` for those and
 * reports `fraction: null` for every sample rather than the `Some(0.0)` it used to invent, so the
 * window must show a job it cannot measure without claiming it is at 0% — and that treatment is only
 * reachable in a preview if the mock can produce such a file. Bare `?nolength` is every clip; a
 * comma-separated list names the ones to leave unmeasurable, so a measured file can be queued beside
 * one as the control.
 */
const NO_LENGTH_ALL = "all";

const lengthUnknown = (name: string): boolean =>
  KNOBS.noLength.includes(NO_LENGTH_ALL) ||
  KNOBS.noLength.some((wanted) => wanted.toLowerCase() === name.toLowerCase());

/**
 * `?browsers=` — the allowlisted browsers this machine is to have.
 *
 * Three machines matter and only one of them can be the default, so all three are reachable: the
 * measured Mac (Chrome and Safari, where borrowing Chrome is one click), the Safari-only Mac (where
 * one click cannot work and the honest offer is the permission), and the Mac with nothing
 * allowlisted (where the only route left is an exported cookies.txt). `?browsers=` with nothing
 * after it is the last of those — an *empty list*, which is not the same as the knob being absent.
 */
const browsersKnob = (raw: string | null): string[] | null =>
  raw === null
    ? null
    : raw
        .split(",")
        .map((part) => part.trim().toLowerCase())
        .filter((part) => part !== "");

/**
 * `?mac=` — which Mac this preview is pretending to be.
 *
 * `?browsers=` says what is *installed*; this says where the sign-in actually is, which is the
 * question the app now answers and the question it used to get wrong. The three values are the
 * three machines the bug was about, and they are named after what is true of them rather than after
 * a browser, because "the Chrome one" and "the Safari one" stop meaning anything the moment the
 * ranking is what is under test.
 */
type MachineKnob = "chrome" | "safari" | "nostore";

const MACHINE_KNOBS: ReadonlyArray<MachineKnob> = ["chrome", "safari", "nostore"];

const machineKnob = (raw: string | null): MachineKnob =>
  MACHINE_KNOBS.find((known) => known === raw) ?? "chrome";

/** `?signin=` — one of `CookieCheck`'s five, or null to let the settings decide. */
const COOKIE_CHECKS: ReadonlyArray<CookieCheck> = [
  "working",
  "unreadable",
  "refused",
  "inconclusive",
  "not_configured",
];

const signInKnob = (raw: string | null): CookieCheck | null =>
  COOKIE_CHECKS.find((known) => known === raw) ?? null;

/**
 * `?cookies=` read into the settings group it stands for.
 *
 * The two unfinished states are the reason this knob exists: both are *entered* by choosing a mode
 * and then not finishing the sentence, both are held in silence, and neither leaves a trace anywhere
 * a suite can see — so a page that starts in one is the only way to look at the hint that explains
 * it. A bare browser name is passed through untouched rather than checked here, because the check
 * belongs to [`classifyLink`] and a knob that quietly corrected `netscape` would hide the refusal
 * this preview exists to show.
 */
function cookieKnob(raw: string | null): LinkSettings {
  const none: LinkSettings = { cookies: "none", cookie_browser: "", cookie_file: null };
  const value = (raw ?? "").trim();
  if (value === "" || value === "none") return none;
  if (value === "browser") return { ...none, cookies: "browser" };
  if (value === "file") return { ...none, cookies: "file" };
  const prefix = "file:";
  if (value.startsWith(prefix)) {
    return { ...none, cookies: "file", cookie_file: value.slice(prefix.length) };
  }
  return { ...none, cookies: "browser", cookie_browser: value };
}

function readKnobs(): Knobs {
  const search = typeof window === "undefined" ? "" : window.location.search;
  const params = new URLSearchParams(search);
  const list = (raw: string): string[] =>
    raw
      .split(",")
      .map((part) => part.trim())
      .filter((part) => part !== "");
  const missing = params.get("missing");
  const failing = params.get("installfail");
  const partial = params.get("installpartial");
  const undiscoverable = params.get("installmissing");
  const lineMs = Number(params.get("installms"));
  const activity = params.get("activity") ?? "";
  const maxFiles = Number(params.get("maxfiles"));
  const linkMs = Number(params.get("linkms"));
  const maxLinks = Number(params.get("maxlinks"));
  const clipSecs = Number(params.get("clipsecs"));
  const noLength = params.get("nolength");
  return {
    missing: missing === null ? null : list(missing),
    noManager: params.has("nobrew"),
    failing: failing === null ? [] : failing === "" ? ["all"] : list(failing),
    partial: partial === null ? [] : partial === "" ? ["all"] : list(partial),
    undiscoverable:
      undiscoverable === null ? [] : undiscoverable === "" ? ["all"] : list(undiscoverable),
    lineMs: Number.isFinite(lineMs) && lineMs > 0 ? lineMs : 320,
    inherited: {
      converting: activity === "converting" || activity === "both",
      installing: activity === "installing" || activity === "both",
    },
    staleActivity: activity === "stale",
    maxFiles:
      Number.isFinite(maxFiles) && maxFiles > 0 ? Math.floor(maxFiles) : MAX_INSPECTED_FILES,
    linkMs: Number.isFinite(linkMs) && linkMs > 0 ? linkMs : 90,
    maxLinks:
      Number.isFinite(maxLinks) && maxLinks > 0 ? Math.floor(maxLinks) : MAX_LINKS_PER_BATCH,
    clipSecs: Number.isFinite(clipSecs) && clipSecs > 0 ? clipSecs : null,
    noLength: noLength === null ? [] : noLength === "" ? [NO_LENGTH_ALL] : list(noLength),
    cookies: cookieKnob(params.get("cookies")),
    linkFail: linkFailKnob(params.get("linkfail")),
    browsers: browsersKnob(params.get("browsers")),
    machine: machineKnob(params.get("mac")),
    fullDiskAccess: params.has("fda"),
    signIn: signInKnob(params.get("signin")),
  };
}

const KNOBS = readKnobs();

// ---------------------------------------------------------------------------------------------
// Packages and binaries — mirrors `convert_core::package`
// ---------------------------------------------------------------------------------------------

/**
 * What a *user* installs, as `package.rs` declares it: an id, a display name, and the binaries the
 * one formula provides.
 *
 * Restated here for the same reason [`CATEGORY_PLURAL`] is: the generated catalog carries the Rust
 * *format* table and nothing else, and the preview needs the package table to answer
 * `get_install_plans` the way the real backend does. Bundled FFmpeg/ffprobe and the built-in `sips`
 * are deliberately absent — there is nothing to install, so they are never offered.
 */
const PACKAGES: ReadonlyArray<{ id: string; name: string; tool_ids: readonly string[] }> = [
  { id: "libreoffice", name: "LibreOffice", tool_ids: ["libreoffice"] },
  { id: "pandoc", name: "Pandoc", tool_ids: ["pandoc"] },
  // The formula is `imagemagick`, the binary is `magick`: the user installs the former.
  { id: "imagemagick", name: "ImageMagick", tool_ids: ["magick"] },
  // Three binaries, one formula, one row.
  { id: "poppler", name: "Poppler", tool_ids: ["pdftoppm", "pdftotext", "pdftohtml"] },
  { id: "ruffle", name: "Ruffle", tool_ids: ["ruffle"] },
  // The one package that unlocks a *source* rather than a format: pasted YouTube and Bilibili links.
  { id: "yt-dlp", name: "yt-dlp", tool_ids: ["yt-dlp"] },
  // The runtime that package needs before YouTube will hand a video over — the second row here that
  // unlocks no format at all. `node` is deliberately *not* a member: it is a runtime this app will
  // happily use if the machine already has it, and installing a JavaScript toolchain on somebody's
  // behalf is not a converter's business — so it has no package here, exactly as in `package.rs`,
  // and therefore no plan, no command and no Install button anywhere in the UI.
  { id: "deno", name: "Deno", tool_ids: ["deno"] },
];

const TOOL_BY_ID: ReadonlyMap<string, ToolStatus> = new Map(
  MOCK_CATALOG.tools.map((tool) => [tool.id, tool]),
);

/**
 * Which way a format is being used. Support differs per direction — PDF reads with six helpers and
 * writes with one — so every availability question has to say which one it is asking about.
 */
type Direction = "read" | "write";

/**
 * The binaries a format's support list names for this direction, as ids, in the planner's order of
 * preference (`FORMAT_HELPERS`, generated from `Support::AnyOf`).
 *
 * `FormatView.needs` is deliberately *not* this: `catalog_view` collapses a support list into deduped
 * package names before the frontend ever sees it, so "Poppler" arrives once for three binaries. The
 * ids stay here, because "can this machine do it?" and "what would the planner spawn?" are questions
 * about binaries, and only the name is a question about a person.
 */
const helperIds = (formatId: string, direction: Direction): string[] =>
  FORMAT_HELPERS[formatId]?.[direction] ?? [];

const packageOf = (toolId: string): (typeof PACKAGES)[number] | undefined =>
  PACKAGES.find((pkg) => pkg.tool_ids.includes(toolId));

const installHintFor = (toolId: string): string => TOOL_BY_ID.get(toolId)?.install_hint ?? "";

/**
 * `Tool::user_facing_name`: the package's name, or — for a binary nothing installs — its own label.
 *
 * Every string a person reads about a missing helper comes through here. It is what stops a planner
 * refusal or a picker option from naming `pdftohtml`, which nobody recognises and nobody installs.
 */
const userFacingName = (toolId: string): string =>
  packageOf(toolId)?.name ?? TOOL_BY_ID.get(toolId)?.label ?? toolId;

/** Package names for a support list, each named once — `catalog_view`'s `needs`. */
function packageNames(toolIds: string[]): string[] {
  const names: string[] = [];
  for (const name of toolIds.map(userFacingName)) {
    if (!names.includes(name)) names.push(name);
  }
  return names;
}

/**
 * Which helper binaries are absent right now. Module-level because a page load has exactly one mock
 * backend, and because a *successful* install has to change the answer — the whole point of
 * re-checking after an install is that a format stops being greyed out.
 */
const missingTools = new Set<string>(
  KNOBS.missing ?? MOCK_CATALOG.tools.filter((tool) => !tool.available).map((tool) => tool.id),
);

const toolStatuses = (): ToolStatus[] =>
  MOCK_CATALOG.tools.map((tool) => {
    const available = !missingTools.has(tool.id);
    return {
      ...tool,
      available,
      // A helper that was just installed has to have a plausible path, or "Found" would have
      // nothing to point at.
      path: available ? (tool.path ?? `/opt/homebrew/bin/${tool.id}`) : null,
    };
  });

/**
 * A format is available when it needs nothing, or when any one of its helper *binaries* is present.
 *
 * `needs` is left exactly as generated: it is already the deduped package names a reader would
 * install, the same collapse `lib.rs::format_view` does, so nothing above this line can see a
 * binary's diagnostic label.
 */
const withAvailability = (format: FormatView, direction: Direction): FormatView => {
  const helpers = helperIds(format.id, direction);
  return {
    ...format,
    available:
      helpers.length === 0 ? format.available : helpers.some((id) => !missingTools.has(id)),
  };
};

/**
 * The catalog as the real backend would rebuild it: format availability derived from the helpers
 * that are actually there, not the ones the generator happened to find.
 */
const catalogView = (): CatalogView => ({
  ...MOCK_CATALOG,
  tools: toolStatuses(),
  categories: MOCK_CATALOG.categories.map((category) => ({
    ...category,
    inputs: category.inputs.map((format) => withAvailability(format, "read")),
    outputs: category.outputs.map((format) => withAvailability(format, "write")),
  })),
});

// ---------------------------------------------------------------------------------------------
// Install plans — mirrors `convert_core::install::install_plans`
// ---------------------------------------------------------------------------------------------

/** How Rust says each category out loud (`install::plural`), which is not how the UI pluralises. */
const CATEGORY_PLURAL: Readonly<Record<CategoryId, string>> = {
  video: "video files",
  audio: "audio files",
  image: "images",
  document: "documents",
  subtitle: "subtitles",
  flash: "Flash movies",
};

const join = (parts: string[], conjunction: string): string =>
  parts.length <= 1
    ? (parts[0] ?? "")
    : `${parts.slice(0, -1).join(", ")} ${conjunction} ${parts[parts.length - 1] ?? ""}`;

/** Catalog names, capped: the settings page is not the format reference. */
function listing(formats: FormatView[]): string {
  const names = formats.slice(0, 6).map((f) => f.name);
  const rest = formats.length - names.length;
  return rest === 0 ? join(names, "and") : `${names.join(", ")} and ${rest} more`;
}

/**
 * What a *package* buys the user, derived from the catalog exactly as `install::unlocks_for` does —
 * same sentence shapes, same 6-name cap, same "some of these also work with…" honesty about overlaps.
 *
 * One derivation over every member, not three finished lists stacked: Poppler's three binaries all
 * read PDF, and "Open documents: PDF" three times is not a list, it is a stutter.
 */
function unlocksFor(toolIds: readonly string[]): string[] {
  // yt-dlp unlocks a *source*, not a format: nothing in the catalog names it, so the derivation
  // below would promise nothing at all. `install::unlocks_for` states it by hand for the same
  // reason, and from the same one fact — the link cap.
  if (toolIds.includes(YT_DLP_TOOL)) {
    return [
      `Convert YouTube and Bilibili video links, or QQ Music, NetEase Music, SoundCloud ` +
        `and Bandcamp tracks (up to ${KNOBS.maxLinks} links at a time).`,
    ];
  }
  // A JavaScript runtime unlocks no format either, and for the same reason cannot be derived: what
  // it buys is that a YouTube *fetch* keeps working. Word for word `install::unlocks_for`, and asked
  // after yt-dlp there as here, because a set holding both is being asked about links.
  if (toolIds.some((id) => JS_RUNTIME_TOOLS.includes(id))) {
    return [
      "Keeps YouTube links working: yt-dlp needs a JavaScript runtime for YouTube's challenges, " +
        "and without one YouTube asks for a sign-in instead of handing the video over.",
    ];
  }
  const declares = (format: FormatView, direction: Direction): boolean =>
    helperIds(format.id, direction).some((id) => toolIds.includes(id));
  const out: string[] = [];
  for (const category of MOCK_CATALOG.categories) {
    const reads = category.inputs.filter((f) => declares(f, "read"));
    const writes = category.outputs.filter((f) => declares(f, "write"));
    if (reads.length === 0 && writes.length === 0) continue;
    const subject = CATEGORY_PLURAL[category.id];
    const same = reads.length === writes.length && reads.every((f, i) => writes[i]?.id === f.id);
    if (same) {
      out.push(`Open and save ${subject}: ${listing(reads)}`);
    } else {
      if (reads.length > 0) out.push(`Open ${subject}: ${listing(reads)}`);
      if (writes.length > 0) out.push(`Save ${subject}: ${listing(writes)}`);
    }
  }
  // Honest about alternatives, and named as a user would install them: two of Poppler's binaries
  // sitting side by side in one support list are one word, not two.
  const shared = packageNames(
    MOCK_CATALOG.tools
      .filter(
        (other) =>
          !toolIds.includes(other.id) &&
          MOCK_CATALOG.categories.some((c) =>
            [
              ...c.inputs.map((f) => helperIds(f.id, "read")),
              ...c.outputs.map((f) => helperIds(f.id, "write")),
            ].some(
              (helpers) =>
                helpers.some((id) => toolIds.includes(id)) && helpers.includes(other.id),
            ),
          ),
      )
      .map((other) => other.id),
  );
  if (shared.length > 0) out.push(`Some of these also work with ${join(shared, "or")}`);
  return out;
}

/**
 * One plan per *package*, in package-table order — `install::install_plans`.
 *
 * The command *is* the install hint, which is the same invariant the Rust tests enforce
 * (`the_command_we_run_is_the_command_we_show`); a hint that is prose rather than a command line
 * means there is nothing to run and therefore no manager. Nothing here says whether the package is
 * present: that is the tool list's job, joined on `tool_ids`.
 */
function installPlans(): PackageInstallPlan[] {
  return PACKAGES.map((pkg) => {
    const hint = installHintFor(pkg.tool_ids[0] ?? "");
    const command = hint.startsWith("brew ") ? hint : "";
    const manager: InstallManager = command === "" ? "none" : "homebrew";
    const managerAvailable = manager === "homebrew" && !KNOBS.noManager;
    return {
      package_id: pkg.id,
      name: pkg.name,
      tool_ids: [...pkg.tool_ids],
      manager,
      manager_available: managerAvailable,
      command,
      // A cask lands in /Applications and can ask for a password; a formula never does.
      needs_admin: command.includes("--cask"),
      can_auto_install: managerAvailable,
      unlocks: unlocksFor(pkg.tool_ids),
    };
  });
}

/** Homebrew's own output, near enough that the log pane is the size it will really be. */
function installTranscript(plan: PackageInstallPlan): string[] {
  const pkg = plan.command.split(" ").pop() ?? plan.package_id;
  if (plan.needs_admin) {
    return [
      `==> Downloading https://download.documentfoundation.org/${pkg}/stable/7.6.4/mac/x86_64/${pkg}_7.6.4_MacOS_x86-64.dmg`,
      "==> Downloading from https://mirror.init7.net/tdf/libreoffice/stable/7.6.4",
      "######################################################################## 100.0%",
      `==> Installing Cask ${pkg}`,
      "==> Verifying checksum for Cask",
      `==> Moving App '${pkg}.app' to '/Applications/${pkg}.app'`,
      "==> Linking Binary 'soffice' to '/opt/homebrew/bin/soffice'",
      "==> Purging files for version 7.6.4",
      `🍺  ${pkg} was successfully installed!`,
    ];
  }
  return [
    `==> Fetching ${pkg}`,
    `==> Downloading https://ghcr.io/v2/homebrew/core/${pkg}/manifests/3.1.13`,
    "######################################################################## 100.0%",
    `==> Downloading https://ghcr.io/v2/homebrew/core/${pkg}/blobs/sha256:6a4f9c1e2b`,
    "######################################################################## 100.0%",
    `==> Pouring ${pkg}--3.1.13.arm64_sonoma.bottle.tar.gz`,
    "==> Caveats",
    "Completion has been installed to /opt/homebrew/etc/bash_completion.d",
    "==> Summary",
    `🍺  /opt/homebrew/Cellar/${pkg}/3.1.13: 12 files, 180.4MB`,
    `==> Running \`brew cleanup ${pkg}\`...`,
    "Disable this behaviour by setting HOMEBREW_NO_INSTALL_CLEANUP.",
  ];
}

/** How an install ended, before it is turned into words — `src-tauri/src/install.rs::Outcome`. */
type InstallOutcome = "installed" | "notDiscoverable" | "incomplete" | "failed";

/**
 * Word for word the wording of `src-tauri/src/install.rs::message_for`, all four endings this mock
 * can reach. The name in every sentence is the *package's*: it is what the user clicked Install on.
 *
 * Three of these changed when the installer's stale-snapshot bug was fixed, and the change was not
 * only a rewording: a message that sent somebody to Terminal to audit an install *this app* had just
 * run got them "Warning: already installed" and nothing else. Every failure now ends in an affordance
 * that is really on screen beside it — "Re-check" in the Helper apps header, "Show log" under the
 * message — so a paraphrase here would be the preview naming a button the app does not have.
 */
function installEnding(
  plan: PackageInstallPlan,
  outcome: InstallOutcome,
  found: number,
): { lines: string[]; message: string } {
  if (outcome === "installed") {
    return {
      lines: [],
      message: `${plan.name} is installed. Flint can use it now.`,
    };
  }
  // Exit 0 and nothing where the app looks: a Homebrew that landed in a prefix this build does not
  // know, a formula that installed under another name. The app is the one that has to look again,
  // so the app's own button is the answer rather than a command line.
  if (outcome === "notDiscoverable") {
    return {
      lines: [],
      message:
        `The installer reported success, but ${plan.name} is not in any of the places ` +
        `Flint looks. Use Re-check to look again.`,
    };
  }
  // Exit 0, and only some of what the formula promised is where the app looks. Not a success (the
  // routes needing the missing binary still cannot run) and not a failure either — and which
  // binary is missing belongs in a log, never in this sentence.
  if (outcome === "incomplete") {
    return {
      lines: [],
      message:
        `The installer finished, but Flint can only find part of ${plan.name} ` +
        `(${found} of its ${plan.tool_ids.length} programs), so some conversions still will not ` +
        `run. Use Re-check, then install ${plan.name} again if it is still incomplete.`,
    };
  }
  // A cask that has to write outside the home directory shells out to sudo, and there is no
  // terminal behind this process: that is a different problem, and it gets a different sentence.
  if (plan.needs_admin) {
    return {
      lines: ["sudo: no tty present and no askpass program specified", "Error: Failure while executing"],
      message:
        `${plan.name} has to be installed with an administrator password, and there is nowhere ` +
        `to type one here. Open Terminal and run \`${plan.command}\` - it will ask for your ` +
        `password, and then this page will find ${plan.name}.`,
    };
  }
  const detail = `Error: No available formula with the name "${plan.command.split(" ").pop() ?? ""}"`;
  return {
    lines: [detail, "Please tap Homebrew/core and try again."],
    message:
      `Could not install ${plan.name} (the installer exited with 1). Last message: ${detail} ` +
      `Show log for everything it wrote, or run \`${plan.command}\` in Terminal to install it by ` +
      `hand.`,
  };
}

// ---------------------------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------------------------

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

/** Stable pseudo-random in [0,1) from a string, so the same file always looks the same. */
function hashUnit(seed: string): number {
  let h = 2166136261;
  for (let i = 0; i < seed.length; i += 1) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return ((h >>> 0) % 100000) / 100000;
}

const extensionOf = (name: string): string => {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
};

const baseName = (path: string): string => path.split(/[\\/]/).pop() ?? path;
const parentOf = (path: string): string => path.slice(0, path.length - baseName(path).length - 1);
const stem = (name: string): string => {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
};

/**
 * `probe::seconds_label`: `0:12`, `2:02:05` — truncated, because Rust truncates.
 *
 * This rounded until the trim gave the number a second reader. A row said `0:13` for a file of 12.6
 * seconds where the app says `0:12`, which was a lie nobody could see; then `trim_past_the_end`
 * started quoting the same length back in a refusal, and the mock's sentence stopped being the
 * backend's sentence over four tenths of a second. One spelling, and it is Rust's.
 *
 * `format.ts` has the same six lines in [`secondsLabel`], and that duplication is kept on purpose:
 * this module stands in for the *backend*, and it imports nothing from the app (only `./types` and
 * the generated catalog). Calling the app's helper here would make the two agree by construction —
 * an app that started rounding would silently take the fake backend with it, and the drift between
 * the two sides that this comment's own bug story is about could never be observed again. They are
 * two independent spellings of `probe::seconds_label`, and the suite asserts they still agree on
 * the one input where truncation and rounding differ (`?clipsecs=12.6`, assertions 322 and 402).
 */
function durationLabel(secs: number): string {
  const total = Math.max(0, Math.trunc(secs));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number): string => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

const RESOLUTIONS: ReadonlyArray<readonly [number, number]> = [
  [1920, 1080],
  [3840, 2160],
  [1280, 720],
  [2560, 1440],
  [1170, 2532],
];

const outputExtensionFor = (targetId: string): string => {
  for (const category of MOCK_CATALOG.categories) {
    const found = category.outputs.find((f) => f.id === targetId);
    if (found) return found.extension;
  }
  return targetId;
};

/**
 * The input format a path resolves to, with its helper availability *now* — a helper installed
 * during the session (or knobbed out of existence) has to change the answer, or the preview would
 * keep failing a conversion the app can suddenly do.
 */
const sourceFormat = (
  path: string,
): { id: string; available: boolean; needs: string[]; helpers: string[] } | null => {
  const formatId = EXTENSION_TO_FORMAT[extensionOf(baseName(path))];
  if (formatId === undefined) return null;
  for (const category of MOCK_CATALOG.categories) {
    const found = category.inputs.find((f) => f.id === formatId);
    if (found) {
      const view = withAvailability(found, "read");
      return {
        id: view.id,
        available: view.available,
        needs: view.needs,
        helpers: helperIds(view.id, "read"),
      };
    }
  }
  return null;
};

const targetFormat = (
  targetId: string,
): { name: string; available: boolean; needs: string[]; helpers: string[] } => {
  for (const category of MOCK_CATALOG.categories) {
    const found = category.outputs.find((f) => f.id === targetId);
    if (found) {
      const view = withAvailability(found, "write");
      return {
        name: view.name,
        available: view.available,
        needs: view.needs,
        helpers: helperIds(view.id, "write"),
      };
    }
  }
  return { name: targetId, available: true, needs: [], helpers: [] };
};

/**
 * Word for word what `plan::PlanError::MissingTool` renders: `"{format} needs {tool} installed
 * ({hint})"`, using the first candidate helper — and, like `plan::need`, naming that helper the way
 * the user would install it: "pdf needs Poppler installed", never "pdftohtml". The preview is the
 * only place these long strings get seen before a real user hits them, so they must be the real
 * length, not a friendly stub.
 */
const missingToolMessage = (formatId: string, helpers: string[], needs: string[]): string => {
  const first = helpers[0];
  if (first === undefined) return `${formatId} needs ${needs[0] ?? "a helper app"} installed ()`;
  return `${formatId} needs ${userFacingName(first)} installed (${installHintFor(first)})`;
};

/**
 * `plan::pdf_text_tool`: the Poppler binary that reads a PDF's *text* into this target.
 *
 * A pair, not a format. `pdf` is readable as long as any one of its six helpers is there, so the
 * format-level checks above say yes on a machine that has `pdftoppm` and nothing else — while
 * `Route::PdfText` needs the binary for *this* target (or LibreOffice, its 800 MB fallback). Without
 * this the preview converts a PDF → HTML the real backend refuses, and the suite believes it.
 */
const PDF_TEXT_TOOL: Readonly<Record<string, string>> = { txt: "pdftotext", html: "pdftohtml" };

/** `Route::PdfText`'s candidates, in the planner's order, or null when this is not that route. */
const pdfTextCandidates = (sourceId: string | null, targetId: string): string[] | null => {
  const poppler = PDF_TEXT_TOOL[targetId];
  if (sourceId !== "pdf" || poppler === undefined) return null;
  return [poppler, "libreoffice"];
};

/**
 * `plan::need`: the first candidate is what the refusal names, whichever one is missing — and it is
 * named as a package, so a failed PDF → HTML asks for Poppler rather than for `pdftohtml`.
 */
const missingRouteMessage = (formatId: string, candidates: string[]): string => {
  const first = candidates[0] ?? "";
  return `${formatId} needs ${userFacingName(first)} installed (${installHintFor(first)})`;
};

/**
 * `settings_store::check_output`, refusal for refusal.
 *
 * Three real commands run this before they do anything — `save_settings`, `start_batch` and
 * `estimate_output_path` — so a mock that accepted every output configuration made the preview (and
 * the suite behind it) believe in a queue the app would refuse to convert: "Custom folder" with no
 * folder chosen writes nothing in Rust and wrote `/Users/you/Desktop//clip.mp4` here.
 *
 * The one honest difference: a browser has no filesystem, so "that folder no longer exists" cannot
 * be checked and an absolute path is taken at its word.
 */
function checkOutput(out: Settings["output"]): string | null {
  if (out.location === "same_folder") return null;
  if (out.location === "subfolder") {
    const name = out.subfolder_name;
    if (name.trim() === "") return "The output subfolder needs a name.";
    if (name.startsWith("/")) return "The output subfolder must be a name, not a full path.";
    // Rust normalises `//` and an inner `.` away, so only `..` and a leading `.` are refused.
    const parts = name.split("/").filter((part) => part !== "");
    if (parts.includes("..") || parts[0] === ".")
      return "The output subfolder must stay inside the folder the files came from.";
    return null;
  }
  const dir = out.custom_dir;
  if (dir === null || dir.trim() === "") return "Choose an output folder, or switch back to a subfolder.";
  if (!dir.startsWith("/")) return "The output folder must be a full path.";
  if (dir.split("/").includes("..")) return "The output folder must not contain `..`.";
  return null;
}

/** Is this a format id the catalog knows? `commands::jobs_from` refuses anything else. */
const knownFormat = (id: string): boolean =>
  MOCK_CATALOG.categories.some(
    (c) => c.inputs.some((f) => f.id === id) || c.outputs.some((f) => f.id === id),
  );

// ---------------------------------------------------------------------------------------------
// The trim — mirrors `settings::TrimSettings`, `settings_store::classify_trim` and `plan`
// ---------------------------------------------------------------------------------------------

/** How long a clip is pretended to be, unless `?clipsecs=` has fixed it for the whole session. */
const clipSeconds = (secs: number): number => KNOBS.clipSecs ?? secs;

/**
 * What `engine.probe` would have said about a path: its category, whether it moves, how long it is.
 *
 * The same three answers [`describe`] puts in the row, derived in one place because the *run* needs
 * them too — the trim's refusal and the length the fake progress is measured against are about the
 * duration this mock invented when the file was dropped, and a second guess at it would let the row
 * and the batch disagree about how long the same file is.
 */
function fakeProbe(path: string): {
  category: CategoryId | null;
  animated: boolean;
  secs: number | null;
} {
  const name = baseName(path);
  const formatId = EXTENSION_TO_FORMAT[extensionOf(name)];
  const meta = formatId === undefined ? undefined : FORMAT_META[formatId];
  if (formatId === undefined || meta === undefined) {
    return { category: null, animated: false, secs: null };
  }
  const category = meta.category as CategoryId;
  const moving = category === "video" || category === "audio" || category === "flash";
  return {
    category,
    animated: (moving && category !== "audio") || formatId === "gif",
    // A moving file whose header never said how long it is: `probe` answers `None`, the row shows no
    // duration, and the run that follows has no percentage to report — see [`lengthUnknown`].
    secs: moving && !lengthUnknown(name) ? clipSeconds(8 + hashUnit(name) * 900) : null,
  };
}

/** `settings::MAX_TRIM_SECS`. */
const MAX_TRIM_SECS = 24 * 60 * 60;

/** `settings::sane_secs`. */
const saneSecs = (value: number): number =>
  Number.isFinite(value) ? Math.min(MAX_TRIM_SECS, Math.max(0, value)) : 0;

/** `TrimSettings::effective`: the pair to act on, or null when this trim asks for nothing. */
function effectiveTrim(trim: Settings["trim"]): { start: number; length: number } | null {
  if (!trim.enabled) return null;
  const length = saneSecs(trim.length_secs);
  return length > 0 ? { start: saneSecs(trim.start_secs), length } : null;
}

/**
 * `TrimSettings::expected_output_secs`: how long the file the user gets will be.
 *
 * The number the fake progress and ETA are measured against, for the same reason the real ones are:
 * a 10 second cut of a 15 minute film measured against the film sits at 1% and then finishes.
 */
function expectedOutputSecs(
  trim: Settings["trim"],
  sourceSecs: number | null,
  timeBased: boolean,
): number | null {
  if (sourceSecs === null) return null;
  const cut = timeBased ? effectiveTrim(trim) : null;
  if (cut === null) return sourceSecs;
  return Math.min(cut.length, Math.max(0, sourceSecs - cut.start));
}

/** `queue::trim_past_the_end`, sentence for sentence — the one refusal a trimmed row can raise. */
function trimPastTheEnd(
  trim: Settings["trim"],
  sourceSecs: number | null,
  timeBased: boolean,
): string | null {
  const cut = timeBased ? effectiveTrim(trim) : null;
  if (cut === null || sourceSecs === null || sourceSecs <= 0 || cut.start < sourceSecs) return null;
  return (
    `The trim starts at ${durationLabel(cut.start)} and this file is only ` +
    `${durationLabel(sourceSecs)} long. Lower the start time, or turn trimming off.`
  );
}

/** `settings_store::Trim`, the three verdicts an incoming trim can get. */
type TrimVerdict = { kind: "usable" } | { kind: "unfinished" | "refused"; why: string };

/** `settings_store::out_of_range`, field by field and message for message. */
function trimOutOfRange(field: string, value: number): string | null {
  if (!Number.isFinite(value)) return `The trim ${field} must be a number of seconds.`;
  if (value < 0) return `The trim ${field} cannot be negative.`;
  if (value > MAX_TRIM_SECS) return `The trim ${field} cannot be longer than 24 hours.`;
  return null;
}

/**
 * `settings_store::classify_trim`: refused, unfinished, or fine — in the order the user reads the
 * two fields.
 *
 * The unfinished arm is the one worth mirroring carefully. Trimming on with no length yet is a box
 * mid-retype, not a mistake, so it is held in silence and only *converting* through it is declined
 * — a mock that answered a keystroke with an error toast would teach the suite the wrong lesson
 * about which half-typed values are allowed to shout.
 */
function classifyTrim(trim: Settings["trim"]): TrimVerdict {
  const start = trimOutOfRange("start", trim.start_secs);
  if (start !== null) return { kind: "refused", why: start };
  const length = trimOutOfRange("length", trim.length_secs);
  if (length !== null) return { kind: "refused", why: length };
  if (trim.enabled && !(trim.length_secs > 0)) {
    return { kind: "unfinished", why: "Enter how many seconds of each file to keep." };
  }
  return { kind: "usable" };
}

/** `settings_store::check_trim`: what refuses a batch, unfinished and hostile alike. */
function checkTrim(trim: Settings["trim"]): string | null {
  const verdict = classifyTrim(trim);
  return verdict.kind === "usable" ? null : verdict.why;
}

// ---------------------------------------------------------------------------------------------
// The sign-in a link borrows — mirrors `settings::LinkSettings` and `settings_store`'s cookie checks
// ---------------------------------------------------------------------------------------------

/**
 * `settings::COOKIE_BROWSERS`, in order and lowercase — the only eight values yt-dlp is ever handed.
 *
 * Restated here rather than imported from the component that renders the picker, for the reason
 * [`outputIsTimeBased`] is restated: a mock that shared the frontend's copy of a rule would agree
 * with the frontend about a wrong answer, and agreeing with *Rust* is the entire job.
 */
const COOKIE_BROWSERS: ReadonlyArray<CookieBrowser> = [
  "safari",
  "chrome",
  "chromium",
  "edge",
  "brave",
  "firefox",
  "vivaldi",
  "opera",
];

/**
 * `settings_store`'s cookie refusals, word for word.
 *
 * The browser sentence names the allowlist by joining it, exactly as Rust does: a ninth browser
 * added to [`COOKIE_BROWSERS`] and left out of the sentence would be a refusal that lies about what
 * it accepts, and there is no way to write that here.
 */
const COOKIE_ERROR = {
  unknownBrowser:
    "Flint cannot take a sign-in from that browser. Choose one of: " +
    `${COOKIE_BROWSERS.join(", ")}.`,
  notAbsolute: "The cookies file must be a full path.",
  notThere: (path: string): string =>
    `The cookies file ${path} is not there. Export cookies.txt from your browser again and choose ` +
    "it, or take the sign-in from a browser instead.",
  isAFolder: (path: string): string =>
    `${path} is a folder, not a cookies.txt file. Choose the exported file itself.`,
};

/**
 * The two sentences `settings_store` holds back in silence, and the sheet renders as hints.
 *
 * Kept beside the refusals because they are the same wording out of the same module, and duplicated
 * from `SettingCards` on purpose: the component's copy is what a user reads, this one is what the
 * backend says, and a suite that finds them different has found a real disagreement rather than a
 * missing import.
 */
const COOKIE_UNFINISHED = {
  noBrowser: "Choose which browser to borrow the sign-in from.",
  noFile: "Choose the cookies.txt file to read the sign-in from, or take it from a browser instead.",
};

/**
 * `settings_store`'s check of a chosen cookies.txt — with the honest difference [`checkOutput`]
 * already has, and the same latitude a dropped file's name is given.
 *
 * "Is it a full path?" is a question about the string and is really asked. "Is it there?" and "is it
 * a folder?" are questions for a filesystem no browser has, so they are asked of the *shape* of the
 * path, the way a row that fails mid-convert is asked of the word `broken` in a filename: a last
 * segment with no extension is a folder, one that says `missing` is the file that was exported,
 * moved, and then chosen anyway, and every other absolute path is taken at its word.
 */
function checkCookieFile(path: string): string | null {
  if (!path.startsWith("/")) return COOKIE_ERROR.notAbsolute;
  if (path.endsWith("/")) return COOKIE_ERROR.isAFolder(path);
  const name = baseName(path);
  if (!name.includes(".")) return COOKIE_ERROR.isAFolder(path);
  if (name.toLowerCase().includes("missing")) return COOKIE_ERROR.notThere(path);
  return null;
}

/**
 * `settings::cookie_browser`: is this one of the eight, whatever case it is spelled in?
 *
 * Case-insensitive because Rust's lookup is (`eq_ignore_ascii_case`), which makes "Safari" from a
 * hand-edited settings file a usable value rather than a refused one — and a mock that refused it
 * would have the UI developed against an allowlist stricter than the app's.
 */
const cookieBrowser = (name: string): CookieBrowser | undefined =>
  COOKIE_BROWSERS.find((allowed) => allowed === name.toLowerCase());

/** `settings_store::Link`, the three verdicts an incoming sign-in gets — the trim's three, again. */
type LinkVerdict = { kind: "usable" } | { kind: "unfinished" | "refused"; why: string };

/**
 * `settings_store::classify_cookies`: refused, unfinished, or fine — judged per mode, looking only
 * at the field that mode actually reads.
 *
 * A browser name or a path left behind by a mode the user has switched away from is not a value they
 * are asking for, so it is not judged: switching back to "not signed in" has to *end* the refusal,
 * not preserve it in a field nothing reads. The unfinished arm is the careful one, for the reason the
 * trim's is — a mode chosen a keystroke before the thing it needs is not a mistake, and answering it
 * with an error toast would teach the suite the wrong lesson about which half-made choices may shout.
 */
function classifyLink(link: Settings["link"]): LinkVerdict {
  if (link.cookies === "browser") {
    const name = link.cookie_browser;
    if (name.trim() === "") return { kind: "unfinished", why: COOKIE_UNFINISHED.noBrowser };
    if (cookieBrowser(name) === undefined) {
      return { kind: "refused", why: COOKIE_ERROR.unknownBrowser };
    }
    return { kind: "usable" };
  }
  if (link.cookies === "file") {
    const path = link.cookie_file;
    if (path === null || path.trim() === "") {
      return { kind: "unfinished", why: COOKIE_UNFINISHED.noFile };
    }
    const bad = checkCookieFile(path);
    return bad === null ? { kind: "usable" } : { kind: "refused", why: bad };
  }
  return { kind: "usable" };
}

/** `settings_store::check_cookies`: what refuses a batch, unfinished and refused alike. */
function checkCookies(link: Settings["link"]): string | null {
  const verdict = classifyLink(link);
  return verdict.kind === "usable" ? null : verdict.why;
}

/**
 * `settings::COOKIE_BROWSER_APPS` — the label a sentence uses and the bundle a `stat` looks for.
 *
 * The bundle name is the *installer's* name rather than the browser's ("Google Chrome.app"), which
 * is the whole reason Rust keeps a third column: a UI that built the path from the label would look
 * for `/Applications/Chrome.app` and report a browser the user has as absent.
 */
const COOKIE_BROWSER_APPS: ReadonlyArray<readonly [CookieBrowser, string, string]> = [
  ["safari", "Safari", "Safari.app"],
  ["chrome", "Chrome", "Google Chrome.app"],
  ["chromium", "Chromium", "Chromium.app"],
  ["edge", "Edge", "Microsoft Edge.app"],
  ["brave", "Brave", "Brave Browser.app"],
  ["firefox", "Firefox", "Firefox.app"],
  ["vivaldi", "Vivaldi", "Vivaldi.app"],
  ["opera", "Opera", "Opera.app"],
];

/**
 * The browsers this pretend Mac has — `?browsers=`, defaulting to Chrome and Safari.
 *
 * A default of "all eight" would have the UI developed against a machine nobody owns, and a default
 * of "none" would hide the one-click path that is the point of the whole flow.
 */
const INSTALLED_BROWSERS: ReadonlySet<string> = new Set(
  (KNOBS.browsers ?? ["chrome", "safari"]).filter((id) => cookieBrowser(id) !== undefined),
);

/** What `stat` would say about one pretend cookie store: is it there, how big, how long ago. */
interface StoreFacts {
  exists: boolean;
  bytes: number | null;
  daysAgo: number | null;
}

const NO_STORE: StoreFacts = { exists: false, bytes: null, daysAgo: null };

/**
 * `settings::EMPTY_COOKIE_STORE_BYTES` — a Chromium jar with a schema in it and no sign-in.
 *
 * The measured number, and the one that makes this whole pass necessary: a browser opened once
 * looks, to anything that only counts bytes, exactly like a browser somebody lives in.
 */
const EMPTY_COOKIE_STORE_BYTES = 65_536;

/**
 * The three Macs, as `stat` sees them — `?mac=`.
 *
 * Written as facts about files rather than as conclusions, because that is all Rust has: a size and
 * a modification time per store, and which browser macOS opens links with. Everything the app says
 * about them is derived from these six numbers.
 */
const MACHINES: Record<
  MachineKnob,
  { default: string; stores: Partial<Record<string, StoreFacts>>; rest: StoreFacts }
> = {
  // Chrome-primary: the sign-in is in the default browser, so one click is honestly the answer.
  chrome: {
    default: "chrome",
    stores: {
      chrome: { exists: true, bytes: 524_288, daysAgo: 0.002 },
      safari: { exists: true, bytes: 98_304, daysAgo: 40 },
    },
    rest: { exists: true, bytes: 229_376, daysAgo: 9 },
  },
  // The Mac the complaint came from: 346 KB of Safari written four minutes ago, against a Chrome
  // that has been sitting at exactly the empty size for two days. The old rule offered the Chrome.
  safari: {
    default: "safari",
    stores: {
      safari: { exists: true, bytes: 354_304, daysAgo: 0.003 },
      chrome: { exists: true, bytes: EMPTY_COOKIE_STORE_BYTES, daysAgo: 2 },
    },
    // Any *third* browser on this Mac has an ordinary jar — which is how the walkthrough's "or use
    // this one instead, it needs no permission" line becomes reachable. It is not reachable with
    // Chrome, and that is the point: Chrome here is the empty one.
    rest: { exists: true, bytes: 229_376, daysAgo: 9 },
  },
  // The same Mac with the jars gone: installed browsers, and not one sign-in between them.
  nostore: { default: "chrome", stores: {}, rest: NO_STORE },
};

const MACHINE = MACHINES[KNOBS.machine];

/**
 * Which browser macOS would open an `https://` link with.
 *
 * Safari when the machine's own default is not installed, which is `launch_services`' rule as well:
 * no recorded handler means Safari, because that is what a Mac does before anybody changes it.
 */
const DEFAULT_BROWSER: string | null = INSTALLED_BROWSERS.has(MACHINE.default)
  ? MACHINE.default
  : INSTALLED_BROWSERS.has("safari")
    ? "safari"
    : null;

const storeFacts = (id: string): StoreFacts =>
  INSTALLED_BROWSERS.has(id) ? (MACHINE.stores[id] ?? MACHINE.rest) : NO_STORE;

/** Where a browser keeps its jar, for the diagnostic field. Never opened, here or in Rust. */
const COOKIE_STORE_PATHS: Readonly<Record<string, string>> = {
  safari:
    "/Users/you/Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies",
  chrome: "/Users/you/Library/Application Support/Google/Chrome/Default/Cookies",
  chromium: "/Users/you/Library/Application Support/Chromium/Default/Cookies",
  edge: "/Users/you/Library/Application Support/Microsoft Edge/Default/Cookies",
  brave: "/Users/you/Library/Application Support/BraveSoftware/Brave-Browser/Default/Cookies",
  firefox: "/Users/you/Library/Application Support/Firefox/Profiles/default/cookies.sqlite",
  vivaldi: "/Users/you/Library/Application Support/Vivaldi/Default/Cookies",
  opera: "/Users/you/Library/Application Support/com.operasoftware.Opera/Cookies",
};

/** `settings::FRESH_COOKIE_STORE_DAYS` and `RECENT_COOKIE_STORE_DAYS`, as bands rather than days. */
const freshnessBand = (facts: StoreFacts): number =>
  facts.daysAgo === null ? 2 : facts.daysAgo <= 7 ? 0 : facts.daysAgo <= 30 ? 1 : 2;

/**
 * `settings::cookie_browser_presence` and `rank_browsers`: every allowlisted browser, in allowlist
 * order, carrying the evidence and the place that evidence earns it.
 *
 * The ranking is restated here rather than imported, for the reason the mock restates every other
 * rule it shares with Rust: a copy that agreed with the frontend about a wrong answer would hide
 * exactly the disagreement this preview exists to expose. The rule is Rust's — usable first, then
 * a store with something in it, then how recently it was written, then the default browser as a
 * tie-break and nothing more, then newest first and allowlist order. What is *not* in it is the
 * permission Safari costs: that is reported as a fact, never folded into the order.
 */
const browserPresence = (): BrowserPresence[] => {
  const now = Date.now() / 1000;
  const rows: BrowserPresence[] = COOKIE_BROWSER_APPS.map(([id, label, bundle]) => {
    const installed = INSTALLED_BROWSERS.has(id);
    const facts = storeFacts(id);
    return {
      id,
      label,
      installed,
      app_path: installed ? `/Applications/${bundle}` : null,
      is_default: DEFAULT_BROWSER === id,
      cookie_store: installed ? (COOKIE_STORE_PATHS[id] ?? null) : null,
      cookie_store_exists: facts.exists,
      cookie_store_bytes: facts.exists ? facts.bytes : null,
      cookie_store_modified:
        facts.exists && facts.daysAgo !== null ? Math.round(now - facts.daysAgo * 86_400) : null,
      needs_full_disk_access: id === "safari",
      rank: 0,
      recommended: false,
    };
  });
  const order = rows.map((_, index) => index);
  order.sort((a, b) => compareKeys(rankKey(rows, a), rankKey(rows, b)));
  order.forEach((index, place) => {
    (rows[index] as BrowserPresence).rank = place + 1;
  });
  const best = rows[order[0] as number] as BrowserPresence | undefined;
  if (best !== undefined) best.recommended = best.installed && best.cookie_store_exists;
  return rows;
};

/** One row's place in the order, as the tuple `rank_browsers` sorts by. Lower is better. */
const rankKey = (rows: BrowserPresence[], index: number): number[] => {
  const row = rows[index] as BrowserPresence;
  const usable = row.installed && row.cookie_store_exists;
  const used = (row.cookie_store_bytes ?? 0) > EMPTY_COOKIE_STORE_BYTES;
  return [
    usable ? 0 : 1,
    used ? 0 : 1,
    freshnessBand(storeFacts(row.id)),
    row.is_default ? 0 : 1,
    -(row.cookie_store_modified ?? 0),
    index,
  ];
};

const compareKeys = (left: number[], right: number[]): number => {
  for (let i = 0; i < left.length; i += 1) {
    const diff = (left[i] as number) - (right[i] as number);
    if (diff !== 0) return diff;
  }
  return 0;
};

/**
 * `commands::check_safari_cookie_access`: one `open` of Safari's jar, and what it proved.
 *
 * Instant, because that is the entire point of the command it stands for. The old route to this
 * answer was the twenty-second probe below, which is a long way to travel to learn that a macOS
 * permission is missing — and `?fda` is the only knob it needs, because a browser tab has no
 * permissions to be short of and no way to earn one.
 */
const safariAccess = (): SafariAccess => {
  const jar = COOKIE_STORE_PATHS["safari"] ?? null;
  const facts = storeFacts("safari");
  if (!facts.exists) {
    return {
      result: "no_cookie_store",
      ok: false,
      message: SAFARI_HAS_NO_COOKIE_STORE,
      cookie_store: jar,
    };
  }
  if (!KNOBS.fullDiskAccess) {
    return {
      result: "needs_full_disk_access",
      ok: false,
      message: FETCH_FAILURE.safariNeedsFullDiskAccess,
      cookie_store: jar,
    };
  }
  return { result: "readable", ok: true, message: SAFARI_COOKIES_READABLE, cookie_store: jar };
};

/**
 * Can this pretend Mac actually read the sign-in the settings name?
 *
 * The question `link::classify_failure` answers from yt-dlp's stderr, asked here of the two things
 * a browser tab does know: what the settings say, and which browsers `?browsers=` says exist. A
 * browser that is not installed has no cookie database to open — that is the measured
 * `BrowserCookiesUnreadable` — and Safari's jar is behind a permission no page can grant, which is
 * the same ending Safari really gives. Everything else is taken at its word.
 *
 * The unreadable arm carries its own sentence because the three of them send the user to three
 * different places, which is the entire distinction Rust stopped folding together.
 */
type SignInState =
  | { kind: "none" }
  | { kind: "usable" }
  | { kind: "unreadable"; message: string };

function signInState(link: Settings["link"]): SignInState {
  // A half-made or refused source never reaches yt-dlp at all: the batch is turned away first.
  if (link.cookies === "none" || checkCookies(link) !== null) return { kind: "none" };
  if (link.cookies === "file") return { kind: "usable" };
  const id = cookieBrowser(link.cookie_browser);
  if (id === "safari") {
    // The permission, and only the permission: with `?fda` the jar opens and Safari is as ordinary
    // as any other browser, which is the state the walkthrough's steps are trying to reach.
    return KNOBS.fullDiskAccess
      ? { kind: "usable" }
      : { kind: "unreadable", message: FETCH_FAILURE.safariNeedsFullDiskAccess };
  }
  if (id === undefined || !INSTALLED_BROWSERS.has(id)) {
    return { kind: "unreadable", message: FETCH_FAILURE.browserCookiesUnreadable };
  }
  // A browser with no cookie store is a jar that never opened, whatever the settings page says.
  if (!storeFacts(id).exists) {
    return { kind: "unreadable", message: FETCH_FAILURE.browserCookiesUnreadable };
  }
  return { kind: "usable" };
}

/** Which category owns an output format — the question `plan` asks of `target.category`. */
function targetCategory(targetId: string): CategoryId | null {
  for (const category of MOCK_CATALOG.categories) {
    if (category.outputs.some((f) => f.id === targetId)) return category.id;
  }
  return null;
}

const ANIMATED_IMAGE = new Set(["gif", "webp", "avif", "apng"]);

/**
 * `plan::output_is_time_based`: does this pairing produce something with a duration?
 *
 * Restated here rather than imported from `./format`, on purpose. The frontend has its own copy of
 * this rule (the row has to answer it before the batch runs), and a mock that shared that copy
 * would agree with the UI about a wrong answer — which is exactly the disagreement with Rust the
 * suite exists to catch.
 */
function outputIsTimeBased(
  sourceCategory: CategoryId | null,
  sourceIsAnimated: boolean,
  targetId: string,
): boolean {
  const target = targetCategory(targetId);
  if (target === "video" || target === "audio") return true;
  if (target !== "image") return false;
  if (sourceCategory === "flash") return ANIMATED_IMAGE.has(targetId);
  // An animated GIF/WebP has a duration only when the source moves, and a frame sequence is pulled
  // out of one for the same reason: both image cases collapse to the one question.
  return sourceIsAnimated;
}

function mockOutputPath(path: string, targetId: string, settings: Settings): string {
  const dir = parentOf(path);
  const out = settings.output;
  const folder =
    out.location === "custom" && out.custom_dir !== null
      ? out.custom_dir
      : out.location === "subfolder"
        ? `${dir}/${out.subfolder_name}`
        : dir;
  return `${folder}/${stem(baseName(path))}.${outputExtensionFor(targetId)}`;
}

// ---------------------------------------------------------------------------------------------
// Links as a source — mirrors `convert_core::link` and `commands::inspect_links`
// ---------------------------------------------------------------------------------------------

/*
 * The cap this section enforces is `KNOBS.maxLinks` — [`MAX_LINKS_PER_BATCH`] unless `?maxlinks=`
 * lowered it. It is declared beside the file cap, up with the other knobs, because `readKnobs`
 * defaults to it; `get_link_support` then reports whatever it resolved to, and the UI reads it back.
 */

/** The binary a link needs, as `Tool::YtDlp` and `package::YT_DLP` both spell it. */
const YT_DLP_TOOL = "yt-dlp";

/**
 * The runtimes yt-dlp will hand YouTube's challenges to — `format::JS_RUNTIMES`, in its order.
 *
 * Both are discovered; only the first is installed. `node` earns a row in Settings because a machine
 * that has it needs nothing else, and no plan, because a converter does not install a JavaScript
 * toolchain for anybody (see [`PACKAGES`]).
 */
const JS_RUNTIME_TOOLS: readonly string[] = ["deno", "node"];

type LinkSite = "youtube" | "bilibili" | "qqmusic" | "netease" | "soundcloud" | "bandcamp";

const linkCategory = (site: LinkSite): "audio" | "video" =>
  site === "youtube" || site === "bilibili" ? "video" : "audio";

const SITE_LABEL: Readonly<Record<LinkSite, string>> = {
  youtube: "YouTube",
  bilibili: "Bilibili",
  qqmusic: "QQ Music",
  netease: "NetEase Music",
  soundcloud: "SoundCloud",
  bandcamp: "Bandcamp",
};

/**
 * `link::ACCEPTED_HOSTS`, in order and exact-match only.
 *
 * Exact match is the point, here as much as in Rust: `youtube.com.evil.test` ends with `youtube.com`
 * and `y0utube.com` reads like it, and a preview that accepted either would develop the UI against
 * a rule the app does not have.
 */
const ACCEPTED_HOSTS: ReadonlyArray<readonly [string, LinkSite]> = [
  ["youtube.com", "youtube"],
  ["www.youtube.com", "youtube"],
  ["m.youtube.com", "youtube"],
  ["music.youtube.com", "youtube"],
  ["youtu.be", "youtube"],
  ["www.youtu.be", "youtube"],
  ["bilibili.com", "bilibili"],
  ["www.bilibili.com", "bilibili"],
  ["m.bilibili.com", "bilibili"],
  ["b23.tv", "bilibili"],
  ["www.b23.tv", "bilibili"],
  ["y.qq.com", "qqmusic"],
  ["music.163.com", "netease"],
  ["y.music.163.com", "netease"],
  ["soundcloud.com", "soundcloud"],
  ["www.soundcloud.com", "soundcloud"],
  ["m.soundcloud.com", "soundcloud"],
];

/**
 * `link::LinkError`, message for message.
 *
 * Copied from the `#[error(…)]` strings rather than paraphrased: these sentences are the entire
 * refusal UI — the sheet renders them verbatim — so the preview has to be developed against the
 * words a user will really read, at the length they will really be.
 */
const LINK_ERROR = {
  notAUrl: (shown: string): string =>
    `\`${shown}\` is not a link. Copy the address from your browser's address bar - it starts with https://.`,
  credentials:
    "That link carries a username and password, which a video link never does. Copy the address " +
    "straight from your browser's address bar instead.",
  unknownHost: (host: string): string =>
    `The site \`${host}\` is not supported. Paste a YouTube, Bilibili, QQ Music, NetEase Music, ` +
    `SoundCloud or artist.bandcamp.com single-track link. Spotify is not supported.`,
  playlist:
    "That is a playlist, not a video. Open the videos you want and paste their individual links - " +
    "up to 20 at a time.",
  channel:
    "That is a channel, not a video. Open the videos you want and paste their individual links - " +
    "up to 20 at a time.",
  live:
    "Live streams cannot be converted while they are running. Wait until the stream has finished " +
    "and paste the link to the recording.",
  notAVideoPage: (site: LinkSite): string =>
    `That is not a ${SITE_LABEL[site]} video page. Open the video itself and paste the link from ` +
    `the address bar.`,
  tooMany: (count: number, max: number): string =>
    `You pasted ${count} links. Flint takes ${max} at a time - remove ${count - max} ` +
    `and paste them as a second batch.`,
  notInstalled:
    "yt-dlp is not installed. Open Settings → Helpers and install it (one click), then try again.",
} as const;

/**
 * `link::FetchFailure`, for the two endings a fetch has an *affordance* for — verbatim again, and
 * for a sharper reason than the refusals above: the frontend reads these sentences back.
 *
 * A row that failed for want of a runtime is what resolves to the Deno row in Settings, and a row
 * that failed on Safari's cookie jar is what grows the button into System Settings. Both decisions
 * are made partly from the words, so a paraphrase here would develop the UI against a message the
 * app never sends (see `missingHelperFor` and `needsFullDiskAccess` in `state/store`).
 */
const FETCH_FAILURE = {
  noJsRuntime:
    "YouTube needs a JavaScript runtime to hand this video over, and Flint could not " +
    "use one. Open Settings → Helpers and install Deno (one click), then try again.",
  /*
   * The three things that actually block a Safari user, in Rust's own words.
   *
   * The permission is not the whole of it, which is what the old sentence assumed: macOS only
   * consults a Full Disk Access grant when a process *starts*, and it keys that grant to a code
   * signature, so an ad-hoc signed copy loses it on every rebuild while its entry sits in the list
   * looking switched on. A user who grants it and does nothing else sees no change at all.
   */
  safariNeedsFullDiskAccess:
    "Safari keeps its cookies where only an app with Full Disk Access can read them, and " +
    "Flint does not have it. Grant it in System Settings → Privacy & Security → Full " +
    "Disk Access, then quit Flint and open it again - the permission only reaches an app " +
    "that was started after it was given. A copy you built yourself loses the grant every time it " +
    "is rebuilt, so switch it off and on again in that list. The easier route is to pick a browser " +
    "in Settings → Links that needs no permission at all.",
  /*
   * The three sign-in endings, which are three different fixes and therefore three sentences.
   *
   * The frontend reads these back too — `signInFailure` in `state/store` decides from them which
   * remedy a failed row may offer — so they are verbatim `FetchFailure` for the same reason the two
   * above are. Folding any two of them together is exactly the bug Rust just stopped having.
   */
  loginRequired:
    "The site wants a sign-in before it will hand this video over. Open Settings → Links and let " +
    "Flint borrow the sign-in from your browser, or point it at a cookies.txt file " +
    "you exported yourself.",
  cookiesNotAccepted:
    "The site would not accept the sign-in Flint borrowed. Sign in to the site again " +
    "in that browser and retry, or export a fresh cookies.txt - the cookies it was given have most " +
    "likely expired, or belong to an account without access to this video.",
  browserCookiesUnreadable:
    "Flint could not read the sign-in from that browser, so the site was asked for " +
    "this video with no sign-in at all. Check that the browser is installed and that you are " +
    "signed in to the site in it, or export a cookies.txt file and point Settings → Links at that " +
    "instead.",
} as const;

/**
 * `link::SAFARI_COOKIES_READABLE` — careful about what one successful `open` proves.
 *
 * The permission is in place and the file opened. Nothing was taken out of it, and whether the site
 * accepts what is inside is a different question with a different answer.
 */
const SAFARI_COOKIES_READABLE =
  "Flint can read Safari's cookies: Full Disk Access is in place. Nothing was read " +
  "out of the file - whether the site accepts that sign-in is the next question.";

/** `link::SAFARI_HAS_NO_COOKIE_STORE` — not a permission problem, and never dressed up as one. */
const SAFARI_HAS_NO_COOKIE_STORE =
  "Safari has no saved cookies on this Mac, so there is no sign-in to borrow from it. Sign in to " +
  "the site in Safari, or choose another browser in Settings → Links.";

/**
 * `link::COOKIE_TEST_URL` and the sentence one working check produces — `link::COOKIE_TEST_WORKS`.
 *
 * The check's whole value is that it is *specific*: it says which video it tried, so a user reading
 * "the site handed the test video over" knows what was proved and what was not.
 */
const COOKIE_TEST_URL = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
const COOKIE_TEST_TIMEOUT_SECS = 20;
const COOKIE_TEST = {
  works: "That sign-in works: Flint read it and the site handed the test video over.",
  /** `CookieProbe::TimedOut` — a verdict about the network, carefully not about the cookies. */
  timedOut:
    `The sign-in check was still running after ${COOKIE_TEST_TIMEOUT_SECS} seconds and was ` +
    "stopped, so it proved nothing either way. Check your internet connection and try again.",
  /** `commands::NO_COOKIE_SOURCE_TO_TEST` — a state, and not a fault. */
  notConfigured:
    "There is no sign-in to check yet. Choose a browser to borrow the sign-in from, or a " +
    "cookies.txt file you exported, and then check it.",
} as const;

/** `char::is_control`, as far as the two places below need it. */
const isControlChar = (c: string): boolean => {
  const code = c.codePointAt(0) ?? 0;
  return code < 0x20 || (code >= 0x7f && code <= 0x9f);
};

/** `link::printable`: a crafted host or line, cut before it reaches a message the UI renders. */
function printableLink(raw: string): string {
  const cleaned = [...raw]
    .filter((c) => !isControlChar(c))
    .slice(0, 80)
    .join("");
  return cleaned.trim() === "" ? "(empty)" : cleaned;
}

/** Path and query, fragment dropped — `link::split_path`. */
function splitLinkPath(remainder: string): { path: string; query: string } {
  const withoutFragment = remainder.split("#")[0] ?? "";
  const mark = withoutFragment.indexOf("?");
  return mark === -1
    ? { path: withoutFragment, query: "" }
    : { path: withoutFragment.slice(0, mark), query: withoutFragment.slice(mark + 1) };
}

const queryValue = (query: string, key: string): string | undefined =>
  query
    .split("&")
    .map((pair) => pair.split("="))
    .find((parts) => parts[0] === key)?.[1];

/** `link::check_path`: is this the URL of *one video*, on this site? */
function checkLinkPath(site: LinkSite, host: string, path: string, query: string): string | null {
  if (linkCategory(site) === "audio") {
    const url = new URL(`https://${host}${path}?${query}`);
    const parts = url.pathname.replace(/^\/|\/$/g, "").split("/");
    let valid = false;
    if (site === "qqmusic") valid = /^\/n\/ryqq\/songDetail\/[A-Za-z0-9]+\/?$/.test(url.pathname);
    if (site === "netease") valid = ["/song", "/m/song"].includes(url.pathname)
      && /^\d+$/.test(url.searchParams.get("id") ?? "");
    if (site === "bandcamp") valid = /^\/track\/[A-Za-z0-9_-]+\/?$/.test(url.pathname);
    if (site === "soundcloud") valid = (parts.length === 2 || (parts.length === 3 && parts[2]?.startsWith("s-") === true))
      && parts.every((p) => /^[A-Za-z0-9_-]+$/.test(p))
      && !["discover", "search", "you", "stations", "stream"].includes(parts[0] ?? "")
      && !["sets", "tracks", "albums", "likes", "reposts", "popular-tracks", "spotlight", "comments"].includes(parts[1] ?? "");
    return valid ? null : `That is not a supported ${SITE_LABEL[site]} single-track page. Open the track itself and copy its full web address; albums, playlists and short share links are not supported.`;
  }
  const trimmed = path.replace(/\/+$/, "").replace(/^\/+/, "");
  const first = trimmed.split("/")[0] ?? "";
  const rest = trimmed.includes("/") ? trimmed.slice(trimmed.indexOf("/") + 1) : "";
  if (site === "youtube" && host.endsWith("youtu.be")) {
    if (first === "") return LINK_ERROR.notAVideoPage(site);
    return first === "playlist" ? LINK_ERROR.playlist : null;
  }
  if (site === "youtube") {
    // `watch?v=X&list=Y` is one video that happens to sit in a playlist, and `--no-playlist` keeps
    // it that way; a bare `/playlist?list=…` is the thing that would expand past the cap.
    if (first === "watch") {
      const v = queryValue(query, "v");
      return v !== undefined && v !== "" ? null : LINK_ERROR.notAVideoPage(site);
    }
    if ((first === "shorts" || first === "embed") && rest !== "") return null;
    if (first === "live") return LINK_ERROR.live;
    if (first === "playlist") return LINK_ERROR.playlist;
    if (first === "channel" || first === "c" || first === "user" || first.startsWith("@")) {
      return LINK_ERROR.channel;
    }
    return LINK_ERROR.notAVideoPage(site);
  }
  if (host.endsWith("b23.tv")) {
    // A short link hides its destination, so the only check possible is "there is one".
    return first === "" ? LINK_ERROR.notAVideoPage(site) : null;
  }
  if (first === "video" && rest !== "") return null;
  if (first === "medialist" || first === "playlist" || first === "watchlater") {
    return LINK_ERROR.playlist;
  }
  if (first === "space") return LINK_ERROR.channel;
  return LINK_ERROR.notAVideoPage(site);
}

/** One line, judged — `link::Link::parse`. Either the accepted link or the refusal to show. */
function parseLink(raw: string): { url: string; site: LinkSite } | { error: string } {
  const url = raw.trim();
  // Whitespace or a control character is not a URL — and it is exactly the shape two links pasted
  // onto one line takes.
  if (url === "" || url.length > 8192 || url.includes("\\") || /\s/.test(url) || [...url].some(isControlChar)) {
    return { error: LINK_ERROR.notAUrl(printableLink(url)) };
  }
  const split = url.indexOf("://");
  if (split === -1) return { error: LINK_ERROR.notAUrl(printableLink(url)) };
  const scheme = url.slice(0, split).toLowerCase();
  if (scheme !== "http" && scheme !== "https") {
    return { error: LINK_ERROR.notAUrl(printableLink(url)) };
  }
  const rest = url.slice(split + 3);
  const end = rest.search(/[/?#]/);
  const authority = end === -1 ? rest : rest.slice(0, end);
  const remainder = end === -1 ? "" : rest.slice(end);
  // `https://youtube.com@evil.test/x` is a link to evil.test.
  if (authority.includes("@")) return { error: LINK_ERROR.credentials };
  const colon = authority.indexOf(":");
  const port = colon === -1 ? null : authority.slice(colon + 1);
  if (port !== null && !((scheme === "https" && port === "443") || (scheme === "http" && port === "80"))) {
    return { error: LINK_ERROR.notAUrl(printableLink(url)) };
  }
  // A trailing dot is a legal FQDN and a classic allowlist bypass: `youtube.com.` resolves exactly
  // like `youtube.com` and would not match the table.
  const host = (colon === -1 ? authority : authority.slice(0, colon))
    .replace(/\.+$/, "")
    .toLowerCase();
  if (host === "") return { error: LINK_ERROR.notAUrl(printableLink(url)) };
  if (!/^[a-z0-9.-]+$/.test(host)) return { error: LINK_ERROR.unknownHost(printableLink(host)) };

  let { path, query } = splitLinkPath(remainder);
  // Before the allowlist refusal: "live.bilibili.com" is a recognisable place, and "that host is
  // not on the list" would not say what is wrong with it.
  if (host.startsWith("live.") && ACCEPTED_HOSTS.some(([h]) => host.endsWith(h))) {
    return { error: LINK_ERROR.live };
  }
  const site = ACCEPTED_HOSTS.find(([h]) => h === host)?.[1]
    ?? (/^(?!www\.|daily\.|get\.)[a-z0-9-]+\.bandcamp\.com$/.test(host) ? "bandcamp" : undefined);
  if (site === undefined) return { error: LINK_ERROR.unknownHost(printableLink(host)) };
  if (site === "netease" && remainder.startsWith("/#/")) ({ path, query } = splitLinkPath(remainder.slice(2)));
  const refusal = checkLinkPath(site, host, path, query);
  return refusal === null ? { url, site } : { error: refusal };
}

/** `link::sanitize_title`, as far as a fake title can exercise it. */
function sanitizeTitle(title: string): string {
  const mapped = title
    .replace(/[/\\:*?"<>|]/g, "-")
    .replace(/-{2,}/g, "-")
    .replace(/\s{2,}/g, " ")
    .trim()
    .slice(0, 80)
    .replace(/^[-.\s]+|[-.\s]+$/g, "");
  return mapped === "" ? "video" : mapped;
}

/**
 * Titles a fetch "resolves" to, chosen by the URL's own hash so a link always comes back as the
 * same video. Long, punctuated and CJK on purpose: the row has to survive the names videos have.
 */
const FAKE_TITLES: readonly string[] = [
  "Why Rust's borrow checker is the best documentation you never read",
  "How we shipped a Tauri app in three weeks (and what broke)",
  "关于视频压缩的十分钟入门",
  "Live coding: writing a batch converter from scratch — part 4/7",
  "The quietest interface I have ever designed",
];

/** The video a link is pretended to be: a stable title and duration for the row. */
const fakeVideo = (url: string): { title: string; secs: number } => {
  const seed = hashUnit(url);
  const title = FAKE_TITLES[Math.floor(seed * FAKE_TITLES.length)] ?? FAKE_TITLES[0] ?? "video";
  // `?clipsecs=` reaches a pasted video too: a link is trimmed like any other row, and the only way
  // to see the cut land on one is to know how long the fetch is going to claim it is.
  return { title, secs: clipSeconds(40 + seed * 1800) };
};

/**
 * `~/Downloads` in the same fake home every other mock path lives in ([`MOCK_DIR`] is its Desktop).
 */
const MOCK_DOWNLOADS = "/Users/you/Downloads";

/**
 * `paths::link_output_dir`: the folder the user named if they ever named one, `~/Downloads` if not.
 *
 * A custom folder counts even when the location setting is something else — it is the only folder
 * they have ever pointed at — and "alongside the source" has no meaning for a URL.
 */
const linkDestination = (settings: Settings): string => {
  const dir = settings.output.custom_dir;
  return dir !== null && dir.trim() !== "" ? dir : MOCK_DOWNLOADS;
};

// ---------------------------------------------------------------------------------------------
// The mock backend
// ---------------------------------------------------------------------------------------------

/** Real sizes recorded from the browser's DataTransfer, keyed by the pseudo-path we hand out. */
const droppedSizes = new Map<string, number>();
/** Folder → files inside it, so a picked folder expands the way the Rust walker would. */
const directoryContents = new Map<string, string[]>();

/** What a picked-but-unknown folder is pretended to contain, so the folder flow shows something. */
const SAMPLE_FOLDER = [
  "keynote-recording.mov",
  "product-shot.HEIC",
  "voice-memo.m4a",
  "roadmap.pptx",
  "captions.vtt",
  "banner.psd",
  "notes.zip",
];

/** Pretend the user's Desktop is where everything lives, so paths look plausible in the UI. */
const MOCK_DIR = "/Users/you/Desktop";
const pseudoPath = (name: string): string => `${MOCK_DIR}/${name}`;

/** Row ids must be unique for the lifetime of the session, exactly like `commands::next_id`. */
let idCounter = 0;
const nextId = (): string => {
  idCounter += 1;
  return `mock-${idCounter.toString(36)}`;
};

/**
 * The keystroke, if any, that the native menu bar would have turned into an action.
 *
 * Inside the app macOS owns these keys and Rust emits `menu://action`; in a browser tab nothing
 * does, so the preview binds them here instead — otherwise Settings would be unreachable, the menu
 * being the only way in.
 */
function menuActionForKey(event: KeyboardEvent): MenuAction | null {
  if (!(event.metaKey || event.ctrlKey)) return null;
  const key = event.key.toLowerCase();
  if (key === "," || (event.shiftKey && event.code === "Comma")) return event.shiftKey ? "skin" : "settings";
  if (key === "o") return event.shiftKey ? "open_folder" : "open_files";
  if (key === "l") return "paste_links";
  if (key === "enter") return "convert";
  if (key === ".") return "stop";
  // ⇧⌘⌫ only: plain ⌘⌫ is "delete backwards" in every text field in the settings sheet.
  if (key === "backspace" && event.shiftKey) return "clear";
  return null;
}

export function createMockBackend(): Backend {
  // `Settings::default()`, except for the sign-in: `?cookies=` starts the session in one of the
  // states the Links group would otherwise have to be clicked into — see [`cookieKnob`].
  let settings: Settings = { ...defaultSettings(), link: KNOBS.cookies };
  /*
   * The last sign-in a save actually *accepted*, which is not always the one on screen.
   *
   * `?cookies=netscape` starts the session in a value this backend refuses — the picker cannot
   * produce one, so a knob is the only way to look at that refusal — and a store that then "held the
   * last usable value" by holding the refused one would be inventing a state Rust never has. This is
   * `settings_store::last_usable_cookies`, kept apart from `settings` so the first refusal holds
   * `LinkSettings::default()` exactly as Rust would with nothing usable behind it. After one accepted
   * save the two are the same value.
   *
   * The knob's own deviation, stated plainly: `settings_store::load` resets a *saved* source it would
   * refuse back to "no cookies" and says so on stderr, so a real launch never opens the sheet on one.
   * A preview has to be able to, or the sentence is unreachable.
   */
  let usableLink: LinkSettings =
    classifyLink(KNOBS.cookies).kind === "usable" ? KNOBS.cookies : defaultSettings().link;
  /**
   * The settings the *running batch* was started with. Separate from the persisted ones because
   * `commands::start_batch` takes a `Settings` for the run and never writes it: a mock that saved
   * them would answer the next `get_settings` with something Rust would not have.
   */
  let batchSettings = defaultSettings();
  const currentBatchSettings = (): Settings => batchSettings;
  const batchHandlers = new Set<(event: BatchEvent) => void>();
  const installHandlers = new Set<(event: InstallEvent) => void>();
  const dropHandlers = new Set<(event: DropEvent) => void>();
  /** Destinations produced earlier in this session — lets the `skip` conflict policy do something. */
  const produced = new Set<string>();
  let running = false;
  let cancelled = false;
  /** One install at a time, exactly like `AppState::begin_install`. */
  let installing = false;
  /** `?activity=stale`: spent on the first `get_activity`, which is the whole point of it. */
  let staleActivity = KNOBS.staleActivity;

  /**
   * One place where the batch slot is freed, for the same reason `commands::start_batch` has one.
   *
   * ORDER MATTERS, and it is Rust's order: the slot is released *before* `batch_finished` reaches a
   * handler, never after, so a window that hears the batch end may start the next one immediately.
   * Freeing it here rather than at each of the places that finish a batch is what makes the
   * backstop honest too - `SlotGuard` closes a panicked batch out with a `batch_finished` its own
   * `run_batch` never sent, and the slot is free after that event exactly as it is after any other.
   */
  const emit = (event: BatchEvent): void => {
    if (event.type === "batch_finished") {
      running = false;
      cancelled = false;
    }
    for (const handler of batchHandlers) handler(event);
  };

  const emitInstall = (event: InstallEvent): void => {
    for (const handler of installHandlers) handler(event);
  };

  /** Every `estimate_output_path` this backend has been asked, for the seam below. */
  const estimated: string[] = [];
  /** Every settings object this backend has actually *written*, for the seam below. */
  const saved: Settings[] = [];
  /** Every sign-in source `test_cookie_source` was actually made to probe, for the seam below. */
  const probed: string[] = [];

  /*
   * Preview-only test seam. `batch://event` is a stream the frontend cannot produce by itself, so
   * the behavioural suite needs a way to post the awkward cases — an event after `batch_finished`,
   * a duplicate `started`, an id from a run that is over — and check that the store ignores them.
   *
   * This lives in the mock and nowhere else: the module is only ever loaded when
   * `__TAURI_INTERNALS__` is absent, so the shipped app has no such hook.
   */
  if (typeof window !== "undefined") {
    (window as unknown as { __ceMockEmit?: (event: BatchEvent) => void }).__ceMockEmit = emit;
    (
      window as unknown as { __ceMockInstallEmit?: (event: InstallEvent) => void }
    ).__ceMockInstallEmit = emitInstall;
    // Every path `estimate_output_path` was asked about, in order. A link row's `path` is the empty
    // string and it has no output to estimate — `paths::link_output_dir` answers that question
    // instead — so "was this row treated as a file?" is otherwise only visible in the copy it broke.
    (window as unknown as { __ceMockEstimates?: string[] }).__ceMockEstimates = estimated;
    // Every settings object `save_settings` actually wrote, in order. A save the frontend holds
    // back — a destination with no folder yet, a trim whose length is mid-retype — is invisible
    // from the outside, and "was the edit made *beside* it kept?" is a question only the write can
    // answer: the sheet goes on showing what the user typed either way.
    (window as unknown as { __ceMockSaves?: Settings[] }).__ceMockSaves = saved;
    // Every sign-in source the twenty-second probe was actually run against, in order. Safari's
    // check is a permission question with a local answer, so the interesting fact about it is a
    // negative one — that nothing was probed at all — and a negative is not visible on screen.
    (window as unknown as { __ceMockProbes?: string[] }).__ceMockProbes = probed;
  }

  /**
   * A simulated `brew install`: a few seconds of streamed output, then one `finished`.
   *
   * One run per *package*, keyed by `package_id` the whole way through, because that is what the
   * user clicked and what the events carry. Success is applied to the tool state before the event
   * goes out, so the `refreshTools` the UI does next really does find the helpers — which is what
   * makes the greyed-out format come back.
   *
   * `?installpartial=` lands one more of the package's binaries and no more: exit 0, some of the
   * formula where the app looks, and the honest "part of Poppler is here" ending
   * (`install::Outcome::Incomplete`) that a one-binary package can never reach. A single-member
   * package is left alone by the knob, since for it "part of" would be the whole thing.
   *
   * `?installmissing=` lands nothing at all while still exiting 0 — `install::Outcome::NotDiscoverable`,
   * which is what a Homebrew installing into a prefix this build does not know looks like from here.
   * Both endings are decided the way Rust decides them, from what the machine has afterwards rather
   * than from the exit code: that is the whole point of the probe `install.rs` injects after the
   * installer has exited.
   */
  async function runInstall(plan: PackageInstallPlan): Promise<void> {
    const knobbed = (list: string[]): boolean =>
      list.includes("all") || list.includes(plan.package_id);
    const fails = knobbed(KNOBS.failing);
    const partial = !fails && plan.tool_ids.length > 1 && knobbed(KNOBS.partial);
    /** Exit 0, and not a binary anywhere the app looks — see the note above. */
    const vanishes = !fails && knobbed(KNOBS.undiscoverable);
    emitInstall({ type: "started", package_id: plan.package_id });
    const transcript = installTranscript(plan);
    // A failure stops part-way through, the way a real one does.
    const shown = fails ? transcript.slice(0, Math.ceil(transcript.length / 2)) : transcript;
    for (const line of shown) {
      await sleep(KNOBS.lineMs);
      emitInstall({ type: "log", package_id: plan.package_id, line });
    }
    if (!fails && !vanishes) {
      // A partial install brings *one* more of the package's programs, so the ending really is
      // "2 of its 3" rather than a number that never moved.
      const absent = plan.tool_ids.filter((toolId) => missingTools.has(toolId));
      const landed = partial ? absent.slice(0, 1) : absent;
      for (const toolId of landed) missingTools.delete(toolId);
    }
    // Presence decides the ending, not the exit code — `install::outcome_of`. Nothing found reads
    // differently from some of it found, and neither of them is success.
    const found = plan.tool_ids.filter((toolId) => !missingTools.has(toolId)).length;
    const outcome: InstallOutcome = fails
      ? "failed"
      : found === plan.tool_ids.length
        ? "installed"
        : found === 0
          ? "notDiscoverable"
          : "incomplete";
    const ending = installEnding(plan, outcome, found);
    for (const line of ending.lines) {
      await sleep(KNOBS.lineMs);
      emitInstall({ type: "log", package_id: plan.package_id, line });
    }
    installing = false;
    emitInstall({
      type: "finished",
      package_id: plan.package_id,
      ok: outcome === "installed",
      message: ending.message,
    });
  }

  function describe(path: string): FileInfo {
    const name = baseName(path);
    const ext = extensionOf(name);
    const formatId = EXTENSION_TO_FORMAT[ext];
    const seed = hashUnit(name);
    const size = droppedSizes.get(path) ?? Math.round(120_000 + seed * 480_000_000);

    const base: FileInfo = {
      id: nextId(),
      path,
      name,
      size_bytes: size,
      supported: false,
      category: null,
      format_id: null,
      format_name: null,
      default_target: null,
      suggested_targets: [],
      duration_secs: null,
      duration_label: null,
      resolution_label: null,
      is_animated: false,
      note: null,
    };

    if (formatId === undefined) {
      return { ...base, note: `Unsupported file type: .${ext}` };
    }

    const meta = FORMAT_META[formatId];
    if (meta === undefined) return { ...base, note: `Unsupported file type: .${ext}` };
    const category = meta.category as CategoryId;
    const view = catalogView().categories.find((c) => c.id === category);
    const inputView = view?.inputs.find((f) => f.id === formatId);

    const info: FileInfo = {
      ...base,
      supported: true,
      category,
      format_id: formatId,
      format_name: meta.name,
      default_target: view?.default_target ?? null,
      suggested_targets: view?.suggested_targets ?? [],
    };

    if (category === "video" || category === "audio" || category === "flash") {
      const probe = fakeProbe(path);
      info.duration_secs = probe.secs;
      info.duration_label = probe.secs === null ? null : durationLabel(probe.secs);
      info.is_animated = probe.animated;
    }
    if (category === "video" || category === "image") {
      const res = RESOLUTIONS[Math.floor(seed * RESOLUTIONS.length)] ?? RESOLUTIONS[0];
      if (res !== undefined) info.resolution_label = `${res[0]}×${res[1]}`;
      info.is_animated = info.is_animated || formatId === "gif";
    }

    // Same note logic as `commands::helper_note`: source decoder first, then default target.
    if (inputView !== undefined && !inputView.available) {
      info.note = `Needs ${inputView.needs.join(" or ")}`;
    } else if (view !== undefined) {
      const target = targetFormat(view.default_target);
      if (!target.available) info.note = `Needs ${target.needs.join(" or ")}`;
    }
    if (formatId === "midi" && info.note === null) info.note = "Renders all MIDI tracks with a basic piano sound.";
    return info;
  }

  /** Stand-in for the server-side directory walk in `commands::expand_paths`. */
  function expandDirectory(path: string): string[] {
    const known = directoryContents.get(path);
    if (known !== undefined) return known;
    if (extensionOf(baseName(path)) !== "") return [path];
    return SAMPLE_FOLDER.map((name) => `${path}/${name}`);
  }

  /**
   * `commands::expand_paths`: folders expanded, duplicates dropped, and the whole thing bounded.
   *
   * The cap is checked on *entry*, before a path is looked at, exactly as `commands::collect` does —
   * so the walk stops at the budget rather than collecting everything and trimming afterwards, and
   * `truncated` means "there was more on offer than the cap allows". It is deliberately
   * conservative in the same way Rust is: the entry turned away at the boundary might have proved to
   * be a duplicate, but finding that out would mean walking on, which is the thing being avoided.
   *
   * A mock that returned every file and set `truncated: false` would be the more forgiving of the
   * two backends, and the suite believes this one.
   */
  function walkPaths(paths: string[]): { files: string[]; truncated: boolean } {
    const files: string[] = [];
    const seen = new Set<string>();
    let truncated = false;
    const collect = (path: string): void => {
      if (files.length >= KNOBS.maxFiles) {
        truncated = true;
        return;
      }
      const children = expandDirectory(path);
      // A folder: recurse, so each child is measured against the budget on its own.
      if (children.length !== 1 || children[0] !== path) {
        for (const child of children) collect(child);
        return;
      }
      if (seen.has(path)) return;
      seen.add(path);
      files.push(path);
    };
    for (const path of paths) collect(path);
    return { files, truncated };
  }

  /**
   * A pasted link, run the way `queue::convert_link` runs one: fetched, then converted.
   *
   * The row is announced *before* the download, with the output path the resolved title produces —
   * a fetch is the slowest thing the app does, and a row with no destination for two minutes looks
   * like a hang. The first sample is deliberately indeterminate, because yt-dlp says nothing about
   * how big the video is until it has finished negotiating with the site.
   */
  async function runLink(item: BatchItemArg, url: string): Promise<"ok" | "failed" | "skipped"> {
    const parsed = parseLink(url);
    if ("error" in parsed) {
      emit({ id: item.id, type: "failed", message: parsed.error });
      return "failed";
    }
    const category = linkCategory(parsed.site);
    const batchSettings = cropMediaSettings(currentBatchSettings(), category);
    if (category === "audio" && targetCategory(item.target_id) !== "audio") {
      emit({ id: item.id, type: "failed", message: "Music links need an audio output format. Choose MP3, M4A, WAV or FLAC." });
      return "failed";
    }
    // What `engine.probe_link` reports first, before a byte is downloaded: the tool has to be there
    // at all. The row fails with the sentence that says how to fix it, and the store turns that into
    // the "Install yt-dlp" microlink.
    if (missingTools.has(YT_DLP_TOOL)) {
      emit({ id: item.id, type: "failed", message: LINK_ERROR.notInstalled });
      return "failed";
    }
    /*
     * `?linkfail=` — the two fetch failures the UI has an affordance for.
     *
     * Both are classified by `link::classify_failure` from yt-dlp's stderr, and neither can be
     * provoked honestly here: one needs a Mac with no JavaScript runtime, the other a Mac without
     * Full Disk Access. They are failures of the *fetch*, so they land before a byte is downloaded,
     * exactly where `engine.fetch_link` would report them.
     *
     * The runtime one is YouTube's alone — Bilibili asks no challenges, so a Bilibili link in the
     * same batch still converts, which is also what keeps the failed row's helper question honest
     * about how many files it stopped.
     */
    if (KNOBS.linkFail === "jsruntime" && parsed.site === "youtube") {
      emit({ id: item.id, type: "failed", message: FETCH_FAILURE.noJsRuntime });
      return "failed";
    }
    if (KNOBS.linkFail === "safari") {
      emit({ id: item.id, type: "failed", message: FETCH_FAILURE.safariNeedsFullDiskAccess });
      return "failed";
    }
    /*
     * The sign-in wall, which is the only one of these knobs a *fix* can clear.
     *
     * `?linkfail=signin` does not say which ending to produce: the settings do. That is what makes
     * the recovery real rather than staged — the question borrows Chrome, the check agrees, the
     * rows are retried with the new settings, and this branch stops firing because there is now a
     * sign-in it can read. A knob that always failed would let a retry loop for ever.
     */
    if (KNOBS.linkFail === "signin" && parsed.site === "youtube") {
      const sign = signInState(batchSettings.link);
      if (sign.kind === "none") {
        emit({ id: item.id, type: "failed", message: FETCH_FAILURE.loginRequired });
        return "failed";
      }
      if (sign.kind === "unreadable") {
        emit({ id: item.id, type: "failed", message: sign.message });
        return "failed";
      }
    }
    // The jar opened and the site said no anyway: a stale account, and nothing this app can mend.
    if (KNOBS.linkFail === "refused" && parsed.site === "youtube") {
      emit({ id: item.id, type: "failed", message: FETCH_FAILURE.cookiesNotAccepted });
      return "failed";
    }
    const target = targetFormat(item.target_id);
    if (!target.available) {
      emit({
        id: item.id,
        type: "failed",
        message: missingToolMessage(item.target_id, target.helpers, target.needs),
      });
      return "failed";
    }

    const video = fakeVideo(url);
    const extension = outputExtensionFor(item.target_id);
    const output = `${linkDestination(batchSettings)}/${sanitizeTitle(video.title)}.${extension}`;
    if (batchSettings.output.on_conflict === "skip" && produced.has(output)) {
      emit({ id: item.id, type: "skipped", reason: "A converted file already exists" });
      return "skipped";
    }
    /*
     * A link is a video, and the trim applies to it exactly as it does to a dropped file — same
     * pre-flight refusal, same cut. The duration comes from the fetch rather than from a probe
     * (`engine.probe_link`, which is why a link row says nothing about its length until the batch
     * starts), but by the time the row is announced it is known, so the same two rules run here.
     */
    const timeBased = outputIsTimeBased(category, category === "video", item.target_id);
    const pastTheEnd = trimPastTheEnd(batchSettings.trim, video.secs, timeBased);
    if (pastTheEnd !== null) {
      emit({ id: item.id, type: "failed", message: pastTheEnd });
      return "failed";
    }
    const expected = expectedOutputSecs(batchSettings.trim, video.secs, timeBased) ?? video.secs;
    const share = video.secs > 0 ? Math.min(1, expected / video.secs) : 1;
    emit({
      id: item.id,
      type: "started",
      output,
      summary: `${SITE_LABEL[parsed.site]} → ${extension.toUpperCase()}`,
    });
    emit({ id: item.id, type: "progress", phase: "downloading", fraction: null, speed: null, eta_secs: null });

    const startedAt = Date.now();
    const seed = hashUnit(url + item.target_id);
    const phases: ReadonlyArray<{ phase: "downloading" | "converting"; steps: number }> = [
      // The fetch is of the whole video, trim or no trim: yt-dlp brings the file down and FFmpeg
      // cuts it afterwards, so only the second half of the job gets shorter.
      { phase: "downloading", steps: 10 + Math.floor(seed * 8) },
      { phase: "converting", steps: Math.max(4, Math.round((8 + Math.floor(seed * 10)) * share)) },
    ];
    for (const { phase, steps } of phases) {
      for (let step = 1; step <= steps; step += 1) {
        await sleep(KNOBS.linkMs);
        if (cancelled) {
          emit({ id: item.id, type: "skipped", reason: "Cancelled" });
          return "skipped";
        }
        emit({
          id: item.id,
          type: "progress",
          phase,
          fraction: step / steps,
          speed: phase === "downloading" ? null : 1.4 + seed,
          eta_secs: ((steps - step) * KNOBS.linkMs) / 1000,
        });
      }
    }

    produced.add(output);
    emit({
      id: item.id,
      type: "finished",
      outputs: [output],
      // A minute of video at the preset's rate, near enough that the row's size reads plausibly —
      // measured on the length that is kept, so a trimmed link lands as a small file.
      bytes: Math.max(120_000, Math.round(expected * 240_000)),
      elapsed_ms: Date.now() - startedAt,
    });
    return "ok";
  }

  async function runItem(item: BatchItemArg): Promise<"ok" | "failed" | "skipped"> {
    const url = item.url ?? "";
    if (url.trim() !== "") return runLink(item, url);
    const path = item.path ?? "";
    const batchSettings = cropMediaSettings(currentBatchSettings(), fakeProbe(path).category);
    const target = targetFormat(item.target_id);
    const output = mockOutputPath(path, item.target_id, batchSettings);
    const source = sourceFormat(path);

    // Same order as the real planner: a decoder we do not have beats an encoder we do not have.
    if (source !== null && !source.available) {
      emit({
        id: item.id,
        type: "failed",
        message: missingToolMessage(source.id, source.helpers, source.needs),
      });
      return "failed";
    }
    if (!target.available) {
      emit({
        id: item.id,
        type: "failed",
        message: missingToolMessage(item.target_id, target.helpers, target.needs),
      });
      return "failed";
    }
    // Then the route, which can need a helper neither format-level check asks about: PDF → TXT/HTML
    // is Poppler's own binary for that target, with LibreOffice behind it.
    const route = pdfTextCandidates(source?.id ?? null, item.target_id);
    if (route !== null && route.every((id) => missingTools.has(id))) {
      emit({ id: item.id, type: "failed", message: missingRouteMessage(item.target_id, route) });
      return "failed";
    }
    if (batchSettings.output.on_conflict === "skip" && produced.has(output)) {
      emit({ id: item.id, type: "skipped", reason: "A converted file already exists" });
      return "skipped";
    }

    const probe = fakeProbe(path);
    const timeBased = outputIsTimeBased(probe.category, probe.animated, item.target_id);
    // `queue::convert_one` refuses this where the other pre-flight refusals live: before the row is
    // announced and before anything is spawned, because `-ss` past the end writes a valid empty file
    // and an empty file reported as a success is the worst outcome available.
    const pastTheEnd = trimPastTheEnd(batchSettings.trim, probe.secs, timeBased);
    if (pastTheEnd !== null) {
      emit({ id: item.id, type: "failed", message: pastTheEnd });
      return "failed";
    }

    const seed = hashUnit(path + item.target_id);
    /*
     * How much of the source this job actually encodes, and therefore how long it takes.
     *
     * The engine is handed `expected_output_secs`, so under a trim the bar and the ETA are about the
     * cut: ten seconds out of a fifteen minute film is a job that finishes in a moment, not one that
     * crawls to 1% and stops. With trimming off the share is 1 and every timing here is what it has
     * always been.
     */
    const expected = expectedOutputSecs(batchSettings.trim, probe.secs, timeBased);
    const share =
      probe.secs !== null && probe.secs > 0 && expected !== null
        ? Math.min(1, expected / probe.secs)
        : 1;
    const steps = Math.max(4, Math.round((12 + Math.floor(seed * 18)) * share));
    const tick = 70 + Math.round(seed * 60);
    emit({
      id: item.id,
      type: "started",
      output,
      summary: `FFmpeg → ${outputExtensionFor(item.target_id)}`,
    });

    /*
     * A job nobody can measure reports no number at all.
     *
     * `Engine::run_step` is handed `expected_output_secs`, and when no duration was ever read - a
     * stream with nothing in its header, a growing recording - `step_fraction` answers `None` and
     * every sample carries `fraction: null` and no ETA. It used to substitute the start of the step
     * instead, which is where "Converting 0%" for a whole working job came from. The last word is
     * still a number, because a *finished* step is measured rather than guessed.
     *
     * Asked of the jobs that have a length to read at all: a JPEG has no duration either, and an
     * image queue drawn as a row of pulses would be a preview lying in the other direction.
     */
    const measurable = !(timeBased && probe.secs === null);

    const startedAt = Date.now();
    for (let step = 1; step <= steps; step += 1) {
      await sleep(tick);
      if (cancelled) {
        emit({ id: item.id, type: "skipped", reason: "Cancelled" });
        return "skipped";
      }
      const fraction = step / steps;
      emit({
        id: item.id,
        type: "progress",
        // A dropped file only ever converts, which is also what `Phase::default()` says.
        phase: "converting",
        fraction: measurable ? fraction : null,
        speed: 1.4 + seed,
        eta_secs: measurable ? ((steps - step) * tick) / 1000 : null,
      });
    }
    if (!measurable) {
      emit({
        id: item.id,
        type: "progress",
        phase: "converting",
        fraction: 1,
        speed: null,
        eta_secs: null,
      });
    }

    // One deterministic failure family, so the error + Retry path is reachable in the preview. The
    // wording is `EngineError::ToolFailed`'s Display impl — the real backend reports a tool's exit
    // status and its last line of stderr, and a preview that invents shorter errors makes the row
    // look roomier than it is.
    if (path.toLowerCase().includes("broken")) {
      emit({
        id: item.id,
        type: "failed",
        message: "ffmpeg exited with status 1: Invalid data found when processing input",
      });
      return "failed";
    }

    const sourceBytes = droppedSizes.get(path) ?? 40_000_000;
    const ratio =
      batchSettings.preset === "smallest" ? 0.18 : batchSettings.preset === "archive" ? 1.6 : 0.42;
    produced.add(output);
    emit({
      id: item.id,
      type: "finished",
      outputs: [output],
      // Times `share`: ten seconds of a fifteen minute film is a small file, and a row that reported
      // the whole film's size next to "Trimmed to 0:10" would argue with itself.
      bytes: Math.max(12_000, Math.round(sourceBytes * ratio * share)),
      elapsed_ms: Date.now() - startedAt,
    });
    return "ok";
  }

  async function runBatch(items: BatchItemArg[]): Promise<void> {
    const tally = { ok: 0, failed: 0, skipped: 0 };
    const queue = [...items];
    const workers = Math.max(1, Math.min(4, batchSettings.output.parallel_jobs || 2));
    const worker = async (): Promise<void> => {
      for (;;) {
        const item = queue.shift();
        if (item === undefined) return;
        if (cancelled) {
          tally.skipped += 1;
          emit({ id: item.id, type: "skipped", reason: "Cancelled" });
          continue;
        }
        tally[await runItem(item)] += 1;
      }
    };
    await Promise.all(Array.from({ length: workers }, worker));
    // The slot is freed by [`emit`], with the event, exactly as `commands::start_batch` frees it.
    emit({ type: "batch_finished", ...tally });
  }

  /** Browser drag & drop + file input, standing in for the native webview events. */
  function installDomBridge(): Unlisten {
    let depth = 0;
    const notify = (event: DropEvent): void => {
      for (const handler of dropHandlers) handler(event);
    };
    const onDragEnter = (e: DragEvent): void => {
      e.preventDefault();
      depth += 1;
      notify({ kind: "hover" });
    };
    const onDragOver = (e: DragEvent): void => {
      e.preventDefault();
      notify({ kind: "hover" });
    };
    const onDragLeave = (e: DragEvent): void => {
      e.preventDefault();
      depth = Math.max(0, depth - 1);
      if (depth === 0) notify({ kind: "leave" });
    };
    const onDrop = (e: DragEvent): void => {
      e.preventDefault();
      depth = 0;
      const files = Array.from(e.dataTransfer?.files ?? []);
      const paths = files.map((file) => {
        const path = pseudoPath(file.name);
        droppedSizes.set(path, file.size);
        return path;
      });
      notify(paths.length > 0 ? { kind: "drop", paths } : { kind: "leave" });
    };
    window.addEventListener("dragenter", onDragEnter);
    window.addEventListener("dragover", onDragOver);
    window.addEventListener("dragleave", onDragLeave);
    window.addEventListener("drop", onDrop);
    return () => {
      window.removeEventListener("dragenter", onDragEnter);
      window.removeEventListener("dragover", onDragOver);
      window.removeEventListener("dragleave", onDragLeave);
      window.removeEventListener("drop", onDrop);
    };
  }

  const chooseWithInput = (directory: boolean): Promise<string[]> =>
    new Promise((resolve) => {
      const input = document.createElement("input");
      input.type = "file";
      input.multiple = true;
      if (directory) input.setAttribute("webkitdirectory", "");
      input.style.display = "none";
      input.addEventListener("change", () => {
        const chosen = Array.from(input.files ?? []);
        const relative = chosen[0]?.webkitRelativePath ?? "";
        const folder = `${MOCK_DIR}/${relative.split("/")[0] || "Folder"}`;
        const paths = chosen.map((file) => {
          const path = `${directory ? folder : MOCK_DIR}/${file.name}`;
          droppedSizes.set(path, file.size);
          return path;
        });
        if (directory) directoryContents.set(folder, paths);
        input.remove();
        resolve(paths);
      });
      // A cancelled picker never fires `change`; `cancel` is widely supported and keeps no promise
      // hanging around forever.
      input.addEventListener("cancel", () => {
        input.remove();
        resolve([]);
      });
      document.body.append(input);
      input.click();
    });

  /**
   * The state a webview reload lands in (`?activity=`): work the *shell* is already doing, which
   * this page never started and has no rows for.
   *
   * Rust keeps the batch and the install alive across a reload — one slot each, held by a thread the
   * new page cannot see. The events that page goes on to receive carry ids from before the reload
   * (`commands::next_id` is per-process and monotonic, so they can never match a row it knows), and
   * `get_activity` is the only way it can find out at all. Both are simulated through the real
   * emitters and the real flags, so `start_batch` really is refused while this runs and
   * `cancel_batch` really does end it.
   */
  function startInheritedWork(): void {
    if (KNOBS.inherited.converting) {
      running = true;
      const id = "f19a2c4d-7"; // the shape `commands::next_id` produces, from the previous page load
      const output = `${MOCK_DIR}/before-the-reload.mp4`;
      void (async () => {
        emit({ id, type: "started", output, summary: "FFmpeg → mp4" });
        const steps = 240;
        for (let step = 1; step <= steps && !cancelled; step += 1) {
          await sleep(250);
          const left = steps - step;
          emit({
            id,
            type: "progress",
            phase: "converting",
            fraction: step / steps,
            speed: 1.5,
            eta_secs: left / 4,
          });
        }
        if (cancelled) emit({ id, type: "skipped", reason: "Cancelled" });
        else emit({ id, type: "finished", outputs: [output], bytes: 8_400_000, elapsed_ms: 60_000 });
        const tally = cancelled ? { ok: 0, failed: 0, skipped: 1 } : { ok: 1, failed: 0, skipped: 0 };
        emit({ type: "batch_finished", ...tally });
      })();
    }
    if (KNOBS.inherited.installing) {
      installing = true;
      const plan = installPlans().find((p) => p.package_id === "pandoc");
      // `Activity` is two booleans: nothing names the tool until its first event arrives.
      if (plan !== undefined) void sleep(KNOBS.lineMs * 3).then(() => runInstall(plan));
    }
  }

  startInheritedWork();

  return {
    getCatalog: async (): Promise<CatalogView> => {
      await sleep(80);
      return catalogView();
    },
    getSettings: async () => ({ ...settings }),
    saveSettings: async (next) => {
      const bad = checkOutput(next.output);
      const unfinished =
        (next.output.location === "custom" && (next.output.custom_dir ?? "").trim() === "") ||
        (next.output.location === "subfolder" && next.output.subfolder_name.trim() === "");
      const output = bad === null ? next.output : {
        ...next.output,
        location: settings.output.location,
        custom_dir: settings.output.custom_dir,
        subfolder_name: settings.output.subfolder_name,
      };
      /*
       * `settings_store::merge`, for the trim: a value we would refuse is held at the last usable
       * one *and* reported, while one that is merely unfinished is held in silence — nothing is
       * said, and every other edit in the same save is still written. Storing `enabled: true`
       * beside a held length would trim the batch to a number the user never typed.
       */
      const verdict = classifyTrim(next.trim);
      const trim = verdict.kind === "usable" ? next.trim : settings.trim;
      /*
       * And the same merge again, independently, for the sign-in: a browser this app would refuse
       * holds the last usable *sign-in* — leaving the trim, the codec and everything else in the
       * same save alone — while a mode chosen before the browser or the file behind it is held with
       * nothing said. Storing `cookies: "browser"` beside a held browser name would tell yt-dlp to
       * borrow a sign-in from a browser the user never named.
       */
      const link = classifyLink(next.link);
      if (link.kind === "usable") usableLink = next.link;
      settings = { ...next, output, trim, link: usableLink };
      saved.push({ ...settings });
      if (bad !== null && !unfinished) throw bad;
      if (verdict.kind === "refused") throw verdict.why;
      if (link.kind === "refused") throw link.why;
      return { ...settings };
    },
    applyPreset: async (presetId) => {
      // `commands::apply_preset`: the preset's own settings, with the current trim and the current
      // sign-in carried across — neither is an opinion about quality, and neither may be lost to a
      // click on "Smallest".
      settings = { ...presetSettings(presetId), trim: settings.trim, link: settings.link };
      return { ...settings };
    },
    inspectFiles: async (paths) => {
      await sleep(140);
      const walk = walkPaths(paths);
      return { files: walk.files.map(describe), truncated: walk.truncated, limit: KNOBS.maxFiles };
    },
    estimateOutputPath: async (path, targetId, current) => {
      // Recorded before anything else can refuse it: what this command is *asked* is the fact the
      // suite is after, and a link row has no path to ask it about — see the seam below.
      estimated.push(path);
      const bad = checkOutput(current.output);
      if (bad !== null) throw bad;
      return mockOutputPath(path, targetId, current);
    },
    /*
     * `commands::inspect_links`: pure, instant, and no network at all.
     *
     * Blank lines are dropped rather than refused (a paste ends in a newline), and the whole paste is
     * refused over the cap — twenty-one links is a mistake to correct, not twenty-one rows to render.
     * A line that *is* refused still comes back as a row, because a paste of twelve with one channel
     * URL in it has to say which line is the problem.
     */
    inspectLinks: async (links): Promise<LinkInspection> => {
      const lines = links.map((line) => line.trim()).filter((line) => line !== "");
      if (lines.length > KNOBS.maxLinks) {
        throw LINK_ERROR.tooMany(lines.length, KNOBS.maxLinks);
      }
      const rows: LinkRow[] = lines.map((line) => {
        const parsed = parseLink(line);
        if ("error" in parsed) {
          return {
            id: nextId(),
            url: printableLink(line),
            site: null,
            site_label: null,
            category: null,
            supported: false,
            default_target: null,
            suggested_targets: [],
            note: parsed.error,
          };
        }
        const category = linkCategory(parsed.site);
        const group = MOCK_CATALOG.categories.find((c) => c.id === category);
        return {
          id: nextId(),
          url: parsed.url,
          site: parsed.site,
          site_label: SITE_LABEL[parsed.site],
          category,
          supported: true,
          default_target: group?.default_target ?? null,
          suggested_targets: group?.suggested_targets ?? [],
          note: null,
        };
      });
      return {
        links: rows,
        accepted: rows.filter((row) => row.supported).length,
        limit: KNOBS.maxLinks,
      };
    },
    getLinkSupport: async (current): Promise<LinkSupport> => ({
      max_links: KNOBS.maxLinks,
      accepted_hosts: ACCEPTED_HOSTS.map(([host]) => host),
      tool_installed: !missingTools.has(YT_DLP_TOOL),
      package_id: YT_DLP_TOOL,
      destination: linkDestination(current),
    }),
    /*
     * The refusals, in the order `commands::start_batch` reaches them — the order matters, because
     * it decides which sentence the user gets when two things are wrong at once, and because
     * everything that could refuse the batch is checked *before* the single slot is claimed.
     */
    startBatch: async (items, current) => {
      if (current.crop) {
        const invalid = cropError(current.crop);
        if (invalid) throw invalid;
      }
      if (items.length === 0) throw "Nothing to convert";
      const bad = checkOutput(current.output);
      if (bad !== null) throw bad;
      // A trim with no length is the same kind of "not yet" as a destination with no folder: the
      // planner would ignore it and convert every file in full, which is not what the window says.
      const badTrim = checkTrim(current.trim);
      if (badTrim !== null) throw badTrim;
      // And a sign-in with no browser or no file chosen is the same "not yet" again: the fetch would
      // send no sign-in at all while the sheet says it is borrowing one, and the failure the user
      // then reads would blame the video.
      const badCookies = checkCookies(current.link);
      if (badCookies !== null && items.some((item) => (item.url ?? "").trim() !== "")) throw badCookies;
      // `commands::jobs_from`, refusal for refusal: the target first, then exactly one source.
      for (const item of items) {
        if (!knownFormat(item.target_id)) throw `Unknown output format \`${item.target_id}\``;
        const path = (item.path ?? "").trim();
        const url = (item.url ?? "").trim();
        if (path !== "" && url !== "") {
          throw "A queued row has both a file and a link; it must have one.";
        }
        if (path === "" && url === "") throw "A queued row has no file and no link.";
        if (url !== "") {
          const parsed = parseLink(url);
          if ("error" in parsed) throw parsed.error;
        }
      }
      // Over the link rows only: a queue of two thousand files and one link is one link.
      const links = items.filter((item) => (item.url ?? "").trim() !== "").length;
      if (links > KNOBS.maxLinks) throw LINK_ERROR.tooMany(links, KNOBS.maxLinks);
      if (running) throw "A conversion is already running. Cancel it first.";
      batchSettings = { ...current };
      running = true;
      cancelled = false;
      void runBatch(items);
    },
    cancelBatch: async () => {
      cancelled = true;
    },
    /*
     * `AppState::activity()`: two booleans, read off the batch and install slots.
     *
     * `?activity=stale` spends its one lie here — the shell was busy when it was asked and idle by
     * the time the answer arrived, which is the race the store's second read exists for.
     */
    getActivity: async () => {
      if (staleActivity) {
        staleActivity = false;
        return { converting: true, installing: true };
      }
      return { converting: running, installing };
    },
    refreshTools: async () => {
      await sleep(400);
      return toolStatuses();
    },
    getInstallPlans: async () => {
      await sleep(60);
      return installPlans();
    },
    /*
     * The four refusals are the Rust ones, word for word, because they are what the UI renders when
     * a click cannot possibly work: an install already running, an id that is not a helper, a
     * helper there is nothing to install, and no Homebrew to install it with.
     */
    installPackage: async (packageId) => {
      if (installing) throw "Another helper is being installed. Wait for it to finish.";
      const plan = installPlans().find((p) => p.package_id === packageId);
      if (plan === undefined) {
        // A *binary* id asked for by mistake gets the answer `install::resolve_install_with` gives
        // it: a member of an installable package is redirected to the package, and a bundled or
        // built-in program is told there is nothing to install at all.
        const member = packageOf(packageId);
        if (member !== undefined) {
          throw (
            `${member.name} is installed as one package, not one program at a time - ask for ` +
            `${member.name} (${installHintFor(member.tool_ids[0] ?? "")}).`
          );
        }
        const known = TOOL_BY_ID.get(packageId);
        if (known !== undefined) {
          throw `There is nothing to install: ${userFacingName(packageId)} - ${known.install_hint}`;
        }
        throw `\`${packageId}\` is not a helper Flint can install.`;
      }
      if (plan.manager === "none") {
        throw `There is nothing to install: ${plan.name} - ${installHintFor(plan.tool_ids[0] ?? "")}`;
      }
      if (!plan.manager_available) {
        throw (
          `Homebrew is not installed, so Flint cannot install ${plan.name} for you. ` +
          `Install Homebrew from https://brew.sh first (it will ask for your password in Terminal), ` +
          `then come back and click Install - or run \`${plan.command}\` in Terminal yourself.`
        );
      }
      installing = true;
      void runInstall(plan);
    },
    // No Finder in a browser tab: succeed quietly rather than fake an error the UI would nag about.
    revealInFinder: async () => {},
    openPath: async () => {},
    /*
     * No System Settings either — and nothing to record: the command takes no argument, so there is
     * nothing a suite could assert about *what* was opened that is not already a constant in Rust.
     * The click is still worth answering, because a rejection here would put a toast over a row that
     * has just done the one thing left to do.
     */
    openFullDiskAccessSettings: async () => {},
    /**
     * `list_cookie_browsers`: the allowlist, in order, with what `?browsers=` says is here and what
     * `?mac=` says is in each jar.
     */
    listCookieBrowsers: async () => browserPresence(),
    /*
     * `check_safari_cookie_access`, and the one thing about it a mock must not get wrong: it does
     * not wait. There is no `sleep` here, because the command it stands for is a single `open(2)`,
     * and a preview that made the user watch a spinner for it would hide the whole improvement.
     */
    checkSafariCookieAccess: async (): Promise<SafariAccess> => safariAccess(),
    /*
     * `test_cookie_source`, with the one difference a browser tab cannot avoid: there is no yt-dlp
     * and no network, so the verdict is derived from the settings and `?browsers=` rather than
     * measured. Everything around it is the command's own shape — the same refusal gate, the same
     * five verdicts, the same sentences, and the same `not_configured` answer given without
     * pretending to probe anything.
     *
     * The wait is real, and deliberately so: the check is the only thing in this app that makes a
     * user sit still for a second, and a UI developed against an instant answer would never grow
     * the "Checking…" state that makes the wait bearable.
     */
    testCookieSource: async (current, url): Promise<CookieTest> => {
      if (url && "error" in parseLink(url)) throw "Choose a supported media link to check.";
      const link = current.link;
      const verdict = (result: CookieCheck, message: string): CookieTest => ({
        result,
        ok: result === "working",
        message,
        tested_url: url ?? COOKIE_TEST_URL,
      });
      if (link.cookies === "none") {
        return verdict("not_configured", COOKIE_TEST.notConfigured);
      }
      // A half-made or unallowlisted source is refused in the settings page's own words, before
      // anything is run — a sentence about the settings is not a verdict about a sign-in.
      const refusal = checkCookies(link);
      if (refusal !== null) throw refusal;
      probed.push(link.cookies === "browser" ? link.cookie_browser : "file");
      await sleep(400);
      const state = signInState(link);
      const measured: CookieCheck = state.kind === "usable" ? "working" : "unreadable";
      const result = KNOBS.signIn ?? measured;
      switch (result) {
        case "working":
          return verdict(result, COOKIE_TEST.works);
        case "unreadable":
          return verdict(
            result,
            state.kind === "unreadable" ? state.message : FETCH_FAILURE.browserCookiesUnreadable,
          );
        case "refused":
          return verdict(result, FETCH_FAILURE.cookiesNotAccepted);
        case "inconclusive":
          return verdict(result, COOKIE_TEST.timedOut);
        case "not_configured":
          return verdict(result, COOKIE_TEST.notConfigured);
      }
    },
    onBatchEvent: async (handler) => {
      batchHandlers.add(handler);
      return () => batchHandlers.delete(handler);
    },
    onInstallEvent: async (handler) => {
      installHandlers.add(handler);
      return () => installHandlers.delete(handler);
    },
    onDragDrop: async (handler) => {
      dropHandlers.add(handler);
      const teardown = installDomBridge();
      return () => {
        dropHandlers.delete(handler);
        teardown();
      };
    },
    onMenuAction: async (handler) => {
      const onKey = (event: KeyboardEvent): void => {
        // `App` binds the same combinations when it is not running under Tauri; whichever listener
        // reaches the event first claims it, and the other one steps aside. One ⌘O, one dialog.
        if (event.defaultPrevented) return;
        const action = menuActionForKey(event);
        if (action === null) return;
        event.preventDefault();
        handler(action);
      };
      window.addEventListener("keydown", onKey);
      return () => window.removeEventListener("keydown", onKey);
    },
    pickFiles: () => chooseWithInput(false),
    pickDirectory: async () => {
      const paths = await chooseWithInput(true);
      const first = paths[0];
      return first === undefined ? null : parentOf(first);
    },
  };
}
