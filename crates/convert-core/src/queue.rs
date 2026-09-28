//! Batch orchestration: a work queue, a small worker pool, one event stream for the UI.
//!
//! FFmpeg is already multi-threaded, so the pool deliberately stays small - spawning eight encodes
//! on an 8-core laptop is slower *and* makes the machine unusable.

use crate::engine::{preserve_timestamps, Engine, EngineError, ProgressUpdate};
use crate::format::{by_path, Category, Format, Tool};
use crate::link::{self, FetchOptions, Link};
use crate::paths::{
    claim_output, is_same_file, link_output_path, output_path, sequence_contains,
    sequence_family_exists,
};
use crate::plan::{output_is_time_based, plan, writes_frame_sequence, PlanRequest};
use crate::probe::{seconds_label, MediaInfo};
use crate::progress::Phase;
use crate::settings::{ConflictPolicy, Settings, TrimSettings};
use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Where a job's bytes come from.
///
/// A link is a *source*, not a second pipeline: once it has been fetched into the job's scratch it
/// is a file like any other, and everything after that point - the planner, the engine, the
/// conflict policy - is the code that was already there.
#[derive(Debug, Clone)]
pub enum ItemSource {
    /// A file the user dropped or picked.
    File(PathBuf),
    /// A validated video link (see [`crate::link::Link::parse`]), downloaded before it converts.
    Link(Box<Link>),
}

impl ItemSource {
    /// What to call this source in a message, before anything has been fetched.
    pub fn label(&self) -> String {
        match self {
            ItemSource::File(path) => {
                path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().to_string()
            }
            ItemSource::Link(link) => link.url().to_string(),
        }
    }

