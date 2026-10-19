//! Which browser macOS would open a link with.
//!
//! One question, one file, no network and no launching of anything. It is here rather than in
//! `convert-core` because it is the one piece of this feature that is *about* macOS: a plist at a
//! fixed path, in a format only Apple writes.
//!
//! The rule it implements is in the core ([`default_browser_from_handler`]), where it can be tested
//! on any machine; this module does nothing but hand it the bundle identifier out of the file.
//!
//! ## Why the file, and why an absent key is an answer
//!
//! macOS keeps its URL scheme handlers in the LaunchServices database. There is no supported API
//! for reading it from Rust, but there is no need for one either: when a user *changes* their
//! default browser, LaunchServices writes the choice to
//! `~/Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist` as an
//! `LSHandlers` entry with `LSHandlerURLScheme = https`. A user who never changed it has no such
//! entry - and their default browser is Safari, which is what macOS ships.
//!
//! That last sentence is the fix. This app used to read the absent entry as "unknowable" and offer
//! the user "any browser that is not Safari"; the user whose bug this is has no `https` entry, no
//! `http` entry, a Safari full of YouTube cookies and a Chrome that has never been signed in to
//! anything. Absence was the answer all along.
//!
//! Measured on macOS 26.6.2: the file is 830 bytes, plain-`stat`able and plain-readable without
//! Full Disk Access, and holds six URL scheme handlers, none of them `https` or `http`.

use convert_core::settings::default_browser_from_handler;
use std::path::{Path, PathBuf};

/// The plist LaunchServices writes a changed default browser into, relative to `$HOME`.
pub const SECURE_HANDLERS_PLIST: &str =
    "Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist";

/// The schemes a web link uses, in the order they are believed. `https` is what a modern default
/// browser change writes; `http` is checked after it for the handful of Macs that only have that.
const WEB_SCHEMES: [&str; 2] = ["https", "http"];

/// Where the plist is on this Mac, or `None` when there is no `HOME` to expand.
pub fn secure_handlers_plist() -> Option<PathBuf> {
    convert_core::tools::home_dir().map(|home| home.join(SECURE_HANDLERS_PLIST))
}

/// The allowlisted browser this Mac opens `https://` links with, if it is one we can borrow from.
///
/// Three answers, and the middle one is the fix:
///
/// * a recorded `https` (or `http`) handler that is on the allowlist - that browser;
/// * **no handler recorded at all, on macOS - Safari**, because that is what an untouched Mac
///   opens links with and an absent entry is how LaunchServices spells "untouched";
/// * a handler for something we cannot borrow cookies from (Arc, Orion), or any of the above on a
///   platform that is not macOS - `None`, because Apple's rule about absence is Apple's alone and
///   claiming Safari on a Linux build would be the same confident guess in the other direction.
pub fn default_browser() -> Option<&'static str> {
    let handler = secure_handlers_plist().and_then(|path| web_scheme_handler(&path));
    if cfg!(target_os = "macos") {
        default_browser_from_handler(handler.as_deref())
    } else {
        default_browser_from_handler(Some(handler.as_deref()?))
    }
}

