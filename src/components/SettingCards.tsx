/**
 * The advanced editor: seven collapsible sections over the real `Settings` struct.
 *
 * Native `<details>` gives keyboard support and disclosure semantics for free, and every control
 * is a real `<select>` / `<input>` — a settings panel is the last place to reinvent widgets.
 */
import { useState, type ReactNode } from "react";
import { parseSeconds, secondsField } from "../lib/format";
import type {
  AudioCodec,
  BrowserPresence,
  ConflictPolicy,
  CookieBrowser,
  CookieSource,
  HardwareAccel,
  OutputLocation,
  QualityLevel,
  Settings,
  VideoCodec,
} from "../lib/types";
import { useStore } from "../state/store";

// ------------------------------------------------------------------------------- primitives

interface SectionProps {
  title: string;
  children: ReactNode;
  defaultOpen?: boolean;
}

function Section({ title, children, defaultOpen = false }: SectionProps): React.JSX.Element {
  return (
    <details className="section" open={defaultOpen}>
      <summary className="section__title">{title}</summary>
      <div className="section__body">{children}</div>
    </details>
  );
}

interface FieldProps {
  label: string;
  hint?: string;
  children: ReactNode;
}

function Field({ label, hint, children }: FieldProps): React.JSX.Element {
  return (
    <label className="field">
      <span className="field__label">
        {label}
        {hint !== undefined && hint !== "" && <span className="field__hint">{hint}</span>}
      </span>
      <span className="field__control">{children}</span>
    </label>
  );
}

interface SelectProps<T extends string> {
  label: string;
  value: T;
  options: ReadonlyArray<readonly [T, string]>;
  /** A word about the choice, under the label — including the sentence a half-made one is missing. */
  hint?: string;
  /** A choice that depends on another one is inert until that one has been made, never hidden. */
  disabled?: boolean;
  onChange: (value: T) => void;
}

function SelectField<T extends string>({
  label,
  value,
  options,
  hint,
  disabled = false,
  onChange,
}: SelectProps<T>): React.JSX.Element {
  return (
    <Field label={label} hint={hint ?? ""}>
      <select
        className="control"
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.currentTarget.value as T)}
      >
        {options.map(([id, text]) => (
          <option key={id} value={id}>
            {text}
          </option>
        ))}
      </select>
    </Field>
  );
}

interface SliderProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
}

function SliderField({ label, value, min, max, step = 1, onChange }: SliderProps): React.JSX.Element {
  return (
    <Field label={label}>
      <span className="slider">
        <input
          type="range"
          className="control control--range"
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={(e) => onChange(Number(e.currentTarget.value))}
        />
        <output className="slider__value">{value}</output>
      </span>
    </Field>
  );
}

interface NumberProps {
  label: string;
  value: number | null;
  unit?: string;
  min?: number;
  max?: number;
  step?: number;
  /** The Rust field is `f32`. Everything else is an integer and gets rounded before it is sent. */
  float?: boolean;
  /**
   * Shown while the box is empty — so only worth setting on a field whose value can *be* empty.
   * A field backed by a plain number is refilled with its fallback the moment it is cleared, and a
   * placeholder behind it would never be read by anybody: say it in `hint` instead.
   */
  placeholder?: string;
  /** A word about the value itself, under the label, where it is read whatever the box says. */
  hint?: string;
  onChange: (value: number | null) => void;
}

/**
 * Empty input means `None` on the Rust side ("keep the source value"), which is a real choice.
 *
 * Everything typed here ends up in a `u8`/`u32`/`usize` field, and `save_settings` rejects the whole
 * settings object if one value does not fit — a stray `-5` or `150.5` would turn every later save
 * into a serde error toast. So values are sanitised on the way out: rounded and capped while
 * typing (a lower bound would fight the keystrokes: "1080" starts as "1"), and pulled up to `min`
 * on blur.
 */