    pub fn as_link(&self) -> Option<&Link> {
        match self {
            ItemSource::Link(link) => Some(link),
            ItemSource::File(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BatchItem {
    pub id: String,
    pub source: ItemSource,
    /// Format id chosen in the UI (defaults come from `plan::default_target_for`).
    pub target_id: String,
}

impl BatchItem {
    pub fn file(
        id: impl Into<String>,
        input: impl Into<PathBuf>,
        target_id: impl Into<String>,
    ) -> Self {
        Self { id: id.into(), source: ItemSource::File(input.into()), target_id: target_id.into() }
    }

    pub fn link(id: impl Into<String>, link: Link, target_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: ItemSource::Link(Box::new(link)),
            target_id: target_id.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BatchEvent {
    Started {
        id: String,
        output: PathBuf,
        summary: String,
    },
    /// One progress sample. `phase` says which half of the job it is about - a dropped file only
    /// ever converts, a link downloads first - so the UI can say "Downloading 42%" and then
    /// "Converting 71%" instead of one bar that mysteriously restarts.
    Progress {
        id: String,
        phase: Phase,
        fraction: Option<f32>,
        speed: Option<f64>,
        eta_secs: Option<f64>,
    },
    Finished {
        id: String,
        outputs: Vec<PathBuf>,
        bytes: u64,
        elapsed_ms: u128,
    },
    Failed {
        id: String,
        message: String,
    },
    Skipped {
        id: String,
        reason: String,
    },
    BatchFinished {
        ok: usize,
        failed: usize,
        skipped: usize,
    },
}

pub type EventSink = Arc<dyn Fn(BatchEvent) + Send + Sync>;

/// How many files to convert at the same time.
///
/// `requested == 0` means "decide for me": FFmpeg already saturates the machine on its own, so
/// half the logical cores (capped at four) keeps the laptop usable while the batch runs. An
/// explicit request is honoured up to 16, above which the disk, not the CPU, is the limit.
pub fn worker_count(requested: usize) -> usize {
    if requested > 0 {
        return requested.min(16);
    }
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (cpus / 2).clamp(1, 4)
}

/// Scratch directory for one job.
///
/// The id comes straight from the frontend and is pasted into a path we later `remove_dir_all`,
/// so anything that could climb out of the scratch root (`..`, an absolute path, a stray
/// separator) is replaced rather than trusted. The process id keeps two app instances - or a
/// second run after a crash left files behind - from sharing a directory.
///
/// Sanitising is lossy, so the id cannot be the only thing keeping two jobs apart: `a/b` and `a_b`
/// both become `a_b`, and ids longer than the 64-char cut share whatever prefix they share (a
/// caller that uses the file path as its row id is one `remove_dir_all` away from deleting a
/// sibling job's scratch mid-encode, and `decoded.png` / `intermediate.html` / `frames/` all live
/// at fixed names inside). A per-call counter makes every job's directory its own.
fn temp_dir_for(id: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let safe: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(64)
        .collect();
    let safe = if safe.trim_matches('_').is_empty() { "job".to_string() } else { safe };
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join("flint").join(format!("{}-{safe}-{n}", std::process::id()))
}

/// Lock a mutex, recovering from poisoning: one panicking worker must not take the whole batch
/// down with it - the tally is a plain counter and is always consistent between locks.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Output paths this batch has already handed out, so two workers cannot pick the same one.
struct BatchClaims {
    outputs: Mutex<ReservedOutputs>,
    sources: Vec<PathBuf>,
}
type Claims = Arc<BatchClaims>;

#[derive(Default)]
struct ReservedOutputs {
    files: HashSet<PathBuf>,
    sequences: Vec<PathBuf>,
}

fn reserve_output(
    desired: &Path,
    policy: ConflictPolicy,
    sequence: bool,
    claims: &Claims,
) -> Option<PathBuf> {
    let mut reserved = lock(&claims.outputs);
    let overlaps = |path: &Path| {
        reserved.sequences.iter().any(|family| sequence_contains(family, path))
            || (sequence && reserved.files.iter().any(|file| sequence_contains(path, file)))
    };
    let protects_source = policy == ConflictPolicy::Overwrite
        && (sequence || desired.exists())
        && claims.sources.iter().any(|source| {
            (desired.exists() && is_same_file(source, desired))
                || (sequence && sequence_contains(desired, source))
        });
    let effective = if policy == ConflictPolicy::Overwrite && (protects_source || overlaps(desired))
    {
        ConflictPolicy::Rename
    } else {
        policy
    };
    // Gather family collisions before mutably borrowing the ordinary claim set.
    let families = reserved.sequences.clone();
    let members = if sequence { reserved.files.clone() } else { HashSet::new() };
    let occupied = |path: &Path| {
        path.exists()
            || (sequence && sequence_family_exists(path))
            || families.iter().any(|family| sequence_contains(family, path))
            || (sequence && members.iter().any(|file| sequence_contains(path, file)))
    };
    let output = claim_output(desired, effective, &mut reserved.files, &occupied)?;
    if sequence {
        reserved.sequences.push(output.clone());
    }
    Some(output)
}

fn may_write_sequence(category: Category, animated: bool, target: &Format) -> bool {
    writes_frame_sequence(animated || category == Category::Video, target)
        || (matches!(category, Category::Document | Category::Flash)
            && target.category == Category::Image)
}

#[derive(Debug, Default)]
struct Tally {
    ok: usize,
    failed: usize,
    skipped: usize,
}

/// Convert everything in `items`, blocking until the batch finishes or is cancelled.
pub fn run_batch(
    items: Vec<BatchItem>,
    settings: Settings,
    engine: Arc<Engine>,
    cancel: Arc<AtomicBool>,
    sink: EventSink,
) {
    // The cap, enforced where it cannot be skipped. The IPC layer refuses an oversized paste
    // before a batch is ever started (so the user gets a message, not twenty half-dead rows), but
    // `run_batch` is public and takes whatever it is handed - and twenty concurrent downloads is
    // the one part of this app that can saturate a connection for an hour.
    //
    // Only the links *over* the cap are refused. Failing the whole batch punished the files a user
    // had queued alongside the paste - a folder of holiday clips, nothing to do with any link, all
    // twelve of them red - and refused links the batch had room for.
    let link_count = items.iter().filter(|i| i.source.as_link().is_some()).count();
    let (items, over_the_cap) = split_at_link_cap(items);
    let tally = Arc::new(Mutex::new(Tally::default()));
    if !over_the_cap.is_empty() {
        // The count the message names is the whole paste, because "you pasted 24 links" is what the
        // person reading it did; `remove 4` then names exactly these rows.
        let message =
            link::check_batch_size(link_count).err().map_or_else(String::new, |e| e.to_string());
        let mut t = lock(&tally);
        for item in &over_the_cap {
            sink(BatchEvent::Failed { id: item.id.clone(), message: message.clone() });
            t.failed += 1;
        }
    }

    let sources = items
        .iter()
        .filter_map(|i| match &i.source {
            ItemSource::File(path) => Some(path.clone()),
            _ => None,
        })
        .collect();
    let queue = Arc::new(Mutex::new(VecDeque::from(items)));
    let claims: Claims =
        Arc::new(BatchClaims { outputs: Mutex::new(ReservedOutputs::default()), sources });
    let workers = worker_count(settings.output.parallel_jobs).min(lock(&queue).len());

    let mut handles = Vec::new();
    for _ in 0..workers {
        let queue = queue.clone();
        let tally = tally.clone();
        let claims = claims.clone();
        let engine = engine.clone();
        let cancel = cancel.clone();
        let sink = sink.clone();
        let settings = settings.clone();
        // The item a worker is holding, so a panic can still be reported against the right row
        // instead of vanishing and leaving the UI waiting for an event that never comes.
        let in_flight: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let worker_slot = in_flight.clone();
        handles.push((
            in_flight,
            std::thread::spawn(move || loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let Some(item) = lock(&queue).pop_front() else { break };
                *lock(&worker_slot) = Some(item.id.clone());
                let result = convert_one(&item, &settings, &engine, &cancel, &sink, &claims);
                *lock(&worker_slot) = None;
                let mut t = lock(&tally);
                match result {
                    ItemResult::Ok => t.ok += 1,
                    ItemResult::Failed => t.failed += 1,
                    ItemResult::Skipped => t.skipped += 1,
                }
            }),
        ));
    }
    for (in_flight, handle) in handles {
        if handle.join().is_err() {
            // A worker panicked. If it was holding an item, that row never got a terminal event
            // and the tally is one short - report both rather than leaving a spinner running.
            // If it was between items, nothing was lost: an event with an empty id would only add
            // a row the UI cannot attribute to anything, so stay quiet.
            if let Some(id) = lock(&in_flight).take() {
                sink(BatchEvent::Failed {
                    id,
                    message: "The converter crashed while processing this file".into(),
                });
                lock(&tally).failed += 1;
            }
        }
    }

    // Cancelling stops workers from *pulling* work, so whatever is left in the queue never
    // produced an event. Close those rows out too: every item must end in exactly one terminal
    // event or the UI keeps a spinner running for the rest of the session.
    let reason = if cancel.load(Ordering::Relaxed) {
        "Cancelled"
    } else {
        // The queue can only be non-empty without a cancel if every worker died.
        "The batch stopped before reaching this file"
    };
    let leftovers: Vec<String> = lock(&queue).drain(..).map(|i| i.id).collect();
    {
        let mut t = lock(&tally);
        for id in leftovers {
            sink(BatchEvent::Skipped { id, reason: reason.into() });
            t.skipped += 1;
        }
    }

    let t = lock(&tally);
    sink(BatchEvent::BatchFinished { ok: t.ok, failed: t.failed, skipped: t.skipped });
}

enum ItemResult {
    Ok,
    Failed,
    Skipped,
}

/// Split a batch into "convert these" and "these links are over the cap", keeping order.
///
/// Which links are the ones over the cap is decided the way a person would decide it: the first
/// [`link::MAX_LINKS_PER_BATCH`] links in the batch are the ones that run, and everything after
/// them is refused. Files are never counted and never refused - the cap is about concurrent
/// downloads, and a dropped file is not one.
fn split_at_link_cap(items: Vec<BatchItem>) -> (Vec<BatchItem>, Vec<BatchItem>) {
    let mut kept = Vec::with_capacity(items.len());
    let mut refused = Vec::new();
    let mut links = 0usize;
    for item in items {
        if item.source.as_link().is_some() {
            links += 1;
            if links > link::MAX_LINKS_PER_BATCH {
                refused.push(item);
                continue;
            }
        }
        kept.push(item);
    }
    (kept, refused)
}

/// A reason this file cannot become that target, known before a single process is spawned.
///
/// Only what the probe can prove: a silent screen recording asked to become an MP3 would make
/// FFmpeg exit with "Output file does not contain any stream", which is not a sentence anybody
/// should have to read. When ffprobe said nothing (documents, or no ffprobe at all) we stay quiet
/// and let the real conversion decide.
fn unconvertible(info: Option<&MediaInfo>, target: &Format) -> Option<&'static str> {
    let info = info?;
    if target.category == Category::Audio && !info.has_audio && (info.has_video || info.is_animated)
    {
        return Some("This file has no audio track");
    }
    None
}

/// A trim that can only produce an empty file, in a sentence naming both numbers.
///
/// The one honest refusal this feature needs. `-ss 30` on a 12 second clip makes FFmpeg write a
/// valid container with nothing in it, and a 0-byte MP4 next to a green tick is indistinguishable
/// from a conversion that worked. Only when the source's duration is *known*: yt-dlp and ffprobe
/// both admit to not knowing sometimes, and guessing would refuse files that would have converted
/// perfectly well.
fn trim_past_the_end(
    trim: &TrimSettings,
    duration: Option<f64>,
    time_based: bool,
) -> Option<String> {
    let duration = duration?;
    if !time_based || !trim.starts_past_the_end(duration) {
        return None;
    }
    let (start, _) = trim.effective()?;
    Some(format!(
        "The trim starts at {} and this file is only {} long. Lower the start time, or turn \
         trimming off.",
        seconds_label(start),
        seconds_label(duration)
    ))
}

fn convert_one(
    item: &BatchItem,
    settings: &Settings,
    engine: &Engine,
    cancel: &Arc<AtomicBool>,
    sink: &EventSink,
    claims: &Claims,
) -> ItemResult {
    match &item.source {
        ItemSource::File(input) => {
            convert_file(item, input, settings, engine, cancel, sink, claims)
        }
        ItemSource::Link(url) => convert_link(item, url, settings, engine, cancel, sink, claims),
    }
}

fn convert_file(
    item: &BatchItem,
    input: &Path,
    settings: &Settings,
    engine: &Engine,
    cancel: &Arc<AtomicBool>,
    sink: &EventSink,
    claims: &Claims,
) -> ItemResult {
    let Some(source) = by_path(input) else {
        sink(BatchEvent::Failed {
            id: item.id.clone(),
            message: format!(
                "Unsupported file type: .{}",
                input.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()
            ),
        });
        return ItemResult::Failed;
    };
    let Some(target) = crate::format::by_id(&item.target_id) else {
        sink(BatchEvent::Failed {
            id: item.id.clone(),
            message: format!("Unknown output format `{}`", item.target_id),
        });
        return ItemResult::Failed;
    };

    // Probe first: duration drives the progress bar, frame count decides single-file vs sequence.
    let info = engine.probe_cancellable(input, cancel);
    if cancel.load(Ordering::Relaxed) {
        sink(BatchEvent::Skipped { id: item.id.clone(), reason: "Cancelled".into() });
        return ItemResult::Skipped;
    }
    let animated = info.as_ref().map(|i| i.is_animated).unwrap_or(false);
    let duration = info.as_ref().and_then(|i| i.duration_secs);
    if let Some(reason) = unconvertible(info.as_ref(), target) {
        sink(BatchEvent::Failed { id: item.id.clone(), message: reason.into() });
        return ItemResult::Failed;
    }

    let desired = output_path(input, target, &settings.output);
    if settings.output.on_conflict == ConflictPolicy::Overwrite && is_same_file(input, &desired) {
        sink(BatchEvent::Skipped {
            id: item.id.clone(),
            reason: "Source and destination are the same file".into(),
        });
        return ItemResult::Skipped;
    }
    let sequence = may_write_sequence(source.category, animated, target);
    let claim = reserve_output(&desired, settings.output.on_conflict, sequence, claims);
    let Some(output) = claim else {
        sink(BatchEvent::Skipped {
            id: item.id.clone(),
            reason: "A converted file already exists".into(),
        });
        return ItemResult::Skipped;
    };
    if is_same_file(input, &output) {
        sink(BatchEvent::Skipped {
            id: item.id.clone(),
            reason: "Source and destination are the same file".into(),
        });
        return ItemResult::Skipped;
    }

    let job = ConversionJob {
        id: &item.id,
        input: input.to_path_buf(),
        output,
        source,
        target,
        animated,
        duration,
        stamp_from: Some(input.to_path_buf()),
        announce: true,
    };
    run_conversion(job, temp_dir_for(&item.id), settings, engine, cancel, sink)
}

/// Fetch a link into the job's scratch, then convert the file it produced.
///
/// The two phases are reported as such (see [`BatchEvent::Progress`]), and the row is announced
/// *before* the download starts: a fetch is the slowest thing this app does, and a row with no
/// destination and no phase for two minutes looks like a hang.
fn convert_link(
    item: &BatchItem,
    url: &Link,
    settings: &Settings,
    engine: &Engine,
    cancel: &Arc<AtomicBool>,
    sink: &EventSink,
    claims: &Claims,
) -> ItemResult {
    if settings
        .output
        .custom_dir
        .as_ref()
        .is_some_and(|p| !p.as_os_str().is_empty() && (!p.is_absolute() || !p.is_dir()))
    {
        sink(BatchEvent::Failed {
            id: item.id.clone(),
            message: "The saved link output folder is unavailable. Choose an existing absolute folder in Settings > Output.".into(),
        });
        return ItemResult::Failed;
    }
    let Some(target) = crate::format::by_id(&item.target_id) else {
        sink(BatchEvent::Failed {
            id: item.id.clone(),
            message: format!("Unknown output format `{}`", item.target_id),
        });
        return ItemResult::Failed;
    };
    if url.site().category() == Category::Audio && target.category != Category::Audio {
        sink(BatchEvent::Failed {
            id: item.id.clone(),
            message: "Music links need an audio output format. Choose MP3, M4A, WAV or FLAC."
                .into(),
        });
        return ItemResult::Failed;
    }

    // Ask what the video *is* before fetching it: the title names the output (and so decides the
    // conflict policy's question), and this is also where "private", "removed" and "yt-dlp is not
    // installed" surface - before a byte has been downloaded.
    // The same cookie source the fetch will use, or the probe would refuse a members-only page a
    // moment before the download that would have worked.
    let cookies = settings.link.effective();
    let info = match engine.probe_link_cancellable(url, cookies.as_ref(), cancel) {
        Ok(info) => info,
        Err(EngineError::Cancelled) => {
            sink(BatchEvent::Skipped { id: item.id.clone(), reason: "Cancelled".into() });
            return ItemResult::Skipped;
        }
        Err(e) => {
            sink(BatchEvent::Failed { id: item.id.clone(), message: e.to_string() });
            return ItemResult::Failed;
        }
    };
    if cancel.load(Ordering::Relaxed) {
        sink(BatchEvent::Skipped { id: item.id.clone(), reason: "Cancelled".into() });
        return ItemResult::Skipped;
    }

    let desired = link_output_path(&link::sanitize_title(&info.title), target, &settings.output);
    let sequence = may_write_sequence(url.site().category(), false, target);
    let claim = reserve_output(&desired, settings.output.on_conflict, sequence, claims);
    let Some(output) = claim else {
        sink(BatchEvent::Skipped {
            id: item.id.clone(),
            reason: "A converted file already exists".into(),
        });
        return ItemResult::Skipped;
    };

    sink(BatchEvent::Started {
        id: item.id.clone(),
        output: output.clone(),
        summary: link::link_summary(url, target),
    });
    // An immediate indeterminate sample, so the row says "Downloading" from the first frame
    // rather than after yt-dlp has finished negotiating with the site.
    sink(BatchEvent::Progress {
        id: item.id.clone(),
        phase: Phase::Downloading,
        fraction: None,
        speed: None,
        eta_secs: None,
    });

    let temp_dir = temp_dir_for(&item.id);
    let options = FetchOptions::for_target(
        temp_dir.join("fetch"),
        target,
        engine.tools.path(Tool::Ffmpeg).map(Path::to_path_buf),
        // The JavaScript runtime YouTube's challenges need, if this machine has one. Without it the
        // site refuses the extraction and blames a missing sign-in.
        engine.js_runtime(),
        // Whose sign-in this fetch may borrow, if the user asked for one at all. `None` by default
        // and for everybody who has not been to Settings → Links.
        cookies,
    );
    let fetched = {
        let mut on_progress = progress_reporter(&item.id, sink);
        engine.fetch_link(url, &options, cancel, &mut on_progress)
    };
    let fetched = match fetched {
        Ok(path) => path,
        Err(EngineError::Cancelled) => {
            let _ = std::fs::remove_dir_all(&temp_dir);
            sink(BatchEvent::Skipped { id: item.id.clone(), reason: "Cancelled".into() });
            return ItemResult::Skipped;
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&temp_dir);
            sink(BatchEvent::Failed { id: item.id.clone(), message: e.to_string() });
            return ItemResult::Failed;
        }
    };

    let Some(source) = by_path(&fetched) else {
        let _ = std::fs::remove_dir_all(&temp_dir);
        sink(BatchEvent::Failed {
            id: item.id.clone(),
            message: format!(
                "The site served a format Flint cannot read (.{})",
                fetched.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()
            ),
        });
        return ItemResult::Failed;
    };

    let probed = engine.probe_cancellable(&fetched, cancel);
    if url.site().category() == Category::Audio {
        let actual = probed.as_ref().and_then(|i| i.duration_secs);
        let invalid = probed.as_ref().is_none_or(|i| !i.has_audio)
            || actual.is_none()
            || link::incomplete_music(info.duration_secs, actual);
        if invalid {
            let _ = std::fs::remove_dir_all(&temp_dir);
            if cancel.load(Ordering::Relaxed) {
                sink(BatchEvent::Skipped { id: item.id.clone(), reason: "Cancelled".into() });
                return ItemResult::Skipped;
            }
            sink(BatchEvent::Failed { id: item.id.clone(), message:
                "The source did not provide a complete readable track. Check access on the source site or convert an authorized local file. No partial result was saved.".into() });
            return ItemResult::Failed;
        }
    }
    let job = ConversionJob {
        id: &item.id,
        input: fetched,
        output,
        source,
        target,
        animated: probed.as_ref().map(|i| i.is_animated).unwrap_or(false),
        duration: probed.as_ref().and_then(|i| i.duration_secs).or(info.duration_secs),
        // Nothing to copy a timestamp from: the "source" is a scratch file created seconds ago,
        // and stamping the result with that would be a lie either way.
        stamp_from: None,
        // Already announced above, with the destination the user is waiting to see.
        announce: false,
    };
    run_conversion(job, temp_dir, settings, engine, cancel, sink)
}

/// Everything the shared half of a job needs, once the input is a real file on disk.
struct ConversionJob<'a> {
    id: &'a str,
    input: PathBuf,
    output: PathBuf,
    source: &'static Format,
    target: &'static Format,
    animated: bool,
    duration: Option<f64>,
    stamp_from: Option<PathBuf>,
    announce: bool,
}

/// Plan and run one conversion. Identical for a dropped file and a fetched link, which is the
/// whole point of fetching into a file first.
fn run_conversion(
    job: ConversionJob<'_>,
    temp_dir: PathBuf,
    settings: &Settings,
    engine: &Engine,
    cancel: &Arc<AtomicBool>,
    sink: &EventSink,
) -> ItemResult {
    let effective = crate::crop::media_settings(settings, job.source.category);
    let settings = &effective;
    let preparing = settings.crop.as_ref().is_some_and(|crop| crop.applies_to(job.source.category));
    if preparing && job.announce {
        sink(BatchEvent::Started {
            id: job.id.to_string(),
            output: job.output.clone(),
            summary: "Crop & convert".into(),
        });
    }
    let mut request = PlanRequest {
        input: job.input.clone(),
        output: job.output.clone(),
        source: job.source,
        target: job.target,
        settings: settings.clone(),
        temp_dir: temp_dir.clone(),
        source_is_animated: job.animated,
    };

    if let Err(e) =
        crate::crop::prepare(&mut request, engine, cancel, &mut progress_reporter(job.id, sink))
    {
        let _ = std::fs::remove_dir_all(&temp_dir);
        if cancel.load(Ordering::Relaxed) {
            sink(BatchEvent::Skipped { id: job.id.to_string(), reason: "Cancelled".into() });
            return ItemResult::Skipped;
        }
        sink(BatchEvent::Failed { id: job.id.to_string(), message: e.to_string() });
        return ItemResult::Failed;
    }

    let selected_copy = request.input != job.input
        && request.source.id == request.target.id
        && matches!(request.source.id, "pdf" | "txt");
    let planned = if selected_copy {
        Ok(crate::plan::Plan {
            midi: None,
            steps: vec![],
            output: job.output.clone(),
            summary: "Selected document range".into(),
            temp_paths: vec![],
        })
    } else {
        plan(&request, &engine.tools)
    };
    let plan = match planned {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&temp_dir);
            sink(BatchEvent::Failed { id: job.id.to_string(), message: e.to_string() });
            return ItemResult::Failed;
        }
    };

