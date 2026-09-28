/**
 * Hand-written mirrors of the Rust structs in `crates/convert-core` and `src-tauri/src/commands.rs`.
 *
 * Field names are snake_case because serde serialises them that way; enum string values match
 * `#[serde(rename_all = "snake_case")]`. Nothing here is optional-by-`?` unless the Rust side can
 * actually omit the key — `Option<T>` becomes `T | null`, which is what serde_json emits.
 */

// ---------------------------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------------------------

export type CategoryId = "video" | "audio" | "image" | "document" | "subtitle" | "flash";

export interface FormatView {
  id: string;
  name: string;
  extension: string;
  extensions: string[];
  notes: string;
  available: boolean;
  needs: string[];
}

export interface CategoryView {
  id: CategoryId;
  label: string;
  default_target: string;
  suggested_targets: string[];
  inputs: FormatView[];
  outputs: FormatView[];
}

export interface PresetView {
  id: PresetId;
  label: string;
  description: string;
}

/**
 * One *binary* the app spawns, and whether this machine has it.
 *
 * `id` is the join key every package plan names in [`PackageInstallPlan.tool_ids`]; presence is read
 * from here and nowhere else. `label` is a **diagnostic** name ("Poppler (pdftohtml)") — right in a
 * log or in `FORMATS.md`, and never what the UI asks a user to install: nobody installs `pdftohtml`,
 * they install Poppler. Use [`PackageInstallPlan.name`] for that.
 */
export interface ToolStatus {
  id: string;
  /** Diagnostic name, executable-level. Not user-facing install copy — see the note above. */
  label: string;
  bundled: boolean;
  available: boolean;
  path: string | null;
  install_hint: string;
}

export interface CatalogView {
  categories: CategoryView[];
  presets: PresetView[];
  tools: ToolStatus[];
  input_extension_count: number;
}

// ---------------------------------------------------------------------------------------------
// Installing the optional helpers — one row per *package*, never per binary
// ---------------------------------------------------------------------------------------------

/**
 * Which package manager could install a helper on *this* machine.
 *
 * `"none"` is `convert_core::install::NO_MANAGER`: this platform has no manager we know how to
 * drive. A future `"winget"` would arrive here as a plain string, so the UI branches on the
 * *booleans* below, never exhaustively on this.
 */
export type InstallManager = "homebrew" | "none";

/**
 * `get_install_plans()` — one row of the helper list: how you would get it, and why you would.
 *
 * A *package* is what a person installs; a tool is a binary we spawn. Poppler is one Homebrew
 * formula shipping `pdftoppm`, `pdftotext` and `pdftohtml`, so it is one row, one command and one
 * name — mirrors `convert_core::install::PackageInstallPlan`.
 *
 * Nothing here says whether the package is *present*. That is derived by joining [`tool_ids`]
 * against [`ToolStatus.id`] (see `src/lib/packages.ts`): all present → installed, **some** present →
 * incomplete and still installable, none → absent.
 */
export interface PackageInstallPlan {
  /** Stable id, and the only thing sent back to start an install (`invoke('install_tool', …)`). */
  package_id: string;
  /** The display name every user-facing string uses: this row, the prompt, a failed row's link. */
  name: string;
  /** The binaries this package provides; each joins onto `ToolStatus.id`. */
  tool_ids: string[];
  manager: InstallManager;
  /** Is that manager actually installed? False here is why `can_auto_install` can be false. */
  manager_available: boolean;
  /** The exact command, e.g. `brew install --cask libreoffice`. Empty when there is no manager. */
  command: string;
  /** The installer may ask for a password, which this app has no terminal to type one into. */
  needs_admin: boolean;
  can_auto_install: boolean;
  /** Plain-language sentences: "Open and save documents: PDF, Word (docx) and 4 more". */
  unlocks: string[];
}

// ---------------------------------------------------------------------------------------------
// Install events (`install://event`, serde tagged on `type` — same shape as `batch://event`)
// ---------------------------------------------------------------------------------------------

