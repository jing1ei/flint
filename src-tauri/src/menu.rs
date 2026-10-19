//! The native menu bar.
//!
//! The window itself is deliberately almost empty - the first thing a user sees is a single drop
//! surface with no chrome at all - so every command that is not "convert these files" lives up
//! here instead. That is also the macOS-native place for them: Settings belongs behind ⌘, in the
//! application menu, not behind a gear icon competing for attention with the artwork.
//!
//! Clicking an item emits [`MENU_EVENT`] with the item's id as the payload. The frontend owns what
//! each one *means*; this module only decides what exists and which key opens it.

use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Runtime};

/// Event carrying the id of the menu item the user picked.
pub const MENU_EVENT: &str = "menu://action";

/// Ids the frontend listens for. Kept as constants so a typo is a compile error on this side and
/// a single grep on the other.
pub mod action {
    pub const SETTINGS: &str = "settings";
    pub const SKIN: &str = "skin";
    pub const OPEN_FILES: &str = "open_files";
    pub const OPEN_FOLDER: &str = "open_folder";
    pub const PASTE_LINKS: &str = "paste_links";
    pub const CONVERT: &str = "convert";
    pub const STOP: &str = "stop";
    pub const CLEAR: &str = "clear";
}

pub const ACTION_ITEMS: &[(&str, &str, &str)] = &[
    (action::SETTINGS, "Settings…", "CmdOrCtrl+,"),
    (action::SKIN, "Customize Skin…", "CmdOrCtrl+Shift+,"),
    (action::OPEN_FILES, "Open…", "CmdOrCtrl+O"),
    (action::OPEN_FOLDER, "Open Folder…", "CmdOrCtrl+Shift+O"),
    (action::PASTE_LINKS, "Paste Links…", "CmdOrCtrl+L"),
    (action::CONVERT, "Convert", "CmdOrCtrl+Return"),
    (action::STOP, "Stop", "CmdOrCtrl+."),
    (action::CLEAR, "Clear List", "CmdOrCtrl+Shift+Backspace"),
];

fn action_item<R: Runtime>(app: &AppHandle<R>, id: &str) -> tauri::Result<MenuItem<R>> {
    let (_, label, accelerator) =
        ACTION_ITEMS.iter().find(|item| item.0 == id).expect("every menu action has a definition");
    MenuItem::with_id(app, id, *label, true, Some(*accelerator))
}

/// Build the whole menu bar.
///
/// Only cross-platform predefined items are used: a Windows build gets the same commands without
/// a second implementation, and the macOS-only niceties Tauri would add (Services, Hide Others)
/// are not worth a `cfg` fork of this list.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let about = AboutMetadata {
        name: Some("Flint".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        comments: Some(
            "Offline all-in-one converter for video, audio, images, documents, subtitles and Flash."
                .into(),
        ),
        ..Default::default()
    };

    let app_menu = Submenu::with_items(
        app,
        "Flint",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("About Flint"), Some(about))?,
            &PredefinedMenuItem::separator(app)?,
            &action_item(app, action::SETTINGS)?,
            &action_item(app, action::SKIN)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some("Quit Flint"))?,
        ],
    )?;

    let file_menu = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &action_item(app, action::OPEN_FILES)?,
            &action_item(app, action::OPEN_FOLDER)?,
            // A pasted link is a third way of naming a source, so it belongs beside the other two
            // rather than in a menu of its own: ⌘L opens the box, and ⌘V anywhere fills it in.
            &action_item(app, action::PASTE_LINKS)?,
            &PredefinedMenuItem::separator(app)?,
            // ⌘↩ to start and ⌘. to stop are the two shortcuts a converter is actually used with.
            &action_item(app, action::CONVERT)?,
            &action_item(app, action::STOP)?,
            &PredefinedMenuItem::separator(app)?,
            &action_item(app, action::CLEAR)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    // Present so the number fields in the settings sheet get the system's text editing bindings.
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let window_menu =
        Submenu::with_items(app, "Window", true, &[&PredefinedMenuItem::minimize(app, None)?])?;

    Menu::with_items(app, &[&app_menu, &file_menu, &edit_menu, &window_menu])
}

/// Forward a menu click to the webview.
///
/// Predefined items (Quit, Copy, …) are handled by the OS and never reach here, so anything that
/// does is one of our own ids.
pub fn forward<R: Runtime>(app: &AppHandle<R>, id: &str) {
    if let Err(e) = app.emit(MENU_EVENT, id) {
        eprintln!("[flint] could not deliver menu action `{id}`: {e}");
    }
}
