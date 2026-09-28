//! The IPC surface.
//!
//! Every command returns `Result<T, String>`: Tauri turns `Err` into a rejected promise, and a
//! plain string is the only error shape the UI ever needs (it renders it verbatim in a toast).
//!
//! Command arguments use `rename_all = "snake_case"`, i.e. the JavaScript side invokes them with
//! exactly the field names written in this file (`{ preset_id }`, `{ target_id }`), matching the
//! payload structs below instead of Tauri's default camelCase remapping. Multi-word arguments are
//! wrapped in [`crate::ipc::Arg`], so a camelCase caller (`{ presetId }`) works just as well.

use crate::ipc::Arg;
use crate::{install, launch_services, settings_store, Activity, AppState, BatchSlot, BATCH_EVENT};
use convert_core::format::{catalog, Category, Format, Support};
use convert_core::install::{install_plans, resolve_install, PackageInstallPlan};
use convert_core::link::{self, CookieCheck, Link, SafariCookieAccess};
use convert_core::paths::{link_output_dir, output_path, resolve_conflict};
use convert_core::plan::{default_target_for, suggested_targets_for};
use convert_core::queue::{worker_count, EventSink};
use convert_core::settings::{
    cookie_browser_presence, BrowserPresence, CookieFlag, CookieSource, RealDisk,
};
use convert_core::{
    by_id, catalog_view, run_batch, BatchEvent, BatchItem, CatalogView, Engine, MediaInfo, Preset,
    Settings, ToolRegistry, ToolStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

/// Deepest directory nesting we walk when a folder is dropped. Deep enough for any real media
/// library, shallow enough that a pathological tree cannot hang the drop handler.
///
/// It is also the second half of the loop protection: symlinked *directories* are never followed
/// (see [`collect`]), and anything that still manages to nest - a bind mount pointing at its own
/// ancestor, a filesystem that resolves links behind our back - runs out of depth here instead of
/// spinning forever.
const MAX_WALK_DEPTH: usize = 16;

/// Most files one drop may put in the queue.
///
/// `inspect_files` used to enumerate whatever it was handed: dropping a home folder walked every
/// directory on the disk, probed thousands of media files and built a row for each - minutes of
/// unresponsive window and a `Vec<FileInfo>` the webview then had to render. 5000 is far more than
/// any real batch and small enough to stay instant, and the count is reported back
/// ([`Inspection::limit`]) so the UI can name the number rather than guess it.
const MAX_INSPECTED_FILES: usize = 5000;
const MAX_WALK_ENTRIES: usize = 20_000;

// ---------------------------------------------------------------------------------------------
// Catalog & settings
// ---------------------------------------------------------------------------------------------

/// Everything the UI needs to render pickers, presets and the "missing helper" banner.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_catalog(state: State<'_, AppState>) -> Result<CatalogView, String> {
    Ok(catalog_view(&state.engine().tools))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_settings<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<Settings, String> {
    Ok(state.settings(&app))
}

/// Persist what the settings drawer just changed.
///
/// Validation is **per field**, not per object. Refusing the whole payload over one unfinished field
/// lost real work: the destination is half-chosen for as long as it takes to pick a folder, and every
/// codec, quality and conflict-policy edit made in that window was silently dropped. So each field is
/// judged on its own, and the destination has three answers rather than two - usable (stored),
/// unfinished (held, everything else stored, nothing said), hostile (repaired, everything else
/// stored, and the reason returned). See [`settings_store::merge`].
///
/// A hostile destination is still refused rather than obeyed *and* still reported rather than quietly
/// repaired: the window keeps showing what the user typed, so saying nothing would make the settings
/// page lie about where files will land.
#[tauri::command(rename_all = "snake_case")]
pub async fn save_settings<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings, String> {
    state.store_settings_from_ui(&app, settings)
}

/// Replace the whole settings object with a preset. Returns the new settings so the advanced panel
/// can re-render from a single source of truth instead of guessing what the preset changed.
///
/// Everything except the trim and the link's cookie source, which are carried across. A preset is a
/// *quality* choice - "1080p
/// H.264, MP3 192k" - and it has no opinion about which ten seconds of the clip the user wants; a
/// batch set up to keep 10 seconds from 0:30 that silently reverted to the whole film because the
/// user tried "Smallest file" would be the picker throwing away an unrelated choice. The core's
/// `Preset::settings()` stays a plain function of the preset (trimming off, like every default) and
/// the shell, which is the only layer that knows the *current* settings, is what remembers.
#[tauri::command(rename_all = "snake_case")]
pub async fn apply_preset<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    preset_id: Arg<String>,
) -> Result<Settings, String> {
    let preset_id = preset_id.0;
    let preset = Preset::ALL
        .iter()
        .copied()
        .find(|p| p.id() == preset_id)
        .ok_or_else(|| format!("Unknown preset `{}`", printable(&preset_id)))?;
    let mut next = preset.settings();
    let current = state.settings(&app);
    next.trim = current.trim;
    // The cookie source is carried across for the same reason as the trim, and it bites harder: a
    // preset click that forgot which browser to borrow the sign-in from would turn the next
    // members-only link back into a failure the user had already been to Settings to fix.
    next.link = current.link;
    state.store_settings(&app, next)
}

// ---------------------------------------------------------------------------------------------
// File inspection
// ---------------------------------------------------------------------------------------------

/// One row in the file list.
#[derive(Debug, Clone, Serialize)]
pub struct FileInfo {
    pub id: String,
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub supported: bool,
    pub category: Option<String>,
    pub format_id: Option<String>,
    pub format_name: Option<String>,
    pub default_target: Option<String>,
    pub suggested_targets: Vec<String>,
    pub duration_secs: Option<f64>,
    pub duration_label: Option<String>,
    pub resolution_label: Option<String>,
    pub is_animated: bool,
    pub note: Option<String>,
}

