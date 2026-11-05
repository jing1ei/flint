//! Running an install, observably.
//!
//! `brew install --cask libreoffice` downloads ~350 MB and takes minutes, so this is the same shape
//! as a conversion batch: a plain OS thread, an event stream, and a single-flight guard keyed to an
//! id (see [`crate::AppState::begin_install`]). stdout and stderr are merged and forwarded line by
//! line, because `brew` writes its progress to stderr and a settings page showing nothing for four
//! minutes is indistinguishable from a hang.
//!
//! Two rules this module exists to keep:
//!
//! * the command comes from [`convert_core::install`]'s allowlist - nothing here builds one, and
//!   the argv it hands us is `&'static str`, so no IPC input can reach it;
//! * success is never *reported*, only *observed*: after the installer exits we re-run discovery
//!   and say `ok: true` only if **every** binary the package promises is genuinely there now. A
//!   formula that landed two of Poppler's three has not succeeded, whatever it exited with.

use convert_core::install::InstallCommand;
use convert_core::package::Presence;
use convert_core::{Package, Tool, ToolRegistry};
use serde::Serialize;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// How many trailing output lines we keep for the failure message. The UI already has every line.
const LOG_TAIL: usize = 20;
/// Longest `log` line we forward. `brew` pipes `curl`'s progress bar through, which can be one very
/// long line once carriage returns are collapsed.
const MAX_LINE: usize = 500;

/// The stream the settings page listens to. `finished` is always the last event for a package.
///
/// Keyed by *package* id, the same id the frontend sent to `install_tool` and the same one
/// [`convert_core::install::PackageInstallPlan::package_id`] carries, so a row can match the events
/// to the button that started them. The member binaries are not in here: the UI already knows them
/// from the plan, and a person reading a log wants "Poppler", not three ids.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InstallEvent {
    Started { package_id: String },
    Log { package_id: String, line: String },
    Finished { package_id: String, ok: bool, message: String },
}

/// Run an install to completion, emitting [`InstallEvent`]s. Blocks: call it on its own thread.
pub fn run<R: Runtime>(app: AppHandle<R>, command: InstallCommand, install_id: u64) {
    // Real discovery, exactly the same code path `refresh_tools` uses - the sidecar directory is
    // irrelevant here because no optional helper ever lives in it. One scan, then asked about each
    // member, so a package is judged on a single consistent view of the machine.
    //
    // The scan lives *inside* this closure rather than in front of it, and that is the whole point
    // of the factory: `run_with` decides when the machine is looked at, and it looks after the
    // installer has exited. Hoisting `discover` out of here would be the bug this shape prevents -
    // a snapshot taken before `brew` ran can only ever say "not there", which turned a perfectly
    // good install into "the installer finished, but we still cannot find it".
    run_with(app, command, install_id, || {
        let found = ToolRegistry::discover(None);
        move |tool| found.has(tool)
    })
}

