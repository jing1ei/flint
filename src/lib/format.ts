/** Pure presentation helpers. No IPC, no state — safe to use from any component. */
import type { CatalogView, CategoryId, FormatView, Phase, Settings } from "./types";

/**
 * The row's left gutter: the source extension, set small and wide.
 *
 * Typography instead of an icon set — an emoji glyph in a row would be the loudest thing in the
 * window, and it changes shape on every macOS release. Four characters is the widest that still
 * fits the gutter ("HEIC", "webm" → "WEBM").
 */
export function extensionLabel(name: string): string {
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return "—";
  const ext = name.slice(dot + 1).toUpperCase();
  return ext.length > 4 ? ext.slice(0, 4) : ext;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const label = units[unit] ?? "TB";
  return `${value >= 100 ? Math.round(value) : value.toFixed(1)} ${label}`;
}

function formatEta(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "";
  const total = Math.round(seconds);
  if (total < 60) return `${total}s`;
  const minutes = Math.floor(total / 60);
  if (minutes < 60) return `${minutes}m ${String(total % 60).padStart(2, "0")}s`;
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
}

/**
 * ` · 2m 30s left`, or nothing at all — the one derivation the row and the action bar share.
 *
 * Anything that would read "0s left" prints nothing instead. `progress::eta_secs` clamps the
 * remaining media at zero, and a container whose duration metadata under-reports (common in MOV and
 * MKV) keeps encoding long after `out_time` has passed it — so the honest answer for the rest of that
 * file is nought, and "0s left" beside a bar that is still moving is the one number the user can see
 * is wrong. A value `formatEta` cannot render prints nothing at all, rather than the word "left" on
 * its own.
 */
export function etaSuffix(seconds: number | null): string {
  if (seconds === null) return "";
  const label = formatEta(seconds);
  return label === "" || label === "0s" ? "" : ` · ${label} left`;
}

/** `Web & Demo — 1080p H.264, MP3 192k, JPEG 2560px` from the preset description. */
export function presetLine(catalog: CatalogView | null, settings: Settings | null): string {
  if (catalog === null || settings === null) return "Loading defaults…";
  const preset = catalog.presets.find((p) => p.id === settings.preset);
  if (preset === undefined) return "Custom settings";
  const [headline] = preset.description.split(". ");
  return `${preset.label} — ${(headline ?? preset.description).replace(/\s·\s/g, ", ")}`;
}

/** Just the preset's name — the action bar shows this and keeps the full line as its tooltip. */
export function presetLabel(catalog: CatalogView | null, settings: Settings | null): string {
  if (catalog === null || settings === null) return "";
  return catalog.presets.find((p) => p.id === settings.preset)?.label ?? "Custom";
}

/** Option text for a target format: the name, plus the missing helper when it is unavailable. */
export const optionLabel = (format: FormatView): string =>
  format.available ? format.name : `${format.name} — needs ${format.needs.join(" or ")}`;

export const baseName = (path: string): string => path.split(/[\\/]/).pop() ?? path;

/** Parent folder, preserving drive roots; a bare filename has no known folder. */
export const parentDir = (path: string): string => {
  const separator = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  if (separator < 0) return "";
  // C: is drive-relative on Windows; C:\ must retain its separator when opened.
  if (separator === 0 || (separator === 2 && /^[A-Za-z]:/.test(path))) {
    return path.slice(0, separator + 1);
  }
  return path.slice(0, separator);
};

/**
 * Outcome of the run that just ended - *not* of everything on screen.
 *
 * The distinction is load-bearing: convert five files, then retry one that failed, and the tally
 * is "0 converted, 1 failed" while five green rows are still visible. Saying "this run" turns an
 * apparent contradiction into a fact.
 */
export function summaryLine(ok: number, failed: number, skipped: number): string {
  const parts: string[] = [`${ok} converted`];
  if (failed > 0) parts.push(`${failed} failed`);
  if (skipped > 0) parts.push(`${skipped} skipped`);
  return `This run: ${parts.join(" · ")}`;
}

const CATEGORY_PLURAL: Readonly<Record<CategoryId, string>> = {
  video: "videos",
  audio: "audio files",
  image: "images",
  document: "documents",
  subtitle: "subtitles",
  flash: "Flash files",
};

export const categoryPlural = (category: CategoryId): string => CATEGORY_PLURAL[category];

export const pluralFiles = (count: number): string => (count === 1 ? "file" : "files");