    // Everything the trim changes about *running* a job, both halves of it derived from the same
    // pair of questions ("does this output have a duration" and "what does the trim leave of it"),
    // so a dropped file and a fetched link cannot drift apart.
    let time_based = output_is_time_based(job.source, job.target, job.animated);
    // Refused where the other pre-flight refusals live: before the row is even announced, and long
    // before a process is spawned.
    if let Some(message) = trim_past_the_end(&settings.trim, job.duration, time_based) {
        let _ = std::fs::remove_dir_all(&temp_dir);
        sink(BatchEvent::Failed { id: job.id.to_string(), message });
        return ItemResult::Failed;
    }

    if job.announce && !preparing {
        sink(BatchEvent::Started {
            id: job.id.to_string(),
            output: plan.output.clone(),
            summary: plan.summary.clone(),
        });
    }

    // The duration the engine measures progress and ETA against is the length of the file the user
    // is going to get, not the length of the source. Hand it the source's own duration under a
    // trim and a 10 second cut of a 10 minute film sits at 2% until it abruptly finishes.
    let expected_duration =
        if time_based { settings.trim.expected_output_secs(job.duration) } else { job.duration };

    let result = {
        let mut on_progress = progress_reporter(job.id, sink);
        if selected_copy {
            crate::crop::publish(&request.input, &request.output, cancel)
        } else {
            engine.run(&plan, expected_duration, cancel, &mut on_progress)
        }
    };
    let _ = std::fs::remove_dir_all(&temp_dir);

    match result {
        Ok(outcome) => {
            if let Some(original) = &job.stamp_from {
                preserve_timestamps(original, &outcome.outputs, &settings.output);
            }
            sink(BatchEvent::Finished {
                id: job.id.to_string(),
                outputs: outcome.outputs,
                bytes: outcome.bytes,
                elapsed_ms: outcome.elapsed_ms,
            });
            ItemResult::Ok
        }
        Err(EngineError::Cancelled) => {
            sink(BatchEvent::Skipped { id: job.id.to_string(), reason: "Cancelled".into() });
            ItemResult::Skipped
        }
        Err(e) => {
            sink(BatchEvent::Failed { id: job.id.to_string(), message: e.to_string() });
            ItemResult::Failed
        }
    }
}