/// What one drop produced: the rows, and whether the cap cut the enumeration short.
///
/// The two extra fields are the whole point of capping - a queue that silently stops at 5000 files
/// is indistinguishable from a queue that found 5000 files, and the user would go looking for the
/// rest of their folder. `limit` travels with the flag so the banner can name the number instead of
/// hard-coding a copy of it that drifts.
///
/// Field names are exactly as written here (no `rename_all`): `files`, `truncated`, `limit`.
#[derive(Debug, Clone, Serialize)]
pub struct Inspection {
    pub files: Vec<FileInfo>,
    /// True only when something was genuinely left out, never merely because the count landed on
    /// the cap.
    pub truncated: bool,
    /// The cap that was applied, for the message the UI shows.
    pub limit: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Expand folders, drop duplicates, identify formats and probe media - all in one round trip so a
/// drag & drop of 200 files costs the frontend a single `invoke`.
///
/// Bounded at [`MAX_INSPECTED_FILES`]: dropping a folder is one gesture, and it must not be able to
/// spend minutes walking a disk or fill memory with rows nothing will ever convert. The walk *stops*
/// at the cap rather than collecting everything and trimming afterwards, which is what keeps the
/// answer quick for the pathological case rather than only correct.
///
/// The whole job (directory walk + `ffprobe` fan-out) runs on a blocking thread: it is filesystem
/// and process work, and parking an async runtime worker for a second would delay every other
/// command - including `cancel_batch`.
#[tauri::command(rename_all = "snake_case")]
pub async fn inspect_files(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<Inspection, String> {
    let engine = state.engine();
    tauri::async_runtime::spawn_blocking(move || {
        let walk = expand_paths(&paths, MAX_INSPECTED_FILES);
        let infos = probe_in_parallel(&engine, &walk.files);
        let files = walk
            .files
            .iter()
            .zip(infos)
            .map(|(path, media)| describe(path, media, &engine.tools))
            .collect();
        Inspection { files, truncated: walk.truncated, limit: walk.limit, warnings: walk.warnings }
    })
    .await
    .map_err(|e| format!("could not inspect the dropped files: {e}"))
}

/// A directory walk in progress, and its budget.
struct Walk {
    /// The queue being filled, in walk order.
    files: Vec<PathBuf>,
    /// Canonical paths already in `files`, so a file dropped twice - or reached through a symlink
    /// and directly - is one row. Its size is also what proves the walk *stopped* at the cap
    /// instead of enumerating a whole disk and truncating the result.
    seen: HashSet<PathBuf>,
    /// Most files this walk may collect.
    limit: usize,
    /// Set when the budget made us skip something that would otherwise have been walked or queued.
    truncated: bool,
    scanned: usize,
    warnings: Vec<String>,
}

impl Walk {
    fn new(limit: usize) -> Self {
        Self {
            files: Vec::new(),
            seen: HashSet::new(),
            limit,
            truncated: false,
            scanned: 0,
            warnings: Vec::new(),
        }
    }

    fn warn(&mut self, message: String) {
        if self.warnings.len() < 8 && !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    fn is_full(&self) -> bool {
        self.files.len() >= self.limit
    }
}

/// Recursively expand directories, skipping dotfiles and de-duplicating, up to `limit` files.
///
/// A path the user dropped explicitly is always kept (even if hidden); hidden *entries discovered
/// by walking* are skipped, which is what keeps `.git`, `.DS_Store` and Spotlight caches out of the
/// queue. Symlinked *directories* are not followed, so a self-referencing link cannot loop forever;
/// symlinked *files* are kept, because a library of links to the real media is a normal way to
/// organise footage and dropping the folder used to yield an empty queue. Nesting is bounded by
/// [`MAX_WALK_DEPTH`] and the total by `limit`.
fn expand_paths(paths: &[String], limit: usize) -> Walk {
    let mut walk = Walk::new(limit);
    for raw in paths {
        collect(Path::new(raw), 0, &mut walk);
        if walk.truncated {
            break;
        }
    }
    walk
}

fn collect(path: &Path, depth: usize, walk: &mut Walk) {
    // Checked on entry, before this path is stat'ed or read: the walk ends the moment the budget is
    // gone, so a folder with a million files costs the same as one with 5000 plus one directory
    // read. Reaching here at all means the drop had more to offer than the cap allows, so the UI is
    // told the list is short - conservatively, since the entry we are turning away here might have
    // turned out to be a duplicate or an empty folder. Finding that out would mean walking on,
    // which is the thing being avoided.
    if walk.is_full() {
        walk.truncated = true;
        return;
    }
    if depth > MAX_WALK_DEPTH {
        walk.warn(format!("Folder nesting is too deep. Add {} directly.", path.display()));
        return;
    }
    if depth > 0 && is_hidden(path) {
        return;
    }
    if path.is_dir() {
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) => {
                walk.warn(format!(
                    "Could not read {}: {error}. Check folder permissions and add it again.",
                    path.display()
                ));
                return;
            }
        };
        let mut children = Vec::new();
        for entry in entries {
            if walk.scanned >= MAX_WALK_ENTRIES {
                walk.warn("Folder scan limit reached. Add smaller folders or select the remaining files directly.".into());
                break;
            }
            walk.scanned += 1;
            match entry {
                Ok(entry) => match entry.file_type() {
                    Ok(kind) if kind.is_symlink() && entry.path().is_dir() => {}
                    Ok(_) => children.push(entry.path()),
                    Err(error) => walk.warn(format!(
                        "Could not inspect {}: {error}. Add it again after checking permissions.",
                        entry.path().display()
                    )),
                },
                Err(error) => walk.warn(format!(
                    "Could not read an entry in {}: {error}. Check permissions and add it again.",
                    path.display()
                )),
            }
        }
        children.sort();
        for child in children {
            collect(&child, depth + 1, walk);
            if walk.truncated {
                break;
            }
        }
        return;
    }
    // Non-existent paths are kept on purpose: `describe` turns them into a visible "File not
    // found" row instead of silently swallowing a file the user believes they added.
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if walk.seen.insert(key) {
        walk.files.push(path.to_path_buf());
    }
}

fn is_hidden(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with('.')).unwrap_or(false)
}

/// Probe only what is worth probing, on a small pool.
///
/// `ffprobe` is a process spawn per file (~10 ms), so 200 dropped files would stall the UI for two
/// seconds if done serially - and documents/subtitles have nothing for ffprobe to say anyway.
fn probe_in_parallel(engine: &Engine, files: &[PathBuf]) -> Vec<Option<MediaInfo>> {
    let slots: Vec<Mutex<Option<MediaInfo>>> = files.iter().map(|_| Mutex::new(None)).collect();
    let todo: Vec<usize> = files
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            convert_core::by_path(p).map(|f| is_probeable(f.category)).unwrap_or(false)
        })
        .map(|(i, _)| i)
        .collect();

    if !todo.is_empty() {
        let next = AtomicUsize::new(0);
        let workers = worker_count(0).min(todo.len());
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| loop {
                    let cursor = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&index) = todo.get(cursor) else {
                        break;
                    };
                    let info = engine.probe(&files[index]);
                    if let Ok(mut slot) = slots[index].lock() {
                        *slot = info;
                    }
                });
            }
        });
    }

    slots.into_iter().map(|s| s.into_inner().unwrap_or(None)).collect()
}

fn is_probeable(category: Category) -> bool {
    matches!(category, Category::Video | Category::Audio | Category::Image | Category::Flash)
}

fn describe(path: &Path, media: Option<MediaInfo>, tools: &ToolRegistry) -> FileInfo {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let exists = path.is_file();

    let mut info = FileInfo {
        id: next_id(),
        path: path.to_string_lossy().to_string(),
        name,
        size_bytes,
        supported: false,
        category: None,
        format_id: None,
        format_name: None,
        default_target: None,
        suggested_targets: Vec::new(),
        duration_secs: media.as_ref().and_then(|m| m.duration_secs),
        duration_label: media.as_ref().and_then(|m| m.duration_label()),
        resolution_label: media.as_ref().and_then(|m| m.resolution_label()),
        is_animated: media.as_ref().map(|m| m.is_animated).unwrap_or(false),
        note: None,
    };

    if !exists {
        info.note = Some("File not found".into());
        return info;
    }

    let Some(format) = convert_core::by_path(path) else {
        info.note = Some(format!("Unsupported file type: .{}", extension_of(path)));
        return info;
    };

    let default_target = default_target_for(format.category);
    info.category = Some(format.category.id().to_string());
    info.format_id = Some(format.id.to_string());
    info.format_name = Some(format.name.to_string());
    info.default_target = Some(default_target.to_string());
    info.suggested_targets =
        suggested_targets_for(format.category).iter().map(|t| t.to_string()).collect();

    // No "write-only format" branch here: `by_path` matches readable formats only, so a format
    // that cannot be read (SVG output, `pdf_page`) never reaches this point.
    info.supported = true;
    info.note = helper_note(format, by_id(default_target), tools);
    if format.id == "midi" && info.note.is_none() {
        info.note = Some("Renders all MIDI tracks with a basic piano sound.".into());
    }
    info
}

/// "Needs LibreOffice" style hint: the one thing standing between this file and a conversion.
///
/// Checked in pipeline order - a source we cannot decode matters more than a target we cannot
/// encode, because the user can change the target with one click but not the file they dropped.
/// Both cases share the same wording: from the user's point of view the fix is identical.
fn helper_note(
    source: &'static Format,
    target: Option<&'static Format>,
    tools: &ToolRegistry,
) -> Option<String> {
    if let Support::AnyOf(helpers) = source.read {
        if tools.first_available(helpers).is_none() {
            return Some(format!("Needs {}", join_labels(helpers)));
        }
    }
    if let Support::AnyOf(helpers) = target?.write {
        if tools.first_available(helpers).is_none() {
            return Some(format!("Needs {}", join_labels(helpers)));
        }
    }
    None
}

/// "Needs Poppler or LibreOffice": the helpers named as a user installs them, each named once.
///
/// The package name, never the executable - `pdftotext` under a dropped file would tell the reader
/// nothing, and Poppler's three binaries would say the same word three times.
fn join_labels(tools: &[convert_core::Tool]) -> String {
    let mut names: Vec<&'static str> = Vec::new();
    for name in tools.iter().map(|t| t.user_facing_name()) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.join(" or ")
}

fn extension_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "".into())
}

/// Monotonic, collision-free row id without pulling in a uuid dependency: process-start nanos give
/// uniqueness across app restarts, the counter gives it within a single run.
fn next_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    format!("f{nanos:x}-{n:x}")
}

// ---------------------------------------------------------------------------------------------
// Conversion
// ---------------------------------------------------------------------------------------------

/// One queued row as sent by the UI.
///
/// A row is *either* a dropped file (`path`) or a pasted video link (`url`) - never both, never
/// neither. The two are kept as separate fields rather than one "source" string because the
/// difference matters before anything runs: a path is opened, a URL is validated against an
/// allowlist and then downloaded, and a UI bug that sent one as the other must be a refusal rather
/// than a guess.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchItemArg {
    /// The `FileInfo::id` / `LinkRow::id` the frontend already knows - every event echoes it back.
    pub id: String,
    /// Absolute path of a dropped file. Omit (or leave empty) for a link row.
    #[serde(default)]
    pub path: Option<String>,
    /// A YouTube or Bilibili video link, exactly as `inspect_links` accepted it.
    #[serde(default)]
    pub url: Option<String>,
    /// Format id from the catalog, e.g. `mp4`. Accepts `targetId` too, for camelCase callers.
    #[serde(alias = "targetId")]
    pub target_id: String,
}

