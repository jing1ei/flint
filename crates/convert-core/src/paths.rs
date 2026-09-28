//! Where does the converted file go, and what is it called?
//!
//! Pure path arithmetic with an injectable "does this exist" probe, because silently overwriting
//! someone's original is unforgivable in a batch tool.

use crate::format::Format;
use crate::plan::output_extension;
use crate::settings::{ConflictPolicy, OutputLocation, OutputSettings};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Destination folder for a converted file. Only ever asked as part of [`output_path`].
fn output_dir(input: &Path, settings: &OutputSettings) -> PathBuf {
    let parent = input.parent().unwrap_or(Path::new(".")).to_path_buf();
    match settings.location {
        OutputLocation::SameFolder => parent,
        OutputLocation::Subfolder => parent.join(&settings.subfolder_name),
        OutputLocation::Custom => settings.custom_dir.clone().unwrap_or(parent),
    }
}

/// Ideal output path, before conflict resolution.
pub fn output_path(input: &Path, target: &Format, settings: &OutputSettings) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".into());
    output_dir(input, settings).join(format!("{stem}.{}", output_extension(target)))
}

/// Where a *link's* output goes.
///
/// "Alongside the source" has no meaning for a URL - there is no source folder, only a scratch
/// directory that is deleted the moment the job ends - so the file location settings cannot be
/// applied as they are. The rule is therefore one sentence, and the UI states it: the custom
/// folder if the user has chosen one, and `~/Downloads` if they have not.
///
/// Note that a custom folder counts even when [`OutputLocation`] is something else: it is the only
/// folder the user has ever named, and dropping a video into an unrelated `~/Downloads` after they
/// pointed the app at `~/Media` would be a surprise.
pub fn link_output_dir(settings: &OutputSettings) -> PathBuf {
    match settings.custom_dir.as_ref().filter(|d| !d.as_os_str().is_empty()) {
        Some(dir) => dir.clone(),
        None => downloads_dir(),
    }
}

/// `~/Downloads`, or the temp directory on a machine with no home (a launchd context, a test).
fn downloads_dir() -> PathBuf {
    #[cfg(windows)]
    if let Some(dir) = dirs::download_dir() {
        return dir;
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty());
    match home {
        Some(home) => PathBuf::from(home).join("Downloads"),
        None => std::env::temp_dir(),
    }
}

/// Ideal output path for a link, from an already-sanitised stem (see [`crate::link::sanitize_title`]).
pub fn link_output_path(stem: &str, target: &Format, settings: &OutputSettings) -> PathBuf {
    link_output_dir(settings).join(format!("{stem}.{}", output_extension(target)))
}

/// Apply the conflict policy. `None` means "skip this file".
pub fn resolve_conflict(
    desired: &Path,
    policy: ConflictPolicy,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if !exists(desired) {
        return Some(desired.to_path_buf());
    }
    match policy {
        ConflictPolicy::Overwrite => Some(desired.to_path_buf()),
        ConflictPolicy::Skip => None,
        ConflictPolicy::Rename => {
            let stem =
                desired.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let ext =
                desired.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let dir = desired.parent().unwrap_or(Path::new("."));
            for n in 1..10_000 {
                let candidate = if ext.is_empty() {
                    dir.join(format!("{stem} ({n})"))
                } else {
                    dir.join(format!("{stem} ({n}).{ext}"))
                };
                if !exists(&candidate) {
                    return Some(candidate);
                }
            }
            None
        }
    }
}

/// Pick an output path and reserve it for the rest of the batch. `None` means "skip this file".
///
/// [`resolve_conflict`] can only ask the filesystem, and the filesystem knows nothing about the job
/// running on the next thread: two sources in one folder (`clip.mov` and `clip.avi`, or
/// `photo.heic` and `photo.png` into one Desktop folder) both resolved to the same output, the
/// second silently overwrote the first, and both rows reported success.
///
/// A name another job in this batch has taken is treated as taken even under
/// [`ConflictPolicy::Overwrite`]: that policy is the user's answer to "may I replace files that
/// were already here", not permission for two of their own conversions to fight over one name.
pub fn claim_output(
    desired: &Path,
    policy: ConflictPolicy,
    claimed: &mut HashSet<PathBuf>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let effective = if policy == ConflictPolicy::Overwrite && claimed.contains(&claim_key(desired))
    {
        ConflictPolicy::Rename
    } else {
        policy
    };
    let taken = |p: &Path| exists(p) || claimed.contains(&claim_key(p));
    let chosen = resolve_conflict(desired, effective, &taken)?;
    claimed.insert(claim_key(&chosen));
    Some(chosen)
}

