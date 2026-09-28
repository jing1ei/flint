//! Planner tests.
//!
//! Kept in their own file because the planner is long; wired in from `plan.rs` with
//! `#[cfg(test)] #[path = "plan_tests.rs"] mod tests;` so this is a normal module body.

use super::*;
use crate::format::{by_extension, by_id, catalog};
use crate::settings::{OutputSettings, Preset, TrimSettings};
use crate::tools::{ToolVersion, ALL_TOOLS};

/// A registry where every helper exists, so tests exercise the *preferred* route.
fn all_tools() -> ToolRegistry {
    let mut r = ToolRegistry::default();
    for t in ALL_TOOLS {
        r.set(*t, PathBuf::from(format!("/opt/test/{}", t.id())));
    }
    r
}

fn only_ffmpeg() -> ToolRegistry {
    let mut r = ToolRegistry::default();
    r.set(Tool::Ffmpeg, PathBuf::from("/app/ffmpeg"));
    r.set(Tool::Ffprobe, PathBuf::from("/app/ffprobe"));
    r
}

/// [`all_tools`] with a known Pandoc version, for the flags that depend on it.
fn pandoc_version(major: u32, minor: u32) -> ToolRegistry {
    let mut r = all_tools();
    r.set_version(Tool::Pandoc, ToolVersion::new(major, minor));
    r
}

fn req(from: &str, to: &str) -> PlanRequest {
    let mut s = Settings::default();
    s.video.hardware_accel = HardwareAccel::Off; // deterministic across CI hosts
    PlanRequest {
        input: PathBuf::from(format!("/in/clip.{from}")),
        output: PathBuf::from(format!(
            "/out/clip.{}",
            by_id(to).map(output_extension).unwrap_or(to)
        )),
        source: by_extension(from).unwrap_or_else(|| panic!("no source format for .{from}")),
        target: by_id(to).unwrap_or_else(|| panic!("no target format `{to}`")),
        settings: s,
        temp_dir: PathBuf::from("/tmp/job1"),
        source_is_animated: matches!(from, "mp4" | "mov" | "mkv" | "webm" | "avi" | "gif"),
    }
}

fn args_of(p: &Plan, step: usize) -> String {
    p.steps[step].args.join(" ")
}

#[test]
fn video_to_mp4_is_web_ready() {
    let p = plan(&req("mov", "mp4"), &all_tools()).unwrap();
    assert_eq!(p.steps.len(), 1);
    let a = args_of(&p, 0);
    assert!(a.contains("-c:v libx264"), "{a}");
    assert!(a.contains("-crf 23"), "{a}");
    assert!(a.contains("-pix_fmt yuv420p"), "{a}");
    assert!(a.contains("-movflags +faststart"), "{a}");
    assert!(a.contains("-c:a aac -b:a 192k"), "{a}");
    assert!(a.contains("scale=-2:trunc(min(1080\\,ih)/2)*2"), "{a}");
    assert!(a.contains("-progress pipe:1"), "progress must be machine readable: {a}");
    assert!(a.ends_with("/out/clip.mp4"), "{a}");
    assert!(p.steps[0].ffmpeg_progress);
}

#[test]
fn webm_switches_to_vp9_and_opus_automatically() {
    let p = plan(&req("mp4", "webm"), &all_tools()).unwrap();
    let a = args_of(&p, 0);
    assert!(a.contains("-c:v libvpx-vp9"), "{a}");
    assert!(a.contains("-b:v 0"), "VP9 needs -b:v 0 for constant quality: {a}");
    assert!(a.contains("-c:a libopus"), "{a}");
}

#[test]
fn extracting_audio_drops_the_video_stream() {
    let p = plan(&req("mkv", "mp3"), &all_tools()).unwrap();
    let a = args_of(&p, 0);
    assert!(a.contains("-vn"), "{a}");
    assert!(a.contains("-c:a libmp3lame -b:a 192k"), "{a}");
    assert!(a.contains("-id3v2_version 3"), "keep tags readable on Windows: {a}");
}

#[test]
fn gif_output_uses_an_optimised_palette() {
    let p = plan(&req("mp4", "gif"), &all_tools()).unwrap();
    let a = args_of(&p, 0);
    assert!(a.contains("palettegen"), "{a}");
    assert!(a.contains("paletteuse"), "{a}");
    assert!(a.contains("fps=12"), "{a}");
    assert!(a.contains("-loop 0"), "{a}");
    assert!(p.steps.iter().all(|s| s.post.is_none()), "one GIF, nothing to collect");
}

#[test]
fn animated_gif_to_mp4_forces_a_constant_frame_rate() {
    let p = plan(&req("gif", "mp4"), &all_tools()).unwrap();
    let a = args_of(&p, 0);
    assert!(a.contains("-fps_mode cfr"), "{a}");
}

#[test]
fn video_to_still_images_writes_a_numbered_sequence() {
    let p = plan(&req("mp4", "png"), &all_tools()).unwrap();
    let a = args_of(&p, 0);
    assert!(a.contains("/out/clip-%04d.png"), "{a}");
    assert!(a.contains("fps=1"), "{a}");
}

#[test]
fn jpeg_flattens_transparency_instead_of_going_black() {
    let p = plan(&req("png", "jpg"), &all_tools()).unwrap();
    let a = args_of(&p, 0);
    assert!(a.contains("-filter_complex"), "labelled graph needs filter_complex: {a}");
    assert!(a.contains("drawbox=c=0xffffffff"), "{a}");
    assert!(a.contains("overlay"), "{a}");
    assert!(a.contains("-q:v"), "{a}");
    assert!(a.contains("-frames:v 1 -update 1"), "{a}");
}

#[test]
fn heic_is_decoded_by_a_helper_then_encoded_by_ffmpeg() {
    let p = plan(&req("heic", "jpg"), &all_tools()).unwrap();
    assert_eq!(p.steps.len(), 2);
    assert_eq!(p.steps[0].tool, Tool::Sips, "sips is preferred on macOS");
    assert_eq!(p.steps[1].tool, Tool::Ffmpeg);
    assert!(args_of(&p, 0).contains("/tmp/job1/decoded.png"));
    assert!(args_of(&p, 1).contains("/tmp/job1/decoded.png"));
    assert!((p.steps.iter().map(|s| s.weight).sum::<f32>() - 1.0).abs() < 1e-5);
}

#[test]
fn camera_raw_and_svg_take_the_same_two_step_path() {
    for ext in ["cr3", "nef", "arw", "svg", "psd"] {
        let p = plan(&req(ext, "jpg"), &all_tools()).unwrap_or_else(|e| panic!(".{ext}: {e}"));
        assert_eq!(p.steps.len(), 2, ".{ext}");
    }
}

#[test]
fn docx_to_pdf_uses_libreoffice_and_moves_the_result() {
    let p = plan(&req("docx", "pdf"), &all_tools()).unwrap();
    assert_eq!(p.steps[0].tool, Tool::LibreOffice);
    let a = args_of(&p, 0);
    assert!(a.contains("--headless"), "{a}");
    assert!(a.contains("--convert-to pdf:writer_pdf_Export"), "{a}");
    assert_eq!(
        p.steps[0].post,
        Some(PostAction::MoveFrom(PathBuf::from("/tmp/job1/office/clip.pdf")))
    );
}

#[test]
fn markdown_to_pdf_goes_through_html_because_pandoc_has_no_pdf_engine() {
    let p = plan(&req("md", "pdf"), &all_tools()).unwrap();
    assert_eq!(p.steps.len(), 2);
    assert_eq!(p.steps[0].tool, Tool::Pandoc);
    assert_eq!(p.steps[1].tool, Tool::LibreOffice);
    assert!(args_of(&p, 0).contains("/tmp/job1/intermediate.html"));
}

