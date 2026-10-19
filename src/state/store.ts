/**
 * The whole app state. Components read slices from here; nothing else holds conversion state.
 *
 * Four rules keep the event stream honest, because a batch runs on several Rust threads and the
 * `batch://event` listener is a window-wide subscription that outlives any single run:
 *   1. events are only accepted while this window believes a batch is running, and only for ids
 *      that belong to *that* batch — a stream from a run the UI has already retired (a webview
 *      reload mid-batch, or a `start_batch` that was refused because Rust still owned the slot)
 *      must not retarget the current queue;
 *   2. events for ids we do not know are dropped (a row can be removed mid-batch);
 *   3. `started` only ever promotes a queued row, so a duplicate cannot rewind a bar or resurrect
 *      a row that already finished;
 *   4. progress and terminal events for a row that already finished/failed/was skipped are
 *      ignored — every row ends in exactly one terminal state, whatever the stream does.
 *
 * A reload is the awkward case those rules are written for: the shell keeps converting, so `init`
 * asks `get_activity` and adopts what it finds (`State.inheritedBatch`, `State.foreignInstall`) —
 * the window can then show and stop work whose row ids it will never recognise.
 */
import { create } from "zustand";
import { cropApplies, cropError, cropRunSettings } from "../lib/crop";
import {
  linkCapNotice,
  linkCapRefusal,
  linkDuplicateNotice,
  parentDir,
  pluralLinks,
  shortUrl,
  truncationNotice,
} from "../lib/format";
import { planNamed, presenceOf } from "../lib/packages";
import * as ipc from "../lib/ipc";
import { errorMessage } from "../lib/ipc";
import type {
  BatchEvent,
  BatchItemArg,
  BrowserPresence,
  CatalogView,
  CategoryId,
  CookieCheck,
  CropSettings,
  FileInfo,
  FormatView,
  InstallEvent,
  LinkInspection,
  LinkRow,
  LinkSettings,
  LinkSupport,
  PackageInstallPlan,
  Phase,
  PresetId,
  Settings,
  ToolStatus,
} from "../lib/types";

export type RowStatus = "queued" | "running" | "done" | "failed" | "skipped";
export type BatchPhase = "idle" | "running" | "finished";

export interface Row {
  runCrop?: CropSettings | null;
  info: FileInfo;
  /**
   * The pasted link this row came from, or null for a dropped file.
   *
   * Non-null is what makes a row a link, everywhere: it decides whether `start` sends a `url` or a
   * `path`, which phase the row begins in, and whether its blocker is a catalog format or yt-dlp.
   * `info` is then a *synthesised* descriptor (see [`linkRow`]) with an empty `path`, because there
   * is no file yet and nothing may treat the row as if there were.
   */
  link: LinkRow | null;
  /** Chosen output format id. Empty for unsupported files, which never enter a batch. */
  target: string;
  status: RowStatus;
  /** Which half of the job the last sample was about: a link downloads before it converts. */
  phase: Phase;
  fraction: number | null;
  eta_secs: number | null;
  output: string | null;
  bytes: number | null;
  /** Failure message or skip reason, whichever applies. */
  message: string | null;
}

export interface BatchSummary {
  ok: number;
  failed: number;
  skipped: number;
}

export type InstallStatus = "running" | "ok" | "failed";

/**
 * The one install this window is showing. Kept after it ends so its verdict survives the row going
 * quiet: a helper that installs successfully stops being "missing" the instant `refreshTools`
 * returns, and a success message that vanished with the row would be no message at all.
 */
export interface InstallRun {
  /** The *package* being installed — what the user clicked, and what every event is keyed by. */
  packageId: string;
  status: InstallStatus;
  /** Installer output, oldest first, capped at [`LOG_LINES`]. */
  lines: string[];
  /** The `finished` message, verbatim. Null while it is still running. */
  message: string | null;
}

/**
 * How much installer output is kept. `brew install --cask libreoffice` writes a few dozen lines;
 * a pathological run must not grow the store without bound, and nobody scrolls back further.
 */
const LOG_LINES = 400;

/**
 * A helper a row is waiting on, named as the *package* a user would install.
 *
 * `packageId` is what `install_tool` takes and what the settings list highlights; `name` is what the
 * row says out loud ("Poppler"). The binary behind it (`pdftohtml`) never reaches this interface: it
 * is a diagnostic, and a microlink offering to install one is offering something nobody sells.
 */
export interface MissingHelper {
  packageId: string;
  name: string;
  /** The format that package reads or writes, named as the catalog names it ("PDF"). */
  format: string;
}

/** One package a settled batch ran into, and what it cost that batch. */
export interface BlockedHelper {
  packageId: string;
  name: string;
  /** How many rows in that batch this one helper stopped. */
  files: number;
  /** The formats those rows could not read or write, most-blocked first. */
  formats: string[];
}

/**
 * The question raised when a batch settles having been blocked by a helper nobody installed.
 *
 * It is only ever a question. Yes walks the user to that helper in Settings — the same trip the
 * failed row's microlink makes — and the install is still theirs to start.
 */
export interface InstallPrompt {
  /** Implicated helpers, worst blocker first; `tools[0]` is what the primary action targets. */
  tools: BlockedHelper[];
  /** Rows a missing helper stopped, across every tool named here. */
  files: number;
}

/**
 * Why a link row failed, when the reason was the sign-in.
 *
 * Three states with three different fixes, which is the entire point of telling them apart:
 *
 *   - `wall` — the site asked for a sign-in and this app had none to give
 *     (`FetchFailure::LoginRequired`, `AgeRestricted`). The fix is to *have* one.
 *   - `unreadable` — a source is configured and the jar never opened
 *     (`BrowserCookiesUnreadable`, `CookiesFileUnreadable`, a refused Keychain prompt). The fix is
 *     a different source, or the permission the guide names.
 *   - `refused` — the jar opened, the cookies went out, and the site said no anyway
 *     (`CookiesNotAccepted`). The fix is a fresh sign-in in that browser, not a setting.
 *
 * Safari's Full Disk Access failure is deliberately none of these: it has an affordance of its own
 * ([`needsFullDiskAccess`]), and a row must never carry two competing ways out.
 */
export type SignInFailure = "wall" | "unreadable" | "refused";

/**
 * What this Mac can actually be offered as a way past a sign-in wall.
 *
 * Measured, not listed: a dropdown of eight browsers on a machine with two is six dead options, and
 * the *shape* of the offer changes with what is there, so it is decided once here and read
 * everywhere. Which browser is named comes from Rust's ranking ([`preferredBrowser`]) — the one the
 * sign-in is likeliest to be in — and never from which one is cheapest for this app to read.
 */
export type SignInRemedy =
  /** Borrow the sign-in from the browser this Mac's evidence points at, in one click. */
  | { kind: "browser"; id: string; label: string; reason: string }
  /**
   * That browser is Safari, whose jar is behind Full Disk Access, so one click cannot do it.
   *
   * The permission is stated as the price of the *right* browser. It used to mean "Safari is all
   * there is", which is a different thing and was the wrong answer on a Mac that also had a Chrome
   * nobody had ever signed in to.
   */
  | { kind: "full_disk_access"; id: string; label: string; reason: string }
  /** No browser here has a sign-in to lend: the only route left is a cookies.txt the user exports. */
  | { kind: "cookie_file" }
  /**
   * The app is *already* using the browser it would otherwise offer, and it did not work.
   *
   * Pressing "Use Chrome" on a machine that is reading Chrome's cookies right now would do exactly
   * what has just failed, dressed as a fix — so the offer becomes the walkthrough, where a stale
   * sign-in, a refused Keychain prompt and the cookies.txt route are each said out loud and the
   * check can prove whichever of them the user tries.
   */
  | { kind: "guide" };

/**
 * The question raised when a batch settles having been stopped by a sign-in this app does not have.
 *
 * The helper question's twin ([`InstallPrompt`]), and asked by the same rules: once per batch,
 * however many rows it stopped, and never when asking would be noise. What differs is what Yes
 * does — a helper costs four minutes and 350 MB and stays the user's to start, while borrowing a
 * sign-in costs nothing and can be *checked*, so this one does the work: it saves the source,
 * verifies it with `test_cookie_source`, and retries the rows that were stopped when it works.
 */
export interface SignInPrompt {
  /** How many rows a sign-in wall stopped. Links, because only a link can hit one. */
  links: number;
  /** Exactly those rows, so accepting retries them and nothing else in the queue. */
  ids: string[];
  remedy: SignInRemedy;
}

/**
 * The verdict of one `test_cookie_source`, shown wherever the check was asked for.
 *
 * The answer the old red row could not give: a user who changed something in Settings → Links had
 * no way to find out whether it helped short of running a conversion. `result` is null while the
 * probe is in flight and for a refusal the backend gave before it ran anything — a sentence about
 * half-made settings is not a verdict about a sign-in, and it must not be dressed as one.
 */
export interface SignInCheck {
  checking: boolean;
  result: CookieCheck | null;
  message: string;
}

