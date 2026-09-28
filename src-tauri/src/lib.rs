//! Flint - Tauri shell.
//!
//! This crate owns exactly three things: process/OS integration (sidecars, Finder, config dir),
//! the application state, and the IPC surface. All conversion logic lives in `convert-core`, which
//! is why porting to Windows is a packaging exercise and porting to mobile only replaces the
//! *execution* layer.

mod commands;
mod install;
mod ipc;
mod launch_services;
mod menu;
mod settings_store;
mod sidecar;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows_shell;

use convert_core::{Engine, Settings, Tool, ToolRegistry, ToolStatus};
use serde::Serialize;
use sidecar::Sidecars;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockWriteGuard};
use tauri::{AppHandle, Manager, Runtime};

/// Event name every [`convert_core::BatchEvent`] is emitted on.
///
/// The `://` prefix is a convention that keeps app events from ever colliding with Tauri's own
/// (`tauri://…`) or a plugin's event names, and makes them obvious in the webview inspector.
pub const BATCH_EVENT: &str = "batch://event";

/// Event name every [`install::InstallEvent`] is emitted on, same convention as [`BATCH_EVENT`].
pub const INSTALL_EVENT: &str = "install://event";

/// Everything the commands share. Held by Tauri as managed state.
pub struct AppState {
    /// Swapped wholesale by `refresh_tools`; readers clone the `Arc` and never block a conversion.
    engine: RwLock<Arc<Engine>>,
    /// Where the bundled FFmpeg/ffprobe were found at startup, kept for the launch-time warning in
    /// `configure`'s setup hook and for nothing else. Every later rediscovery
    /// ([`AppState::refresh_tools`]) hunts again rather than reusing this snapshot.
    sidecars: Sidecars,
    /// `None` until the first `get_settings`, which lazily reads the config file.
    settings: Mutex<Option<Settings>>,
    /// Replaced by a fresh flag on every `start_batch`. Replacing instead of resetting means a
    /// batch that is still winding down after a cancel can never be "un-cancelled" by the next one.
    cancel: Mutex<Arc<AtomicBool>>,
    /// Single-batch guard: the UI has one queue and one progress bar, so a second concurrent batch
    /// would produce interleaved events for rows the frontend has already retired.
    ///
    /// It holds the *id* of the batch that owns the slot (0 = free) rather than a bool, because a
    /// finishing batch frees the slot from two places (its last event and a `Drop` on the worker)
    /// and a bool cannot tell "my slot" from "the slot my successor already claimed".
    slot: AtomicU64,
    /// Hands out those ids. Only ever incremented.
    batches: AtomicU64,
    /// Single-install guard, the same shape as `slot` and for the same reason: two `brew install`
    /// runs at once fight over Homebrew's own lock and interleave their `log` events into one
    /// unreadable stream. Holds the id of the install that owns it (0 = free), so a release from a
    /// finished install can never free its successor's.
    install_slot: AtomicU64,
    /// Hands out install ids. Only ever incremented.
    installs: AtomicU64,
}

/// The single batch slot, claimed by [`AppState::begin_batch`].
pub struct BatchSlot {
    /// Identifies this batch when it later releases the slot.
    pub id: u64,
    /// The flag `run_batch` polls; set by [`AppState::cancel`].
    pub cancel: Arc<AtomicBool>,
}

impl AppState {
    fn new() -> Self {
        let sidecars = sidecar::locate();
        let engine = Engine::new(registry_with_sidecars(&sidecars));
        Self {
            engine: RwLock::new(Arc::new(engine)),
            sidecars,
            settings: Mutex::new(None),
            cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
            slot: AtomicU64::new(0),
            batches: AtomicU64::new(0),
            install_slot: AtomicU64::new(0),
            installs: AtomicU64::new(0),
        }
    }

