//! Executing a [`Plan`]: spawn the tool, stream progress, honour cancellation, tidy up.

use crate::format::{Tool, JS_RUNTIMES};
use crate::link::{self, CookieProbe, FetchFailure, FetchOptions, JsRuntime, Link, LinkInfo};
use crate::paths::{sequence_contains, sequence_index};
use crate::plan::{Plan, PostAction, Step};
use crate::probe::{ffprobe_args, parse_ffprobe_json, MediaInfo};
use crate::progress::{eta_secs, overall_fraction, step_fraction, Phase, ProgressParser};
use crate::settings::{CookieFlag, OutputSettings};
use crate::tools::ToolRegistry;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{0}")]
    Selection(String),
    #[error("cancelled")]
    Cancelled,
    #[error("The metadata check timed out. Check the source and try again.")]
    ProbeTimedOut,
    #[error("{tool} exited with status {code}{}", if stderr.is_empty() { String::new() } else { format!(": {stderr}") })]
    ToolFailed { tool: String, code: String, stderr: String },
    #[error("could not launch {program}: {source}")]
    Spawn { program: String, source: std::io::Error },
    #[error("the conversion produced no output file")]
    NoOutput,
    /// A link could not be fetched. Its own type because the *reasons* are different in kind from
    /// a converter failing - "this video is private" is not an exit code - and the message the user
    /// reads is written in [`crate::link`], next to the stderr pattern that produced it.
    #[error("{0}")]
    Fetch(#[from] FetchFailure),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// What a finished job produced.
#[derive(Debug, Clone, Default)]
pub struct JobOutcome {
    pub outputs: Vec<PathBuf>,
    pub bytes: u64,
    pub elapsed_ms: u128,
}

/// Progress callback payload.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProgressUpdate {
    pub fraction: Option<f32>,
    pub speed: Option<f64>,
    pub eta_secs: Option<f64>,
    /// Which half of the job this is about. Always [`Phase::Converting`] for a dropped file; a link
    /// reports [`Phase::Downloading`] first and then converts like anything else.
    pub phase: Phase,
}

pub struct Engine {
    pub tools: ToolRegistry,
}

impl Engine {
    pub fn new(tools: ToolRegistry) -> Self {
        Self { tools }
    }

    /// Run `ffprobe` and parse the result. Returns `None` when ffprobe is unavailable or the file
    /// is not media (documents), which is not an error - the planner simply gets less information.
    pub fn probe(&self, input: &Path) -> Option<MediaInfo> {
        self.probe_cancellable(input, &Arc::new(AtomicBool::new(false)))
    }

    pub fn probe_cancellable(&self, input: &Path, cancel: &Arc<AtomicBool>) -> Option<MediaInfo> {
        if crate::format::by_path(input).is_some_and(|f| f.id == "midi") {
            return crate::midi::probe(input, cancel);
        }
        let program = self.tools.path(Tool::Ffprobe)?;
        let mut output = String::new();
        let result = run_probe(
            program,
            &ffprobe_args(input),
            None,
            cancel,
            Duration::from_secs(30),
            &mut |line| {
                if output.len() + line.len() < 1024 * 1024 {
                    output.push_str(line);
                    output.push('\n');
                }
            },
        )
        .ok()?;
        if !matches!(result, ProcOutcome::Ok) {
            return None;
        }
        parse_ffprobe_json(&output).ok()
    }