interface State {
  cropOpen: boolean;
  setCropOpen: (open: boolean) => void;
  initializationFailed: boolean;
  catalog: CatalogView | null;
  settings: Settings | null;
  files: Record<string, Row>;
  order: string[];
  /**
   * True once this window has held a file.
   *
   * The empty canvas has two meanings, and they want different behaviour from the keyboard: a first
   * launch, which is meant to be untouched, and a queue the user has just emptied, where the control
   * they were standing on has gone and something has to take the focus. This is the only thing that
   * tells them apart.
   */
  hasHeldFiles: boolean;
  /**
   * Ids taking part in the current/last batch — what the aggregate progress is measured over.
   *
   * A set, not a list: every batch event and every row's render asks "is this id in the batch",
   * which over a 2000-file queue is 2000 linear scans per progress tick.
   */
  batchIds: ReadonlySet<string>;
  phase: BatchPhase;
  /**
   * The running batch belongs to a page load this one replaced: `get_activity` said the shell was
   * still converting, so there are rows being worked on that this window has no `Row` for and no
   * `batchIds` for. It may be stopped and it must not be summarised — the tally is not ours.
   */
  inheritedBatch: boolean;
  /** True between "Stop" and the batch actually winding down, so the button cannot be re-armed. */
  stopping: boolean;
  /**
   * The user asked this batch to stop, and the cancel was accepted.
   *
   * Distinct from `stopping`, which is cleared the moment a cancel *fails*: this one survives
   * until the batch is over, because a deliberate stop must never be mistaken for a batch that
   * failed and be answered with a prompt.
   */
  stopRequested: boolean;
  summary: BatchSummary | null;
  drawerOpen: boolean;
  drawerPage: "conversion" | "skin";
  /**
   * One install plan per *package*, in the backend's order. Empty until `init` (or on an older
   * backend). Presence is not in here: it is derived by joining `tool_ids` against the tool list
   * (`lib/packages.ts`), because a package with only some of its binaries is not installed.
   */
  installPlans: PackageInstallPlan[];
  /** The install in flight, or the last one's verdict. Only ever one: the backend enforces it. */
  install: InstallRun | null;
  /**
   * An install the *shell* is running that this window did not start and cannot yet name:
   * `get_activity` answers "installing: true" without saying which helper. Every Install button is
   * disabled until its first `install://event` arrives and [`Actions.handleInstallEvent`] adopts it.
   */
  foreignInstall: boolean;
  /** A package the user was sent to from a failed row, so the settings list can point at it. */
  highlightedPackage: string | null;
  /** The "shall I take you to that helper?" question, or null when nothing is being asked. */
  installPrompt: InstallPrompt | null;
  /**
   * Packages this session has already been asked about. Told once is enough: the failed row's
   * microlink stays as the always-available route, and a package that gets installed drops out of
   * the question anyway because it stops being missing.
   */
  askedPackages: string[];
  dragging: boolean;
  busy: boolean;
  selectedId: string | null;
  error: string | null;
  /**
   * Something worth saying that is not a failure, shown in the same slot as `error` and in a
   * quieter hand: at present only a drop the enumeration cap cut short (`Inspection.truncated`).
   *
   * Each inspection answers this afresh — a notice that outlived the drop it described would send
   * the user looking for missing files in a queue that has all of them.
   */
  notice: string | null;
  bannerDismissed: boolean;
  /**
   * Where the queue's output will land, or null while nothing can be said.
   *
   * Two shapes on purpose: a file row's own `estimate_output_path` (a full output path, whose parent
   * is the folder) or — for a queue of nothing but links, which has no source file to estimate from
   * — `LinkSupport.destination`, which is already the folder. [`destinationFolder`] is the one place
   * that tells them apart.
   */
  destination: string | null;
  /** Is the paste box up? A modal over the same window, exactly like the install prompt. */
  linksOpen: boolean;
  /** Text the box opens with: the clipboard, for the ⌘V-anywhere path. Empty when opened bare. */
  linksPrefill: string;
  /**
   * A refusal about what is in the paste box — at present only "you pasted too many".
   *
   * The sheet's own line, not the global toast: the box is covering the window the toast appears at
   * the bottom of, and a message about what was typed belongs beside what was typed.
   */
  linksError: string | null;
  /** The cap, the accepted hosts, whether yt-dlp is here, and where links land. Null until asked. */
  linkSupport: LinkSupport | null;
  /**
   * Every allowlisted browser and whether this machine has it — `list_cookie_browsers`, verbatim.
   *
   * Empty until it answers, and empty forever on a shell too old to have the command. Both are the
   * same thing to everything downstream: *nothing is known*, so no browser is named out loud and no
   * option is marked as absent. Guessing "Chrome, probably" would be the confident wrong answer
   * this app does not give.
   */
  cookieBrowsers: BrowserPresence[];
  /** The "shall I borrow your Chrome sign-in?" question, or null when nothing is being asked. */
  signInPrompt: SignInPrompt | null;
  /**
   * Told once is enough, exactly as [`State.askedPackages`] is: the failed row's own action and
   * Settings → Links both stay as the always-available route, so a second card would be nagging.
   */
  askedAboutSignIn: boolean;
  /** Is the guided sign-in sheet up? A modal over the same window, like the paste box. */
  signInOpen: boolean;
  /**
   * Was the guided sheet opened from inside Settings?
   *
   * Sheets in this app replace each other rather than stack ([`Actions.setDrawer`] documents why),
   * so opening the guide from Settings → Links closes Settings — and closing the guide has to put
   * the user back where they were standing, or the route out of Settings would be a route away
   * from it.
   */
  signInFromSettings: boolean;
  /** The last sign-in check's verdict, or null when none has been asked for. */
  signInCheck: SignInCheck | null;
}

interface Actions {
  init: () => Promise<void>;
  addPaths: (paths: string[]) => Promise<void>;
  openFiles: () => Promise<void>;
  openFolder: () => Promise<void>;
  /**
   * Open the paste box, optionally with text already in it (the clipboard, for ⌘V anywhere).
   *
   * Refused while the install question is up — that has to be answered first — and it closes the
   * settings sheet rather than stacking over it: two modals over one window is a mistake this app
   * already made and documented (see [`Actions.setDrawer`]).
   */
  openLinks: (prefill?: string) => void;
  closeLinks: () => void;
  /**
   * Judge what is in the box, line by line. Null when the backend refused the whole paste, whose
   * message goes on the sheet's own error line rather than into the global toast.
   */
  checkLinks: (lines: string[]) => Promise<LinkInspection | null>;
  /**
   * Drop a refusal about a paste that no longer exists.
   *
   * `checkLinks` clears it on the next answer, but an *empty* box is never sent to the backend at
   * all — so backspacing over the twenty-first link left "You pasted 21 links" standing beside a box
   * with nothing in it, which is the sheet describing text the user has already deleted.
   */
  clearLinksError: () => void;
  /** Queue the accepted links, deduped by URL and capped over the whole queue. Closes the box. */
  addLinks: (rows: LinkRow[]) => void;
  removeFile: (id: string) => void;
  clearAll: () => void;
  selectFile: (id: string | null) => void;
  setTarget: (id: string, target: string) => void;
  setCategoryTarget: (category: CategoryId, target: string) => void;
  start: () => Promise<void>;
  startCrop: (crop: CropSettings) => Promise<void>;
  stop: () => Promise<void>;
  retry: (id: string) => Promise<void>;
  /**
   * Run these rows again, as one batch — what "Retry" does, for more than one row.
   *
   * The rows are named by id rather than by predicate because the caller has already decided which
   * ones it means: the automatic retry after a sign-in is fixed must touch exactly the rows that
   * sign-in stopped, and not a row the user failed on their own account a minute earlier.
   */
  retryRows: (ids: string[]) => Promise<void>;
  patchSettings: (next: Settings) => void;
  choosePreset: (preset: PresetId) => Promise<void>;
  resetSettings: () => Promise<void>;
  pickOutputDir: () => Promise<void>;
  /**
   * Choose the exported cookies.txt a pasted link should read the sign-in from.
   *
   * Switches the group into "from a file" as it lands, exactly as choosing a folder switches the
   * destination to "in a folder I choose": the user pointed at a file, so pointing at it is the
   * choice. Only the path is kept — see [`LinkSettings.cookie_file`].
   */
  pickCookieFile: () => Promise<void>;
  refreshTools: () => Promise<void>;
  installPackage: (packageId: string) => Promise<void>;
  /** Forget a finished install's verdict, so the helper list goes quiet again. */
  dismissInstall: () => void;
  /** Open settings on a particular package — the path out of a row that failed for want of it. */
  showPackage: (packageId: string) => void;
  /** "Yes": close the question and make exactly that trip into Settings. Installs nothing. */
  confirmInstallPrompt: () => void;
  /** "Not now": close the question, and do not ask about those helpers again this session. */
  dismissInstallPrompt: () => void;
  setDrawer: (open: boolean) => void;
  openSkin: () => void;
  showConversionSettings: () => void;
  dismissBanner: () => void;
  clearError: () => void;
  clearNotice: () => void;
  reveal: (path: string) => Promise<void>;
  open: (path: string) => Promise<void>;
  /**
   * Open System Settings → Privacy & Security → Full Disk Access, the one thing that fixes a Safari
   * cookie jar macOS will not let this app read ([`needsFullDiskAccess`]).
   *
   * Takes nothing, and the command behind it takes nothing either: the URL is a constant in Rust, so
   * this route cannot be talked into opening anything else.
   */
  openFullDiskAccess: () => Promise<void>;
  /**
   * Borrow the sign-in from this browser, then find out whether that actually worked.
   *
   * One call because they are one intention: choosing a browser and then having to go and prove it
   * yourself is the dead end this whole flow replaces. Resolves true only when the probe came back
   * `working`, which is the one fact a caller may act on.
   */
  useBrowserSignIn: (browserId: string) => Promise<boolean>;
  /** Run `test_cookie_source` over the settings as they stand and record the verdict in place. */
  checkSignIn: () => Promise<boolean>;
  /** Open the guided sheet — from a failed row, from the question, or from Settings → Links. */
  openSignInGuide: () => void;
  closeSignInGuide: () => void;
  /**
   * "Yes": do the whole thing — save the source, check it, and retry the rows the wall stopped.
   *
   * Where the helper question hands over and stops, this one finishes the job, because it can: a
   * sign-in check is two seconds and a public video, and there is nothing to install and nothing to
   * pay. When the check says otherwise, the guided sheet opens with that verdict still standing.
   */
  confirmSignInPrompt: () => Promise<void>;
  /** "Not now": close the question, and do not raise it again this session. */
  dismissSignInPrompt: () => void;
  handleEvent: (event: BatchEvent) => void;
  handleInstallEvent: (event: InstallEvent) => void;
}

export type Store = State & Actions;

const TERMINAL: ReadonlySet<RowStatus> = new Set<RowStatus>(["done", "failed", "skipped"]);

/**
 * The link cap to assume before `get_link_support` has answered — `link::MAX_LINKS_PER_BATCH`.
 *
 * Only ever a fallback: every number a user reads comes from [`State.linkSupport`], and the core
 * enforces the real cap whatever this side believes.
 */
const FALLBACK_MAX_LINKS = 20;

/**
 * The binary a link needs, as `Tool::YtDlp` and `package::YT_DLP` both spell it.
 *
 * The join key for a link row's blocker: no catalog format names yt-dlp (it unlocks a *source*),
 * so the format-derived derivation in [`missingHelperFor`] cannot find it.
 */
const YT_DLP_TOOL = "yt-dlp";

/**
 * The JavaScript runtimes yt-dlp will hand YouTube's challenges to, as `format::JS_RUNTIMES` orders
 * them — Deno first, because that is the order yt-dlp itself prefers.
 *
 * Written down here for exactly the reason [`YT_DLP_TOOL`] is: a runtime unlocks no format either
 * (it unlocks a *fetch*), so no `FormatView.needs` will ever name one and the format-derived
 * derivation in [`missingHelperFor`] cannot find it. Node is in the list because it is a runtime
 * that makes a link *work*, never because it is one this app would install: nothing installs Node,
 * so no plan claims it, and the lookup below can only ever resolve to Deno.
 */
const JS_RUNTIME_TOOLS = ["deno", "node"] as const;

/**
 * The words `link::FetchFailure::NoJsRuntime` is built around.
 *
 * Not the whole sentence, and not a parse: it narrows *which rows* were stopped by a missing
 * runtime, because the tool list alone cannot tell a YouTube challenge nobody could answer from a
 * video that was simply deleted on a machine that also happens to have no Deno. Which *helper* to
 * name is still derived from the tool list and the plans, so a reworded message costs a microlink
 * rather than inventing one for a row that never wanted a runtime at all.
 */
const JS_RUNTIME_FAILURE = "JavaScript runtime";

/**
 * The permission `link::FetchFailure::SafariNeedsFullDiskAccess` is about.
 *
 * Read for the same reason and with the same restraint as [`JS_RUNTIME_FAILURE`]: this failure has
 * no helper to install and no setting to flip from here — the only thing that fixes it is a trip to
 * System Settings, so the row needs to know it is *that* failure to be able to offer the trip.
 */
const FULL_DISK_ACCESS_FAILURE = "Full Disk Access";

/**
 * Did this row fail because macOS keeps Safari's cookies behind Full Disk Access?
 *
 * The one failure whose fix is a permission rather than a helper or a setting, which is why it gets
 * an affordance of its own instead of going through [`missingHelperFor`]: there is nothing to
 * install, and Settings → Links (pick another browser) is already named in the message itself.
 */
export const needsFullDiskAccess = (row: Row): boolean =>
  row.status === "failed" && (row.message ?? "").includes(FULL_DISK_ACCESS_FAILURE);

/**
 * The words each sign-in ending is built around — `link::FetchFailure`, read with the same
 * restraint as [`JS_RUNTIME_FAILURE`] and [`FULL_DISK_ACCESS_FAILURE`] above.
 *
 * Fragments rather than whole sentences, and fragments chosen to be the *distinguishing* clause of
 * one variant each: what the row needs to know is not the wording but which of three different
 * fixes it is about, and the three fixes are the whole reason Rust stopped folding them together.
 * A reworded message costs an affordance, exactly as it does for the two above — never a wrong one,
 * because no other failure in the app contains any of these clauses.
 */