/// LibreOffice has no single "export to PDF" filter: each application registers its own, and
/// asking Calc or Impress for `writer_pdf_Export` is refused outright with *no export filter for
/// ...* - a row that fails for a reason the user cannot act on. Every document family that can
/// reach PDF has to name the filter of the application that opens it.
#[test]
fn every_document_family_prints_pdf_through_its_own_export_filter() {
    let writer = ["docx", "doc", "odt", "rtf", "txt", "html"];
    let calc = ["xlsx", "xls", "ods", "csv", "tsv"];
    let impress = ["pptx", "ppt", "odp"];
    for (family, filter) in [
        (&writer[..], "writer_pdf_Export"),
        (&calc[..], "calc_pdf_Export"),
        (&impress[..], "impress_pdf_Export"),
    ] {
        for from in family {
            let p = plan(&req(from, "pdf"), &all_tools()).unwrap();
            let a = args_of(&p, 0);
            assert_eq!(p.steps[0].tool, Tool::LibreOffice, "{from} -> pdf");
            assert!(a.contains(&format!("--convert-to pdf:{filter} ")), "{from} -> pdf: {a}");
        }
    }
    // A spreadsheet asked for a PDF, in full - the exact argv, because this is the command line
    // the defect was in.
    let p = plan(&req("xlsx", "pdf"), &all_tools()).unwrap();
    assert_eq!(
        p.steps[0].args,
        vec![
            "-env:UserInstallation=file:///tmp/job1/lo-profile",
            "--headless",
            "--norestore",
            "--invisible",
            "--nolockcheck",
            "--convert-to",
            "pdf:calc_pdf_Export",
            "--outdir",
            "/tmp/job1/office",
            "/in/clip.xlsx",
        ]
    );
    // Markdown reaches LibreOffice as the intermediate HTML Pandoc wrote, and HTML is Writer's.
    let p = plan(&req("md", "pdf"), &all_tools()).unwrap();
    assert!(args_of(&p, 1).contains("--convert-to pdf:writer_pdf_Export"), "{}", args_of(&p, 1));
    // Everything else keeps the target-only filter it always had.
    let p = plan(&req("xlsx", "csv"), &all_tools()).unwrap();
    assert!(args_of(&p, 0).contains("--convert-to csv:Text - txt - csv (StarCalc)"));
}

/// `soffice --headless` shares one user profile between instances, so two of them started at the
/// same time fight over it: the second refuses to start, exits silently, or hangs. A batch of ten
/// documents is the whole point of this app, so every invocation gets a profile of its own inside
/// the scratch directory the job already owns (and which the queue already deletes afterwards).
#[test]
fn two_jobs_planned_at_once_get_their_own_libreoffice_profile() {
    let profile_of = |p: &Plan, step: usize| -> String {
        p.steps[step]
            .args
            .iter()
            .find_map(|a| a.strip_prefix("-env:UserInstallation=").map(str::to_string))
            .unwrap_or_else(|| panic!("no private profile in {:?}", p.steps[step].args))
    };

    let mut first = req("docx", "pdf");
    first.temp_dir = PathBuf::from("/tmp/job-a");
    let mut second = req("xlsx", "pdf");
    second.temp_dir = PathBuf::from("/tmp/job-b");

    let a = plan(&first, &all_tools()).unwrap();
    let b = plan(&second, &all_tools()).unwrap();
    let (pa, pb) = (profile_of(&a, 0), profile_of(&b, 0));
    assert_ne!(pa, pb, "both jobs would fight over one profile");
    // A URL, not a path - LibreOffice reads a bare path as relative and silently falls back to
    // the shared profile - and inside the job's own scratch directory, so the existing cleanup
    // takes it away.
    assert_eq!(pa, "file:///tmp/job-a/lo-profile");
    assert_eq!(pb, "file:///tmp/job-b/lo-profile");
    assert!(a.steps[0].ensure_dirs.contains(&PathBuf::from("/tmp/job-a/lo-profile")));
    assert!(b.steps[0].ensure_dirs.contains(&PathBuf::from("/tmp/job-b/lo-profile")));

    // Every route that runs LibreOffice, not just the direct one: the Markdown hop and the
    // "slides to images" hop spawn it too.
    for (from, to, step) in [("md", "pdf", 1), ("pptx", "png", 0), ("xlsx", "csv", 0)] {
        let p = plan(&req(from, to), &all_tools()).unwrap();
        assert_eq!(p.steps[step].tool, Tool::LibreOffice, "{from} -> {to}");
        assert_eq!(profile_of(&p, step), "file:///tmp/job1/lo-profile", "{from} -> {to}");
    }
}

/// The profile switch is a URL, and a user's temp directory is not guaranteed to be URL-safe.
#[test]
fn a_scratch_directory_with_awkward_characters_still_makes_a_valid_url() {
    let mut r = req("docx", "pdf");
    r.temp_dir = PathBuf::from("/tmp/50% off/my job #2");
    let p = plan(&r, &all_tools()).unwrap();
    let profile = p.steps[0].args[0].clone();
    assert_eq!(profile, "-env:UserInstallation=file:///tmp/50%25%20off/my%20job%20%232/lo-profile");
}

#[test]
fn markup_pairs_prefer_pandoc_over_libreoffice() {
    for (from, to) in [("docx", "md"), ("md", "docx"), ("epub", "md"), ("html", "epub")] {
        let p = plan(&req(from, to), &all_tools()).unwrap();
        assert_eq!(p.steps[0].tool, Tool::Pandoc, "{from} -> {to}");
    }
    // ...but spreadsheets stay with LibreOffice
    let p = plan(&req("xlsx", "csv"), &all_tools()).unwrap();
    assert_eq!(p.steps[0].tool, Tool::LibreOffice);
}

#[test]
fn pandoc_pairs_fall_back_to_libreoffice_when_pandoc_is_missing() {
    let mut tools = ToolRegistry::default();
    for t in ALL_TOOLS.iter().filter(|t| **t != Tool::Pandoc) {
        tools.set(*t, PathBuf::from(format!("/opt/test/{}", t.id())));
    }
    let p = plan(&req("docx", "html"), &tools).unwrap();
    assert_eq!(p.steps[0].tool, Tool::LibreOffice);
}

#[test]
fn pdf_to_png_prefers_poppler_and_collects_every_page() {
    let p = plan(&req("pdf", "png"), &all_tools()).unwrap();
    assert_eq!(p.steps[0].tool, Tool::PdfToPpm);
    let a = args_of(&p, 0);
    assert!(a.contains("-r 150"), "{a}");
    assert!(a.contains("-png"), "{a}");
    assert!(matches!(p.steps[0].post, Some(PostAction::CollectSequence { .. })));
}

#[test]
fn slides_become_images_via_pdf() {
    let p = plan(&req("pptx", "png"), &all_tools()).unwrap();
    assert_eq!(p.steps.len(), 2);
    assert_eq!(p.steps[0].tool, Tool::LibreOffice);
    // Impress, not Writer: the intermediate PDF is printed by the application that opens the deck.
    assert!(args_of(&p, 0).contains("--convert-to pdf:impress_pdf_Export"), "{}", args_of(&p, 0));
    assert_eq!(p.steps[1].tool, Tool::PdfToPpm);
}

#[test]
fn image_to_pdf_uses_imagemagick() {
    let p = plan(&req("png", "pdf_page"), &all_tools()).unwrap();
    assert_eq!(p.steps[0].tool, Tool::Magick);
    assert!(args_of(&p, 0).ends_with("/out/clip.pdf"));
}

#[test]
fn flash_renders_with_ruffle_then_encodes() {
    let p = plan(&req("swf", "mp4"), &all_tools()).unwrap();
    assert_eq!(p.steps.len(), 2);
    assert_eq!(p.steps[0].tool, Tool::Ruffle);
    assert!(args_of(&p, 1).contains("-pattern_type glob"));
    assert!(args_of(&p, 1).contains("/tmp/job1/frames/*.png"));
}

#[test]
fn subtitles_convert_between_each_other() {
    let p = plan(&req("ass", "vtt"), &all_tools()).unwrap();
    assert_eq!(p.steps.len(), 1);
    assert!(args_of(&p, 0).ends_with("/out/clip.vtt"));
}

#[test]
fn missing_helper_produces_an_actionable_error() {
    let err = plan(&req("docx", "pdf"), &only_ffmpeg()).unwrap_err();
    match err {
        PlanError::MissingTool { tool, hint, .. } => {
            assert_eq!(tool, "LibreOffice");
            assert!(hint.contains("brew"), "hint should tell the user what to run: {hint}");
        }
        other => panic!("expected MissingTool, got {other:?}"),
    }
}

#[test]
fn nonsense_pairs_are_rejected_not_attempted() {
    assert!(matches!(
        plan(&req("mp3", "mp4"), &all_tools()),
        Err(PlanError::UnsupportedPair { .. })
    ));
    assert!(matches!(
        plan(&req("mp3", "jpg"), &all_tools()),
        Err(PlanError::UnsupportedPair { .. })
    ));
    assert!(matches!(
        plan(&req("srt", "mp4"), &all_tools()),
        Err(PlanError::UnsupportedPair { .. })
    ));
}

#[test]
fn presets_change_the_command_line() {
    let mut r = req("mov", "mp4");
    r.settings = Preset::Smallest.settings();
    r.settings.video.hardware_accel = HardwareAccel::Off;
    let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
    assert!(a.contains("-c:v libx265"), "{a}");
    assert!(a.contains("-crf 32"), "{a}");
    assert!(a.contains("scale=-2:trunc(min(720\\,ih)/2)*2"), "{a}");
    assert!(a.contains("-c:a libopus -b:a 96k"), "{a}");
    assert!(a.contains("-tag:v hvc1"), "HEVC needs hvc1 to play in QuickTime: {a}");
}

