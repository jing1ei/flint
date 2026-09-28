//! # convert-core
//!
//! Everything Flint knows how to do, with no GUI and no platform assumptions:
//!
//! * [`format`] - the catalog of every supported format (the single source of truth)
//! * [`settings`] - user settings; `Settings::default()` *is* the one-click preset
//! * [`plan`] - turns (source, target, settings) into concrete command lines
//! * [`engine`] - runs those commands, streams progress, supports cancellation
//! * [`queue`] - batch orchestration over a small worker pool
//! * [`probe`] / [`progress`] / [`paths`] / [`tools`] - the supporting pure logic
//! * [`package`] - the *packages* a user installs, as against the binaries the engine runs
//! * [`install`] - the allowlist of optional helpers the app may install, and what each unlocks
//! * [`link`] - pasted video links as a second kind of source, validated before they are argv
//!
//! The Tauri layer (`src-tauri`) is a thin shell over this crate, which is what makes a Windows
//! build - or an iOS/Android build - a packaging job rather than a rewrite.

pub mod crop;
pub mod engine;
pub mod format;
pub mod install;
pub mod link;
pub mod midi;
pub mod package;
pub mod paths;
pub mod plan;
pub mod probe;
pub mod progress;
pub mod queue;
pub mod report;
pub mod settings;
pub mod tools;

/// Facade for the handful of items the app shell reaches for by name. Everything else stays
/// reachable through its own module (`convert_core::plan::plan`, `convert_core::engine::Engine`),
/// so this list is deliberately what is actually used rather than a mirror of the whole crate.
pub use engine::Engine;
pub use format::{by_id, by_path, Tool};
pub use package::{Package, Presence};
pub use probe::MediaInfo;
pub use queue::{run_batch, BatchEvent, BatchItem};
pub use settings::{Preset, Settings};
pub use tools::{ToolRegistry, ToolStatus};

use format::{readable_in, writable_in, Category, Format, Support};
use plan::{default_target_for, output_extension, suggested_targets_for};
use serde::Serialize;

/// Everything the UI needs to render its pickers, in one payload.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogView {
    pub categories: Vec<CategoryView>,
    pub presets: Vec<PresetView>,
    pub tools: Vec<ToolStatus>,
    /// Total number of accepted input extensions - shown on the empty drop zone.
    pub input_extension_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryView {
    pub id: &'static str,
    pub label: &'static str,
    pub default_target: &'static str,
    pub suggested_targets: Vec<&'static str>,
    pub inputs: Vec<FormatView>,
    pub outputs: Vec<FormatView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormatView {
    pub id: &'static str,
    pub name: &'static str,
    pub extension: &'static str,
    pub extensions: Vec<&'static str>,
    pub notes: &'static str,
    /// `false` when a helper tool is missing on this machine.
    pub available: bool,
    /// The helpers that can do this format, named the way a person installs them ("Poppler", not
    /// `pdftotext`), each named once even when a package ships three binaries for the job.
    pub needs: Vec<&'static str>,
}

/// Build the payload the frontend renders, marking anything that needs a missing helper.
pub fn catalog_view(tools: &ToolRegistry) -> CatalogView {
    let mut categories = Vec::new();
    for c in Category::ALL {
        let inputs =
            readable_in(c).into_iter().map(|f| format_view(f, f.read, tools)).collect::<Vec<_>>();
        let outputs =
            writable_in(c).into_iter().map(|f| format_view(f, f.write, tools)).collect::<Vec<_>>();
        categories.push(CategoryView {
            id: c.id(),
            label: c.label(),
            default_target: default_target_for(c),
            suggested_targets: suggested_targets_for(c).to_vec(),
            inputs,
            outputs,
        });
    }

    CatalogView {
        categories,
        presets: Preset::ALL
            .iter()
            .map(|p| PresetView { id: p.id(), label: p.label(), description: p.description() })
            .collect(),
        tools: tools.statuses(),
        input_extension_count: format::readable_extension_count(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PresetView {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

fn format_view(f: &'static Format, support: Support, tools: &ToolRegistry) -> FormatView {
    // "PDF - needs LibreOffice or Poppler or …": the reader installs packages, so the helper list
    // is package names, and Poppler's three binaries collapse into the one name.
    let mut needs: Vec<&'static str> = Vec::new();
    for helper in support.helpers() {
        let name = helper.user_facing_name();
        if !needs.contains(&name) {
            needs.push(name);
        }
    }
    let available = match support {
        Support::Bundled => true,
        Support::AnyOf(list) => tools.first_available(list).is_some(),
        Support::Unsupported => false,
    };
    FormatView {
        id: f.id,
        name: f.name,
        extension: output_extension(f),
        extensions: f.extensions.to_vec(),
        notes: f.notes,
        available,
        needs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_view_marks_helper_formats_unavailable_on_a_bare_machine() {
        let mut bare = ToolRegistry::default();
        bare.set(Tool::Ffmpeg, "/app/ffmpeg".into());
        bare.set(Tool::Ffprobe, "/app/ffprobe".into());
        let view = catalog_view(&bare);

        let images = view.categories.iter().find(|c| c.id == "image").unwrap();
        let jpg = images.outputs.iter().find(|f| f.id == "jpg").unwrap();
        assert!(jpg.available, "JPEG must work with nothing installed");

        let docs = view.categories.iter().find(|c| c.id == "document").unwrap();
        let pdf = docs.outputs.iter().find(|f| f.id == "pdf").unwrap();
        assert!(!pdf.available, "PDF needs LibreOffice");
        assert_eq!(pdf.needs, vec!["LibreOffice"]);

        assert!(view.input_extension_count > 150);
        assert_eq!(view.presets.len(), 4);
        assert!(view.tools.iter().any(|t| t.id == "ffmpeg" && t.available));
    }

    #[test]
    fn every_category_offers_a_default_target() {
        let view = catalog_view(&ToolRegistry::default());
        for c in &view.categories {
            assert!(!c.default_target.is_empty(), "{}", c.id);
            assert!(!c.inputs.is_empty(), "{}", c.id);
        }
    }
}