export type InstallEvent =
  | { type: "started"; package_id: string }
  | { type: "log"; package_id: string; line: string }
  /** Always the last event for a package. `message` is shown verbatim, success or failure. */
  | { type: "finished"; package_id: string; ok: boolean; message: string };

// ---------------------------------------------------------------------------------------------
// What the shell is busy with (`get_activity`)
// ---------------------------------------------------------------------------------------------

/**
 * `get_activity()` — the two long-running things the *shell* owns, as booleans.
 *
 * A webview reload throws away every listener and this whole store, but not the batch or the
 * install: the window used to come back believing it was idle while a conversion it could neither
 * see nor stop still held the single slot, and the next Convert was refused. This is the one
 * question a freshly loaded page can ask to find that out.
 */
export interface Activity {
  converting: boolean;
  installing: boolean;
}

// ---------------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------------

export type PresetId = "web_and_demo" | "smallest" | "high_quality" | "archive";
export type QualityLevel = "small" | "balanced" | "high" | "max";
export type VideoCodec = "auto" | "h264" | "h265" | "vp9" | "av1" | "pro_res" | "copy";
export type AudioCodec =
  | "auto"
  | "mp3"
  | "aac"
  | "opus"
  | "vorbis"
  | "flac"
  | "alac"
  | "pcm_wav"
  | "copy";
export type HardwareAccel = "auto" | "off";
export type OutputLocation = "same_folder" | "subfolder" | "custom";
export type ConflictPolicy = "rename" | "overwrite" | "skip";

export interface VideoSettings {
  codec: VideoCodec;
  quality: QualityLevel;
  max_height: number | null;
  fps_cap: number | null;
  bitrate_kbps: number | null;
  faststart: boolean;
  strip_metadata: boolean;
  hardware_accel: HardwareAccel;
}

export interface AudioSettings {
  codec: AudioCodec;
  bitrate_kbps: number;
  sample_rate: number | null;
  channels: number | null;
  normalize_loudness: boolean;
}

export interface ImageSettings {
  quality: number;
  max_dimension: number | null;
  strip_metadata: boolean;
  flatten_background: string;
  lossless: boolean;
  frame_extract_fps: number;
}

export interface GifSettings {
  fps: number;
  width: number;
  optimize_palette: boolean;
  loop_count: number;
}

export interface DocumentSettings {
  raster_dpi: number;
  first_page_only: boolean;
}

export interface OutputSettings {
  location: OutputLocation;
  custom_dir: string | null;
  subfolder_name: string;
  on_conflict: ConflictPolicy;
  parallel_jobs: number;
  preserve_timestamps: boolean;
}

/**
 * One cut, applied to every row in the batch that produces something with a duration.
 *
 * Two numbers and a switch, deliberately: there is no preview, no player and nothing per-file, so
 * "keep 10 seconds from 0:30" is the same 10 seconds from the same 0:30 for the whole queue. Both
 * numbers are seconds (`f64` in Rust, so fractions are legal), and both are validated by
 * `settings_store::classify_trim` on the way in — negative, unreadable or longer than 24 hours is
 * refused with a sentence, while *enabled with no length yet* is merely unfinished and is held back
 * in silence, exactly as a half-chosen destination is.
 *
 * A source shorter than `length_secs` keeps its own length; a still image, a document and a
 * subtitle are unaffected, because the thing they produce has no duration to cut.
 */
export interface TrimSettings {
  enabled: boolean;
  start_secs: number;
  length_secs: number;
}

/**
 * Where a pasted link's sign-in comes from, if anywhere: `LinkSettings::cookies`.
 *
 * "none" is the whole app for most links. The other two exist because some media is only handed
 * over to someone who is signed in — an age-restricted video, a members-only upload, Bilibili's
 * higher resolutions — and yt-dlp can borrow a sign-in the user already has rather than asking them
 * for a password this app would then be holding.
 */