/// Start converting. Events arrive on [`BATCH_EVENT`]; this call returns as soon as the worker
/// thread is running so the UI never waits on a long batch.
#[tauri::command(rename_all = "snake_case")]
pub async fn start_batch<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    items: Vec<BatchItemArg>,
    settings: Settings,
) -> Result<(), String> {
    if items.is_empty() {
        return Err("Nothing to convert".into());
    }
    // Everything that could refuse this batch is checked *before* the slot is claimed: a refusal
    // that latched the guard would lock the app out of converting anything for the rest of the run.
    settings_store::check_output(&settings.output)?;
    // A trim with no length is the same kind of "not yet" as a destination with no folder: the
    // planner would ignore it and convert every file in full, which is not what the window says.
    settings_store::check_trim(&settings.trim)?;
    if let Some(crop) = &settings.crop {
        crop.validate()?;
    }
    // And a cookie source with no browser or no file chosen is the same "not yet" again: the fetch
    // would send no sign-in at all while the drawer says it is borrowing one, and the failure the
    // user then reads would blame the video.
    let jobs = jobs_from(items)?;
    if jobs.iter().any(|job| job.source.as_link().is_some()) {
        settings_store::check_cookies(&settings.link)?;
    }
    let engine = state.engine();
    let BatchSlot { id: batch_id, cancel } = state.begin_batch()?;

    let emitter = app.clone();
    // What the window has actually been told, so a terminal event invented after a panic can be a
    // tally instead of a guess. Counted here rather than read from `run_batch`, whose own tally
    // dies with the thread that was keeping it.
    let reported = Arc::new(Reported::default());
    let counter = reported.clone();
    let sink: EventSink = Arc::new(move |event: BatchEvent| {
        // ORDER MATTERS: the slot is freed *before* `batch_finished` is emitted, never after.
        //
        // Two things depend on it. `batch_finished` is the UI's cue that it may queue another run,
        // and releasing afterwards leaves a window in which a user clicking Convert the instant the
        // batch ends is told "already running". Worse, the frontend's adoption path (a webview
        // reloaded mid-batch: `adoptShellWork` in `src/state/store.ts`) confirms what it adopted by
        // reading `get_activity` again *after* this event - so a slot still held here reads back as
        // "still converting" and the adopted batch sticks in the UI forever, with a Stop button that
        // clears nothing.
        //
        // Pinned by `tests::the_batch_slot_is_free_at_the_instant_batch_finished_is_emitted`, which
        // fails if these two statements are ever swapped.
        if matches!(event, BatchEvent::BatchFinished { .. }) {
            release_slot(&emitter, batch_id);
        }
        counter.record(&event);
        // A failed emit means the window went away mid-batch; log and keep converting rather than
        // aborting work the user already paid for.
        if let Err(e) = emitter.emit(BATCH_EVENT, event) {
            eprintln!("[flint] could not deliver batch event: {e}");
        }
    });

    // A plain OS thread, not the async runtime: `run_batch` blocks and owns its own worker pool.
    let total = jobs.len();
    let spawned = std::thread::Builder::new().name("convert-batch".into()).spawn(move || {
        // The sink normally frees the slot on `batch_finished`; this guard is the backstop for the
        // path that has no such event - a panic unwinding out of `run_batch` itself. Releasing is
        // keyed to this batch's id, so this late second release cannot free a successor's slot.
        let _slot = SlotGuard { app, batch_id, total, reported };
        run_batch(jobs, settings, engine, cancel, sink);
    });

    if let Err(e) = spawned {
        // Never leave the single-batch guard latched if the thread could not start.
        state.end_batch(batch_id);
        return Err(format!("Could not start the conversion thread: {e}"));
    }
    Ok(())
}

/// Turn what the webview queued into jobs, refusing anything the batch would only fail on later.
///
/// The target id is checked here rather than per row: it is the one field the UI derives from the
/// catalog, so a value that is not in it means the payload was not written by this UI - and the
/// answer to that is one clear refusal, not N `failed` events carrying a crafted string back into
/// the toast that raised them.
///
/// Links get the same treatment twice over: every URL is validated (scheme, host allowlist, "is
/// this one video") *here*, before it can become an argument, and the 20-link cap is checked over
/// the whole payload. Both refusals are also enforced deeper down (`convert_core::link`,
/// `run_batch`); this is the layer that turns them into a message the user sees before the batch
/// starts instead of twenty failed rows.
fn jobs_from(items: Vec<BatchItemArg>) -> Result<Vec<BatchItem>, String> {
    let jobs: Vec<BatchItem> = items
        .into_iter()
        .map(|i| {
            if by_id(&i.target_id).is_none() {
                return Err(format!("Unknown output format `{}`", printable(&i.target_id)));
            }
            let path = i.path.filter(|p| !p.trim().is_empty());
            let url = i.url.filter(|u| !u.trim().is_empty());
            match (path, url) {
                (Some(path), None) => Ok(BatchItem::file(i.id, PathBuf::from(path), i.target_id)),
                (None, Some(url)) => {
                    let link = Link::parse(&url).map_err(|e| e.to_string())?;
                    Ok(BatchItem::link(i.id, link, i.target_id))
                }
                (Some(_), Some(_)) => {
                    Err("A queued row has both a file and a link; it must have one.".into())
                }
                (None, None) => Err("A queued row has no file and no link.".into()),
            }
        })
        .collect::<Result<_, String>>()?;

    let links = jobs.iter().filter(|j| j.source.as_link().is_some()).count();
    link::check_batch_size(links).map_err(|e| e.to_string())?;
    Ok(jobs)
}

// ---------------------------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------------------------

/// One pasted line, judged. Refused lines are rows too: a paste of twelve links with one channel
/// URL in it must show *which* line is the problem, not fail as a block.
///
/// Field names are exactly as written here (no `rename_all`).
#[derive(Debug, Clone, Serialize)]
pub struct LinkRow {
    /// Row id, in the same namespace as `FileInfo::id` - pass it back in `BatchItemArg::id`.
    pub id: String,
    /// The URL as it will be used, trimmed. Pass this back as `BatchItemArg::url`.
    pub url: String,
    /// `youtube` / `bilibili`, or `null` when the line was refused.
    pub site: Option<String>,
    /// "YouTube" / "Bilibili", for the row.
    pub site_label: Option<String>,
    pub category: Option<Category>,
    /// False when the line was refused; `note` then says why and what to do instead.
    pub supported: bool,
    /// Suggested target for a link, and the alternatives - the same lists a video file gets, so a
    /// link row's format picker is the video picker.
    pub default_target: Option<String>,
    pub suggested_targets: Vec<String>,
    /// The refusal message, or the helper hint ("Needs yt-dlp") when nothing else is wrong.
    pub note: Option<String>,
}

/// What one paste produced.
#[derive(Debug, Clone, Serialize)]
pub struct LinkInspection {
    pub links: Vec<LinkRow>,
    /// How many of them are actually convertible.
    pub accepted: usize,
    /// The cap ([`convert_core::link::MAX_LINKS_PER_BATCH`]), so the UI names the number rather
    /// than hard-coding a copy that drifts.
    pub limit: usize,
}

/// Validate a paste of video links, without touching the network.
///
/// Pure and instant: no titles, no durations, no HEAD requests. What a link *is* (title, length)
/// is asked of yt-dlp when the batch runs, because that costs a round trip per link and the paste
/// box must not stall.
///
/// The whole paste is refused when it is over the cap - twenty-one links is a mistake to correct,
/// not twenty-one rows to render.
#[tauri::command(rename_all = "snake_case")]
pub async fn inspect_links(links: Vec<String>) -> Result<LinkInspection, String> {
    let lines: Vec<String> =
        links.iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    link::check_batch_size(lines.len()).map_err(|e| e.to_string())?;

    let rows: Vec<LinkRow> = lines
        .iter()
        .map(|raw| match Link::parse(raw) {
            Ok(link) => LinkRow {
                id: next_id(),
                url: link.url().to_string(),
                site: Some(link.site().id().to_string()),
                site_label: Some(link.site().label().to_string()),
                category: Some(link.site().category()),
                supported: true,
                default_target: Some(default_target_for(link.site().category()).to_string()),
                suggested_targets: suggested_targets_for(link.site().category())
                    .iter()
                    .map(|t| t.to_string())
                    .collect(),
                note: None,
            },
            Err(e) => LinkRow {
                id: next_id(),
                url: printable(raw),
                site: None,
                site_label: None,
                category: None,
                supported: false,
                default_target: None,
                suggested_targets: Vec::new(),
                note: Some(e.to_string()),
            },
        })
        .collect();

    let accepted = rows.iter().filter(|r| r.supported).count();
    Ok(LinkInspection { links: rows, accepted, limit: link::MAX_LINKS_PER_BATCH })
}