#[test]
fn explicit_bitrate_overrides_quality() {
    let mut r = req("mov", "mp4");
    r.settings.video.bitrate_kbps = Some(2500);
    let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
    assert!(a.contains("-b:v 2500k"), "{a}");
    assert!(!a.contains("-crf"), "{a}");
}

#[test]
fn stream_copy_skips_re_encoding() {
    let mut r = req("mkv", "mp4");
    r.settings.video.codec = VideoCodec::Copy;
    r.settings.audio.codec = AudioCodec::Copy;
    let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
    assert!(a.contains("-c:v copy"), "{a}");
    assert!(a.contains("-c:a copy"), "{a}");
    assert!(!a.contains("-crf"), "{a}");
}

#[test]
fn loudness_normalisation_is_opt_in() {
    let mut r = req("mp4", "mp3");
    assert!(!args_of(&plan(&r, &all_tools()).unwrap(), 0).contains("loudnorm"));
    r.settings.audio.normalize_loudness = true;
    assert!(args_of(&plan(&r, &all_tools()).unwrap(), 0).contains("loudnorm=I=-16"));
}

#[test]
fn quality_scales_are_monotonic() {
    assert!(jpeg_quality_scale(100) < jpeg_quality_scale(50));
    assert!(jpeg_quality_scale(50) < jpeg_quality_scale(10));
    assert!((2..=31).contains(&jpeg_quality_scale(1)));
    assert!(quality_to_crf(100) < quality_to_crf(50));
    assert!(quality_to_crf(50) <= 63);
}

#[test]
fn output_extensions_follow_real_container_names() {
    assert_eq!(output_extension(by_id("alac").unwrap()), "m4a");
    assert_eq!(output_extension(by_id("pdf_page").unwrap()), "pdf");
    assert_eq!(output_extension(by_id("apng").unwrap()), "png");
    assert_eq!(output_extension(by_id("mp4").unwrap()), "mp4");
}

/// Every chip the UI offers must actually produce a plan - no dead buttons.
#[test]
fn every_suggested_target_is_plannable() {
    let tools = all_tools();
    let sources = [
        (Category::Video, "mp4"),
        (Category::Audio, "mp3"),
        (Category::Image, "png"),
        (Category::Document, "docx"),
        (Category::Subtitle, "srt"),
        (Category::Flash, "swf"),
    ];
    for (cat, src) in sources {
        for target in suggested_targets_for(cat) {
            let r = req(src, target);
            plan(&r, &tools).unwrap_or_else(|e| panic!("{src} -> {target}: {e}"));
        }
        let default = default_target_for(cat);
        plan(&req(src, default), &tools)
            .unwrap_or_else(|e| panic!("{src} -> default {default}: {e}"));
    }
}

#[test]
fn output_settings_defaults_are_conservative() {
    let o = OutputSettings::default();
    assert_eq!(o.subfolder_name, "Converted");
    assert!(o.preserve_timestamps);
}

// ------------------------------------------------------------------- container / codec matching

#[test]
fn the_hvc1_tag_is_only_written_into_iso_containers() {
    // QuickTime needs it; Matroska and WebM have no concept of a four-character codec tag and
    // FFmpeg fails with "Could not find tag for codec hevc" when it is forced on them.
    for (container, expected) in [("mp4", true), ("mov", true), ("mkv", false)] {
        let mut r = req("mov", container);
        r.settings.video.codec = VideoCodec::H265;
        let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
        assert!(a.contains("-c:v libx265"), "{container}: {a}");
        assert_eq!(a.contains("-tag:v hvc1"), expected, "{container}: {a}");
    }
}

#[test]
fn containers_that_cannot_carry_the_requested_codec_fall_back_instead_of_failing() {
    for (container, encoder) in [
        ("webm", "libvpx-vp9"), // no HEVC in WebM
        ("mov", "libx264"),     // no VP9 in QuickTime
        ("flv", "libx264"),
    ] {
        let mut r = req("mov", container);
        r.settings.video.codec = VideoCodec::H265;
        if container == "mov" {
            r.settings.video.codec = VideoCodec::Vp9;
        }
        let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
        assert!(a.contains(&format!("-c:v {encoder}")), "{container}: {a}");
    }
}

#[test]
fn lossless_audio_containers_never_get_a_bitrate() {
    // `Auto` used to reach the muxer default and still be handed `-b:a 192k`, which FLAC and WAV
    // reject outright - so the "Lossless / archive" preset failed on its own headline formats.
    for container in ["flac", "wav"] {
        let a = args_of(&plan(&req("mp3", container), &all_tools()).unwrap(), 0);
        assert!(!a.contains("-b:a"), "{container}: {a}");
    }
    let a = args_of(&plan(&req("mp4", "flac"), &all_tools()).unwrap(), 0);
    assert!(a.contains("-c:a flac"), "{a}");
}

#[test]
fn audio_codecs_a_container_cannot_hold_are_swapped_for_one_it_can() {
    // The Archive preset asks for FLAC everywhere; M4A and MP3 cannot carry it.
    for (container, encoder) in [("m4a", "aac"), ("mp3", "libmp3lame"), ("caf", "pcm_s16le")] {
        let mut r = req("wav", container);
        r.settings.audio.codec = AudioCodec::Flac;
        let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
        assert!(a.contains(&format!("-c:a {encoder}")), "{container}: {a}");
    }
}

#[test]
fn speech_and_experimental_codecs_get_the_flags_they_insist_on() {
    let a = args_of(&plan(&req("wav", "dts"), &all_tools()).unwrap(), 0);
    assert!(a.contains("-strict -2"), "the DTS encoder is experimental: {a}");
}

/// The catalog offered `.amr` and `.gsm` as *outputs*, but neither codec has an encoder in FFmpeg
/// itself: AMR-NB needs `--enable-libopencore-amrnb` and GSM 06.10 needs `--enable-libgsm`, and the
/// Apple Silicon FFmpeg this app ships (osxexperts 7.1.1) is configured with neither - only the
/// muxers are there, so the row died with "Output file does not contain any stream" after the whole
/// decode. Decoding both is native, so they stay on the input side.
#[test]
fn speech_formats_with_no_encoder_are_not_offered_as_targets() {
    for id in ["amr", "gsm"] {
        let f = by_id(id).unwrap();
        assert!(f.read.is_supported(), "{id} still decodes");
        assert!(!f.write.is_supported(), "{id} has no encoder in the FFmpeg we ship");
        assert_eq!(
            plan(&req("wav", id), &all_tools()).unwrap_err(),
            PlanError::UnsupportedPair { from: "wav", to: f.id }
        );
    }
}

/// Both container-quirk tables are keyed by *target* format id, so an entry the catalog no longer
/// writes is an arm that can never run - and a comment describing behaviour the app does not have.
#[test]
fn the_container_quirk_tables_only_name_formats_we_still_write() {
    for id in NO_BITRATE_CONTAINERS {
        let f = by_id(id).unwrap_or_else(|| panic!("`{id}` is not a catalog format at all"));
        assert!(f.write.is_supported(), "`{id}` is not a target any more");
    }
    for (id, hz) in FIXED_SAMPLE_RATE_CONTAINERS {
        let f = by_id(id).unwrap_or_else(|| panic!("`{id}` is not a catalog format at all"));
        assert!(f.write.is_supported(), "`{id}` is not a target any more");
        assert_eq!(sane_sample_rate(*hz), *hz, "`{id}`: {hz} Hz would be clamped away");
    }
}

// ------------------------------------------------------------------------------ image encoding

#[test]
fn alpha_is_flattened_to_a_pixel_format_the_target_can_store() {
    for (target, pix) in [("jpg", "yuvj420p"), ("bmp", "bgr24"), ("pcx", "rgb24")] {
        let a = args_of(&plan(&req("png", target), &all_tools()).unwrap(), 0);
        assert!(a.contains(&format!("format={pix}")), "{target}: {a}");
    }
    // Formats that keep their alpha must not be pushed through the flatten graph at all.
    let a = args_of(&plan(&req("png", "webp"), &all_tools()).unwrap(), 0);
    assert!(!a.contains("drawbox"), "{a}");
}

#[test]
fn ico_is_capped_on_both_edges() {
    // A width-only cap left a 240x400 portrait 400 px tall, and the ICO muxer refuses anything
    // over 256 in *either* direction.
    let a = args_of(&plan(&req("png", "ico"), &all_tools()).unwrap(), 0);
    assert!(a.contains("h=min(ih\\,256)"), "{a}");
    assert!(a.contains("force_original_aspect_ratio=decrease"), "{a}");
}

