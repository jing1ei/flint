//! Helper binary discovery.
//!
//! FFmpeg/ffprobe are shipped as Tauri *sidecars* inside the app bundle, so they are always found.
//! Everything else is optional: we look for it, and the UI greys out the handful of formats that
//! need a helper the user does not have (with a one-line install hint instead of a cryptic error).

use crate::format::Tool;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Executable names to try, in order of preference.
fn binary_names(tool: Tool) -> &'static [&'static str] {
    match tool {
        Tool::Ffmpeg => &["ffmpeg"],
        Tool::Ffprobe => &["ffprobe"],
        Tool::LibreOffice => &["soffice", "libreoffice"],
        Tool::Pandoc => &["pandoc"],
        // ImageMagick 6 installs `convert` instead of `magick` - but on Windows `convert.exe` is
        // Microsoft's *filesystem* converter (FAT -> NTFS), which sits in System32 and is on
        // everybody's PATH. Running that with image arguments is not a risk worth taking.
        #[cfg(not(windows))]
        Tool::Magick => &["magick", "convert"],
        #[cfg(windows)]
        Tool::Magick => &["magick"],
        // Poppler ships `pdftoppm`, `pdftotext` and `pdftohtml` in one package, but each is looked
        // for on its own: a package that only *usually* installs all three is not a guarantee, and
        // planning a `pdftotext` run because `pdftoppm` was found would fail at spawn time.
        Tool::PdfToPpm => &["pdftoppm"],
        Tool::PdfToText => &["pdftotext"],
        Tool::PdfToHtml => &["pdftohtml"],
        Tool::Ruffle => &["ruffle_exporter", "ruffle"],
        Tool::YtDlp => &["yt-dlp"],
        Tool::Deno => &["deno"],
        Tool::Node => &["node"],
        Tool::Sips => &["sips"],
    }
}

/// The user's own `~/Applications`, where an app installed without an administrator password
/// lands (and where a drag-and-drop install from a `.dmg` often goes). `None` when there is no
/// `HOME` to expand - a launchd context, or Windows.
fn user_applications() -> Option<PathBuf> {
    user_dir("Applications")
}

/// A directory inside the user's home, or `None` when there is no `HOME` to expand.
fn user_dir(inside: &str) -> Option<PathBuf> {
    Some(home_dir()?.join(inside))
}

/// The user's home directory, or `None` when there is no `HOME` to expand (a launchd context).
///
/// The one place `HOME` is read, so that a browser's application bundle and a browser's cookie
/// store can never be looked for under two different homes.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        dirs::home_dir()
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("HOME")?;
        if home.is_empty() {
            return None;
        }
        Some(PathBuf::from(home))
    }
}

/// Every place an application bundle of this name could be installed, in priority order.
///
/// `inside` is either a bundle name (`Google Chrome.app`) or a path within one
/// (`LibreOffice.app/Contents/MacOS/soffice`) - the two callers want different depths of the same
/// answer, and joining a subpath is what makes one table serve both.
///
/// `/Applications` first, then the user's own `~/Applications`, because that is where an
/// installation that could not ask for an administrator password ends up. Safari needs no third
/// entry: it is not installable and not removable, and macOS keeps it at `/Applications/Safari.app`
/// (measured on macOS 26.6.2, where `/System/Applications/Safari.app` does not exist).
pub fn application_bundles(inside: &str) -> Vec<PathBuf> {
    let mut found = vec![PathBuf::from("/Applications").join(inside)];
    found.extend(user_applications().map(|d| d.join(inside)));
    found
}