function NumberField({
  label,
  value,
  unit,
  min,
  max,
  step,
  float = false,
  placeholder,
  hint,
  onChange,
}: NumberProps): React.JSX.Element {
  const sanitize = (raw: string, floor: number): number | null => {
    if (raw.trim() === "") return null;
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) return null;
    const bounded = Math.min(max ?? Number.MAX_SAFE_INTEGER, Math.max(floor, parsed));
    return float ? bounded : Math.round(bounded);
  };

  return (
    <Field label={label} hint={hint ?? ""}>
      <span className="numberbox">
        <input
          type="number"
          className="control control--number"
          value={value === null ? "" : value}
          min={min}
          max={max}
          step={step}
          placeholder={placeholder}
          onChange={(e) => onChange(sanitize(e.currentTarget.value, 0))}
          onBlur={(e) => {
            const next = sanitize(e.currentTarget.value, min ?? 0);
            if (next !== value) onChange(next);
          }}
        />
        {unit !== undefined && <span className="numberbox__unit">{unit}</span>}
      </span>
    </Field>
  );
}

interface CheckProps {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}

function CheckField({ label, checked, onChange }: CheckProps): React.JSX.Element {
  return (
    <label className="check">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.currentTarget.checked)} />
      <span>{label}</span>
    </label>
  );
}

interface SecondsProps {
  label: string;
  hint: string;
  value: number;
  disabled: boolean;
  onChange: (secs: number) => void;
}

/**
 * A length of time, typed: `10`, `10.5`, `1:05`, `1:02:03`.
 *
 * A `type="number"` box cannot hold `1:05`, and [`NumberField`]'s sanitiser is the wrong instinct
 * here twice over: it would clamp `-5` away, and the sentence for a negative trim is one the
 * *backend* owns and the user is better off reading than never seeing. So the value leaves this
 * field exactly as it was typed, and `save_settings` decides.
 *
 * The draft is why the box is not destructive. `1:05` passes through `1:` on its way in, which
 * [`parseSeconds`] refuses; without a draft the field would re-render from the last good number and
 * eat the colon. While the box has focus the characters stand as typed, and only a parseable value
 * is pushed into the settings; on blur the draft is dropped and the canonical spelling comes back.
 */
function SecondsField({ label, hint, value, disabled, onChange }: SecondsProps): React.JSX.Element {
  const [draft, setDraft] = useState<string | null>(null);
  return (
    <Field label={label} hint={hint}>
      <input
        type="text"
        inputMode="text"
        className="control control--number"
        value={draft ?? secondsField(value)}
        disabled={disabled}
        onChange={(e) => {
          const typed = e.currentTarget.value;
          setDraft(typed);
          const secs = parseSeconds(typed);
          if (secs !== null) onChange(secs);
        }}
        onBlur={() => setDraft(null)}
      />
    </Field>
  );
}

// ------------------------------------------------------------------------------- sections

const VIDEO_CODECS: ReadonlyArray<readonly [VideoCodec, string]> = [
  ["auto", "Automatic (recommended)"],
  ["h264", "H.264 — plays everywhere"],
  ["h265", "H.265 / HEVC — smaller"],
  ["vp9", "VP9 — web"],
  ["av1", "AV1 — smallest, slow"],
  ["pro_res", "ProRes — editing"],
  ["copy", "Copy stream (remux only)"],
];

const QUALITY: ReadonlyArray<readonly [QualityLevel, string]> = [
  ["small", "Small"],
  ["balanced", "Balanced"],
  ["high", "High"],
  ["max", "Maximum"],
];

const AUDIO_CODECS: ReadonlyArray<readonly [AudioCodec, string]> = [
  ["auto", "Automatic (recommended)"],
  ["mp3", "MP3"],
  ["aac", "AAC"],
  ["opus", "Opus"],
  ["vorbis", "Vorbis"],
  ["flac", "FLAC (lossless)"],
  ["alac", "ALAC (lossless)"],
  ["pcm_wav", "PCM / WAV"],
  ["copy", "Copy stream"],
];

const HW: ReadonlyArray<readonly [HardwareAccel, string]> = [
  ["auto", "Use hardware when possible"],
  ["off", "Software only"],
];