#[test]
fn animation_loop_options_use_each_muxers_own_spelling() {
    // 0 means "forever" for all three, but the option name does not: gif/webp take `-loop`,
    // apng takes `-plays`.
    let gif = args_of(&plan(&req("mp4", "gif"), &all_tools()).unwrap(), 0);
    assert!(gif.contains("-loop 0"), "{gif}");

    let webp = args_of(&plan(&req("gif", "webp"), &all_tools()).unwrap(), 0);
    assert!(webp.contains("-c:v libwebp_anim"), "{webp}");
    assert!(webp.contains("-loop 0"), "{webp}");

    let apng = args_of(&plan(&req("gif", "apng"), &all_tools()).unwrap(), 0);
    assert!(apng.contains("-plays 0"), "{apng}");
    assert!(!apng.contains("-loop"), "apng has no -loop option: {apng}");
}

// -------------------------------------------------------------------------------- metadata

#[test]
fn metadata_is_only_stripped_where_the_user_asked_for_it() {
    // Video and images carry EXIF/GPS and have their own switch; audio tags are what make a
    // library usable and subtitles carry nothing worth removing - stripping those was pointless.
    let video = args_of(&plan(&req("mov", "mp4"), &all_tools()).unwrap(), 0);
    assert!(video.contains("-map_metadata -1"), "{video}");

    let audio = args_of(&plan(&req("mp4", "mp3"), &all_tools()).unwrap(), 0);
    assert!(!audio.contains("-map_metadata"), "audio tags must survive: {audio}");

    let subs = args_of(&plan(&req("ass", "vtt"), &all_tools()).unwrap(), 0);
    assert!(!subs.contains("-map_metadata"), "{subs}");

    let mut r = req("mov", "mp4");
    r.settings.video.strip_metadata = false;
    let kept = args_of(&plan(&r, &all_tools()).unwrap(), 0);
    assert!(!kept.contains("-map_metadata"), "{kept}");
}

#[test]
fn the_frame_rate_cap_is_an_output_option() {
    // `-fpsmax` only means anything after the input; in front of `-i` FFmpeg reads it as an input
    // option and silently ignores it.
    let p = plan(&req("mov", "mp4"), &all_tools()).unwrap();
    let args = &p.steps[0].args;
    let i = args.iter().position(|a| a == "-i").expect("an input");
    let cap = args.iter().position(|a| a == "-fpsmax").expect("a frame rate cap");
    assert!(cap > i, "{args:?}");
    assert_eq!(args[cap + 1], "60");
    assert!(cap < args.len() - 1, "the cap belongs before the output file: {args:?}");
}

// ------------------------------------------------------------------------- declared directories

#[test]
fn steps_declare_the_directories_they_need() {
    // The engine used to sniff argument strings for "/tmp", "office" and "frames"; now every step
    // says what it needs, so a user folder called "My frames" is no longer a special case.
    let p = plan(&req("mov", "mp4"), &all_tools()).unwrap();
    assert_eq!(p.steps[0].ensure_dirs, vec![PathBuf::from("/out")]);

    // LibreOffice needs its output directory *and* the private profile it is pointed at, both
    // inside this job's scratch directory.
    let p = plan(&req("docx", "pdf"), &all_tools()).unwrap();
    assert_eq!(
        p.steps[0].ensure_dirs,
        vec![PathBuf::from("/tmp/job1/office"), PathBuf::from("/tmp/job1/lo-profile")]
    );

    let p = plan(&req("swf", "mp4"), &all_tools()).unwrap();
    assert_eq!(p.steps[0].ensure_dirs, vec![PathBuf::from("/tmp/job1/frames")]);
}

#[test]
fn every_route_that_writes_many_files_declares_what_to_collect() {
    // A sequence route has exactly one way to tell the engine what it produced.
    for (from, to) in [("mp4", "png"), ("pdf", "png"), ("swf", "png")] {
        let p = plan(&req(from, to), &all_tools()).unwrap();
        let declared =
            p.steps.iter().any(|s| matches!(s.post, Some(PostAction::CollectSequence { .. })));
        assert!(declared, "{from} -> {to} produces a sequence but never says so");
    }
    // ...and a single-file job must not claim one.
    let p = plan(&req("png", "jpg"), &all_tools()).unwrap();
    assert!(p.steps.iter().all(|s| s.post.is_none()));
}

#[test]
fn a_frame_sequence_names_the_files_it_will_write() {
    let p = plan(&req("mp4", "png"), &all_tools()).unwrap();
    assert_eq!(
        p.steps[0].post,
        Some(PostAction::CollectSequence {
            dir: PathBuf::from("/out"),
            prefix: "clip".into(),
            extension: "png".into(),
        })
    );
}

// ------------------------------------------------------------------------------- hostile input

#[test]
fn nonsense_settings_are_clamped_instead_of_producing_a_broken_command() {
    let mut r = req("mov", "mp4");
    r.settings.video.max_height = Some(0);
    r.settings.video.fps_cap = Some(-5.0);
    r.settings.audio.bitrate_kbps = 0;
    r.settings.audio.sample_rate = Some(0);
    r.settings.audio.channels = Some(0);
    let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
    assert!(a.contains("min(2\\,ih)"), "{a}");
    assert!(!a.contains("-fpsmax -"), "a negative cap would abort the encode: {a}");
    assert!(!a.contains("-b:a 0k"), "{a}");
    assert!(!a.contains("-ar 0"), "{a}");
    assert!(!a.contains("-ac 0"), "{a}");

    let mut r = req("mp4", "gif");
    r.settings.gif.fps = f32::NAN;
    r.settings.gif.width = 0;
    let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
    assert!(!a.contains("NaN"), "{a}");
    assert!(a.contains("min(2\\,iw)"), "{a}");

    let mut r = req("pdf", "png");
    r.settings.document.raster_dpi = 0;
    let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
    assert!(!a.contains("-r 0"), "{a}");
}

#[test]
fn the_flatten_colour_cannot_smuggle_a_filter_into_the_graph() {
    let mut r = req("png", "jpg");
    r.settings.image.flatten_background = "ff0000,crop=1:1".into();
    let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
    assert!(!a.contains("crop=1:1"), "{a}");
    assert!(a.contains("drawbox=c=0xffffffff"), "bad colours fall back to white: {a}");

    assert_eq!(hex_background("#A1B2C3"), "a1b2c3");
    assert_eq!(hex_background("#f0f"), "ff00ff");
    assert_eq!(hex_background("rebeccapurple"), "ffffff");
    assert_eq!(hex_background(""), "ffffff");
}

// ------------------------------------------------------------------ catalog / planner agreement
//
// The catalog is the only thing the UI reads, so a plan that contradicts it is a promise the app
// cannot keep. These two tests walk *every* readable source against *every* writable target.

fn every_pair() -> Vec<(&'static Format, &'static Format)> {
    let readable: Vec<_> = catalog().iter().filter(|f| f.read.is_supported()).collect();
    let writable: Vec<_> = catalog().iter().filter(|f| f.write.is_supported()).collect();
    let mut out = Vec::new();
    for s in &readable {
        for t in &writable {
            out.push((*s, *t));
        }
    }
    out
}

fn request_for(source: &'static Format, target: &'static Format) -> PlanRequest {
    let mut r = req("mp4", "mp4");
    r.input = PathBuf::from(format!("/in/clip.{}", source.extensions[0]));
    r.output = PathBuf::from(format!("/out/clip.{}", output_extension(target)));
    r.source = source;
    r.target = target;
    r.source_is_animated = matches!(source.category, Category::Video | Category::Flash);
    r
}

#[test]
fn no_plan_writes_a_file_with_the_wrong_extension() {
    for (source, target) in every_pair() {
        let Ok(p) = plan(&request_for(source, target), &all_tools()) else { continue };
        let want = output_extension(target);
        for step in &p.steps {
            if let Some(PostAction::CollectSequence { extension, .. }) = &step.post {
                assert_eq!(
                    extension, want,
                    "{} -> {}: collects .{extension} files but the user asked for .{want}",
                    source.id, target.id
                );
            }
        }
        assert_eq!(
            p.output.extension().and_then(|e| e.to_str()),
            Some(want),
            "{} -> {}",
            source.id,
            target.id
        );
    }
}

#[test]
fn no_plan_asks_ffmpeg_to_write_a_format_only_a_helper_can_write() {
    for (source, target) in every_pair() {
        let Ok(p) = plan(&request_for(source, target), &all_tools()) else { continue };
        let last = p.steps.last().expect("a plan with no steps can produce nothing");
        // The catalog splits "a PDF" into two entries: the Document `pdf` LibreOffice prints, and
        // the Image `pdf_page` ImageMagick wraps a single image in. An image source legitimately
        // takes the second route even when it names the first, so accept either writer there.
        let target = match (source.category, target.id) {
            (Category::Image, "pdf") => by_id("pdf_page").unwrap(),
            _ => target,
        };
        // Document targets are chosen per *pair*, not per format: LibreOffice cannot open Markdown
        // and Pandoc cannot lay out a page, so md -> docx is Pandoc's job even though the catalog
        // lists docx as LibreOffice's. `plan()` returns MissingTool for the pairs it cannot serve,
        // which is the only place that pair-level truth can live.
        if target.category == Category::Document {
            continue;
        }
        if target.write.needs_helper() {
            assert!(
                target.write.helpers().contains(&last.tool),
                "{} -> {}: the catalog says {:?} write it, the plan asked {:?}",
                source.id,
                target.id,
                target.write.helpers(),
                last.tool,
            );
        } else {
            assert_ne!(
                last.tool,
                Tool::Ffprobe,
                "{} -> {}: ffprobe never writes anything",
                source.id,
                target.id
            );
        }
    }
}