/// The rules the UI has to state and enforce, read from the core rather than restated in TypeScript.
///
/// Field names are exactly as written here (no `rename_all`).
#[derive(Debug, Clone, Serialize)]
pub struct LinkSupport {
    /// Most links one batch may carry. The UI enforces it for a message before the click; the core
    /// enforces it again because the UI is the untrusted caller.
    pub max_links: usize,
    /// Every host that will be accepted, exact match, lower case.
    pub accepted_hosts: Vec<&'static str>,
    /// True when yt-dlp is on this machine. When false, the queue can still hold link rows but
    /// converting them fails with an install hint - so the UI should offer the installer instead.
    pub tool_installed: bool,
    /// The `package_id` to hand `install_tool`, and the row to point at in Settings → Helpers.
    pub package_id: &'static str,
    /// Where a link's output will land under `settings`: the custom folder if one is set,
    /// `~/Downloads` otherwise. "Alongside the source" cannot apply to a URL.
    pub destination: String,
}

/// Everything the link UI needs that is not per-link: the cap, the hosts, the destination, and
/// whether the helper is installed.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_link_support(
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<LinkSupport, String> {
    Ok(LinkSupport {
        max_links: link::MAX_LINKS_PER_BATCH,
        accepted_hosts: link::ACCEPTED_HOSTS.iter().map(|(host, _)| *host).collect(),
        tool_installed: state.engine().tools.path(convert_core::Tool::YtDlp).is_some(),
        package_id: convert_core::package::YT_DLP.id,
        destination: link_output_dir(&settings.output).to_string_lossy().to_string(),
    })
}

/// Which browsers this Mac actually has, which one it opens links with, and where a sign-in is
/// most likely to be found - so a recovery flow can offer one rather than list eight.
///
/// One row per name on `COOKIE_BROWSERS`, in allowlist order, whether it is installed or not: the
/// caller needs the whole list to tell "you have Chrome" from "you have nothing we can borrow
/// from", and a filtered list would make those two look the same. The `id` of any row whose
/// `installed` is true can be stored straight into `settings.link.cookie_browser`.
///
/// Each row now carries the evidence as well as the presence: `is_default` (the LaunchServices
/// rule, in which *no recorded handler* means Safari), `cookie_store_exists`,
/// `cookie_store_bytes`, `cookie_store_modified`, `needs_full_disk_access`, and the `rank` those
/// add up to. The rule this replaces was "prefer any browser that is not Safari, because Safari
/// costs a permission", and it failed a real user: their default *is* Safari, their Safari jar had
/// been written minutes earlier, and the Chrome it offered instead held a 64 KB empty database
/// that had not been touched in two days. Convenience is not evidence.
///
/// Reads no cookies and launches nothing: `stat` on a handful of application bundles and cookie
/// stores, and one small plist. Not one of those files is opened - the sizes and times come from
/// `stat`, which is the same doctrine helper discovery uses for LibreOffice and Ruffle.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_cookie_browsers() -> Result<Vec<BrowserPresence>, String> {
    Ok(cookie_browser_presence(&RealDisk, launch_services::default_browser(), SystemTime::now()))
}

/// What one `open` of Safari's cookie jar proved. Field names are exactly as written here (no
/// `rename_all`).
#[derive(Debug, Clone, Serialize)]
pub struct SafariAccess {
    /// `readable` | `needs_full_disk_access` | `no_cookie_store` | `unreadable`.
    pub result: SafariCookieAccess,
    /// True for `readable` only: the app has Full Disk Access and Safari's jar can be handed to
    /// yt-dlp. It does *not* say the site will accept what is inside.
    pub ok: bool,
    /// The sentence to show, in the words a failed row would have used for the same cause.
    pub message: String,
    /// The file that was opened, for a diagnostic. `None` only when there is no `HOME` to expand.
    pub cookie_store: Option<PathBuf>,
}

/// Answer "can Flint read Safari's cookies?" locally, now, and without yt-dlp.
///
/// The old route to this answer was a network probe: run yt-dlp against a public video with
/// `--cookies-from-browser safari`, wait up to twenty seconds, and read the permission out of its
/// stderr. But the question is not a question about YouTube. macOS keeps
/// `Cookies.binarycookies` behind Full Disk Access, so `open(2)` settles it in microseconds -
/// `EPERM` means the app does not have the permission, and nothing else on that path does.
///
/// The file is opened and dropped. No byte is read, nothing is parsed, and nothing about the
/// user's cookies - not a domain, not a count, not a name - can reach this payload, because none
/// of it is ever in memory.
#[tauri::command(rename_all = "snake_case")]
pub async fn check_safari_cookie_access() -> Result<SafariAccess, String> {
    let Some(jar) = convert_core::settings::safari_cookie_jar(&RealDisk) else {
        return Err(NO_SAFARI_COOKIE_JAR.to_string());
    };
    let access = link::safari_cookie_access(&jar);
    Ok(SafariAccess {
        result: access,
        ok: access.ok(),
        message: access.message(),
        cookie_store: Some(jar),
    })
}

/// Said when there is no home directory to find Safari's jar in - a launchd context, not a user's
/// Mac. Distinct from "the file is not there", which is a verdict rather than an error.
pub const NO_SAFARI_COOKIE_JAR: &str =
    "Flint could not work out where Safari keeps its cookies on this Mac.";

/// What one sign-in check proved. Field names are exactly as written here (no `rename_all`).
#[derive(Debug, Clone, Serialize)]
pub struct CookieTest {
    /// `working` | `unreadable` | `refused` | `inconclusive` | `not_configured`. The three the user
    /// can act on are the first three; `inconclusive` means the check learned nothing about the
    /// cookies and must not be shown as a cookie problem.
    pub result: CookieCheck,
    /// True for `working` only. The one field a "retry now" button should look at.
    pub ok: bool,
    /// The sentence to show, in the words a failed row would have used for the same cause.
    pub message: String,
    /// The public video the check was run against, so the UI can be specific about what was tried.
    pub tested_url: String,
}

/// Answer "will the sign-in you configured actually work?" before a link depends on it.
///
/// The check the old message could not offer: it named Settings → Links and left the user to find
/// out on the next batch whether anything they did there had helped. One throwaway metadata probe
/// of a public video ([`link::COOKIE_TEST_URL`]) with the configured cookie source, and one of
/// three answers - the cookies were read and the site served the video; the cookie source could
/// not be read at all; it was read and the site refused it anyway.
///
/// `settings` is the drawer's *current* state rather than what is on disk, because the point is to
/// check the choice the user just made, before saving it teaches the rest of the app to trust it.
/// It is validated by the same gate that guards a batch ([`settings_store::check_cookies`]), so a
/// half-finished source is refused here in the words the settings page already uses instead of
/// being reported as a broken sign-in.
///
/// Nothing of the jar comes back. The probe prints a title and a duration and the engine drops
/// both; what returns is a verdict and a sentence from [`link::classify_failure`], and neither
/// command line asks yt-dlp for `--verbose`, which is the only channel that prints cookie values.
#[tauri::command(rename_all = "snake_case")]
pub async fn test_cookie_source(
    state: State<'_, AppState>,
    settings: Settings,
    url: Option<String>,
) -> Result<CookieTest, String> {
    let url =
        Link::parse(url.as_deref().unwrap_or(link::COOKIE_TEST_URL)).map_err(|e| e.to_string())?;
    let tested_url = url.url().to_string();
    // Nothing configured is not a failing check: it is the honest "there is nothing to test", and
    // running a probe to discover it would be a network round trip to learn what the settings
    // already say.
    if settings.link.cookies == CookieSource::None {
        return Ok(CookieTest {
            result: CookieCheck::NotConfigured,
            ok: false,
            message: NO_COOKIE_SOURCE_TO_TEST.to_string(),
            tested_url,
        });
    }
    // A browser that is not on the allowlist, or a cookies.txt that is not there, never reaches
    // yt-dlp - here for the same reason it never reaches a batch.
    settings_store::check_cookies(&settings.link)?;
    let Some(cookies) = settings.link.effective() else {
        return Err(NO_COOKIE_SOURCE_TO_TEST.to_string());
    };
    // Safari is answered here, before anything is spawned. Its jar is behind Full Disk Access, and
    // an `open` that fails with `EPERM` has already proved what twenty seconds of yt-dlp would
    // have: the app cannot read it. Only a jar that *does* open leaves a question a probe can
    // answer - whether the site accepts the sign-in inside it - so only that case goes on.
    if cookies == CookieFlag::Browser("safari") {
        if let Some(jar) = convert_core::settings::safari_cookie_jar(&RealDisk) {
            let access = link::safari_cookie_access(&jar);
            if let Some(result) = access.verdict() {
                return Ok(CookieTest {
                    result,
                    ok: result.ok(),
                    message: access.message(),
                    tested_url,
                });
            }
        }
    }
    // Up to `COOKIE_TEST_TIMEOUT_SECS` of a blocked thread, so not on an async worker: every other
    // command - including `cancel_batch` - shares that pool.
    let engine = state.engine();
    let probe = tauri::async_runtime::spawn_blocking(move || {
        engine.check_cookie_source(
            &url,
            Some(&cookies),
            Duration::from_secs(link::COOKIE_TEST_TIMEOUT_SECS),
        )
    })
    .await
    .map_err(|e| format!("The sign-in check could not be started: {e}"))?;

    let result = probe.verdict();
    Ok(CookieTest {
        result,
        ok: result.ok(),
        message: if result.ok() {
            format!("The configured source could access {tested_url}. This does not prove access to other tracks.")
        } else {
            probe.message()
        },
        tested_url,
    })
}