const SIGN_IN_WALL: readonly string[] = ["wants a sign-in", "without signing in to the site"];
const SIGN_IN_REFUSED = "would not accept the sign-in";
const SIGN_IN_UNREADABLE: readonly string[] = [
  "could not read the sign-in from that browser",
  "could not read that cookies.txt file",
  "unlock that browser's cookies",
];

/**
 * Did this row fail over the sign-in, and in which of the three ways?
 *
 * Only ever asked of a *link*: a dropped file has no site to be signed in to, and a filename that
 * happens to contain one of the clauses above is not a fetch failure. Safari's Full Disk Access
 * ending is excluded here rather than classified as `unreadable` — it already has the one button
 * that fixes it ([`needsFullDiskAccess`]), and a row offering both would be asking the user to
 * choose between two ways out of a problem they have not been told the shape of.
 */
export const signInFailure = (row: Row): SignInFailure | null => {
  if (row.status !== "failed" || row.link === null) return null;
  const message = row.message ?? "";
  if (message.includes(FULL_DISK_ACCESS_FAILURE)) return null;
  if (SIGN_IN_UNREADABLE.some((clause) => message.includes(clause))) return "unreadable";
  if (message.includes(SIGN_IN_REFUSED)) return "refused";
  if (SIGN_IN_WALL.some((clause) => message.includes(clause))) return "wall";
  return null;
};

/**
 * yt-dlp's own spelling for Safari — the one browser whose cookies cost a permission.
 *
 * `settings::COOKIE_BROWSERS[0]`, and the reason this module has to know one browser by name at
 * all: every other allowlisted browser is interchangeable to the flow ("borrow it, press the
 * button"), and Safari is not.
 */
const SAFARI = "safari";

/**
 * Is the sign-in this app is configured to borrow Safari's?
 *
 * Asked before every check, because Safari's answer comes from a permission rather than from a
 * network: `check_safari_cookie_access` settles it locally and instantly, and the probe is only
 * worth running once the file has been shown to open at all.
 */
const borrowingFromSafari = (link: LinkSettings): boolean =>
  link.cookies === "browser" && link.cookie_browser.trim().toLowerCase() === SAFARI;

/**
 * A browser store worth borrowing from: the file is there, and something has been saved in it.
 *
 * `settings::EMPTY_COOKIE_STORE_BYTES` — a Chromium browser that has been opened once and signed in
 * to nothing has a 65,536-byte database holding a schema and no cookies. Offering that is how the
 * bug behind this pass happened, so "it exists" is never the whole test.
 */
const EMPTY_COOKIE_STORE_BYTES = 65_536;

const usedStore = (browser: BrowserPresence): boolean =>
  browser.installed &&
  browser.cookie_store_exists &&
  (browser.cookie_store_bytes ?? 0) > EMPTY_COOKIE_STORE_BYTES;

/** `settings::FRESH_COOKIE_STORE_DAYS`, in the seconds a browser clock counts in. */
const FRESH_COOKIE_STORE_SECS = 7 * 86_400;

const usedRecently = (browser: BrowserPresence, now: number): boolean =>
  usedStore(browser) &&
  browser.cookie_store_modified !== null &&
  now - browser.cookie_store_modified <= FRESH_COOKIE_STORE_SECS;

/**
 * The browser to offer this machine, or null when there is nothing honest to offer.
 *
 * Rust ranks the rows by *evidence* — a sign-in store that has been written to recently beats an
 * empty or stale one, the default browser is a tie-break and nothing more — and this reads rank 1
 * back rather than repeating the rule. `recommended` is that row when it is worth offering at all,
 * so a null here is "no browser on this Mac has a cookie store to lend", which is the cookies.txt
 * machine.
 *
 * What this deliberately no longer does is prefer *anything* over Safari. The old rule was "the
 * first installed browser that is not Safari", chosen because Safari's jar costs a Full Disk Access
 * grant — and on the Mac that reported this, that rule offered a Chrome nobody had signed in to
 * while the user's actual YouTube session sat in the Safari they browse with. A permission is a cost
 * to state, not a reason to borrow from the wrong browser.
 */
export const preferredBrowser = (browsers: BrowserPresence[]): BrowserPresence | null =>
  browsers.find((browser) => browser.recommended) ?? null;

/**
 * Another browser this Mac could borrow from without paying for a permission, or null.
 *
 * Only ever a *usable* one: "you could use Chrome instead" is worth saying to somebody weighing up
 * four levels of System Settings, and worth saying only when Chrome could actually answer. A Chrome
 * with an empty jar is not an easier route, it is the same dead end one step further along.
 */
export const permissionFreeAlternative = (
  browsers: BrowserPresence[],
  offer: BrowserPresence | null,
): BrowserPresence | null =>
  browsers.find(
    (browser) => browser.id !== offer?.id && !browser.needs_full_disk_access && usedStore(browser),
  ) ?? null;

/**
 * Why *this* browser, in one clause a person reads without stopping.
 *
 * The app names a browser and then has to be able to say what that name is based on, or the user is
 * being asked to trust a guess — which is what the last rule was. Four clauses, in the order the
 * evidence is worth anything:
 *
 *   - it was used recently, which is the strongest thing `stat` can say about where a live sign-in
 *     is (a mtime, never a cookie: nothing in this app opens that file);
 *   - it is the browser macOS opens links with, which is what the ranking treats it as — a
 *     tie-break;
 *   - its store is empty, which is not a reason at all but a *caveat*, and the one case where the
 *     app must say something rather than let a click fail quietly;
 *   - and otherwise the plain truth, which is that this is where a sign-in is likeliest to be.
 *
 * Written to sit after an em dash behind the browser's name — "Safari — you used it recently" —
 * so the empty-store case reads as the warning it is instead of as a boast.
 */
export const browserReason = (
  browser: BrowserPresence,
  now: number = Date.now() / 1000,
): string => {
  if (!usedStore(browser)) return "nothing has been saved in it yet";
  if (usedRecently(browser, now)) return "you used it recently";
  if (browser.is_default) return "it is your default browser";
  return "it is the likeliest place your sign-in is";
};

/**
 * What to offer this machine, or null while nothing is known about it yet.
 *
 * Null is not "nothing works" — it is `list_cookie_browsers` never having answered (an older
 * shell, or the very first seconds of a launch), and the difference matters: a question that named
 * a browser on a guess would be exactly the dead end in reverse.
 *
 * The offer is Rust's rank 1 and the reason it earned that place, so the question can say which
 * browser and why in one breath. Two machines from the complaint that started this pass:
 *
 *   - Safari the default and written to minutes ago, Chrome installed with a 65,536-byte jar
 *     untouched for two days — the offer is Safari, and its permission is quoted as the price;
 *   - the same Mac with no cookie store anywhere — nothing is offered, because an offer to borrow
 *     from a jar that is not there is the confident wrong answer this app does not give.
 */
export const signInRemedy = (browsers: BrowserPresence[]): SignInRemedy | null => {
  if (browsers.length === 0) return null;
  const offer = preferredBrowser(browsers);
  if (offer === null) return { kind: "cookie_file" };
  const named = { id: offer.id, label: offer.label, reason: browserReason(offer) };
  // Safari is not demoted for costing a permission — it is named, and the permission is said out
  // loud, because the alternative is borrowing from a browser the sign-in is not in.
  if (offer.needs_full_disk_access) return { kind: "full_disk_access", ...named };
  return { kind: "browser", ...named };
};

/** What the check says while it is still saying it. Two seconds of yt-dlp, so it is worth saying. */
const CHECKING_SIGN_IN = "Checking that sign-in…";

/**
 * The one question, in the words this machine has earned.
 *
 * Four sentences rather than one with a slot in it, because the offers are not the same offer: one
 * is a click that will finish the job, one is a browser the sign-in really is in whose cookies cost
 * a permission first, and one is a file the user has to go and make in a browser extension. A
 * question that promised "and try again" for the last two would be the dead end wearing a button.
 *
 * The permission sentence names the browser and why it was picked, because "Safari is the only
 * browser here" — which is what it used to say — is false on the Mac this came from, and a user
 * who can see Chrome in their Dock would rightly stop believing the rest of it.
 */
export const signInQuestion = (prompt: SignInPrompt): string => {
  const need = `${prompt.links} ${pluralLinks(prompt.links)} ${prompt.links === 1 ? "needs" : "need"} a sign-in.`;
  switch (prompt.remedy.kind) {
    case "browser":
      return `${need} Use your ${prompt.remedy.label} sign-in and try again?`;
    case "full_disk_access":
      return (
        `${need} It is most likely in ${prompt.remedy.label} — ${prompt.remedy.reason} — and macOS ` +
        "keeps those cookies behind Full Disk Access."
      );
    case "cookie_file":
      return `${need} No browser here can lend one, so the way in is an exported cookies.txt file.`;
    case "guide":
      return `${need} The sign-in Flint already has did not work — see what to try?`;
  }
};

/** The verb on the accepting button — the same three offers, said in one or two words. */
export const signInConfirmLabel = (remedy: SignInRemedy): string => {
  switch (remedy.kind) {
    case "browser":
      return `Use ${remedy.label}`;
    case "full_disk_access":
      return "Open Full Disk Access…";
    case "cookie_file":
    case "guide":
      return "Show me how…";
  }
};

/** Everything about a row that belongs to one *run* rather than to the source itself. */
const CLEAN = {
  status: "queued",
  fraction: null,
  eta_secs: null,
  output: null,
  bytes: null,
  message: null,
} as const satisfies Omit<Row, "info" | "link" | "target" | "phase">;

/**
 * Which half a row starts in.
 *
 * A link is fetched before it is converted, and its first frame must not read "Converting 0%" for
 * the split second before the backend's first `downloading` sample lands. A file only ever converts,
 * which is also what `progress::Phase::default()` says.
 */
const startingPhase = (link: LinkRow | null): Phase =>
  link === null ? "converting" : "downloading";

const newRow = (info: FileInfo): Row => ({
  info,
  link: null,
  target: info.default_target ?? "",
  phase: "converting",
  ...CLEAN,
});

/**
 * A queued link, as a row.
 *
 * `info` is synthesised rather than probed: `inspect_links` is pure and instant, so nothing is known
 * about the media yet: not its title, length or size. The source category determines its picker
 * and bulk target group. `path` is empty and stays empty until the source helper produces a file;
 * every path-shaped question in this store asks about a *file* row for exactly that reason.
 */
export const linkRow = (link: LinkRow): Row => ({
  info: {
    id: link.id,
    path: "",
    // Host and tail of the URL, which is what tells two links apart at a glance. Once the batch
    // announces the row, `FileRow` shows the resolved video title instead.
    name: shortUrl(link.url),
    size_bytes: 0,
    supported: link.supported,
    category: link.supported ? link.category : null,
    // No file, so no source format: `format_name` carries the site, which is what a link's meta
    // line has to say instead.
    format_id: null,
    format_name: link.site_label,
    default_target: link.default_target,
    suggested_targets: link.suggested_targets,
    duration_secs: null,
    duration_label: null,
    resolution_label: null,
    is_animated: false,
    note: link.note,
  },
  link,
  target: link.default_target ?? "",
  phase: "downloading",
  ...CLEAN,
});

/** Back to "not yet converted", keeping the source and the chosen target. */
const resetRun = (row: Row): Row => ({ ...row, ...CLEAN, phase: startingPhase(row.link) });

/**
 * One row as the backend wants it: exactly one of `path` or `url`.
 *
 * `jobs_from` refuses an item that names both or neither, and a link row's `info.path` is the empty
 * string — sending that as a path would turn every link into "no such file".
 */
const batchItem = (row: Row): BatchItemArg =>
  row.link !== null
    ? { id: row.info.id, url: row.link.url, target_id: row.target }
    : { id: row.info.id, path: row.info.path, target_id: row.target };