#[test]
fn heic_is_encoded_by_a_helper_because_ffmpeg_cannot() {
    // Before: this planned `ffmpeg ... /out/clip.heic`, which fails after the whole decode.
    let p = plan(&req("jpg", "heic"), &all_tools()).unwrap();
    assert_eq!(p.steps.len(), 1);
    assert_eq!(p.steps[0].tool, Tool::Sips);
    assert!(args_of(&p, 0).contains("-s format heic"), "{}", args_of(&p, 0));

    // ...and with no helper at all the user is told which one to install.
    let err = plan(&req("jpg", "heic"), &only_ffmpeg()).unwrap_err();
    assert!(matches!(err, PlanError::MissingTool { format: "heic", .. }), "{err:?}");
}

#[test]
fn a_helper_only_source_and_a_helper_only_target_still_meet_in_the_middle() {
    let p = plan(&req("heic", "icns"), &all_tools()).unwrap();
    assert_eq!(p.steps.len(), 2, "decode to PNG, then encode");
    assert_eq!(p.steps[1].tool, Tool::Sips);
    assert!(args_of(&p, 1).contains("-s format icns"), "{}", args_of(&p, 1));
    assert_eq!(p.temp_paths, vec![PathBuf::from("/tmp/job1/decoded.png")]);
}

#[test]
fn a_video_frame_cannot_be_a_heic_and_we_say_so() {
    // sips/magick cannot read an MP4, so there is no honest route here.
    let err = plan(&req("mp4", "heic"), &all_tools()).unwrap_err();
    assert_eq!(err, PlanError::UnsupportedPair { from: "mp4", to: "heic" });
}

#[test]
fn rasterising_a_pdf_only_offers_the_formats_the_rasterisers_write() {
    for to in ["webp", "avif", "tiff", "heic"] {
        let err = plan(&req("pdf", to), &all_tools()).unwrap_err();
        assert_eq!(err, PlanError::UnsupportedPair { from: "pdf", to: by_id(to).unwrap().id });
    }
    assert!(plan(&req("pdf", "png"), &all_tools()).is_ok());
    assert!(plan(&req("pdf", "jpg"), &all_tools()).is_ok());
}

#[test]
fn flash_stills_are_png_because_that_is_what_ruffle_writes() {
    let p = plan(&req("swf", "png"), &all_tools()).unwrap();
    assert!(
        matches!(&p.steps[0].post, Some(PostAction::CollectSequence { extension, .. }) if extension == "png")
    );
    let err = plan(&req("swf", "jpg"), &all_tools()).unwrap_err();
    assert_eq!(err, PlanError::UnsupportedPair { from: "swf", to: "jpg" });
    // animated targets are encoded from the frames, so they stay available
    assert!(plan(&req("swf", "gif"), &all_tools()).is_ok());
}

#[test]
fn with_only_sips_a_multipage_pdf_admits_it_renders_one_page() {
    let mut tools = only_ffmpeg();
    tools.set(Tool::Sips, PathBuf::from("/usr/bin/sips"));
    let p = plan(&req("pdf", "png"), &tools).unwrap();
    assert_eq!(p.steps[0].tool, Tool::Sips);
    assert!(p.summary.contains("first page only"), "{}", p.summary);
    assert!(p.steps[0].post.is_none(), "sips writes exactly the file we named");

    // ask for one page and there is nothing to warn about
    let mut r = req("pdf", "png");
    r.settings.document.first_page_only = true;
    let p = plan(&r, &tools).unwrap();
    assert!(!p.summary.contains("first page"), "{}", p.summary);
}

/// A machine whose FFmpeg sidecar never made it into the bundle (or was never fetched in dev) can
/// still turn a SWF into PNG frames: Ruffle writes them itself. Demanding FFmpeg for a step that
/// does not exist turned a working conversion into "png needs FFmpeg installed".
#[test]
fn flash_frames_do_not_need_an_encoder_that_never_runs() {
    let mut ruffle_only = ToolRegistry::default();
    ruffle_only.set(Tool::Ruffle, PathBuf::from("/Applications/Ruffle.app/ruffle"));

    let p = plan(&req("swf", "png"), &ruffle_only).expect("Ruffle alone can write PNG frames");
    assert_eq!(p.steps.len(), 1, "{:?}", p.steps);
    assert_eq!(p.steps[0].tool, Tool::Ruffle);
    assert!(!p.summary.contains("encode"), "{}", p.summary);

    // ...but an animated target really is encoded from those frames, so it must still say so.
    let err = plan(&req("swf", "gif"), &ruffle_only).unwrap_err();
    assert_eq!(
        err,
        PlanError::MissingTool {
            format: "gif",
            tool: Tool::Ffmpeg.user_facing_name(),
            hint: Tool::Ffmpeg.install_hint(),
        }
    );
}

/// The Archive preset promises "keeps metadata" in the picker. Video metadata is where that
/// matters most - capture date, camera, GPS - and it was being stripped anyway, silently, with no
/// way for the user to tell until the file was already written.
#[test]
fn the_archive_preset_really_keeps_metadata() {
    let mut r = req("mov", "mov");
    r.settings = Preset::Archive.settings();
    r.settings.video.hardware_accel = HardwareAccel::Off;
    let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
    assert!(!a.contains("-map_metadata"), "the archive preset threw the metadata away: {a}");

    // The web preset still strips it: small, shareable files are the whole point there.
    let mut web = req("mov", "mp4");
    web.settings = Preset::WebAndDemo.settings();
    web.settings.video.hardware_accel = HardwareAccel::Off;
    let a = args_of(&plan(&web, &all_tools()).unwrap(), 0);
    assert!(a.contains("-map_metadata -1"), "{a}");
}

// ------------------------------------------------------------------- percent signs in file names

/// `av_get_frame_filename` decides whether FFmpeg sees a name as a sequence *pattern*, and its
/// rules are narrower than "contains a %": `%%` is a literal, two `%d`s cancel out, and any other
/// conversion invalidates the whole thing. Getting this wrong in either direction is a broken
/// command line, so the table is pinned here.
#[test]
fn only_a_real_frame_pattern_counts_as_a_sequence_pattern() {
    for yes in ["a%d.png", "a%04d.png", "/in/%4d.png", "%d", "/50%%off/a%d.png"] {
        assert!(has_image2_sequence_spec(yes), "{yes} is a pattern to FFmpeg");
    }
    for no in [
        "clip.png",
        "100%_done.png", // `%_` is not a conversion: FFmpeg opens the file as-is
        "50% off.png",   // ...as is a trailing `% `
        "a%%d.png",      // an escaped percent sign
        "a%d_%d.png",    // two frame numbers: FFmpeg refuses the pattern outright
        "a%s.png",
        "trailing%",
    ] {
        assert!(!has_image2_sequence_spec(no), "{no} is a plain file name to FFmpeg");
    }
}

/// A folder called "50% off" and a file called "100%_done.mp4" are ordinary macOS names, but the
/// image2 muxer reads its output path as a printf pattern: the job used to write one file literally
/// named `100%_done-%04d.png` and then die with "Cannot write more than one file with the same
/// name", leaving that file behind for the user to find.
#[test]
fn percent_signs_in_the_output_path_are_escaped_for_the_image2_muxer() {
    let mut r = req("mp4", "png");
    r.input = PathBuf::from("/in/100%_done.mp4");
    r.output = PathBuf::from("/out/50% off/100%_done.png");
    let p = plan(&r, &all_tools()).unwrap();
    let a = args_of(&p, 0);
    assert!(a.ends_with("/out/50%% off/100%%_done-%04d.png"), "{a}");
    // FFmpeg writes the files with a single `%`, so the collector must look for the real name -
    // otherwise the job "succeeds" with no outputs and the frames are left as litter.
    assert_eq!(
        p.steps[0].post,
        Some(PostAction::CollectSequence {
            dir: PathBuf::from("/out/50% off"),
            prefix: "100%_done".into(),
            extension: "png".into(),
        })
    );
}

