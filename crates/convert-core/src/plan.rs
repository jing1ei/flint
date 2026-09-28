//! The planner: `(input format, target format, settings)` -> an executable list of steps.
//!
//! Nothing here touches the filesystem or spawns a process, which makes the interesting part of the
//! app (the command lines) fully unit-testable - see the tests at the bottom.

use crate::format::{Category, Format, Tool};
use crate::settings::{AudioCodec, HardwareAccel, QualityLevel, Settings, VideoCodec};
use crate::tools::ToolRegistry;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Step {
    pub tool: Tool,
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Share of the job's progress bar, 0..1 across all steps.
    pub weight: f32,
    /// `true` when the step emits `-progress pipe:1` key/value output we can parse.
    pub ffmpeg_progress: bool,
    pub label: String,
    /// Some tools insist on naming their own output; the runner fixes that up afterwards.
    pub post: Option<PostAction>,
    /// Directories this step needs to exist before it runs. Declared here rather than guessed by
    /// the engine: only the planner knows which argument is a directory and which is a file.
    pub ensure_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PostAction {
    /// Move `from` to the step's intended output path.
    MoveFrom(PathBuf),
    /// The step wrote a numbered sequence `<prefix>-<n>.<extension>` into `dir`; those files (and
    /// only those - anything already in the directory before the step ran is somebody else's) are
    /// the job's result. When `dir` is not where the output belongs, the engine moves them there.
    CollectSequence { dir: PathBuf, prefix: String, extension: String },
}

#[derive(Debug, Clone)]
pub struct Plan {
    /// MIDI source and private PCM intermediate, rendered before external encoding.
    pub midi: Option<(PathBuf, PathBuf)>,
    pub steps: Vec<Step>,
    /// Where the single-file result belongs. A job that writes a numbered sequence declares it with
    /// [`PostAction::CollectSequence`]; the engine records exactly the files that step created.
    pub output: PathBuf,
    /// Short human sentence shown under the file row, e.g. "H.264 1080p · AAC 192k".
    pub summary: String,
    pub temp_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PlanError {
    #[error("{from} cannot be converted to {to}")]
    UnsupportedPair { from: &'static str, to: &'static str },
    #[error("{format} needs {tool} installed ({hint})")]
    MissingTool { format: &'static str, tool: &'static str, hint: &'static str },
    #[error("unknown format `{0}`")]
    UnknownFormat(String),
}

#[derive(Debug, Clone)]
pub struct PlanRequest {
    pub input: PathBuf,
    /// Final destination (already de-duplicated by the caller).
    pub output: PathBuf,
    pub source: &'static Format,
    pub target: &'static Format,
    pub settings: Settings,
    /// Scratch directory for intermediate files, unique per job.
    pub temp_dir: PathBuf,
    /// `true` when the source has more than one frame (video, animated GIF/WebP, Flash).
    pub source_is_animated: bool,
}

/// File extension used for a target format.
///
/// Formats that live in a differently named container (`alac` -> `.m4a`, `pdf_page` -> `.pdf`)
/// simply declare that container in `extensions[0]`; APNG is the one real exception, because
/// `.apng` is a genuine input extension but every viewer expects the output to be called `.png`.
pub fn output_extension(target: &Format) -> &'static str {
    match target.id {
        "apng" => "png",
        _ => target.extensions[0],
    }
}

/// The default target every category converts to on a single click.
pub fn default_target_for(category: Category) -> &'static str {
    match category {
        Category::Video | Category::Flash => "mp4",
        Category::Audio => "mp3",
        Category::Image => "jpg",
        Category::Document => "pdf",
        Category::Subtitle => "srt",
    }
}

/// Targets we surface as one-click chips in the UI (the long tail stays in the dropdown).
pub fn suggested_targets_for(category: Category) -> &'static [&'static str] {
    match category {
        Category::Video => &["mp4", "webm", "mov", "gif", "mp3"],
        Category::Flash => &["mp4", "gif", "png"],
        Category::Audio => &["mp3", "m4a", "wav", "flac", "opus"],
        Category::Image => &["jpg", "png", "webp", "avif", "pdf_page"],
        Category::Document => &["pdf", "docx", "md", "html", "txt"],
        Category::Subtitle => &["srt", "vtt", "ass"],
    }
}

