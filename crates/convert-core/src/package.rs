//! Packages: the thing a *user* installs, as distinct from the binary we execute.
//!
//! [`crate::format::Tool`] is a program: `pdftotext` either exists at a path we can spawn or it does
//! not, and planning a run for one we have not found would fail at spawn time. That precision is
//! right inside the engine and wrong in front of a person - Poppler ships `pdftoppm`, `pdftotext` and
//! `pdftohtml` in one formula, so a settings page built on tools offered three near-identical rows
//! with the same `brew install poppler` behind each, and a failed PDF → HTML told the user they
//! needed "pdftohtml", which is not a thing anybody installs.
//!
//! So there are two names for the same reality, and which one is used is decided by the audience:
//!
//! * a [`Package`] has an id, a display name and a set of member tools. It is what the settings page
//!   lists, what the install prompt asks about, and what [`crate::install`] resolves an install for;
//! * a [`Tool`] keeps its own label ("Poppler (pdftotext)") for diagnostics and `FORMATS.md`, where
//!   naming the exact executable is the useful thing to do.
//!
//! Ids here are `&'static str` written in this file and are only ever *compared* against what the
//! webview sends - see [`crate::install`] for why nothing from IPC may reach an argument.

use crate::format::Tool;
use crate::tools::ToolRegistry;

/// One installable thing, named as a user would name it.
///
/// Membership is the whole point: everything derived per-package (what it unlocks, whether it is
/// there, what one click installs) is computed from `tools`, so a fourth Poppler binary is one line
/// in this file and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Package {
    /// Stable id, and the only key the frontend ever sends back. Indexes the install allowlist.
    pub id: &'static str,
    /// The name every user-facing string uses: a settings row, the prompt, a failed row's microlink.
    pub name: &'static str,
    /// The binaries this package provides, in the order discovery and the planner prefer them.
    pub tools: &'static [Tool],
}

pub const LIBREOFFICE: Package =
    Package { id: "libreoffice", name: "LibreOffice", tools: &[Tool::LibreOffice] };
pub const PANDOC: Package = Package { id: "pandoc", name: "Pandoc", tools: &[Tool::Pandoc] };
/// The formula is `imagemagick`, the binary is `magick`: the user installs the former.
pub const IMAGEMAGICK: Package =
    Package { id: "imagemagick", name: "ImageMagick", tools: &[Tool::Magick] };
/// Three binaries, one formula, one row. `pdftoppm` rasterises, `pdftotext` and `pdftohtml` extract.
pub const POPPLER: Package = Package {
    id: "poppler",
    name: "Poppler",
    tools: &[Tool::PdfToPpm, Tool::PdfToText, Tool::PdfToHtml],
};
pub const RUFFLE: Package = Package { id: "ruffle", name: "Ruffle", tools: &[Tool::Ruffle] };
/// The one package that unlocks a *source* rather than a format: with it, a pasted YouTube or
/// Bilibili link becomes a file the ordinary pipeline converts. Formula and binary share a name.
pub const YT_DLP: Package = Package { id: "yt-dlp", name: "yt-dlp", tools: &[Tool::YtDlp] };
/// The runtime yt-dlp needs before YouTube will hand a video over, and the second package that
/// unlocks no format at all. Formula and binary share a name, as with yt-dlp.
///
/// [`Tool::Node`] is deliberately *not* a member: it is a runtime we will happily use if the machine
/// already has it, and installing a JavaScript toolchain on somebody's behalf is not this app's
/// business - so there is one row, and one click, and it installs Deno.
pub const DENO: Package = Package { id: "deno", name: "Deno", tools: &[Tool::Deno] };

/// Every package, in the order the settings page lists them (which is [`crate::tools::ALL_TOOLS`]
/// order, taken at each package's first member).
///
/// Bundled FFmpeg/ffprobe, the built-in `sips` and [`Tool::Node`] are deliberately absent: there is
/// nothing for this app to install, so they must never appear as something a user could install.
pub const ALL_PACKAGES: &[&Package] =
    &[&LIBREOFFICE, &PANDOC, &IMAGEMAGICK, &POPPLER, &RUFFLE, &YT_DLP, &DENO];

/// How much of a package is actually on this machine.
///
/// [`Presence::Partial`] is real and has to be said out loud: `brew install poppler` normally lands
/// all three binaries, but a half-finished install, a pruned Cellar or a hand-built copy can leave
/// one behind - and the route that needs *that* binary cannot run. A partial package is therefore
/// **not installed** (it may not claim to be) and **still installable** (one click is the fix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Every member binary was found.
    Complete,
    /// Some were found, some were not.
    Partial,
    /// None were found.
    Absent,
}