/// The other direction: a file whose own name *is* a frame pattern. FFmpeg went looking for
/// `take0000.png`..`take0004.png` and reported "Could find no file with path" for a file that is
/// sitting right there.
#[test]
fn a_file_named_like_a_frame_pattern_is_still_read_as_one_file() {
    let mut r = req("png", "jpg");
    r.input = PathBuf::from("/in/take%04d.png");
    let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
    assert!(a.contains("-pattern_type none -i /in/take%04d.png"), "{a}");

    // The switch exists only on the image2 demuxer, so it must not be handed to anything else:
    // gif/apng/avif/ico have their own demuxers (they reject the option outright), non-image
    // sources never went near image2, and a `%` that is not a conversion is not a pattern.
    for (from, to, name) in [
        ("gif", "mp4", "/in/take%04d.gif"),
        ("mp4", "webm", "/in/take%04d.mp4"),
        ("wav", "mp3", "/in/take%04d.wav"),
        ("png", "jpg", "/in/100%_done.png"),
    ] {
        let mut r = req(from, to);
        r.input = PathBuf::from(name);
        let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
        assert!(!a.contains("-pattern_type"), "{from} -> {to}: {a}");
    }
}

// --------------------------------------------------------------------------------- MXF is fussy

/// The MXF muxer implements exactly one audio sample rate and one set of frame rates. Both
/// defaults were left to the source, so a plain 44.1 kHz / 15 fps clip - a screen recording, a
/// phone video - died with "only 48khz is implemented" or "Unsupported frame rate 15/1".
#[test]
fn mxf_output_meets_the_two_rules_its_muxer_enforces() {
    let a = args_of(&plan(&req("mp4", "mxf"), &all_tools()).unwrap(), 0);
    assert!(a.contains("-ar 48000"), "the MXF muxer only writes 48 kHz audio: {a}");
    assert!(a.contains("-strict unofficial"), "non-broadcast frame rates need this: {a}");

    // ...and no other container pays for it.
    let a = args_of(&plan(&req("mp4", "mov"), &all_tools()).unwrap(), 0);
    assert!(!a.contains("-strict"), "{a}");
    assert!(!a.contains("-ar "), "{a}");
}

// ------------------------------------------------------------------------- subtitle honesty

/// The catalog is what the UI offers. FFmpeg has `microdvd`/`subviewer` decoders but no encoder for
/// either, and a TTML encoder but no demuxer or decoder (trac #4859), so both entries promised a
/// direction that always failed - `.sub` with "Output file #0 does not contain any stream".
#[test]
fn subtitle_formats_only_claim_the_direction_ffmpeg_has() {
    assert!(by_id("sub").unwrap().read.is_supported(), "MicroDVD/SubViewer decode fine");
    assert!(!by_id("sub").unwrap().write.is_supported(), "there is no MicroDVD encoder");
    assert!(by_id("ttml").unwrap().write.is_supported(), "the TTML encoder is real");
    assert!(!by_id("ttml").unwrap().read.is_supported(), "there is no TTML demuxer");

    assert_eq!(
        plan(&req("srt", "sub"), &all_tools()).unwrap_err(),
        PlanError::UnsupportedPair { from: "srt", to: "sub" }
    );
    assert!(plan(&req("srt", "ttml"), &all_tools()).is_ok(), "the write direction still works");
    // A generic `.xml` is not a subtitle file, and claiming it made every XML in a folder look
    // convertible.
    assert!(by_extension("xml").is_none());
}

// ------------------------------------------------------------------------------ pandoc versions

/// Pandoc is a helper the *user* installs, so its age is not ours to choose, and the flag that
/// embeds images into the HTML was renamed mid-life: `--embed-resources` only exists from 2.19 (Aug
/// 2022) and 2.17 exits 6 with "Unknown option", while `--self-contained` still works in 3.x but is
/// deprecated. Verified against real binaries here: 2.17.1.1 accepts only the old spelling, 2.19.2
/// and 3.8.1 accept both (warning on the old one) and all three embed the image as a data URI.
#[test]
fn markdown_to_html_uses_the_flag_the_installed_pandoc_understands() {
    let old = args_of(&plan(&req("md", "html"), &pandoc_version(2, 17)).unwrap(), 0);
    assert!(old.contains("--self-contained"), "{old}");
    assert!(!old.contains("--embed-resources"), "2.17 exits 6 on it: {old}");

    for (major, minor) in [(2, 19), (3, 8)] {
        let new = args_of(&plan(&req("md", "html"), &pandoc_version(major, minor)).unwrap(), 0);
        assert!(new.contains("--embed-resources"), "{major}.{minor}: {new}");
        assert!(!new.contains("--self-contained"), "deprecated from 2.19: {new}");
    }

    // Discovery could not run the binary (broken install, sandbox, exotic build): take the
    // spelling that works on every release we could test rather than the one that fails hard.
    let unknown = args_of(&plan(&req("md", "html"), &all_tools()).unwrap(), 0);
    assert!(unknown.contains("--self-contained"), "{unknown}");

    // Only HTML embeds anything; nothing else should carry either flag.
    let docx = args_of(&plan(&req("md", "docx"), &pandoc_version(3, 8)).unwrap(), 0);
    assert!(!docx.contains("--embed-resources") && !docx.contains("--self-contained"), "{docx}");
}

// -------------------------------------------------------------------------- helper declarations

/// The catalog is where "needs LibreOffice" comes from, and it claimed LibreOffice for Markdown.
/// LibreOffice has no Markdown filter and the planner has never had a route that used it for one,
/// so a machine with only LibreOffice showed `.md` as ready to convert and then failed the row.
#[test]
fn markdown_does_not_claim_a_tool_that_cannot_read_it() {
    assert_eq!(by_id("md").unwrap().read.helpers(), &[Tool::Pandoc]);
    assert_eq!(by_id("md").unwrap().write.helpers(), &[Tool::Pandoc]);

    let mut office_only = only_ffmpeg();
    office_only.set(Tool::LibreOffice, PathBuf::from("/Applications/LibreOffice.app/soffice"));
    let err = plan(&req("md", "docx"), &office_only).unwrap_err();
    assert_eq!(
        err,
        PlanError::MissingTool {
            format: "docx",
            tool: Tool::Pandoc.user_facing_name(),
            hint: Tool::Pandoc.install_hint(),
        }
    );

    // Plain text and HTML really do round-trip through LibreOffice alone, so those keep both.
    for id in ["txt", "html"] {
        assert!(by_id(id).unwrap().read.helpers().contains(&Tool::LibreOffice), "{id}");
        assert!(plan(&req(id, "docx"), &office_only).is_ok(), "{id} -> docx via LibreOffice");
    }
}

/// `OfficeThenPdfRaster` keeps LibreOffice's intermediate PDF and rasterises it in place, so it has
/// to name the exact file LibreOffice wrote. Both sides now derive it from one function; this pins
/// the agreement so a change to the naming cannot quietly point the rasteriser at nothing.
#[test]
fn the_rasteriser_reads_the_file_libreoffice_actually_wrote() {
    let p = plan(&req("pptx", "png"), &all_tools()).unwrap();
    assert_eq!(p.steps.len(), 2);
    assert_eq!(p.temp_paths, vec![PathBuf::from("/tmp/job1/office/clip.pdf")]);
    assert!(p.steps[0].post.is_none(), "the PDF stays where LibreOffice put it");
    assert!(args_of(&p, 1).contains("/tmp/job1/office/clip.pdf"), "{}", args_of(&p, 1));
}

// -------------------------------------------------------------------------------- PDF text

/// PDF -> text and PDF -> HTML demanded LibreOffice - an 800 MB install, and a re-import of the
/// whole page layout into Writer - for a job Poppler's own readers do in 15 MB. The catalog never
/// said LibreOffice was the only way to read a PDF, so the UI offered the pair on a machine with
/// Poppler and then failed the row. Poppler owns it now; LibreOffice stays as the fallback.
#[test]
fn pdf_text_is_read_out_by_poppler_with_libreoffice_as_the_fallback() {
    let p = plan(&req("pdf", "txt"), &all_tools()).unwrap();
    assert_eq!(p.steps.len(), 1);
    assert_eq!(p.steps[0].tool, Tool::PdfToText);
    assert_eq!(args_of(&p, 0), "-layout -enc UTF-8 /in/clip.pdf /out/clip.txt");
    assert!(p.steps[0].post.is_none(), "pdftotext writes exactly the file we named");

    let p = plan(&req("pdf", "html"), &all_tools()).unwrap();
    assert_eq!(p.steps.len(), 1);
    assert_eq!(p.steps[0].tool, Tool::PdfToHtml);
    // One self-contained file, not a frameset plus a `page<n>.html` and a `.png` per image.
    assert_eq!(args_of(&p, 0), "-s -noframes -dataurls -enc UTF-8 /in/clip.pdf /out/clip.html");
    assert!(p.steps[0].post.is_none());

    // No Poppler on this machine: LibreOffice still does it, through the PDF import filter.
    let mut office_only = only_ffmpeg();
    office_only.set(Tool::LibreOffice, PathBuf::from("/Applications/LibreOffice.app/soffice"));
    for to in ["txt", "html"] {
        let p = plan(&req("pdf", to), &office_only).unwrap();
        assert_eq!(p.steps[0].tool, Tool::LibreOffice, "{to}");
        let a = args_of(&p, 0);
        assert!(a.contains("--infilter=writer_pdf_import"), "{to}: {a}");
    }

    // Neither installed: the hint names the 15 MB helper, not the 800 MB one.
    let err = plan(&req("pdf", "txt"), &only_ffmpeg()).unwrap_err();
    assert_eq!(
        err,
        PlanError::MissingTool {
            format: "txt",
            tool: Tool::PdfToText.user_facing_name(),
            hint: Tool::PdfToText.install_hint(),
        }
    );
    // Poppler is one package with three binaries, so that hint is the same for all of them.
    assert_eq!(Tool::PdfToText.install_hint(), Tool::PdfToPpm.install_hint());
}