/**
 * Point a row at a different output format.
 *
 * The result of the previous run no longer describes this row, so it is thrown away: keeping a
 * "failed: no audio track" message next to a target the user just changed to MP4 would tell them
 * about a conversion that is no longer the one queued up. A no-op change is left untouched, or the
 * bulk "convert all to" picker would wipe results it did not actually alter.
 */
const retarget = (row: Row, target: string): Row =>
  row.target === target ? row : { ...resetRun(row), target };

/**
 * Which row should hold the selection once `id` has gone: the one after it, or the one before it
 * when it was last. Null only when the queue is now empty.
 *
 * Takes the order from both sides of the removal rather than an index, so it cannot be off by one
 * against a list that has already changed.
 */
const successor = (before: string[], after: string[], id: string): string | null => {
  const index = before.indexOf(id);
  if (index === -1 || after.length === 0) return null;
  return after[Math.min(index, after.length - 1)] ?? null;
};

/** Every row in the queue, in list order. The one place that turns `order` + `files` into rows. */
export const allRows = (state: Pick<State, "files" | "order">): Row[] =>
  state.order.map((id) => state.files[id]).filter((row): row is Row => row !== undefined);

/** Rows that would take part in a batch right now. */
export const convertibleRows = (state: Pick<State, "files" | "order">): Row[] =>
  allRows(state).filter(
    (row) => row.info.supported && row.target !== "" && row.status !== "done",
  );

/** Rows that came from a pasted link. The link cap is over these, not over the whole queue. */
export const linkRows = (state: Pick<State, "files" | "order">): Row[] =>
  allRows(state).filter((row) => row.link !== null);

/**
 * The folder to name under the action bar, or null while there is nothing to name.
 *
 * [`State.destination`] is deliberately two shapes, so the difference is read here rather than
 * guessed by the bar: an estimate for a file row is a full output path and its *parent* is the
 * folder, while a queue of nothing but links carries `LinkSupport.destination`, which already is
 * one. Chopping the last component off that would name the folder above the one files land in.
 */
export const destinationFolder = (
  state: Pick<State, "files" | "order" | "destination">,
): string | null => {
  const destination = state.destination;
  if (destination === null) return null;
  const rows = convertibleRows(state);
  const onlyLinks = rows.length > 0 && rows.every((row) => row.link !== null);
  if (onlyLinks) return destination;
  // `""` is [`parentDir`] saying the estimate names no folder at all. Nothing to name is `null`,
  // the same as no destination: the bar stays quiet rather than offering the file as a folder.
  const folder = parentDir(destination);
  return folder === "" ? null : folder;
};

/**
 * Does *this* window have rows in the batch that is running?
 *
 * False for a batch a reload left behind (`State.inheritedBatch`): the shell is converting rows this
 * page has no `Row` for, so nothing in the queue belongs to Rust and the queue-wide controls — Clear
 * all, the bulk target pickers — have no reason to be dead. The per-row locks ask about the row
 * itself (`batchIds.has(id)`), which answers the same question one row at a time.
 */
export const ownsRunningBatch = (state: Pick<State, "phase" | "batchIds">): boolean =>
  state.phase === "running" && state.batchIds.size > 0;

/**
 * Is the engine that does nearly every conversion absent?
 *
 * The bundled FFmpeg sidecar is what a damaged or quarantined install loses first, and without it
 * the app can convert nothing at all — so this is the one fact worth saying before the user has
 * asked anything. Two places say it (the settings sheet's banner, and one line under the empty
 * canvas), which is exactly why the condition lives here: the same three terms, dismissed once.
 *
 * `undefined` while the catalog is still loading, so neither place flickers a warning on launch.
 */
export const engineMissing = (state: Pick<State, "catalog" | "bannerDismissed">): boolean => {
  if (state.bannerDismissed) return false;
  const ffmpeg = state.catalog?.tools.find((tool) => tool.id === "ffmpeg");
  return ffmpeg !== undefined && !ffmpeg.available;
};

/** An incomplete destination has no path to preview. Saving still uses Rust's per-field merge. */
const unfinishedDestination = (settings: Settings): boolean => {
  const out = settings.output;
  if (out.location === "custom") return out.custom_dir === null || out.custom_dir.trim() === "";
  if (out.location === "subfolder") return out.subfolder_name.trim() === "";
  return false;
};

/** React 18+ StrictMode mounts effects twice in dev; the subscriptions below must not double up. */
let initialised = false;
let saveTimer: ReturnType<typeof setTimeout> | null = null;
let destinationTimer: ReturnType<typeof setTimeout> | null = null;
/** Overlapping `addPaths` calls; `busy` stays true until the last inspection returns. */
let inspecting = 0;