/// Said when there is no sign-in configured to check, which is a state and not a fault.
pub const NO_COOKIE_SOURCE_TO_TEST: &str =
    "There is no sign-in to check yet. Choose a browser to borrow the sign-in from, or a \
     cookies.txt file you exported, and then check it.";

/// Ask the running batch to stop after the current step. Safe when nothing is running.
#[tauri::command(rename_all = "snake_case")]
pub async fn cancel_batch(state: State<'_, AppState>) -> Result<(), String> {
    state.cancel();
    Ok(())
}

/// Free the single-batch slot, if `batch_id` still owns it. `try_state` rather than `state`: this
/// runs from a worker thread and from a `Drop` that may be unwinding, and panicking there would
/// abort the process.
fn release_slot<R: Runtime>(app: &AppHandle<R>, batch_id: u64) {
    if let Some(state) = app.try_state::<AppState>() {
        state.end_batch(batch_id);
    }
}

/// What the window has been told about a batch so far.
///
/// Only the terminal events are counted, because they are the only ones that settle a row. It
/// exists so [`SlotGuard`] can close a crashed batch out with a tally that is *true* - "two
/// converted, the rest never ran" - rather than a made-up one.
#[derive(Debug, Default)]
pub(crate) struct Reported {
    ok: AtomicUsize,
    failed: AtomicUsize,
    skipped: AtomicUsize,
    /// Set by the real `batch_finished`, which is what tells the guard it has nothing to do.
    settled: std::sync::atomic::AtomicBool,
}

impl Reported {
    pub(crate) fn record(&self, event: &BatchEvent) {
        let counter = match event {
            BatchEvent::Finished { .. } => &self.ok,
            BatchEvent::Failed { .. } => &self.failed,
            BatchEvent::Skipped { .. } => &self.skipped,
            BatchEvent::BatchFinished { .. } => {
                self.settled.store(true, Ordering::SeqCst);
                return;
            }
            BatchEvent::Started { .. } | BatchEvent::Progress { .. } => return,
        };
        counter.fetch_add(1, Ordering::SeqCst);
    }
}

/// Frees the batch slot when the worker thread ends, however it ends - and, if it ended the one
/// way that emits no `batch_finished` of its own, says so.
///
/// [`run_batch`] closes every row and ends with `batch_finished` on every path it controls,
/// including a worker thread that panics mid-file. What it cannot close out is a panic on its *own*
/// thread: `std::thread::spawn` panics when the OS refuses a thread, `Mutex` poisoning is recovered
/// but the sink itself runs arbitrary caller code, and any of that unwinds straight past the final
/// `sink(BatchFinished)`. The slot was already freed here, so the app stayed usable - but the window
/// never heard the batch end, and `batch_finished` is the only event that settles the rows and takes
/// the queue out of `running` (see `handleEvent` in `src/state/store.ts`). The result was a spinner
/// for the rest of the session over work that had stopped.
///
/// So the unwind path emits one, and it tells the truth: the terminal events the window actually
/// received are counted as they happened, and every item that never got one is reported as skipped -
/// which is exactly what happened to it.
pub(crate) struct SlotGuard<R: Runtime> {
    pub(crate) app: AppHandle<R>,
    pub(crate) batch_id: u64,
    /// Items handed to `run_batch`, so the invented tally can account for all of them.
    pub(crate) total: usize,
    pub(crate) reported: Arc<Reported>,
}

impl<R: Runtime> Drop for SlotGuard<R> {
    fn drop(&mut self) {
        // Same order as the happy path: the slot is free before the window is told it is over.
        release_slot(&self.app, self.batch_id);
        if self.reported.settled.load(Ordering::SeqCst) {
            return;
        }
        let ok = self.reported.ok.load(Ordering::SeqCst);
        let failed = self.reported.failed.load(Ordering::SeqCst);
        let reported_skipped = self.reported.skipped.load(Ordering::SeqCst);
        // Whatever never reached a terminal event never ran to an answer. `skipped` is the honest
        // bucket for it, and it is the one the frontend already uses for rows a batch left behind.
        let unaccounted = self.total.saturating_sub(ok + failed + reported_skipped);
        let event =
            BatchEvent::BatchFinished { ok, failed, skipped: reported_skipped + unaccounted };
        eprintln!("[flint] batch {} ended without finishing; closing it out", self.batch_id);
        if let Err(e) = self.app.emit(BATCH_EVENT, event) {
            eprintln!("[flint] could not deliver batch event: {e}");
        }
    }
}

/// Re-scan for optional helpers (e.g. right after the user installed LibreOffice).
#[tauri::command(rename_all = "snake_case")]
pub async fn refresh_tools(state: State<'_, AppState>) -> Result<Vec<ToolStatus>, String> {
    Ok(state.refresh_tools())
}

/// What the shell is busy with. The one command a freshly (re)loaded webview needs to tell "idle"
/// from "a conversion this window has never seen is still running", which is otherwise unknowable:
/// a reload drops every event listener, and the batch it cannot see still owns the single slot.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_activity(state: State<'_, AppState>) -> Result<Activity, String> {
    Ok(state.activity())
}

// ---------------------------------------------------------------------------------------------
// Installing the optional helpers
// ---------------------------------------------------------------------------------------------

/// Every installable package, what it unlocks, and the one command that installs it.
///
/// One row per *package*, not per binary: `tool_ids` names the member binaries, and each of those
/// joins onto [`ToolStatus::id`] from `get_catalog`/`refresh_tools`, which is where the UI reads
/// *whether* the members are there - so "Poppler, two of three binaries present" is derivable
/// without this list claiming anything about presence itself.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_install_plans() -> Result<Vec<PackageInstallPlan>, String> {
    // Blocking work is two `stat` calls (is Homebrew there?), so no `spawn_blocking` needed.
    Ok(install_plans())
}