/// Forward engine progress - phase and all - to the event stream for one row.
fn progress_reporter(id: &str, sink: &EventSink) -> impl FnMut(ProgressUpdate) + use<> {
    let id = id.to_string();
    let sink = sink.clone();
    move |u: ProgressUpdate| {
        sink(BatchEvent::Progress {
            id: id.clone(),
            phase: u.phase,
            fraction: u.fraction,
            speed: u.speed,
            eta_secs: u.eta_secs,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Tool;
    use crate::settings::{ConflictPolicy, OutputLocation};
    use crate::tools::ToolRegistry;
    use std::path::Path;

    #[test]
    fn overwrite_cannot_replace_another_batches_source() {
        let dir = sandbox("protected-source");
        let first = dir.join("clip.mov");
        let second = dir.join("clip.mp4");
        std::fs::write(&first, "original mov").unwrap();
        std::fs::write(&second, "original mp4").unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let settings = Settings {
            output: crate::settings::OutputSettings {
                location: OutputLocation::SameFolder,
                on_conflict: ConflictPolicy::Overwrite,
                parallel_jobs: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        run_batch(
            vec![
                BatchItem::file("first", &first, "mp4"),
                BatchItem::file("second", &second, "mkv"),
            ],
            settings,
            engine_for(&dir),
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "original mp4");
        assert!(dir.join("clip (1).mp4").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cc-queue-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stand-in for ffmpeg. `delay` seconds of "encoding" before it writes anything, which is
    /// how a race between two workers becomes reproducible. Every invocation appends its whole argv
    /// to `<dir>/ffmpeg.log` (see [`ffmpeg_calls`]), which is how the argv assertions see what the
    /// planner really produced for a job that went through the queue.
    fn fake_ffmpeg(dir: &Path, delay: &str) -> PathBuf {
        let log = dir.join("ffmpeg.log");
        let p = dir.join("ffmpeg");
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> \"{log}\"; done\nprintf '===\\n' >> \"{log}\"\necho 'out_time_us=1000000'\necho 'progress=end'\nsleep {delay}\nout=\"\"\nfor a in \"$@\"; do out=\"$a\"; done\nprintf 'ok' > \"$out\"\n",
                log = log.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    /// ffmpeg's argv, one invocation per inner vector, in the order they ran.
    fn ffmpeg_calls(dir: &Path) -> Vec<Vec<String>> {
        let text = std::fs::read_to_string(dir.join("ffmpeg.log")).unwrap_or_default();
        text.split("===\n")
            .filter(|c| !c.trim().is_empty())
            .map(|c| c.lines().map(str::to_string).collect())
            .collect()
    }

    /// The one call in a batch whose output is `name`, so a mixed batch can be read row by row.
    fn ffmpeg_call_for(dir: &Path, name: &str) -> Vec<String> {
        let calls = ffmpeg_calls(dir);
        calls
            .iter()
            .find(|c| c.last().map(|last| last.ends_with(name)).unwrap_or(false))
            .cloned()
            .unwrap_or_else(|| panic!("no ffmpeg call wrote {name}: {calls:#?}"))
    }

    fn engine_for(dir: &Path) -> Arc<Engine> {
        engine_delayed(dir, "0")
    }

    /// An ffmpeg that behaves like the image2 muxer: it expands the `-%04d` pattern in the output
    /// path into two numbered files, plus an ffprobe that reports an animated source so the planner
    /// actually chooses a sequence.
    fn engine_sequence(dir: &Path) -> Arc<Engine> {
        let ffmpeg = script(
            dir,
            "ffmpeg-seq",
            r#"out=""
for a in "$@"; do out="$a"; done
d=$(dirname "$out")
b=$(basename "$out")
prefix=${b%-*}
ext=${b##*.}
mkdir -p "$d"
printf 'a' > "$d/$prefix-0001.$ext"
printf 'b' > "$d/$prefix-0002.$ext"
echo 'progress=end'
"#,
        );
        let ffprobe = script(
            dir,
            "ffprobe-seq",
            r#"printf '{"streams":[{"codec_type":"video","codec_name":"h264","nb_frames":"30"}],"format":{"duration":"2.0"}}'
"#,
        );
        let mut tools = ToolRegistry::default();
        tools.set(Tool::Ffmpeg, ffmpeg);
        tools.set(Tool::Ffprobe, ffprobe);
        Arc::new(Engine::new(tools))
    }

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    fn engine_delayed(dir: &Path, delay: &str) -> Arc<Engine> {
        let mut tools = ToolRegistry::default();
        tools.set(Tool::Ffmpeg, fake_ffmpeg(dir, delay));
        Arc::new(Engine::new(tools))
    }

    fn collect(events: &Arc<Mutex<Vec<BatchEvent>>>) -> EventSink {
        let sink = events.clone();
        Arc::new(move |e: BatchEvent| sink.lock().unwrap().push(e))
    }

    // -----------------------------------------------------------------------------------------
    // Links
    // -----------------------------------------------------------------------------------------

    /// A stand-in for yt-dlp that answers a probe with `title`, and a fetch with `delay` seconds of
    /// downloading and a file in the directory it was told to write into. Every invocation appends
    /// its whole argv to `<dir>/yt-dlp.log`, which is how the argv assertions see what the queue
    /// really passed.
    fn fake_yt_dlp(dir: &Path, title: &str, delay: &str) -> PathBuf {
        let log = dir.join("yt-dlp.log");
        script(
            dir,
            "yt-dlp",
            &format!(
                r#"for a in "$@"; do printf '%s\n' "$a" >> "{log}"; done
printf -- '--\n' >> "{log}"
skip=0
out=""
prev=""
for a in "$@"; do
  case "$a" in --skip-download) skip=1 ;; esac
  if [ "$prev" = "-o" ]; then out="$a"; fi
  prev="$a"
done
if [ "$skip" = "1" ]; then
  printf '%s\n' '{title}'
  printf '%s\n' '123'
  exit 0
fi
d=$(dirname "$out")
mkdir -p "$d"
echo "[download] Destination: $d/source.mp4"
echo "[download]   0.0% of 10.00MiB at 1.00MiB/s ETA 00:10"
printf 'partial' > "$d/source.mp4"
sleep {delay}
echo "[download]  42.0% of 10.00MiB at 2.00MiB/s ETA 00:07"
printf 'a fetched video' > "$d/source.mp4"
echo "[download] 100% of 10.00MiB in 00:05"
"#,
                log = log.display()
            ),
        )
    }

    /// yt-dlp's argv, one invocation per inner vector.
    fn yt_dlp_calls(dir: &Path) -> Vec<Vec<String>> {
        let text = std::fs::read_to_string(dir.join("yt-dlp.log")).unwrap_or_default();
        let mut calls = Vec::new();
        let mut current = Vec::new();
        for line in text.lines() {
            if line == "--" && !current.is_empty() {
                // A real `--` separator is followed by the URL, so only the sentinel the script
                // writes after the last argument ends a call.
                if current.last().map(|s: &String| s.starts_with("http")).unwrap_or(false) {
                    calls.push(std::mem::take(&mut current));
                    continue;
                }
            }
            current.push(line.to_string());
        }
        if !current.is_empty() {
            calls.push(current);
        }
        calls
    }

    /// A registry with a fake ffmpeg and a fake yt-dlp, i.e. the machine a link job runs on.
    fn engine_with_yt_dlp(dir: &Path, title: &str, delay: &str) -> (Arc<Engine>, PathBuf) {
        let ffmpeg = fake_ffmpeg(dir, "0");
        let mut tools = ToolRegistry::default();
        tools.set(Tool::Ffmpeg, ffmpeg.clone());
        tools.set(Tool::YtDlp, fake_yt_dlp(dir, title, delay));
        (Arc::new(Engine::new(tools)), ffmpeg)
    }

    fn to_folder(dir: &Path) -> Settings {
        let mut settings = Settings::default();
        // Links have no source folder, so the destination is the custom one when there is one.
        std::fs::create_dir_all(dir.join("out")).unwrap();
        settings.output.custom_dir = Some(dir.join("out"));
        settings.output.parallel_jobs = 1;
        settings
    }

    fn parsed(url: &str) -> Link {
        Link::parse(url).expect("a link the parser accepts")
    }

    fn progress_phases(events: &[BatchEvent]) -> Vec<(Phase, Option<f32>)> {
        events
            .iter()
            .filter_map(|e| match e {
                BatchEvent::Progress { phase, fraction, .. } => Some((*phase, *fraction)),
                _ => None,
            })
            .collect()
    }

    /// The whole feature, end to end: a pasted link is fetched and then converted by the pipeline
    /// that was already there, and the row reports the two phases separately so the UI can say
    /// "Downloading 42%" before it says "Converting".
    #[test]
    fn a_link_is_fetched_and_then_converted_in_two_reported_phases() {
        let dir = sandbox("link-two-phase");
        let (engine, _) = engine_with_yt_dlp(&dir, "A Talk About Rust", "0");

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            vec![BatchItem::link(
                "1",
                parsed("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
                "mp4",
            )],
            to_folder(&dir),
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        let output = events
            .iter()
            .find_map(|e| match e {
                BatchEvent::Finished { outputs, .. } => outputs.first().cloned(),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the link should convert: {events:#?}"));
        // Named from the video's title, in the folder the user chose.
        assert_eq!(output, dir.join("out").join("A Talk About Rust.mp4"));
        assert!(output.exists());

        let phases = progress_phases(&events);
        assert!(
            phases.iter().any(|(p, f)| *p == Phase::Downloading && *f == Some(0.42)),
            "the download must report its own progress: {phases:?}"
        );
        assert!(
            phases.iter().any(|(p, _)| *p == Phase::Converting),
            "and the conversion must report its own: {phases:?}"
        );
        let first_convert =
            phases.iter().position(|(p, _)| *p == Phase::Converting).expect("a convert sample");
        let last_download = phases.iter().rposition(|(p, _)| *p == Phase::Downloading).unwrap();
        assert!(last_download < first_convert, "download comes first: {phases:?}");
        // The scratch is gone, download and all.
        let root = std::env::temp_dir().join("flint");
        assert!(!root.join("1").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An audio target must fetch the audio stream on its own - the difference between a 4 MB
    /// download and a 400 MB one - and yt-dlp must be handed *our* ffmpeg, never asked to find one.
    #[test]
    fn an_audio_target_fetches_audio_only_and_is_given_our_bundled_ffmpeg() {
        for (target, expected) in [("mp3", "bestaudio*/best"), ("mp4", "bestvideo*+bestaudio/best")]
        {
            let dir = sandbox(&format!("link-format-{target}"));
            let (engine, ffmpeg) = engine_with_yt_dlp(&dir, "Clip", "0");
            let events = Arc::new(Mutex::new(Vec::new()));
            run_batch(
                vec![BatchItem::link("1", parsed("https://youtu.be/abc123"), target)],
                to_folder(&dir),
                engine,
                Arc::new(AtomicBool::new(false)),
                collect(&events),
            );

            let calls = yt_dlp_calls(&dir);
            assert_eq!(calls.len(), 2, "one probe, one fetch: {calls:#?}");
            let fetch = calls.last().unwrap();
            let selector = fetch
                .iter()
                .position(|a| a == "-f")
                .map(|i| fetch[i + 1].clone())
                .unwrap_or_else(|| panic!("no -f in {fetch:?}"));
            assert_eq!(selector, expected, "{target}");
            let location = fetch
                .iter()
                .position(|a| a == "--ffmpeg-location")
                .map(|i| PathBuf::from(&fetch[i + 1]))
                .unwrap_or_else(|| panic!("no --ffmpeg-location in {fetch:?}"));
            assert_eq!(location, ffmpeg, "yt-dlp must use the ffmpeg we ship");
            // ...and the URL is the last argument, behind a `--`, so it can never be read as one.
            assert_eq!(fetch.last().unwrap(), "https://youtu.be/abc123");
            assert_eq!(fetch[fetch.len() - 2], "--");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Titles are remote text. `/` would silently become a directory, and the rest of these are
    /// perfectly ordinary videos.
    #[test]
    fn the_output_is_named_from_the_title_whatever_the_title_is() {
        for (title, stem) in [
            ("AC/DC - Back In Black", "AC-DC - Back In Black"),
            ("   ", "video"),
            ("...", "video"),
            ("-rf /", "rf"),
        ] {
            let dir = sandbox("link-title");
            let (engine, _) = engine_with_yt_dlp(&dir, title, "0");
            let events = Arc::new(Mutex::new(Vec::new()));
            run_batch(
                vec![BatchItem::link("1", parsed("https://youtu.be/abc123"), "mp4")],
                to_folder(&dir),
                engine,
                Arc::new(AtomicBool::new(false)),
                collect(&events),
            );
            let events = events.lock().unwrap();
            let output = events
                .iter()
                .find_map(|e| match e {
                    BatchEvent::Finished { outputs, .. } => outputs.first().cloned(),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("`{title}` should convert: {events:#?}"));
            assert_eq!(output, dir.join("out").join(format!("{stem}.mp4")), "`{title}`");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Stop during a download has to reach yt-dlp itself, and what it was part-way through writing
    /// must not survive: a 40% MP4 in the destination folder is indistinguishable from a result.
    #[test]
    fn cancelling_during_the_fetch_kills_yt_dlp_and_leaves_nothing_behind() {
        let dir = sandbox("link-cancel");
        let (engine, _) = engine_with_yt_dlp(&dir, "Long Video", "30");
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            flag.store(true, Ordering::Relaxed);
        });

        let events = Arc::new(Mutex::new(Vec::new()));
        let started = std::time::Instant::now();
        run_batch(
            vec![BatchItem::link("1", parsed("https://youtu.be/abc123"), "mp4")],
            to_folder(&dir),
            engine,
            cancel,
            collect(&events),
        );
        assert!(started.elapsed().as_secs() < 20, "Stop must not wait for the download to finish");

        let events = events.lock().unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, BatchEvent::Skipped { reason, .. } if reason == "Cancelled")),
            "{events:#?}"
        );
        assert!(!events.iter().any(|e| matches!(e, BatchEvent::Finished { .. })));
        // Nothing in the destination, and no scratch left holding the partial download.
        let left: Vec<String> = std::fs::read_dir(dir.join("out"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(left.is_empty(), "a cancelled fetch left files behind: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Twenty is the cap, and the core is where it is enforced: a payload that never went through
    /// the UI is still refused, with a message that names the count it was given.
    ///
    /// Refused *link by link*, not batch by batch. The twenty-first link is the one over the cap,
    /// so it is the one that fails; the twenty in front of it are exactly the batch the cap allows
    /// and they convert.
    #[test]
    fn a_paste_of_more_than_twenty_links_is_refused_by_the_core() {
        let dir = sandbox("link-cap");
        let (engine, _) = engine_with_yt_dlp(&dir, "Clip", "0");
        let items: Vec<BatchItem> = (0..21)
            .map(|i| {
                BatchItem::link(
                    format!("{i}"),
                    parsed(&format!("https://youtu.be/video{i:03}")),
                    "mp4",
                )
            })
            .collect();

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            items,
            to_folder(&dir),
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        let failures: Vec<(&String, &String)> = events
            .iter()
            .filter_map(|e| match e {
                BatchEvent::Failed { id, message } => Some((id, message)),
                _ => None,
            })
            .collect();
        assert_eq!(failures.len(), 1, "only the link over the cap fails: {events:#?}");
        assert_eq!(failures[0].0, "20", "the twenty-first link is the one refused");
        assert!(failures[0].1.contains("21"), "the message names the count: {}", failures[0].1);
        assert!(failures[0].1.contains("20 at a time"), "{}", failures[0].1);
        assert!(failures[0].1.contains("remove 1"), "{}", failures[0].1);
        assert!(matches!(
            events.last().unwrap(),
            BatchEvent::BatchFinished { ok: 20, failed: 1, skipped: 0 }
        ));
        // ...and exactly twenty is fine.
        assert!(link::check_batch_size(20).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The cap is about concurrent downloads, so a dropped file has nothing to do with it.
    ///
    /// An oversized paste used to fail *every* row in the batch, including the folder of holiday
    /// clips the user had queued beside it: twelve red rows, none of which was a link, and no way
    /// to tell from the message why their own files had been refused.
    #[test]
    fn a_file_queued_beside_an_oversized_paste_still_converts() {
        let dir = sandbox("link-cap-files");
        let (engine, _) = engine_with_yt_dlp(&dir, "Clip", "0");
        std::fs::write(dir.join("holiday.mov"), b"x").expect("write");

        let mut items: Vec<BatchItem> =
            vec![BatchItem::file("file", dir.join("holiday.mov"), "mp4")];
        items.extend((0..21).map(|i| {
            BatchItem::link(format!("{i}"), parsed(&format!("https://youtu.be/video{i:03}")), "mp4")
        }));

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            items,
            to_folder(&dir),
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        let refused: Vec<&String> = events
            .iter()
            .filter_map(|e| match e {
                BatchEvent::Failed { id, .. } => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(refused, vec!["20"], "the file was punished for the paste: {events:#?}");
        assert!(
            events.iter().any(|e| matches!(e, BatchEvent::Finished { id, .. } if id == "file")),
            "the dropped file never converted: {events:#?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// yt-dlp not being installed is the first thing most people will hit, and it is a one-click
    /// fix - so the row has to say so rather than reporting a spawn failure.
    #[test]
    fn a_link_without_yt_dlp_says_how_to_install_it() {
        let dir = sandbox("link-no-tool");
        let mut tools = ToolRegistry::default();
        tools.set(Tool::Ffmpeg, fake_ffmpeg(&dir, "0"));
        let engine = Arc::new(Engine::new(tools));

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            vec![BatchItem::link("1", parsed("https://youtu.be/abc123"), "mp3")],
            to_folder(&dir),
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        let message = events
            .iter()
            .find_map(|e| match e {
                BatchEvent::Failed { message, .. } => Some(message.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{events:#?}"));
        assert!(message.contains("yt-dlp is not installed"), "{message}");
        assert!(message.contains("Settings"), "{message}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file dropped into the same batch as a link still behaves exactly as it did: same
    /// destination rules, same events, and its progress is `converting` from the first sample.
    #[test]
    fn files_and_links_convert_side_by_side_in_one_batch() {
        let dir = sandbox("link-mixed");
        let (engine, _) = engine_with_yt_dlp(&dir, "Clip", "0");
        let input = dir.join("holiday.mov");
        std::fs::write(&input, b"x").unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        let mut settings = to_folder(&dir);
        settings.output.location = crate::settings::OutputLocation::SameFolder;
        run_batch(
            vec![
                BatchItem::file("file", input, "mp4"),
                BatchItem::link("link", parsed("https://youtu.be/abc123"), "mp4"),
            ],
            settings,
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        let outputs: Vec<(String, PathBuf)> = events
            .iter()
            .filter_map(|e| match e {
                BatchEvent::Finished { id, outputs, .. } => {
                    Some((id.clone(), outputs.first().cloned().unwrap_or_default()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(outputs.len(), 2, "{events:#?}");
        let file_out = &outputs.iter().find(|(id, _)| id == "file").unwrap().1;
        let link_out = &outputs.iter().find(|(id, _)| id == "link").unwrap().1;
        // The dropped file keeps "same folder"; the link cannot, so it goes to the chosen folder.
        assert_eq!(file_out, &dir.join("holiday.mp4"));
        assert_eq!(link_out, &dir.join("out").join("Clip.mp4"));
        // A dropped file never claims to be downloading.
        let file_phases: Vec<Phase> = events
            .iter()
            .filter_map(|e| match e {
                BatchEvent::Progress { id, phase, .. } if id == "file" => Some(*phase),
                _ => None,
            })
            .collect();
        assert!(!file_phases.is_empty());
        assert!(file_phases.iter().all(|p| *p == Phase::Converting), "{file_phases:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn worker_count_is_conservative() {
        assert_eq!(worker_count(3), 3);
        assert_eq!(worker_count(999), 16);
        let auto = worker_count(0);
        assert!((1..=4).contains(&auto), "auto workers = {auto}");
    }

    /// Item ids come from the frontend and end up in a path the engine later `remove_dir_all`s.
    #[test]
    fn a_job_id_cannot_climb_out_of_the_scratch_directory() {
        let root = std::env::temp_dir().join("flint");
        for id in ["../../etc", "/etc/passwd", "..", "a/b", "", "  "] {
            let dir = temp_dir_for(id);
            assert!(dir.starts_with(&root), "{id} -> {}", dir.display());
            assert_eq!(dir.components().count(), root.components().count() + 1, "{id}");
            let leaf = dir.file_name().unwrap().to_string_lossy().to_string();
            assert!(!leaf.contains(".."), "{id} -> {leaf}");
        }
        // Sanitising is lossy and the id is cut at 64 characters, so two different rows can arrive
        // at the same safe name. Every job still gets its own directory - otherwise one job's
        // cleanup deletes the scratch another job is still writing into.
        assert_ne!(temp_dir_for("a/b"), temp_dir_for("a_b"));
        let long = "x".repeat(100);
        assert_ne!(temp_dir_for(&format!("{long}-first")), temp_dir_for(&format!("{long}-second")));
        assert_ne!(temp_dir_for("job-1"), temp_dir_for("job-1"));
    }

    #[test]
    fn a_silent_video_is_rejected_before_anything_is_spawned() {
        let silent = MediaInfo {
            has_video: true,
            has_audio: false,
            is_animated: true,
            ..Default::default()
        };
        let mp3 = crate::format::by_id("mp3").unwrap();
        assert_eq!(unconvertible(Some(&silent), mp3), Some("This file has no audio track"));
        // ...but the same file is perfectly convertible to a video, and an unprobed file (a
        // document, or no ffprobe on the machine) must not be pre-judged.
        assert_eq!(unconvertible(Some(&silent), crate::format::by_id("mp4").unwrap()), None);
        assert_eq!(unconvertible(None, mp3), None);
        let with_sound = MediaInfo { has_audio: true, ..silent.clone() };
        assert_eq!(unconvertible(Some(&with_sound), mp3), None);
    }

    #[test]
    fn converts_a_whole_batch_in_parallel() {
        let dir = sandbox("batch");
        let engine = engine_for(&dir);
        let mut items = vec![];
        for i in 0..5 {
            let input = dir.join(format!("clip{i}.mov"));
            std::fs::write(&input, b"x").unwrap();
            items.push(BatchItem::file(format!("job{i}"), input, "mp4"));
        }

        let events = Arc::new(Mutex::new(Vec::new()));
        let mut settings = Settings::default();
        settings.output.parallel_jobs = 2;
        run_batch(items, settings, engine, Arc::new(AtomicBool::new(false)), collect(&events));

        let events = events.lock().unwrap();
        let finished = events.iter().filter(|e| matches!(e, BatchEvent::Finished { .. })).count();
        assert_eq!(finished, 5, "{events:#?}");
        match events.last().unwrap() {
            BatchEvent::BatchFinished { ok, failed, skipped } => {
                assert_eq!((*ok, *failed, *skipped), (5, 0, 0));
            }
            other => panic!("expected BatchFinished, got {other:?}"),
        }
        for i in 0..5 {
            assert!(dir.join(format!("Converted/clip{i}.mp4")).exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_files_fail_individually_without_stopping_the_batch() {
        let dir = sandbox("mixed");
        let engine = engine_for(&dir);
        let good = dir.join("a.mov");
        let bad = dir.join("b.sketch");
        std::fs::write(&good, b"x").unwrap();
        std::fs::write(&bad, b"x").unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            vec![BatchItem::file("1", good, "mp4"), BatchItem::file("2", bad, "mp4")],
            Settings::default(),
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        assert!(events.iter().any(|e| matches!(e, BatchEvent::Finished { id, .. } if id == "1")));
        let failed = events
            .iter()
            .find_map(|e| match e {
                BatchEvent::Failed { id, message } if id == "2" => Some(message.clone()),
                _ => None,
            })
            .expect("the unsupported file should report a failure");
        assert!(failed.contains("sketch"), "{failed}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_output_is_skipped_when_asked() {
        let dir = sandbox("skip");
        let engine = engine_for(&dir);
        let input = dir.join("clip.mov");
        std::fs::write(&input, b"x").unwrap();
        std::fs::write(dir.join("clip.mp4"), b"already here").unwrap();

        let mut settings = Settings::default();
        settings.output.location = OutputLocation::SameFolder;
        settings.output.on_conflict = ConflictPolicy::Skip;

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            vec![BatchItem::file("1", input, "mp4")],
            settings,
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        assert!(events.iter().any(|e| matches!(e, BatchEvent::Skipped { .. })), "{events:#?}");
        assert_eq!(std::fs::read_to_string(dir.join("clip.mp4")).unwrap(), "already here");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A frame extraction writes `clip-0001.png`, ... but only ever claimed `clip.png` - a name it
    /// never creates - so the conflict policy was asked the wrong question and answered "no
    /// conflict". Running the same extraction twice therefore overwrote the first run's frames even
    /// on the default Rename policy, which is exactly the promise the app must not break.
    #[test]
    fn a_second_frame_extraction_does_not_overwrite_the_first_runs_frames() {
        let dir = sandbox("frames-again");
        let engine = engine_sequence(&dir);
        let input = dir.join("clip.mov");
        std::fs::write(&input, b"x").unwrap();
        // What a first run left behind.
        std::fs::write(dir.join("clip-0001.png"), b"first run").unwrap();
        std::fs::write(dir.join("clip-0002.png"), b"first run").unwrap();

        let mut settings = Settings::default();
        settings.output.location = OutputLocation::SameFolder; // land next to the first run

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            vec![BatchItem::file("1", input, "png")],
            settings,
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        assert!(
            events.iter().any(|e| matches!(e, BatchEvent::Finished { .. })),
            "the row should still convert, just under another name: {events:#?}"
        );
        assert_eq!(std::fs::read_to_string(dir.join("clip-0001.png")).unwrap(), "first run");
        assert_eq!(std::fs::read_to_string(dir.join("clip-0002.png")).unwrap(), "first run");
        assert!(dir.join("clip (1)-0001.png").exists(), "{:?}", dir_listing(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn dir_listing(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect()
    }

    #[test]
    fn links_respect_existing_frame_families_under_rename_and_skip() {
        let dir = sandbox("link-frame-conflicts");
        let mut tools = engine_sequence(&dir).tools.clone();
        tools.set(Tool::YtDlp, fake_yt_dlp(&dir, "Clip", "0"));
        let engine = Arc::new(Engine::new(tools));
        let mut settings = to_folder(&dir);
        let old = dir.join("out/Clip-0001.png");
        std::fs::write(&old, b"earlier result").unwrap();
        for policy in [ConflictPolicy::Rename, ConflictPolicy::Skip] {
            settings.output.on_conflict = policy;
            let events = Arc::new(Mutex::new(Vec::new()));
            run_batch(
                vec![BatchItem::link("link", parsed("https://youtu.be/abc"), "png")],
                settings.clone(),
                engine.clone(),
                Arc::new(AtomicBool::new(false)),
                collect(&events),
            );
            assert_eq!(std::fs::read(&old).unwrap(), b"earlier result");
            let events = events.lock().unwrap();
            if policy == ConflictPolicy::Skip {
                assert!(
                    events.iter().any(|e| matches!(e, BatchEvent::Skipped { .. })),
                    "{events:?}"
                );
            } else {
                assert!(dir.join("out/Clip (1)-0001.png").is_file(), "{events:?}");
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sequence_claims_protect_sources_and_other_outputs_in_either_order() {
        let dir = sandbox("sequence-claims");
        let frame = dir.join("Clip-0001.png");
        std::fs::write(&frame, b"source").unwrap();
        let claims = Arc::new(BatchClaims {
            outputs: Mutex::new(ReservedOutputs::default()),
            sources: vec![frame.clone()],
        });
        let base = dir.join("Clip.png");
        assert_eq!(
            reserve_output(&base, ConflictPolicy::Overwrite, true, &claims),
            Some(dir.join("Clip (1).png"))
        );
        let next_frame = dir.join("Clip (1)-0001.png");
        assert_eq!(
            reserve_output(&next_frame, ConflictPolicy::Overwrite, false, &claims),
            Some(dir.join("Clip (1)-0001 (1).png"))
        );
        let single = dir.join("other-0001.png");
        assert_eq!(reserve_output(&single, ConflictPolicy::Rename, false, &claims), Some(single));
        assert_eq!(
            reserve_output(&dir.join("other.png"), ConflictPolicy::Overwrite, true, &claims),
            Some(dir.join("other (1).png"))
        );
        assert!(may_write_sequence(
            Category::Document,
            false,
            crate::format::by_id("png").unwrap()
        ));
        assert_eq!(std::fs::read(frame).unwrap(), b"source");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rename_policy_keeps_both_files() {
        let dir = sandbox("rename");
        let engine = engine_for(&dir);
        let input = dir.join("clip.mov");
        std::fs::write(&input, b"x").unwrap();
        std::fs::write(dir.join("clip.mp4"), b"original").unwrap();

        let mut settings = Settings::default();
        settings.output.location = OutputLocation::SameFolder; // conflict on purpose

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            vec![BatchItem::file("1", input, "mp4")],
            settings,
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        assert_eq!(std::fs::read_to_string(dir.join("clip.mp4")).unwrap(), "original");
        assert!(dir.join("clip (1).mp4").exists(), "renamed output missing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_sources_with_one_name_do_not_overwrite_each_others_output() {
        let dir = sandbox("collide");
        // `clip.mov` and `clip.avi` in one folder both want `Converted/clip.mp4`. With two workers
        // both jobs pick their path before either has written a file, so the filesystem check
        // cannot see the conflict - only the batch can.
        let engine = engine_delayed(&dir, "0.4");
        let mut items = vec![];
        for (i, ext) in ["mov", "avi"].iter().enumerate() {
            let input = dir.join(format!("clip.{ext}"));
            std::fs::write(&input, b"x").unwrap();
            items.push(BatchItem::file(format!("{i}"), input, "mp4"));
        }

        let events = Arc::new(Mutex::new(Vec::new()));
        let mut settings = Settings::default(); // Converted/ subfolder, rename on conflict
        settings.output.parallel_jobs = 2;
        run_batch(items, settings, engine, Arc::new(AtomicBool::new(false)), collect(&events));

        let events = events.lock().unwrap();
        let outputs: Vec<PathBuf> = events
            .iter()
            .filter_map(|e| match e {
                BatchEvent::Finished { outputs, .. } => outputs.first().cloned(),
                _ => None,
            })
            .collect();
        assert_eq!(outputs.len(), 2, "{events:#?}");
        assert_ne!(outputs[0], outputs[1], "both rows reported the same file: {outputs:?}");
        assert!(dir.join("Converted/clip.mp4").exists());
        assert!(dir.join("Converted/clip (1).mp4").exists(), "the second result was clobbered");
        let written = std::fs::read_dir(dir.join("Converted")).unwrap().count();
        assert_eq!(written, 2, "one output per row");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The panic path, which no amount of care in `convert_one` can rule out: a worker that dies
    /// mid-row must still close that row, and the rows nobody reached must not spin forever.
    #[test]
    fn a_panicking_worker_still_closes_its_row_and_ends_the_batch() {
        let dir = sandbox("panic");
        let engine = engine_for(&dir);
        let mut items = vec![];
        for i in 0..2 {
            let input = dir.join(format!("c{i}.mov"));
            std::fs::write(&input, b"x").unwrap();
            items.push(BatchItem::file(format!("{i}"), input, "mp4"));
        }

        let events: Arc<Mutex<Vec<BatchEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded = events.clone();
        let sink: EventSink = Arc::new(move |e: BatchEvent| {
            let boom = matches!(&e, BatchEvent::Progress { id, .. } if id == "0");
            lock(&recorded).push(e);
            assert!(!boom, "simulated crash while reporting progress");
        });

        let mut settings = Settings::default();
        settings.output.parallel_jobs = 1; // one worker, so item 1 is never pulled
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {})); // the crash is the point; do not print it
        run_batch(items, settings, engine, Arc::new(AtomicBool::new(false)), sink);
        std::panic::set_hook(hook);

        let events = lock(&events).clone();
        let terminal: Vec<(&str, &str)> = events
            .iter()
            .filter_map(|e| match e {
                BatchEvent::Finished { id, .. } => Some((id.as_str(), "finished")),
                BatchEvent::Failed { id, message } => Some((id.as_str(), message.as_str())),
                BatchEvent::Skipped { id, reason } => Some((id.as_str(), reason.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(terminal.len(), 2, "every row ends exactly once: {events:#?}");
        assert_eq!(terminal[0].0, "0");
        assert!(terminal[0].1.contains("crashed"), "{:?}", terminal[0]);
        assert_eq!(terminal[1].0, "1");
        assert!(terminal[1].1.contains("stopped before"), "{:?}", terminal[1]);
        match events.last().unwrap() {
            BatchEvent::BatchFinished { ok, failed, skipped } => {
                assert_eq!((*ok, *failed, *skipped), (0, 1, 1));
            }
            other => panic!("batch_finished must be the last event, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two documents converting at the same time used to be handed LibreOffice's one shared user
    /// profile, and `soffice` refuses to start (or hangs) while another instance holds it - so a
    /// batch of documents failed rows at random. The stand-in below behaves the same way: it
    /// locks the profile it was pointed at and fails if that lock is already taken.
    ///
    /// This runs the real queue, so it also pins the two properties the argv assertions cannot:
    /// the profile really is unique per job, and the job's own scratch cleanup really removes it.
    #[test]
    fn two_document_jobs_at_once_do_not_fight_over_one_libreoffice_profile() {
        let dir = sandbox("soffice");
        let log = dir.join("profiles.log");
        let shared = dir.join("shared-profile"); // where an unconfigured soffice would go
        std::fs::create_dir_all(&shared).unwrap();
        let soffice = script(
            &dir,
            "soffice",
            &format!(
                r#"prof=""
outdir=""
prev=""
input=""
for a in "$@"; do
  case "$a" in -env:UserInstallation=*) prof="${{a#-env:UserInstallation=}}" ;; esac
  if [ "$prev" = "--outdir" ]; then outdir="$a"; fi
  prev="$a"
  input="$a"
done
echo "$prof" >> "{log}"
profile_dir="${{prof#file://}}"
[ -n "$profile_dir" ] || profile_dir="{shared}"
mkdir -p "$profile_dir"
mkdir "$profile_dir/.lock" 2>/dev/null || {{ echo "user profile is already in use" >&2; exit 1; }}
sleep 0.5
stem=$(basename "$input")
mkdir -p "$outdir"
printf 'pdf' > "$outdir/${{stem%.*}}.pdf"
rmdir "$profile_dir/.lock"
"#,
                log = log.display(),
                shared = shared.display()
            ),
        );

        let mut tools = ToolRegistry::default();
        tools.set(Tool::LibreOffice, soffice);
        let engine = Arc::new(Engine::new(tools));
        let mut items = vec![];
        for i in 0..2 {
            let input = dir.join(format!("report{i}.docx"));
            std::fs::write(&input, b"x").unwrap();
            items.push(BatchItem::file(format!("doc{i}"), input, "pdf"));
        }

        let events = Arc::new(Mutex::new(Vec::new()));
        let mut settings = Settings::default();
        settings.output.parallel_jobs = 2; // both documents in flight at the same time
        run_batch(items, settings, engine, Arc::new(AtomicBool::new(false)), collect(&events));

        let events = events.lock().unwrap();
        let finished = events.iter().filter(|e| matches!(e, BatchEvent::Finished { .. })).count();
        assert_eq!(finished, 2, "both documents must convert: {events:#?}");

        let used: Vec<String> =
            std::fs::read_to_string(&log).unwrap().lines().map(|l| l.trim().to_string()).collect();
        assert_eq!(used.len(), 2, "each job runs LibreOffice once: {used:?}");
        assert_ne!(used[0], used[1], "both jobs were given the same profile: {used:?}");
        let root = std::env::temp_dir().join("flint");
        for profile in &used {
            let path = PathBuf::from(
                profile.strip_prefix("file://").unwrap_or_else(|| panic!("not a URL: {profile}")),
            );
            assert!(path.starts_with(&root), "outside the job's scratch: {profile}");
            assert!(!path.exists(), "the profile outlived the job: {profile}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cancelled_batch_stops_pulling_work() {
        let dir = sandbox("cancelbatch");
        let engine = engine_for(&dir);
        let mut items = vec![];
        for i in 0..4 {
            let input = dir.join(format!("c{i}.mov"));
            std::fs::write(&input, b"x").unwrap();
            items.push(BatchItem::file(format!("{i}"), input, "mp4"));
        }
        let cancel = Arc::new(AtomicBool::new(true)); // cancelled before it starts
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut settings = Settings::default();
        settings.output.parallel_jobs = 1;
        run_batch(items, settings, engine, cancel, collect(&events));

        let events = events.lock().unwrap();
        assert!(!events.iter().any(|e| matches!(e, BatchEvent::Finished { .. })));
        match events.last().unwrap() {
            BatchEvent::BatchFinished { ok, .. } => assert_eq!(*ok, 0),
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------------------------
    // Trimming
    // -----------------------------------------------------------------------------------------

    /// An ffprobe that answers "a video `secs` long" for movie extensions and "one still frame" for
    /// everything else, so a mixed batch is planned the way a real one would be - a `.png` that came
    /// back as a 10 minute animation would be turned into a frame sequence and prove nothing.
    fn engine_probing(dir: &Path, secs: f64) -> Arc<Engine> {
        let ffprobe = script(
            dir,
            "ffprobe-duration",
            &format!(
                r#"input=""
for a in "$@"; do input="$a"; done
case "$input" in
  *.mov|*.mp4|*.mkv|*.webm|*.avi|*.m4a|*.mp3|*.wav)
    printf '{{"streams":[{{"codec_type":"video","codec_name":"h264","nb_frames":"300"}},{{"codec_type":"audio","codec_name":"aac"}}],"format":{{"duration":"{secs}"}}}}'
    ;;
  *)
    printf '{{"streams":[{{"codec_type":"video","codec_name":"png","nb_frames":"1"}}],"format":{{}}}}'
    ;;
esac
"#
            ),
        );
        let mut tools = ToolRegistry::default();
        tools.set(Tool::Ffmpeg, fake_ffmpeg(dir, "0"));
        tools.set(Tool::Ffprobe, ffprobe);
        Arc::new(Engine::new(tools))
    }

    fn trimming(dir: &Path, start: f64, length: f64) -> Settings {
        let mut settings = to_folder(dir);
        settings.trim = TrimSettings { enabled: true, start_secs: start, length_secs: length };
        settings
    }

    fn fractions(events: &[BatchEvent]) -> Vec<f32> {
        progress_phases(events).into_iter().filter_map(|(_, f)| f).collect()
    }

    /// The whole feature for a dropped file: the cut reaches the command line, and the bar is about
    /// the cut. Ten seconds of a ten minute film measured against the film would report 0.17% for
    /// the first sample and then jump to done - a bar that makes a working app look hung.
    #[test]
    fn a_trimmed_row_is_cut_on_the_command_line_and_measured_against_the_cut() {
        let dir = sandbox("trim-progress");
        let engine = engine_probing(&dir, 600.0);
        let input = dir.join("film.mov");
        std::fs::write(&input, b"x").unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            vec![BatchItem::file("1", input, "mp4")],
            trimming(&dir, 30.0, 10.0),
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        assert!(
            events.iter().any(|e| matches!(e, BatchEvent::Finished { .. })),
            "the trimmed clip must convert: {events:#?}"
        );
        let call = ffmpeg_call_for(&dir, "film.mp4");
        let ss = call.iter().position(|a| a == "-ss").unwrap_or_else(|| panic!("{call:?}"));
        assert_eq!(call[ss + 1], "30");
        assert_eq!(call[call.iter().position(|a| a == "-t").unwrap() + 1], "10");
        assert!(ss < call.iter().position(|a| a == "-i").unwrap(), "{call:?}");

        // The fake reports one second of output. That is 10% of the ten seconds being written, and
        // 0.17% of the film the trim is a slice of.
        let samples = fractions(&events);
        assert!(
            samples.iter().any(|f| (f - 0.1).abs() < 0.001),
            "one second of a ten second cut is 10%: {samples:?}"
        );
        assert!(
            !samples.iter().any(|f| *f > 0.0 && *f < 0.01),
            "a fraction of the untrimmed duration leaked into the bar: {samples:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// "If it is shorter than 10 seconds, keep it as it is." Nothing special happens: same argv, the
    /// row converts, and the bar is measured against the six seconds that exist rather than the ten
    /// that were asked for - so it still reaches 100% instead of stopping at 60%.
    #[test]
    fn a_clip_shorter_than_the_trim_keeps_its_own_length() {
        let dir = sandbox("trim-short");
        let engine = engine_probing(&dir, 6.0);
        let input = dir.join("short.mov");
        std::fs::write(&input, b"x").unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            vec![BatchItem::file("1", input, "mp4")],
            trimming(&dir, 0.0, 10.0),
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        assert!(
            !events.iter().any(|e| matches!(e, BatchEvent::Failed { .. })),
            "a short clip is not an error: {events:#?}"
        );
        assert!(events.iter().any(|e| matches!(e, BatchEvent::Finished { .. })), "{events:#?}");
        let call = ffmpeg_call_for(&dir, "short.mp4");
        assert_eq!(call[call.iter().position(|a| a == "-t").unwrap() + 1], "10");
        let samples = fractions(&events);
        assert!(
            samples.iter().any(|f| (f - 1.0 / 6.0).abs() < 0.001),
            "one second of a six second file is a sixth: {samples:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The one honest refusal. `-ss 30` on a twelve second clip makes FFmpeg write a valid, empty
    /// MP4, and an empty file reported as a success is the worst outcome available - so the row fails
    /// before anything is spawned, in a sentence that names both numbers.
    #[test]
    fn a_trim_that_starts_past_the_end_fails_the_row_before_anything_is_spawned() {
        let dir = sandbox("trim-past-end");
        let engine = engine_probing(&dir, 12.0);
        let input = dir.join("teaser.mov");
        std::fs::write(&input, b"x").unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            vec![BatchItem::file("1", input, "mp4")],
            trimming(&dir, 30.0, 10.0),
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        let message = events
            .iter()
            .find_map(|e| match e {
                BatchEvent::Failed { message, .. } => Some(message.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{events:#?}"));
        assert_eq!(
            message,
            "The trim starts at 0:30 and this file is only 0:12 long. Lower the start time, or \
             turn trimming off."
        );
        assert!(
            !dir.join("ffmpeg.log").exists(),
            "ffmpeg ran for a row that cannot produce a file"
        );
        assert!(!dir.join("out").join("teaser.mp4").exists());
        assert!(!events.iter().any(|e| matches!(e, BatchEvent::Finished { .. })), "{events:#?}");
        assert!(matches!(
            events.last().unwrap(),
            BatchEvent::BatchFinished { ok: 0, failed: 1, skipped: 0 }
        ));

        // Within the clip, the same trim converts: the refusal is about the start point, not about
        // asking for more seconds than the file has.
        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            vec![BatchItem::file("1", dir.join("teaser.mov"), "mp4")],
            trimming(&dir, 2.0, 10.0),
            engine_probing(&dir, 12.0),
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );
        assert!(
            events.lock().unwrap().iter().any(|e| matches!(e, BatchEvent::Finished { .. })),
            "{:#?}",
            events.lock().unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The batch the feature has to survive: ten clips and one of everything else. A trim is
    /// meaningless for a still image, a document and a subtitle file, so those rows are converted
    /// exactly as they would have been - silently ignored, never refused.
    #[test]
    fn a_mixed_batch_trims_the_clips_and_leaves_everything_else_alone() {
        let dir = sandbox("trim-mixed");
        let soffice_log = dir.join("soffice.log");
        let soffice = script(
            &dir,
            "soffice",
            &format!(
                r#"for a in "$@"; do printf '%s\n' "$a" >> "{log}"; done
outdir=""
prev=""
input=""
for a in "$@"; do
  if [ "$prev" = "--outdir" ]; then outdir="$a"; fi
  prev="$a"
  input="$a"
done
mkdir -p "$outdir"
stem=$(basename "$input")
printf 'pdf' > "$outdir/${{stem%.*}}.pdf"
"#,
                log = soffice_log.display()
            ),
        );
        let engine = engine_probing(&dir, 600.0);
        let mut tools = engine.tools.clone();
        tools.set(Tool::LibreOffice, soffice);
        let engine = Arc::new(Engine::new(tools));

        for name in ["clip.mov", "photo.png", "notes.docx", "captions.srt"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let items = vec![
            BatchItem::file("clip", dir.join("clip.mov"), "mp4"),
            BatchItem::file("photo", dir.join("photo.png"), "jpg"),
            BatchItem::file("doc", dir.join("notes.docx"), "pdf"),
            BatchItem::file("subs", dir.join("captions.srt"), "vtt"),
        ];

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            items,
            trimming(&dir, 30.0, 10.0),
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        let finished: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                BatchEvent::Finished { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(finished.len(), 4, "every row must convert: {events:#?}");

        // Only the clip was cut.
        let clip = ffmpeg_call_for(&dir, "clip.mp4");
        assert!(clip.contains(&"-ss".to_string()) && clip.contains(&"-t".to_string()), "{clip:?}");
        for name in ["photo.jpg", "captions.vtt"] {
            let call = ffmpeg_call_for(&dir, name);
            assert!(!call.iter().any(|a| a == "-ss" || a == "-t"), "{name} was trimmed: {call:?}");
        }
        let office: Vec<String> =
            std::fs::read_to_string(&soffice_log).unwrap().lines().map(str::to_string).collect();
        assert!(!office.is_empty(), "the document row never ran LibreOffice");
        assert!(!office.iter().any(|a| a == "-ss" || a == "-t"), "{office:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pasted link is fetched whole and cut *during the conversion*, by the same planner and the
    /// same queue as a dropped file - there is no second implementation, and no
    /// `--download-sections` for the two to disagree about. The duration the trim is measured
    /// against can come from yt-dlp's own metadata (this fake reports 123 seconds), and the two
    /// phases still read as themselves.
    #[test]
    fn a_trimmed_link_is_cut_during_the_conversion_not_the_download() {
        let dir = sandbox("trim-link");
        let (engine, _) = engine_with_yt_dlp(&dir, "A Talk About Rust", "0");

        let events = Arc::new(Mutex::new(Vec::new()));
        run_batch(
            vec![BatchItem::link("1", parsed("https://youtu.be/abc123"), "mp4")],
            trimming(&dir, 5.0, 10.0),
            engine,
            Arc::new(AtomicBool::new(false)),
            collect(&events),
        );

        let events = events.lock().unwrap();
        let output = events
            .iter()
            .find_map(|e| match e {
                BatchEvent::Finished { outputs, .. } => outputs.first().cloned(),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the link should convert: {events:#?}"));
        assert_eq!(output, dir.join("out").join("A Talk About Rust.mp4"));

        // The conversion carries the trim...
        let call = ffmpeg_call_for(&dir, "A Talk About Rust.mp4");
        let ss = call.iter().position(|a| a == "-ss").unwrap_or_else(|| panic!("{call:?}"));
        assert_eq!(call[ss + 1], "5");
        assert_eq!(call[call.iter().position(|a| a == "-t").unwrap() + 1], "10");
        assert!(ss < call.iter().position(|a| a == "-i").unwrap(), "{call:?}");
        // ...and the download does not: fetching the whole video cannot desync from the cut.
        for fetch in yt_dlp_calls(&dir) {
            assert!(
                !fetch.iter().any(|a| a.starts_with("--download-sections")),
                "the fetch must stay a plain download: {fetch:?}"
            );
        }

        // Both phases still report, download first, and the converting samples are measured against
        // the ten second cut of a video yt-dlp said was 123 seconds long.
        let phases = progress_phases(&events);
        let last_download = phases
            .iter()
            .rposition(|(p, _)| *p == Phase::Downloading)
            .unwrap_or_else(|| panic!("{phases:?}"));
        let first_convert = phases
            .iter()
            .position(|(p, _)| *p == Phase::Converting)
            .unwrap_or_else(|| panic!("{phases:?}"));
        assert!(last_download < first_convert, "{phases:?}");
        let converting: Vec<f32> = phases
            .iter()
            .filter(|(p, _)| *p == Phase::Converting)
            .filter_map(|(_, f)| *f)
            .collect();
        assert!(
            converting.iter().any(|f| (f - 0.1).abs() < 0.001),
            "one second of a ten second cut is 10%: {converting:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