    /// Execute every step of a plan.
    ///
    /// Either the job succeeds and every file it wrote is listed in the outcome, or it fails and
    /// nothing it wrote survives: a cancelled encode used to leave a truncated `clip.mp4` sitting
    /// in the destination folder, indistinguishable from a finished conversion.
    pub fn run(
        &self,
        plan: &Plan,
        duration_secs: Option<f64>,
        cancel: &Arc<AtomicBool>,
        on_progress: &mut dyn FnMut(ProgressUpdate),
    ) -> Result<JobOutcome, EngineError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled);
        }
        let started = Instant::now();
        let mut originals = OutputRollback::capture(plan)?;
        let mut sequence: Vec<PathBuf> = Vec::new();
        // A file that is already there belongs to the user (or to the earlier run the conflict
        // policy told us to overwrite), so it must survive a failure. Anything this job creates is
        // ours to take back.
        let output_existed = plan.output.exists();

        let outcome = (|| {
            if let Some((input, wav)) = &plan.midi {
                if let Some(parent) = wav.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                crate::midi::render(input, wav, cancel, &mut |fraction| {
                    on_progress(ProgressUpdate {
                        fraction: Some(fraction * 0.25),
                        speed: None,
                        eta_secs: None,
                        phase: Phase::Converting,
                    });
                })?;
            }
            self.run_steps(
                plan,
                duration_secs,
                cancel,
                &mut |mut update| {
                    if plan.midi.is_some() {
                        update.fraction = update.fraction.map(|f| 0.25 + 0.75 * f);
                    }
                    on_progress(update);
                },
                &mut sequence,
            )
        })();
        let outputs = match outcome {
            Ok(()) => {
                let outputs = if sequence.is_empty() {
                    collect_single(plan)
                } else {
                    std::mem::take(&mut sequence)
                };
                if outputs.is_empty() {
                    remove_temp_paths(plan);
                    originals.restore()?;
                    return Err(EngineError::NoOutput);
                }
                outputs
            }
            Err(e) => {
                discard_partial_results(plan, output_existed, &sequence);
                remove_temp_paths(plan);
                originals.restore()?;
                return Err(e);
            }
        };

        let bytes = outputs.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        on_progress(ProgressUpdate {
            fraction: Some(1.0),
            speed: None,
            eta_secs: Some(0.0),
            phase: Phase::Converting,
        });
        remove_temp_paths(plan);
        originals.commit();
        Ok(JobOutcome { outputs, bytes, elapsed_ms: started.elapsed().as_millis() })
    }

    fn run_steps(
        &self,
        plan: &Plan,
        duration_secs: Option<f64>,
        cancel: &Arc<AtomicBool>,
        on_progress: &mut dyn FnMut(ProgressUpdate),
        sequence: &mut Vec<PathBuf>,
    ) -> Result<(), EngineError> {
        let weights: Vec<f32> = plan.steps.iter().map(|s| s.weight).collect();
        for (index, step) in plan.steps.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(EngineError::Cancelled);
            }
            prepare_dirs(step, plan)?;
            // Snapshot before running so a sequence can be told apart from files that were
            // already sitting in the destination folder (an older `clip-2.png`, or the output of
            // a different job that happens to share the prefix).
            let before = match &step.post {
                Some(PostAction::CollectSequence { dir, .. }) => dir_snapshot(dir),
                _ => DirSnapshot::new(),
            };
            let result = self
                .run_step(step, index, &weights, duration_secs, cancel, on_progress)
                .and_then(|()| match &step.post {
                    Some(post) => apply_post(post, plan, &before, sequence),
                    None => Ok(()),
                });
            if let Err(e) = result {
                // A rasteriser that dies on page 7 has already written six files; they are this
                // job's, they are incomplete, and the post action never got to record them.
                if let Some(PostAction::CollectSequence { dir, prefix, extension }) = &step.post {
                    for (_, path) in new_sequence_files(dir, prefix, extension, &before) {
                        let _ = std::fs::remove_file(path);
                    }
                }
                return Err(e);
            }
        }
        Ok(())
    }

    fn run_step(
        &self,
        step: &Step,
        index: usize,
        weights: &[f32],
        duration_secs: Option<f64>,
        cancel: &Arc<AtomicBool>,
        on_progress: &mut dyn FnMut(ProgressUpdate),
    ) -> Result<(), EngineError> {
        // The closure borrows `on_progress`, so it has to be gone before the reports below.
        let outcome = {
            let mut parser = ProgressParser::new();
            let mut on_line = |line: &str| {
                if !step.ffmpeg_progress {
                    return;
                }
                if let Some(sample) = parser.push_line(line) {
                    // No duration, no number: [`step_fraction`] answers `None` on purpose, and that
                    // `None` is carried all the way to the UI, which renders "Converting…" with no
                    // percentage. Substituting a fraction here (the previous `or_else`, which
                    // reported the *start* of this step) pinned the bar at 0% for the whole job and
                    // said "Converting 0%" while a file was demonstrably being written - a working
                    // conversion that reads as a hang.
                    let inner = step_fraction(sample.out_time_secs, duration_secs);
                    on_progress(ProgressUpdate {
                        fraction: inner.map(|f| overall_fraction(index, weights, f)),
                        speed: sample.speed,
                        eta_secs: eta_secs(sample.out_time_secs, duration_secs, sample.speed),
                        phase: Phase::Converting,
                    });
                }
            };
            // No `PATH` of our own: a step's program is absolute and looks nothing up.
            run_child(&step.program, &step.args, None, cancel, &mut on_line)?
        };

        if let ProcOutcome::Failed { code, stderr } = outcome {
            return Err(EngineError::ToolFailed {
                tool: step.tool.label().to_string(),
                code,
                stderr: last_meaningful_line(&stderr),
            });
        }
        on_progress(ProgressUpdate {
            fraction: Some(overall_fraction(index, weights, 1.0)),
            speed: None,
            eta_secs: None,
            phase: Phase::Converting,
        });
        Ok(())
    }

    /// Ask yt-dlp what a link *is* - title and duration - without downloading anything.
    ///
    /// Done before the fetch because both answers are needed before it: the title names the output
    /// file (and therefore decides the conflict policy's question), and the duration is what makes
    /// the convert phase's progress bar a number rather than a spinner.
    ///
    /// The cookie source is passed in rather than read here, because this layer holds no `Settings`.
    /// It has to be passed at all because a members-only page refuses the probe just as flatly as
    /// the download, so a probe without the cookies would fail a link the fetch could have had.
    pub fn probe_link(
        &self,
        link: &Link,
        cookies: Option<&CookieFlag>,
    ) -> Result<LinkInfo, FetchFailure> {
        self.probe_link_cancellable(link, cookies, &Arc::new(AtomicBool::new(false))).map_err(|e| {
            match e {
                EngineError::Fetch(failure) => failure,
                other => FetchFailure::Other(other.to_string()),
            }
        })
    }

    pub fn probe_link_cancellable(
        &self,
        link: &Link,
        cookies: Option<&CookieFlag>,
        cancel: &Arc<AtomicBool>,
    ) -> Result<LinkInfo, EngineError> {
        let program = self.tools.path(Tool::YtDlp).ok_or(FetchFailure::NotInstalled)?;
        let runtime = self.js_runtime();
        let mut output = String::new();
        let result = run_probe(
            program,
            &link::probe_args(link, runtime.as_ref(), cookies),
            Some(&fetch_path(runtime.as_ref())),
            cancel,
            Duration::from_secs(30),
            &mut |line| {
                output.push_str(line);
                output.push('\n');
            },
        )?;
        if let ProcOutcome::Failed { stderr, .. } = result {
            return Err(link::classify_failure(&stderr, runtime.as_ref(), cookies).into());
        }
        if link.site().category() == crate::format::Category::Audio {
            link::parse_music_probe(&output).map_err(EngineError::Selection)
        } else {
            Ok(link::parse_probe_output(&output))
        }
    }

    /// Answer "will the sign-in you configured actually work?" with one throwaway probe.
    ///
    /// The same command line a real link gets ([`link::probe_args`]) against
    /// [`link::COOKIE_TEST_URL`], and the same classification ([`link::classify_failure`]): a check
    /// that ran its own command line, or wrote its own sentences, would drift from the fetch it is
    /// supposed to predict. `--skip-download` is already in those arguments, so nothing is
    /// downloaded and nothing is written anywhere.
    ///
    /// Nothing of the cookie jar comes back out. The probe's stdout is the video's title and
    /// duration and is dropped on the floor here rather than parsed, stored or logged; the only
    /// things this returns are a verdict and one of the sentences `classify_failure` already
    /// produces. Neither command line ever asks yt-dlp for `--verbose`, which is the only channel
    /// that would print a cookie value at all (pinned by
    /// `link::tests::no_cookie_state_ever_asks_yt_dlp_to_be_verbose`).
    ///
    /// One honest limitation, inherited from [`link::probe_args`]: it passes `--no-warnings`, and a
    /// refused Keychain prompt is a *warning*. On that one path the site sees undecrypted cookies
    /// and answers with its sign-in wall, so the check reports [`link::CookieCheck::Refused`] where
    /// a fetch would have said the Keychain prompt was refused.
    ///
    /// A second one, and the right answer rather than a limitation: on a machine with no JavaScript
    /// runtime, YouTube's bot check is [`FetchFailure::NoJsRuntime`] and the verdict is
    /// [`link::CookieCheck::Inconclusive`]. Nothing was learned about the cookies, because the
    /// thing that stopped the page is not the cookies.
    ///
    /// `timeout` is a budget, not a deadline for the network: when it runs out the child is killed
    /// by the same process-group kill Stop uses, so nothing is left running behind the drawer.
    pub fn check_cookie_source(
        &self,
        link: &Link,
        cookies: Option<&CookieFlag>,
        timeout: Duration,
    ) -> CookieProbe {
        let Some(program) = self.tools.path(Tool::YtDlp).map(Path::to_path_buf) else {
            return CookieProbe::Failed(FetchFailure::NotInstalled);
        };
        let runtime = self.js_runtime();
        let args = link::probe_args(link, runtime.as_ref(), cookies);
        let path = fetch_path(runtime.as_ref());

        let cancel = Arc::new(AtomicBool::new(false));
        // The title and duration this prints are of no interest to a sign-in check, and are read
        // only to keep the pipe draining.
        let mut metadata = String::new();
        let mut on_line = |line: &str| {
            if metadata.len() + line.len() < 1024 * 1024 {
                metadata.push_str(line);
                metadata.push('\n');
            }
        };
        let outcome = run_probe(&program, &args, Some(&path), &cancel, timeout, &mut on_line);

        match outcome {
            Ok(ProcOutcome::Ok) if link.site().category() == crate::format::Category::Audio => {
                match link::parse_music_probe(&metadata) {
                    Ok(_) => CookieProbe::Worked,
                    Err(message) => CookieProbe::Failed(FetchFailure::Other(message)),
                }
            }
            Ok(ProcOutcome::Ok) => CookieProbe::Worked,
            Ok(ProcOutcome::Failed { stderr, .. }) => {
                CookieProbe::Failed(link::classify_failure(&stderr, runtime.as_ref(), cookies))
            }
            Err(EngineError::ProbeTimedOut) => CookieProbe::TimedOut,
            Err(e) => {
                CookieProbe::Failed(FetchFailure::Other(format!("could not start yt-dlp: {e}")))
            }
        }
    }

    /// The JavaScript runtime a fetch will hand yt-dlp, or `None` when this machine has none.
    ///
    /// yt-dlp's own order of preference ([`JS_RUNTIMES`]) decided by the registry's existing
    /// "first tool from a preference list" lookup, so the runtime we name is the one yt-dlp would
    /// have chosen if its `PATH` had contained ours.
    pub fn js_runtime(&self) -> Option<JsRuntime> {
        let tool = self.tools.first_available(JS_RUNTIMES)?;
        JsRuntime::new(tool, self.tools.path(tool)?.to_path_buf())
    }

    /// Fetch a link into `options.dir`, reporting [`Phase::Downloading`] as it goes.
    ///
    /// Cancellation goes through exactly the same process-group kill as a conversion step (see
    /// [`run_child`]), so Stop reaches yt-dlp *and* the ffmpeg it may have started to merge
    /// streams. Whatever the outcome, a fetch that did not succeed leaves nothing behind: the whole
    /// scratch directory goes, so there is no half-downloaded file for the next run - or the user -
    /// to mistake for a result.
    pub fn fetch_link(
        &self,
        link: &Link,
        options: &FetchOptions,
        cancel: &Arc<AtomicBool>,
        on_progress: &mut dyn FnMut(ProgressUpdate),
    ) -> Result<PathBuf, EngineError> {
        let Some(program) = self.tools.path(Tool::YtDlp).map(Path::to_path_buf) else {
            return Err(FetchFailure::NotInstalled.into());
        };
        std::fs::create_dir_all(&options.dir)?;
        let args = link::fetch_args(link, options);
        let path = fetch_path(options.js_runtime.as_ref());

        let outcome = {
            let mut on_line = |line: &str| {
                if let Some(sample) = link::parse_download_line(line) {
                    on_progress(ProgressUpdate {
                        fraction: Some(sample.fraction),
                        speed: None,
                        eta_secs: sample.eta_secs,
                        phase: Phase::Downloading,
                    });
                }
            };
            run_child(&program, &args, Some(&path), cancel, &mut on_line)
        };

        let failure = match outcome {
            Ok(ProcOutcome::Ok) => match link::downloaded_file(&options.dir) {
                Some(file) => return Ok(file),
                None => EngineError::Fetch(FetchFailure::NoFile),
            },
            Ok(ProcOutcome::Failed { stderr, .. }) => EngineError::Fetch(link::classify_failure(
                &stderr,
                options.js_runtime.as_ref(),
                options.cookies.as_ref(),
            )),
            Err(e) => e,
        };
        // Cancelled, refused or empty-handed: the scratch is ours and none of it is a result.
        let _ = std::fs::remove_dir_all(&options.dir);
        Err(failure)
    }
}