/// Rasterising is still `pdftoppm`'s job: the text readers must not be picked for an image target.
#[test]
fn the_pdf_text_readers_are_never_asked_for_an_image() {
    let mut text_only = only_ffmpeg();
    for tool in [Tool::PdfToText, Tool::PdfToHtml] {
        text_only.set(tool, PathBuf::from(format!("/opt/test/{}", tool.id())));
    }
    let err = plan(&req("pdf", "png"), &text_only).unwrap_err();
    assert_eq!(
        err,
        PlanError::MissingTool {
            format: "pdf",
            tool: Tool::PdfToPpm.user_facing_name(),
            hint: Tool::PdfToPpm.install_hint(),
        }
    );
}

// --------------------------------------------------------- the catalog is what the UI promises

/// Helpers the catalog declares for a pair - i.e. every tool the UI is entitled to name when this
/// conversion cannot run.
///
/// Two routes legitimately reach past the two endpoints, and both are spelled out rather than
/// waved through: an image target of `pdf` is written by the Image-category `pdf_page` entry (the
/// same substitution `no_plan_asks_ffmpeg_to_write_a_format_only_a_helper_can_write` makes), and a
/// document rasterised to an image (pptx -> png) prints to a PDF first, so the `pdf` entry's own
/// readers are part of what that pair needs.
fn declared_helpers(source: &'static Format, target: &'static Format) -> Vec<Tool> {
    let target = match (source.category, target.id) {
        (Category::Image, "pdf") => by_id("pdf_page").unwrap(),
        _ => target,
    };
    let mut out: Vec<Tool> = source.read.helpers().to_vec();
    out.extend_from_slice(target.write.helpers());
    if source.category == Category::Document
        && source.id != "pdf"
        && target.category == Category::Image
    {
        out.extend_from_slice(by_id("pdf").unwrap().read.helpers());
    }
    out.sort_by_key(|t| t.id());
    out.dedup();
    out
}

/// Every present/absent combination of those helpers: `1 << n` registries, the empty one included.
fn helper_combinations(declared: &[Tool]) -> Vec<Vec<Tool>> {
    (0..1u32 << declared.len())
        .map(|mask| {
            declared
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, t)| *t)
                .collect()
        })
        .collect()
}

/// The helpers a user-facing name stands for: one binary for most of them, three for Poppler,
/// because the name in "needs Poppler installed" is the package's and the planner's demand is
/// satisfied by whichever member of it does the job.
fn tools_named(name: &str) -> Vec<Tool> {
    ALL_TOOLS.iter().copied().filter(|t| t.user_facing_name() == name).collect()
}

/// The "PDF needs LibreOffice" strip, and the one-click installer next to it, are generated from
/// the catalog - so a plan that demands a helper the catalog never declared for the pair sends the
/// user to install the wrong thing, and the conversion fails again afterwards. This is the guard
/// that caught PDF -> text asking for LibreOffice when Poppler was the tool for the job.
///
/// Exhaustive on purpose: every catalog pair, every combination of the helpers it declares, and -
/// because a plan may need two helpers - the tools are installed one at a time in the order the
/// planner asks for them, until it either builds a plan or names something it should not.
#[test]
fn no_plan_demands_a_helper_the_catalog_did_not_declare_for_the_pair() {
    let mut complaints: Vec<String> = Vec::new();
    for (source, target) in every_pair() {
        let declared = declared_helpers(source, target);
        for present in helper_combinations(&declared) {
            let mut reg = only_ffmpeg();
            for tool in &present {
                reg.set(*tool, PathBuf::from(format!("/opt/test/{}", tool.id())));
            }
            let mut asked: Vec<&'static str> = Vec::new();
            let mut note = |what: String| {
                complaints.push(format!("{} -> {} with {present:?}: {what}", source.id, target.id))
            };
            loop {
                match plan(&request_for(source, target), &reg) {
                    Ok(_) => break,
                    // An honest refusal: this pair has no route, whatever is installed.
                    Err(PlanError::UnsupportedPair { .. }) if asked.is_empty() => break,
                    Err(PlanError::UnsupportedPair { .. }) => {
                        note(format!("asked for {asked:?} and then refused the pair anyway"));
                        break;
                    }
                    Err(PlanError::MissingTool { tool, .. }) => {
                        let named = tools_named(tool);
                        if named.is_empty() {
                            note(format!("wants `{tool}`, which is not a helper at all"));
                            break;
                        }
                        // Only the members the catalog declared are installed, so a plan that wants
                        // a *different* binary out of the same package still shows up below as the
                        // same name asked for twice.
                        let declared_members: Vec<Tool> =
                            named.into_iter().filter(|t| declared.contains(t)).collect();
                        if declared_members.is_empty() {
                            note(format!("wants {tool}, which the catalog declares for neither"));
                            break;
                        }
                        if asked.contains(&tool) {
                            note(format!("asks for {tool} again after it was installed"));
                            break;
                        }
                        asked.push(tool);
                        for t in declared_members {
                            reg.set(t, PathBuf::from(format!("/opt/test/{}", t.id())));
                        }
                    }
                    Err(e) => {
                        note(format!("{e}"));
                        break;
                    }
                }
            }
        }
    }
    complaints.sort();
    complaints.dedup();
    assert!(
        complaints.is_empty(),
        "{} disagreements between catalog and planner:\n{}",
        complaints.len(),
        complaints.join("\n")
    );
}

// -------------------------------------------------------------------------------------- trimming

/// [`req`] with the batch-wide trim switched on.
fn trimmed(from: &str, to: &str, start: f64, length: f64) -> PlanRequest {
    let mut r = req(from, to);
    r.settings.trim = TrimSettings { enabled: true, start_secs: start, length_secs: length };
    r
}

/// The step FFmpeg runs, whatever else the route does first (Ruffle, LibreOffice, a helper decode).
fn ffmpeg_args(p: &Plan) -> Vec<String> {
    p.steps
        .iter()
        .find(|s| s.tool == Tool::Ffmpeg)
        .map(|s| s.args.clone())
        .unwrap_or_else(|| panic!("no ffmpeg step in {:?}", p.summary))
}

fn arg_after(args: &[String], flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).cloned()
}

/// Where the two arguments go, exactly. `-ss` is an *input* option (fast seek: FFmpeg moves the
/// demuxer instead of decoding and throwing away the first half hour) and `-t` limits what is
/// written after the input is open. Swap either and the app either encodes the whole film or seeks
/// the wrong stream, so the order is pinned rather than described.
#[test]
fn a_trim_seeks_before_the_input_and_limits_the_length_after_it() {
    let p = plan(&trimmed("mov", "mp4", 30.0, 10.0), &all_tools()).unwrap();
    let a = &p.steps[0].args;
    assert_eq!(
        a[..14],
        [
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-y",
            "-ss",
            "30",
            "-i",
            "/in/clip.mov",
            "-t",
            "10",
            "-progress",
            "pipe:1",
            "-nostats",
        ]
        .map(String::from),
        "{a:?}"
    );
    // ...and the rest of the encode is untouched by the trim.
    let joined = a.join(" ");
    assert!(joined.contains("-c:v libx264"), "{joined}");
    assert!(joined.ends_with("/out/clip.mp4"), "{joined}");

    // Off by default means not one extra argument.
    let plain = args_of(&plan(&req("mov", "mp4"), &all_tools()).unwrap(), 0);
    assert!(!plain.contains(" -ss "), "{plain}");
    assert!(!plain.contains(" -t "), "{plain}");
}