/// Install one package. Returns as soon as the installer is running; progress arrives on
/// [`crate::INSTALL_EVENT`] and ends with exactly one `finished` event.
///
/// `package_id` is a *package* id (`poppler`), never a binary one (`pdftohtml`): one click is one
/// `brew install`, and asking per binary would run the same formula three times. It is *only* looked
/// up in the allowlist (`convert_core::install`), which yields a fixed argv. Nothing from this
/// argument reaches a process, there is no shell, and an id that is not in the package table is
/// refused here before any thread is spawned.
#[tauri::command(rename_all = "snake_case")]
pub async fn install_tool<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    package_id: Arg<String>,
) -> Result<(), String> {
    // Resolved before the slot is claimed: a refusal (unknown id, no Homebrew, nothing to install)
    // must not lock out the install that would have worked.
    let command = resolve_install(&package_id.0)?;
    let install_id = state.begin_install()?;

    // A plain OS thread, not the async runtime: `brew install` blocks for minutes, and parking a
    // runtime worker for that long would delay every other command - including `cancel_batch`.
    let spawned = std::thread::Builder::new()
        .name("install-tool".into())
        .spawn(move || install::run(app, command, install_id));

    if let Err(e) = spawned {
        // Never leave the single-install guard latched if the thread could not start.
        state.end_install(install_id);
        return Err(format!("Could not start the installer thread: {e}"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// OS integration
// ---------------------------------------------------------------------------------------------

/// Show a file in Finder / Explorer / the system file manager, selected.
///
/// Revealing only *selects*; it never launches anything, so any path the user can see is fair game.
/// The path is still canonicalised first, because that is what guarantees the string we hand the OS
/// helper is an absolute path and not something `open`'s own option parser will read as a flag.
#[tauri::command(rename_all = "snake_case")]
pub async fn reveal_in_finder(path: String) -> Result<(), String> {
    let target = existing(&path)?;
    #[cfg(windows)]
    {
        windows_open(&target, true).await
    }
    #[cfg(not(windows))]
    {
        let shown = target.to_string_lossy().to_string();

        #[cfg(target_os = "macos")]
        let command = ("open", vec!["-R".to_string(), shown]);
        #[cfg(all(unix, not(target_os = "macos")))]
        // No cross-desktop "select this file" call exists on Linux, so we open the containing folder.
        let command = {
            let _ = shown;
            (
                "xdg-open",
                vec![target.parent().unwrap_or(Path::new(".")).to_string_lossy().to_string()],
            )
        };

        spawn_detached(command.0, &command.1)
    }
}

/// Open a file or folder with whatever the OS considers its default application.
///
/// This is the one command in the app that asks the OS to *run* something, and the argument comes
/// from the webview - so it is restricted to the two things the UI ever opens: the folder a batch
/// wrote into, and a file this app produced. [`openable`] is where that is decided.
#[tauri::command(rename_all = "snake_case")]
pub async fn open_path(path: String) -> Result<(), String> {
    let target = openable(&path)?;
    #[cfg(windows)]
    {
        windows_open(&target, false).await
    }
    #[cfg(not(windows))]
    {
        let path = target.to_string_lossy().to_string();

        #[cfg(target_os = "macos")]
        let command = ("open", vec![path]);
        #[cfg(all(unix, not(target_os = "macos")))]
        let command = ("xdg-open", vec![path]);

        spawn_detached(command.0, &command.1)
    }
}

/// Shell APIs receive the path as data, never through cmd.exe's command language.
#[cfg(windows)]
async fn windows_open(path: &Path, reveal: bool) -> Result<(), String> {
    let path = path.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || crate::windows_shell::open(&path, reveal))
        .await
        .map_err(|e| format!("Windows shell task failed: {e}"))?
}

/// Where macOS grants Full Disk Access, as a URL the OS resolves to that exact pane.
///
/// A constant rather than a parameter, and that is the whole design of the command below: see
/// [`open_full_disk_access_settings`].
///
/// **The old spelling on purpose.** Ventura replaced System Preferences' `.prefPane` bundles with
/// ExtensionKit extensions, so this pane also answers to
/// `x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles` - and
/// that address exists only on 13+. The one below works on both: the modern extension still declares
/// the old id for exactly this purpose (`EXAppExtensionAttributes.legacyBundleIdentifier =
/// com.apple.preference.security` in `SecurityPrivacyExtension.appex`, checked on macOS 26.6), and it
/// is the only address that resolves on the 11 and 12 machines this app still ships to
/// (`bundle.macOS.minimumSystemVersion`). Switching to the modern spelling would trade a pane that
/// opens everywhere for one that opens on new Macs and dumps everyone else at the top of System
/// Preferences, and `open` exits 0 either way - so there is nothing to fall back *on*.
pub const FULL_DISK_ACCESS_PANE: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

/// Open System Settings → Privacy & Security → Full Disk Access.
///
/// The way out of the one failure this app cannot fix for the user: Safari keeps its cookies in a
/// container only an app with Full Disk Access can read, so `--cookies-from-browser safari` comes
/// back `Operation not permitted` however well-formed the request
/// (`link::FetchFailure::SafariNeedsFullDiskAccess`). Four levels of System Settings is a lot to ask
/// of somebody who only pasted a link, so the failed row offers the trip.
///
/// **It takes no argument, and that is the security property.** [`open_path`] has to allowlist the
/// path it is handed because the webview chooses it; a `x-apple.systempreferences:` URL cannot be
/// allowlisted usefully - the scheme *is* "open a settings pane", and its payload is a pane id this
/// app has no table of. So nothing is passed: the address is the constant above, the request is the
/// call itself, and a compromised webview that can reach this command can do exactly one thing with
/// it - the thing the button says.
#[tauri::command(rename_all = "snake_case")]
pub async fn open_full_disk_access_settings() -> Result<(), String> {
    // Named as a platform fact rather than left to `open` to fail on: on Linux and Windows there is
    // no such pane and no `open`, and "could not run `open`" is not an answer to "grant a macOS
    // permission". The frontend only ever shows the button on a row Rust classified, which only
    // happens on macOS - this is the honest answer if a build ever gets there anyway.
    if !cfg!(target_os = "macos") {
        return Err("Full Disk Access is a macOS permission, and this is not macOS.".into());
    }
    spawn_detached("open", &[FULL_DISK_ACCESS_PANE.to_string()])
}

/// Resolve an IPC path to something that exists, absolutely.
///
/// `canonicalize` is doing three jobs: it proves the path exists, it makes it absolute (a relative
/// path would otherwise be resolved against the *app's* working directory, and a name beginning
/// with `-` would be read by `open` as an option), and it collapses `..` and symlinks so what we
/// hand the OS is the file we checked - not a link that pointed somewhere else by the time it ran.
fn existing(path: &str) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err("There is no file to show.".into());
    }
    std::fs::canonicalize(path).map_err(|_| format!("{} no longer exists", printable(path)))
}

/// Decide whether the OS may be asked to *open* this path.
///
/// `open` is not "show me this file", it is "do whatever this file is for": handed a `.app`
/// bundle, a `.command`, a `.pkg` or a `.dmg` it executes code, and the argument arrives over IPC.
/// A compromised webview could otherwise turn one of the UI's own microlinks into "launch anything
/// on this disk that the user could have double-clicked".
///
/// So the rule matches what the UI actually opens:
///
/// * a plain directory - the folder a batch wrote into - unless it is one of macOS's *bundle*
///   directories, which are executable despite being folders;
/// * a file whose extension is in the format catalog, i.e. something this app reads or writes.
///
/// Both are allowlists over the catalog rather than a denylist of dangerous extensions, because a
/// denylist is only ever as complete as the last macOS release.
fn openable(path: &str) -> Result<PathBuf, String> {
    let target = existing(path)?;
    let extension =
        target.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();

    if target.is_dir() {
        return if BUNDLE_DIRECTORIES.contains(&extension.as_str()) {
            Err(format!("{} is an application, not a folder.", printable(path)))
        } else {
            Ok(target)
        };
    }
    if catalog_extensions().contains(extension.as_str()) {
        Ok(target)
    } else {
        Err(format!(
            "Flint only opens the files it converts, and .{extension} is not one of \
             them. Use Show in Finder instead."
        ))
    }
}

/// Directory extensions macOS treats as runnable bundles. A folder named like one of these is a
/// program: `open` launches it exactly as double-clicking would.
const BUNDLE_DIRECTORIES: &[&str] = &[
    "app",
    "appex",
    "action",
    "bundle",
    "framework",
    "kext",
    "plugin",
    "prefpane",
    "qlgenerator",
    "saver",
    "scptd",
    "service",
    "workflow",
    "wdgt",
    "xpc",
];

/// Every extension the format catalog knows, in either direction, lower-cased.
///
/// Derived from the catalog rather than listed here: a format added to `convert-core` becomes
/// openable in the same commit that makes it convertible, and nothing else does.
fn catalog_extensions() -> &'static HashSet<String> {
    static EXTENSIONS: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
    EXTENSIONS.get_or_init(|| {
        catalog()
            .iter()
            .flat_map(|f| f.extensions.iter().copied())
            .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
            .collect()
    })
}

/// Run a short-lived OS helper and report a launch failure, never its exit code: `open`/`xdg-open`
/// return immediately and their status says nothing about what the user's app did afterwards.
fn spawn_detached(program: &str, args: &[String]) -> Result<(), String> {
    std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not run `{program}`: {e}"))
}

/// An IPC string we are about to put in a message the UI will render.
///
/// Control characters would let a crafted argument smuggle escape sequences into a toast or the
/// log, and an unbounded one would flood both. (`convert_core::install` does the same to tool ids;
/// its helper is private, and duplicating six lines beats widening that crate's API.)
fn printable(raw: &str) -> String {
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).take(120).collect();
    if cleaned.trim().is_empty() {
        "(empty)".to_string()
    } else {
        cleaned
    }
}