/**
 * "link" / "links" — the same job as [`pluralFiles`], for the rows that are not files.
 *
 * A pasted link has no file behind it until it has been fetched, so a question about two of them
 * that said "2 files" would be naming something the queue does not hold.
 */
export const pluralLinks = (count: number): string => (count === 1 ? "link" : "links");

/**
 * A whole number with thousands separated: `5000` → `5,000`.
 *
 * Grouped by hand rather than through `Intl`: the app's copy is English, and the one number this is
 * for ends up in a sentence the behavioural suite matches character for character.
 */
const formatCount = (count: number): string =>
  String(Math.trunc(count)).replace(/\B(?=(\d{3})+(?!\d))/g, ",");

/**
 * What a drop that hit the enumeration cap says — quietly, once, and with the number in it.
 *
 * The cap is the only reason the queue is short, and a queue that silently stopped would send the
 * user looking through their folder for files the app had decided not to mention. So it is said
 * plainly, and it is not a failure: the files that did land are already convertible, which is why
 * the second sentence is an invitation rather than an apology. The count comes from
 * `Inspection.limit`, so this sentence cannot drift from the cap the backend actually applied.
 */
export function truncationNotice(limit: number): string {
  const files = limit === 1 ? "file was" : "files were";
  return `Only the first ${formatCount(limit)} ${files} added. Convert these, then drop the rest.`;
}

/**
 * A pasted link, short enough to sit in a row: host and path, without the noise.
 *
 * `https://` and `www.` carry no information a user needs to tell two links apart, and a YouTube
 * URL that came off a share sheet can be 200 characters of tracking parameters. What is left is
 * elided in the *middle* rather than the end, because the tail of a link (the video id) is the part
 * that differs between two rows from the same channel. The full URL is still the row's `title`.
 */