pub fn plan(req: &PlanRequest, tools: &ToolRegistry) -> Result<Plan, PlanError> {
    // Same answer either way: a source we cannot read and a target we cannot write are both just
    // "not a conversion this app does".
    if !req.source.read.is_supported() || !req.target.write.is_supported() {
        return Err(PlanError::UnsupportedPair { from: req.source.id, to: req.target.id });
    }

    if req.source.id == "midi" {
        if req.target.category != Category::Audio {
            return Err(PlanError::UnsupportedPair { from: req.source.id, to: req.target.id });
        }
        let mut audio = req.clone();
        audio.input = req.temp_dir.join("midi-piano.wav");
        audio.source = crate::format::by_id("wav").expect("WAV in catalog");
        // MIDI has no encoded audio stream to copy.
        if audio.settings.audio.codec == AudioCodec::Copy {
            audio.settings.audio.codec = AudioCodec::Auto;
        }
        let mut result = plan(&audio, tools)?;
        result.midi = Some((req.input.clone(), audio.input.clone()));
        result.temp_paths.push(audio.input);
        result.summary = format!("Piano render · {}", result.summary);
        return Ok(result);
    }

    let route = route_for(req.source, req.target, tools)?;
    match route {
        Route::Ffmpeg => {
            let ffmpeg = need(tools, &[Tool::Ffmpeg], req.target.id)?;
            let step = ffmpeg_step(tools, ffmpeg, &req.input, &req.output, req, None)?;
            Ok(Plan {
                midi: None,
                summary: summarize(req),
                output: req.output.clone(),
                temp_paths: vec![],
                steps: vec![step],
            })
        }
        Route::HelperEncode => {
            let encoder = need(tools, &encoders_for(req.target), req.target.id)?;
            let step = helper_encode_step(
                tools,
                encoder,
                &req.input,
                &req.output,
                req.target,
                &req.settings,
            );
            Ok(Plan {
                midi: None,
                summary: summarize(req),
                output: req.output.clone(),
                temp_paths: vec![],
                steps: vec![step],
            })
        }
        Route::DecodeThenEncode => {
            // HEIC / RAW / SVG / PSD: a helper renders a lossless PNG, then whoever can write the
            // target finishes the job.
            let decoder = need(tools, &decoders_for(req.source), req.source.id)?;
            let temp = req.temp_dir.join("decoded.png");
            let mut steps = vec![decode_step(tools, decoder, &req.input, &temp, &req.settings)];
            if writes_pdf(req.target) {
                let magick = need(tools, &[Tool::Magick], req.target.id)?;
                steps.push(image_to_pdf_step(tools, magick, &temp, &req.output, &req.settings));
            } else if req.target.write.needs_helper() {
                let encoder = need(tools, &encoders_for(req.target), req.target.id)?;
                steps.push(helper_encode_step(
                    tools,
                    encoder,
                    &temp,
                    &req.output,
                    req.target,
                    &req.settings,
                ));
            } else {
                let ffmpeg = need(tools, &[Tool::Ffmpeg], req.target.id)?;
                steps.push(ffmpeg_step(tools, ffmpeg, &temp, &req.output, req, None)?);
            }
            rebalance(&mut steps);
            Ok(Plan {
                midi: None,
                summary: summarize(req),
                output: req.output.clone(),
                temp_paths: vec![temp],
                steps,
            })
        }
        Route::ImageToPdf => {
            let magick = need(tools, &[Tool::Magick], req.target.id)?;
            let step = image_to_pdf_step(tools, magick, &req.input, &req.output, &req.settings);
            Ok(Plan {
                midi: None,
                summary: "PDF page".into(),
                output: req.output.clone(),
                temp_paths: vec![],
                steps: vec![step],
            })
        }
        Route::PdfRaster => {
            let tool = need(tools, &[Tool::PdfToPpm, Tool::Magick, Tool::Sips], req.source.id)?;
            let step = pdf_raster_step(tools, tool, req, &req.input);
            Ok(Plan {
                midi: None,
                summary: raster_summary(tool, req),
                output: req.output.clone(),
                temp_paths: vec![],
                steps: vec![step],
            })
        }
        Route::OfficeThenPdfRaster => {
            let soffice = need(tools, &[Tool::LibreOffice], req.source.id)?;
            let raster = need(tools, &[Tool::PdfToPpm, Tool::Magick, Tool::Sips], req.target.id)?;
            let temp_pdf = office_product(req, &req.input, "pdf");
            let mut office = office_step(tools, soffice, &req.input, &temp_pdf, req, "pdf");
            // Keep LibreOffice's own output where it landed; the rasteriser reads it directly.
            office.post = None;
            let mut steps = vec![office, pdf_raster_step(tools, raster, req, &temp_pdf)];
            rebalance(&mut steps);
            Ok(Plan {
                midi: None,
                summary: format!("via PDF · {}", raster_summary(raster, req)),
                output: req.output.clone(),
                temp_paths: vec![temp_pdf],
                steps,
            })
        }
        Route::Office => {
            let soffice = need(tools, &[Tool::LibreOffice], req.target.id)?;
            Ok(office_plan(tools, soffice, req))
        }
        Route::PdfText => {
            // A PDF already contains its text; Poppler reads it out directly. LibreOffice can do
            // it too, by re-importing the whole page layout into Writer - 800 MB of helper for a
            // job `pdftotext` does in 15, so it is the fallback rather than the plan.
            let poppler = pdf_text_tool(req.target.id)
                .expect("route_for only sends txt/html targets down this route");
            let tool = need(tools, &[poppler, Tool::LibreOffice], req.target.id)?;
            if tool == Tool::LibreOffice {
                return Ok(office_plan(tools, tool, req));
            }
            Ok(Plan {
                midi: None,
                summary: format!("Poppler → {}", output_extension(req.target)),
                output: req.output.clone(),
                temp_paths: vec![],
                steps: vec![pdf_text_step(tools, tool, &req.input, &req.output)],
            })
        }
        Route::Pandoc => {
            let pandoc = need(tools, &[Tool::Pandoc], req.target.id)?;
            let step = pandoc_step(tools, pandoc, &req.input, &req.output);
            Ok(Plan {
                midi: None,
                summary: format!("Pandoc → {}", output_extension(req.target)),
                output: req.output.clone(),
                temp_paths: vec![],
                steps: vec![step],
            })
        }
        Route::MarkupToPdf => {
            // Pandoc has no PDF engine unless LaTeX is installed, so we render HTML and let
            // LibreOffice print it. Two cheap steps beat asking the user to install MacTeX.
            let pandoc = need(tools, &[Tool::Pandoc], "pdf")?;
            let soffice = need(tools, &[Tool::LibreOffice], "pdf")?;
            let temp = req.temp_dir.join("intermediate.html");
            let mut steps = vec![
                pandoc_step(tools, pandoc, &req.input, &temp),
                office_step(tools, soffice, &temp, &req.output, req, "pdf"),
            ];
            rebalance(&mut steps);
            Ok(Plan {
                midi: None,
                summary: "Markdown → HTML → PDF".into(),
                output: req.output.clone(),
                temp_paths: vec![temp],
                steps,
            })
        }
        Route::Flash => {
            let ruffle = need(tools, &[Tool::Ruffle], req.source.id)?;
            let frames_dir = req.temp_dir.join("frames");
            let mut steps = vec![flash_export_step(tools, ruffle, &req.input, &frames_dir)];
            let summary =
                if req.target.category == Category::Image && !animated_image(req.target.id) {
                    // "Give me the frames" - Ruffle already wrote PNGs, so this job needs no encoder.
                    // Asking for FFmpeg here refused the whole conversion on a machine whose sidecar
                    // never made it into the bundle, for a step that would not have run.
                    steps[0].post = Some(PostAction::CollectSequence {
                        dir: frames_dir.clone(),
                        prefix: String::new(),
                        extension: "png".into(),
                    });
                    "Ruffle render → PNG frames".to_string()
                } else {
                    let ffmpeg = need(tools, &[Tool::Ffmpeg], req.target.id)?;
                    let glob = frames_dir.join("*.png");
                    steps.push(ffmpeg_step(
                        tools,
                        ffmpeg,
                        &frames_dir,
                        &req.output,
                        req,
                        Some(glob.to_string_lossy().to_string()),
                    )?);
                    "Ruffle render → encode".to_string()
                };
            rebalance(&mut steps);
            Ok(Plan {
                midi: None,
                summary,
                output: req.output.clone(),
                temp_paths: vec![frames_dir],
                steps,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Ffmpeg,
    /// A helper renders the source to a lossless PNG, then whoever can write the target takes over.
    DecodeThenEncode,
    /// FFmpeg cannot encode the target at all (HEIC, ICNS, AI): the helper writes it directly.
    HelperEncode,
    ImageToPdf,
    PdfRaster,
    Office,
    OfficeThenPdfRaster,
    /// PDF -> plain text / HTML, read straight out of the PDF by Poppler (`pdftotext`,
    /// `pdftohtml`), with LibreOffice as the fallback.
    PdfText,
    Pandoc,
    MarkupToPdf,
    Flash,
}

/// Formats Pandoc can read / write. Pandoc keeps structure (headings, tables, footnotes) so it wins
/// over LibreOffice for anything markup-ish.
fn pandoc_readable(id: &str) -> bool {
    matches!(
        id,
        "md" | "html"
            | "txt"
            | "rst"
            | "tex"
            | "epub"
            | "fb2"
            | "json_doc"
            | "docx"
            | "odt"
            | "csv"
    )
}

fn pandoc_writable(id: &str) -> bool {
    matches!(
        id,
        "md" | "html"
            | "txt"
            | "rst"
            | "tex"
            | "epub"
            | "fb2"
            | "json_doc"
            | "docx"
            | "odt"
            | "rtf"
            | "pptx"
    )
}

fn route_for(
    source: &'static Format,
    target: &'static Format,
    tools: &ToolRegistry,
) -> Result<Route, PlanError> {
    use Category::*;
    let pair = (source.category, target.category);
    let unsupported = || PlanError::UnsupportedPair { from: source.id, to: target.id };

    // Flash always starts with a Ruffle render.
    if source.category == Flash {
        return match target.category {
            Video => Ok(Route::Flash),
            // An animated image is encoded from the rendered frames, so any of those is fine. A
            // still target is the frames themselves - and Ruffle only writes PNG, so promising
            // `.webp` would hand the user PNG bytes under the wrong extension.
            Image if animated_image(target.id) || target.id == "png" => Ok(Route::Flash),
            _ => Err(unsupported()),
        };
    }

    match pair {
        // ---- media
        (Video, Video) | (Video, Audio) | (Audio, Audio) | (Subtitle, Subtitle) => {
            Ok(Route::Ffmpeg)
        }
        // frames, or GIF/WebP/APNG animation. FFmpeg has to be able to write the target: there is
        // no way to hand a frame of an MP4 to `sips`, so a HEIC/ICNS target is simply not on offer.
        (Video, Image) => encoder_route(source, target, unsupported),
        (Image, Video) => Ok(Route::Ffmpeg), // animated GIF/WebP -> real video
        (Image, Image) => {
            if source.read.needs_helper() {
                Ok(Route::DecodeThenEncode)
            } else if writes_pdf(target) {
                Ok(Route::ImageToPdf)
            } else {
                encoder_route(source, target, unsupported)
            }
        }
        // ---- documents
        (Document, Image) => {
            // The rasterisers write JPEG or PNG and nothing else. Accepting another image target
            // used to produce PNG bytes in a file named `.webp`.
            if !matches!(target.id, "jpg" | "png") {
                return Err(unsupported());
            }
            if source.id == "pdf" {
                Ok(Route::PdfRaster)
            } else {
                // pptx -> png etc: print to PDF first, then rasterise. Great for slide thumbnails.
                Ok(Route::OfficeThenPdfRaster)
            }
        }
        (Image, Document) if writes_pdf(target) => {
            if source.read.needs_helper() {
                Ok(Route::DecodeThenEncode)
            } else {
                Ok(Route::ImageToPdf)
            }
        }
        (Document, Document) => {
            let pandoc_pair = pandoc_readable(source.id) && pandoc_writable(target.id);
            let office_pair = office_handles(target.id) && office_handles(source.id);
            if target.id == "pdf" {
                // LibreOffice prints real page layout; Pandoc alone has no PDF engine.
                if office_handles(source.id) {
                    Ok(Route::Office)
                } else if pandoc_readable(source.id) {
                    Ok(Route::MarkupToPdf)
                } else {
                    Err(unsupported())
                }
            } else if source.id == "pdf" && pdf_text_tool(target.id).is_some() {
                // Poppler owns "get the text out of this PDF" - see [`Route::PdfText`].
                Ok(Route::PdfText)
            } else if pandoc_pair && (tools.has(Tool::Pandoc) || !office_pair) {
                Ok(Route::Pandoc)
            } else if office_pair {
                Ok(Route::Office)
            } else {
                Err(unsupported())
            }
        }
        // ---- subtitles can be demuxed straight out of a video container
        (Video, Subtitle) => Ok(Route::Ffmpeg),
        _ => Err(unsupported()),
    }
}

/// Formats LibreOffice can open *and* save directly (so no Pandoc hop is needed).
///
/// One list, not two: the import and export filter sets were spelled out separately and were
/// identical, which is a pair of lists waiting to drift apart. Everything here round-trips -
/// `--infilter=writer_pdf_import` is what puts PDF on the read side.
fn office_handles(id: &str) -> bool {
    matches!(
        id,
        "pdf"
            | "docx"
            | "doc"
            | "odt"
            | "rtf"
            | "txt"
            | "html"
            | "pptx"
            | "ppt"
            | "odp"
            | "xlsx"
            | "xls"
            | "ods"
            | "csv"
            | "tsv"
    )
}

/// The Poppler binary that turns a PDF into this target, if one does.
///
/// `None` for every other document target: Poppler has no writer for them, so those pairs stay
/// with LibreOffice. Keyed by target id (not extension) so `Route::PdfText` and the route chooser
/// cannot disagree about which pairs Poppler owns.
fn pdf_text_tool(target_id: &str) -> Option<Tool> {
    match target_id {
        "txt" => Some(Tool::PdfToText),
        "html" => Some(Tool::PdfToHtml),
        _ => None,
    }
}

/// The two catalog entries that produce a PDF from a single image: the Document `pdf` and the
/// Image-category `pdf_page` pseudo-format the UI offers as an image target.
fn writes_pdf(target: &Format) -> bool {
    matches!(target.id, "pdf" | "pdf_page")
}

/// Choose between the FFmpeg route and the helper that owns the target format.
///
/// The catalog is the authority on who can *write* a format. FFmpeg ships no encoder for HEIC,
/// ICNS or AI, so a plan that pointed FFmpeg at one of those extensions was a command line that
/// could only ever fail - and it failed after the user had waited for the decode.
fn encoder_route(
    source: &'static Format,
    target: &'static Format,
    unsupported: impl Fn() -> PlanError,
) -> Result<Route, PlanError> {
    if !target.write.needs_helper() {
        return Ok(Route::Ffmpeg);
    }
    // The helpers read still images, not video containers: there is no way to hand them a frame
    // of an MP4, so "one frame of this video as a HEIC" has no honest route.
    if source.category != Category::Image || encoders_for(target).is_empty() {
        return Err(unsupported());
    }
    Ok(Route::HelperEncode)
}

/// Encoders `helper_encode_step` knows how to drive, in catalog order.
fn encoders_for(target: &'static Format) -> Vec<Tool> {
    target
        .write
        .helpers()
        .iter()
        .copied()
        .filter(|t| matches!(t, Tool::Sips | Tool::Magick))
        .collect()
}

/// Decoders `decode_step` knows how to drive. Anything else in a format's helper list would turn
/// into an argument list the tool does not understand, so it must not be selected.
fn decoders_for(source: &'static Format) -> Vec<Tool> {
    source
        .read
        .helpers()
        .iter()
        .copied()
        .filter(|t| matches!(t, Tool::Sips | Tool::Magick))
        .collect()
}

fn animated_image(id: &str) -> bool {
    matches!(id, "gif" | "webp" | "avif" | "apng")
}

/// Will this pairing write a numbered sequence (`clip-0001.png`, ...) instead of one file?
///
/// Public because the answer decides more than the argv: the caller has to reserve a whole family of
/// names rather than the single `clip.png` it asked for. Deriving it in two places is how the queue
/// and the planner would come to disagree about what a job writes.
pub(crate) fn writes_frame_sequence(source_is_animated: bool, target: &Format) -> bool {
    source_is_animated && target.category == Category::Image && !animated_image(target.id)
}

fn is_frame_sequence(req: &PlanRequest) -> bool {
    writes_frame_sequence(req.source_is_animated, req.target)
}

/// Does this pairing produce something that has a *duration*?
///
/// The question the batch-wide trim ([`crate::settings::TrimSettings`]) turns on, and it is about
/// the **output**: video, audio, an animated GIF/WebP made from something that moves, and a frame
/// sequence pulled out of a moving source are all cuts of a timeline. A JPEG, a PDF, a subtitle
/// file and Ruffle's still-frame dump are not, and a trim is silently ignored for them rather than
/// refused - a batch of ten clips and one PDF has to convert the PDF.
///
/// Public in the crate for the same reason as [`writes_frame_sequence`]: the queue asks it too (to
/// know whether a duration is trimmed and whether the "starts past the end" refusal applies at
/// all), and deriving the answer twice is how the two would come to disagree.
pub(crate) fn output_is_time_based(
    source: &Format,
    target: &Format,
    source_is_animated: bool,
) -> bool {
    match target.category {
        Category::Video | Category::Audio => true,
        Category::Image => {
            // Flash to stills is Ruffle's frame dump: no encoder runs at all, so there is nothing
            // to hand a trim to and nothing to measure it against.
            if source.category == Category::Flash {
                return animated_image(target.id);
            }
            if animated_image(target.id) {
                source_is_animated
            } else {
                writes_frame_sequence(source_is_animated, target)
            }
        }
        Category::Subtitle | Category::Document | Category::Flash => false,
    }
}

fn need(
    tools: &ToolRegistry,
    candidates: &[Tool],
    format: &'static str,
) -> Result<Tool, PlanError> {
    tools.first_available(candidates).ok_or_else(|| {
        let t = candidates.first().copied().unwrap_or(Tool::Ffmpeg);
        // "PDF page needs Poppler installed (brew install poppler)". The user installs a package,
        // so the name in the sentence is the package's, not `pdftoppm`'s.
        PlanError::MissingTool { format, tool: t.user_facing_name(), hint: t.install_hint() }
    })
}

fn program(tools: &ToolRegistry, tool: Tool) -> PathBuf {
    tools.path(tool).map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from(tool.id()))
}

fn rebalance(steps: &mut [Step]) {
    let w = 1.0 / steps.len() as f32;
    for s in steps.iter_mut() {
        s.weight = w;
    }
}

// ---------------------------------------------------------------------------------------- ffmpeg

fn ffmpeg_step(
    tools: &ToolRegistry,
    tool: Tool,
    input: &Path,
    output: &Path,
    req: &PlanRequest,
    glob_input: Option<String>,
) -> Result<Step, PlanError> {
    let s = &req.settings;
    let target = req.target;
    let mut a: Vec<String> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
    ];

    // The batch-wide trim, injected here and only here: every route that produces something with a
    // duration - video, audio, animated GIF/WebP, a frame sequence, Flash rendered to a movie -
    // ends up in this function, so repeating `-ss`/`-t` per route would be five chances to forget
    // one. `output_is_time_based` is what keeps it away from stills, documents and subtitles.
    //
    // `-ss` goes *before* `-i` (fast seek: FFmpeg jumps in the demuxer instead of decoding and
    // discarding the first half hour, and everything here re-encodes, so the cut lands where the
    // user asked). `-t` goes after the input, as a limit on what is written. A source shorter than
    // the trim needs no handling at all: the input simply ends first.
    let trim = req
        .settings
        .trim
        .effective()
        .filter(|_| output_is_time_based(req.source, req.target, req.source_is_animated));
    if let Some((start, _)) = trim {
        a.push("-ss".into());
        a.push(seconds_arg(start));
    }

    match &glob_input {
        Some(pattern) => {
            a.push("-framerate".into());
            a.push("30".into());
            a.push("-pattern_type".into());
            a.push("glob".into());
            a.push("-i".into());
            a.push(pattern.clone());
        }
        None => {
            // A file literally named `frame%04d.png` *is* a valid image2 sequence pattern, so
            // FFmpeg goes hunting for `frame0000.png`..`frame0004.png` and fails with "Could find
            // no file with path". Only the image2 demuxer understands this switch; gif/apng/avif/
            // ico have their own demuxers that reject the option outright - and those are exactly
            // the image formats whose content probe outscores image2, so they never take the
            // pattern path in the first place.
            if req.source.category == Category::Image
                && !matches!(req.source.id, "gif" | "apng" | "avif" | "ico")
                && has_image2_sequence_spec(&input.to_string_lossy())
            {
                a.push("-pattern_type".into());
                a.push("none".into());
            }
            a.push("-i".into());
            a.push(input.to_string_lossy().into());
        }
    }

    if let Some((_, length)) = trim {
        a.push("-t".into());
        a.push(seconds_arg(length));
    }

    // Progress on stdout, machine readable, so the UI can show a real percentage.
    a.push("-progress".into());
    a.push("pipe:1".into());
    a.push("-nostats".into());

    // Metadata is stripped per media kind, because "strip metadata" means different things:
    // images carry EXIF/GPS (own switch), audio carries the tags that make a library usable
    // (never stripped), subtitles carry nothing worth removing. Rotation is unaffected - a
    // display matrix is stream *side data*, and on re-encode FFmpeg bakes it into the pixels.
    let strip_metadata = match target.category {
        Category::Video => s.video.strip_metadata,
        Category::Image => s.image.strip_metadata,
        Category::Audio | Category::Subtitle | Category::Document | Category::Flash => false,
    };
    if strip_metadata {
        a.push("-map_metadata".into());
        a.push("-1".into());
    }

    match target.category {
        Category::Audio => {
            a.push("-vn".into());
            push_audio_args(&mut a, target.id, s);
        }
        Category::Video => {
            push_video_args(&mut a, target.id, s, req.source_is_animated);
            push_audio_args(&mut a, target.id, s);
            if s.video.faststart && matches!(target.id, "mp4" | "m4v" | "mov") {
                a.push("-movflags".into());
                a.push("+faststart".into());
            }
            // The MXF muxer only accepts broadcast frame rates (24/25/30/50/60 and their
            // 1000/1001 variants). A 15 fps screen capture or a 12 fps GIF is refused outright
            // with "Unsupported frame rate 15/1. Set -strict option to 'unofficial'".
            if target.id == "mxf" {
                a.push("-strict".into());
                a.push("unofficial".into());
            }
        }
        Category::Image => match target.id {
            "gif" => push_gif_args(&mut a, s),
            id if animated_image(id) && req.source_is_animated => {
                push_animated_image_args(&mut a, id, s)
            }
            _ => push_still_image_args(&mut a, target.id, s, req.source_is_animated),
        },
        // Subtitles are a straight text remux; documents and Flash never reach FFmpeg as a target.
        Category::Subtitle | Category::Document | Category::Flash => {}
    }

    let stem = stem_of(output);
    let ext = output.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let dir = output.parent().unwrap_or(Path::new(".")).to_path_buf();
    let (out, post) = if is_frame_sequence(req) {
        // clip.mp4 -> "clip-0001.jpg", "clip-0002.jpg", ...
        //
        // The whole path is a *printf pattern* to the image2 muxer, so every `%` the user
        // contributed has to be doubled: "100%_done.mp4" in a folder called "50% off" would
        // otherwise be an invalid pattern (one literal file, then "Cannot write more than one
        // file with the same name"), and "take%04d.mp4" would have the frame number substituted
        // into the middle of its own name.
        let escaped = escape_image2_pattern(&output.with_file_name(&stem).to_string_lossy());
        (
            format!("{escaped}-%04d.{ext}"),
            // The files land on disk with their single `%` back, so the prefix the engine
            // matches against is the real stem, not the escaped one.
            Some(PostAction::CollectSequence { dir: dir.clone(), prefix: stem, extension: ext }),
        )
    } else {
        (output.to_string_lossy().to_string(), None)
    };
    a.push(out);

    Ok(Step {
        tool,
        program: program(tools, tool),
        args: a,
        weight: 1.0,
        ffmpeg_progress: true,
        label: format!("Encode → {}", output_extension(target)),
        post,
        ensure_dirs: vec![dir],
    })
}

fn push_video_args(a: &mut Vec<String>, container: &str, s: &Settings, animated: bool) {
    let encoder = video_encoder(container, s);
    if let Some(enc) = encoder {
        a.push("-c:v".into());
        a.push(enc.into());
    }
    if encoder == Some("copy") {
        return; // a remux: filters, pixel formats and rate control are all meaningless
    }

    if let Some(enc) = encoder {
        push_video_rate_control(a, enc, s);
        if enc == "libx264" {
            // Baseline-ish compatibility: plays on old Safari, PowerPoint, Keynote, Android.
            a.push("-profile:v".into());
            a.push("high".into());
            // Level 4.1 is the compatibility sweet spot but it caps the frame size at 1080p;
            // claiming it for a larger frame produces a stream hardware decoders may refuse.
            if s.video.max_height.map(|h| h <= 1080).unwrap_or(false) {
                a.push("-level".into());
                a.push("4.1".into());
            }
        }
        // `hvc1` is a QuickTime/MP4 codec *tag*; only the ISO-BMFF muxers know what to do with it.
        if enc == "libx265" && matches!(container, "mp4" | "m4v" | "mov") {
            a.push("-tag:v".into());
            a.push("hvc1".into()); // required for QuickTime/Safari HEVC playback
        }
        a.push("-pix_fmt".into());
        a.push(if enc == "prores_ks" { "yuv422p10le".into() } else { "yuv420p".to_string() });
    }

    let mut filters: Vec<String> = Vec::new();
    if let Some(h) = s.video.max_height.map(sane_pixels) {
        filters.push(format!("scale=-2:trunc(min({h}\\,ih)/2)*2:flags=lanczos"));
    }
    push_filters(a, &filters);
    if let Some(fps) = s.video.fps_cap.map(sane_fps) {
        // Output option, after the filters and before the file: a *cap*, so unlike `-r` it never
        // duplicates frames to reach the target rate. Needs FFmpeg >= 5.0, which is what we ship.
        a.push("-fpsmax".into());
        a.push(trim_float(fps));
    }
    if animated {
        // GIF/WebP sources have a variable frame delay; force a sane constant rate.
        a.push("-fps_mode".into());
        a.push("cfr".into());
    }
}

fn push_video_rate_control(a: &mut Vec<String>, encoder: &str, s: &Settings) {
    if let Some(kbps) = s.video.bitrate_kbps {
        a.push("-b:v".into());
        a.push(format!("{kbps}k"));
        return;
    }
    match encoder {
        "h264_videotoolbox" | "hevc_videotoolbox" => {
            // VideoToolbox has no CRF; -q:v 1..100 (higher = better) is the closest equivalent.
            let q = match s.video.quality {
                QualityLevel::Small => 40,
                QualityLevel::Balanced => 55,
                QualityLevel::High => 70,
                QualityLevel::Max => 85,
            };
            a.push("-q:v".into());
            a.push(q.to_string());
        }
        "prores_ks" => {
            a.push("-profile:v".into());
            a.push("3".into()); // ProRes 422 HQ
        }
        "libvpx-vp9" => {
            a.push("-crf".into());
            a.push(s.crf_for(VideoCodec::Vp9).to_string());
            a.push("-b:v".into());
            a.push("0".into());
            a.push("-row-mt".into());
            a.push("1".into());
        }
        "libsvtav1" => {
            a.push("-crf".into());
            a.push(s.crf_for(VideoCodec::Av1).to_string());
            a.push("-preset".into());
            a.push("6".into());
        }
        enc => {
            let codec = if enc == "libx265" { VideoCodec::H265 } else { VideoCodec::H264 };
            a.push("-crf".into());
            a.push(s.crf_for(codec).to_string());
            a.push("-preset".into());
            a.push("medium".into());
        }
    }
}

/// Which codec can actually go into this container.
///
/// FFmpeg *refuses* (it does not warn) when a stream cannot be muxed: HEVC in WebM/FLV/3GP/ASF,
/// ProRes in MP4, VP9 in MOV, anything but Theora in Ogg. Falling back to the container's native
/// codec is always better than emitting a command line that dies on the user.
///
/// `None` means "single-codec container - let the muxer pick": Ogg/Theora, raw YUV4MPEG, MJPEG and
/// raw H.264 elementary streams accept exactly one thing, and it is the muxer's own default.
fn resolve_video_codec(container: &str, requested: VideoCodec) -> Option<VideoCodec> {
    use VideoCodec::*;
    if requested == Copy {
        return Some(Copy); // an explicit remux; FFmpeg validates the stream against the container
    }
    Some(match container {
        "ogv" | "y4m" | "mjpeg" | "h264" => return None,
        "webm" => match requested {
            Vp9 | Av1 => requested,
            _ => Vp9,
        },
        "mp4" | "m4v" => match requested {
            Auto | ProRes => H264,
            other => other,
        },
        // QuickTime carries ProRes but not VP9/AV1.
        "mov" => match requested {
            Auto | Vp9 | Av1 => H264,
            other => other,
        },
        "flv" | "f4v" | "3gp" | "wmv" | "asf" => H264,
        _ => match requested {
            Auto => H264,
            other => other,
        },
    })
}

/// FFmpeg encoder name for a container + settings pair, or `None` for "let the muxer decide".
/// Single source of truth for both the command line and the human summary.
fn video_encoder(container: &str, s: &Settings) -> Option<&'static str> {
    let codec = resolve_video_codec(container, s.video.codec)?;
    let hw = matches!(s.video.hardware_accel, HardwareAccel::Auto)
        && matches!(codec, VideoCodec::H264 | VideoCodec::H265)
        && cfg!(target_os = "macos");
    Some(match (codec, hw) {
        (VideoCodec::H264, true) => "h264_videotoolbox",
        (VideoCodec::H264, false) => "libx264",
        (VideoCodec::H265, true) => "hevc_videotoolbox",
        (VideoCodec::H265, false) => "libx265",
        (VideoCodec::Vp9, _) => "libvpx-vp9",
        (VideoCodec::Av1, _) => "libsvtav1",
        (VideoCodec::ProRes, _) => "prores_ks",
        (VideoCodec::Copy, _) => "copy",
        // `resolve_video_codec` maps Auto onto a real codec; deferring to the muxer is the safe
        // answer if that ever stops being true.
        (VideoCodec::Auto, _) => return None,
    })
}

/// Human name for what `video_encoder` chose, used by the one-line job summary.
fn encoder_label(encoder: Option<&str>) -> &'static str {
    match encoder {
        Some("libx264" | "h264_videotoolbox") => "H.264",
        Some("libx265" | "hevc_videotoolbox") => "H.265",
        Some("libvpx-vp9") => "VP9",
        Some("libsvtav1") => "AV1",
        Some("prores_ks") => "ProRes 422 HQ",
        Some("copy") => "stream copy",
        _ => "container default",
    }
}