const LOCATIONS: ReadonlyArray<readonly [OutputLocation, string]> = [
  ["same_folder", "Next to the original"],
  ["subfolder", "In a subfolder"],
  ["custom", "In a folder I choose"],
];

const CONFLICTS: ReadonlyArray<readonly [ConflictPolicy, string]> = [
  ["rename", "Keep both (rename)"],
  ["overwrite", "Overwrite"],
  ["skip", "Skip"],
];

const COOKIE_SOURCES: ReadonlyArray<readonly [CookieSource, string]> = [
  ["none", "Not signed in"],
  ["browser", "Borrow it from a browser"],
  ["file", "Read it from a cookies.txt file"],
];

/**
 * The eight browsers `settings_store` allows, plus the empty value that means nothing is picked yet.
 *
 * The id is what is sent and it is lowercase, exactly as the allowlist spells it; the capitalisation
 * lives in the label, where a person reads it. Anything else is refused by name in the backend's own
 * sentence, which is why this list — and not a free-text box — is the control.
 */
const COOKIE_BROWSER_OPTIONS: ReadonlyArray<readonly ["" | CookieBrowser, string]> = [
  ["", "No browser chosen"],
  ["safari", "Safari"],
  ["chrome", "Chrome"],
  ["chromium", "Chromium"],
  ["edge", "Edge"],
  ["brave", "Brave"],
  ["firefox", "Firefox"],
  ["vivaldi", "Vivaldi"],
  ["opera", "Opera"],
];

/**
 * Which option the browser select shows for whatever string the settings hold.
 *
 * `cookie_browser` is a `String` in Rust and this app is not the only thing that can write the
 * settings file, so a name the list does not offer has to land somewhere: it reads as *nothing
 * chosen* rather than as a ninth, invented option, and the group stays visibly unfinished until a
 * real choice replaces it. The match ignores case because `settings::cookie_browser` does — a
 * hand-written "Safari" is a value the backend accepts, so the select has to show it as Safari
 * rather than as nothing at all.
 */
/**
 * The browser picker, with the ones this machine does not have kept apart from the ones it does.
 *
 * A flat menu of eight browsers on a Mac with two is six ways to configure a sign-in that can never
 * be read, and the failure that produces ("could not read the sign-in from that browser") arrives a
 * whole conversion later. `list_cookie_browsers` is measured, so the menu can be honest at the
 * moment of choosing instead.
 *
 * Grouped rather than removed or greyed out, and for two reasons: a browser that is absent today
 * may be installed tomorrow, and a name that simply vanished would leave "why is Firefox not in
 * here?" as the user's problem. The heading answers it. Every option keeps the allowlist's own
 * lowercase value and its own label — the group is what says "not here", never the label, because
 * the label is also what a `--cookies-from-browser` argument is chosen by.
 *
 * With nothing measured yet — an older shell, or the first seconds of a launch — the list is one
 * flat group, unmarked. Guessing which browsers are absent would be worse than saying nothing.
 */
function BrowserField({
  value,
  browsers,
  hint,
  disabled,
  onChange,
}: {
  value: "" | CookieBrowser;
  browsers: BrowserPresence[];
  hint: string;
  disabled: boolean;
  onChange: (value: string) => void;
}): React.JSX.Element {
  const installed = new Map(browsers.map((browser) => [browser.id, browser.installed]));
  const known = browsers.length > 0;
  const option = ([id, label]: readonly ["" | CookieBrowser, string]): React.JSX.Element => (
    <option key={id} value={id}>
      {label}
    </option>
  );
  const here = COOKIE_BROWSER_OPTIONS.filter(([id]) => !known || installed.get(id) !== false);
  const absent = COOKIE_BROWSER_OPTIONS.filter(([id]) => known && installed.get(id) === false);
  return (
    <Field label="Browser" hint={hint}>
      <select
        className="control"
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.currentTarget.value)}
      >
        {here.map(option)}
        {absent.length > 0 && (
          <optgroup label="Not installed on this Mac">{absent.map(option)}</optgroup>
        )}
      </select>
    </Field>
  );
}