/// Same as [`run`] with the "is it there now?" check injected - as a *factory*, not a probe.
///
/// The indirection is load-bearing. A ready-made `&dyn Fn(Tool) -> bool` has to be built by the
/// caller, and a caller that builds it by scanning a disk has already scanned that disk before this
/// function did anything: `classify` then asks a pre-install snapshot "is it there now?", and a
/// fresh install reads back as [`Outcome::NotDiscoverable`]. Taking a `FnOnce` that *makes* the
/// probe moves that decision here, where the one call sits after [`stream`] has returned - so an
/// early scan is no longer something a caller can get wrong, it is something they cannot express.
///
/// The probe itself stays per *binary* - that is what can actually be answered by looking at a
/// disk - and this module folds the answers into the package's [`Presence`]. Injectable because
/// neither outcome is reachable in CI otherwise: no test machine has Homebrew, and a test must
/// never install 350 MB of LibreOffice to find out what we do afterwards.
pub fn run_with<R, Probe, MakeProbe>(
    app: AppHandle<R>,
    command: InstallCommand,
    install_id: u64,
    probe_the_machine: MakeProbe,
) where
    R: Runtime,
    Probe: Fn(Tool) -> bool,
    MakeProbe: FnOnce() -> Probe,
{
    let package_id = command.package.id.to_string();
    // Backstop for the path that emits no `finished` event at all - a panic unwinding out of here.
    // Releasing is keyed to this install's id, so a late release cannot free a successor's slot.
    let mut guard =
        SlotGuard { app: app.clone(), install_id, package_id: package_id.clone(), settled: false };
    emit(&app, InstallEvent::Started { package_id: package_id.clone() });

    let transcript = stream(&app, &command, &package_id);
    // The one look at the machine, and it happens here: the installer has exited, so whatever it
    // laid down is on disk by now. Pinned by
    // `crate::tests::the_machine_is_only_scanned_once_the_installer_has_finished`.
    let discovered = probe_the_machine();
    let outcome = classify(&transcript, command.package, &discovered);
    // The one place the exact executables belong: knowing *which* of Poppler's three the formula
    // failed to leave behind is what makes a bug report actionable, and a log has no user in it.
    if let Outcome::Incomplete { missing } = &outcome {
        let names: Vec<&str> = missing.iter().map(|t| t.id()).collect();
        eprintln!("[flint] {} installed without {}", command.package.id, names.join(", "));
    }
    let (ok, message) = message_for(command.package, &command.display, &outcome);

    // ORDER MATTERS: the slot is freed *before* the UI is told the install is over, never after.
    //
    // Same discipline as a batch, and the same two dependants. The settings page re-enables its
    // Install button on `finished`, so a user clicking it the instant that lands must not be told "an
    // install is already running". And the frontend's adoption path (a webview reloaded mid-install:
    // `adoptShellWork` in `src/state/store.ts`) confirms what it adopted by reading `get_activity`
    // again *after* this event - a slot still held here reads back as "still installing" and leaves
    // every Install button dead for the rest of the session.
    //
    // Pinned by `crate::tests::the_install_slot_is_free_at_the_instant_finished_is_emitted`, which
    // fails if these two statements are ever swapped.
    guard.settled = true; // this `finished` is the real one; the guard must not invent a second
    release(&app, install_id);
    emit(&app, InstallEvent::Finished { package_id, ok, message });
}

/// What the installer said and how it ended.
#[derive(Debug, Default)]
struct Transcript {
    status: Option<ExitStatus>,
    /// Set when the manager could not even be launched (deleted between plan and click).
    spawn_error: Option<String>,
    tail: Vec<String>,
    /// Whether anything in the output looked like a request for an administrator password.
    saw_password_prompt: bool,
}

/// Spawn the installer and forward every line it writes.
fn stream<R: Runtime>(
    app: &AppHandle<R>,
    command: &InstallCommand,
    package_id: &str,
) -> Transcript {
    // The last gate before a process starts. `InstallCommand::program` is meant to be an absolute
    // path probed on disk, but the resolver's final fallback walks `PATH` - and a `PATH` holding a
    // relative entry (`.`, or an empty element, which is how a trailing `:` reads) yields a
    // relative program that `Command` resolves against *this app's* working directory. Refusing it
    // here is what keeps "we always execute an absolute path" true rather than aspirational.
    if !command.program.is_absolute() {
        return Transcript {
            spawn_error: Some(format!(
                "`{}` is not an absolute path, so it is not the package manager we probed for",
                command.program.display()
            )),
            ..Default::default()
        };
    }
    let mut child = match Command::new(&command.program)
        .args(&command.args)
        .env("PATH", child_path(&command.program))
        .envs(command.env.iter().copied())
        // No stdin: there is no terminal here, so a password prompt must fail fast rather than
        // block a thread for the rest of the session waiting for input that cannot arrive.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return Transcript { spawn_error: Some(e.to_string()), ..Default::default() },
    };

    // Both pipes onto one channel: `brew` puts its headings and progress on stderr and only some
    // results on stdout, so reading them separately would show the user half a story - and reading
    // one after the other would deadlock when the unread pipe fills up.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mut pipes: Vec<Box<dyn Read + Send>> = Vec::new();
    if let Some(out) = child.stdout.take() {
        pipes.push(Box::new(out));
    }
    if let Some(err) = child.stderr.take() {
        pipes.push(Box::new(err));
    }
    let readers: Vec<_> = pipes
        .into_iter()
        .map(|pipe| {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
        })
        .collect();
    drop(tx); // the loop below ends when the last reader hangs up

    let mut tail: VecDeque<String> = VecDeque::new();
    let mut saw_password_prompt = false;
    for raw in rx {
        let line = readable(&raw);
        if line.is_empty() {
            continue;
        }
        saw_password_prompt |= asks_for_a_password(&line);
        if tail.len() == LOG_TAIL {
            tail.pop_front();
        }
        tail.push_back(line.clone());
        emit(app, InstallEvent::Log { package_id: package_id.to_string(), line });
    }

    let status = child.wait().ok();
    for reader in readers {
        let _ = reader.join();
    }
    Transcript { status, spawn_error: None, tail: tail.into(), saw_password_prompt }
}