export function shortUrl(url: string): string {
  const bare = url
    .replace(/^[a-z]+:\/\//i, "")
    .replace(/^www\./i, "")
    .replace(/\/$/, "");
  if (bare.length <= 48) return bare;
  return `${bare.slice(0, 28)}…${bare.slice(-16)}`;
}

/**
 * What a paste that ran past the link cap says, in the quiet notice slot.
 *
 * The cap is over the whole queue, not over one paste, so this can fire on a paste that would have
 * been fine on its own — which is why it names the number rather than saying "too many". A link that
 * silently failed to arrive is a link the user pastes again.
 */
export const linkCapNotice = (cap: number): string =>
  `Only ${formatCount(cap)} links can be queued at once. Convert these, then paste the rest.`;

/**
 * What a paste the queue had already seen says, in the same quiet slot.
 *
 * A link is queued once — two rows for one URL would race each other for one output path — and the
 * box cannot know what the queue holds, so it offers "Add 2 links" for a paste of two the user
 * queued a minute ago and then adds neither. Without this the sheet simply closed over an unchanged
 * queue, which reads as a lost click; with it, the reason is on screen.
 */
export const linkDuplicateNotice = (count: number): string =>
  count === 1
    ? "That link is already in the queue."
    : `Those ${formatCount(count)} links are already in the queue.`;

/**
 * Why a batch was not started: it carries more links than the core will accept.
 *
 * Said once, before the queue is touched, rather than letting Rust refuse the batch and leaving the
 * user with a wall of identically failed rows to read.
 */
export const linkCapRefusal = (count: number, cap: number): string =>
  `${formatCount(count)} links in the queue, and only ${formatCount(cap)} can convert at once. Remove some, or convert them in batches.`;

/** The stem of a path or filename: `A Talk About Rust.mp4` → `A Talk About Rust`. */
export const stemOf = (path: string): string => {
  const name = baseName(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
};

/**
 * A link row's gutter, where a file row carries its extension.
 *
 * `YT` / `BILI` rather than the site's full name: the gutter is four characters wide and set in the
 * same wide small caps as `WEBM`, so a link reads as one more row in the list rather than as a
 * special case. `LINK` is the honest answer for a site this build does not know about yet.
 */
export function linkLabel(site: string | null): string {
  if (site === "youtube") return "YT";
  if (site === "bilibili") return "BILI";
  if (site === "qqmusic") return "QQ";
  if (site === "netease") return "NET";
  if (site === "soundcloud") return "SC";
  if (site === "bandcamp") return "BAND";
  return "LINK";
}

// ---------------------------------------------------------------------------------------------
// The trim, said out loud
// ---------------------------------------------------------------------------------------------

/**
 * `0:12`, `2:02:05` — `probe::seconds_label`, digit for digit.
 *
 * The one spelling of a length of time this app has. Every duration a row shows is Rust's
 * (`FileInfo.duration_label` comes through the same function), and the two sentences below name
 * durations Rust never computed — so they are spelled here rather than with a second format, or a
 * trimmed length would read like a different kind of number to the source length beside it.
 * Truncated, not rounded, for the same reason Rust truncates: 9.9 seconds of media is `0:09`, and a
 * clip that says `0:10` when it is a frame short of it is the kind of small lie nobody can debug.
 *
 * `mock.ts` spells it a second time (`durationLabel`) rather than importing this. That is not an
 * oversight: the mock is the *backend's* stand-in and imports nothing from the app, so the two
 * spellings stay independent and a drift between app and backend remains something the suite can
 * see (assertions 322 and 402 hold them to the same digits for the same 12.6 seconds).
 */
export function secondsLabel(secs: number): string {
  const total = Math.max(0, Math.trunc(secs));
  const mm = Math.floor((total % 3600) / 60);
  const ss = total % 60;
  const pad = (n: number): string => String(n).padStart(2, "0");
  return total >= 3600 ? `${Math.floor(total / 3600)}:${pad(mm)}:${pad(ss)}` : `${mm}:${pad(ss)}`;
}

/**
 * A typed timestamp, back into seconds: `10`, `10.5`, `1:05`, `1:02:03`. Null when it is not one.
 *
 * People write "10" when they mean ten seconds and "1:05" when they mean sixty-five, and both are
 * the same field — so both are read, rather than making the user convert their own minutes. Null is
 * the important return: `1:` is what `1:05` looks like halfway through being typed, and a parser
 * that answered `60` (or `0`) would rewrite the box under the user's fingers.
 *
 * An empty box is `0`, not null: for the start that is "from the beginning", and for the length it
 * is the *unfinished* state `settings_store::classify_trim` holds back in silence. A minus sign is
 * read and passed on rather than clamped away — `-5` is a number the user really typed, and the
 * refusal for it is the backend's to give.
 */
export function parseSeconds(raw: string): number | null {
  const text = raw.trim();
  if (text === "") return 0;
  const sign = text.startsWith("-") ? -1 : 1;
  const parts = text.replace(/^[+-]/, "").split(":");
  if (parts.length > 3) return null;
  if (!parts.every((part) => /^\d+(\.\d+)?$/.test(part))) return null;
  // `1:05` is 65 whichever way it is written; the last group is always seconds.
  const seconds = parts.reduce((total, part) => total * 60 + Number(part), 0);
  return Number.isFinite(seconds) ? sign * seconds : null;
}

/**
 * How a trim value is written back into its own field: `10`, `0.5`, `1:05`.
 *
 * Under a minute it stays a plain number of seconds, which is what the user typed and what they can
 * edit with one keystroke; from a minute up it becomes the timestamp, because `605` is not a number
 * anybody reads as ten minutes. Both forms are accepted by [`parseSeconds`], so what the field shows
 * is always something it will take back.
 */
export const secondsField = (secs: number): string =>
  secs >= 60 ? secondsLabel(secs) : String(Math.round(secs * 1000) / 1000);

const ANIMATED_IMAGE: ReadonlySet<string> = new Set(["gif", "webp", "avif", "apng"]);

/**
 * Does this pairing produce something with a duration? — `plan::output_is_time_based`, condition
 * for condition.
 *
 * The question the trim turns on, and it is about the *output*: video, audio, an animated GIF/WebP
 * made from something that moves, and a frame sequence pulled out of a moving source are all cuts
 * of a timeline; a JPEG, a PDF, a subtitle and Ruffle's still-frame dump are not. Restated on this
 * side because the row has to know whether to say anything *before* the batch runs, and a row that
 * guessed would promise a cut to a PNG.
 */
export function outputIsTimeBased(
  catalog: CatalogView | null,
  sourceCategory: CategoryId | null,
  sourceIsAnimated: boolean,
  targetId: string,
): boolean {
  if (catalog === null || sourceCategory === null) return false;
  const target = catalog.categories
    .flatMap((category) => category.outputs.map((format) => [category.id, format] as const))
    .find(([, format]) => format.id === targetId);
  if (target === undefined) return false;
  const [targetCategory] = target;
  if (targetCategory === "video" || targetCategory === "audio") return true;
  if (targetCategory !== "image") return false;
  // Flash to stills is Ruffle's frame dump: no encoder runs, so there is nothing to trim.
  if (sourceCategory === "flash") return ANIMATED_IMAGE.has(targetId);
  // An animated GIF/WebP has a duration only when the source moves, and a frame sequence
  // (`plan::writes_frame_sequence`) is pulled out of one for exactly the same reason — so both
  // image cases collapse to the one question.
  return sourceIsAnimated;
}

/** `settings::MAX_TRIM_SECS`: 24 hours, the end of the range where these numbers are a duration. */
const MAX_TRIM_SECS = 24 * 60 * 60;

/** `settings::sane_secs`, so the two sides cannot disagree about what an absurd number means. */
const saneSecs = (value: number): number =>
  Number.isFinite(value) ? Math.min(MAX_TRIM_SECS, Math.max(0, value)) : 0;

/**
 * `TrimSettings::effective`: the pair to act on, or null when this trim asks for nothing.
 *
 * Clamped rather than rejected, exactly as Rust clamps: a value the backend refuses (a negative
 * start, a week) is still on screen for as long as it takes the toast to explain itself, and in
 * that moment the bar should say what the app *would* do with it rather than fall silent about a
 * trim that is still switched on.
 */
const effectiveTrim = (settings: Settings | null): { start: number; length: number } | null => {
  if (settings === null || !settings.trim.enabled) return null;
  const length = saneSecs(settings.trim.length_secs);
  // A length of zero is not a trim to obey: it is a field nobody has finished typing.
  return length > 0 ? { start: saneSecs(settings.trim.start_secs), length } : null;
};

/**
 * What the action bar says about the trim, beside where the files land: `Trimming to 0:10`, or
 * `Trimming to 0:10 from 0:30` when the cut does not start at the beginning.
 *
 * A setting that quietly shortens every file is the one dangerous thing this app persists, so it is
 * said where the user is already looking rather than only in a sheet they have to open. The start
 * is named only when it is not zero: "from 0:00" is noise in a line that has a folder path in it.
 * Empty when trimming is off, which is what keeps the bar quiet in the state nearly every run is in.
 */
export function trimSummary(settings: Settings | null): string {
  const trim = effectiveTrim(settings);
  if (trim === null) return "";
  const from = trim.start > 0 ? ` from ${secondsLabel(trim.start)}` : "";
  return `Trimming to ${secondsLabel(trim.length)}${from}`;
}

/**
 * What *one row* will become: `Trimmed to 0:10`, or nothing at all.
 *
 * `min(length, max(0, duration - start))` — `TrimSettings::expected_output_secs`, which is also the
 * duration the progress bar and the ETA are measured against. Nothing is said unless all three
 * conditions hold, because each silence is a claim this row could not keep:
 *
 *   - the output has no duration (an image, a document, a subtitle): the trim does not touch it;
 *   - the source's length is unknown (a pasted link, until the backend announces one): guessing
 *     would put a number on a row that has none;
 *   - the cut leaves the file exactly as long as it already is, or leaves nothing at all: a clip
 *     shorter than the length keeps its own length silently, and a trim that starts past the end is
 *     a row the backend refuses by name rather than one that quietly becomes `0:00`.
 *
 * The last of those is measured on the *spelling* as well as the number, because a 10.4 second clip
 * trimmed to 10 seconds would otherwise read `0:10 · Trimmed to 0:10`, which is a row arguing with
 * itself over four tenths of a second nobody can see.
 */
export function trimmedLength(
  settings: Settings | null,
  durationSecs: number | null,
  timeBased: boolean,
): string {
  const trim = effectiveTrim(settings);
  if (trim === null || !timeBased || durationSecs === null || !Number.isFinite(durationSecs)) {
    return "";
  }
  const kept = Math.min(trim.length, Math.max(0, durationSecs - trim.start));
  if (kept <= 0 || kept >= durationSecs) return "";
  const label = secondsLabel(kept);
  return label === secondsLabel(durationSecs) ? "" : `Trimmed to ${label}`;
}

/**
 * What half of the job a running link row is in: `Downloading 42%`, `Converting 71%`.
 *
 * `Downloading…` with no number while the fraction is indeterminate — yt-dlp says nothing about how
 * big a video is until it has finished negotiating with the site, and a bar that sat at "0%" for
 * those few seconds was read as a hang. A file row never comes through here: it only ever converts,
 * and its status text is the bare percentage it has always been.
 */
export function phaseStatus(phase: Phase, fraction: number | null): string {
  const label = phase === "downloading" ? "Downloading" : "Converting";
  return fraction === null ? `${label}…` : `${label} ${Math.round(fraction * 100)}%`;
}