    /// Current engine. Cheap: one `Arc` clone.
    pub fn engine(&self) -> Arc<Engine> {
        // A read lock cannot be poisoned by *this* function, but it can be by any writer that
        // panicked, and an engine handle is on the path of every single command.
        self.engine.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Re-run helper discovery (LibreOffice may have been installed while the app was open) and
    /// publish a new engine built from it.
    ///
    /// The sidecars are *located again*, not replayed from [`AppState::sidecars`]. Re-check is the
    /// button a user presses after fixing something, and the bundled engine is one of the things
    /// that can be fixed: a `.app` whose sidecar was quarantined, stripped by a copy that lost the
    /// executable bit, or simply not fetched yet in a dev tree. Rebuilding from the startup
    /// snapshot answered that press with the state of the machine as it was at launch - "FFmpeg is
    /// still missing" over a directory that now holds it, and no way out but restarting the app.
    pub fn refresh_tools(&self) -> Vec<ToolStatus> {
        self.refresh_tools_with(sidecar::locate)
    }

    /// [`AppState::refresh_tools`] with the sidecar hunt injected.
    ///
    /// The seam exists for the test: [`sidecar::locate`] only ever looks in the directories a real
    /// bundle or dev build uses, so "a sidecar appeared after launch" cannot be staged in a scratch
    /// folder without it - and the regression this guards is precisely that the *startup* snapshot
    /// is not what gets used.
    fn refresh_tools_with(&self, locate: impl FnOnce() -> Sidecars) -> Vec<ToolStatus> {
        let registry = registry_with_sidecars(&locate());
        let statuses = registry.statuses();
        let mut slot = write_lock(&self.engine);
        *slot = Arc::new(Engine::new(registry));
        statuses
    }

    /// Settings as the user last left them, loading from disk exactly once.
    pub fn settings<R: Runtime>(&self, app: &AppHandle<R>) -> Settings {
        let mut slot = lock(&self.settings);
        if let Some(existing) = slot.as_ref() {
            return existing.clone();
        }
        let loaded = settings_store::load(app).unwrap_or_default();
        *slot = Some(loaded.clone());
        loaded
    }

    /// Persist and cache in one step, so the in-memory copy can never drift from the file.
    ///
    /// The lock is held across the write, which is what makes two concurrent `save_settings` calls
    /// (the UI debounces, but a preset click and a slider drag can still overlap) apply in one
    /// order to both the file and the cache. Releasing it first let the *second* writer win the
    /// file while the *first* won the cache, so the next launch reverted a change the window was
    /// still showing.
    pub fn store_settings<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        settings: Settings,
    ) -> Result<Settings, String> {
        let mut slot = lock(&self.settings);
        settings_store::save(app, &settings)?;
        *slot = Some(settings.clone());
        Ok(settings)
    }