export type CookieSource = "none" | "browser" | "file";

/**
 * The browsers yt-dlp can be asked to borrow a sign-in from — `settings::COOKIE_BROWSERS`, in order.
 *
 * Lowercase because these are the words that reach the command line; a visible label may be
 * capitalised, and `settings_store` refuses anything outside this list by name. Not the type of
 * [`LinkSettings.cookie_browser`], on purpose: that field is a `String` in Rust and arrives as one,
 * so the allowlist has to be *checked* rather than assumed by a cast.
 */
export type CookieBrowser =
  | "safari"
  | "chrome"
  | "chromium"
  | "edge"
  | "brave"
  | "firefox"
  | "vivaldi"
  | "opera";

/**
 * One allowlisted browser, whether this machine has it, and where a sign-in most likely is —
 * `settings::BrowserPresence`.
 *
 * The reason it crosses the boundary at all: a menu of eight browsers is seven dead options on a
 * Mac with one installed, and a recovery flow that offers a sign-in the user cannot give is the
 * dead end the whole sign-in walkthrough exists to close.
 *
 * `id` is yt-dlp's own spelling and exactly what [`LinkSettings.cookie_browser`] takes; `label` is
 * the word a sentence uses ("Chrome", never "Google Chrome.app").
 *
 * The rest is *evidence*, and it is here because presence alone picked the wrong browser for a real
 * user: the flow preferred any browser that was not Safari, because Safari's jar costs a permission,
 * and so it offered a Chrome whose cookie store was an untouched 64 KB file while that user's
 * YouTube session sat in the Safari they browse with. `rank` is Rust's one ordering of that evidence
 * (1 is the likeliest place the sign-in is) and `recommended` is rank 1 when it is worth offering at
 * all. None of these fields is the *content* of a cookie store: `stat` reports a size and a time,
 * and nothing in this app opens the file to produce them.
 */
export interface BrowserPresence {
  id: string;
  label: string;
  installed: boolean;
  /** Where the bundle was found, for a diagnostic. Null when it is not installed. */
  app_path: string | null;
  /** True for the browser macOS opens an `https://` link with. At most one row, and never two. */
  is_default: boolean;
  /** The cookie store found on disk, for a diagnostic. Null when there is none. */
  cookie_store: string | null;
  /** Is that file there at all? The one fact that decides whether borrowing can work. */
  cookie_store_exists: boolean;
  /** Its size from `stat`. A Chromium browser opened once and never signed in to is 65,536. */
  cookie_store_bytes: number | null;
  /** When it was last written, in whole seconds since the Unix epoch. */
  cookie_store_modified: number | null;
  /** True for Safari alone: its jar is behind Full Disk Access. A cost to state, never a demotion. */
  needs_full_disk_access: boolean;
  /** Where this row comes in `settings::rank_browsers`, 1 being the likeliest. Unique, 1..n. */
  rank: number;
  /** True for rank 1, and only when that row is installed with a cookie store that exists. */
  recommended: boolean;
}

/**
 * What one `open` of Safari's cookie jar proved — `link::SafariCookieAccess`.
 *
 * Four answers, because the four have four different fixes: the permission is in place, the
 * permission is missing, Safari has no cookies to lend in the first place, or the file is there and
 * would not open for some other reason.
 */
export type SafariCookieAccess =
  | "readable"
  | "needs_full_disk_access"
  | "no_cookie_store"
  | "unreadable";

/**
 * `check_safari_cookie_access()` — the permission question answered locally, and instantly.
 *
 * The old route to this answer was the twenty-second network probe `test_cookie_source` runs, which
 * is a strange way to find out whether an app has a macOS permission: one `open(2)` settles it in
 * microseconds, and `EPERM` on that path means Full Disk Access and nothing else. No byte of the
 * jar is read, so nothing about the user's cookies can be in this payload.
 */
