//! Locating the bundled FFmpeg/ffprobe sidecars.
//!
//! Tauri ships `externalBin` entries under two different names depending on how the app runs:
//!
//! * **Packaged** (`.app` / `.exe`): the binary is copied next to the main executable with the
//!   target-triple suffix stripped, i.e. `Flint.app/Contents/MacOS/ffmpeg`.
//! * **`cargo tauri dev`**: the CLI copies `src-tauri/binaries/ffmpeg-<triple>` next to the debug
//!   executable, but developers who ran `scripts/fetch-sidecars.sh` also have the *suffixed*
//!   originals sitting in `src-tauri/binaries/`.
//!
//! So we look for both spellings in every plausible directory and only then fall back to whatever
//! FFmpeg is on `PATH` (handled by [`convert_core::ToolRegistry::discover`]). Not finding a sidecar
//! is *not* fatal: the UI renders a "FFmpeg missing" banner built from the `ToolStatus` list.

use std::path::{Path, PathBuf};

/// Target triple this binary was compiled for, injected by `build.rs`.
pub const TARGET_TRIPLE: &str = env!("BUILD_TARGET_TRIPLE");

/// Result of the sidecar hunt.
#[derive(Debug, Clone, Default)]
pub struct Sidecars {
    /// Directory the sidecars were found in, passed to `ToolRegistry::discover` so that helper
    /// lookups prefer bundled binaries over anything installed system-wide.
    pub dir: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
}

impl Sidecars {
    /// True when we can convert media at all without relying on a system FFmpeg.
    pub fn is_complete(&self) -> bool {
        self.ffmpeg.is_some() && self.ffprobe.is_some()
    }
}

/// Search every directory an app bundle or a dev build could keep sidecars in.
pub fn locate() -> Sidecars {
    let mut found = Sidecars::default();
    for dir in candidate_dirs() {
        if found.ffmpeg.is_none() {
            found.ffmpeg = executable_in(&dir, "ffmpeg");
        }
        if found.ffprobe.is_none() {
            found.ffprobe = executable_in(&dir, "ffprobe");
        }
        if found.dir.is_none() && (found.ffmpeg.is_some() || found.ffprobe.is_some()) {
            found.dir = Some(dir);
        }
        if found.is_complete() {
            break;
        }
    }
    found
}

/// Directories to inspect, most specific first, without repeats.
fn candidate_dirs() -> Vec<PathBuf> {
    search_dirs(
        std::env::current_exe().ok().as_deref(),
        std::env::current_dir().ok().as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
        cfg!(debug_assertions),
    )
}

fn search_dirs(
    exe: Option<&Path>,
    cwd: Option<&Path>,
    manifest: &Path,
    development: bool,
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    // `Vec::dedup` would only collapse *adjacent* repeats, and the duplicates here are not
    // adjacent: running the dev binary from the crate root makes `current_exe`'s directory and
    // `current_dir()/src-tauri/binaries` collide from opposite ends of the list.
    let mut seen = std::collections::HashSet::new();
    let mut push = |dir: PathBuf, dirs: &mut Vec<PathBuf>| {
        if seen.insert(dir.clone()) {
            dirs.push(dir);
        }
    };

    if let Some(exe) = exe {
        if let Some(exe_dir) = exe.parent() {
            // Packaged: Tauri copies sidecars next to the executable, suffix stripped.
            push(exe_dir.to_path_buf(), &mut dirs);
            // macOS bundle layout: MacOS/<exe> and Resources/ are siblings. Tauri puts sidecars in
            // MacOS/, but resources declared by hand end up in Resources/ - check both.
            if let Some(contents) = exe_dir.parent() {
                push(contents.join("Resources"), &mut dirs);
            }
            push(exe_dir.join("binaries"), &mut dirs);
        }
    }

    // Source-tree and working-directory lookups are development conveniences only.
    // Release builds do not implicitly trust source or working-directory paths.
    if development {
        push(manifest.join("binaries"), &mut dirs);
        if let Some(cwd) = cwd {
            push(cwd.join("src-tauri").join("binaries"), &mut dirs);
            push(cwd.join("binaries"), &mut dirs);
        }
    }

    dirs
}

/// Look for `name`, `name-<triple>` (and the Windows `.exe` spellings) inside `dir`.
fn executable_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let candidates = [
        format!("{name}-{TARGET_TRIPLE}"),
        name.to_string(),
        format!("{name}-{TARGET_TRIPLE}.exe"),
        format!("{name}.exe"),
    ];
    candidates.iter().map(|c| dir.join(c)).find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch_exe(path: &Path) {
        std::fs::write(path, b"#!/bin/sh\nexit 0\n").expect("write fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod fixture");
        }
    }

    #[test]
    fn prefers_the_triple_suffixed_binary_used_by_tauri_dev() {
        let dir = std::env::temp_dir().join(format!("cc-sidecar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        touch_exe(&dir.join("ffmpeg"));
        touch_exe(&dir.join(format!("ffmpeg-{TARGET_TRIPLE}")));

        assert_eq!(
            executable_in(&dir, "ffmpeg"),
            Some(dir.join(format!("ffmpeg-{TARGET_TRIPLE}")))
        );
        assert_eq!(executable_in(&dir, "ffprobe"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_search_does_not_use_build_or_launch_directories() {
        let exe = Path::new("/Applications/Flint.app/Contents/MacOS/flint");
        let dirs = search_dirs(
            Some(exe),
            Some(Path::new("/untrusted")),
            Path::new("/build/src-tauri"),
            false,
        );
        assert!(dirs.iter().all(|p| p.starts_with("/Applications/Flint.app/Contents")));
        assert!(!dirs.iter().any(|p| p.starts_with("/untrusted") || p.starts_with("/build")));
        let dev = search_dirs(
            Some(exe),
            Some(Path::new("/workspace")),
            Path::new("/build/src-tauri"),
            true,
        );
        assert!(dev.contains(&PathBuf::from("/build/src-tauri/binaries")));
        assert!(dev.contains(&PathBuf::from("/workspace/binaries")));
    }

    #[test]
    fn a_missing_directory_is_not_an_error() {
        assert_eq!(executable_in(Path::new("/definitely/not/here"), "ffmpeg"), None);
    }
}