export const useStore = create<Store>()((set, get) => {
  // All writes share one queue: a preset must not overtake a debounced or in-flight save.
  let writes: Promise<unknown> = Promise.resolve();
  let pendingSettings: Settings | null = null;
  let destinationRevision = 0;
  let linksRevision = 0;
  let checkRevision = 0;
  let settingsRevision = 0;
  const enqueueWrite = <T,>(write: () => Promise<T>): Promise<T> => {
    const result = writes.then(write);
    writes = result.catch(() => undefined);
    return result;
  };
  const flushSave = (): Promise<unknown> => {
    if (saveTimer !== null) clearTimeout(saveTimer);
    saveTimer = null;
    const settings = pendingSettings;
    pendingSettings = null;
    if (settings === null) return writes;
    return enqueueWrite(() => ipc.saveSettings(settings));
  };
  /** Persist at most once per 300 ms, without replacing the user's in-progress fields. */
  const scheduleSave = (settings: Settings): void => {
    if (saveTimer !== null) clearTimeout(saveTimer);
    pendingSettings = settings;
    saveTimer = setTimeout(() => {
      // Rust merges unfinished fields independently; never suppress unrelated preferences here.
      void flushSave().catch((e: unknown) => set({ error: errorMessage(e) }));
    }, 300);
  };

  /**
   * Persist these settings now, and make sure a save already in the timer cannot undo them.
   *
   * The debounce holds a *snapshot*: a pending save fires with the settings it was scheduled with,
   * so leaving one queued behind an immediate write would rewrite the browser choice back to
   * whatever was there 200 ms ago. Only used where the next step depends on the choice having
   * landed — everything a user merely types still goes through [`scheduleSave`].
   */
  const saveNow = async (settings: Settings): Promise<void> => {
    settingsRevision += 1;
    if (saveTimer !== null) {
      clearTimeout(saveTimer);
      saveTimer = null;
    }
    pendingSettings = null;
    set({ settings });
    await enqueueWrite(() => ipc.saveSettings(settings));
  };

  /** "Files land in …" under the action bar — one estimate for the first queued row. */
  const scheduleDestination = (): void => {
    const revision = ++destinationRevision;
    if (destinationTimer !== null) clearTimeout(destinationTimer);
    set({ destination: null });
    const current = (): boolean => revision === destinationRevision;
    destinationTimer = setTimeout(() => {
      destinationTimer = null;
      const state = get();
      const rows = convertibleRows(state);
      // `estimate_output_path` runs the same `check_output` a save does, so an unfinished
      // destination would answer this with the same refusal: there is simply nothing to show yet.
      if (state.settings === null || unfinishedDestination(state.settings)) {
        set({ destination: null });
        return;
      }
      const first = rows.find((row) => row.link === null);
      if (first === undefined) {
        /*
         * Nothing but links (or nothing at all). A link has no source file to estimate from — and
         * "alongside the source" cannot apply to a URL, which is why `paths::link_output_dir` sends
         * it to the chosen folder or to ~/Downloads instead. That folder is the honest answer.
         *
         * Asked of the backend under *these* settings, exactly as the file branch below asks
         * `estimate_output_path` again on every change: `get_link_support` is where the rule lives,
         * and the copy the last look at it left behind is a folder the user may since have moved.
         * Reusing it named ~/Downloads under a queue that was about to land somewhere else.
         */
        if (rows.length === 0) {
          set({ destination: null });
          return;
        }
        ipc
          .getLinkSupport(state.settings)
          .then((support) => {
            if (current()) set({ linkSupport: support, destination: support.destination });
          })
          // No answer at all is better than last week's: an older shell without the command has
          // nothing to say about where a link lands.
          .catch(() => { if (current()) set({ destination: null }); });
        return;
      }
      ipc
        .estimateOutputPath(first.info.path, first.target, state.settings)
        .then((path) => { if (current()) set({ destination: path }); })
        .catch(() => { if (current()) set({ destination: null }); });
    }, 200);
  };

  /**
   * Re-read what the link UI has to state: the cap, the hosts, the destination, and whether yt-dlp
   * is on this machine.
   *
   * Asked again after every `refreshTools`, because installing yt-dlp changes `tool_installed` — and
   * after a settings change would move where a link lands, which is what `openLinks` covers.
   */
  const loadLinkSupport = async (): Promise<void> => {
    const settings = get().settings;
    if (settings === null) return;
    try {
      const support = await ipc.getLinkSupport(settings);
      if (get().settings === settings) set({ linkSupport: support });
    } catch {
      // An older shell without the command: the box falls back to the cap the core ships with, and
      // the core refuses anything over the real one anyway.
    }
  };

  /**
   * Re-read the install plans. Cheap (two `stat` calls in Rust) and worth doing again after a
   * refresh: the answer to "is Homebrew there?" changes the moment the user installs it.
   */
  const loadInstallPlans = async (): Promise<void> => {
    try {
      set({ installPlans: await ipc.getInstallPlans() });
    } catch {
      // Nothing to tell the user: the list still shows what is missing and how to fix it by hand.
    }
  };

  /**
   * Which browsers this machine has. A handful of `stat` calls in Rust, asked once at launch.
   *
   * A shell without the command leaves the list empty, and empty is a state everything downstream
   * already understands: the picker marks nothing as absent and the question is not raised at all,
   * because there is no browser this app can honestly claim the user owns.
   */
  const loadCookieBrowsers = async (): Promise<void> => {
    try {
      set({ cookieBrowsers: await ipc.listCookieBrowsers() });
    } catch {
      // Older shell: the browser list stays the allowlist, unmarked, exactly as it was before.
    }
  };

  /**
   * One sign-in check, over the settings handed in rather than the ones on disk.
   *
   * The settings are a parameter because the interesting moment is the one *before* a save has
   * landed: `useBrowserSignIn` picks a browser and checks that same choice in the same breath, and
   * a check that read the store back would race the 300ms debounce and test the previous answer.
   *
   * Safari is answered before any of that. Its cookies are behind Full Disk Access, so the failure
   * a Safari user hits is a macOS permission — and asking a twenty-second probe of a public video
   * whether this app holds a permission is twenty seconds spent arriving at what one `open(2)`
   * knows immediately. `check_safari_cookie_access` is that open: instant, offline, and it reads
   * nothing out of the file. A `readable` answer means only that the permission is in place, so the
   * probe still runs after it — that is the site's half of the question, and it is a real one.
   *
   * A refusal is not a verdict. `settings_store::check_cookies` rejects a half-made source in the
   * words the settings page already uses, and that sentence is shown where the check was asked for
   * — never as a failed sign-in, and never as a toast over a sheet that is covering the toast slot.
   */
  const runCheck = async (settings: Settings): Promise<boolean> => {
    const revision = ++checkRevision;
    const current = (): boolean =>
      revision === checkRevision && get().settings?.link === settings.link;
    set({ signInCheck: { checking: true, result: null, message: CHECKING_SIGN_IN } });
    if (borrowingFromSafari(settings.link)) {
      try {
        const access = await ipc.checkSafariCookieAccess();
        if (!current()) return false;
        if (access.result !== "readable") {
          // Every not-readable answer is a jar this app cannot open, which is what `unreadable`
          // means to the rest of the UI — the *sentence* is what tells the three of them apart.
          set({ signInCheck: { checking: false, result: "unreadable", message: access.message } });
          return false;
        }
      } catch {
        // An older shell without the command. The probe below is the answer it used to give, and a
        // slow honest verdict beats an error about a command the user has never heard of.
      }
    }
    try {
      const failed = allRows(get()).find((row) => row.link !== null && signInFailure(row) !== null);
      const test = await ipc.testCookieSource(settings, failed?.link?.url);
      if (!current()) return false;
      set({ signInCheck: { checking: false, result: test.result, message: test.message } });
      return test.ok;
    } catch (e: unknown) {
      if (!current()) return false;
      set({ signInCheck: { checking: false, result: null, message: errorMessage(e) } });
      return false;
    }
  };

  /**
   * What is the shell already busy with — and is it *still* busy with it?
   *
   * A reload throws away the store and every listener but not the work: the window used to come
   * back believing it was idle while a conversion still held the single batch slot, so Convert was
   * refused with "a conversion is already running" and there was nothing on screen to stop.
   * Adopting what `get_activity` reports makes the window honest instead — Stop works, and the rows
   * being converted are simply not ones this page has (`State.inheritedBatch`,
   * `State.foreignInstall`).
   *
   * The answer is a snapshot, though, and work that ended between the shell taking it and this
   * store adopting it sent its last event to a window that was not listening yet. Adopting *that*
   * is worse than missing it: a Stop button nothing would ever clear, and every Install button
   * disabled for the rest of the session. So an adoption is confirmed by a second read, which the
   * shell answers after any such event — and if the shell has gone quiet, so does the window.
   */
  const adoptShellWork = async (): Promise<void> => {
    try {
      const activity = await ipc.getActivity();
      const converting = activity.converting && get().phase !== "running";
      const installing = activity.installing && get().install === null;
      if (converting) set({ phase: "running", inheritedBatch: true, batchIds: new Set<string>() });
      if (installing) set({ foreignInstall: true });
      if (!converting && !installing) return;
      const now = await ipc.getActivity();
      if (converting && !now.converting && get().inheritedBatch) {
        set({ phase: "idle", inheritedBatch: false, stopping: false, stopRequested: false });
      }
      if (installing && !now.installing && get().foreignInstall) set({ foreignInstall: false });
    } catch {
      // An older shell without `get_activity`: assume idle, which is what it used to do anyway.
    }
  };

  const updateRow = (id: string, patch: (row: Row) => Row): void => {
    const row = get().files[id];
    if (row === undefined) return; // unknown id: the row was removed, drop the event
    const next = patch(row);
    if (next !== row) set({ files: { ...get().files, [id]: next } });
  };

  /**
   * True while the batch in flight owns this row. Such a row is Rust's until `batch_finished`:
   * the UI may not change its target or drop its results.
   */
  const ownedByBatch = (id: string): boolean => {
    const state = get();
    return state.phase === "running" && state.batchIds.has(id);
  };

  /**
   * Terminal transition: first verdict wins. Rust promises exactly one of finished/failed/skipped
   * per row, but a duplicate (or a cancellation racing a completion) must not turn a green row red.
   */
  const settleRow = (id: string, patch: (row: Row) => Row): void => {
    updateRow(id, (row) => (TERMINAL.has(row.status) ? row : patch(row)));
  };

  const runRows = async (rows: Row[], crop: CropSettings | null = null): Promise<void> => {
    const state = get();
    if (state.phase === "running" || state.settings === null || rows.length === 0) return;
    const cap = state.linkSupport?.max_links ?? FALLBACK_MAX_LINKS;
    const links = rows.filter((row) => row.link !== null).length;
    if (links > cap) {
      set({ error: linkCapRefusal(links, cap) });
      return;
    }
    const files = { ...state.files };
    const batchIds = new Set(rows.map((row) => row.info.id));
    for (const row of rows) files[row.info.id] = { ...resetRun(row), runCrop: crop };
    set({
      files, batchIds, phase: "running", inheritedBatch: false,
      stopping: false, stopRequested: false, summary: null, error: null,
    });
    try {
      await ipc.startBatch(rows.map(batchItem), cropRunSettings(state.settings, crop));
    } catch (e: unknown) {
      // Restore only this attempt's rows. A drop while IPC was pending still belongs to the user.
      const current = get();
      if (current.batchIds !== batchIds) return;
      const restored = { ...current.files };
      for (const row of rows) if (restored[row.info.id] !== undefined) restored[row.info.id] = row;
      const unchanged = current.order === state.order;
      set({
        files: restored, batchIds: state.batchIds,
        phase: unchanged ? state.phase : "idle",
        summary: unchanged ? state.summary : null,
        error: errorMessage(e), stopping: false, stopRequested: false,
      });
      scheduleDestination();
    }
  };

  /**
   * Ask, once, whether the user wants to go and install the helper their batch just needed.
   *
   * Raised when the batch *settles*, not per file: ten failing DOCX files are one missing helper,
   * so they are one question. The row microlink is too easy to miss — this is the same trip, said
   * out loud — but it is still only a question, and it is not asked when asking would be noise:
   *
   *   - the user stopped the batch themselves; a deliberate stop is not a failure;
   *   - nothing failed for want of a helper (a corrupt input, a write error): those get the row's
   *     own message and nothing else;
   *   - the helper is already there, or *that* helper is being installed right now
   *     (`blockedHelpers` only reports helpers the catalog says are missing, and a running install
   *     is about to change that) — an install of some other helper says nothing about this one and
   *     must not silence the question;
   *   - this helper has already been asked about this session;
   *   - Settings is already open, in which case the helper is highlighted where the user is
   *     looking rather than covered with a modal.
   */
  const askAboutHelpers = (files: Record<string, Row>, ids: Iterable<string>, cancelled: boolean): void => {
    if (cancelled) return;
    const state = get();
    if (state.linksOpen || state.signInOpen || state.signInPrompt !== null) return;
    const installing =
      state.install !== null && state.install.status === "running" ? state.install.packageId : null;
    const rows = [...ids].map((id) => files[id]).filter((row): row is Row => row !== undefined);
    const tools = blockedHelpers(state, rows).filter(
      (helper) =>
        !state.askedPackages.includes(helper.packageId) && helper.packageId !== installing,
    );
    const primary = tools[0];
    if (primary === undefined) return;
    if (state.drawerOpen) {
      get().showPackage(primary.packageId);
      return;
    }
    set({
      installPrompt: { tools, files: tools.reduce((sum, helper) => sum + helper.files, 0) },
    });
  };

  /**
   * Ask, once, whether the user wants this app to borrow a sign-in for the links it could not get.
   *
   * [`askAboutHelpers`]'s sibling, and deliberately built to the same rules: raised when the batch
   * settles rather than per row (five links behind one wall are one question), never after a stop
   * the user asked for, never twice in a session, and never over an open Settings sheet — where the
   * picker, the check and the guide are already on screen under the user's cursor.
   *
   * It differs in one way, and only because it can afford to: the helper question hands over and
   * stops, because installing is a download and a password and possibly money. Borrowing a sign-in
   * is a saved setting and a two-second probe, so this one finishes the job — see
   * [`Actions.confirmSignInPrompt`].
   *
   * Never asked at the same moment as the helper question. One batch, one question: a run that
   * failed for want of yt-dlp *and* a sign-in has no sign-in problem worth solving yet, because
   * without yt-dlp nothing was ever going to be fetched.
   */
  const askAboutSignIn = (files: Record<string, Row>, ids: Iterable<string>, cancelled: boolean): void => {
    if (cancelled) return;
    const state = get();
    if (state.askedAboutSignIn || state.installPrompt !== null || state.signInPrompt !== null) return;
    const walled = [...ids]
      .map((id) => files[id])
      .filter((row): row is Row => row !== undefined && signInFailure(row) !== null);
    if (walled.length === 0) return;
    // Nothing is known about this machine's browsers, so there is no browser to name and no honest
    // question to ask. The rows still carry their own way out.
    const offered = signInRemedy(state.cookieBrowsers);
    if (offered === null) return;
    if (state.drawerOpen || state.linksOpen || state.signInOpen) return;
    /*
     * One last honesty check on the offer itself.
     *
     * If the app is already borrowing from the very browser it is about to name, then "Use Chrome
     * and try again" is a button that repeats what has just failed. The walkthrough is what is
     * actually left — a stale sign-in, a refused Keychain prompt, another browser, a cookies.txt —
     * so the question offers that instead of a click it cannot honour.
     */
    const source = state.settings?.link ?? null;
    const already =
      offered.kind === "browser" &&
      source !== null &&
      source.cookies === "browser" &&
      source.cookie_browser.trim().toLowerCase() === offered.id;
    const remedy: SignInRemedy = already ? { kind: "guide" } : offered;
    set({
      signInPrompt: { links: walled.length, ids: walled.map((row) => row.info.id), remedy },
    });
  };

  return {
    initializationFailed: false,
    catalog: null,
    settings: null,
    files: {},
    order: [],
    hasHeldFiles: false,
    batchIds: new Set<string>(),
    phase: "idle",
    inheritedBatch: false,
    stopping: false,
    stopRequested: false,
    summary: null,
    cropOpen: false,
    setCropOpen: (open) => {
      if (open && (get().phase === "running" || get().drawerOpen || get().linksOpen ||
        get().signInOpen || get().installPrompt !== null || get().signInPrompt !== null)) return;
      set({ cropOpen: open });
    },
    drawerOpen: false,
    drawerPage: "conversion",
    installPlans: [],
    install: null,
    foreignInstall: false,
    highlightedPackage: null,
    installPrompt: null,
    askedPackages: [],
    dragging: false,
    busy: false,
    selectedId: null,
    error: null,
    notice: null,
    bannerDismissed: false,
    destination: null,
    linksOpen: false,
    linksPrefill: "",
    linksError: null,
    linkSupport: null,
    cookieBrowsers: [],
    signInPrompt: null,
    askedAboutSignIn: false,
    signInOpen: false,
    signInFromSettings: false,
    signInCheck: null,

    init: async () => {
      if (initialised) return;
      initialised = true;
      set({ initializationFailed: false, error: null });
      const subscriptions: ipc.Unlisten[] = [];
      try {
        // Subscribe before querying activity so completion events cannot fall between them.
        subscriptions.push(await ipc.onBatchEvent((event) => get().handleEvent(event)));
        subscriptions.push(await ipc.onInstallEvent((event) => get().handleInstallEvent(event)));
        subscriptions.push(await ipc.onDragDrop((event) => {
          if (event.kind === "hover") {
            if (!get().dragging) set({ dragging: true });
          } else if (event.kind === "leave") set({ dragging: false });
          else {
            set({ dragging: false });
            void get().addPaths(event.paths);
          }
        }));
        subscriptions.push(await ipc.onMenuAction((action) => {
          if (get().cropOpen) return;
          const store = get();
          switch (action) {
            case "settings":
              store.setDrawer(!store.drawerOpen);
              break;
            case "skin":
              store.openSkin();
              break;
            case "open_files":
              void store.openFiles();
              break;
            case "open_folder":
              void store.openFolder();
              break;
            case "paste_links":
              store.openLinks();
              break;
            case "convert":
              void store.start();
              break;
            case "stop":
              void store.stop();
              break;
            case "clear":
              store.clearAll();
              break;
          }
        }));
        const [catalog, settings] = await Promise.all([ipc.getCatalog(), ipc.getSettings()]);
        set({ catalog, settings });
        // Optional capabilities may be absent in older shells.
        void loadInstallPlans();
        void loadLinkSupport();
        void loadCookieBrowsers();
        await adoptShellWork();
      } catch (e: unknown) {
        for (const unsubscribe of subscriptions) unsubscribe();
        initialised = false;
        set({ initializationFailed: true, error: `Could not initialize the app: ${errorMessage(e)}` });
      }
    },

    addPaths: async (paths) => {
      if (paths.length === 0) return;
      // Two drops can overlap (a folder walk is slow, and the window stays droppable), so `busy`
      // is reference-counted: the first inspection to return must not clear the other's shimmer.
      inspecting += 1;
      set({ busy: true });
      try {
        /*
         * The backend enumerates up to `Inspection.limit` files and says so when that stopped it
         * short. The files it did find are an ordinary queue — convertible the moment they land —
         * so the cap is reported and nothing else about this path changes: a truncated drop is a
         * short answer, not an error.
         */
        const inspection = await ipc.inspectFiles(paths);
        const state = get();
        // File rows only: a link row's `path` is the empty string, and one of those in the queue
        // would make every later drop look like a file that was already here.
        const known = new Set(
          allRows(state)
            .filter((row) => row.link === null)
            .map((row) => row.info.path),
        );
        const files = { ...state.files };
        const order = [...state.order];
        for (const info of inspection.files) {
          if (known.has(info.path)) continue;
          known.add(info.path);
          files[info.id] = newRow(info);
          order.push(info.id);
        }
        set({
          files,
          order,
          hasHeldFiles: state.hasHeldFiles || order.length > 0,
          summary: null,
          phase: state.phase === "finished" ? "idle" : state.phase,
          // This inspection's own answer, so the notice never outlives the drop it describes.
          notice: [
            ...(inspection.truncated ? [truncationNotice(inspection.limit)] : []),
            ...(inspection.warnings ?? []),
          ].join(" ") || null,
        });
        scheduleDestination();
      } catch (e: unknown) {
        set({ error: errorMessage(e) });
      } finally {
        inspecting -= 1;
        if (inspecting === 0) set({ busy: false });
      }
    },

    openFiles: async () => {
      try {
        await get().addPaths(await ipc.pickFiles());
      } catch (e: unknown) {
        set({ error: errorMessage(e) });
      }
    },

    openFolder: async () => {
      try {
        const dir = await ipc.pickDirectory();
        if (dir !== null) await get().addPaths([dir]);
      } catch (e: unknown) {
        set({ error: errorMessage(e) });
      }
    },

    /**
     * Open the paste box.
     *
     * `prefill` is the ⌘V-anywhere path: the clipboard already holds a link, so the box opens with
     * it in place rather than making the user paste a second time.
     */
    openLinks: (prefill) => {
      const state = get();
      // The install question owns the window until it is answered. Two modals over one window left
      // both focus traps fighting for Tab and Esc closing whichever heard it first, which is exactly
      // the bug `setDrawer` documents — so this refuses rather than stacking.
      if (state.installPrompt !== null) return;
      // Same for the sign-in question and its guided sheet: both are the window's one modal, and
      // both are about links, so a paste box over them would be the app talking over itself.
      if (state.signInPrompt !== null || state.signInOpen) return;
      // Already up, and nothing new to put in it: ⌘L (or the File menu) on an open box is the user
      // asking for the box they are already looking at, and re-seeding it would silently empty a
      // paste they had begun editing. A clipboard that *does* carry links still replaces it.
      if (state.linksOpen && (prefill ?? "") === "") return;
      // Settings is a sheet over the same window, and the box replaces it rather than covering it.
      if (state.drawerOpen) get().setDrawer(false);
      set({ linksOpen: true, linksPrefill: prefill ?? "", linksError: null });
      // Fire and forget: the box opens on what is already known, and the cap, the destination and
      // "is yt-dlp here" are re-read behind it. All three can have changed since the last look.
      void loadLinkSupport();
    },

    closeLinks: () => {
      linksRevision += 1;
      set({ linksOpen: false, linksPrefill: "", linksError: null });
    },

    checkLinks: async (lines) => {
      const revision = ++linksRevision;
      try {
        const inspection = await ipc.inspectLinks(lines);
        if (revision !== linksRevision) return null;
        // A refusal that has been corrected must not be left standing next to a box that is now fine.
        if (get().linksError !== null) set({ linksError: null });
        return inspection;
      } catch (e: unknown) {
        if (revision !== linksRevision) return null;
        set({ linksError: errorMessage(e) });
        return null;
      }
    },

    clearLinksError: () => {
      linksRevision += 1;
      if (get().linksError !== null) set({ linksError: null });
    },

    /**
     * Queue the links the backend accepted.
     *
     * Two things are enforced here that the box itself cannot see: a link already in the queue is
     * not added twice (matched on the URL, since every paste brings fresh row ids), and the cap is
     * measured over the *whole* queue rather than over this paste — five links pasted twice is ten
     * rows, and the tenth is still under the cap only if the first five were. Whatever is left out
     * is said in the quiet notice slot, naming the cap, because a link that silently failed to
     * arrive is a link the user will paste again.
     */
    addLinks: (rows) => {
      const state = get();
      const cap = state.linkSupport?.max_links ?? FALLBACK_MAX_LINKS;
      const queued = new Set(linkRows(state).map((row) => row.link?.url ?? ""));
      const held = queued.size;
      const files = { ...state.files };
      const order = [...state.order];
      let added = 0;
      let refused = 0;
      let repeated = 0;
      for (const link of rows) {
        if (!link.supported) continue;
        if (queued.has(link.url)) {
          repeated += 1;
          continue;
        }
        if (held + added >= cap) {
          refused += 1;
          continue;
        }
        queued.add(link.url);
        const row = linkRow(link);
        files[row.info.id] = row;
        order.push(row.info.id);
        added += 1;
      }
      set({
        files,
        order,
        hasHeldFiles: state.hasHeldFiles || order.length > 0,
        summary: null,
        phase: state.phase === "finished" ? "idle" : state.phase,
        /*
         * What was left out, if anything was. The cap wins the slot when both apply, because it is
         * the one with a number in it the user has to act on; a link the queue already holds is a
         * paste that changed nothing, and saying so is the whole point — the box offers "Add 2
         * links" for a pair it cannot see are already there, and closing over an unchanged queue in
         * silence reads as a click that went nowhere.
         */
        notice:
          refused > 0
            ? linkCapNotice(cap)
            : repeated > 0
              ? linkDuplicateNotice(repeated)
              : null,
        linksOpen: false,
        linksPrefill: "",
        linksError: null,
      });
      scheduleDestination();
    },

    removeFile: (id) => {
      const state = get();
      const row = state.files[id];
      if (row === undefined) return;
      // A row the running batch still owns cannot be dropped: Rust would convert it anyway, so
      // hiding it would be a lie. Rows that already finished, and rows added after the batch
      // started, are free to go.
      const locked = ownedByBatch(id) && !TERMINAL.has(row.status);
      if (locked) return;
      const files = { ...state.files };
      delete files[id];
      const order = state.order.filter((x) => x !== id);
      set({
        files,
        order,
        // The selection moves down the list rather than evaporating, exactly as it does in Finder
        // or Mail: ⌫ is how the keyboard prunes a queue, and a selection that vanished after the
        // first row meant reaching for the mouse to delete the second. `FileList` follows this with
        // the focus ring, so holding ⌫ walks the list.
        selectedId: state.selectedId === id ? successor(state.order, order, id) : state.selectedId,
      });
      scheduleDestination();
    },

    clearAll: () => {
      const state = get();
      // Rows the running batch owns cannot be dropped, for the same reason `removeFile` refuses
      // them one at a time: Rust would convert them anyway.
      if (ownsRunningBatch(state)) return;
      destinationRevision += 1;
      if (destinationTimer !== null) clearTimeout(destinationTimer);
      set({
        files: {},
        order: [],
        batchIds: new Set<string>(),
        summary: null,
        // The notice was about the files in this queue; there are none left for it to describe.
        notice: null,
        // A batch inherited from a previous page load is still running with rows of its own; only a
        // run of *ours* ends when its queue is emptied.
        phase: state.phase === "running" ? "running" : "idle",
        selectedId: null,
        destination: null,
      });
    },

    selectFile: (id) => set({ selectedId: id }),

    setTarget: (id, target) => {
      // The running batch was started with a fixed target per row; letting the UI change one would
      // show a format Rust is not writing (and `retarget` would wipe progress mid-flight). The
      // pickers are disabled in that state, but menu/automation paths reach this directly.
      if (ownedByBatch(id)) return;
      const before = get().files[id];
      updateRow(id, (row) => retarget(row, target));
      if (before !== undefined && before.target !== target && get().phase === "finished") {
        set({ phase: "idle", summary: null });
      }
      scheduleDestination();
    },

    /** Bulk pick: only touches rows of that category, never the rest of the queue. */
    setCategoryTarget: (category, target) => {
      const state = get();
      const files = { ...state.files };
      let changed = false;
      for (const id of state.order) {
        const row = files[id];
        if (row === undefined || row.info.category !== category || !row.info.supported) continue;
        if (ownedByBatch(id)) continue; // rows the batch owns keep the target it was started with
        changed ||= row.target !== target;
        files[id] = retarget(row, target);
      }
      set({ files, ...(changed && state.phase === "finished" ? { phase: "idle", summary: null } : {}) });
      scheduleDestination();
    },

    start: async () => {
      if (get().cropOpen) return;
      await runRows(convertibleRows(get()));
    },
    startCrop: async (crop) => {
      const error = cropError(crop);
      if (error) { set({ error }); return; }
      const rows = convertibleRows(get()).filter((row) => cropApplies(crop, row.info.category));
      if (rows.length === 0) { set({ error: "No queued files match these crop ranges." }); return; }
      await runRows(rows, crop);
    },

    stop: async () => {
      if (get().phase !== "running" || get().stopping) return;
      set({ stopping: true, stopRequested: true });
      try {
        await ipc.cancelBatch();
      } catch (e: unknown) {
        // The batch is still running, so this was not a stop: it must not silence the prompt.
        set({ error: errorMessage(e), stopping: false, stopRequested: false });
      }
    },

    retry: async (id) => {
      await get().retryRows([id]);
    },

    retryRows: async (ids) => {
      const state = get();
      // A batch in flight owns the single slot, and an automatic retry must never be the thing
      // that takes it: this declines, rather than queueing behind work the user started.
      if (state.settings === null || state.phase === "running") return;
      const rows = [...new Set(ids)]
        .map((id) => state.files[id])
        .filter((row): row is Row => row !== undefined && row.info.supported && row.target !== "");
      const crop = rows[0]?.runCrop ?? null;
      if (rows.some((row) => JSON.stringify(row.runCrop ?? null) !== JSON.stringify(crop))) {
        set({ error: "These files used different crop ranges. Retry them separately or start a new crop batch." });
        return;
      }
      await runRows(rows, crop);
    },

    patchSettings: (next) => {
      settingsRevision += 1;
      if (get().settings?.link !== next.link) {
        checkRevision += 1;
        set({ signInCheck: null });
      }
      set({ settings: next });
      scheduleSave(next);
      scheduleDestination();
    },

    choosePreset: async (preset) => {
      const before = get().settings;
      const revision = ++settingsRevision;
      try {
        const saved = flushSave().catch((e: unknown) => set({ error: errorMessage(e) }));
        const applying = enqueueWrite(() => ipc.applyPreset(preset));
        const [, settings] = await Promise.all([saved, applying]);
        // A later edit owns the UI. Its queued save will follow this preset write.
        if (settingsRevision !== revision) return;
        const next = before === null ? settings : { ...settings, trim: before.trim, link: before.link };
        set({ settings: next, signInCheck: null });
        scheduleDestination();
      } catch (e: unknown) {
        set({ error: errorMessage(e) });
      }
    },

    resetSettings: async () => {
      await get().choosePreset("web_and_demo");
    },

    pickOutputDir: async () => {
      if (get().settings === null) return;
      try {
        const dir = await ipc.pickDirectory();
        if (dir === null) return;
        const state = get();
        if (state.settings === null) return;
        const settings: Settings = {
          ...state.settings,
          output: { ...state.settings.output, custom_dir: dir, location: "custom" },
        };
        get().patchSettings(settings);
      } catch (e: unknown) {
        set({ error: errorMessage(e) });
      }
    },

    pickCookieFile: async () => {
      if (get().settings === null) return;
      try {
        // The system picker is the only thing in the app that can produce an absolute path, and
        // `settings_store` refuses anything else — so the file is chosen the way the output folder
        // is, and the path it hands back is all that is ever kept. A cancelled dialog is not an
        // edit: the mode and any file already chosen stay exactly as they were.
        const paths = await ipc.pickFiles();
        const path = paths[0];
        if (path === undefined) return;
        const state = get();
        if (state.settings === null) return;
        const settings: Settings = {
          ...state.settings,
          link: { ...state.settings.link, cookie_file: path, cookies: "file" },
        };
        get().patchSettings(settings);
      } catch (e: unknown) {
        set({ error: errorMessage(e) });
      }
    },

    refreshTools: async () => {
      try {
        // The Rust side rebuilds its engine from the fresh registry, so the *catalog* changes too:
        // formats that were greyed out ("needs LibreOffice") become selectable. Re-reading only
        // `tools` would leave every target picker stale until the next app launch.
        const tools: ToolStatus[] = await ipc.refreshTools();
        const catalog = await ipc.getCatalog().catch(() => null);
        const previous = get().catalog;
        if (catalog !== null) set({ catalog });
        else if (previous !== null) set({ catalog: { ...previous, tools } });
        // A user who just installed Homebrew itself and pressed Re-check has earned an Install
        // button, so the plans are re-read alongside the tools.
        await loadInstallPlans();
        // Installing yt-dlp is what turns `LinkSupport.tool_installed` true, and the paste box says
        // so out loud — a box still warning about a helper the user has just installed is wrong.
        await loadLinkSupport();
      } catch (e: unknown) {
        set({ error: errorMessage(e) });
      }
    },

    /**
     * Ask the backend to install one helper.
     *
     * The install state is set to `running` *before* the call, so the button it was clicked on is
     * disabled by the time the event loop comes back round — and a refusal (an install already in
     * flight, an id the backend does not know, no Homebrew) lands on this tool's own row instead of
     * in the global toast, next to the command that would have done the same thing by hand.
     *
     * An install the *shell* is already running counts as one in flight even before it has named
     * itself (`State.foreignInstall`). Every button is disabled in that state, but this is also the
     * only path that could wreck the adoption: claiming `install` here — for a call Rust is about to
     * refuse — leaves `foreignInstall` set with a settled run in its way, so the foreign install's
     * first event can never be adopted and every Install button stays dead for the session.
     */
    installPackage: async (packageId) => {
      const state = get();
      if (state.foreignInstall) return;
      if (state.install !== null && state.install.status === "running") return;
      set({ install: { packageId, status: "running", lines: [], message: null } });
      try {
        await ipc.installPackage(packageId);
      } catch (e: unknown) {
        set({ install: { packageId, status: "failed", lines: [], message: errorMessage(e) } });
      }
    },

    dismissInstall: () => {
      // Never while it is running: the log and the "Installing…" line are the only proof of life.
      if (get().install?.status === "running") return;
      set({ install: null });
    },

    showPackage: (packageId) => {
      get().setDrawer(true);
      set({ highlightedPackage: packageId, drawerPage: "conversion" });
    },

    /*
     * Yes. Exactly the failed row's microlink path — `showPackage`, nothing else — so the two routes
     * cannot drift apart: the sheet opens, the helper scrolls into view, its Install button takes
     * focus, and the click that spends four minutes and 350 MB is still the user's to make.
     *
     * Every helper the question *named* counts as asked, whether or not it was the one visited: the
     * user has been told about them, and being told twice is nagging.
     */
    confirmInstallPrompt: () => {
      const prompt = get().installPrompt;
      if (prompt === null) return;
      const primary = prompt.tools[0];
      set({ installPrompt: null, askedPackages: asked(get().askedPackages, prompt) });
      if (primary !== undefined) get().showPackage(primary.packageId);
    },

    dismissInstallPrompt: () => {
      const prompt = get().installPrompt;
      if (prompt === null) return;
      set({ installPrompt: null, askedPackages: asked(get().askedPackages, prompt) });
    },

    /*
     * Opening Settings answers the question, rather than stacking a sheet behind the card.
     *
     * ⌘, stays live while the card is up — the native menu bar cannot be disabled by a div — and
     * two modals over one window left the sheet unclickable behind the prompt's veil, both focus
     * traps fighting for Tab, and Esc closing whichever one heard it first. So the same rule as
     * "Settings was already open" applies: the helper is highlighted where the user is now looking,
     * and every tool the question named counts as asked.
     */
    openSkin: () => {
      get().setDrawer(true);
      set({ drawerPage: "skin", highlightedPackage: null });
    },
    showConversionSettings: () => set({ drawerPage: "conversion" }),
    setDrawer: (open) => {
      if (!open) {
        // The highlight belongs to one trip from a failed row into settings; it must not still be
        // there the next time the sheet is opened from the menu bar.
        set({ drawerOpen: false, highlightedPackage: null, drawerPage: "conversion" });
        return;
      }
      // ⌘, is a menu accelerator and stays live under the paste box, so opening settings has to
      // *answer* the box rather than open a second sheet behind it: two modals over one window left
      // Tab walking into controls the veil covers, and Esc closing whichever heard it first.
      if (get().linksOpen) set({ linksOpen: false, linksPrefill: "", linksError: null });
      // The guided sheet is the same kind of sheet, and Settings is where it sends people anyway:
      // opening Settings answers it rather than leaving it under the drawer.
      if (get().signInOpen) set({ signInOpen: false, signInFromSettings: false });
      get().dismissSignInPrompt();
      const prompt = get().installPrompt;
      if (prompt === null) {
        set({ drawerOpen: true });
        return;
      }
      set({
        drawerOpen: true,
        installPrompt: null,
        askedPackages: asked(get().askedPackages, prompt),
        highlightedPackage: prompt.tools[0]?.packageId ?? null,
      });
    },
    dismissBanner: () => set({ bannerDismissed: true }),
    clearError: () => set({ error: null }),
    clearNotice: () => set({ notice: null }),

    reveal: async (path) => {
      try {
        await ipc.revealInFinder(path);
      } catch (e: unknown) {
        set({ error: errorMessage(e) });
      }
    },

    open: async (path) => {
      try {
        await ipc.openPath(path);
      } catch (e: unknown) {
        set({ error: errorMessage(e) });
      }
    },

    openFullDiskAccess: async () => {
      try {
        await ipc.openFullDiskAccessSettings();
      } catch (e: unknown) {
        // A build that is not macOS, or an OS that refused the URL: the row's own message already
        // names the pane, so the toast is the whole of what is left to say.
        set({ error: errorMessage(e) });
      }
    },

    useBrowserSignIn: async (browserId) => {
      const state = get();
      if (state.settings === null) return false;
      const settings: Settings = {
        ...state.settings,
        link: { ...state.settings.link, cookies: "browser", cookie_browser: browserId },
      };
      try {
        // Written through immediately rather than through the 300 ms debounce: the retry that may
        // follow reads the settings back out of the store and hands them to `start_batch`, and a
        // save still sitting in a timer is a choice the next launch would not have.
        await saveNow(settings);
      } catch (e: unknown) {
        set({ signInCheck: { checking: false, result: null, message: errorMessage(e) } });
        return false;
      }
      return runCheck(settings);
    },

    checkSignIn: async () => {
      const settings = get().settings;
      if (settings === null) return false;
      return runCheck(settings);
    },

    openSignInGuide: () => {
      const state = get();
      // Same rule as the paste box: one sheet over the window, and opening this one *answers*
      // whatever was up rather than stacking a second focus trap behind the first.
      const fromSettings = state.drawerOpen;
      get().dismissInstallPrompt();
      get().closeLinks();
      if (state.drawerOpen) get().setDrawer(false);
      set({
        signInOpen: true,
        signInFromSettings: fromSettings,
        linksOpen: false,
        signInPrompt: null,
        askedAboutSignIn: state.askedAboutSignIn || state.signInPrompt !== null,
      });
    },

    closeSignInGuide: () => {
      if (!get().signInOpen) return;
      const back = get().signInFromSettings;
      // The verdict is left standing rather than cleared: Settings → Links shows the same check,
      // and a user who just proved their sign-in works should not find that fact gone.
      set({ signInOpen: false, signInFromSettings: false });
      if (back) get().setDrawer(true);
    },

    confirmSignInPrompt: async () => {
      const prompt = get().signInPrompt;
      if (prompt === null) return;
      const remedy = prompt.remedy;
      set({ signInPrompt: null, askedAboutSignIn: true });

      if (remedy.kind === "cookie_file" || remedy.kind === "guide") {
        // Nothing to save: "from a file" with no file is a half-made choice this app holds back
        // (see `settings_store::merge`), and the guide is where the file gets chosen and checked.
        get().openSignInGuide();
        return;
      }
      if (remedy.kind === "full_disk_access") {
        const settings = get().settings;
        // Safari is the recommended source, but the permission comes first,
        // and no probe is run yet: it could only report the failure the user is on their way to fix.
        if (settings !== null) {
          get().patchSettings({
            ...settings,
            link: { ...settings.link, cookies: "browser", cookie_browser: remedy.id },
          });
        }
        await get().openFullDiskAccess();
        get().openSignInGuide();
        return;
      }

      const ok = await get().useBrowserSignIn(remedy.id);
      if (!ok) {
        // The check said no. Opening the guide with that verdict still on screen is the difference
        // between "it did not work" and the red sentence this whole flow exists to replace.
        get().openSignInGuide();
        return;
      }
      /*
       * The check said the site handed the test video over, so the app may go on by itself.
       *
       * Only over rows that are *still* stopped by a sign-in: the queue is the user's between the
       * question and the answer, and a row they removed, retried by hand, or re-targeted in the
       * meantime is not one this retry was given permission for. `retryRows` declines outright if a
       * batch is running, so an automatic retry can never take the slot from a deliberate one.
       */
      const files = get().files;
      const still = prompt.ids.filter((id) => {
        const row = files[id];
        return row !== undefined && signInFailure(row) !== null;
      });
      await get().retryRows(still);
    },

    dismissSignInPrompt: () => {
      if (get().signInPrompt === null) return;
      set({ signInPrompt: null, askedAboutSignIn: true });
    },

    handleEvent: (event) => {
      const state = get();
      // `batch://event` is a window-wide subscription: it survives the run that produced it. Any
      // event that arrives when no batch is in flight belongs to a run this window has already
      // retired, and applying it would rewrite rows the user now owns.
      if (state.phase !== "running") return;

      if (event.type === "batch_finished") {
        /*
         * A batch this window inherited from a previous page load (see `State.inheritedBatch`) ends
         * here and goes quiet: it has no rows to settle, and its tally describes files the user
         * cannot see. Reporting "3 converted" over an untouched queue, or asking about a helper
         * those files needed, would be this window taking credit for work it never watched.
         */
        if (state.inheritedBatch) {
          set({
            phase: "idle",
            inheritedBatch: false,
            stopping: false,
            stopRequested: false,
            summary: null,
          });
          return;
        }
        const files = { ...state.files };
        // Anything from this batch that never reached a terminal event (cancelled mid-flight, or a
        // worker that died before reporting) settles here, so no row keeps a spinner forever.
        for (const id of state.batchIds) {
          const row = files[id];
          if (row !== undefined && !TERMINAL.has(row.status)) {
            files[id] = { ...row, status: "skipped", message: "Stopped", fraction: null, eta_secs: null };
          }
        }
        set({
          files,
          phase: "finished",
          stopping: false,
          stopRequested: false,
          summary: { ok: event.ok, failed: event.failed, skipped: event.skipped },
        });
        // Every row in this batch is terminal now, so this is the one moment the whole run can be
        // read at once — and therefore the only honest place to ask about a helper it needed.
        askAboutHelpers(files, state.batchIds, state.stopRequested);
        askAboutSignIn(files, state.batchIds, state.stopRequested);
        return;
      }

      // Per-item events are only meaningful for the rows this batch was started with. A row that
      // was added after the run began, an id from an older run, or - after a reload - an id from
      // the batch this window inherited and has no rows for at all, is not ours to touch.
      if (!state.batchIds.has(event.id)) return;

      switch (event.type) {
        case "started":
          // Only ever promotes a row that is still waiting: a duplicate `started` must not rewind
          // a bar that is already moving, nor revive a row that has finished.
          updateRow(event.id, (row) =>
            row.status === "queued"
              ? { ...row, status: "running", fraction: 0, output: event.output, message: null }
              : row,
          );
          break;
        case "progress":
          updateRow(event.id, (row) =>
            // A tick that arrives after the row already finished is stale: ignore it.
            row.status === "running"
              ? // The phase comes off the sample rather than being guessed from the row: a link
                // downloads and then converts, and the two halves report the same shape of
                // progress. A file only ever sends `converting`.
                { ...row, phase: event.phase, fraction: event.fraction, eta_secs: event.eta_secs }
              : row,
          );
          break;
        case "finished":
          settleRow(event.id, (row) => ({
            ...row,
            status: "done",
            fraction: 1,
            eta_secs: null,
            output: event.outputs[0] ?? row.output,
            bytes: event.bytes,
            message: null,
          }));
          break;
        case "failed":
          settleRow(event.id, (row) => ({
            ...row,
            status: "failed",
            fraction: null,
            eta_secs: null,
            message: event.message,
          }));
          break;
        case "skipped":
          settleRow(event.id, (row) => ({
            ...row,
            status: "skipped",
            fraction: null,
            eta_secs: null,
            message: event.reason,
          }));
          break;
      }
    },

    /**
     * `install://event`, with the same discipline as the batch stream: it is a window-wide
     * subscription that outlives any one install, so an event is only applied when it belongs to
     * the install *this* window started and has not already settled.
     *
     * The exception is an install the *shell* was already running when this page loaded
     * (`State.foreignInstall`): `get_activity` says only that one is in flight, never which helper,
     * so its first event is what names it and this is where it gets adopted.
     *
     * `finished` is the last event for a package, and what it did to the machine is only ever
     * *observed* by Rust (it re-runs discovery), so it is the moment to re-read the tools and the
     * catalog — a format still greyed out after its helper arrived would send the user round the
     * loop again.
     *
     * Re-read on *any* verdict, not only `ok: true`: an install can finish `ok: false` having still
     * landed part of a package ("2 of its 3 programs"), and a row left saying "1 of 3" underneath a
     * message saying 2 would be the UI contradicting itself. A genuine failure changes nothing, so
     * the re-read simply confirms what the row already says.
     */
    handleInstallEvent: (event) => {
      let run = get().install;
      if (run === null) {
        if (!get().foreignInstall) return;
        run = { packageId: event.package_id, status: "running", lines: [], message: null };
        set({ install: run, foreignInstall: false });
      }
      if (run.packageId !== event.package_id) return;
      const current = run;

      switch (event.type) {
        case "started":
          // Nothing to change: the click already put this tool into `running`. A duplicate must not
          // wipe lines that have already arrived, and it must not revive a settled install.
          break;
        case "log":
          if (current.status !== "running") return;
          set({
            install: { ...current, lines: [...current.lines, event.line].slice(-LOG_LINES) },
          });
          break;
        case "finished":
          if (current.status !== "running") return; // first verdict wins
          set({
            install: { ...current, status: event.ok ? "ok" : "failed", message: event.message },
          });
          void get().refreshTools();
          break;
      }
    },
  };
});