export interface SafariAccess {
  result: SafariCookieAccess;
  /** True for `readable` only — the file opened. It does not say the site will accept what is in it. */
  ok: boolean;
  /** The sentence to show, in the words a failed row would have used for the same cause. */
  message: string;
  /** The file that was opened, for a diagnostic. */
  cookie_store: string | null;
}

/**
 * What one sign-in check proved — `link::CookieCheck`.
 *
 * Three answers the user can act on and two they cannot mistake for the others: "it works", "the
 * sign-in could not be read", and "it was read and the site said no" send people to three different
 * places, and before the check existed all three read as the same red line of text.
 */
export type CookieCheck = "working" | "unreadable" | "refused" | "inconclusive" | "not_configured";

/** `test_cookie_source()` — one bounded probe of a public video, and the verdict it produced. */
export interface CookieTest {
  result: CookieCheck;
  /** True for `working` only. The one field a "retry those rows now" decision may look at. */
  ok: boolean;
  /** The sentence to show, in the words a failed row would have used for the same cause. */
  message: string;
  /** The public video the check was run against — `link::COOKIE_TEST_URL`. */
  tested_url: string;
}

/**
 * How a pasted link proves who is watching — the sign-in, never the password.
 *
 * `settings_store` treats two states as merely *unfinished*, exactly as a half-chosen destination or
 * a half-typed trim is: "from a browser" with no browser picked yet, and "from a file" with no file
 * chosen yet. Both are held back in silence, other edits in the same save still landing, and both
 * say what is missing as an inline hint rather than as an error.
 *
 * Everything else is a value the user really chose and its refusal is theirs to read: a browser
 * outside [`CookieBrowser`], a relative path, a path that is not there, a folder instead of a file.
 *
 * A preset never touches this group, for the reason it never touches the trim: which account the
 * user is signed in as is not an opinion about quality.
 */
export interface LinkSettings {
  cookies: CookieSource;
  /** A [`CookieBrowser`], or "" while nothing is picked. A `String` in Rust, so a string here. */
  cookie_browser: string;
  /**
   * Absolute path to an exported cookies.txt — `Option<PathBuf>`, and null until one is chosen.
   *
   * A path is all the frontend ever holds: the file's *contents* are a sign-in, they are read by
   * yt-dlp in the Rust process, and nothing in this app ever displays or stores a cookie value.
   */
  cookie_file: string | null;
}

export interface CropSettings {
  image: { x: number; y: number; width: number; height: number } | null;
  media: { start_secs: number; length_secs: number } | null;
  document: { unit: "pages" | "words"; start: number; end: number } | null;
}

export interface Settings {
  crop?: CropSettings | null;
  preset: PresetId;
  video: VideoSettings;
  audio: AudioSettings;
  image: ImageSettings;
  gif: GifSettings;
  document: DocumentSettings;
  output: OutputSettings;
  /** Applies across formats, and a preset never touches it — see [`TrimSettings`]. */
  trim: TrimSettings;
  /** Pasted links only, and a preset never touches it either — see [`LinkSettings`]. */
  link: LinkSettings;
}

// ---------------------------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------------------------

export interface FileInfo {
  id: string;
  path: string;
  name: string;
  size_bytes: number;
  supported: boolean;
  category: CategoryId | null;
  format_id: string | null;
  format_name: string | null;
  default_target: string | null;
  suggested_targets: string[];
  duration_secs: number | null;
  duration_label: string | null;
  resolution_label: string | null;
  is_animated: boolean;
  note: string | null;
}

/**
 * What one `inspect_files` produced: the rows, and whether the cap cut the enumeration short.
 *
 * Dropping a home folder used to walk every directory on the disk and build a row for each, which
 * is minutes of unresponsive window; the walk now *stops* at [`Inspection.limit`]. A queue that
 * silently stops at 5000 files is indistinguishable from a folder that held 5000 files, so the flag
 * travels with the count the UI names — the number is read from here, never hard-coded, so the two
 * sides cannot drift.
 *
 * Field names are exactly the Rust ones (`commands::Inspection`, no `rename_all`).
 */