/// How an install ended, before it is turned into words.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// Exit 0 and every member binary is discoverable.
    Installed,
    /// The installer failed, but the package is complete anyway - almost always "already installed".
    AlreadyThere,
    /// Nothing the package promises is where the app looks. Never reported as success.
    NotDiscoverable,
    /// Some members landed and some did not - `brew install poppler` that left `pdftohtml` behind.
    /// Not a success (the routes needing the missing binary still cannot run) and not nothing
    /// either, so it gets its own answer and keeps the binaries it is missing for the log.
    Incomplete {
        missing: Vec<Tool>,
    },
    /// Failed asking for an administrator password, which there is no TTY to type into.
    NeedsPassword,
    Failed {
        code: String,
        detail: String,
    },
    CouldNotStart(String),
}

/// The package's [`Presence`] according to the injected probe, plus the members that are missing.
///
/// [`Package::presence`] answers the same question from a [`ToolRegistry`]; this one exists because
/// the check here is injectable (no CI machine has Homebrew) and because the missing members
/// themselves are worth keeping - they are what a log line needs to be useful.
fn presence_of(package: &Package, discovered: &dyn Fn(Tool) -> bool) -> (Presence, Vec<Tool>) {
    let missing: Vec<Tool> = package.tools.iter().copied().filter(|t| !discovered(*t)).collect();
    let presence = match missing.len() {
        0 => Presence::Complete,
        n if n == package.tools.len() => Presence::Absent,
        _ => Presence::Partial,
    };
    (presence, missing)
}

/// Decide what happened. The probe is the only source of truth for "is it installed": an exit code
/// says the *installer* succeeded, not that this app can now find what it promised (a cask that lands
/// somewhere unexpected, a formula that only installs a library, and a formula that installs two of
/// its three programs all exit 0).
fn classify(
    transcript: &Transcript,
    package: &'static Package,
    discovered: &dyn Fn(Tool) -> bool,
) -> Outcome {
    if let Some(e) = &transcript.spawn_error {
        return Outcome::CouldNotStart(e.clone());
    }
    let (presence, missing) = presence_of(package, discovered);
    let installer_succeeded = transcript.status.map(|s| s.success()).unwrap_or(false);
    match presence {
        Presence::Complete if installer_succeeded => Outcome::Installed,
        Presence::Complete => Outcome::AlreadyThere,
        // Whatever the exit code said: a half-landed package is not installed. Same answer either
        // way, because the user's next step is the same one.
        Presence::Partial => Outcome::Incomplete { missing },
        Presence::Absent if installer_succeeded => Outcome::NotDiscoverable,
        Presence::Absent if transcript.saw_password_prompt => Outcome::NeedsPassword,
        Presence::Absent => Outcome::Failed {
            code: transcript
                .status
                .and_then(|s| s.code())
                .map(|c| c.to_string())
                .unwrap_or_else(|| "no exit code".into()),
            detail: transcript.tail.last().cloned().unwrap_or_default(),
        },
    }
}