/// Absolute locations that are not on `PATH` when an app is launched from Finder / Dock.
/// (A GUI app on macOS inherits a minimal `PATH`, which is why so many converters "work in the
/// terminal but not in the app" - we look these up explicitly.)
///
/// Every Homebrew path appears twice: `/opt/homebrew` is Apple Silicon and `/usr/local` is Intel,
/// and half the Macs this app runs on are the second kind. Every app bundle appears twice too,
/// because `~/Applications` is where an installation that could not ask for an administrator
/// password ends up (see [`application_bundles`]).
fn well_known_paths(tool: Tool) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut paths = Vec::new();
        if tool == Tool::LibreOffice {
            for base in ["ProgramFiles", "ProgramFiles(x86)"].iter().filter_map(std::env::var_os) {
                paths.push(PathBuf::from(base).join("LibreOffice/program/soffice.exe"));
            }
        }
        for base in windows_search_dirs() {
            for name in binary_names(tool) {
                paths.push(base.join(format!("{name}.exe")));
            }
        }
        paths
    }
    #[cfg(not(windows))]
    {
        let mut paths = match tool {
            Tool::LibreOffice => application_bundles("LibreOffice.app/Contents/MacOS/soffice"),
            Tool::Ruffle => application_bundles("Ruffle.app/Contents/MacOS/ruffle"),
            // Deno's own installer (`curl -fsSL https://deno.land/install.sh | sh`) puts it here, on no
            // `PATH` a Finder-launched app will ever see. A user who has Deno already must not be told
            // to install a second copy.
            Tool::Deno => user_dir(".deno/bin/deno").into_iter().collect(),
            _ => Vec::new(),
        };
        let extra: &[&str] = match tool {
            Tool::LibreOffice => &[
                "/opt/homebrew/bin/soffice",
                "/usr/local/bin/soffice",
                "/usr/bin/soffice",
                "C:\\Program Files\\LibreOffice\\program\\soffice.exe",
            ],
            Tool::Ruffle => &["/opt/homebrew/bin/ruffle", "/usr/local/bin/ruffle"],
            // Both Homebrew prefixes, spelled out for the same reason as everything else here: the two
            // `bin` directories are searched anyway, and naming them keeps this table the answer to
            // "where does the app look" even if the search list is ever trimmed.
            Tool::Deno => &["/opt/homebrew/bin/deno", "/usr/local/bin/deno"],
            Tool::Node => &["/opt/homebrew/bin/node", "/usr/local/bin/node"],
            Tool::Sips => &["/usr/bin/sips"],
            _ => &[],
        };
        paths.extend(extra.iter().map(PathBuf::from));
        paths
    }
}

#[cfg(windows)]
fn windows_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home_dir() {
        dirs.extend([".deno/bin", "scoop/shims"].iter().map(|part| home.join(part)));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        dirs.push(local.join("Microsoft/WinGet/Links"));
        dirs.push(local.join("Pandoc"));
    }
    if let Some(programs) = std::env::var_os("ProgramFiles") {
        let programs = PathBuf::from(programs);
        dirs.push(programs.join("nodejs"));
        dirs.push(programs.join("Pandoc"));
        dirs.push(programs.join("LibreOffice/program"));
    }
    dirs
}

/// The absolute directories every helper is looked for in, before `PATH` is consulted at all.
///
/// A separate constant because two callers need it: discovery, and [`child_path`] - the `PATH` we
/// hand a helper we spawn, which must contain the same Homebrew directories this process was
/// launched without.
#[cfg(windows)]
pub const FIXED_SEARCH_DIRS: &[&str] = &[];
#[cfg(not(windows))]
pub const FIXED_SEARCH_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
    "/opt/local/bin",
    // ImageMagick 7 is keg-only under both Homebrew prefixes, so its `bin` is on neither of
    // the two above: Apple Silicon first, then the Intel copy of the same keg.
    "/opt/homebrew/opt/imagemagick/bin",
    "/usr/local/opt/imagemagick/bin",
];

/// Directories searched for every tool, in priority order.
fn default_search_dirs(sidecar_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = sidecar_dir {
        dirs.push(d.to_path_buf());
    }
    dirs.extend(FIXED_SEARCH_DIRS.iter().map(PathBuf::from));
    #[cfg(windows)]
    dirs.extend(windows_search_dirs());
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs
}