/// The bundle identifier registered for `https` (or, failing that, `http`) in one plist.
///
/// `None` covers every way of not having an answer - no file, a file that is not a plist, no
/// `LSHandlers` array, no web scheme in it - because they all mean the same thing here: the user
/// never changed their default browser.
///
/// Not gated to macOS, so that the parse and the rule behind it are tested wherever the suite runs;
/// only [`default_browser`] is, because only *it* claims to know what an absent entry means.
fn web_scheme_handler(path: &Path) -> Option<String> {
    let value = plist::Value::from_file(path).ok()?;
    let handlers = value.as_dictionary()?.get("LSHandlers")?.as_array()?;
    WEB_SCHEMES.iter().find_map(|scheme| {
        handlers.iter().find_map(|handler| {
            let handler = handler.as_dictionary()?;
            let registered = handler.get("LSHandlerURLScheme")?.as_string()?;
            if !registered.eq_ignore_ascii_case(scheme) {
                return None;
            }
            // `LSHandlerRoleAll` is what a browser registers. `LSHandlerRoleViewer` is the older
            // spelling and still turns up on upgraded Macs.
            let role = handler
                .get("LSHandlerRoleAll")
                .or_else(|| handler.get("LSHandlerRoleViewer"))?
                .as_string()?;
            Some(role.to_string())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An XML plist with the handlers named, written where a test can point at it. `plist` reads
    /// XML and Apple's binary format with the same call, so this exercises the real parse.
    fn plist_with(handlers: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "flint-ls-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let path = dir.join("com.apple.launchservices.secure.plist");
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
             <key>LSHandlers</key><array>{handlers}</array></dict></plist>"
        );
        let mut file = std::fs::File::create(&path).expect("a temp plist");
        file.write_all(body.as_bytes()).expect("write");
        path
    }

    fn handler(scheme: &str, bundle_id: &str) -> String {
        format!(
            "<dict><key>LSHandlerURLScheme</key><string>{scheme}</string>\
             <key>LSHandlerRoleAll</key><string>{bundle_id}</string></dict>"
        )
    }

    /// The user's own plist, reproduced: six handlers for apps that are not browsers, and no web
    /// scheme anywhere in it. That absence is the whole bug - it used to read as "unknowable".
    #[test]
    fn a_plist_with_no_web_handler_means_the_default_browser_is_safari() {
        let schemes = ["seal", "codex", "aime", "corplink", "feilian", "sealsuite"];
        let body: String =
            schemes.iter().map(|s| handler(s, "com.volcengine.corplink")).collect::<String>();
        let path = plist_with(&body);
        assert_eq!(web_scheme_handler(&path), None, "no https handler is recorded");
        assert_eq!(default_browser_from_handler(None), Some("safari"));

        // A file that is not there, and one that is not a plist, are the same absence.
        assert_eq!(web_scheme_handler(Path::new("/nope/does-not-exist.plist")), None);
        let junk = plist_with("</array></dict></plist> not a plist at all");
        assert_eq!(web_scheme_handler(&junk), None);
    }

    /// A Mac whose owner did change their default browser, in every spelling LaunchServices uses.
    #[test]
    fn a_recorded_https_handler_names_the_browser_it_belongs_to() {
        let path = plist_with(&format!(
            "{}{}",
            handler("http", "com.google.chrome"),
            handler("https", "com.google.chrome")
        ));
        assert_eq!(web_scheme_handler(&path).as_deref(), Some("com.google.chrome"));
        assert_eq!(default_browser_from_handler(Some("com.google.chrome")), Some("chrome"));

        // `http` alone is still an answer, for a Mac that only has the older entry.
        let http_only = plist_with(&handler("http", "org.mozilla.firefox"));
        assert_eq!(web_scheme_handler(&http_only).as_deref(), Some("org.mozilla.firefox"));

        // A default we cannot borrow cookies from is `None`, not a guess at the next best thing.
        let arc = plist_with(&handler("https", "company.thebrowser.Browser"));
        assert_eq!(web_scheme_handler(&arc).as_deref(), Some("company.thebrowser.Browser"));
        assert_eq!(default_browser_from_handler(Some("company.thebrowser.Browser")), None);
    }

    /// The machine this build is running on, whatever it is: read-only, and never a panic.
    #[test]
    fn this_machines_own_default_browser_is_read_without_a_permission() {
        let path = secure_handlers_plist().expect("a HOME");
        assert!(path.ends_with(SECURE_HANDLERS_PLIST), "{path:?}");
        // Whatever it says, the answer is one of the allowlist or nothing at all.
        if let Some(id) = default_browser() {
            assert_eq!(convert_core::settings::cookie_browser(id), Some(id));
        }
    }
}