/// Turn an outcome into `(ok, message)`. Every failure ends in a step the user can actually take,
/// and which step depends on who is able to do something about it: the command to run by hand where
/// only a Terminal can help (a password there is nowhere here to type, a manager that will not
/// start), and this app's own "Re-check" where the app is the one that has to look again.
///
/// What no message does any more is send someone to Terminal to audit work this app already did.
/// The user who hit that typed `brew install yt-dlp`, was told "already installed", and learned
/// nothing: every line the installer wrote is behind the "Show log" affordance next to the message,
/// and looking again is what "Re-check" is for.
///
/// The name in every sentence is the *package's*: it is what the user clicked Install on and what
/// they would type in Terminal. The member binary that is actually missing is a log line, not this.
fn message_for(package: &Package, command: &str, outcome: &Outcome) -> (bool, String) {
    let label = package.name;
    match outcome {
        Outcome::Installed => (true, format!("{label} is installed. Flint can use it now.")),
        Outcome::AlreadyThere => (
            true,
            format!(
                "{label} is available now. The installer reported a problem, which usually means \
                 it was already installed."
            ),
        ),
        Outcome::NotDiscoverable => (
            false,
            format!(
                "The installer reported success, but {label} is not in any of the places \
                 Flint looks. Use Re-check to look again."
            ),
        ),
        Outcome::Incomplete { missing } => {
            let total = package.tools.len();
            let found = total.saturating_sub(missing.len());
            (
                false,
                format!(
                    "The installer finished, but Flint can only find part of \
                     {label} ({found} of its {total} programs), so some conversions still will not \
                     run. Use Re-check, then install {label} again if it is still incomplete."
                ),
            )
        }
        Outcome::NeedsPassword => (
            false,
            format!(
                "{label} has to be installed with an administrator password, and there is nowhere \
                 to type one here. Open Terminal and run `{command}` - it will ask for your \
                 password, and then this page will find {label}."
            ),
        ),
        Outcome::Failed { code, detail } => {
            let detail =
                if detail.is_empty() { String::new() } else { format!("Last message: {detail} ") };
            (
                false,
                format!(
                    "Could not install {label} (the installer exited with {code}). {detail}Show \
                     log for everything it wrote, or run `{command}` in Terminal to install it by \
                     hand."
                ),
            )
        }
        Outcome::CouldNotStart(e) => (
            false,
            format!("Could not start the installer for {label}: {e}. Run `{command}` in Terminal."),
        ),
    }
}

/// Does this line look like a request for an administrator password?
///
/// A cask that writes outside `~` shells out to `sudo`, which with no TTY and `NONINTERACTIVE` set
/// fails with one of these. The user needs to be told to run the command in Terminal, not shown
/// "exited with 1".
fn asks_for_a_password(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "no tty present",
        "askpass",
        "sudo:",
        "sudo access",
        "password:",
        "password is required",
        "administrator password",
        "requires administrator",
        "not in the sudoers file",
        "superuser privileges",
        "needs superuser",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// One line of installer output, safe to hand to a webview.
///
/// Progress bars are redrawn with carriage returns, so only the text after the last `\r` is current;
/// control characters (including ANSI escapes) are dropped rather than passed to the UI, and the
/// result is capped so one pathological line cannot flood the event channel.
fn readable(raw: &str) -> String {
    let current = raw.rsplit('\r').next().unwrap_or(raw);
    let cleaned: String = current.chars().filter(|c| !c.is_control()).take(MAX_LINE).collect();
    cleaned.trim().to_string()
}

/// A minimal, absolute `PATH` for the installer.
///
/// Whatever a GUI app inherited from Finder is not usable (often just `/usr/bin:/bin`), and
/// Homebrew shells out to `git`, `curl` and `ruby`. The manager's own directory goes first so its
/// helpers win over anything the system ships.
///
/// Both Homebrew prefixes are then named, not just the one this manager happens to live in.
/// `/opt/homebrew` is Apple Silicon and `/usr/local` is Intel, and they are not alternatives: half
/// the Macs this app runs on are Intel, an Apple Silicon machine that was migrated from one still
/// has formulae under `/usr/local`, and a dependency a formula shells out to sitting in the prefix
/// we left out is invisible - `brew` fails with "command not found" for a program that is on the
/// disk. Same two directories `convert_core::tools::FIXED_SEARCH_DIRS` searches, for the same
/// reason.
fn child_path(program: &Path) -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = program.parent() {
        dirs.push(dir.to_path_buf());
    }
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        let dir = PathBuf::from(dir);
        // The manager's own directory is one of these on a normal install, and naming it twice
        // would say nothing new.
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(&dirs).unwrap_or_default()
}

fn emit<R: Runtime>(app: &AppHandle<R>, event: InstallEvent) {
    // A failed emit means the window went away mid-install; the installer keeps going (the user
    // still wants the tool) and we log it rather than aborting a download that is half done.
    if let Err(e) = app.emit(crate::INSTALL_EVENT, event) {
        eprintln!("[flint] could not deliver install event: {e}");
    }
}

