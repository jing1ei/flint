//! Command lines checked against the real tools, when the real tools are here.
//!
//! Everything in the unit tests asserts what argv the planner *builds*; nothing there can tell
//! whether FFmpeg or Pandoc accepts it. These do - they plan a conversion and run it. Every test
//! gates on [`ToolRegistry::discover`] and returns quietly when the tool it needs is missing, so a
//! machine (or CI runner) without Pandoc, or with an FFmpeg built without some encoder, still gets
//! a clean `cargo test`: a test that fails for want of an optional helper is a test people learn to
//! ignore.
//!
//! They are deliberately few. Each one exists because it caught something.

use convert_core::engine::Engine;
use convert_core::format::{by_extension, by_id, Tool};
use convert_core::link::{self, FetchOptions, Link};
use convert_core::plan::{output_extension, plan, PlanRequest};
use convert_core::settings::{CookieFlag, Settings, COOKIE_BROWSERS};
use convert_core::tools::ToolRegistry;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// The tools this machine actually has, versions included (that is what `discover` adds).
fn discovered() -> ToolRegistry {
    ToolRegistry::discover(None)
}

/// `None` and a note on stderr when the tool is absent - the caller returns and the test passes.
fn require(tools: &ToolRegistry, tool: Tool) -> Option<PathBuf> {
    match tools.path(tool) {
        Some(p) => Some(p.to_path_buf()),
        None => {
            eprintln!("skipping: {} is not installed on this machine", tool.label());
            None
        }
    }
}

/// Is this FFmpeg built with the encoder a case needs? Encoder sets differ wildly between builds
/// (Debian 12 ships 5.1 with no `wbmp` or `hdr`; the macOS sidecars ship 7.x with both), and that
/// is a property of the machine, not a bug in the planner.
fn has_encoder(ffmpeg: &Path, name: &str) -> bool {
    lists(ffmpeg, "-encoders").iter().any(|n| n == name)
}

/// Same question for a muxer. A container is not a codec: a slim build can have `libx264` and no
/// `mxf` muxer, and then nothing about that case is ours to fix.
fn has_muxer(ffmpeg: &Path, name: &str) -> bool {
    lists(ffmpeg, "-muxers").iter().any(|n| n == name)
}

/// The names in one of FFmpeg's `-encoders` / `-muxers` tables (second column of each row).
fn lists(ffmpeg: &Path, what: &str) -> Vec<String> {
    let out = match Command::new(ffmpeg).args(["-hide_banner", what]).output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1).map(str::to_string))
        .collect()
}