/// Which audio codec can actually go into this container.
///
/// Muxers *reject* a stream they cannot carry, they do not transcode it away: FLAC into MP3 or
/// M4A, MP3 into M4A and AAC into CAF are all hard errors. Since the "Lossless / archive" preset
/// asks for FLAC for every target, honouring the request blindly made that preset fail on the two
/// most common audio outputs. `None` means "one codec per container - the muxer already knows".
fn resolve_audio_codec(container: &str, requested: AudioCodec) -> Option<AudioCodec> {
    use AudioCodec::*;
    if requested == Copy {
        return Some(Copy); // an explicit remux; FFmpeg validates the stream against the container
    }
    Some(match container {
        "mp3" => Mp3,
        "aac" => Aac,
        "flac" => Flac,
        "opus" => Opus,
        "alac" => Alac,
        "wav" | "w64" | "aiff" => PcmWav,
        // MP4 proper is liberal (AAC/MP3/Opus/Vorbis/ALAC) but has no tag for FLAC or PCM...
        "mp4" => match requested {
            Auto | Flac | PcmWav => Aac,
            other => other,
        },
        // ...while `.m4v`/`.m4a` go through the stricter iPod muxer: AAC or ALAC only.
        "m4v" | "m4a" => match requested {
            Alac => Alac,
            _ => Aac,
        },
        "mov" => match requested {
            Aac | Alac | Mp3 | PcmWav => requested,
            _ => Aac,
        },
        "3gp" => Aac,
        "flv" | "f4v" => match requested {
            Mp3 | Aac => requested,
            _ => Aac,
        },
        "avi" => match requested {
            Mp3 | Aac | Vorbis | Flac | PcmWav => requested,
            _ => Mp3,
        },
        // Core Audio carries PCM and ALAC; its muxer refuses AAC.
        "caf" => match requested {
            Alac | PcmWav => requested,
            _ => PcmWav,
        },
        "webm" => match requested {
            Opus | Vorbis => requested,
            _ => Opus,
        },
        "ogg" | "ogv" => match requested {
            Opus | Vorbis | Flac => requested,
            _ => Vorbis,
        },
        // Matroska/TS/NUT carry anything; only `Auto` needs an opinion.
        "mkv" | "mka" | "ts" | "nut" => match requested {
            Auto => Aac,
            other => other,
        },
        // ac3, eac3, dts, wma, mp2, wv, tta, au, voc, ...: single-codec containers.
        _ => return None,
    })
}