/**
 * How much of *one row's* work is done, 0..1 — the number the row's 1px line draws and the bar
 * averages.
 *
 * A dropped file has one job and `fraction` is all of it. A link has two, and the sample only ever
 * reports the half it is in: a fetch at 100% is a row *half* done, and drawing it as a full line
 * that then snapped back to nothing said the row had finished and started again. The text beside it
 * keeps naming the half's own percentage — "Downloading 100%" is true of the download — because that
 * is the number a person watching a fetch wants; this is the number a *bar* may draw.
 */
export const rowProgress = (row: Row): number => {
  const fraction = row.fraction ?? 0;
  if (row.link === null) return fraction;
  return row.phase === "downloading" ? fraction / 2 : 0.5 + fraction / 2;
};

/** Mean of the per-file fractions, counting finished rows as 1 — what the action bar shows. */
export function aggregateProgress(rows: Row[]): number {
  if (rows.length === 0) return 0;
  const total = rows.reduce((sum, row) => {
    if (TERMINAL.has(row.status)) return sum + 1;
    return sum + rowProgress(row);
  }, 0);
  return total / rows.length;
}

export const isTerminal = (status: RowStatus): boolean => TERMINAL.has(status);

/**
 * The *package* a row is waiting on, or null.
 *
 * This is the join the user cannot make for themselves: the row knows it failed, the catalog knows
 * which packages a format needs (`FormatView.needs` is package names), and the plans plus the tool
 * list say how much of each package is here. Derived rather than parsed out of the message text, so
 * it stays right when the wording changes — and so it disappears by itself the moment the package is
 * installed and the catalog is re-read.
 *
 * The source format is checked before the target, in the same order as the Rust planner: a decoder
 * we do not have beats an encoder we do not have. Among the packages a format accepts, one we could
 * actually install wins — HEIC reads with either `sips` or ImageMagick, and only one of those is a
 * button. A package only *part* of which is here still counts as missing: the conversion that needed
 * the absent binary cannot run, and one more click is the fix.
 *
 * A link is the one row whose blocker is not a catalog format at all: yt-dlp unlocks a *source* and
 * a JavaScript runtime unlocks the fetch itself, so no `FormatView.needs` ever names either and the
 * join above cannot find them. Both cases are answered from the tool list directly, and everything
 * downstream — the failed row's microlink, the settled batch's question — then works unchanged,
 * because it only ever reads this shape.
 */
