//! Generates `FORMATS.md` straight from the catalog, so the documentation cannot drift away from
//! what the app actually supports. Run `cargo run -p convert-core --bin format-report`.

use crate::format::{catalog, Category, Support};
use crate::plan::{default_target_for, suggested_targets_for};

fn support_cell(s: Support) -> String {
    match s {
        Support::Unsupported => "—".into(),
        Support::Bundled => "✅".into(),
        Support::AnyOf(tools) => {
            let names: Vec<&str> = tools.iter().map(|t| t.label()).collect();
            format!("⚙︎ {}", names.join(" / "))
        }
    }
}

pub fn markdown() -> String {
    let mut out = String::new();
    out.push_str("# Supported formats\n\n");
    out.push_str(
        "Generated from the format catalog in `crates/convert-core/src/format.rs` \
         (`cargo run -p convert-core --bin format-report`). Do not edit by hand.\n\n\
         **✅ works out of the box** (bundled engine) · **⚙︎ needs a free helper app** \
         (auto-detected; the app tells you the one-line install command) · **—** not supported \
         in that direction.\n\n",
    );

    let total_in: usize =
        catalog().iter().filter(|f| f.read.is_supported()).map(|f| f.extensions.len()).sum();
    let total_out = catalog().iter().filter(|f| f.write.is_supported()).count();
    out.push_str(&format!(
        "**{total_in} input file extensions · {total_out} output formats · {} categories**\n\n",
        Category::ALL.len()
    ));

    for c in Category::ALL {
        let formats: Vec<_> = catalog().iter().filter(|f| f.category == c).collect();
        if formats.is_empty() {
            continue;
        }
        out.push_str(&format!("## {}\n\n", c.label()));
        out.push_str(&format!(
            "One-click default: **{}** · quick picks: {}\n\n",
            default_target_for(c),
            suggested_targets_for(c)
                .iter()
                .map(|t| format!("`{t}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        out.push_str("| Format | Extensions | Read | Write | Notes |\n");
        out.push_str("| --- | --- | :---: | :---: | --- |\n");
        for f in formats {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                f.name,
                f.extensions.iter().map(|e| format!("`.{e}`")).collect::<Vec<_>>().join(" "),
                support_cell(f.read),
                support_cell(f.write),
                f.notes
            ));
        }
        out.push('\n');
    }

    out.push_str("## Cross-category conversions\n\n");
    out.push_str(
        "| From | To | How |\n| --- | --- | --- |\n\
         | Video | Audio | stream extracted and re-encoded (`clip.mp4` → `clip.mp3`) |\n\
         | Video / Flash | GIF, animated WebP, APNG | frame-rate + palette optimised |\n\
         | Video | JPEG, PNG, WebP, … | frame sequence, 1 frame per second by default |\n\
         | Flash | PNG | the frames Ruffle renders, one file per frame |\n\
         | Animated GIF / WebP | MP4, WebM | real video, constant frame rate |\n\
         | Images | HEIC, ICNS, AI | written by the helper that owns the format (sips / ImageMagick) |\n\
         | Images | PDF | one page per image (ImageMagick) |\n\
         | PDF | PNG, JPEG | one image per page (Poppler → ImageMagick → sips) |\n\
         | Office / Slides | PNG, JPEG | printed to PDF first, then rasterised |\n\
         | Markdown / HTML / EPUB | PDF | Pandoc → HTML → LibreOffice print |\n\
         | Video | SRT, VTT, ASS | embedded subtitle track demuxed |\n\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_covers_every_category_and_marks_helpers() {
        let md = markdown();
        for c in Category::ALL {
            assert!(md.contains(&format!("## {}", c.label())), "missing {}", c.label());
        }
        assert!(md.contains("MP4"));
        assert!(md.contains("⚙︎ LibreOffice"));
        assert!(md.contains("Camera RAW"));
        assert!(md.lines().count() > 100);
    }
}