impl Package {
    /// The package this id names, or `None`. The allowlist gate: the id is compared and discarded.
    pub fn by_id(id: &str) -> Option<&'static Package> {
        ALL_PACKAGES.iter().copied().find(|p| p.id == id)
    }

    pub fn provides(&self, tool: Tool) -> bool {
        self.tools.contains(&tool)
    }

    pub fn presence(&self, tools: &ToolRegistry) -> Presence {
        let found = self.tools.iter().copied().filter(|t| tools.has(*t)).count();
        match found {
            0 => Presence::Absent,
            n if n == self.tools.len() => Presence::Complete,
            _ => Presence::Partial,
        }
    }

    /// Is the whole package here? Anything less is a package to offer, not one to tick off.
    pub fn is_installed(&self, tools: &ToolRegistry) -> bool {
        self.presence(tools) == Presence::Complete
    }
}

impl Tool {
    /// The package that installs this binary, or `None` for the bundled engine and macOS `sips`.
    pub fn package(self) -> Option<&'static Package> {
        ALL_PACKAGES.iter().copied().find(|p| p.provides(self))
    }

    /// The name to say to a user: the package's, or - for a tool nothing installs - the tool's own.
    ///
    /// Every user-facing string about a missing helper goes through here, which is what stops a
    /// planner error or a picker option from spelling out an executable nobody would recognise.
    pub fn user_facing_name(self) -> &'static str {
        match self.package() {
            Some(package) => package.name,
            None => self.label(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ALL_TOOLS;
    use std::path::PathBuf;

    #[test]
    fn every_tool_belongs_to_at_most_one_package() {
        for tool in ALL_TOOLS.iter().copied() {
            let owners: Vec<&str> =
                ALL_PACKAGES.iter().filter(|p| p.provides(tool)).map(|p| p.id).collect();
            assert!(owners.len() <= 1, "{} is claimed by {:?}", tool.id(), owners);
        }
        // A package with no members would be a row that installs nothing.
        for package in ALL_PACKAGES {
            assert!(!package.tools.is_empty(), "{} provides nothing", package.id);
            assert!(!package.name.is_empty());
        }
    }

    /// The two audiences, side by side: `label` is for us, `user_facing_name` is for them.
    #[test]
    fn a_user_is_told_the_package_and_a_log_is_told_the_binary() {
        assert_eq!(Tool::PdfToHtml.label(), "Poppler (pdftohtml)");
        assert_eq!(Tool::PdfToHtml.user_facing_name(), "Poppler");
        assert_eq!(Tool::Magick.user_facing_name(), "ImageMagick");
        // Nothing installs these, so their own label is the honest answer.
        assert_eq!(Tool::Sips.user_facing_name(), "macOS sips");
        assert_eq!(Tool::Ffmpeg.user_facing_name(), "FFmpeg (bundled)");
        for tool in ALL_TOOLS.iter().copied() {
            assert!(
                !tool.user_facing_name().contains("pdfto"),
                "{} names a binary to a user",
                tool.id()
            );
        }
    }

    #[test]
    fn ids_are_stable_and_only_ever_looked_up() {
        assert_eq!(Package::by_id("poppler").map(|p| p.name), Some("Poppler"));
        for crafted in ["Poppler", "poppler ", "pdftohtml", "", "poppler;rm -rf ~"] {
            assert!(Package::by_id(crafted).is_none(), "`{crafted}` must not resolve");
        }
    }

    /// The awkward middle: `pdftoppm` there, the other two not. Not installed, still installable.
    #[test]
    fn a_half_installed_package_is_neither_present_nor_hidden() {
        let mut registry = ToolRegistry::default();
        assert_eq!(POPPLER.presence(&registry), Presence::Absent);

        registry.set(Tool::PdfToPpm, PathBuf::from("/opt/homebrew/bin/pdftoppm"));
        assert_eq!(POPPLER.presence(&registry), Presence::Partial);
        assert!(!POPPLER.is_installed(&registry), "a partial package must not claim to be there");

        registry.set(Tool::PdfToText, PathBuf::from("/opt/homebrew/bin/pdftotext"));
        assert_eq!(POPPLER.presence(&registry), Presence::Partial);
        registry.set(Tool::PdfToHtml, PathBuf::from("/opt/homebrew/bin/pdftohtml"));
        assert_eq!(POPPLER.presence(&registry), Presence::Complete);
        assert!(POPPLER.is_installed(&registry));

        // A one-binary package is complete the moment its binary is found.
        assert_eq!(PANDOC.presence(&registry), Presence::Absent);
        registry.set(Tool::Pandoc, PathBuf::from("/opt/homebrew/bin/pandoc"));
        assert!(PANDOC.is_installed(&registry));
    }
}