/// How a child process ended, when it ended on its own terms.
///
/// A non-zero exit is *not* an [`EngineError`] here: the two callers need different things from it
/// (a converter's exit code and last stderr line; yt-dlp's whole stderr, which is what
/// [`crate::link::classify_failure`] reads), so the runner hands both back and neither guesses.
pub(crate) enum ProcOutcome {
    Ok,
    Failed { code: String, stderr: String },
}

/// The `PATH` a link fetch's yt-dlp runs with: the runtime's own directory first, then the
/// directories discovery knows about, then whatever this process inherited.
///
/// A backstop, not the mechanism. The runtime is named by absolute path in `--js-runtimes` and our
/// FFmpeg in `--ffmpeg-location`, so neither depends on this - but yt-dlp shells out to more than
/// it is told about, and a Finder-launched app hands a child `/usr/bin:/bin:/usr/sbin:/sbin`, where
/// nothing installed by Homebrew (or by Deno's own installer) can be found.
fn fetch_path(runtime: Option<&JsRuntime>) -> std::ffi::OsString {
    let extra: Vec<PathBuf> =
        runtime.and_then(JsRuntime::dir).map(Path::to_path_buf).into_iter().collect();
    crate::tools::child_path(&extra)
}

/// Existing outputs belong to the user until every step succeeds. Keep copies on
/// their destination volume so restoring a cancelled overwrite is an atomic rename.
#[derive(Default)]
struct OutputRollback {
    originals: Vec<(PathBuf, tempfile::NamedTempFile)>,
}

impl OutputRollback {
    fn capture(plan: &Plan) -> Result<Self, EngineError> {
        let mut paths = std::collections::HashSet::new();
        if plan.output.is_file() {
            paths.insert(plan.output.clone());
        }
        let destination = plan.output.parent().unwrap_or(Path::new("."));
        for step in &plan.steps {
            if let Some(PostAction::CollectSequence { extension, .. }) = &step.post {
                let family = plan.output.with_extension(extension);
                match std::fs::read_dir(destination) {
                    Ok(entries) => {
                        for entry in entries {
                            let path = entry?.path();
                            if path.is_file() && sequence_contains(&family, &path) {
                                paths.insert(path);
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        let mut originals = Vec::new();
        for path in paths {
            let backup = tempfile::Builder::new()
                .prefix(".cc-backup-")
                .tempfile_in(path.parent().unwrap_or(Path::new(".")))?;
            std::fs::copy(&path, backup.path())?;
            if let Ok(modified) = std::fs::metadata(&path)?.modified() {
                backup.as_file().set_modified(modified)?;
            }
            originals.push((path, backup));
        }
        Ok(Self { originals })
    }

    fn commit(&mut self) {
        self.originals.clear();
    }

    fn restore(&mut self) -> Result<(), EngineError> {
        let mut errors = Vec::new();
        for (path, backup) in self.originals.drain(..) {
            if let Err(error) = backup.persist(&path) {
                let why = error.error.to_string();
                // Do not delete the recovery copy if the destination became unavailable.
                let recovery = error
                    .file
                    .into_temp_path()
                    .keep()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|e| e.to_string());
                errors.push(format!("{}: {why}. Recovery copy: {recovery}", path.display()));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(EngineError::Selection(format!(
                "Could not restore an earlier result. {}",
                errors.join("; ")
            )))
        }
    }
}

impl Drop for OutputRollback {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!("[flint] {error}");
        }
    }
}

/// Metadata probes have a deadline as well as Stop. Dropping `done` wakes the watchdog even
/// during unwinding; a timeout only cancels this child, never the rest of the batch.
pub(crate) fn run_probe(
    program: &Path,
    args: &[String],
    path: Option<&std::ffi::OsStr>,
    cancel: &Arc<AtomicBool>,
    timeout: Duration,
    on_line: &mut dyn FnMut(&str),
) -> Result<ProcOutcome, EngineError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }
    let local = Arc::new(AtomicBool::new(false));
    let (done, finished) = std::sync::mpsc::channel::<()>();
    let result = std::thread::scope(|scope| {
        let local = &local;
        scope.spawn(move || {
            let deadline = Instant::now() + timeout;
            loop {
                if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
                    local.store(true, Ordering::Relaxed);
                    return;
                }
                if !matches!(
                    finished.recv_timeout(Duration::from_millis(20)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ) {
                    return;
                }
            }
        });
        let result = run_child(program, args, path, local, on_line);
        drop(done);
        result
    });
    match result {
        Err(EngineError::Cancelled) if !cancel.load(Ordering::Relaxed) => {
            Err(EngineError::ProbeTimedOut)
        }
        other => other,
    }
}

/// Spawn a tool, stream its stdout line by line to `on_line`, and honour Stop throughout.
///
/// Shared by every conversion step and by the link fetch, which is the point: cancellation,
/// process-group kill, the pipe-deadlock avoidance and the "tool closed stdout but is still
/// running" case are all subtle, all already right here, and must not be reimplemented for a
/// second kind of child.
///
/// `child_path` replaces the child's `PATH` when it is given. Conversion steps pass `None`: every
/// one of them is a single absolute program that looks nothing up, and a converter's environment is
/// not the place to start experimenting. The fetch passes one (see [`fetch_path`]).
fn run_child(
    program: &Path,
    args: &[String],
    child_path: Option<&std::ffi::OsStr>,
    cancel: &Arc<AtomicBool>,
    on_line: &mut dyn FnMut(&str),
) -> Result<ProcOutcome, EngineError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }
    let mut child = RunningChild::new(spawn_with_retry(program, args, child_path)?);

    // Drain stderr on a helper thread: a full pipe would otherwise deadlock the child.
    let stderr = child.stderr();
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut s) = stderr {
            let _ = s.read_to_string(&mut buf);
        }
        buf
    });

    // Progress is read on its own thread and delivered over a channel. Polling the channel
    // (instead of blocking on the pipe) is what makes Stop feel instant even when a tool has
    // gone quiet - which happens constantly with long GOPs, two-pass encodes and LibreOffice.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let stdout_thread = child.stdout().map(|stdout| {
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        })
    });

    let mut cancelled = false;
    loop {
        if cancel.load(Ordering::Relaxed) {
            child.kill_group();
            cancelled = true;
            break;
        }
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(line) => on_line(&line),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // No output for a while: the tool may simply be done (or silent, like sips).
                if child.has_exited() {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    // The loop above leaves either an exited child or one that was just killed - but it also
    // leaves on `Disconnected`, i.e. a tool that closed stdout while still running. Waiting on
    // that one has to keep watching the flag, or Stop would do nothing until it felt like
    // exiting. Both pipes are closing by now, so the readers are collected here rather than
    // left detached - with a budget, because a grandchild that survived the kill can hold them
    // open indefinitely (see `join_within`).
    let (status, killed_while_waiting) = child.wait_watching(cancel)?;
    let cancelled = cancelled || killed_while_waiting;
    drop(child); // the child is reaped; nothing left to kill
    let budget = std::time::Duration::from_millis(if cancelled { 200 } else { 2000 });
    if let Some(t) = stdout_thread {
        join_within(t, budget);
    }
    if cancelled || cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }
    // `try_wait` regularly observes the exit before the reader has forwarded the last block;
    // without this drain the final `progress=end` - and with it the 100% update - was dropped.
    while let Ok(line) = rx.try_recv() {
        on_line(&line);
    }
    let stderr_text = join_within(stderr_thread, budget).unwrap_or_default();

    if !status.success() {
        return Ok(ProcOutcome::Failed {
            code: status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into()),
            stderr: stderr_text,
        });
    }
    Ok(ProcOutcome::Ok)
}

/// A running tool, killed and reaped by `Drop` if the step is left without waiting for it.
///
/// The step can be abandoned by an unwind - a progress callback that panics, for instance - and a
/// plain `Child` neither kills nor reaps on drop: FFmpeg carried on writing to a file nobody was
/// waiting for any more, and the process stayed until the app quit.
struct RunningChild {
    child: std::process::Child,
    pid: u32,
    reaped: bool,
}

impl RunningChild {
    fn new(child: std::process::Child) -> Self {
        let pid = child.id();
        Self { child, pid, reaped: false }
    }