/// Containers whose muxer-default encoder ignores or *rejects* `-b:a`: the lossless codecs
/// (WavPack, True Audio, AU/VOC PCM) error out instead of quietly ignoring the request.
///
/// A table rather than a `match` arm so a test can check every entry is still a format the app
/// offers as a target - `gsm`, `amr` and `8svx` sat here long after they stopped being writable.
const NO_BITRATE_CONTAINERS: &[&str] = &["wv", "tta", "au", "voc"];

/// Containers whose muxer implements exactly one audio sample rate, whatever the source had.
///
/// FFmpeg's MXF muxer implements 48 kHz audio *only*, so a plain 44.1 kHz source - i.e. most
/// files - cannot be written at all unless the rate is forced here.
const FIXED_SAMPLE_RATE_CONTAINERS: &[(&str, u32)] = &[("mxf", 48000)];

fn container_default_rejects_bitrate(container: &str) -> bool {
    NO_BITRATE_CONTAINERS.contains(&container)
}

fn container_fixed_sample_rate(container: &str) -> Option<u32> {
    FIXED_SAMPLE_RATE_CONTAINERS.iter().find(|(id, _)| *id == container).map(|(_, hz)| *hz)
}

fn push_audio_args(a: &mut Vec<String>, container: &str, s: &Settings) {
    let codec = resolve_audio_codec(container, s.audio.codec);

    let encoder = match codec {
        Some(AudioCodec::Mp3) => Some("libmp3lame"),
        Some(AudioCodec::Aac) => Some("aac"),
        Some(AudioCodec::Opus) => Some("libopus"),
        Some(AudioCodec::Vorbis) => Some("libvorbis"),
        Some(AudioCodec::Flac) => Some("flac"),
        Some(AudioCodec::Alac) => Some("alac"),
        Some(AudioCodec::PcmWav) => Some("pcm_s16le"),
        Some(AudioCodec::Copy) => Some("copy"),
        // `resolve_audio_codec` never returns `Auto`; both it and `None` mean "muxer default".
        Some(AudioCodec::Auto) | None => None,
    };
    if let Some(enc) = encoder {
        a.push("-c:a".into());
        a.push(enc.into());
    }
    // FFmpeg's DTS encoder is flagged experimental and refuses to start without this.
    if container == "dts" {
        a.push("-strict".into());
        a.push("-2".into());
    }

    let wants_bitrate = match codec {
        Some(c) => !matches!(
            c,
            AudioCodec::Flac | AudioCodec::Alac | AudioCodec::PcmWav | AudioCodec::Copy
        ),
        None => !container_default_rejects_bitrate(container),
    };
    if wants_bitrate {
        a.push("-b:a".into());
        a.push(format!("{}k", sane_bitrate(s.audio.bitrate_kbps)));
    }

    // Some containers implement exactly one sample rate, so the container overrules the user here
    // rather than producing a command line that cannot run.
    let sample_rate = container_fixed_sample_rate(container)
        .or_else(|| s.audio.sample_rate.map(sane_sample_rate));
    let channels = s.audio.channels.map(sane_channels);
    if let Some(rate) = sample_rate {
        a.push("-ar".into());
        a.push(rate.to_string());
    }
    if let Some(ch) = channels {
        a.push("-ac".into());
        a.push(ch.to_string());
    }
    if s.audio.normalize_loudness {
        a.push("-af".into());
        a.push("loudnorm=I=-16:TP=-1.5:LRA=11".into());
    }
    if container == "mp3" {
        a.push("-id3v2_version".into());
        a.push("3".into());
    }
}