export function missingHelperFor(
  state: Pick<State, "catalog" | "installPlans">,
  row: Row,
): MissingHelper | null {
  const catalog = state.catalog;
  if (catalog === null) return null;
  if (row.status !== "failed" && row.status !== "skipped") return null;

  if (row.link !== null) {
    const absent = (id: string): boolean =>
      catalog.tools.find((t) => t.id === id)?.available !== true;
    /*
     * Which helper this link was waiting on, in the order the fetch needs them.
     *
     * yt-dlp first: without it nothing is fetched at all, so every link row is about that. Then the
     * runtime, which is the same blocker one step further along — yt-dlp is here and it ran, and
     * YouTube would not hand the video over because there was nothing to answer its challenges
     * with. `FetchFailure::NoJsRuntime` is the row that says so, and it is only attributed to a
     * runtime when the tool list agrees this machine has none: neither fact alone is enough (see
     * [`JS_RUNTIME_FAILURE`]).
     */
    const blocker = absent(YT_DLP_TOOL)
      ? YT_DLP_TOOL
      : (row.message ?? "").includes(JS_RUNTIME_FAILURE) && JS_RUNTIME_TOOLS.every(absent)
        ? JS_RUNTIME_TOOLS[0]
        : null;
    if (blocker === null) return null;
    // Through the plans, so the name is the one the settings list and the install button use — and
    // so a helper nothing installs cannot be asked for: Node is in no plan, so it can never be the
    // answer here however the tool list looks.
    const plan = state.installPlans.find((p) => p.tool_ids.includes(blocker));
    return plan === undefined
      ? null
      : // Not a format the catalog knows, and singular because the sentence it goes into reads
        // "<this> needs yt-dlp": "Links needs yt-dlp" is not a sentence anybody wrote.
        { packageId: plan.package_id, name: plan.name, format: "A pasted link" };
  }

  // Read and write availability differ per format (PDF reads with four helpers and writes with
  // one), so the source is looked up among the inputs and the target among the outputs.
  const inputs: FormatView[] = catalog.categories.flatMap((c) => c.inputs);
  const outputs: FormatView[] = catalog.categories.flatMap((c) => c.outputs);
  const source = inputs.find((f) => f.id === row.info.format_id);
  const target = outputs.find((f) => f.id === row.target);
  const blocked = [source, target].find(
    (f) => f !== undefined && !f.available && f.needs.length > 0,
  );
  if (blocked === undefined) return null;

  // `needs` is already the deduped package names the backend collapsed a support list into, in the
  // planner's order of preference, so a name is turned straight back into the row it came from.
  const candidates = blocked.needs
    .map((name) => planNamed(state.installPlans, name))
    .filter(
      (plan): plan is PackageInstallPlan =>
        plan !== undefined && presenceOf(plan, catalog.tools) !== "installed",
    );
  const installable = candidates.find((plan) => plan.can_auto_install);
  const chosen = installable ?? candidates[0];
  return chosen === undefined
    ? null
    : { packageId: chosen.package_id, name: chosen.name, format: blocked.name };
}