/// Case-folded key for the claim set.
///
/// `A.PNG` and `a.png` are one file on macOS and Windows, so a batch holding both used to hand two
/// workers the same destination: the second overwrote the first and both rows reported success.
/// Folding costs a needless ` (1)` on a case-sensitive volume, which is the better failure.
fn claim_key(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    // Resolve the nearest existing parent, including when Converted/ does not
    // exist yet. A symlinked folder must not give a second job the same output.
    let resolved = absolute
        .parent()
        .and_then(|parent| {
            parent.ancestors().find_map(|ancestor| {
                let canonical = dunce::canonicalize(ancestor).ok()?;
                Some(canonical.join(absolute.strip_prefix(ancestor).ok()?))
            })
        })
        .unwrap_or(absolute);
    PathBuf::from(resolved.as_os_str().to_string_lossy().to_lowercase())
}

/// Whether a file belongs to a reserved numbered output family.
pub(crate) fn sequence_contains(family: &Path, file: &Path) -> bool {
    let family = claim_key(family);
    let file = claim_key(file);
    family.parent() == file.parent()
        && sequence_index(
            &file.file_name().unwrap_or_default().to_string_lossy(),
            &family.file_stem().unwrap_or_default().to_string_lossy(),
            &family.extension().unwrap_or_default().to_string_lossy(),
        )
        .is_some()
}

/// `<prefix>-<number>.<extension>` -> the number, for exactly the names a rasteriser or FFmpeg's
/// image2 muxer produces.
///
/// The strict shape matters: a plain `starts_with(prefix)` also matches `clip-old.png` (a stale
/// file) and `clip2-0001.png` (a *different* job writing into the same folder). Lives here rather
/// than in the engine because both ends of the app need the same answer - the engine to know which
/// files a step produced, and [`sequence_family_exists`] to know which files a step is about to
/// overwrite.
pub(crate) fn sequence_index(name: &str, prefix: &str, extension: &str) -> Option<u64> {
    let stem = name.strip_suffix(extension)?.strip_suffix('.')?;
    if prefix.is_empty() {
        // Flash frames land in a private temp directory under whatever name Ruffle picked, so
        // there is no prefix to check and nothing unrelated can be in there.
        return Some(0);
    }
    let digits = stem.strip_prefix(prefix)?.strip_prefix('-')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Is there already a `<stem>-<number>.<ext>` family where a frame sequence is about to be written?
///
/// A job that writes `clip-0001.png`, `clip-0002.png`, ... only ever claimed `clip.png` - a name it
/// never creates. So the conflict policy was never consulted about the files that actually get
/// written, and a second extraction into the same folder overwrote the first run's frames whatever
/// the user had chosen. Passed to [`claim_output`] as part of its `exists` probe, which turns the
/// existing family into an ordinary conflict: skipped, renamed to `clip (1)-0001.png`, or
/// overwritten because that is what the user asked for.
pub(crate) fn sequence_family_exists(path: &Path) -> bool {
    let Some(dir) = path.parent() else { return false };
    let stem = match path.file_stem() {
        Some(s) => s.to_string_lossy().to_lowercase(),
        None => return false,
    };
    if stem.is_empty() {
        return false;
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        sequence_index(&e.file_name().to_string_lossy().to_lowercase(), &stem, &ext).is_some()
    })
}