fn push_gif_args(a: &mut Vec<String>, s: &Settings) {
    let g = &s.gif;
    let scale = format!("scale=min({w}\\,iw):-2:flags=lanczos", w = sane_pixels(g.width));
    let fps = trim_float(sane_fps(g.fps));
    let chain = if g.optimize_palette {
        format!(
            "fps={fps},{scale},split[a][b];[a]palettegen=stats_mode=diff[p];\
             [b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle"
        )
    } else {
        format!("fps={fps},{scale}")
    };
    a.push("-filter_complex".into());
    a.push(chain);
    a.push("-loop".into());
    a.push(g.loop_count.to_string());
}

fn push_animated_image_args(a: &mut Vec<String>, target: &str, s: &Settings) {
    let g = &s.gif; // animation knobs are shared with GIF
    a.push("-vf".into());
    a.push(format!(
        "fps={fps},scale=min({w}\\,iw):-2:flags=lanczos",
        fps = trim_float(sane_fps(g.fps)),
        w = sane_pixels(g.width)
    ));
    match target {
        "webp" => {
            a.push("-c:v".into());
            a.push("libwebp_anim".into());
            a.push("-lossless".into());
            a.push(if s.image.lossless { "1".into() } else { "0".into() });
            a.push("-q:v".into());
            a.push(s.image.quality.to_string());
            a.push("-loop".into());
            a.push(g.loop_count.to_string());
        }
        "apng" => {
            a.push("-f".into());
            a.push("apng".into());
            a.push("-plays".into());
            a.push(g.loop_count.to_string());
        }
        "avif" => {
            a.push("-c:v".into());
            a.push("libaom-av1".into());
            a.push("-crf".into());
            a.push(quality_to_crf(s.image.quality).to_string());
            a.push("-cpu-used".into());
            a.push("6".into());
        }
        _ => {}
    }
}