/** Package ids already asked about, plus every one this question named. Order is irrelevant. */
const asked = (already: string[], prompt: InstallPrompt): string[] => [
  ...new Set([...already, ...prompt.tools.map((helper) => helper.packageId)]),
];

/**
 * The packages a settled batch was blocked by, worst blocker first.
 *
 * The same derivation as one row's microlink ([`missingHelperFor`]), summed over the run: each row
 * is attributed to exactly one package, so the counts add up to the number of files a missing helper
 * stopped and never double-count a file. Per package rather than per binary for the same reason the
 * settings list is: three rows asking for three Poppler binaries would be one `brew install poppler`
 * asked for three times. Rows that failed for their own reasons — a corrupt input, a write error —
 * contribute nothing, which is what keeps the question from being asked about them.
 */
function blockedHelpers(
  state: Pick<State, "catalog" | "installPlans">,
  rows: Row[],
): BlockedHelper[] {
  /** packageId → files it blocked, and how often each format was the thing it could not do. */
  const tally = new Map<string, { name: string; files: number; formats: Map<string, number> }>();
  for (const row of rows) {
    const helper = missingHelperFor(state, row);
    if (helper === null) continue;
    const entry = tally.get(helper.packageId) ?? {
      name: helper.name,
      files: 0,
      formats: new Map(),
    };
    entry.files += 1;
    entry.formats.set(helper.format, (entry.formats.get(helper.format) ?? 0) + 1);
    tally.set(helper.packageId, entry);
  }
  return [...tally.entries()]
    .map(([packageId, entry]) => ({
      packageId,
      name: entry.name,
      files: entry.files,
      formats: [...entry.formats.entries()]
        .sort((a, b) => b[1] - a[1])
        .map(([format]) => format),
    }))
    // The primary action targets whichever package is holding up the most work.
    .sort((a, b) => b.files - a.files || a.name.localeCompare(b.name));
}