const browserOption = (value: string): "" | CookieBrowser => {
  const wanted = value.toLowerCase();
  const found = COOKIE_BROWSER_OPTIONS.find(([id]) => id === wanted);
  return found === undefined ? "" : found[0];
};

/*
 * `settings_store`'s two unfinished sentences, word for word.
 *
 * They are hints and never toasts: the backend holds only the unfinished source while persisting
 * unrelated edits. The field that is missing half of a choice is the
 * only honest place to say which half. Everything the backend *refuses* — a browser outside the
 * allowlist, a relative path, a file that is not there, a folder — travels the error route the rest
 * of the app already uses, in the backend's own words.
 */
const NO_BROWSER_YET = "Choose which browser to borrow the sign-in from.";
const NO_COOKIE_FILE_YET =
  "Choose the cookies.txt file to read the sign-in from, or take it from a browser instead.";

/** The nested groups of `Settings`; `preset` is the only scalar and is owned by the preset cards. */
type Group = "video" | "audio" | "image" | "gif" | "document" | "output" | "trim" | "link";

type Patch = <G extends Group>(group: G, patch: Partial<Settings[G]>) => void;

function useSettings(): [Settings | null, Patch] {
  const settings = useStore((s) => s.settings);
  const patchSettings = useStore((s) => s.patchSettings);
  const patch: Patch = (group, fields) => {
    if (settings === null) return;
    patchSettings({ ...settings, [group]: { ...settings[group], ...fields } });
  };
  return [settings, patch];
}