/// Guard against `clip.mp4 -> clip.mp4` clobbering the input when the user keeps the same format
/// in the same folder.
///
/// Canonicalising first catches the cases a plain `==` misses - `./clip.mp4`, a symlinked
/// folder, a drop from a path with a trailing separator. The output usually does not exist yet
/// (canonicalise fails), so its parent is canonicalised instead and the file name compared.
pub fn is_same_file(input: &Path, output: &Path) -> bool {
    if input == output {
        return true;
    }
    // Names alone cannot see that `VIDEO.MP4` and `VIDEO.mp4` are one file on a case-insensitive
    // volume, nor that the destination is a link back to the source. Both only show up once the
    // destination exists - which is exactly the "overwrite in the source folder" case where
    // getting this wrong feeds FFmpeg its own output and destroys the original.
    if same_file_on_disk(input, output) {
        return true;
    }
    let real = |p: &Path| -> Option<PathBuf> {
        let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        Some(dir.canonicalize().ok()?.join(p.file_name()?))
    };
    match (real(input), real(output)) {
        (Some(a), Some(b)) => a == b || same_file_on_disk(&a, &b),
        _ => false,
    }
}

/// Does the filesystem itself say these are the same file? Inode identity is the only answer that
/// survives case-insensitive volumes, symlinks and hard links.
fn same_file_on_disk(a: &Path, b: &Path) -> bool {
    same_file::is_same_file(a, b).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::by_id;
    use crate::settings::OutputSettings;

    fn no_files(_: &Path) -> bool {
        false
    }

    #[test]
    fn defaults_write_into_a_converted_subfolder() {
        let s = OutputSettings::default();
        let p = output_path(Path::new("/Users/me/Movies/clip.mov"), by_id("mp4").unwrap(), &s);
        assert_eq!(p, PathBuf::from("/Users/me/Movies/Converted/clip.mp4"));
    }

    #[test]
    fn same_folder_and_custom_folder_modes() {
        let mut s = OutputSettings { location: OutputLocation::SameFolder, ..Default::default() };
        assert_eq!(
            output_path(Path::new("/a/b/clip.mov"), by_id("mp3").unwrap(), &s),
            PathBuf::from("/a/b/clip.mp3")
        );
        s.location = OutputLocation::Custom;
        s.custom_dir = Some(PathBuf::from("/Users/me/Desktop"));
        assert_eq!(
            output_path(Path::new("/a/b/clip.mov"), by_id("mp3").unwrap(), &s),
            PathBuf::from("/Users/me/Desktop/clip.mp3")
        );
    }

    /// A URL has no folder to sit "alongside", so the rule is the custom folder or `~/Downloads` -
    /// and it has to hold whatever the user's *file* location setting happens to be, because that
    /// setting cannot be honoured for a link at all.
    #[test]
    fn a_links_output_goes_to_the_custom_folder_or_downloads() {
        let mp3 = by_id("mp3").unwrap();
        for location in
            [OutputLocation::SameFolder, OutputLocation::Subfolder, OutputLocation::Custom]
        {
            let chosen = OutputSettings {
                location,
                custom_dir: Some(PathBuf::from("/Users/me/Media")),
                ..Default::default()
            };
            assert_eq!(link_output_dir(&chosen), PathBuf::from("/Users/me/Media"), "{location:?}");
            assert_eq!(
                link_output_path("A Talk", mp3, &chosen),
                PathBuf::from("/Users/me/Media/A Talk.mp3")
            );

            // Nothing chosen (including the "custom" mode with an empty box): ~/Downloads.
            for custom_dir in [None, Some(PathBuf::new())] {
                let bare = OutputSettings { location, custom_dir, ..Default::default() };
                let dir = link_output_dir(&bare);
                assert!(dir.ends_with("Downloads"), "{location:?} -> {}", dir.display());
                if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
                    assert_eq!(dir, PathBuf::from(home).join("Downloads"));
                }
            }
        }
    }

    #[test]
    fn container_aliases_are_respected() {
        let s = OutputSettings { location: OutputLocation::SameFolder, ..Default::default() };
        assert_eq!(
            output_path(Path::new("/a/song.wav"), by_id("alac").unwrap(), &s),
            PathBuf::from("/a/song.m4a")
        );
    }

    #[test]
    fn names_with_dots_and_spaces_survive() {
        let s = OutputSettings { location: OutputLocation::SameFolder, ..Default::default() };
        assert_eq!(
            output_path(Path::new("/a/My Holiday v1.2.final.mov"), by_id("mp4").unwrap(), &s),
            PathBuf::from("/a/My Holiday v1.2.final.mp4")
        );
    }

    #[test]
    fn rename_policy_never_overwrites() {
        let taken = |p: &Path| matches!(p.to_str(), Some("/a/clip.mp4") | Some("/a/clip (1).mp4"));
        assert_eq!(
            resolve_conflict(Path::new("/a/clip.mp4"), ConflictPolicy::Rename, &taken),
            Some(PathBuf::from("/a/clip (2).mp4"))
        );
    }

    #[test]
    fn overwrite_and_skip_policies() {
        let taken = |_: &Path| true;
        assert_eq!(
            resolve_conflict(Path::new("/a/clip.mp4"), ConflictPolicy::Overwrite, &taken),
            Some(PathBuf::from("/a/clip.mp4"))
        );
        assert_eq!(resolve_conflict(Path::new("/a/clip.mp4"), ConflictPolicy::Skip, &taken), None);
    }

    #[test]
    fn a_free_path_is_used_as_is() {
        assert_eq!(
            resolve_conflict(Path::new("/a/clip.mp4"), ConflictPolicy::Rename, &no_files),
            Some(PathBuf::from("/a/clip.mp4"))
        );
    }

    #[test]
    fn two_jobs_in_one_batch_never_claim_the_same_name() {
        let mut claimed = HashSet::new();
        let first = claim_output(
            Path::new("/a/Converted/clip.mp4"),
            ConflictPolicy::Rename,
            &mut claimed,
            &no_files,
        );
        assert_eq!(first, Some(PathBuf::from("/a/Converted/clip.mp4")));
        // clip.avi arrives while clip.mov is still encoding: nothing is on disk yet, but the name
        // is spoken for.
        let second = claim_output(
            Path::new("/a/Converted/clip.mp4"),
            ConflictPolicy::Rename,
            &mut claimed,
            &no_files,
        );
        assert_eq!(second, Some(PathBuf::from("/a/Converted/clip (1).mp4")));
        assert_eq!(claimed.len(), 2);
    }

    #[test]
    #[cfg(unix)]
    fn folder_aliases_share_claims_before_and_after_output_folder_creation() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        let alias = dir.path().join("alias");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let first = real.join("Converted/clip.png");
        let second = alias.join("Converted/clip.png");
        let mut claimed = HashSet::new();
        assert_eq!(
            claim_output(&first, ConflictPolicy::Rename, &mut claimed, &no_files),
            Some(first)
        );
        std::fs::create_dir(real.join("Converted")).unwrap();
        assert_eq!(
            claim_output(&second, ConflictPolicy::Overwrite, &mut claimed, &no_files),
            Some(alias.join("Converted/clip (1).png"))
        );
        assert!(sequence_contains(&second, &real.join("Converted/CLIP-0001.PNG")));
        assert!(!sequence_contains(&second, &real.join("Converted/clip-+1.png")));
    }

    #[test]
    fn overwrite_replaces_an_old_file_but_not_a_sibling_job() {
        let mut claimed = HashSet::new();
        let on_disk = |p: &Path| p == Path::new("/a/clip.mp4");
        // The file was already there and the user said overwrite: use it.
        assert_eq!(
            claim_output(
                Path::new("/a/clip.mp4"),
                ConflictPolicy::Overwrite,
                &mut claimed,
                &on_disk
            ),
            Some(PathBuf::from("/a/clip.mp4"))
        );
        // The next job wants the same name; overwriting now would destroy the first job's result.
        assert_eq!(
            claim_output(
                Path::new("/a/clip.mp4"),
                ConflictPolicy::Overwrite,
                &mut claimed,
                &on_disk
            ),
            Some(PathBuf::from("/a/clip (1).mp4"))
        );
    }

    #[test]
    fn skip_policy_still_skips_a_name_a_sibling_job_took() {
        let mut claimed = HashSet::new();
        assert!(claim_output(
            Path::new("/a/clip.mp4"),
            ConflictPolicy::Skip,
            &mut claimed,
            &no_files
        )
        .is_some());
        assert_eq!(
            claim_output(Path::new("/a/clip.mp4"), ConflictPolicy::Skip, &mut claimed, &no_files),
            None
        );
    }

    #[test]
    fn detects_in_place_collisions() {
        assert!(is_same_file(Path::new("/a/clip.mp4"), Path::new("/a/clip.mp4")));
        assert!(!is_same_file(Path::new("/a/clip.mp4"), Path::new("/a/Converted/clip.mp4")));
    }

    /// macOS and Windows hand back the *same file* for names that differ only in case, so a batch
    /// holding `A.png` and `a.png` (or `photo.JPG` and `photo.jpg` converted to the same target)
    /// aimed two workers at one destination: the second silently overwrote the first.
    #[test]
    fn two_names_that_differ_only_in_case_are_one_destination() {
        let mut claimed = HashSet::new();
        assert_eq!(
            claim_output(Path::new("/a/A.jpg"), ConflictPolicy::Rename, &mut claimed, &no_files),
            Some(PathBuf::from("/a/A.jpg"))
        );
        assert_eq!(
            claim_output(Path::new("/a/a.jpg"), ConflictPolicy::Rename, &mut claimed, &no_files),
            Some(PathBuf::from("/a/a (1).jpg"))
        );
        // ...and Overwrite must not be read as permission to overwrite a sibling job either.
        assert_eq!(
            claim_output(Path::new("/a/A.JPG"), ConflictPolicy::Overwrite, &mut claimed, &no_files),
            Some(PathBuf::from("/a/A (2).JPG"))
        );
    }

    /// Two questions, one rule: "has another job claimed this name?" and - on the platforms with no
    /// inode to ask - "is this output the file we are reading?" are both "same name, different
    /// case", and both go through [`claim_key`]. The Windows arm of `same_file_on_disk` used to
    /// case-fold on its own, where it could quietly stop agreeing with the claim set.
    #[test]
    fn one_name_in_two_cases_is_one_key_wherever_the_question_is_asked() {
        assert_eq!(claim_key(Path::new("/a/VIDEO.MP4")), claim_key(Path::new("/a/video.mp4")));
        assert_ne!(claim_key(Path::new("/a/video.mp4")), claim_key(Path::new("/a/video2.mp4")));
        // Name reservation is case-insensitive, but identity requires actual files.
        assert!(!same_file_on_disk(
            Path::new("/definitely/absent/VIDEO.MP4"),
            Path::new("/definitely/absent/video.mp4")
        ));
    }

    /// The family check has to be as strict as the collector that reads those names back, or a
    /// folder holding `clip-notes.png` would make every frame extraction rename itself forever.
    #[test]
    fn an_existing_frame_family_is_seen_but_only_the_real_thing() {
        let dir = std::env::temp_dir().join(format!("cc-paths-family-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("clip.png");
        assert!(!sequence_family_exists(&target), "an empty folder is not a conflict");

        for decoy in ["clip-notes.png", "clip2-0001.png", "clip-0001.jpg", "clip.png"] {
            std::fs::write(dir.join(decoy), b"x").unwrap();
        }
        assert!(!sequence_family_exists(&target), "none of these is `clip-<number>.png`");

        std::fs::write(dir.join("clip-0007.png"), b"x").unwrap();
        assert!(sequence_family_exists(&target), "a real earlier frame must be seen");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The in-place guard compared *names*, so it missed every way a filesystem can hand back one
    /// file under two names: a case-insensitive volume (`VIDEO.MP4` / `VIDEO.mp4` on stock macOS
    /// and Windows), a symlink, a hard link. With "same folder" + "overwrite" that fed FFmpeg its
    /// own input as the output file and destroyed the original.
    #[test]
    fn a_destination_that_is_really_the_source_is_detected() {
        let dir = std::env::temp_dir().join(format!("cc-paths-same-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("clip.mov");
        std::fs::write(&input, b"not really a movie").unwrap();

        // A hard link stands in for the case-insensitive volume: same inode, different name.
        // (The literal `CLIP.MOV` cannot be created *on* such a volume - it is already there.)
        let other_name = dir.join("clip-again.mov");
        std::fs::hard_link(&input, &other_name).unwrap();
        assert!(is_same_file(&input, &other_name), "overwriting this destroys the source");

        #[cfg(unix)]
        {
            let link = dir.join("shortcut.mov");
            std::os::unix::fs::symlink(&input, &link).unwrap();
            assert!(is_same_file(&input, &link), "the symlink points at the source");
        }

        // A real, separate destination is still just a destination.
        let elsewhere = dir.join("Converted").join("clip.mov");
        assert!(!is_same_file(&input, &elsewhere));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