/// `false` and a note on stderr when this FFmpeg cannot build the fixture (a build without the
/// encoder, or without the `lavfi` input these fixtures are synthesised from). The caller returns:
/// a test that fails because of how somebody's FFmpeg was configured is a test people learn to
/// ignore, and none of these cases say anything about the planner.
fn ffmpeg_make(ffmpeg: &Path, args: &[&str]) -> bool {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-nostdin", "-y", "-v", "error"]).args(args);
    match cmd.output() {
        Ok(out) if out.status.success() => true,
        Ok(out) => {
            eprintln!(
                "skipping: this FFmpeg could not build the fixture: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            false
        }
        Err(e) => {
            eprintln!("skipping: {} could not be started: {e}", ffmpeg.display());
            false
        }
    }
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cc-real-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Plan `input -> to` into `dir/<out_name>` and run it, exactly as the queue would.
fn convert(
    tools: &ToolRegistry,
    input: &Path,
    from: &str,
    to: &str,
    out_name: &str,
    dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    let target = by_id(to).unwrap();
    let req = PlanRequest {
        input: input.to_path_buf(),
        output: dir.join(format!("{out_name}.{}", output_extension(target))),
        source: by_extension(from).unwrap_or_else(|| panic!("no source format for .{from}")),
        target,
        settings: Settings::default(),
        temp_dir: dir.join("tmp"),
        source_is_animated: matches!(from, "mp4" | "mov" | "gif"),
    };
    let engine = Engine::new(tools.clone());
    let p = plan(&req, &engine.tools).map_err(|e| format!("plan: {e}"))?;
    let argv: Vec<String> =
        p.steps.iter().map(|s| format!("{} {}", s.program.display(), s.args.join(" "))).collect();
    engine
        .run(&p, None, &Arc::new(AtomicBool::new(false)), &mut |_| {})
        .map(|o| o.outputs)
        .map_err(|e| format!("{e}\nargv: {}", argv.join("\n      ")))
}

/// The one flag in the whole planner whose spelling depends on the installed version. Running it is
/// the only way to know we chose right: Pandoc 2.17 exits 6 on `--embed-resources`, and 3.x only
/// warns on `--self-contained` today. Whatever this machine has, the command must work.
#[test]
fn the_pandoc_html_command_runs_on_the_pandoc_that_is_installed() {
    let tools = discovered();
    let Some(pandoc) = require(&tools, Tool::Pandoc) else { return };
    let dir = scratch("pandoc");
    let input = dir.join("doc.md");
    std::fs::write(&input, "# Title\n\nHello *world*.\n").unwrap();

    let outputs = convert(&tools, &input, "md", "html", "res", &dir).expect("md -> html");
    let html = std::fs::read_to_string(&outputs[0]).unwrap();
    assert!(html.contains("<h1"), "not standalone HTML: {html:.200}");
    eprintln!("pandoc at {} accepted the plan", pandoc.display());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The MXF muxer refuses 44.1 kHz audio outright ("only 48khz is implemented") and refuses any
/// non-broadcast frame rate unless `-strict unofficial` is set. A 15 fps screen recording is the
/// common case, so this pair - forced sample rate plus the strictness flag - is what makes MXF work
/// at all; drop either one and this test fails on any FFmpeg build.
#[test]
fn mxf_accepts_a_44100_hz_15_fps_source() {
    let tools = discovered();
    let Some(ffmpeg) = require(&tools, Tool::Ffmpeg) else { return };
    // The fixture is H.264 and the target is an MXF: both are properties of this FFmpeg build.
    if !has_encoder(&ffmpeg, "libx264") || !has_muxer(&ffmpeg, "mxf") {
        eprintln!("skipping: this FFmpeg has no libx264 encoder or no mxf muxer");
        return;
    }
    let dir = scratch("mxf");
    let input = dir.join("clip.mp4");
    let made = ffmpeg_make(
        &ffmpeg,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=160x120:rate=15:duration=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100:duration=1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            input.to_str().unwrap(),
        ],
    );
    if !made {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let outputs = convert(&tools, &input, "mp4", "mxf", "res", &dir).expect("mp4 -> mxf");
    assert!(outputs[0].metadata().unwrap().len() > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two halves of the same trap: the image2 muxer reads its *output* path as a printf pattern, and
/// the image2 demuxer reads its *input* path as one. An unescaped `%` in a folder or file name used
/// to make "extract frames" write one file and then fail, and a still image genuinely called
/// `take%04d.png` used to be searched for as a numbered sequence that does not exist.
#[test]
fn a_literal_percent_in_a_path_is_not_read_as_a_frame_pattern() {
    let tools = discovered();
    let Some(ffmpeg) = require(&tools, Tool::Ffmpeg) else { return };
    if !has_encoder(&ffmpeg, "libx264") {
        eprintln!("skipping: this FFmpeg has no libx264 encoder for the fixture");
        return;
    }
    let dir = scratch("percent");

    // Output side: frames extracted into a folder whose name contains a percent sign.
    let clip = dir.join("clip.mp4");
    let made = ffmpeg_make(
        &ffmpeg,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=64x48:rate=5:duration=3",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            clip.to_str().unwrap(),
        ],
    );
    if !made {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let odd = dir.join("50% off");
    std::fs::create_dir_all(&odd).unwrap();
    let frames =
        convert(&tools, &clip, "mp4", "png", "100%_done", &odd).expect("mp4 -> png frames");
    assert!(frames.len() > 1, "expected a sequence, got {frames:?}");
    for f in &frames {
        assert!(f.exists(), "{f:?} was reported but not written");
        let name = f.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("100%_done-"), "the user's `%` came back doubled: {name}");
    }

    // Input side: a single still image whose name is itself a valid sequence pattern. It has to be
    // *copied* into place - handing the name to FFmpeg as an output would expand it to `take0001`,
    // which is the same confusion from the other end - and it has to sit in a folder of its own, or
    // image2 finds a numbered sibling and the bug hides.
    let alone = dir.join("alone");
    std::fs::create_dir_all(&alone).unwrap();
    let plain = dir.join("still.png");
    if !ffmpeg_make(
        &ffmpeg,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=32x32:d=1",
            "-frames:v",
            "1",
            plain.to_str().unwrap(),
        ],
    ) {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let still = alone.join("take%04d.png");
    std::fs::copy(&plain, &still).unwrap();
    convert(&tools, &still, "png", "jpg", "res", &alone).expect("png -> jpg");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The catalog says subtitles convert in one direction only for `.sub` (decoder, no encoder) and
/// TTML (encoder, no demuxer). This is the half that has to actually run; the half that has to be
/// refused before spawning anything is asserted in the planner's own tests.
#[test]
fn ffmpeg_really_writes_ttml_and_really_cannot_read_it() {
    let tools = discovered();
    let Some(ffmpeg) = require(&tools, Tool::Ffmpeg) else { return };
    if !has_encoder(&ffmpeg, "ttml") {
        eprintln!("skipping: this FFmpeg has no ttml encoder");
        return;
    }
    let dir = scratch("subs");
    let input = dir.join("subs.srt");
    std::fs::write(&input, "1\n00:00:00,000 --> 00:00:01,000\nhello <i>world</i>\n\n").unwrap();

    let outputs = convert(&tools, &input, "srt", "ttml", "res", &dir).expect("srt -> ttml");
    let ttml = std::fs::read_to_string(&outputs[0]).unwrap();
    assert!(ttml.contains("<tt"), "{ttml:.200}");

    // ...and the reverse really is impossible, so the catalog is not being pessimistic: ask FFmpeg
    // directly rather than trusting a plan that (correctly) refuses to build.
    let probe = Command::new(&ffmpeg)
        .args(["-hide_banner", "-v", "error", "-i"])
        .arg(&outputs[0])
        .args(["-f", "srt", "-y", dir.join("back.srt").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!probe.status.success(), "FFmpeg grew a TTML demuxer - the catalog can claim reading");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Formats whose encoder is an *external library* cannot be promised by a catalog compiled once:
/// the Apple Silicon FFmpeg the app ships (osxexperts 7.1.1) is built without libopencore-amrnb and
/// without libgsm, so AMR and GSM output is impossible there however the argv is written. If this
/// machine's FFmpeg happens to have the encoder, that is a property of this machine - the test only
/// insists that whatever we *claim*, we can do.
#[test]
fn the_catalog_never_claims_an_encoder_this_ffmpeg_lacks_for_speech_formats() {
    let tools = discovered();
    let Some(ffmpeg) = require(&tools, Tool::Ffmpeg) else { return };
    for (id, encoder) in [("amr", "libopencore_amrnb"), ("gsm", "libgsm")] {
        let claimed = by_id(id).unwrap().write.is_supported();
        if claimed && !has_encoder(&ffmpeg, encoder) {
            panic!("the catalog offers .{id} output but this FFmpeg has no {encoder} encoder");
        }
    }
}

/// LibreOffice is the only helper this app runs *concurrently* (a batch of documents is the whole
/// premise), and the two things that go wrong there only go wrong when it really runs: two
/// instances sharing the default user profile - the second refuses to start or hangs - and a PDF
/// export filter that belongs to another application, which is rejected with "no export filter".
/// A spreadsheet and a text document are converted at the same time to catch both at once.
///
/// Skipped, quietly, on a machine without LibreOffice.
#[test]
fn libreoffice_converts_two_documents_at_once_with_the_right_pdf_filters() {
    let tools = discovered();
    let Some(soffice) = require(&tools, Tool::LibreOffice) else { return };
    let dir = scratch("soffice");
    let sheet = dir.join("sheet.csv");
    std::fs::write(&sheet, "Region,Units\nNorth,12\nSouth,9\n").unwrap();
    let letter = dir.join("letter.txt");
    std::fs::write(&letter, "Dear reader,\n\nHello from Flint.\n").unwrap();
    // Separate destinations, so each job gets its own scratch (and therefore its own profile),
    // exactly as the queue hands them out.
    let calc_dir = dir.join("calc");
    let writer_dir = dir.join("writer");
    std::fs::create_dir_all(&calc_dir).unwrap();
    std::fs::create_dir_all(&writer_dir).unwrap();

    let (calc, writer) = std::thread::scope(|s| {
        let calc = s.spawn(|| convert(&tools, &sheet, "csv", "pdf", "sheet", &calc_dir));
        let writer = s.spawn(|| convert(&tools, &letter, "txt", "pdf", "letter", &writer_dir));
        (calc.join().unwrap(), writer.join().unwrap())
    });
    let calc = calc.expect("csv -> pdf while a second LibreOffice was running");
    let writer = writer.expect("txt -> pdf while a second LibreOffice was running");

    for out in [&calc[0], &writer[0]] {
        let head = std::fs::read(out).unwrap();
        assert!(head.starts_with(b"%PDF"), "{} is not a PDF", out.display());
    }
    eprintln!("{} printed both documents at the same time", soffice.display());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A one-page PDF with one line of text, built here rather than checked in: every other fixture in
/// this file is made by the tools themselves, and a binary blob in the repo is a fixture nobody can
/// read or edit. The offsets are computed, so the xref table is real and Poppler has nothing to
/// repair - a PDF it had to reconstruct would prove less than this one does.
fn minimal_pdf(line: &str) -> Vec<u8> {
    let stream = format!("BT /F1 18 Tf 20 40 Td ({line}) Tj ET\n");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 100] \
         /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
            .to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
    ];

    let mut out = String::from("%PDF-1.4\n");
    let mut offsets: Vec<usize> = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
    }
    let start_xref = out.len();
    out.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1));
    for offset in offsets {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start_xref}\n%%EOF\n",
        objects.len() + 1
    ));
    out.into_bytes()
}

/// PDF -> text and PDF -> HTML are Poppler's job now (LibreOffice is 800 MB for the same answer),
/// and the flags only fail when they run: `-dataurls` did not exist before Poppler 0.75, and
/// without `-s -noframes` `pdftohtml` writes a frameset plus a `page<n>.html` per page - so the
/// file the plan names would be a wrapper pointing at siblings, which is not what the user asked
/// for and not what survives an email.
#[test]
fn the_poppler_commands_write_one_readable_file_each() {
    let tools = discovered();
    for (tool, target) in [(Tool::PdfToText, "txt"), (Tool::PdfToHtml, "html")] {
        let Some(program) = require(&tools, tool) else { continue };
        let dir = scratch(&format!("poppler-{target}"));
        let input = dir.join("doc.pdf");
        std::fs::write(&input, minimal_pdf("Hello Flint")).unwrap();

        let outputs = convert(&tools, &input, "pdf", target, "res", &dir)
            .unwrap_or_else(|e| panic!("pdf -> {target}: {e}"));
        assert_eq!(outputs.len(), 1, "one file, not a pile: {outputs:?}");
        let text = std::fs::read_to_string(&outputs[0]).unwrap();
        assert!(text.contains("Hello Flint"), "pdf -> {target}: {text:.300}");

        // Self-contained: no extracted images and no per-page siblings next to the result.
        let strays: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".png") || n.starts_with("page"))
            .collect();
        assert!(strays.is_empty(), "pdf -> {target} left {strays:?} beside the output");

        eprintln!("{} accepted the plan", program.display());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Every flag the link feature emits, checked against the yt-dlp that is actually installed.
///
/// yt-dlp renames and retires options between releases (`--no-part` and `--newline` are old,
/// `--print` arrived in 2021, `--ffmpeg-location` moved from youtube-dl unchanged), and a flag this
/// build does not know is an instant "unrecognised arguments" failure on every single link. The
/// unit tests can only assert what argv we *build*; this asserts the installed binary knows it.
///
/// Deliberately network-free: nothing here downloads, resolves a host or opens a socket, so it is
/// safe in CI. It skips cleanly when yt-dlp is absent, which is the normal case.
#[test]
fn the_yt_dlp_flags_exist_in_the_yt_dlp_that_is_installed() {
    let tools = discovered();
    let Some(program) = require(&tools, Tool::YtDlp) else { return };

    let version = Command::new(&program).arg("--version").output();
    match version {
        Ok(out) if out.status.success() => {
            eprintln!("yt-dlp {}", String::from_utf8_lossy(&out.stdout).trim());
        }
        _ => {
            eprintln!("skipping: {} could not be run", program.display());
            return;
        }
    }

    let help = match Command::new(&program).arg("--help").output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(e) => {
            eprintln!("skipping: {} could not be run: {e}", program.display());
            return;
        }
    };

    let link = Link::parse("https://www.youtube.com/watch?v=dQw4w9WgXcQ").expect("a valid link");
    // A runtime with a made-up path: nothing here runs it, and `--js-runtimes` has to be a flag the
    // installed yt-dlp knows whether or not this machine happens to have Deno.
    let runtime = link::JsRuntime::new(Tool::Deno, PathBuf::from("/nowhere/deno"))
        .expect("Deno is a runtime yt-dlp accepts");
    // A cookie source with a made-up path, for the same reason: nothing here reads a jar or opens
    // a file, and `--cookies` has to be a flag the installed yt-dlp knows either way.
    let cookies = CookieFlag::File(PathBuf::from("/nowhere/cookies.txt"));
    let options = FetchOptions {
        ffmpeg: Some(PathBuf::from("/nowhere/ffmpeg")),
        js_runtime: Some(runtime.clone()),
        dir: PathBuf::from("/nowhere/scratch"),
        audio_only: true,
        cookies: Some(cookies.clone()),
    };
    let mut argv = link::fetch_args(&link, &options);
    argv.extend(link::probe_args(&link, Some(&runtime), Some(&cookies)));
    // ...and the other of the two cookie flags, which the argv above cannot contain at once.
    argv.extend(link::fetch_args(
        &link,
        &FetchOptions { cookies: Some(CookieFlag::Browser("safari")), ..options.clone() },
    ));

    for flag in argv.iter().filter(|a| a.starts_with('-') && a.len() > 1 && *a != "--") {
        assert!(
            help.contains(flag.as_str()),
            "this yt-dlp does not know `{flag}`:\n{}",
            argv.join(" ")
        );
    }
    // ...and the runtime names we may put in front of the colon are yt-dlp's own, which is the fact
    // `Tool::js_runtime_name` claims and only the installed binary can confirm.
    for tool in convert_core::format::JS_RUNTIMES {
        let name = tool.js_runtime_name().expect("a runtime name");
        assert!(help.contains(name), "this yt-dlp does not list `{name}` as a JavaScript runtime");
    }
    // The browser allowlist is the same kind of claim: those eight strings are spelled the way
    // `--cookies-from-browser` spells them, and only the installed binary can confirm it. A name we
    // got wrong would fail every link the user configured it for, with yt-dlp's own usage text.
    let browsers = help
        .split("--cookies-from-browser")
        .nth(1)
        .expect("this yt-dlp has no --cookies-from-browser");
    // Bounded to that option's own paragraph, so a name that happens to appear elsewhere in the
    // help text cannot pass this off.
    let browsers = browsers.split("\n    --").next().unwrap_or(browsers);
    for name in COOKIE_BROWSERS {
        assert!(
            browsers.contains(name),
            "this yt-dlp does not list `{name}` as a browser it can take cookies from"
        );
    }

    // The URL is the last argument and sits behind a `--`, whatever else changes.
    let fetch = link::fetch_args(&link, &options);
    assert_eq!(fetch.last().map(String::as_str), Some(link.url()));
    assert_eq!(fetch[fetch.len() - 2], "--");
}