    fn stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.child.stderr.take()
    }

    fn stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.stdout.take()
    }

    fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Kill the tool and everything it spawned, by group and then directly.
    fn kill_group(&mut self) {
        kill_pid(self.pid);
        let _ = self.child.kill();
    }

    /// Wait for the tool to exit while still honouring Stop, reporting whether it had to be killed.
    ///
    /// `Child::wait` blocks with nothing watching the flag. A tool that closes stdout but keeps
    /// running (or one whose grandchild does the real work) parks the step there, and Stop only
    /// took effect whenever the tool chose to exit - which for a stuck helper is never.
    fn wait_watching(
        &mut self,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(std::process::ExitStatus, bool), EngineError> {
        let mut killed = false;
        loop {
            if !killed && cancel.load(Ordering::Relaxed) {
                self.kill_group();
                killed = true;
            }
            if let Some(status) = self.child.try_wait()? {
                self.reaped = true;
                return Ok((status, killed));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

impl Drop for RunningChild {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        self.kill_group();
        let _ = self.child.wait();
    }
}

/// Spawn a tool, retrying `ETXTBSY` - a freshly written sidecar can still be "busy" for a few
/// milliseconds if another thread forked while the file was open.
fn spawn_with_retry(
    program: &Path,
    args: &[String],
    child_path: Option<&std::ffi::OsStr>,
) -> Result<std::process::Child, EngineError> {
    let mut last: Option<std::io::Error> = None;
    for attempt in 0..6 {
        let mut cmd = Command::new(program);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(path) = child_path {
            cmd.env("PATH", path);
        }
        // Own process group, so cancelling kills the tool *and* anything it spawned
        // (LibreOffice starts a background soffice.bin that would otherwise keep the pipe open;
        // yt-dlp starts an ffmpeg of its own to merge streams).
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        match cmd.spawn() {
            Ok(child) => return Ok(child),
            Err(e) => {
                let busy = e.raw_os_error() == Some(26); // ETXTBSY
                last = Some(e);
                if !busy {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20 * (attempt + 1)));
            }
        }
    }
    Err(EngineError::Spawn {
        program: program.display().to_string(),
        source: last.unwrap_or_else(|| std::io::Error::other("spawn failed")),
    })
}

/// Arguments for killing the whole process group led by `pid`.
///
/// The `--` is not decoration: without it `kill` parses `-1234` as *options*, quietly exits 0 and
/// kills nothing - which left every grandchild (FFmpeg's own workers, `soffice.bin`) alive after a
/// Stop, holding the pipes open. Worse, a short pid can be read as `-1`, i.e. "every process you
/// own". `kill -9 -- -1234` is the only spelling that reliably means "signal process group 1234".
#[cfg(unix)]
fn kill_group_args(pid: u32) -> [String; 3] {
    ["-9".into(), "--".into(), format!("-{pid}")]
}

/// Kill by pid so the watchdog does not need the `Child` handle (which the reader thread owns).
fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        // Negative pid = the whole process group we created in `spawn_with_retry`.
        let _ = Command::new("kill").args(kill_group_args(pid)).output();
        let _ = Command::new("kill").args(["-9", "--", &pid.to_string()]).output();
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .creation_flags(0x08000000)
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}

/// Join a reader thread, but only if it can finish promptly.
///
/// A tool can leave a grandchild holding its pipes open even after being killed, and a read on
/// that pipe blocks until *every* writer is gone. Waiting for that would freeze a cancelled job
/// for as long as the runaway process lives, so past the budget the thread is abandoned: it owns
/// nothing but its own buffer and the read end of a pipe, both released when it finally sees EOF.
fn join_within<T>(handle: std::thread::JoinHandle<T>, budget: std::time::Duration) -> Option<T> {
    let deadline = Instant::now() + budget;
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    handle.join().ok()
}

/// Tools do not create their own directories, and only the planner knows which of a step's
/// arguments is a directory - so it declares them and we create exactly those, plus the folder the
/// final output lives in.
fn prepare_dirs(step: &Step, plan: &Plan) -> Result<(), EngineError> {
    if let Some(parent) = plan.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    for dir in &step.ensure_dirs {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

/// What the destination directory held before a step ran: name -> (size, modified).
///
/// Names alone are not enough. A second run of the same frame extraction *rewrites* `clip-0001.png`
/// instead of adding a file, so a name-only snapshot classified every result as "somebody else's":
/// the job reported "produced no output file" (or listed only the frames the first run had not
/// reached) while having quietly replaced the earlier ones.
type DirSnapshot =
    std::collections::HashMap<std::ffi::OsString, (u64, Option<std::time::SystemTime>)>;

/// Files currently in `dir`, with the stamps that reveal a rewrite. Missing directories are empty.
fn dir_snapshot(dir: &Path) -> DirSnapshot {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            let stamp = e.metadata().map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None));
            (e.file_name(), stamp)
        })
        .collect()
}