    /// [`AppState::store_settings`] for a payload that came from the webview: every field the UI
    /// sent is persisted on its own merits, and only the *destination* can be held back.
    ///
    /// Validating the object as a whole was a real data loss: a user who changed a codec in the
    /// settings drawer while the output destination was still half-chosen ("Custom folder", no
    /// folder picked yet) had the codec silently dropped, because the destination refused the whole
    /// save. [`settings_store::merge`] draws the line - unfinished destinations are held, hostile
    /// ones are repaired and reported.
    ///
    /// Note the deliberate shape of the failure: an `Err` here means *the destination* was refused,
    /// and the rest of the object has still been written. The caller's error text is what stops the
    /// settings page from quietly lying about where files are going to land.
    pub fn store_settings_from_ui<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        incoming: Settings,
    ) -> Result<Settings, String> {
        // One lock for read-merge-write, exactly as `store_settings` holds it across the write: two
        // overlapping saves must not merge against a destination the other one has already replaced.
        let mut slot = lock(&self.settings);
        let current = match slot.as_ref() {
            Some(existing) => existing.clone(),
            // First save of the session (the UI can save before it ever reads): the file, or the
            // defaults, is what an unfinished destination falls back to.
            None => settings_store::load(app).unwrap_or_default(),
        };
        let (merged, refused) = settings_store::merge(&current, incoming);
        settings_store::save(app, &merged)?;
        *slot = Some(merged.clone());
        match refused {
            Some(why) => Err(why),
            None => Ok(merged),
        }
    }

    /// Claim the single batch slot, returning the batch's id and a fresh cancellation flag.
    ///
    /// The id must be handed back to [`AppState::end_batch`]; that is what makes releasing the slot
    /// idempotent *and* safe against a late release from a previous batch.
    pub fn begin_batch(&self) -> Result<BatchSlot, String> {
        // Allocated before the claim, so a losing caller burns an id rather than reusing one. Ids
        // only have to be unique, never contiguous.
        let id = self.batches.fetch_add(1, Ordering::SeqCst) + 1;
        // The cancel flag is swapped under the same lock a `cancel()` takes, and the lock is held
        // across the claim: a Stop that arrives while a batch is starting either flips the previous
        // batch's flag (it arrived first) or this one's - never a flag nobody is polling.
        let mut current = lock(&self.cancel);
        if self.slot.compare_exchange(0, id, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err("A conversion is already running. Cancel it first.".into());
        }
        let fresh = Arc::new(AtomicBool::new(false));
        *current = fresh.clone();
        Ok(BatchSlot { id, cancel: fresh })
    }

    /// Release the batch slot, but only if `id` still owns it.
    ///
    /// Called as soon as the batch's final event is on its way out, and again from a `Drop` guard on
    /// the worker thread so a panic cannot latch the slot shut. Those two paths mean a batch can
    /// release twice, and the second release can land *after* its successor has already claimed the
    /// slot - which with a plain flag would free the successor's slot and let a third batch run
    /// concurrently with it, interleaving events for rows the UI has already retired. Comparing the
    /// id makes the late release a no-op.
    pub fn end_batch(&self, id: u64) {
        let _ = self.slot.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Whether a batch currently owns the slot.
    pub fn is_running(&self) -> bool {
        self.slot.load(Ordering::SeqCst) != 0
    }

    /// Claim the single-install slot, returning the id that must be handed to
    /// [`AppState::end_install`]. Same id-keyed discipline as [`AppState::begin_batch`].
    pub fn begin_install(&self) -> Result<u64, String> {
        let id = self.installs.fetch_add(1, Ordering::SeqCst) + 1;
        if self.install_slot.compare_exchange(0, id, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err("Another helper is being installed. Wait for it to finish.".into());
        }
        Ok(id)
    }

    /// Release the install slot, but only if `id` still owns it.
    pub fn end_install(&self, id: u64) {
        let _ = self.install_slot.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Whether an install currently owns the slot.
    pub fn is_installing(&self) -> bool {
        self.install_slot.load(Ordering::SeqCst) != 0
    }

    /// Ask the running batch to stop. Harmless when nothing is running.
    pub fn cancel(&self) {
        lock(&self.cancel).store(true, Ordering::SeqCst);
    }

    /// What this process is busy with, for a webview that has just been reloaded and cannot know.
    pub fn activity(&self) -> Activity {
        Activity { converting: self.is_running(), installing: self.is_installing() }
    }
}

/// Answer to `get_activity`: the two long-running things the shell owns, as booleans.
///
/// A reload throws away every `batch://event` listener and the whole UI store, but not the batch:
/// the window came back believing it was idle while a conversion it could neither see nor stop
/// still owned the slot, and the next Convert was refused with "a conversion is already running".
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Activity {
    pub converting: bool,
    pub installing: bool,
}

/// Lock a mutex, recovering from poisoning.
///
/// Every mutex in here guards a value that is *replaced*, never mutated in halves, so a panic
/// elsewhere cannot leave one of them half-written - whereas refusing to lock a poisoned mutex
/// turned one panic into a window where settings, cancellation and (through the engine) every
/// conversion stayed broken until the app was restarted.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// [`lock`] for the engine's `RwLock`.
fn write_lock<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

/// Discovery + explicit sidecar registration.
///
/// `discover` already searches the sidecar directory first, but it only knows the plain
/// `ffmpeg`/`ffprobe` names - in `cargo tauri dev` the files carry a `-<triple>` suffix, so we set
/// those paths explicitly afterwards. A machine with neither sidecar nor system FFmpeg still gets a
/// usable registry: the catalog simply reports the affected formats as unavailable.
fn registry_with_sidecars(sidecars: &Sidecars) -> ToolRegistry {
    let mut registry = ToolRegistry::discover(sidecars.dir.as_deref());
    if let Some(path) = sidecars.ffmpeg.clone() {
        registry.set(Tool::Ffmpeg, path);
    }
    if let Some(path) = sidecars.ffprobe.clone() {
        registry.set(Tool::Ffprobe, path);
    }
    registry
}

/// Register plugins, state and the IPC surface on a builder.
///
/// Split out of [`run`] so the integration tests can drive the *exact* same command set on Tauri's
/// mock runtime - a handler list that only exists in `run()` is a handler list nothing tests.
fn configure<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        // Dialog only: the file picker is the one native surface the webview needs. Opening and
        // revealing files is done by our own commands, which validate the path first.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .setup(|app| {
            // The window has no chrome of its own by design, so the menu bar is the only place
            // Settings, Open and Convert live. A menu that fails to build must not stop the app:
            // every one of its commands is also reachable from the UI or a webview key handler.
            match menu::build(app.handle()) {
                Ok(menu) => {
                    if let Err(e) = app.set_menu(menu) {
                        eprintln!("[flint] could not install the menu bar: {e}");
                    }
                }
                Err(e) => eprintln!("[flint] could not build the menu bar: {e}"),
            }

            // Warn in the log - never a dialog, never a panic. The UI already renders a banner
            // from `get_catalog().tools`, and the app is still useful for the format browser.
            if let Some(state) = app.try_state::<AppState>() {
                if !state.sidecars.is_complete() {
                    eprintln!(
                        "[flint] FFmpeg sidecar not found (looked for ffmpeg/ffmpeg-{}). \
                         Run scripts/fetch-sidecars.sh; falling back to any FFmpeg on PATH.",
                        sidecar::TARGET_TRIPLE
                    );
                }
            }
            Ok(())
        })
        .on_menu_event(|app, event| menu::forward(app, event.id().as_ref()))
        .invoke_handler(tauri::generate_handler![
            commands::get_catalog,
            commands::get_settings,
            commands::save_settings,
            commands::apply_preset,
            commands::inspect_files,
            commands::inspect_links,
            commands::get_link_support,
            commands::list_cookie_browsers,
            commands::check_safari_cookie_access,
            commands::test_cookie_source,
            commands::start_batch,
            commands::cancel_batch,
            commands::refresh_tools,
            commands::get_activity,
            commands::get_install_plans,
            commands::install_tool,
            commands::reveal_in_finder,
            commands::open_path,
            commands::open_full_disk_access_settings,
            commands::estimate_output_path,
        ])
}

/// Build and run the desktop app.
///
/// # Panics
/// Only if Tauri itself cannot create a window, which is unrecoverable and must be loud.
fn app_context<R: Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

pub fn run() {
    configure(tauri::Builder::default())
        .run(app_context())
        .expect("failed to start the Flint window");
}