export function SettingSections(): React.JSX.Element | null {
  const [settings, patch] = useSettings();
  const pickOutputDir = useStore((s) => s.pickOutputDir);
  const pickCookieFile = useStore((s) => s.pickCookieFile);
  const cookieBrowsers = useStore((s) => s.cookieBrowsers);
  const signInCheck = useStore((s) => s.signInCheck);
  const checkSignIn = useStore((s) => s.checkSignIn);
  const openSignInGuide = useStore((s) => s.openSignInGuide);
  if (settings === null) return null;

  const { video, audio, image, gif, document: doc, output, trim, link } = settings;
  // `Option<PathBuf>` arrives as null, and a blank string is the same "nothing chosen" — both are
  // read once here so the row and the hint below cannot disagree about which state this is.
  const cookieFile = (link.cookie_file ?? "").trim();

  return (
    <div className="sections">
      <Section title="Video" defaultOpen>
        <SelectField
          label="Codec"
          value={video.codec}
          options={VIDEO_CODECS}
          onChange={(codec) => patch("video", { codec })}
        />
        <SelectField
          label="Quality"
          value={video.quality}
          options={QUALITY}
          onChange={(quality) => patch("video", { quality })}
        />
        <NumberField
          label="Max height"
          value={video.max_height}
          unit="px"
          min={144}
          max={4320}
          step={2}
          placeholder="Original"
          onChange={(max_height) => patch("video", { max_height })}
        />
        <NumberField
          label="Frame rate cap"
          value={video.fps_cap}
          unit="fps"
          min={1}
          max={240}
          float
          placeholder="Original"
          onChange={(fps_cap) => patch("video", { fps_cap })}
        />
        <NumberField
          label="Bitrate"
          value={video.bitrate_kbps}
          unit="kbps"
          min={100}
          max={200000}
          step={100}
          placeholder="Automatic"
          onChange={(bitrate_kbps) => patch("video", { bitrate_kbps })}
        />
        <SelectField
          label="Hardware encoding"
          value={video.hardware_accel}
          options={HW}
          onChange={(hardware_accel) => patch("video", { hardware_accel })}
        />
        <CheckField
          label="Web fast start (stream before download finishes)"
          checked={video.faststart}
          onChange={(faststart) => patch("video", { faststart })}
        />
        <CheckField
          label="Strip metadata"
          checked={video.strip_metadata}
          onChange={(strip_metadata) => patch("video", { strip_metadata })}
        />
      </Section>

      <Section title="Audio">
        <SelectField
          label="Codec"
          value={audio.codec}
          options={AUDIO_CODECS}
          onChange={(codec) => patch("audio", { codec })}
        />
        <NumberField
          label="Bitrate"
          value={audio.bitrate_kbps}
          unit="kbps"
          min={32}
          max={512}
          step={16}
          onChange={(value) => patch("audio", { bitrate_kbps: value ?? 192 })}
        />
        <NumberField
          label="Sample rate"
          value={audio.sample_rate}
          unit="Hz"
          min={8000}
          max={192000}
          step={1000}
          placeholder="Original"
          onChange={(sample_rate) => patch("audio", { sample_rate })}
        />
        <NumberField
          label="Channels"
          value={audio.channels}
          min={1}
          max={8}
          placeholder="Original"
          onChange={(channels) => patch("audio", { channels })}
        />
        <CheckField
          label="Normalise loudness (-16 LUFS)"
          checked={audio.normalize_loudness}
          onChange={(normalize_loudness) => patch("audio", { normalize_loudness })}
        />
      </Section>

      <Section title="Image & GIF">
        <SliderField
          label="Image quality"
          value={image.quality}
          min={1}
          max={100}
          onChange={(quality) => patch("image", { quality })}
        />
        <NumberField
          label="Max dimension"
          value={image.max_dimension}
          unit="px"
          min={64}
          max={16000}
          step={10}
          placeholder="Original"
          onChange={(max_dimension) => patch("image", { max_dimension })}
        />
        <Field label="Flatten background" hint="used when dropping transparency">
          <input
            type="color"
            className="control control--color"
            value={image.flatten_background}
            onChange={(e) => patch("image", { flatten_background: e.currentTarget.value })}
          />
        </Field>
        <NumberField
          label="Frames per second when extracting stills"
          value={image.frame_extract_fps}
          unit="fps"
          min={0.1}
          max={60}
          step={0.1}
          float
          onChange={(value) => patch("image", { frame_extract_fps: value ?? 1 })}
        />
        <CheckField
          label="Lossless where supported (WebP / AVIF)"
          checked={image.lossless}
          onChange={(lossless) => patch("image", { lossless })}
        />
        <CheckField
          label="Strip metadata"
          checked={image.strip_metadata}
          onChange={(strip_metadata) => patch("image", { strip_metadata })}
        />
        <NumberField
          label="GIF frame rate"
          value={gif.fps}
          unit="fps"
          min={1}
          max={50}
          float
          onChange={(value) => patch("gif", { fps: value ?? 12 })}
        />
        <NumberField
          label="GIF width"
          value={gif.width}
          unit="px"
          min={64}
          max={1920}
          step={10}
          onChange={(value) => patch("gif", { width: value ?? 480 })}
        />
        <NumberField
          label="GIF loops"
          value={gif.loop_count}
          min={0}
          max={100}
          hint="0 loops forever"
          onChange={(value) => patch("gif", { loop_count: value ?? 0 })}
        />
        <CheckField
          label="Optimise GIF palette"
          checked={gif.optimize_palette}
          onChange={(optimize_palette) => patch("gif", { optimize_palette })}
        />
      </Section>

      <Section title="Documents">
        <NumberField
          label="Rasterise at"
          value={doc.raster_dpi}
          unit="dpi"
          min={36}
          max={600}
          step={6}
          onChange={(value) => patch("document", { raster_dpi: value ?? 150 })}
        />
        <CheckField
          label="First page only"
          checked={doc.first_page_only}
          onChange={(first_page_only) => patch("document", { first_page_only })}
        />
      </Section>

      <Section title="Links">
        <SelectField
          label="Sign-in for pasted links"
          value={link.cookies}
          options={COOKIE_SOURCES}
          onChange={(cookies) => patch("link", { cookies })}
        />
        <BrowserField
          value={browserOption(link.cookie_browser)}
          browsers={cookieBrowsers}
          hint={
            link.cookies === "browser" && link.cookie_browser.trim() === "" ? NO_BROWSER_YET : ""
          }
          disabled={link.cookies !== "browser"}
          onChange={(cookie_browser) => patch("link", { cookie_browser })}
        />
        <Field
          label="Cookies file"
          hint={link.cookies === "file" && cookieFile === "" ? NO_COOKIE_FILE_YET : ""}
        >
          <span className="filepick">
            {/* The path, and only ever the path: what is inside the file is a sign-in. */}
            <span className="filepick__path" title={cookieFile}>
              {cookieFile === "" ? "No file chosen" : cookieFile}
            </span>
            <button
              type="button"
              className="button"
              disabled={link.cookies !== "file"}
              onClick={() => void pickCookieFile()}
            >
              Choose…
            </button>
          </span>
        </Field>
        {/* Not a `Field`: the two controls here answer a question about the three fields above
            rather than being a fourth setting, and giving them a label in the same column would
            read as one. The question is the one the old red row could not answer — "did what I
            just changed help?" — and one bounded probe of a public video answers it in place, with
            no conversion to run and nothing about the account coming back. */}
        <div className="signincheck">
          <button
            type="button"
            className="button"
            disabled={signInCheck?.checking === true}
            onClick={() => void checkSignIn()}
          >
            {signInCheck?.checking === true ? "Checking…" : "Check sign-in"}
          </button>
          <button type="button" className="microlink" onClick={openSignInGuide}>
            Walk me through it…
          </button>
        </div>
        {signInCheck !== null && (
          <p
            className="signin__verdict"
            data-result={signInCheck.result ?? "pending"}
            role="status"
          >
            {signInCheck.message}
          </p>
        )}
        <p className="section__note">
          Some videos only hand the media over to someone who is signed in — age-restricted,
          members-only, Bilibili's higher resolutions — so this borrows the sign-in a browser already
          has instead of asking you for a password.
        </p>
      </Section>

      <Section title="Trim">
        <CheckField
          label="Trim every clip to the same length"
          checked={trim.enabled}
          onChange={(enabled) => patch("trim", { enabled })}
        />
        <SecondsField
          label="Start at"
          hint="seconds, or m:ss"
          value={trim.start_secs}
          disabled={!trim.enabled}
          onChange={(start_secs) => patch("trim", { start_secs })}
        />
        <SecondsField
          label="Keep"
          hint="seconds, or m:ss"
          value={trim.length_secs}
          disabled={!trim.enabled}
          onChange={(length_secs) => patch("trim", { length_secs })}
        />
        <p className="section__note">
          Applies to every video and audio file in the queue, pasted links included. Anything shorter
          than that keeps its own length; images, documents and subtitles are untouched.
        </p>
      </Section>

      <Section title="Output">
        <SelectField
          label="Save converted files"
          value={output.location}
          options={LOCATIONS}
          onChange={(location) => patch("output", { location })}
        />
        {output.location === "subfolder" && (
          <Field label="Subfolder name">
            <input
              type="text"
              className="control control--text"
              value={output.subfolder_name}
              onChange={(e) => patch("output", { subfolder_name: e.currentTarget.value })}
            />
          </Field>
        )}
        {output.location === "custom" && (
          <Field label="Folder">
            <span className="folderpick">
              <span className="folderpick__path" title={output.custom_dir ?? ""}>
                {output.custom_dir ?? "No folder chosen"}
              </span>
              <button type="button" className="button" onClick={() => void pickOutputDir()}>
                Choose…
              </button>
            </span>
          </Field>
        )}
        <SelectField
          label="If a file already exists"
          value={output.on_conflict}
          options={CONFLICTS}
          onChange={(on_conflict) => patch("output", { on_conflict })}
        />
        <NumberField
          label="Parallel conversions"
          value={output.parallel_jobs === 0 ? null : output.parallel_jobs}
          min={1}
          max={16}
          placeholder="Automatic"
          onChange={(value) => patch("output", { parallel_jobs: value ?? 0 })}
        />
        <CheckField
          label="Keep original date & time"
          checked={output.preserve_timestamps}
          onChange={(preserve_timestamps) => patch("output", { preserve_timestamps })}
        />
      </Section>
    </div>
  );
}