export interface Inspection {
  files: FileInfo[];
  /** True only when something was genuinely left out, never merely because the count hit the cap. */
  truncated: boolean;
  /** The cap that was actually applied, currently 5000. */
  limit: number;
  /** Paths omitted because of permissions, nesting, or the directory-scan budget. */
  warnings?: string[];
}

/**
 * One queued row, as `start_batch` takes it — `commands::BatchItemArg`.
 *
 * Exactly one of `path` and `url` is set: a row is *either* a dropped file or a pasted video link,
 * never both and never neither. Rust refuses the other two combinations rather than guessing, so
 * the two fields are kept apart here as well instead of collapsing into one "source" string.
 */
export interface BatchItemArg {
  id: string;
  path?: string;
  url?: string;
  target_id: string;
}

// ---------------------------------------------------------------------------------------------
// Links as a source
// ---------------------------------------------------------------------------------------------

/**
 * One pasted line, judged by `inspect_links` — `commands::LinkRow`.
 *
 * A refused line is a row too, with `supported: false` and a `note` that says what to do instead:
 * one channel URL in a paste of twelve must show *which* line is the problem rather than failing
 * the lot. Every message in `note` comes from `convert_core::link::LinkError` and is shown verbatim
 * — those sentences already name the next step.
 */
export interface LinkRow {
  /** Row id, in the same namespace as `FileInfo.id`. */
  id: string;
  /** The URL as it will be used, trimmed. Passed straight back as `BatchItemArg.url`. */
  url: string;
  /** Source service identifier, or null when the line was refused. */
  site: string | null;
  /** Source service label for the row. */
  site_label: string | null;
  category: "audio" | "video" | null;
  supported: boolean;
  /** Targets from the source's audio or video category. */
  default_target: string | null;
  suggested_targets: string[];
  note: string | null;
}

/** What one paste produced — `commands::LinkInspection`. */
export interface LinkInspection {
  links: LinkRow[];
  /** How many of them are actually convertible. */
  accepted: number;
  /** The cap (`link::MAX_LINKS_PER_BATCH`), so the copy names the number rather than repeating it. */
  limit: number;
}

/**
 * The rules the link UI has to state and enforce — `commands::LinkSupport`.
 *
 * Read from the backend rather than restated in TypeScript: the cap, the host allowlist and the
 * destination are all facts the core owns, and a second copy of any of them would drift.
 */
export interface LinkSupport {
  max_links: number;
  /** Every host that will be accepted, exact match, lower case. */
  accepted_hosts: string[];
  /** False when yt-dlp is absent: links may still be queued, and each fails with an install hint. */
  tool_installed: boolean;
  /** The `package_id` for `install_tool`, and the row to point at in Settings → Helpers. */
  package_id: string;
  /** Where a link's output lands: the custom folder if one is set, `~/Downloads` otherwise. */
  destination: string;
}

// ---------------------------------------------------------------------------------------------
// Batch events (`batch://event`, serde tagged on `type`)
// ---------------------------------------------------------------------------------------------

/**
 * Which half of a job a progress sample is about — `convert_core::progress::Phase`.
 *
 * A dropped file only ever converts; a link is fetched first. Carried on every sample so a row can
 * say "Downloading 42%" and then "Converting 71%" instead of one bar that mysteriously restarts.
 */
export type Phase = "downloading" | "converting";

export type BatchEvent =
  | { type: "started"; id: string; output: string; summary: string }
  | {
      type: "progress";
      id: string;
      phase: Phase;
      fraction: number | null;
      speed: number | null;
      eta_secs: number | null;
    }
  | { type: "finished"; id: string; outputs: string[]; bytes: number; elapsed_ms: number }
  | { type: "failed"; id: string; message: string }
  | { type: "skipped"; id: string; reason: string }
  | { type: "batch_finished"; ok: number; failed: number; skipped: number };