fn push_still_image_args(a: &mut Vec<String>, target: &str, s: &Settings, animated_source: bool) {
    let mut filters: Vec<String> = Vec::new();

    if animated_source {
        // A video/animation asked for a still format -> sample a frame every N seconds.
        filters.push(format!("fps={}", trim_float(sane_fps(s.image.frame_extract_fps))));
    } else {
        a.push("-frames:v".into());
        a.push("1".into());
        a.push("-update".into());
        a.push("1".into());
    }

    if target == "ico" {
        // Both edges must end up <= 256 or the ICO muxer refuses the frame outright, so this
        // cannot be a width-only scale: a 240x400 portrait would stay 400 tall and fail.
        filters.push("scale=w=min(iw\\,256):h=min(ih\\,256):force_original_aspect_ratio=decrease:flags=lanczos".into());
    } else if let Some(max) = s.image.max_dimension.map(sane_pixels) {
        filters.push(format!(
            "scale=w=min(iw\\,{max}):h=min(ih\\,{max}):force_original_aspect_ratio=decrease:flags=lanczos"
        ));
    }
    if let Some(flat) = flatten_pixel_format(target) {
        // These formats have no alpha channel. Dropping it naively turns transparent pixels black,
        // so composite the image over the user's background colour first.
        let bg = hex_background(&s.image.flatten_background);
        filters.push(format!(
            "format=rgba,split[fg][bgsrc];[bgsrc]drawbox=c=0x{bg}ff:t=fill[bg];\
             [bg][fg]overlay=format=auto,format={flat}"
        ));
    }
    push_filters(a, &filters);

    match target {
        "jpg" => {
            a.push("-q:v".into());
            a.push(jpeg_quality_scale(s.image.quality).to_string());
        }
        "webp" => {
            a.push("-c:v".into());
            a.push("libwebp".into());
            a.push("-lossless".into());
            a.push(if s.image.lossless { "1".into() } else { "0".into() });
            a.push("-q:v".into());
            a.push(s.image.quality.to_string());
        }
        "avif" => {
            a.push("-c:v".into());
            a.push("libaom-av1".into());
            a.push("-still-picture".into());
            a.push("1".into());
            a.push("-crf".into());
            a.push(quality_to_crf(s.image.quality).to_string());
            a.push("-cpu-used".into());
            a.push("6".into());
        }
        "png" | "apng" => {
            a.push("-compression_level".into());
            a.push("9".into());
        }
        "tiff" => {
            a.push("-compression_algo".into());
            a.push("deflate".into());
        }
        _ => {}
    }
}

/// Pixel format the alpha-flatten graph must end on, for the formats that have no alpha channel.
///
/// It has to be one the target's encoder actually accepts. `yuvj420p` is right for JPEG (full
/// range, which is what every JPEG decoder expects) but wrong for the lossless RGB formats:
/// forcing BMP/PCX through 4:2:0 chroma subsampling threw away colour detail for no reason
/// (measured ~34 dB PSNR against a lossless round trip), and the monochrome WBMP encoder cannot
/// take a YUV frame at all. `None` = the format keeps its alpha, nothing to flatten.
fn flatten_pixel_format(target: &str) -> Option<&'static str> {
    match target {
        "jpg" => Some("yuvj420p"),
        "bmp" => Some("bgr24"),
        "pcx" => Some("rgb24"),
        "wbmp" => Some("monob"),
        _ => None,
    }
}

/// FFmpeg's mjpeg encoder uses `-q:v 2..31` (lower = better); map our 1..100 scale onto it.
fn jpeg_quality_scale(quality: u8) -> u32 {
    let q = quality.clamp(1, 100) as f32;
    (2.0 + (100.0 - q) / 100.0 * 29.0).round().clamp(2.0, 31.0) as u32
}

// -------------------------------------------------------------------------- user input guards
//
// Settings arrive as JSON from the frontend (and from a settings file a user can edit), so every
// number below is untrusted: a zero width, a negative frame rate or an infinity would build a
// filter graph FFmpeg rejects outright, and the job would fail with a wall of ffmpeg text instead
// of doing something sensible. Clamping happens here, at the point the value becomes a command
// line argument, so no caller can forget it.

/// Frame rates. FFmpeg rejects `fps=0` and anything negative; non-finite values cannot even be
/// formatted into a filter.
fn sane_fps(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.05, 1000.0)
    } else {
        30.0
    }
}

/// Pixel dimensions. Two is the smallest even size the scalers accept; 16384 is FFmpeg's own limit.
fn sane_pixels(v: u32) -> u32 {
    v.clamp(2, 16384)
}

/// `-b:a 0k` is an error, and no audio encoder here goes above a few thousand kbps.
fn sane_bitrate(kbps: u32) -> u32 {
    kbps.clamp(6, 5000)
}

fn sane_sample_rate(hz: u32) -> u32 {
    hz.clamp(4000, 384_000)
}

fn sane_channels(ch: u8) -> u8 {
    ch.clamp(1, 8)
}

/// Rasteriser resolution: `pdftoppm -r 0` and `magick -density 0` both fail.
fn sane_dpi(dpi: u32) -> u32 {
    dpi.clamp(9, 1200)
}

/// The flatten colour goes straight into `drawbox=c=0x…`, which FFmpeg parses - so an arbitrary
/// string from the settings file could break the graph or smuggle another filter into it. Only
/// `#rgb` / `#rrggbb` is accepted; anything else falls back to the default white.
fn hex_background(raw: &str) -> String {
    let hex = raw.trim().trim_start_matches('#');
    let hexish = hex.chars().all(|c| c.is_ascii_hexdigit());
    match hex.len() {
        6 if hexish => hex.to_ascii_lowercase(),
        3 if hexish => hex.chars().flat_map(|c| [c, c]).collect::<String>().to_ascii_lowercase(),
        _ => "ffffff".into(),
    }
}

/// AV1/AVIF still images: 1..100 quality -> 63..0 CRF.
fn quality_to_crf(quality: u8) -> u32 {
    let q = quality.clamp(1, 100) as f32;
    ((100.0 - q) * 0.63).round() as u32
}

fn trim_float(v: f32) -> String {
    if (v - v.round()).abs() < f32::EPSILON {
        format!("{}", v.round() as i64)
    } else {
        format!("{v}")
    }
}

/// A number of seconds as FFmpeg wants to read it (`-ss 30`, `-t 10.5`).
///
/// Rounded to milliseconds first: the value came from a text field via `f64`, and `format!("{v}")`
/// is happy to put `9.999999999999998` on a command line for a number the user typed as `10`.
/// Callers pass values that [`crate::settings::TrimSettings::effective`] has already made finite and
/// bounded, so there is no NaN or exponent to print here.
fn seconds_arg(v: f64) -> String {
    let rounded = (v * 1000.0).round() / 1000.0;
    if (rounded - rounded.round()).abs() < 1e-9 {
        format!("{}", rounded.round() as i64)
    } else {
        format!("{rounded}")
    }
}

/// Push a filter chain, choosing `-vf` for simple chains and `-filter_complex` for graphs that use
/// labels (`split`/`overlay`), which `-vf` cannot express.
fn push_filters(a: &mut Vec<String>, filters: &[String]) {
    if filters.is_empty() {
        return;
    }
    let chain = filters.join(",");
    if chain.contains(';') || chain.contains('[') {
        a.push("-filter_complex".into());
    } else {
        a.push("-vf".into());
    }
    a.push(chain);
}

// ------------------------------------------------------------------------------------- helpers

/// Double every `%` so FFmpeg's image2 muxer writes the name the user actually chose.
fn escape_image2_pattern(path: &str) -> String {
    path.replace('%', "%%")
}