/// The `<prefix>-<number>.<extension>` files a step just wrote in `dir`, in page/frame order.
///
/// Sorted numerically rather than lexically: `clip-10.png` must not come before `clip-2.png`
/// (pdftoppm only zero-pads when the page count needs it). "Just wrote" means either a name that
/// was not there before, or one that was there and has since changed - the second run of a job
/// overwrites its own frames, and those results are still results.
fn new_sequence_files(
    dir: &Path,
    prefix: &str,
    extension: &str,
    before: &DirSnapshot,
) -> Vec<(u64, PathBuf)> {
    let mut found: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            match before.get(&e.file_name()) {
                // Untouched leftovers are somebody else's: a stale file from an older run that this
                // one did not reach, or another job writing into the same folder.
                Some(was) => {
                    let now =
                        e.metadata().map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None));
                    now != *was
                }
                None => true,
            }
        })
        .filter_map(|e| {
            let index = sequence_index(&e.file_name().to_string_lossy(), prefix, extension)?;
            Some((index, e.path()))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    found
}

/// Take back everything a failed or cancelled job wrote, so no half-finished file is left looking
/// like a result. Errors are ignored on purpose: this runs while another error is on its way up.
fn discard_partial_results(plan: &Plan, output_existed: bool, sequence: &[PathBuf]) {
    for path in sequence {
        let _ = std::fs::remove_file(path);
    }
    if !output_existed {
        let _ = std::fs::remove_file(&plan.output);
    }
}

fn remove_temp_paths(plan: &Plan) {
    for temp in &plan.temp_paths {
        let _ =
            if temp.is_dir() { std::fs::remove_dir_all(temp) } else { std::fs::remove_file(temp) };
    }
}

fn apply_post(
    post: &PostAction,
    plan: &Plan,
    before: &DirSnapshot,
    sequence: &mut Vec<PathBuf>,
) -> Result<(), EngineError> {
    match post {
        PostAction::MoveFrom(from) => {
            if !from.exists() {
                return Err(EngineError::NoOutput);
            }
            if let Some(parent) = plan.output.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // rename() fails across devices (temp dir on another volume) - fall back to copy.
            if std::fs::rename(from, &plan.output).is_err() {
                std::fs::copy(from, &plan.output)?;
                let _ = std::fs::remove_file(from);
            }
            Ok(())
        }
        PostAction::CollectSequence { dir, prefix, extension } => {
            let found = new_sequence_files(dir, prefix, extension, before);
            let dest = plan.output.parent().unwrap_or(Path::new("."));
            if dir == dest {
                sequence.extend(found.into_iter().map(|(_, p)| p));
                return Ok(());
            }
            // The tool wrote into a scratch directory (Flash frames): move the files next to the
            // requested output and give them the numbering the user asked for.
            let stem = plan
                .output
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            std::fs::create_dir_all(dest)?;
            for (n, (_, from)) in found.into_iter().enumerate() {
                let to = dest.join(format!("{stem}-{:04}.{extension}", n + 1));
                if std::fs::rename(&from, &to).is_err() {
                    std::fs::copy(&from, &to)?;
                    let _ = std::fs::remove_file(&from);
                }
                sequence.push(to);
            }
            Ok(())
        }
    }
}

/// The single file a job was asked to produce, if it is there.
///
/// Multi-file jobs never reach this: they declare `PostAction::CollectSequence`, and the engine
/// records exactly the files that step created. A one-page PDF *does* land here - ImageMagick
/// writes `clip.png` rather than `clip-0.png` when there is nothing to number.
fn collect_single(plan: &Plan) -> Vec<PathBuf> {
    if plan.output.exists() {
        vec![plan.output.clone()]
    } else {
        vec![]
    }
}

/// FFmpeg/LibreOffice error output is verbose; the useful part is usually the last line.
fn last_meaningful_line(stderr: &str) -> String {
    // Walked from the back: stderr can be thousands of lines and only the tail is interesting.
    stderr
        .lines()
        .map(|l| l.trim())
        .rev()
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(400)
        .collect()
}

/// Copy the source file's modification time onto the result, so batches keep their order in Finder.
pub fn preserve_timestamps(input: &Path, outputs: &[PathBuf], settings: &OutputSettings) {
    if !settings.preserve_timestamps {
        return;
    }
    let Ok(meta) = std::fs::metadata(input) else { return };
    let Ok(mtime) = meta.modified() else { return };
    for out in outputs {
        // Read-only results are a real case (a locked destination folder, or a tool that chmods
        // its output), and opening those for writing fails outright. Windows needs a writable
        // handle to set a timestamp, POSIX does not - so try write first, then fall back to read.
        let opened =
            std::fs::File::options().write(true).open(out).or_else(|_| std::fs::File::open(out));
        if let Ok(f) = opened {
            let _ = f.set_modified(mtime);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::by_id;
    use crate::plan::{plan, PlanRequest};
    use crate::settings::Settings;

    #[test]
    fn failed_or_cancelled_overwrite_restores_the_earlier_result() {
        for cancelled in [false, true] {
            let sandbox = Sandbox::new(if cancelled { "rollback-cancel" } else { "rollback-fail" });
            let tool = sandbox.script(
                "ffmpeg",
                r#"
for out in "$@"; do :; done
printf 'truncated' > "$out"
echo 'out_time_us=1000000'
echo 'progress=continue'
sleep 0.1
exit 7
"#,
            );
            let engine = engine_with(tool);
            let output = sandbox.path("out.mp4");
            std::fs::write(&output, b"earlier complete result").unwrap();
            let plan = plan_for(
                &sandbox.path("in.mov"),
                &output,
                ("mov", "mp4"),
                &sandbox.path("scratch"),
                true,
                &engine.tools,
            );
            let cancel = Arc::new(AtomicBool::new(false));
            let result = engine.run(&plan, Some(10.0), &cancel, &mut |_| {
                if cancelled {
                    cancel.store(true, Ordering::Relaxed);
                }
            });
            assert!(result.is_err());
            assert_eq!(std::fs::read(&output).unwrap(), b"earlier complete result");
            assert!(!std::fs::read_dir(&sandbox.dir).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".cc-backup-")));
        }
    }

    #[test]
    fn failed_frame_overwrite_restores_old_frames_and_removes_new_ones() {
        let sandbox = Sandbox::new("rollback-frames");
        let tool = sandbox.script(
            "ffmpeg",
            r#"
for out in "$@"; do :; done
prefix=${out%-%04d.png}
printf 'partial' > "$prefix-0001.png"
printf 'partial' > "$prefix-0002.png"
exit 7
"#,
        );
        let engine = engine_with(tool);
        let old = sandbox.path("out-0001.png");
        std::fs::write(&old, b"old complete frame").unwrap();
        let plan = plan_for(
            &sandbox.path("in.mov"),
            &sandbox.path("out.png"),
            ("mov", "png"),
            &sandbox.path("scratch"),
            true,
            &engine.tools,
        );
        assert!(engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).is_err());
        assert_eq!(std::fs::read(old).unwrap(), b"old complete frame");
        assert!(!sandbox.path("out-0002.png").exists());
    }

    #[test]
    fn rollback_survives_unwinding() {
        let sandbox = Sandbox::new("rollback-unwind");
        let output = sandbox.path("out.mp4");
        std::fs::write(&output, b"original").unwrap();
        let plan = Plan {
            midi: None,
            output: output.clone(),
            summary: String::new(),
            steps: vec![],
            temp_paths: vec![],
        };
        let result = std::panic::catch_unwind(|| {
            let _originals = OutputRollback::capture(&plan).unwrap();
            std::fs::write(&output, b"partial").unwrap();
            panic!("worker failed");
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(output).unwrap(), b"original");
    }

    #[test]
    fn failed_restore_keeps_and_names_the_recovery_copy() {
        let sandbox = Sandbox::new("rollback-unavailable");
        let output = sandbox.path("out.mp4");
        std::fs::write(&output, b"original").unwrap();
        let plan = Plan {
            midi: None,
            output: output.clone(),
            summary: String::new(),
            steps: vec![],
            temp_paths: vec![],
        };
        let mut originals = OutputRollback::capture(&plan).unwrap();
        std::fs::remove_file(&output).unwrap();
        std::fs::create_dir(&output).unwrap();
        let message = originals.restore().unwrap_err().to_string();
        let recovery = std::fs::read_dir(&sandbox.dir)
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().starts_with(".cc-backup-"))
            .unwrap()
            .path();
        assert!(message.contains(recovery.to_str().unwrap()), "{message}");
        drop(originals);
        assert_eq!(std::fs::read(recovery).unwrap(), b"original");
    }

    struct Sandbox {
        dir: PathBuf,
    }

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("cc-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }

        /// Write a fake tool so the engine can be tested end to end without shipping FFmpeg.
        fn script(&self, name: &str, body: &str) -> PathBuf {
            let path = self.dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            path
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.dir.join(rel)
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// A stand-in for ffmpeg: emits three progress blocks then writes the output file.
    const FAKE_FFMPEG: &str = r#"
for t in 1000000 2000000 3000000; do
  echo "frame=10"
  echo "speed=2.0x"
  echo "out_time_us=$t"
  echo "progress=continue"
done
echo "progress=end"
out=""
for a in "$@"; do out="$a"; done
printf 'converted' > "$out"
exit 0
"#;

    fn engine_with(ffmpeg: PathBuf) -> Engine {
        let mut tools = ToolRegistry::default();
        tools.set(Tool::Ffmpeg, ffmpeg);
        Engine::new(tools)
    }

    /// Build a plan the way the queue does, from a pair of format ids.
    fn plan_for(
        input: &Path,
        output: &Path,
        ids: (&str, &str),
        temp: &Path,
        animated: bool,
        tools: &ToolRegistry,
    ) -> Plan {
        let req = PlanRequest {
            input: input.to_path_buf(),
            output: output.to_path_buf(),
            source: by_id(ids.0).expect("source format"),
            target: by_id(ids.1).expect("target format"),
            settings: Settings::default(),
            temp_dir: temp.to_path_buf(),
            source_is_animated: animated,
        };
        plan(&req, tools).expect("the plan should build")
    }

    fn simple_plan(input: &Path, output: &Path, temp: &Path, tools: &ToolRegistry) -> Plan {
        plan_for(input, output, ("mov", "mp4"), temp, true, tools)
    }

    #[test]
    fn runs_a_plan_and_reports_increasing_progress() {
        let sb = Sandbox::new("run");
        let ffmpeg = sb.script("ffmpeg", FAKE_FFMPEG);
        let engine = engine_with(ffmpeg);
        std::fs::write(sb.path("clip.mov"), b"x").unwrap();

        let out = sb.path("out/clip.mp4");
        let plan = simple_plan(&sb.path("clip.mov"), &out, &sb.path("tmp"), &engine.tools);

        let mut fractions: Vec<f32> = vec![];
        let cancel = Arc::new(AtomicBool::new(false));
        let outcome = engine
            .run(&plan, Some(4.0), &cancel, &mut |u| {
                if let Some(f) = u.fraction {
                    fractions.push(f);
                }
            })
            .expect("plan should run");

        assert_eq!(outcome.outputs, vec![out.clone()]);
        assert_eq!(outcome.bytes, "converted".len() as u64);
        assert!(out.exists(), "the fake encoder should have written the file");
        assert!(fractions.len() >= 3, "expected progress updates, got {fractions:?}");
        assert!(
            fractions.windows(2).all(|w| w[1] >= w[0]),
            "progress went backwards: {fractions:?}"
        );
        assert_eq!(fractions.last().copied(), Some(1.0));
    }

    /// A job whose length nobody knows must report *no* number, not a zero.
    ///
    /// `None` is the shape the window has an answer for: `phaseStatus` in `src/lib/format.ts`
    /// renders "Converting…" and the bar goes indeterminate. A `Some(0.0)` is a different sentence
    /// entirely - "Converting 0%" for the whole run, which is what a hung job looks like.
    #[test]
    fn a_job_of_unknown_length_reports_no_percentage_rather_than_zero() {
        let sb = Sandbox::new("indeterminate");
        let ffmpeg = sb.script("ffmpeg", FAKE_FFMPEG);
        let engine = engine_with(ffmpeg);
        std::fs::write(sb.path("clip.mov"), b"x").unwrap();

        let out = sb.path("out/clip.mp4");
        let plan = simple_plan(&sb.path("clip.mov"), &out, &sb.path("tmp"), &engine.tools);

        let mut updates: Vec<Option<f32>> = vec![];
        let cancel = Arc::new(AtomicBool::new(false));
        engine
            .run(&plan, None, &cancel, &mut |u| updates.push(u.fraction))
            .expect("plan should run");

        // Three ffmpeg samples, and not one of them may carry a fraction.
        assert!(
            updates.iter().filter(|f| f.is_none()).count() >= 3,
            "an unmeasurable job reported percentages: {updates:?}"
        );
        assert!(
            !updates.contains(&Some(0.0)),
            "a bar stuck at 0% is the lie this replaces: {updates:?}"
        );
        // A *finished* step is still worth saying out loud: that number is measured, not guessed.
        assert_eq!(updates.last().copied(), Some(Some(1.0)));

        // And with a duration the very same run is a number again.
        let mut measured: Vec<Option<f32>> = vec![];
        engine
            .run(&plan, Some(4.0), &cancel, &mut |u| measured.push(u.fraction))
            .expect("plan should run");
        assert!(
            measured.iter().any(|f| matches!(f, Some(v) if *v > 0.0 && *v < 1.0)),
            "{measured:?}"
        );
    }

    #[test]
    fn a_failing_tool_surfaces_its_own_error_message() {
        let sb = Sandbox::new("fail");
        let ffmpeg =
            sb.script("ffmpeg", "echo 'Invalid data found when processing input' 1>&2\nexit 1");
        let engine = engine_with(ffmpeg);
        let plan =
            simple_plan(&sb.path("clip.mov"), &sb.path("clip.mp4"), &sb.path("tmp"), &engine.tools);

        let err =
            engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Invalid data found"), "{msg}");
        assert!(msg.contains("FFmpeg"), "{msg}");
    }

    #[test]
    fn a_missing_binary_is_a_clear_spawn_error() {
        let sb = Sandbox::new("missing");
        let engine = engine_with(sb.path("does-not-exist"));
        let plan =
            simple_plan(&sb.path("clip.mov"), &sb.path("clip.mp4"), &sb.path("tmp"), &engine.tools);
        let err =
            engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap_err();
        assert!(matches!(err, EngineError::Spawn { .. }), "{err}");
    }

    #[test]
    fn cancellation_kills_the_child_quickly() {
        let sb = Sandbox::new("cancel");
        // Emits one block, then hangs for a minute unless killed.
        let ffmpeg = sb.script(
            "ffmpeg",
            "echo 'out_time_us=1000000'\necho 'progress=continue'\nsleep 60\nexit 0",
        );
        let engine = engine_with(ffmpeg);
        let plan =
            simple_plan(&sb.path("clip.mov"), &sb.path("clip.mp4"), &sb.path("tmp"), &engine.tools);

        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let started = Instant::now();
        let err = engine
            .run(&plan, Some(10.0), &cancel, &mut |_| {
                flag.store(true, Ordering::Relaxed); // cancel as soon as progress arrives
            })
            .unwrap_err();
        assert!(matches!(err, EngineError::Cancelled), "{err}");
        assert!(started.elapsed().as_secs() < 10, "cancel must not wait for the child to finish");
    }

    /// A tool that stops talking is the normal case (sips, LibreOffice); one that *closes* stdout
    /// and then hangs is the pathological one. The step must still answer Stop, or the whole batch
    /// - and the app's Convert button - waits for a process that is never coming back.
    #[cfg(unix)]
    #[test]
    fn stop_reaches_a_tool_that_closed_its_pipes_and_hung() {
        let sb = Sandbox::new("silent-hang");
        let ffmpeg = sb.script("ffmpeg", "exec 1>&- 2>&-\nsleep 60\nexit 0");
        let engine = engine_with(ffmpeg);
        let plan =
            simple_plan(&sb.path("clip.mov"), &sb.path("clip.mp4"), &sb.path("tmp"), &engine.tools);

        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        // Nothing will ever call the progress callback here, so Stop arrives from the outside -
        // exactly as it does from `cancel_batch`.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            flag.store(true, Ordering::Relaxed);
        });

        let started = Instant::now();
        let err = engine.run(&plan, Some(10.0), &cancel, &mut |_| {}).unwrap_err();
        assert!(matches!(err, EngineError::Cancelled), "{err}");
        assert!(
            started.elapsed().as_secs() < 10,
            "Stop waited for the hung tool: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_cancelled_job_leaves_no_half_written_file_behind() {
        let sb = Sandbox::new("cancel-partial");
        // Writes a truncated file, reports progress, then hangs - exactly what a real encoder
        // looks like the moment the user hits Stop.
        let ffmpeg = sb.script(
            "ffmpeg",
            r#"
out=""
for a in "$@"; do out="$a"; done
mkdir -p "$(dirname "$out")"
printf 'half a video' > "$out"
echo 'out_time_us=1000000'
echo 'progress=continue'
sleep 60
"#,
        );
        let engine = engine_with(ffmpeg);
        let out = sb.path("out/clip.mp4");
        let plan = simple_plan(&sb.path("clip.mov"), &out, &sb.path("tmp"), &engine.tools);

        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let err = engine
            .run(&plan, Some(10.0), &cancel, &mut |_| flag.store(true, Ordering::Relaxed))
            .unwrap_err();
        assert!(matches!(err, EngineError::Cancelled), "{err}");
        assert!(!out.exists(), "a cancelled job must not leave a truncated file that looks done");
    }

    #[test]
    fn a_failed_job_takes_its_partial_output_and_temp_files_with_it() {
        let sb = Sandbox::new("fail-partial");
        let ffmpeg = sb.script(
            "ffmpeg",
            r#"
out=""
for a in "$@"; do out="$a"; done
mkdir -p "$(dirname "$out")"
printf 'garbage' > "$out"
echo 'Output file is empty, nothing was encoded' 1>&2
exit 1
"#,
        );
        let engine = engine_with(ffmpeg);
        let out = sb.path("out/clip.mp4");
        let plan = simple_plan(&sb.path("clip.mov"), &out, &sb.path("tmp"), &engine.tools);
        let err =
            engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap_err();
        assert!(matches!(err, EngineError::ToolFailed { .. }), "{err}");
        assert!(!out.exists(), "the unusable file must not be left in the destination folder");
    }

    #[test]
    fn a_file_that_was_already_there_survives_a_failure() {
        let sb = Sandbox::new("keep-existing");
        // Overwrite policy: the plan points at a file that already exists. If the tool fails
        // without touching it, the user's file must still be there afterwards.
        let ffmpeg = sb.script("ffmpeg", "exit 1");
        let engine = engine_with(ffmpeg);
        let out = sb.path("out/clip.mp4");
        std::fs::create_dir_all(sb.path("out")).unwrap();
        std::fs::write(&out, b"the original").unwrap();
        let plan = simple_plan(&sb.path("clip.mov"), &out, &sb.path("tmp"), &engine.tools);
        let err =
            engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap_err();
        assert!(matches!(err, EngineError::ToolFailed { .. }), "{err}");
        assert_eq!(std::fs::read(&out).unwrap(), b"the original");
    }

    #[test]
    fn a_half_written_sequence_is_removed_when_the_step_dies() {
        let sb = Sandbox::new("partial-seq");
        let ffmpeg = sb.script(
            "ffmpeg",
            r#"
out=""
for a in "$@"; do out="$a"; done
dir=$(dirname "$out")
mkdir -p "$dir"
printf 'a' > "$dir/clip-0001.png"
printf 'b' > "$dir/clip-0002.png"
echo 'Killed' 1>&2
exit 1
"#,
        );
        let engine = engine_with(ffmpeg);
        let out = sb.path("frames/clip.png");
        // A file already in the destination that only *looks* like part of the sequence.
        std::fs::create_dir_all(sb.path("frames")).unwrap();
        std::fs::write(sb.path("frames/clip-0009.png"), b"older run").unwrap();
        let plan = plan_for(
            &sb.path("clip.mp4"),
            &out,
            ("mp4", "png"),
            &sb.path("tmp"),
            true,
            &engine.tools,
        );
        let err =
            engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap_err();
        assert!(matches!(err, EngineError::ToolFailed { .. }), "{err}");
        assert!(!sb.path("frames/clip-0001.png").exists());
        assert!(!sb.path("frames/clip-0002.png").exists());
        assert!(
            sb.path("frames/clip-0009.png").exists(),
            "somebody else's file must be left alone"
        );
    }

    #[test]
    fn temp_files_do_not_survive_a_failure() {
        let sb = Sandbox::new("temp-leak");
        // Fake sips decode succeeds and writes the intermediate PNG; the second step then fails.
        let sips = sb.script(
            "sips",
            r#"
out=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--out" ]; then out="$a"; fi
  prev="$a"
done
mkdir -p "$(dirname "$out")"
printf 'png' > "$out"
"#,
        );
        let ffmpeg = sb.script(
            "ffmpeg",
            "echo 'boom' 1>&2
exit 1",
        );
        let mut tools = ToolRegistry::default();
        tools.set(Tool::Sips, sips);
        tools.set(Tool::Ffmpeg, ffmpeg);
        let engine = Engine::new(tools);
        let temp = sb.path("tmp");
        let plan = plan_for(
            &sb.path("photo.heic"),
            &sb.path("out/photo.jpg"),
            ("heic", "jpg"),
            &temp,
            false,
            &engine.tools,
        );
        assert_eq!(plan.temp_paths, vec![temp.join("decoded.png")]);
        let err =
            engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap_err();
        assert!(matches!(err, EngineError::ToolFailed { .. }), "{err}");
        assert!(!temp.join("decoded.png").exists(), "scratch files must not outlive the job");
    }

    #[cfg(unix)]
    #[test]
    fn a_panic_in_the_progress_callback_does_not_leave_the_tool_running() {
        let sb = Sandbox::new("orphan");
        // Reports progress, then keeps "encoding" and leaves a mark if it is still alive later.
        let ffmpeg = sb.script(
            "ffmpeg",
            r#"
echo 'out_time_us=1000000'
echo 'progress=continue'
sleep 2
printf 'orphan' > "$(dirname "$0")/alive"
"#,
        );
        let engine = engine_with(ffmpeg);
        let plan = simple_plan(
            &sb.path("clip.mov"),
            &sb.path("out/clip.mp4"),
            &sb.path("tmp"),
            &engine.tools,
        );

        let cancel = Arc::new(AtomicBool::new(false));
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {})); // the crash is the point; do not print it
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            engine.run(&plan, Some(10.0), &cancel, &mut |_| panic!("the UI blew up"))
        }));
        std::panic::set_hook(hook);
        assert!(result.is_err(), "the panic must not be swallowed");

        // Long enough for the abandoned process to reach its `printf`, if it survived.
        std::thread::sleep(std::time::Duration::from_millis(2500));
        assert!(
            !sb.path("alive").exists(),
            "the tool outlived the job: an orphan FFmpeg keeps writing to the user's file"
        );
    }

    #[test]
    fn no_output_file_is_reported_instead_of_a_silent_success() {
        let sb = Sandbox::new("noout");
        let ffmpeg = sb.script("ffmpeg", "echo 'progress=end'\nexit 0");
        let engine = engine_with(ffmpeg);
        let plan =
            simple_plan(&sb.path("clip.mov"), &sb.path("clip.mp4"), &sb.path("tmp"), &engine.tools);
        let err =
            engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap_err();
        assert!(matches!(err, EngineError::NoOutput), "{err}");
    }

    #[test]
    fn libreoffice_style_output_is_moved_into_place() {
        let sb = Sandbox::new("office");
        // Fake soffice: writes <stem>.pdf into the --outdir it was given.
        let soffice = sb.script(
            "soffice",
            r#"
outdir=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--outdir" ]; then outdir="$a"; fi
  prev="$a"
  last="$a"
done
mkdir -p "$outdir"
stem=$(basename "$last" | sed 's/\.[^.]*$//')
printf '%%PDF-1.4' > "$outdir/$stem.pdf"
"#,
        );
        let mut tools = ToolRegistry::default();
        tools.set(Tool::LibreOffice, soffice);
        let engine = Engine::new(tools);

        let input = sb.path("report.docx");
        std::fs::write(&input, b"x").unwrap();
        let output = sb.path("out/report.pdf");
        let plan =
            plan_for(&input, &output, ("docx", "pdf"), &sb.path("tmp"), false, &engine.tools);

        let outcome = engine
            .run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {})
            .expect("office conversion should succeed");
        assert_eq!(outcome.outputs, vec![output.clone()]);
        assert!(output.exists());
    }

    #[test]
    fn frame_sequences_are_collected() {
        let sb = Sandbox::new("frames");
        let ffmpeg = sb.script(
            "ffmpeg",
            r#"
out=""
for a in "$@"; do out="$a"; done
dir=$(dirname "$out")
mkdir -p "$dir"
printf 'a' > "$dir/clip-0001.png"
printf 'b' > "$dir/clip-0002.png"
echo 'progress=end'
"#,
        );
        let engine = engine_with(ffmpeg);
        let out = sb.path("frames/clip.png");
        let plan = plan_for(
            &sb.path("clip.mp4"),
            &out,
            ("mp4", "png"),
            &sb.path("tmp"),
            true,
            &engine.tools,
        );
        let outcome =
            engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap();
        assert_eq!(outcome.outputs.len(), 2, "{:?}", outcome.outputs);
        assert_eq!(outcome.bytes, 2);
    }

    /// The old collector scanned the output folder for a filename *prefix*, so a leftover from a
    /// previous run - or another job writing `clip-2.png` next door - was reported as this job's
    /// work, inflating the file count and the byte total.
    #[test]
    fn files_that_were_already_there_are_not_claimed_as_output() {
        let sb = Sandbox::new("stale");
        let ffmpeg = sb.script(
            "ffmpeg",
            r#"
out=""
for a in "$@"; do out="$a"; done
dir=$(dirname "$out")
printf 'a' > "$dir/clip-0001.png"
printf 'bb' > "$dir/clip-0002.png"
echo 'progress=end'
"#,
        );
        let engine = engine_with(ffmpeg);
        let dir = sb.path("frames");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clip-9.png"), b"stale from an earlier run").unwrap();
        std::fs::write(dir.join("clip-old.png"), b"not even numbered").unwrap();
        std::fs::write(dir.join("clip2-0001.png"), b"a different job").unwrap();

        let plan = plan_for(
            &sb.path("clip.mp4"),
            &dir.join("clip.png"),
            ("mp4", "png"),
            &sb.path("tmp"),
            true,
            &engine.tools,
        );
        let outcome =
            engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap();
        assert_eq!(outcome.outputs.len(), 2, "{:?}", outcome.outputs);
        assert_eq!(outcome.bytes, 3, "only the bytes this job wrote");
    }

    /// Converting the same video to frames twice *rewrites* `clip-0001.png` rather than adding a
    /// file, and the collector only looked at names: the second run reported "the conversion
    /// produced no output file" (or listed only the frames the first run had not reached) while
    /// having already replaced them on disk. Failure plus destruction is the worst pair.
    #[test]
    fn frames_that_replace_a_previous_run_are_still_this_jobs_output() {
        let sb = Sandbox::new("rewrite");
        let ffmpeg = sb.script(
            "ffmpeg",
            r#"
out=""
for a in "$@"; do out="$a"; done
dir=$(dirname "$out")
mkdir -p "$dir"
printf 'aa' > "$dir/clip-0001.png"
printf 'bb' > "$dir/clip-0002.png"
echo 'progress=end'
"#,
        );
        let engine = engine_with(ffmpeg);
        let dir = sb.path("frames");
        std::fs::create_dir_all(&dir).unwrap();
        // Exactly what a first run left behind - same names, different content.
        std::fs::write(dir.join("clip-0001.png"), b"first run").unwrap();
        std::fs::write(dir.join("clip-0002.png"), b"first run").unwrap();
        // ...next to a leftover the second run does not reach, which stays somebody else's.
        std::fs::write(dir.join("clip-0003.png"), b"first run, longer").unwrap();

        let plan = plan_for(
            &sb.path("clip.mp4"),
            &dir.join("clip.png"),
            ("mp4", "png"),
            &sb.path("tmp"),
            true,
            &engine.tools,
        );
        let outcome =
            engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap();
        assert_eq!(outcome.outputs.len(), 2, "{:?}", outcome.outputs);
        assert_eq!(outcome.bytes, 4, "only the frames this run wrote");
        assert!(dir.join("clip-0003.png").exists(), "an untouched leftover is not ours to report");
    }

    #[test]
    fn sequence_names_must_match_the_exact_numbering() {
        assert_eq!(sequence_index("clip-0007.png", "clip", "png"), Some(7));
        assert_eq!(sequence_index("clip-10.png", "clip", "png"), Some(10));
        assert_eq!(sequence_index("clip-old.png", "clip", "png"), None);
        assert_eq!(sequence_index("clip2-0001.png", "clip", "png"), None);
        assert_eq!(sequence_index("clip-0001.jpg", "clip", "png"), None);
        // No prefix = a private scratch directory, where every file of that type is ours.
        assert_eq!(sequence_index("frame_0001.png", "", "png"), Some(0));
    }

    /// Ruffle renders into a scratch directory, so "collect the sequence" also has to move the
    /// files next to the output the user actually asked for.
    #[test]
    fn a_scratch_sequence_is_moved_next_to_the_requested_output() {
        let sb = Sandbox::new("flashframes");
        let ruffle = sb.script(
            "ruffle",
            r#"
dir="$2"
mkdir -p "$dir"
printf 'a' > "$dir/frame_0001.png"
printf 'b' > "$dir/frame_0002.png"
"#,
        );
        let mut tools = ToolRegistry::default();
        tools.set(Tool::Ruffle, ruffle);
        tools.set(Tool::Ffmpeg, sb.path("unused-ffmpeg"));
        let engine = Engine::new(tools);

        let out = sb.path("out/game.png");
        let plan = plan_for(
            &sb.path("game.swf"),
            &out,
            ("swf", "png"),
            &sb.path("tmp"),
            true,
            &engine.tools,
        );
        let outcome =
            engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap();
        assert_eq!(
            outcome.outputs,
            vec![sb.path("out/game-0001.png"), sb.path("out/game-0002.png")]
        );
        assert!(outcome.outputs.iter().all(|p| p.exists()));
    }

    /// The sign-in check, driven by a stand-in yt-dlp: what each kind of exit becomes, and that a
    /// probe which will not come back is killed instead of holding the drawer open.
    ///
    /// The verdicts themselves are `link`'s business and tested there; what is tested here is the
    /// part only the engine can get wrong - running the thing, bounding it, and letting nothing but
    /// a verdict and a classified sentence out.
    #[cfg(unix)]
    #[test]
    fn a_sign_in_check_is_bounded_and_says_only_what_it_learned() {
        use crate::link::{CookieCheck, CookieProbe};
        let sb = Sandbox::new("cookie-check");
        let link = Link::parse(link::COOKIE_TEST_URL).expect("the test URL");
        let chrome = CookieFlag::Browser("chrome");
        // A JavaScript runtime is registered throughout, because without one every sign-in wall is
        // classified as the missing runtime it really is - which is the right answer, and not the
        // one this test is about.
        let deno = sb.script("deno", "exit 0");
        let check = |program: PathBuf, budget: Duration| {
            let mut tools = ToolRegistry::default();
            tools.set(Tool::YtDlp, program);
            tools.set(Tool::Deno, deno.clone());
            Engine::new(tools).check_cookie_source(&link, Some(&chrome), budget)
        };

        // Read, and served: the one answer that means "go ahead and retry the link".
        let served =
            sb.script("yt-dlp-ok", "echo 'Rick Astley - Never Gonna Give You Up'\necho 213");
        let probe = check(served, Duration::from_secs(20));
        assert_eq!(probe, CookieProbe::Worked);
        assert_eq!(probe.verdict(), CookieCheck::Working);

        // Refused, in the words yt-dlp really uses. The verdict distinguishes a jar that never
        // opened from cookies the site would not take, and the message is the classifier's - not
        // one word of the tool's own output reaches the user.
        let unreadable = sb.script(
            "yt-dlp-nojar",
            "echo 'ERROR: could not find chrome cookies database in \"/x\"' 1>&2\nexit 1",
        );
        let probe = check(unreadable, Duration::from_secs(20));
        assert_eq!(probe.verdict(), CookieCheck::Unreadable);
        assert_eq!(probe.message(), FetchFailure::BrowserCookiesUnreadable.to_string());
        assert!(!probe.message().contains("cookies database"), "{}", probe.message());

        let stale = sb.script(
            "yt-dlp-wall",
            "echo \"ERROR: [youtube] abc: Sign in to confirm you're not a bot.\" 1>&2\nexit 1",
        );
        assert_eq!(check(stale, Duration::from_secs(20)).verdict(), CookieCheck::Refused);

        // A prompt nobody is there to answer, or a network that never replies: the child is killed
        // by the same process-group kill Stop uses, and the check admits it learned nothing rather
        // than blaming the user's cookies.
        let hangs = sb.script("yt-dlp-hang", "exec 1>&- 2>&-\nsleep 60\nexit 0");
        let started = Instant::now();
        let probe = check(hangs, Duration::from_millis(300));
        assert_eq!(probe, CookieProbe::TimedOut);
        assert_eq!(probe.verdict(), CookieCheck::Inconclusive);
        assert!(started.elapsed().as_secs() < 10, "the budget was not honoured: {started:?}");

        // No yt-dlp at all is not a cookie problem either, and is answered without a probe.
        let probe = Engine::new(ToolRegistry::default()).check_cookie_source(
            &link,
            Some(&chrome),
            Duration::from_secs(20),
        );
        assert_eq!(probe, CookieProbe::Failed(FetchFailure::NotInstalled));
        assert_eq!(probe.verdict(), CookieCheck::Inconclusive);
    }

    #[test]
    fn probe_returns_none_when_ffprobe_is_absent() {
        let engine = Engine::new(ToolRegistry::default());
        assert!(engine.probe(Path::new("/nope.mp4")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn stop_interrupts_file_and_link_metadata_probes() {
        let sb = Sandbox::new("cancel-probes");
        let program = sb.script("hung-probe", "exec sleep 60");
        let mut tools = ToolRegistry::default();
        tools.set(Tool::Ffprobe, program.clone());
        tools.set(Tool::YtDlp, program);
        let engine = Engine::new(tools);
        for link_probe in [false, true] {
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let stop = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                flag.store(true, Ordering::Relaxed);
            });
            let started = Instant::now();
            if link_probe {
                let link = Link::parse(link::COOKIE_TEST_URL).unwrap();
                assert!(matches!(
                    engine.probe_link_cancellable(&link, None, &cancel),
                    Err(EngineError::Cancelled)
                ));
            } else {
                assert!(engine.probe_cancellable(&sb.path("clip.mp4"), &cancel).is_none());
            }
            stop.join().unwrap();
            assert!(started.elapsed() < Duration::from_secs(5));
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_probe_timeout_does_not_cancel_the_batch() {
        let sb = Sandbox::new("probe-deadline");
        let program = sb.script("hung-probe", "exec sleep 60");
        let cancel = Arc::new(AtomicBool::new(false));
        let started = Instant::now();
        let result =
            run_probe(&program, &[], None, &cancel, Duration::from_millis(100), &mut |_| {});
        assert!(matches!(result, Err(EngineError::ProbeTimedOut)));
        assert!(!cancel.load(Ordering::Relaxed));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn a_cancelled_probe_never_launches_a_child() {
        let cancel = Arc::new(AtomicBool::new(true));
        let result = run_probe(
            Path::new("/does-not-exist"),
            &[],
            None,
            &cancel,
            Duration::from_secs(1),
            &mut |_| {},
        );
        assert!(matches!(result, Err(EngineError::Cancelled)));
    }

    #[test]
    fn stderr_is_trimmed_to_the_useful_line() {
        assert_eq!(last_meaningful_line("warn\n\nreal error\n\n"), "real error");
        assert_eq!(last_meaningful_line(""), "");
    }

    /// `kill -9 -1234` is parsed as options by procps: it exits 0 having killed nothing (leaving
    /// grandchildren holding the pipes), and a short pid can even read as `-1` = everything.
    #[cfg(unix)]
    #[test]
    fn the_group_kill_separates_options_from_the_negative_pid() {
        assert_eq!(kill_group_args(4321), ["-9", "--", "-4321"]);
    }

    #[test]
    fn join_within_gives_up_on_a_reader_that_will_not_finish() {
        let stuck = std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(30));
            "never".to_string()
        });
        let started = Instant::now();
        assert!(join_within(stuck, std::time::Duration::from_millis(20)).is_none());
        assert!(started.elapsed().as_secs() < 5, "the budget was not honoured");

        let quick = std::thread::spawn(|| "done".to_string());
        assert_eq!(join_within(quick, std::time::Duration::from_secs(5)).as_deref(), Some("done"));
    }
}