/// Audio is a timeline too, and it goes through the same function, so the same two arguments land in
/// the same two places - in front of the codec choice, never in place of it.
#[test]
fn an_audio_route_is_trimmed_the_same_way() {
    let p = plan(&trimmed("mkv", "mp3", 5.5, 10.0), &all_tools()).unwrap();
    let a = &p.steps[0].args;
    let ss = a.iter().position(|x| x == "-ss").unwrap_or_else(|| panic!("{a:?}"));
    let input = a.iter().position(|x| x == "-i").unwrap();
    let t = a.iter().position(|x| x == "-t").unwrap();
    assert!(ss < input, "{a:?}");
    assert_eq!(t, input + 2, "-t comes straight after the input: {a:?}");
    // A fractional start is spelled as a number of seconds, not as 5.500000000000001.
    assert_eq!(a[ss + 1], "5.5");
    assert_eq!(a[t + 1], "10");
    let joined = a.join(" ");
    assert!(joined.contains("-vn"), "{joined}");
    assert!(joined.contains("-c:a libmp3lame -b:a 192k"), "{joined}");
}

/// A trim is meaningless for anything whose output has no duration, and a mixed batch is the normal
/// case: ten clips and one PDF. So those rows are converted exactly as they would have been - the
/// trim is ignored, never refused.
#[test]
fn a_trim_is_ignored_where_the_output_has_no_duration() {
    for (from, to) in [
        ("png", "jpg"),  // still to still
        ("heic", "jpg"), // helper decode, then FFmpeg
        ("png", "gif"),  // a one-frame GIF is not an animation
        ("pdf", "png"),  // page rasterisation
        ("docx", "pdf"), // LibreOffice
        ("md", "html"),  // Pandoc
        ("mkv", "srt"),  // a subtitle track demuxed out of a movie
        ("srt", "vtt"),  // text to text
        ("swf", "png"),  // Ruffle's frame dump: no encoder to hand a trim to
    ] {
        let p = plan(&trimmed(from, to, 30.0, 10.0), &all_tools()).unwrap();
        for (i, step) in p.steps.iter().enumerate() {
            assert!(
                !step.args.iter().any(|a| a == "-ss" || a == "-t"),
                "{from} -> {to} step {i} was trimmed: {:?}",
                step.args
            );
        }
        // Byte for byte the plan the user would have got with trimming off.
        let untrimmed = plan(&req(from, to), &all_tools()).unwrap();
        for (step, expected) in p.steps.iter().zip(untrimmed.steps.iter()) {
            assert_eq!(step.args, expected.args, "{from} -> {to}");
        }
    }
}

/// Every route whose output *is* a timeline, including the two that do not look like video: an
/// animated GIF/WebP, and a video pulled apart into a numbered frame sequence.
#[test]
fn a_trim_reaches_every_route_that_writes_a_timeline() {
    for (from, to) in [
        ("mov", "mp4"),
        ("mp4", "webm"),
        ("mkv", "mp3"),
        ("mp4", "wav"),
        ("mp4", "gif"),
        ("gif", "webp"),
        ("gif", "mp4"),
        ("mp4", "png"), // frame sequence
        ("swf", "mp4"), // Ruffle renders the frames, FFmpeg encodes the movie
    ] {
        let p = plan(&trimmed(from, to, 30.0, 10.0), &all_tools()).unwrap();
        let a = ffmpeg_args(&p);
        assert_eq!(arg_after(&a, "-ss").as_deref(), Some("30"), "{from} -> {to}: {a:?}");
        assert_eq!(arg_after(&a, "-t").as_deref(), Some("10"), "{from} -> {to}: {a:?}");
        let ss = a.iter().position(|x| x == "-ss").unwrap();
        let input = a.iter().position(|x| x == "-i").unwrap();
        assert!(ss < input, "{from} -> {to} must seek before the input: {a:?}");
        assert!(input < a.iter().position(|x| x == "-t").unwrap(), "{from} -> {to}: {a:?}");
    }
}

/// The planner takes the settings as they arrive over IPC, i.e. from two text fields. None of these
/// may become an argument: `-t NaN` fails the encode, `-ss -30` is read as a flag, and
/// `-t 100000000000000000` is a number no muxer should have to think about.
#[test]
fn a_trim_with_impossible_numbers_never_reaches_the_command_line() {
    for (start, length) in
        [(0.0, f64::NAN), (0.0, 0.0), (0.0, -10.0), (f64::INFINITY, f64::INFINITY)]
    {
        let a = args_of(&plan(&trimmed("mov", "mp4", start, length), &all_tools()).unwrap(), 0);
        assert!(!a.contains(" -ss "), "start {start} length {length}: {a}");
        assert!(!a.contains(" -t "), "start {start} length {length}: {a}");
        assert!(!a.contains("NaN") && !a.contains("inf"), "{a}");
    }
    // Numbers that are merely enormous are clamped to a day rather than dropped.
    let a = args_of(&plan(&trimmed("mov", "mp4", 1e30, 1e30), &all_tools()).unwrap(), 0);
    assert!(a.contains("-ss 86400 "), "{a}");
    assert!(a.contains("-t 86400 "), "{a}");

    // And the numbers are inert while the checkbox is off, whatever they say.
    let mut r = req("mov", "mp4");
    r.settings.trim = TrimSettings { enabled: false, start_secs: 30.0, length_secs: 10.0 };
    let a = args_of(&plan(&r, &all_tools()).unwrap(), 0);
    assert!(!a.contains(" -ss ") && !a.contains(" -t "), "{a}");
}

/// A clip shorter than the trim is not a case the command line knows about. The planner is never
/// told a duration, so `-t 10` is the whole of the handling: FFmpeg stops when the input ends, and
/// nothing pads, refuses or mentions it. (The queue owns the other half - how long the *output* is
/// expected to be - because it is the only layer that has probed the file.)
#[test]
fn a_source_shorter_than_the_trim_needs_no_handling_in_the_argv() {
    let a = args_of(&plan(&trimmed("mov", "mp4", 0.0, 10.0), &all_tools()).unwrap(), 0);
    assert!(a.contains("-t 10"), "{a}");
    assert!(!a.contains("shortest"), "no muxer flag stands in for a short input: {a}");
    assert!(!a.contains("tpad") && !a.contains("apad"), "nothing is padded to length: {a}");
    // The request the planner answers has no duration in it at all, which is what makes the argv
    // for a 6 second clip and a 6 hour film identical.
    assert!(
        !format!("{:?}", trimmed("mov", "mp4", 0.0, 10.0)).contains("duration"),
        "a PlanRequest that carried a duration would invite a second, disagreeing calculation"
    );
}

/// `-ss`/`-t` are decided by what the *output* is, so this is the predicate both the planner and the
/// queue read. Pinned directly: the queue's refusal and the progress bar's expected duration hang
/// off the same answer, and a disagreement between them is the bug that shows an empty file as a
/// success.
#[test]
fn only_time_based_outputs_are_trimmable() {
    let time_based = |from: &str, to: &str| {
        let r = req(from, to);
        output_is_time_based(r.source, r.target, r.source_is_animated)
    };
    for (from, to) in
        [("mov", "mp4"), ("mkv", "mp3"), ("mp4", "gif"), ("gif", "webp"), ("mp4", "png")]
    {
        assert!(time_based(from, to), "{from} -> {to}");
    }
    for (from, to) in [
        ("png", "jpg"),
        ("png", "gif"),
        ("heic", "png"),
        ("pdf", "png"),
        ("docx", "pdf"),
        ("md", "html"),
        ("srt", "vtt"),
        ("mkv", "srt"),
        ("swf", "png"),
    ] {
        assert!(!time_based(from, to), "{from} -> {to}");
    }
    // Flash is the pair that needs both answers: rendered to a movie it is a timeline, dumped as
    // stills it is a folder of pictures.
    assert!(time_based("swf", "mp4"));
    assert!(time_based("swf", "gif"));
}

#[test]
fn midi_is_piano_input_only_and_copy_settings_still_encode_audio() {
    let mut request = PlanRequest {
        input: PathBuf::from("/songs/piano.mid"),
        output: PathBuf::from("/out/piano.mp3"),
        source: by_id("midi").unwrap(),
        target: by_id("mp3").unwrap(),
        settings: Settings::default(),
        temp_dir: PathBuf::from("/scratch/job"),
        source_is_animated: false,
    };
    request.settings.audio.codec = AudioCodec::Copy;
    let result = plan(&request, &only_ffmpeg()).unwrap();
    assert_eq!(result.midi, Some((request.input.clone(), request.temp_dir.join("midi-piano.wav"))));
    assert!(result.steps[0].args.iter().any(|arg| arg.ends_with("midi-piano.wav")));
    assert!(!result.steps[0].args.iter().any(|arg| arg == "copy"));
    request.source = by_id("wav").unwrap();
    request.target = by_id("midi").unwrap();
    assert!(matches!(plan(&request, &only_ffmpeg()), Err(PlanError::UnsupportedPair { .. })));
    assert!(crate::format::by_extension("MID").is_some());
}