/// A `PATH` for a helper *we* spawn: `first`, then the directories discovery knows about, then
/// whatever this process inherited.
///
/// Every binary this app runs is named by absolute path, so nothing depends on this - it is a
/// backstop for what a helper shells out to behind our back. yt-dlp is the case that earned it: it
/// starts an ffmpeg of its own and looks for a JavaScript runtime on `PATH`, and a Finder-launched
/// app inherits `/usr/bin:/bin:/usr/sbin:/sbin`, where neither Homebrew nor a runtime lives. The
/// inherited `PATH` is kept last rather than dropped: a machine with something in an unusual place
/// keeps working, and it can no longer *decide* anything, because our own directories come first.
pub fn child_path(first: &[PathBuf]) -> std::ffi::OsString {
    let inherited: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    let mut dirs: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    let first = [first, &windows_search_dirs()].concat();
    for dir in
        first.iter().cloned().chain(FIXED_SEARCH_DIRS.iter().map(PathBuf::from)).chain(inherited)
    {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    // `join_paths` only fails on a directory containing the separator itself, which cannot come
    // from the table above; an inherited one that odd is not worth failing a fetch over.
    std::env::join_paths(&dirs).unwrap_or_else(|_| {
        std::env::join_paths(FIXED_SEARCH_DIRS.iter().map(PathBuf::from))
            .expect("the fixed directories contain no path separator")
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatus {
    pub id: &'static str,
    /// The *diagnostic* name: "Poppler (pdftotext)", the exact executable this row is about. It is
    /// what makes "two of Poppler's three binaries are here" answerable at all, and for that reason
    /// it is not a label to put in front of a user - a settings row names the package
    /// ([`crate::install::PackageInstallPlan::name`]), which is joined to these rows by `id`.
    pub label: &'static str,
    pub bundled: bool,
    pub available: bool,
    pub path: Option<PathBuf>,
    pub install_hint: &'static str,
}

/// A helper's `major.minor`, for the rare case where the command line depends on it.
///
/// Patch level is dropped on purpose: no flag has ever appeared or vanished in one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ToolVersion {
    pub major: u32,
    pub minor: u32,
}

impl ToolVersion {
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Is this at least `major.minor`? Reads better than an `Ord` comparison at the call site.
    pub fn at_least(self, major: u32, minor: u32) -> bool {
        (self.major, self.minor) >= (major, minor)
    }
}

/// Tools whose version we look up at discovery time, because a flag spelling depends on it.
///
/// Pandoc only: `--embed-resources` does not exist before 2.19, and its predecessor
/// `--self-contained` is deprecated from 2.19 on. Nothing else earns the cost of an extra process.
const VERSIONED_TOOLS: &[Tool] = &[Tool::Pandoc];

#[derive(Debug, Clone, Default)]
pub struct ToolRegistry {
    resolved: HashMap<Tool, PathBuf>,
    versions: HashMap<Tool, ToolVersion>,
}

impl ToolRegistry {
    /// Probe the real filesystem.
    pub fn discover(sidecar_dir: Option<&Path>) -> Self {
        let dirs = default_search_dirs(sidecar_dir);
        let mut registry = Self::discover_with(&dirs, &|p: &Path| is_executable(p));
        // Discovery is the one place allowed to run a helper: `--version` is cheap, harmless and
        // the only way to know which spelling of a flag the installed copy understands.
        for tool in VERSIONED_TOOLS.iter().copied() {
            if let Some(path) = registry.path(tool).map(Path::to_path_buf) {
                if let Some(v) = ask_version(&path) {
                    registry.set_version(tool, v);
                }
            }
        }
        registry
    }

    /// Same as [`Self::discover`] but with an injectable filesystem probe (used by tests).
    pub fn discover_with(dirs: &[PathBuf], probe: &dyn Fn(&Path) -> bool) -> Self {
        let mut resolved = HashMap::new();
        for tool in ALL_TOOLS.iter().copied() {
            if let Some(found) = resolve(tool, dirs, probe) {
                resolved.insert(tool, found);
            }
        }
        Self { resolved, versions: HashMap::new() }
    }

    /// Register a tool explicitly (e.g. a Tauri sidecar path, or a user override).
    ///
    /// Deliberately does *not* run the binary: callers do this while holding locks and during
    /// startup. A tool registered this way has an unknown version, which every caller of
    /// [`Self::version`] has to handle anyway.
    pub fn set(&mut self, tool: Tool, path: PathBuf) {
        self.resolved.insert(tool, path);
        self.versions.remove(&tool);
    }

    /// Record a version without running anything (used by [`Self::discover`] and by tests).
    pub fn set_version(&mut self, tool: Tool, version: ToolVersion) {
        self.versions.insert(tool, version);
    }

    /// The installed version, if discovery managed to ask for it.
    pub fn version(&self, tool: Tool) -> Option<ToolVersion> {
        self.versions.get(&tool).copied()
    }

    pub fn path(&self, tool: Tool) -> Option<&Path> {
        self.resolved.get(&tool).map(|p| p.as_path())
    }

    pub fn has(&self, tool: Tool) -> bool {
        self.resolved.contains_key(&tool)
    }

    /// First usable tool from a preference list - this is how the planner picks a decoder.
    pub fn first_available(&self, tools: &[Tool]) -> Option<Tool> {
        tools.iter().copied().find(|t| self.has(*t))
    }

    pub fn statuses(&self) -> Vec<ToolStatus> {
        ALL_TOOLS
            .iter()
            .map(|t| ToolStatus {
                id: t.id(),
                label: t.label(),
                bundled: t.is_bundled(),
                available: self.has(*t),
                path: self.resolved.get(t).cloned(),
                install_hint: t.install_hint(),
            })
            .collect()
    }
}

pub const ALL_TOOLS: &[Tool] = &[
    Tool::Ffmpeg,
    Tool::Ffprobe,
    Tool::LibreOffice,
    Tool::Pandoc,
    Tool::Magick,
    Tool::PdfToPpm,
    Tool::PdfToText,
    Tool::PdfToHtml,
    Tool::Ruffle,
    Tool::YtDlp,
    // The JavaScript runtimes sit next to yt-dlp because that is the only thing that uses them:
    // they convert nothing, and a fetch that has none of them is the failure they exist to prevent.
    Tool::Deno,
    Tool::Node,
    Tool::Sips,
];

fn resolve(tool: Tool, dirs: &[PathBuf], probe: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    for name in binary_names(tool) {
        for dir in dirs {
            for candidate in [dir.join(name), dir.join(format!("{name}.exe"))] {
                if probe(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    well_known_paths(tool).into_iter().find(|p| probe(p))
}

/// Run `<tool> --version` and read the number off the first line.
///
/// Failures are all one answer - `None`, "unknown version" - because there is nothing useful to
/// distinguish: a helper that cannot be executed will fail again when the conversion runs, and the
/// only caller has a spelling that works whatever the version turns out to be.
fn ask_version(path: &Path) -> Option<ToolVersion> {
    use std::sync::{atomic::AtomicBool, Arc};
    let mut first = None;
    let outcome = crate::engine::run_probe(
        path,
        &["--version".into()],
        None,
        &Arc::new(AtomicBool::new(false)),
        std::time::Duration::from_secs(3),
        &mut |line| {
            if first.is_none() {
                first = Some(line.to_string());
            }
        },
    )
    .ok()?;
    if !matches!(outcome, crate::engine::ProcOutcome::Ok) {
        return None;
    }
    parse_version(first.as_deref()?)
}

/// First `<digits>.<digits>` on the first line of a `--version` banner.
///
/// Pandoc prints `pandoc 3.8.1`, older builds print `pandoc.exe 2.17.1.1`, and both are followed by
/// paragraphs of build detail that must not be searched for numbers.
fn parse_version(text: &str) -> Option<ToolVersion> {
    let line = text.lines().next()?;
    let mut rest = line;
    while let Some(at) = rest.find(|c: char| c.is_ascii_digit()) {
        let tail = &rest[at..];
        let digits = |s: &str| s.len() - s.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let major_len = digits(tail);
        let after = &tail[major_len..];
        if let Some(minor_part) = after.strip_prefix('.') {
            let minor_len = digits(minor_part);
            if minor_len > 0 {
                return Some(ToolVersion {
                    major: tail[..major_len].parse().ok()?,
                    minor: minor_part[..minor_len].parse().ok()?,
                });
            }
        }
        rest = after;
    }
    None
}

/// Is this an executable file? Shared with [`crate::install`], which probes for a package manager
/// the same way discovery probes for a helper.
#[cfg(unix)]
pub(crate) fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub(crate) fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_hung_version_probe_cannot_block_startup_forever() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("cc-version-{}", std::process::id()));
        std::fs::write(&path, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(ask_version(&path), None);
        assert!(started.elapsed() < std::time::Duration::from_secs(8));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn sidecar_dir_wins_over_system_ffmpeg() {
        let dirs = vec![PathBuf::from("/app/sidecar"), PathBuf::from("/usr/bin")];
        let reg = ToolRegistry::discover_with(&dirs, &|p| {
            matches!(p.to_str(), Some("/app/sidecar/ffmpeg") | Some("/usr/bin/ffmpeg"))
        });
        assert_eq!(reg.path(Tool::Ffmpeg).unwrap(), Path::new("/app/sidecar/ffmpeg"));
        assert!(!reg.has(Tool::Pandoc));
    }

    #[test]
    fn finds_libreoffice_in_applications_even_without_path() {
        let reg = ToolRegistry::discover_with(&[], &|p| {
            p == Path::new("/Applications/LibreOffice.app/Contents/MacOS/soffice")
        });
        assert!(reg.has(Tool::LibreOffice));
        assert_eq!(
            reg.first_available(&[Tool::Pandoc, Tool::LibreOffice]),
            Some(Tool::LibreOffice)
        );
    }

    /// Half the Macs this app runs on are Intel, where Homebrew lives under `/usr/local` rather
    /// than `/opt/homebrew`, and a user without an administrator password installs an app into
    /// their own `~/Applications`. Both used to be invisible to discovery, so the app said
    /// "LibreOffice is not installed" next to a LibreOffice the user had just installed.
    #[test]
    fn helpers_are_found_on_intel_macs_and_in_a_users_own_applications_folder() {
        for (tool, path) in
            [(Tool::LibreOffice, "/usr/local/bin/soffice"), (Tool::Ruffle, "/usr/local/bin/ruffle")]
        {
            let reg = ToolRegistry::discover_with(&[], &|p| p == Path::new(path));
            assert_eq!(reg.path(tool).map(Path::to_path_buf), Some(PathBuf::from(path)));
        }
        // Apple Silicon must keep working, and it is still the first place we look.
        let reg = ToolRegistry::discover_with(&[], &|p| {
            matches!(p.to_str(), Some("/opt/homebrew/bin/soffice") | Some("/usr/local/bin/soffice"))
        });
        assert_eq!(reg.path(Tool::LibreOffice).unwrap(), Path::new("/opt/homebrew/bin/soffice"));

        // An app bundle in the user's own folder, which is on no `PATH` at all.
        let home = std::env::var_os("HOME").expect("a HOME to expand");
        for (tool, inside) in [
            (Tool::LibreOffice, "LibreOffice.app/Contents/MacOS/soffice"),
            (Tool::Ruffle, "Ruffle.app/Contents/MacOS/ruffle"),
        ] {
            let app = PathBuf::from(&home).join("Applications").join(inside);
            let reg = ToolRegistry::discover_with(&[], &|p| p == app);
            assert!(reg.has(tool), "{} was not looked for", app.display());
        }

        // Keg-only ImageMagick ships its binaries under the prefix, not in `bin` - on both.
        let dirs = default_search_dirs(None);
        for keg in ["/opt/homebrew/opt/imagemagick/bin", "/usr/local/opt/imagemagick/bin"] {
            assert!(dirs.contains(&PathBuf::from(keg)), "{keg} is not searched");
        }
    }

    #[test]
    fn statuses_cover_every_tool() {
        let reg = ToolRegistry::default();
        let s = reg.statuses();
        assert_eq!(s.len(), ALL_TOOLS.len());
        assert!(s.iter().all(|t| !t.available));
    }

    /// The whole point of finding a JavaScript runtime ourselves: yt-dlp looks for one on `PATH`
    /// only, and a Finder-launched app has none of these directories in its `PATH`. Each location
    /// is a real install: Homebrew on Apple Silicon, Homebrew on Intel, and Deno's own
    /// `curl … | sh` installer, which drops it in the user's home.
    #[test]
    fn a_javascript_runtime_is_found_in_every_place_it_is_normally_installed() {
        let home = std::env::var_os("HOME").expect("a HOME to expand");
        let deno_home = PathBuf::from(&home).join(".deno/bin/deno");
        let places = [
            PathBuf::from("/opt/homebrew/bin/deno"),
            PathBuf::from("/usr/local/bin/deno"),
            deno_home.clone(),
        ];
        for place in &places {
            // No search directories at all: the well-known paths have to carry this on their own.
            let reg = ToolRegistry::discover_with(&[], &|p| p == place.as_path());
            assert_eq!(
                reg.path(Tool::Deno).map(Path::to_path_buf),
                Some(place.clone()),
                "{} was not looked for",
                place.display()
            );
            assert!(!reg.has(Tool::Node), "only the runtime that exists may be reported");
        }
        // Apple Silicon wins on a migrated machine that has both.
        let both = ToolRegistry::discover_with(&[], &|p| {
            matches!(p.to_str(), Some("/opt/homebrew/bin/deno") | Some("/usr/local/bin/deno"))
        });
        assert_eq!(both.path(Tool::Deno).unwrap(), Path::new("/opt/homebrew/bin/deno"));

        // Node is looked for in the two Homebrew prefixes as well, and yt-dlp prefers Deno when
        // both are here - which is the order `first_available` reads off `JS_RUNTIMES`.
        let node = ToolRegistry::discover_with(&[], &|p| p == Path::new("/opt/homebrew/bin/node"));
        assert_eq!(node.path(Tool::Node).unwrap(), Path::new("/opt/homebrew/bin/node"));
        assert_eq!(node.first_available(crate::format::JS_RUNTIMES), Some(Tool::Node));
        let all = ToolRegistry::discover_with(&[], &|p| {
            matches!(p.to_str(), Some("/opt/homebrew/bin/deno") | Some("/opt/homebrew/bin/node"))
        });
        assert_eq!(all.first_available(crate::format::JS_RUNTIMES), Some(Tool::Deno));
        assert_eq!(ToolRegistry::default().first_available(crate::format::JS_RUNTIMES), None);
    }

    /// The `PATH` a helper we spawn gets. Our own directories come first so nothing about a
    /// conversion depends on the user's shell, and the inherited `PATH` is still there behind them.
    #[test]
    fn a_spawned_helper_gets_a_path_with_homebrew_in_it() {
        let runtime_dir = PathBuf::from("/opt/homebrew/opt/deno/bin");
        let path = child_path(std::slice::from_ref(&runtime_dir));
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert_eq!(dirs.first(), Some(&runtime_dir), "the runtime's own directory comes first");
        for expected in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
            assert!(dirs.contains(&PathBuf::from(expected)), "{expected} is missing from {path:?}");
        }
        // No duplicates, however much the inherited `PATH` overlaps with our own list.
        let mut deduped = dirs.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(deduped.len(), dirs.len(), "{path:?}");
        // A runtime that already lives in one of our directories must not be listed twice.
        let twice = child_path(&[PathBuf::from("/opt/homebrew/bin")]);
        let dirs: Vec<PathBuf> = std::env::split_paths(&twice).collect();
        assert_eq!(dirs.iter().filter(|d| *d == Path::new("/opt/homebrew/bin")).count(), 1);
        // ...and with nothing to put first it is still a usable `PATH`.
        assert!(std::env::split_paths(&child_path(&[])).any(|d| d == Path::new("/usr/bin")));
    }

    #[test]
    fn windows_never_mistakes_the_filesystem_converter_for_imagemagick() {
        let names = binary_names(Tool::Magick);
        assert_eq!(names.contains(&"convert"), cfg!(not(windows)));
        assert!(names.contains(&"magick"));
    }

    /// Real banners, copied from the binaries this audit ran: 2.17.1.1 (Debian 12), 2.19.2 and
    /// 3.8.1 (upstream tarballs). Only the first line may be searched - the paragraphs that follow
    /// are full of numbers (pandoc-types 1.22.2.1, texmath 0.12.4, ...).
    #[test]
    fn version_banners_are_read_off_the_first_line_only() {
        let cases = [
            (
                "pandoc 2.17.1.1\nCompiled with pandoc-types 1.22.2.1, texmath 0.12.4\n",
                Some((2, 17)),
            ),
            ("pandoc 2.19.2\nFeatures: +server +lua\n", Some((2, 19))),
            ("pandoc 3.8.1\nFeatures: +server +lua\n", Some((3, 8))),
            ("pandoc.exe 2.9\n", Some((2, 9))),
            ("LibreOffice 7.6.4.1 60(Build:1)\n", Some((7, 6))),
            ("", None),
            ("pandoc\nversion 3.1\n", None), // the number is not on the first line
        ];
        for (banner, want) in cases {
            let got = parse_version(banner).map(|v| (v.major, v.minor));
            assert_eq!(got, want, "{banner:?}");
        }
    }

    /// A version is knowledge about *this* binary, so pointing the registry at a different one has
    /// to forget it - otherwise a sidecar override inherits the flags of whatever was on PATH.
    #[test]
    fn re_registering_a_tool_forgets_its_version() {
        let mut reg = ToolRegistry::default();
        reg.set(Tool::Pandoc, PathBuf::from("/usr/bin/pandoc"));
        reg.set_version(Tool::Pandoc, ToolVersion::new(3, 8));
        assert!(reg.version(Tool::Pandoc).unwrap().at_least(2, 19));
        reg.set(Tool::Pandoc, PathBuf::from("/opt/ancient/pandoc"));
        assert_eq!(reg.version(Tool::Pandoc), None);
    }
}