/// Does FFmpeg read this path as an image2 *sequence pattern* rather than a single file?
///
/// A faithful port of the validity rules in `av_get_frame_filename2`, which is what
/// `av_filename_number_test` - and therefore image2's filename probe - is built on: `%`, optional
/// digits, then `d`, at most once. `%%` is an escaped percent sign, and any other conversion makes
/// the whole pattern invalid, at which point FFmpeg treats the name as one plain file. Those cases
/// must answer `false`: they work today and adding `-pattern_type none` would break them, because
/// the option only exists on the image2 demuxer.
pub(crate) fn has_image2_sequence_spec(path: &str) -> bool {
    let mut chars = path.chars().peekable();
    let mut found = false;
    while let Some(c) = chars.next() {
        if c != '%' {
            continue;
        }
        while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
            chars.next();
        }
        match chars.next() {
            Some('%') => continue, // an escaped percent sign, not a conversion
            Some('d') if !found => found = true,
            // A second `%d`, an unknown conversion, or a trailing `%`: not a valid pattern.
            _ => return false,
        }
    }
    found
}

/// Directories that must exist before a step can write these files.
fn parent_dirs(files: &[&Path]) -> Vec<PathBuf> {
    files.iter().filter_map(|f| f.parent()).map(|p| p.to_path_buf()).collect()
}

fn decode_step(tools: &ToolRegistry, tool: Tool, input: &Path, out: &Path, s: &Settings) -> Step {
    let args: Vec<String> = match tool {
        Tool::Sips => vec![
            "-s".into(),
            "format".into(),
            "png".into(),
            input.to_string_lossy().into(),
            "--out".into(),
            out.to_string_lossy().into(),
        ],
        _ => vec![
            "-density".into(),
            sane_dpi(s.document.raster_dpi).to_string(),
            "-background".into(),
            "none".into(),
            format!("{}[0]", input.to_string_lossy()),
            out.to_string_lossy().into(),
        ],
    };
    Step {
        tool,
        program: program(tools, tool),
        args,
        weight: 0.5,
        ffmpeg_progress: false,
        label: "Decode".into(),
        post: None,
        ensure_dirs: parent_dirs(&[out]),
    }
}

/// Encode a still image with `sips`/`magick` for the formats FFmpeg cannot write.
fn helper_encode_step(
    tools: &ToolRegistry,
    tool: Tool,
    input: &Path,
    output: &Path,
    target: &'static Format,
    s: &Settings,
) -> Step {
    let args: Vec<String> = match tool {
        Tool::Sips => vec![
            "-s".into(),
            "format".into(),
            sips_format(target).into(),
            input.to_string_lossy().into(),
            "--out".into(),
            output.to_string_lossy().into(),
        ],
        _ => vec![
            input.to_string_lossy().into(),
            "-quality".into(),
            s.image.quality.to_string(),
            output.to_string_lossy().into(),
        ],
    };
    Step {
        tool,
        program: program(tools, tool),
        args,
        weight: 1.0,
        ffmpeg_progress: false,
        label: format!("Encode → {}", output_extension(target)),
        post: None,
        ensure_dirs: parent_dirs(&[output]),
    }
}