/// Where a file *would* land, so the row can show its destination before anything runs.
///
/// When the conflict policy is `Skip` and the destination is taken, `resolve_conflict` returns
/// `None` - we still hand back the ideal path, because that is the file the UI must point at when
/// it later receives the `skipped` event for this row.
#[tauri::command(rename_all = "snake_case")]
pub async fn estimate_output_path(
    path: String,
    target_id: Arg<String>,
    settings: Settings,
) -> Result<String, String> {
    let target_id = target_id.0;
    let target = by_id(&target_id)
        .ok_or_else(|| format!("Unknown output format `{}`", printable(&target_id)))?;
    // The preview has to be refused on exactly the same terms as the batch, or the row would
    // promise a destination that `start_batch` then declines to write to.
    settings_store::check_output(&settings.output)?;
    let input = PathBuf::from(path);
    let desired = output_path(&input, target, &settings.output);
    let resolved =
        resolve_conflict(&desired, settings.output.on_conflict, &|p| p.exists()).unwrap_or(desired);
    Ok(resolved.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cc-cmd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn folders_expand_recursively_without_dotfiles_or_duplicates() {
        let dir = sandbox("walk");
        std::fs::create_dir_all(dir.join("nested/deeper")).expect("mkdir");
        std::fs::write(dir.join("a.mov"), b"x").expect("write");
        std::fs::write(dir.join(".hidden.mov"), b"x").expect("write");
        std::fs::write(dir.join("nested/b.mp3"), b"x").expect("write");
        std::fs::write(dir.join("nested/deeper/c.png"), b"x").expect("write");

        let dropped = vec![
            dir.to_string_lossy().to_string(),
            dir.join("a.mov").to_string_lossy().to_string(), // dropped twice on purpose
        ];
        let walk = expand_paths(&dropped, MAX_INSPECTED_FILES);
        let found = walk.files;
        let names: Vec<String> = found
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into())
            .collect();

        assert_eq!(names, vec!["a.mov", "b.mp3", "c.png"], "{found:#?}");
        assert!(!walk.truncated, "a folder well under the cap is not truncated");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Dropping a folder is one gesture, and it used to be able to enumerate a whole disk: the walk
    /// took minutes, `ffprobe` was spawned thousands of times and every file became a row the
    /// webview then had to render. The cap bounds all three - and the walk *stops* at it rather than
    /// collecting everything and trimming, which is what `seen` proves below: nothing beyond the cap
    /// was so much as canonicalised.
    #[test]
    fn a_folder_bigger_than_the_cap_is_cut_short_and_says_so() {
        let dir = sandbox("cap");
        // Two folders, so the cap has to be reached in the middle of the walk rather than at the
        // end of the only directory there is.
        std::fs::create_dir_all(dir.join("a")).expect("mkdir");
        std::fs::create_dir_all(dir.join("b")).expect("mkdir");
        for n in 0..12 {
            let leaf = if n < 8 { "a" } else { "b" };
            std::fs::write(dir.join(leaf).join(format!("clip-{n:02}.mov")), b"x").expect("write");
        }

        let walk = expand_paths(&[dir.to_string_lossy().to_string()], 10);
        assert_eq!(walk.files.len(), 10, "the cap is the cap");
        assert_eq!(walk.limit, 10, "the UI is told which number stopped it");
        assert!(walk.truncated, "a drop that was cut short must say so");
        assert_eq!(
            walk.seen.len(),
            10,
            "the walk kept going past the cap and truncated afterwards: {:#?}",
            walk.files
        );

        // Exactly on the cap is not truncation: nothing was left out, so nothing is claimed.
        let exact = expand_paths(&[dir.join("a").to_string_lossy().to_string()], 8);
        assert_eq!(exact.files.len(), 8);
        assert!(!exact.truncated, "a folder that just fits is complete");

        // And a drop of many paths is bounded in total, not per path.
        let both = expand_paths(
            &[
                dir.join("a").to_string_lossy().to_string(),
                dir.join("b").to_string_lossy().to_string(),
            ],
            5,
        );
        assert_eq!(both.files.len(), 5);
        assert!(both.truncated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The other half of the bound: nesting. A tree deeper than [`MAX_WALK_DEPTH`] stops being
    /// walked, which is what makes a cycle harmless even if the filesystem resolves one behind our
    /// back (a bind mount, or a link `is_dir()` reports as a plain directory).
    #[test]
    fn nesting_deeper_than_the_depth_bound_is_not_walked() {
        let dir = sandbox("depth");
        let mut deep = dir.clone();
        for level in 0..(MAX_WALK_DEPTH + 4) {
            deep = deep.join(format!("level-{level}"));
        }
        std::fs::create_dir_all(&deep).expect("mkdir");
        std::fs::write(deep.join("buried.mov"), b"x").expect("write");
        // ...and one file shallow enough to be found, so an empty answer cannot pass by accident.
        std::fs::write(dir.join("visible.mov"), b"x").expect("write");

        let walk = expand_paths(&[dir.to_string_lossy().to_string()], MAX_INSPECTED_FILES);
        let names: Vec<String> = walk
            .files
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into())
            .collect();
        assert_eq!(names, vec!["visible.mov"], "{:#?}", walk.files);
        assert!(walk
            .warnings
            .iter()
            .any(|message| message.contains("Add ") && message.contains("directly")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_enumeration_stops_at_its_own_budget_and_reports_it() {
        let dir = sandbox("scan-budget");
        std::fs::write(dir.join("a.mov"), b"x").unwrap();
        std::fs::write(dir.join("b.mov"), b"x").unwrap();
        let mut walk = Walk::new(MAX_INSPECTED_FILES);
        walk.scanned = MAX_WALK_ENTRIES - 1;
        collect(&dir, 0, &mut walk);
        assert_eq!(walk.scanned, MAX_WALK_ENTRIES);
        assert_eq!(walk.files.len(), 1);
        assert!(!walk.truncated, "this was the scan budget, not the file cap");
        assert!(walk.warnings.iter().any(|message| message.contains("smaller folders")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_files_are_queued_but_symlinked_folders_are_not_walked() {
        let dir = sandbox("symlink");
        let media = dir.join("media");
        let real = dir.join("real");
        std::fs::create_dir_all(&media).expect("mkdir");
        std::fs::create_dir_all(&real).expect("mkdir");
        std::fs::write(real.join("original.mov"), b"x").expect("write");
        // A curated folder of links to the real footage - the common case this used to drop.
        std::os::unix::fs::symlink(real.join("original.mov"), media.join("link.mov"))
            .expect("symlink file");
        // ...and a link to a folder, which must not be followed (it could point at an ancestor).
        std::os::unix::fs::symlink(&real, media.join("loop")).expect("symlink dir");

        let found = expand_paths(&[media.to_string_lossy().to_string()], MAX_INSPECTED_FILES).files;
        let names: Vec<String> = found
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into())
            .collect();
        assert_eq!(names, vec!["link.mov"], "{found:#?}");

        // Dropping the link *and* its target must still produce one row, not two.
        let both = expand_paths(
            &[
                media.join("link.mov").to_string_lossy().to_string(),
                real.join("original.mov").to_string_lossy().to_string(),
            ],
            MAX_INSPECTED_FILES,
        )
        .files;
        assert_eq!(both.len(), 1, "{both:#?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn describe_reports_formats_and_missing_helpers() {
        let dir = sandbox("describe");
        let doc = dir.join("deck.pptx");
        std::fs::write(&doc, b"x").expect("write");

        let bare = ToolRegistry::default();
        let info = describe(&doc, None, &bare);
        assert!(info.supported);
        assert_eq!(info.category.as_deref(), Some("document"));
        assert_eq!(info.default_target.as_deref(), Some("pdf"));
        assert_eq!(info.note.as_deref(), Some("Needs LibreOffice"));
        assert!(!info.id.is_empty());

        let junk = dir.join("thing.sketch");
        std::fs::write(&junk, b"x").expect("write");
        let info = describe(&junk, None, &bare);
        assert!(!info.supported);
        assert_eq!(info.note.as_deref(), Some("Unsupported file type: .sketch"));

        let info = describe(&dir.join("ghost.mov"), None, &bare);
        assert_eq!(info.note.as_deref(), Some("File not found"));

        // Anything FFmpeg handles is noteless even on a machine with no helpers installed.
        let clip = dir.join("clip.mov");
        std::fs::write(&clip, b"x").expect("write");
        let media = MediaInfo {
            duration_secs: Some(65.0),
            width: Some(1920),
            height: Some(1080),
            has_video: true,
            is_animated: true,
            ..Default::default()
        };
        let info = describe(&clip, Some(media), &bare);
        assert!(info.supported && info.note.is_none(), "{info:?}");
        assert_eq!(info.duration_label.as_deref(), Some("1:05"));
        assert_eq!(info.resolution_label.as_deref(), Some("1920×1080"));
        assert_eq!(info.default_target.as_deref(), Some("mp4"));
        assert!(info.suggested_targets.contains(&"webm".to_string()));
        assert!(info.is_animated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The shell's half of the two-names rule (`convert_core::install`'s
    /// `nothing_a_user_reads_names_a_binary` is the other half): every sentence this file puts in
    /// front of a person names the *package*, so dropping a PDF on a bare machine reads "Needs
    /// Poppler or LibreOffice" and never "Needs Poppler (pdftotext) or ...".
    ///
    /// Exhaustive over the catalog, so a new format that leans on a Poppler binary is covered on the
    /// day it is added.
    #[test]
    fn nothing_the_shell_shows_a_user_names_a_binary() {
        let bare = ToolRegistry::default();
        let mut shown: Vec<String> = Vec::new();

        // Every "Needs …" note, for every readable format against every target it suggests.
        for source in catalog() {
            if matches!(source.read, Support::Unsupported) {
                continue;
            }
            let targets = suggested_targets_for(source.category);
            for target in targets.iter().copied().chain([default_target_for(source.category)]) {
                if let Some(note) = helper_note(source, by_id(target), &bare) {
                    shown.push(note);
                }
            }
        }
        // ...and the helper list the pickers render next to every format.
        for category in convert_core::catalog_view(&bare).categories {
            for format in category.inputs.iter().chain(category.outputs.iter()) {
                shown.extend(format.needs.iter().map(|n| (*n).to_string()));
            }
        }
        // ...and the install rows the settings page draws.
        for plan in convert_core::install::install_plans() {
            shown.push(plan.name.clone());
            shown.push(plan.command.clone());
            shown.extend(plan.unlocks.clone());
        }

        for line in &shown {
            assert!(!line.contains("pdfto"), "`{line}` names a binary to a user");
        }
        // The note that started all this: the `pdf` entry declares three Poppler binaries among its
        // readers and the sentence says "Poppler" once.
        let pdf_note = helper_note(by_id("pdf").expect("pdf"), by_id("html"), &bare);
        assert_eq!(
            pdf_note.as_deref(),
            Some("Needs LibreOffice or Poppler or ImageMagick or macOS sips")
        );
        assert!(shown.iter().any(|l| l.contains("Poppler")), "{shown:#?}");
    }

    #[test]
    fn ids_never_repeat() {
        let ids: HashSet<String> = (0..1000).map(|_| next_id()).collect();
        assert_eq!(ids.len(), 1000);
    }

    /// `open_path` asks the OS to *do whatever this path is for*, and its argument comes from the
    /// webview. Handed a `.command`, a `.pkg`, or an `.app` (which is a *directory*), `open`
    /// executes code - so a compromised webview could turn the row's own "Open" microlink into
    /// "launch anything on this disk". Only the two things the UI opens are allowed through.
    #[test]
    fn open_only_ever_hands_the_os_a_converted_file_or_a_plain_folder() {
        let dir = sandbox("openable");
        let real = std::fs::canonicalize(&dir).expect("canonical sandbox");

        // What the UI opens: the folder a batch wrote into, and a file this app produced.
        std::fs::create_dir_all(dir.join("Converted")).expect("mkdir");
        std::fs::write(dir.join("Converted/clip.mp4"), b"x").expect("write");
        std::fs::write(dir.join("Converted/notes.pdf"), b"x").expect("write");
        assert_eq!(openable(&dir.join("Converted").to_string_lossy()), Ok(real.join("Converted")));
        assert_eq!(
            openable(&dir.join("Converted/clip.mp4").to_string_lossy()),
            Ok(real.join("Converted/clip.mp4"))
        );
        assert!(openable(&dir.join("Converted/notes.pdf").to_string_lossy()).is_ok());

        // What it must never open. `.app` is a folder, which is exactly why a plain `is_dir()`
        // check is not enough: `open Something.app` is a launch.
        std::fs::create_dir_all(dir.join("Malware.app/Contents/MacOS")).expect("mkdir");
        std::fs::write(dir.join("payload.command"), b"#!/bin/sh\nid\n").expect("write");
        std::fs::write(dir.join("installer.pkg"), b"x").expect("write");
        std::fs::write(dir.join("disk.dmg"), b"x").expect("write");
        std::fs::write(dir.join("script.sh"), b"#!/bin/sh\nid\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["payload.command", "script.sh"] {
                std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o755))
                    .expect("chmod");
            }
        }
        for forbidden in
            ["Malware.app", "payload.command", "installer.pkg", "disk.dmg", "script.sh"]
        {
            assert!(
                openable(&dir.join(forbidden).to_string_lossy()).is_err(),
                "{forbidden} must not be openable"
            );
        }
        // Nothing at all, and nothing named at all.
        assert!(openable(&dir.join("ghost.mp4").to_string_lossy()).is_err());
        assert!(openable("").is_err());
        assert!(openable("   ").is_err());

        // Revealing does not launch, so it stays permissive - but only for paths that exist.
        assert!(existing(&dir.join("script.sh").to_string_lossy()).is_ok());
        assert!(existing(&dir.join("ghost.mp4").to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Whatever the webview sends, the OS helper is handed an absolute, canonical path: never a
    /// relative one (resolved against the *app's* working directory), never one beginning with `-`
    /// (which `open` reads as an option), and never a symlink that could point elsewhere by the
    /// time the process starts.
    #[cfg(unix)]
    #[test]
    fn an_ipc_path_reaches_the_os_absolute_and_flag_free() {
        let dir = sandbox("argv");
        let real = std::fs::canonicalize(&dir).expect("canonical sandbox");
        std::fs::create_dir_all(dir.join("sub")).expect("mkdir");
        std::fs::write(dir.join("clip.mp4"), b"x").expect("write");
        std::os::unix::fs::symlink(dir.join("clip.mp4"), dir.join("-R.mp4")).expect("symlink");

        // `..` is collapsed, and a name that would have been read as a flag is resolved to the
        // file it points at - under its real, absolute name.
        let climbed = openable(&dir.join("sub/../clip.mp4").to_string_lossy()).expect("openable");
        assert_eq!(climbed, real.join("clip.mp4"));
        let flaggy = openable(&dir.join("-R.mp4").to_string_lossy()).expect("openable");
        assert_eq!(flaggy, real.join("clip.mp4"));
        for resolved in [climbed, flaggy] {
            assert!(resolved.is_absolute(), "{resolved:?}");
            assert!(!resolved.to_string_lossy().starts_with('-'), "{resolved:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every id that comes back out in an error message is bounded and control-free: an unbounded
    /// one floods the toast that renders it verbatim, and an escape sequence dresses it up as
    /// something else in the log.
    #[test]
    fn an_id_echoed_back_into_an_error_cannot_carry_control_characters() {
        assert_eq!(printable("mp\u{1b}[31m4"), "mp[31m4");
        assert_eq!(printable("mp4\nrm -rf ~"), "mp4rm -rf ~");
        assert_eq!(printable(&"x".repeat(5_000)).len(), 120);
        assert_eq!(printable("  "), "(empty)");

        let err = jobs_from(vec![BatchItemArg {
            id: "row".into(),
            path: Some("/tmp/clip.mov".into()),
            url: None,
            target_id: "mp4\u{1b}]0;pwned\u{7}".into(),
        }])
        .expect_err("an unknown target must be refused");
        assert!(!err.chars().any(|c| c.is_control()), "{err:?}");
    }

    /// A crafted batch is refused whole, before a slot is claimed and before any row can carry the
    /// crafted value back into the UI as a `failed` event.
    #[test]
    fn a_batch_with_an_unknown_target_or_a_nameless_file_is_refused_whole() {
        let good = BatchItemArg {
            id: "row-1".into(),
            path: Some("/tmp/clip.mov".into()),
            url: None,
            target_id: "mp4".into(),
        };
        assert_eq!(jobs_from(vec![good.clone()]).map(|j| j.len()), Ok(1));

        let unknown = BatchItemArg { target_id: "not_a_format".into(), ..good.clone() };
        assert!(jobs_from(vec![good.clone(), unknown]).is_err());
        let nameless = BatchItemArg { path: Some("   ".into()), ..good.clone() };
        assert!(jobs_from(vec![good, nameless]).is_err());
    }
}