/// Free the single-install slot, if `install_id` still owns it. `try_state` because this also runs
/// from a `Drop` that may be unwinding, where panicking would abort the process.
fn release<R: Runtime>(app: &AppHandle<R>, install_id: u64) {
    if let Some(state) = app.try_state::<crate::AppState>() {
        state.end_install(install_id);
    }
}

/// Frees the install slot when the worker thread ends, however it ends - and, when it ended the one
/// way that emits no `finished` of its own, says so.
///
/// `finished` is the settings page's only way out of "Installing…": it re-enables the Install
/// button and stops the spinner. A panic unwinding out of [`run_with`] - the OS refusing a reader
/// thread in [`stream`], a discovery probe that trips over a broken `PATH` entry - skipped straight
/// past the emit at the end, so the slot was freed but the row spun for the rest of the session over
/// an install that had stopped. The guard now closes the row out instead, and honestly: it never saw
/// the installer succeed, so it says it did not.
struct SlotGuard<R: Runtime> {
    app: AppHandle<R>,
    install_id: u64,
    package_id: String,
    /// Set once the real `finished` is on its way, which is what tells the guard to stay quiet.
    settled: bool,
}

impl<R: Runtime> Drop for SlotGuard<R> {
    fn drop(&mut self) {
        // Same order as the happy path: the slot is free before the window is told it is over.
        release(&self.app, self.install_id);
        if self.settled {
            return;
        }
        eprintln!("[flint] install of {} ended without finishing", self.package_id);
        emit(
            &self.app,
            InstallEvent::Finished {
                package_id: self.package_id.clone(),
                ok: false,
                message: "The installer stopped unexpectedly. Nothing was verified, so try again \
                          or install it yourself."
                    .into(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert_core::package::{LIBREOFFICE, PANDOC, POPPLER};

    fn transcript(code: Option<i32>, lines: &[&str]) -> Transcript {
        Transcript {
            status: code.map(fake_status),
            spawn_error: None,
            tail: lines.iter().map(|l| l.to_string()).collect(),
            saw_password_prompt: lines.iter().any(|l| asks_for_a_password(l)),
        }
    }

    /// A real `ExitStatus` for a given code, without inventing one: run `sh -c "exit N"`.
    fn fake_status(code: i32) -> ExitStatus {
        Command::new("sh")
            .args(["-c", &format!("exit {code}")])
            .status()
            .expect("sh must be available")
    }

    #[test]
    fn success_is_only_reported_when_the_tool_is_actually_there() {
        let ok = transcript(Some(0), &["==> Pouring pandoc"]);
        assert_eq!(classify(&ok, &PANDOC, &|_| true), Outcome::Installed);
        // Exit 0 and still missing: the honest answer is "no", plus the one thing left to try.
        assert_eq!(classify(&ok, &PANDOC, &|_| false), Outcome::NotDiscoverable);
        let (reported, message) =
            message_for(&PANDOC, "brew install pandoc", &classify(&ok, &PANDOC, &|_| false));
        assert!(!reported, "{message}");
        assert!(message.contains("Pandoc is not in any of the places"), "{message}");
        assert!(message.contains("Re-check"), "{message}");
        // Never again: this is the app failing to find its own install, so asking the user to go
        // and check up on it in Terminal only gets them "Warning: already installed".
        assert!(!message.contains("Terminal"), "{message}");
        assert!(!message.contains("brew install pandoc"), "{message}");
    }

    /// The reason success is measured per *member*: `brew install poppler` exits 0 having laid down
    /// `pdftoppm` and `pdftotext` but not `pdftohtml`, and PDF → HTML still cannot run. Trusting the
    /// exit code would tick Poppler off and leave the user with a failing conversion and no button.
    #[test]
    fn a_formula_that_landed_two_of_three_binaries_has_not_succeeded() {
        let brew_said_fine =
            transcript(Some(0), &["==> Pouring poppler", "🍺  poppler: 484 files"]);
        let landed = |tool: Tool| tool != Tool::PdfToHtml;

        assert_eq!(
            classify(&brew_said_fine, &POPPLER, &landed),
            Outcome::Incomplete { missing: vec![Tool::PdfToHtml] }
        );
        let (ok, message) = message_for(
            &POPPLER,
            "brew install poppler",
            &classify(&brew_said_fine, &POPPLER, &landed),
        );
        assert!(!ok, "a half-installed package must never be reported as installed: {message}");
        assert!(message.contains("part of Poppler"), "{message}");
        assert!(message.contains("2 of its 3 programs"), "{message}");
        // The app already knows *what* the formula did - it counted - so the next step is its own
        // Re-check and, failing that, another install, not a trip to Terminal to be told the same.
        assert!(message.contains("Re-check"), "{message}");
        assert!(!message.contains("Terminal"), "{message}");
        // The sentence says Poppler; the log says which executable (see `run_with`).
        assert!(!message.contains("pdfto"), "{message}");

        // Every member there: that, and only that, is an install.
        assert_eq!(classify(&brew_said_fine, &POPPLER, &|_| true), Outcome::Installed);
        // None of them, and the formula claiming success: the existing "cannot find it" answer.
        assert_eq!(classify(&brew_said_fine, &POPPLER, &|_| false), Outcome::NotDiscoverable);
        // A one-binary package cannot be partial, so its answers are unchanged by any of this.
        assert_eq!(classify(&brew_said_fine, &PANDOC, &|_| true), Outcome::Installed);
    }

    /// A partial package that the installer *also* reported a failure for is still partial: the
    /// user's next step ("run it in Terminal and see") does not depend on the exit code.
    #[test]
    fn a_half_installed_package_is_not_mistaken_for_one_that_was_already_there() {
        let failed = transcript(Some(1), &["Error: Cannot link poppler"]);
        assert_eq!(
            classify(&failed, &POPPLER, &|tool| tool == Tool::PdfToPpm),
            Outcome::Incomplete { missing: vec![Tool::PdfToText, Tool::PdfToHtml] }
        );
        // ...whereas a *complete* package after a failed install is the "already installed" case.
        assert_eq!(classify(&failed, &POPPLER, &|_| true), Outcome::AlreadyThere);
    }

    #[test]
    fn a_cask_that_wanted_a_password_says_so_and_points_at_terminal() {
        let denied = transcript(
            Some(1),
            &["==> Installing Cask libreoffice", "sudo: no tty present and no askpass program"],
        );
        assert_eq!(classify(&denied, &LIBREOFFICE, &|_| false), Outcome::NeedsPassword);
        let (ok, message) = message_for(
            &LIBREOFFICE,
            "brew install --cask libreoffice",
            &classify(&denied, &LIBREOFFICE, &|_| false),
        );
        assert!(!ok);
        assert!(message.contains("administrator password"), "{message}");
        assert!(message.contains("Terminal"), "{message}");
        assert!(message.contains("brew install --cask libreoffice"), "{message}");
    }

    #[test]
    fn an_ordinary_failure_keeps_the_last_thing_the_installer_said() {
        let failed = transcript(Some(1), &["==> Downloading", "Error: No available formula"]);
        let outcome = classify(&failed, &PANDOC, &|_| false);
        assert_eq!(
            outcome,
            Outcome::Failed { code: "1".into(), detail: "Error: No available formula".into() }
        );
        let (ok, message) = message_for(&PANDOC, "brew install pandoc", &outcome);
        assert!(!ok);
        assert!(message.contains("exited with 1"), "{message}");
        assert!(message.contains("Error: No available formula"), "{message}");
    }

    /// `brew install --cask libreoffice` exits 1 when the app is already in `/Applications`. The
    /// tool *is* usable, so refusing to say so would send the user chasing a non-problem.
    #[test]
    fn a_failure_on_an_already_installed_tool_is_still_the_truth() {
        let failed = transcript(Some(1), &["Error: It seems there is already an App at ..."]);
        assert_eq!(classify(&failed, &LIBREOFFICE, &|_| true), Outcome::AlreadyThere);
        let (ok, message) =
            message_for(&LIBREOFFICE, "brew install --cask libreoffice", &Outcome::AlreadyThere);
        assert!(ok);
        assert!(message.contains("available now"), "{message}");
    }

    #[test]
    fn a_manager_that_cannot_be_launched_is_reported_as_such() {
        let t = Transcript {
            spawn_error: Some("No such file or directory".into()),
            ..Default::default()
        };
        let outcome = classify(&t, &PANDOC, &|_| false);
        let (ok, message) = message_for(&PANDOC, "brew install pandoc", &outcome);
        assert!(!ok);
        assert!(message.contains("Could not start the installer"), "{message}");
    }

    #[test]
    fn a_killed_installer_has_no_exit_code_and_is_not_a_success() {
        let t = Transcript { status: None, ..Default::default() };
        let outcome = classify(&t, &PANDOC, &|_| false);
        assert_eq!(outcome, Outcome::Failed { code: "no exit code".into(), detail: String::new() });
        assert!(!message_for(&PANDOC, "brew install pandoc", &outcome).0);
    }

    /// Nothing the settings page prints about an install may name an executable: the user asked for
    /// Poppler and has no idea what a `pdftohtml` is. Every outcome is checked, because the wording
    /// is where such a name would slip back in.
    #[test]
    fn no_install_message_ever_names_a_binary() {
        for package in convert_core::package::ALL_PACKAGES.iter().copied() {
            // A plausible partial install for this package: everything but its first binary. Empty
            // for a one-binary package, which cannot be partial in the first place.
            let missing: Vec<Tool> = package.tools.iter().copied().skip(1).collect();
            let outcomes = [
                Outcome::Installed,
                Outcome::AlreadyThere,
                Outcome::NotDiscoverable,
                Outcome::Incomplete { missing },
                Outcome::NeedsPassword,
                Outcome::Failed { code: "1".into(), detail: "Error: something".into() },
                Outcome::CouldNotStart("No such file or directory".into()),
            ];
            for outcome in &outcomes {
                let (_, message) = message_for(package, "brew install poppler", outcome);
                assert!(!message.contains("pdfto"), "{}: {message}", package.id);
                assert!(message.contains(package.name), "{}: {message}", package.id);
            }
        }
    }

    #[test]
    fn password_prompts_are_recognised_but_ordinary_lines_are_not() {
        for line in [
            "sudo: no tty present and no askpass program specified",
            "Password:",
            "==> This cask requires administrator privileges",
            "sudo access is required to install",
        ] {
            assert!(asks_for_a_password(line), "{line}");
        }
        for line in [
            "==> Downloading https://formulae.brew.sh/pandoc",
            "==> Pouring pandoc--3.1.13.arm64_sonoma.bottle.tar.gz",
            "🍺  /opt/homebrew/Cellar/pandoc/3.1.13: 12 files, 180MB",
        ] {
            assert!(!asks_for_a_password(line), "{line}");
        }
    }

    #[test]
    fn progress_bars_and_escape_codes_never_reach_the_ui() {
        assert_eq!(readable("  10%\r  55%\r 100%  "), "100%");
        assert_eq!(readable("\u{1b}[32m==> Downloading\u{1b}[0m"), "[32m==> Downloading[0m");
        assert_eq!(readable("\t\n"), "");
        assert_eq!(readable(&"x".repeat(5_000)).len(), MAX_LINE);
    }

    #[test]
    fn the_installer_gets_a_usable_path_even_when_the_app_inherited_none() {
        let path = child_path(Path::new("/opt/homebrew/bin/brew"));
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert_eq!(dirs.first().map(|d| d.as_path()), Some(Path::new("/opt/homebrew/bin")));
        assert!(dirs.iter().any(|d| d == Path::new("/usr/bin")), "{dirs:?}");
    }

    /// Both Homebrew prefixes, whichever one `brew` itself lives in.
    ///
    /// An Apple Silicon `brew` is at `/opt/homebrew/bin/brew`, and only that directory used to
    /// reach the child: a formula shelling out to something in `/usr/local/bin` - the Intel prefix,
    /// still populated on any Mac that was migrated - got "command not found" for a program sitting
    /// on the disk. The Intel case was the mirror image and was covered by accident, because there
    /// `brew`'s own directory *is* `/usr/local/bin`.
    #[test]
    fn the_installer_can_see_both_homebrew_prefixes() {
        for brew in ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"] {
            let path = child_path(Path::new(brew));
            let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
            assert_eq!(dirs.first().map(|d| d.as_path()), Path::new(brew).parent(), "{dirs:?}");
            for prefix in ["/opt/homebrew/bin", "/usr/local/bin"] {
                assert_eq!(
                    dirs.iter().filter(|d| d.as_path() == Path::new(prefix)).count(),
                    1,
                    "`{brew}` must name {prefix} exactly once: {dirs:?}"
                );
            }
        }
    }
}