/// `sips -s format` names: mostly the extension, with JPEG spelled out.
fn sips_format(target: &'static Format) -> &'static str {
    match output_extension(target) {
        "jpg" => "jpeg",
        "tiff" => "tiff",
        other => other,
    }
}

fn image_to_pdf_step(
    tools: &ToolRegistry,
    tool: Tool,
    input: &Path,
    output: &Path,
    s: &Settings,
) -> Step {
    Step {
        tool,
        program: program(tools, tool),
        args: vec![
            input.to_string_lossy().into(),
            "-quality".into(),
            s.image.quality.to_string(),
            "-compress".into(),
            "jpeg".into(),
            output.to_string_lossy().into(),
        ],
        weight: 1.0,
        ffmpeg_progress: false,
        label: "Write PDF".into(),
        post: None,
        ensure_dirs: parent_dirs(&[output]),
    }
}

/// Says what the rasteriser will really produce. `sips` has no page selection at all, so with only
/// `sips` installed a multi-page PDF yields page one - the row should say so rather than let the
/// user discover it by counting files.
fn raster_summary(tool: Tool, req: &PlanRequest) -> String {
    let dpi = sane_dpi(req.settings.document.raster_dpi);
    let ext = output_extension(req.target);
    if tool == Tool::Sips && !req.settings.document.first_page_only {
        format!("{dpi} DPI {ext} · first page only (sips)")
    } else {
        format!("{dpi} DPI {ext}")
    }
}

fn pdf_raster_step(tools: &ToolRegistry, tool: Tool, req: &PlanRequest, input: &Path) -> Step {
    let dpi = sane_dpi(req.settings.document.raster_dpi);
    // `jpg` and `png` are the only raster targets `route_for` sends here, so one flag decides
    // every tool-specific spelling of "JPEG" instead of repeating the ternary in three branches.
    let jpeg = output_extension(req.target) == "jpg";
    let ext = if jpeg { "jpg" } else { "png" };
    let stem = stem_of(&req.output);
    let dir = req.output.parent().unwrap_or(Path::new(".")).to_path_buf();
    let all_pages = !req.settings.document.first_page_only;
    // Both rasterisers name multi-page output `<stem>-<n>.<ext>` next to the requested file.
    let collect = PostAction::CollectSequence {
        dir: dir.clone(),
        prefix: stem.clone(),
        extension: ext.into(),
    };

    let (args, post): (Vec<String>, Option<PostAction>) = match tool {
        Tool::PdfToPpm => {
            let mut a: Vec<String> = vec!["-r".into(), dpi.to_string()];
            if !all_pages {
                a.push("-f".into());
                a.push("1".into());
                a.push("-l".into());
                a.push("1".into());
            }
            a.push(if jpeg { "-jpeg".into() } else { "-png".to_string() });
            a.push(input.to_string_lossy().into());
            // pdftoppm appends `-<page>.<ext>` to the prefix itself.
            a.push(dir.join(&stem).to_string_lossy().into());
            (a, Some(collect))
        }
        // sips has no page selection: it renders page one to the exact path we ask for, so there
        // is never a sequence to collect.
        Tool::Sips => (
            vec![
                "-s".into(),
                "format".into(),
                if jpeg { "jpeg".into() } else { "png".to_string() },
                input.to_string_lossy().into(),
                "--out".into(),
                req.output.to_string_lossy().into(),
            ],
            None,
        ),
        _ => {
            let spec = if all_pages {
                input.to_string_lossy().to_string()
            } else {
                format!("{}[0]", input.to_string_lossy())
            };
            (
                vec![
                    "-density".into(),
                    dpi.to_string(),
                    spec,
                    "-quality".into(),
                    req.settings.image.quality.to_string(),
                    req.output.to_string_lossy().into(),
                ],
                // ImageMagick writes `<stem>.<ext>` for a one-page PDF but `<stem>-0.<ext>`,
                // `<stem>-1.<ext>`, ... as soon as there is more than one page.
                all_pages.then_some(collect),
            )
        }
    };

    Step {
        tool,
        program: program(tools, tool),
        args,
        weight: 1.0,
        ffmpeg_progress: false,
        label: "Rasterise pages".into(),
        post,
        ensure_dirs: vec![dir],
    }
}

/// Where LibreOffice will drop its result: it always writes `<input stem>.<ext>` into `--outdir`.
///
/// One function because two routes need it - the mover in [`office_step`] and the
/// `OfficeThenPdfRaster` route, which keeps the intermediate PDF and rasterises it in place. When
/// they were computed separately, any change to the naming silently sent the rasteriser at a file
/// that was not there.
fn office_product(req: &PlanRequest, input: &Path, ext: &str) -> PathBuf {
    req.temp_dir.join("office").join(format!("{}.{ext}", stem_of(input)))
}

/// One LibreOffice step straight from the source to the target - the [`Route::Office`] plan, and
/// what [`Route::PdfText`] falls back to when no Poppler is installed. One function so the two
/// cannot drift into two different summaries for the same command line.
fn office_plan(tools: &ToolRegistry, soffice: Tool, req: &PlanRequest) -> Plan {
    let ext = output_extension(req.target);
    Plan {
        midi: None,
        summary: format!("LibreOffice → {ext}"),
        output: req.output.clone(),
        temp_paths: vec![],
        steps: vec![office_step(tools, soffice, &req.input, &req.output, req, ext)],
    }
}

/// Read a PDF's own text out with Poppler, straight to the file the user asked for.
///
/// `pdftotext -layout` keeps columns and tables where the page had them (without it, a two-column
/// paper interleaves its lines). `pdftohtml` defaults to a *frameset* plus one `page<n>.html` and a
/// `.png` per image, which is a folder nobody can email: `-s` puts every page in one document,
/// `-noframes` drops the frameset, and `-dataurls` inlines the images, so the result is a single
/// self-contained file. Both take `-enc UTF-8`; the default is Latin-1, which mangles every
/// accented character and every dash a word processor produced.
fn pdf_text_step(tools: &ToolRegistry, tool: Tool, input: &Path, output: &Path) -> Step {
    let mut args: Vec<String> = match tool {
        Tool::PdfToHtml => {
            vec!["-s".into(), "-noframes".into(), "-dataurls".into()]
        }
        _ => vec!["-layout".into()],
    };
    args.push("-enc".into());
    args.push("UTF-8".into());
    args.push(input.to_string_lossy().into());
    args.push(output.to_string_lossy().into());
    Step {
        tool,
        program: program(tools, tool),
        args,
        weight: 1.0,
        ffmpeg_progress: false,
        label: format!("Extract → {}", file_name_of(output)),
        post: None,
        ensure_dirs: parent_dirs(&[output]),
    }
}

/// The private LibreOffice user profile this job runs with.
///
/// `soffice --headless` otherwise uses the one shared profile under `~/Library/Application
/// Support/LibreOffice/4`, and that profile is locked by whichever instance got there first: the
/// second simultaneous instance refuses to start, exits silently, or hangs waiting for the lock.
/// In a batch of ten documents - the whole premise of this app - that shows up as a handful of
/// rows failing for no reason the user can see. `-env:UserInstallation` gives each invocation a
/// profile of its own; putting it inside the job's existing scratch directory means the queue's
/// `remove_dir_all(temp_dir)` already takes it away afterwards, and the per-job uniqueness the
/// scratch directory guarantees (see `queue::temp_dir_for`) is exactly the uniqueness needed here.
fn office_profile_dir(req: &PlanRequest) -> PathBuf {
    req.temp_dir.join("lo-profile")
}

/// `-env:UserInstallation` takes a *URL*, not a path: a bare path is read as a relative one and
/// LibreOffice quietly falls back to the shared profile, which is the bug this is here to fix.
/// Everything outside the unreserved set is percent-encoded, because user temp directories can
/// contain spaces and `#` (`file:///a/b#c` would otherwise be parsed as a fragment).
fn file_url(path: &Path) -> String {
    let absolute = std::path::absolute(path).expect("conversion scratch path is absolute");
    url::Url::from_file_path(dunce::simplified(&absolute))
        .expect("conversion scratch path is a filesystem path")
        .into()
}

fn office_step(
    tools: &ToolRegistry,
    tool: Tool,
    input: &Path,
    output: &Path,
    req: &PlanRequest,
    to_ext: &str,
) -> Step {
    let ext = to_ext;
    let outdir = req.temp_dir.join("office");
    let profile = office_profile_dir(req);
    let produced = office_product(req, input, ext);
    // The profile switch goes first: it is a bootstrap variable, read before LibreOffice looks at
    // anything else on the command line.
    let mut args: Vec<String> = vec![
        format!("-env:UserInstallation={}", file_url(&profile)),
        "--headless".into(),
        "--norestore".into(),
        "--invisible".into(),
        "--nolockcheck".into(),
    ];
    if req.source.id == "pdf" {
        args.push("--infilter=writer_pdf_import".into());
    }
    args.push("--convert-to".into());
    args.push(office_filter(ext, input));
    args.push("--outdir".into());
    args.push(outdir.to_string_lossy().into());
    args.push(input.to_string_lossy().into());

    Step {
        tool,
        program: program(tools, tool),
        args,
        weight: 1.0,
        ffmpeg_progress: false,
        label: format!("LibreOffice → {ext} ({})", file_name_of(output)),
        post: Some(PostAction::MoveFrom(produced)),
        ensure_dirs: vec![outdir, profile],
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// File name without the extension, lossily - the catalog is ASCII but user files are not, and a
/// name that is not valid UTF-8 still has to come out the other side as *something*.
fn stem_of(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// Explicit LibreOffice export filters where the default guess is wrong or lossy.
///
/// PDF is the one target whose filter depends on the *source*, so the file being converted is
/// passed in - see [`office_pdf_filter`].
fn office_filter(ext: &str, input: &Path) -> String {
    match ext {
        "pdf" => format!("pdf:{}", office_pdf_filter(input)),
        "docx" => "docx:MS Word 2007 XML".into(),
        "xlsx" => "xlsx:Calc MS Excel 2007 XML".into(),
        "pptx" => "pptx:Impress MS PowerPoint 2007 XML".into(),
        "csv" => "csv:Text - txt - csv (StarCalc)".into(),
        "txt" => "txt:Text (encoded):UTF8".into(),
        "html" => "html:HTML (StarWriter)".into(),
        other => other.to_string(),
    }
}

/// The PDF export filter for the application that will open this file.
///
/// There is no one "export to PDF" filter in LibreOffice: each application registers its own, and
/// naming another one's ("Calc MS Excel..." for a spreadsheet asked to print through
/// `writer_pdf_Export`) is refused with *no export filter for ...* - a failed row with a message
/// no user can act on. Keyed off the file that is actually opened rather than `req.source`, which
/// is the original document: the Markdown route hands LibreOffice an intermediate `.html`, and
/// that is a Writer document however the job started.
fn office_pdf_filter(input: &Path) -> &'static str {
    match crate::format::by_path(input).map(|f| f.id) {
        Some("xlsx" | "xls" | "ods" | "csv" | "tsv") => "calc_pdf_Export",
        Some("pptx" | "ppt" | "odp") => "impress_pdf_Export",
        // Writer opens everything else this app hands to LibreOffice (doc/docx/odt/rtf/txt/html,
        // and PDF itself through `writer_pdf_import`), and is the right guess for an extension the
        // catalog does not know.
        _ => "writer_pdf_Export",
    }
}

fn pandoc_step(tools: &ToolRegistry, tool: Tool, input: &Path, output: &Path) -> Step {
    let mut args: Vec<String> = vec!["--standalone".into()];
    if output.extension().and_then(|e| e.to_str()) == Some("html") {
        // Embedding images and CSS in the HTML is spelled two different ways, and picking one blind
        // breaks half the installed base: `--embed-resources` does not exist before Pandoc 2.19
        // (2.17 exits 6 with "Unknown option"), while `--self-contained` is deprecated from 2.19
        // onwards - it still works in 3.x, with a warning, but a deprecated flag is a flag that
        // will be removed. Pandoc is a user-installed helper of any age, so discovery asks it which
        // one it is. An unknown version falls back to the older spelling: that is the one verified
        // to work on every release from 2.17 to 3.8, so guessing it can only cost a warning,
        // whereas guessing the new one costs the conversion.
        let embed = tools.version(tool).is_some_and(|v| v.at_least(2, 19));
        args.push(if embed { "--embed-resources".into() } else { "--self-contained".into() });
        args.push("--metadata".into());
        args.push(format!("title={}", stem_of(input)));
    }
    args.push("-o".into());
    args.push(output.to_string_lossy().into());
    args.push(input.to_string_lossy().into());
    Step {
        tool,
        program: program(tools, tool),
        args,
        weight: 1.0,
        ffmpeg_progress: false,
        label: "Pandoc".into(),
        post: None,
        ensure_dirs: parent_dirs(&[output]),
    }
}

fn flash_export_step(tools: &ToolRegistry, tool: Tool, input: &Path, frames_dir: &Path) -> Step {
    Step {
        tool,
        program: program(tools, tool),
        args: vec![
            input.to_string_lossy().into(),
            frames_dir.to_string_lossy().into(),
            "--frames".into(),
            "900".into(),
            "--scale".into(),
            "2".into(),
        ],
        weight: 0.6,
        ffmpeg_progress: false,
        label: "Render Flash frames".into(),
        post: None,
        // Ruffle writes into this directory; it does not create it.
        ensure_dirs: vec![frames_dir.to_path_buf()],
    }
}

fn summarize(req: &PlanRequest) -> String {
    let s = &req.settings;
    match req.target.category {
        Category::Video => {
            let codec = encoder_label(video_encoder(req.target.id, s));
            let res =
                s.video.max_height.map(|h| format!("{h}p")).unwrap_or_else(|| "source".into());
            format!("{codec} · {res} · audio {}k", s.audio.bitrate_kbps)
        }
        Category::Audio => format!("{} · {} kbps", req.target.name, s.audio.bitrate_kbps),
        Category::Image if req.target.id == "gif" => {
            format!("{} px · {} fps", s.gif.width, trim_float(s.gif.fps))
        }
        Category::Image => {
            let dim = s
                .image
                .max_dimension
                .map(|d| format!("max {d} px"))
                .unwrap_or_else(|| "original size".into());
            format!("quality {} · {dim}", s.image.quality)
        }
        _ => req.target.name.to_string(),
    }
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
