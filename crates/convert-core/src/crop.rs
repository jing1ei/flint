//! Batch-only numeric selections. Preparations write only into the job's scratch directory.
use crate::{
    engine::{Engine, EngineError, ProgressUpdate},
    format::{by_id, Category, Tool},
    plan::{has_image2_sequence_spec, plan, Plan, PlanRequest, Step},
    settings::{AudioCodec, Settings, TrimSettings, VideoCodec},
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CropSettings {
    pub image: Option<ImageCrop>,
    pub media: Option<MediaCrop>,
    pub document: Option<DocumentCrop>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageCrop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaCrop {
    pub start_secs: f64,
    pub length_secs: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentUnit {
    Pages,
    Words,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentCrop {
    pub unit: DocumentUnit,
    /// One-based inclusive bounds.
    pub start: u32,
    pub end: u32,
}

impl CropSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.image.is_none() && self.media.is_none() && self.document.is_none() {
            return Err("Choose at least one crop range.".into());
        }
        if let Some(c) = self.image {
            if c.width == 0
                || c.height == 0
                || [c.x, c.y, c.width, c.height].iter().any(|n| *n > 100_000)
            {
                return Err(
                    "Image width and height must be 1-100000 px; X and Y must be 0-100000 px."
                        .into(),
                );
            }
        }
        if let Some(c) = self.media {
            if !c.start_secs.is_finite()
                || !c.length_secs.is_finite()
                || c.start_secs < 0.0
                || c.length_secs <= 0.0
                || c.start_secs + c.length_secs > 86_400.0
            {
                return Err("Media range must have positive length and end within 24 hours.".into());
            }
        }
        if let Some(c) = self.document {
            if c.start == 0 || c.end < c.start || c.end > 10_000_000 {
                return Err("Document ranges are inclusive: start at 1, end at or after start (maximum 10000000).".into());
            }
        }
        Ok(())
    }

    pub fn applies_to(&self, category: Category) -> bool {
        match category {
            Category::Image => self.image.is_some(),
            Category::Video | Category::Audio | Category::Flash => self.media.is_some(),
            Category::Document => self.document.is_some(),
            Category::Subtitle => false,
        }
    }
}

fn refused(message: impl Into<String>) -> EngineError {
    EngineError::Selection(message.into())
}

fn check_cancel(cancel: &Arc<AtomicBool>) -> Result<(), EngineError> {
    if cancel.load(Ordering::Relaxed) {
        Err(EngineError::Cancelled)
    } else {
        Ok(())
    }
}

/// Apply time selection only to media inputs, never an image turned into a slideshow.
pub fn media_settings(settings: &Settings, category: Category) -> Settings {
    let mut next = settings.clone();
    if let Some(crop) = &settings.crop {
        next.trim = TrimSettings::default();
        if matches!(category, Category::Video | Category::Audio | Category::Flash) {
            if let Some(c) = crop.media {
                next.trim = TrimSettings {
                    enabled: true,
                    start_secs: c.start_secs,
                    length_secs: c.length_secs,
                };
                // Packet-copy cuts land at keyframes; numeric ranges require decoding.
                if next.video.codec == VideoCodec::Copy {
                    next.video.codec = VideoCodec::Auto;
                }
                if next.audio.codec == AudioCodec::Copy {
                    next.audio.codec = AudioCodec::Auto;
                }
            }
        }
    }
    next
}

fn convert_intermediate(
    engine: &Engine,
    req: &PlanRequest,
    output: &Path,
    target: &str,
    cancel: &Arc<AtomicBool>,
    progress: &mut dyn FnMut(ProgressUpdate),
) -> Result<(), EngineError> {
    let mut request = req.clone();
    request.output = output.to_path_buf();
    request.target = by_id(target).expect("internal format");
    request.settings.crop = None;
    request.settings.trim = TrimSettings::default();
    request.settings.image.max_dimension = None;
    request.settings.image.strip_metadata = false;
    request.settings.image.lossless = true;
    request.settings.document.first_page_only = false;
    let plan = plan(&request, &engine.tools).map_err(|e| refused(e.to_string()))?;
    engine.run(&plan, None, cancel, progress)?;
    Ok(())
}

pub fn prepare(
    req: &mut PlanRequest,
    engine: &Engine,
    cancel: &Arc<AtomicBool>,
    progress: &mut dyn FnMut(ProgressUpdate),
) -> Result<(), EngineError> {
    let Some(crop) = req.settings.crop.take() else { return Ok(()) };
    crop.validate().map_err(refused)?;
    check_cancel(cancel)?;
    std::fs::create_dir_all(&req.temp_dir)?;
    match req.source.category {
        Category::Image if crop.image.is_some() => {
            let c = crop.image.unwrap();
            let mut input = req.input.clone();
            if req.source.read.needs_helper() {
                if req.source_is_animated {
                    return Err(refused("This animated image needs a decoder that cannot crop all frames. Convert it to APNG first, then crop."));
                }
                input = req.temp_dir.join("crop-decoded.png");
                convert_intermediate(engine, req, &input, "png", cancel, progress)?;
            }
            // ffprobe also interprets printf-like names as sequences. Give both tools
            // a literal scratch name without changing the source or loading it into RAM.
            if has_image2_sequence_spec(&input.to_string_lossy()) {
                let alias = req.temp_dir.join(format!("crop-input.{}", req.source.id));
                if std::fs::hard_link(&input, &alias).is_err() {
                    std::fs::copy(&input, &alias)?;
                }
                input = alias;
            }
            let info = engine.probe_cancellable(&input, cancel);
            check_cancel(cancel)?;
            let (w, h) = info.and_then(|i| Some((i.width?, i.height?))).ok_or_else(|| {
                refused("Cannot read image dimensions. Convert to PNG or APNG first, then crop.")
            })?;
            if c.x + c.width > w || c.y + c.height > h {
                return Err(refused(format!(
                    "Crop {}x{} at {},{} exceeds this image's {}x{} bounds. Reduce the rectangle.",
                    c.width, c.height, c.x, c.y, w, h
                )));
            }
            let id = if req.source_is_animated { "apng" } else { "png" };
            let output = req.temp_dir.join(format!("cropped.{id}"));
            let program = engine
                .tools
                .path(Tool::Ffmpeg)
                .ok_or_else(|| refused("Cropping needs the bundled FFmpeg. Reinstall the app."))?;
            let mut args = vec![
                "-hide_banner".into(),
                "-nostdin".into(),
                "-loglevel".into(),
                "error".into(),
                "-y".into(),
            ];
            args.extend([
                "-i".into(),
                input.to_string_lossy().into_owned(),
                "-vf".into(),
                format!("format=rgba,crop={}:{}:{}:{}:exact=1", c.width, c.height, c.x, c.y),
                "-c:v".into(),
                id.into(),
            ]);
            if req.source_is_animated {
                args.extend(["-f".into(), "apng".into(), "-plays".into(), "0".into()]);
            } else {
                args.extend(["-frames:v".into(), "1".into(), "-update".into(), "1".into()]);
            }
            args.push(output.to_string_lossy().into_owned());
            let plan = Plan {
                midi: None,
                output: output.clone(),
                summary: "Crop image".into(),
                temp_paths: vec![],
                steps: vec![Step {
                    tool: Tool::Ffmpeg,
                    program: program.to_path_buf(),
                    args,
                    weight: 1.0,
                    ffmpeg_progress: false,
                    label: "Crop image".into(),
                    post: None,
                    ensure_dirs: vec![req.temp_dir.clone()],
                }],
            };
            engine.run(&plan, None, cancel, progress)?;
            req.input = output;
            req.source = by_id(id).unwrap();
        }
        Category::Document if crop.document.is_some() => {
            let c = crop.document.unwrap();
            match c.unit {
                DocumentUnit::Pages => {
                    if req.target.category == Category::Image
                        && !engine.tools.has(Tool::PdfToPpm)
                        && !engine.tools.has(Tool::Magick)
                    {
                        return Err(refused(format!(
                            "Page ranges need Poppler or ImageMagick for image output. {}",
                            Tool::PdfToPpm.install_hint()
                        )));
                    }
                    let mut input = req.input.clone();
                    if req.source.id != "pdf" {
                        input = req.temp_dir.join("crop-pages.pdf");
                        convert_intermediate(engine, req, &input, "pdf", cancel, progress)?;
                    }
                    limited_file(&input)?;
                    let mut pdf = lopdf::Document::load(&input)
                        .map_err(|e| refused(format!("Cannot read PDF pages: {e}")))?;
                    if pdf.is_encrypted() {
                        return Err(refused("Unlock this PDF before selecting pages."));
                    }
                    let pages = pdf.get_pages();
                    if c.start as usize > pages.len() {
                        return Err(refused(format!("Page {} is past the document's {} pages. Choose an earlier start page.", c.start, pages.len())));
                    }
                    let remove: Vec<u32> =
                        pages.keys().copied().filter(|p| *p < c.start || *p > c.end).collect();
                    pdf.delete_pages(&remove);
                    pdf.prune_objects();
                    check_cancel(cancel)?;
                    let output = req.temp_dir.join("cropped.pdf");
                    pdf.save(&output)?;
                    req.input = output;
                    req.source = by_id("pdf").unwrap();
                    // An explicit page range overrides the ordinary first-page-only raster setting.
                    req.settings.document.first_page_only = false;
                }
                DocumentUnit::Words => {
                    let mut input = req.input.clone();
                    if req.source.id != "txt" {
                        input = req.temp_dir.join("crop-text.txt");
                        convert_intermediate(engine, req, &input, "txt", cancel, progress)?;
                    }
                    limited_file(&input)?;
                    let text = std::fs::read_to_string(&input)
                        .map_err(|e| refused(format!("Cannot read extracted UTF-8 text: {e}")))?;
                    check_cancel(cancel)?;
                    let selected = select_words(&text, c.start, c.end)?;
                    if req.target.id == "txt" {
                        let output = req.temp_dir.join("cropped.txt");
                        std::fs::write(&output, selected)?;
                        req.input = output;
                        req.source = by_id("txt").unwrap();
                        return check_cancel(cancel);
                    }
                    let output = req.temp_dir.join("cropped.html");
                    // Plain text is escaped, never interpreted as HTML from the input document.
                    let html =
                        selected.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                    std::fs::write(&output, format!("<!doctype html><html><head><meta charset=\"utf-8\"></head><body><pre>{html}</pre></body></html>"))?;
                    req.input = output;
                    req.source = by_id("html").unwrap();
                }
            }
        }
        _ => {}
    }
    check_cancel(cancel)
}

fn limited_file(path: &Path) -> Result<(), EngineError> {
    if std::fs::metadata(path)?.len() > 128 * 1024 * 1024 {
        return Err(refused(
            "Document selection supports files up to 128 MB. Split this document first.",
        ));
    }
    Ok(())
}

/// Publish a selected PDF/text file without re-importing it through an office converter.
/// The staged file is on the destination volume; a failed copy leaves existing output intact.
pub fn publish(
    input: &Path,
    output: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<crate::engine::JobOutcome, EngineError> {
    use std::io::Write;
    check_cancel(cancel)?;
    let parent = output.parent().ok_or_else(|| refused("Output needs a parent folder."))?;
    std::fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    let mut source = std::fs::File::open(input)?;
    let bytes = std::io::copy(&mut source, &mut staged)?;
    staged.flush()?;
    check_cancel(cancel)?;
    staged.persist(output).map_err(|e| EngineError::Io(e.error))?;
    Ok(crate::engine::JobOutcome { outputs: vec![output.to_path_buf()], bytes, elapsed_ms: 0 })
}

fn select_words(text: &str, start: u32, end: u32) -> Result<&str, EngineError> {
    let mut count = 0u32;
    let mut begin = None;
    let mut finish = text.len();
    let mut in_word = false;
    for (i, ch) in text.char_indices() {
        if ch.is_whitespace() {
            in_word = false;
        } else if !in_word {
            in_word = true;
            count += 1;
            if count == start {
                begin = Some(i);
            }
            if count > end {
                finish = i;
                break;
            }
        }
    }
    begin.map(|i| text[i..finish].trim_end()).ok_or_else(|| refused(format!(
        "Word {start} is past the document's {count} extracted words. Choose an earlier start; scanned PDFs need OCR first."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Document, Object};

    fn request(dir: &Path, source: &str, unit: DocumentUnit, start: u32, end: u32) -> PlanRequest {
        let settings = Settings {
            crop: Some(CropSettings {
                document: Some(DocumentCrop { unit, start, end }),
                ..Default::default()
            }),
            ..Default::default()
        };
        PlanRequest {
            input: dir.join(format!("input.{source}")),
            output: dir.join(format!("result.{source}")),
            source: by_id(source).unwrap(),
            target: by_id(source).unwrap(),
            settings,
            temp_dir: dir.join("scratch"),
            source_is_animated: false,
        }
    }

    #[test]
    fn selected_pdf_pages_keep_page_objects_and_the_original_without_helpers() {
        let dir = tempfile::tempdir().unwrap();
        let mut req = request(dir.path(), "pdf", DocumentUnit::Pages, 2, 3);
        let mut pdf = Document::with_version("1.5");
        let pages_id = pdf.new_object_id();
        let kids: Vec<Object> = (1..=4)
            .map(|n| {
                pdf.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                "TestMarker" => n })
                    .into()
            })
            .collect();
        pdf.objects.insert(
            pages_id,
            dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 4 }.into(),
        );
        let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        pdf.trailer.set("Root", catalog);
        pdf.save(&req.input).unwrap();
        let original = std::fs::read(&req.input).unwrap();
        let engine = Engine::new(Default::default());
        let cancel = Arc::new(AtomicBool::new(false));
        prepare(&mut req, &engine, &cancel, &mut |_| {}).unwrap();
        let cropped = Document::load(&req.input).unwrap();
        let markers: Vec<i64> = cropped
            .get_pages()
            .values()
            .map(|id| {
                cropped
                    .get_object(*id)
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get(b"TestMarker")
                    .unwrap()
                    .as_i64()
                    .unwrap()
            })
            .collect();
        assert_eq!(markers, vec![2, 3]);
        assert_eq!(std::fs::read(dir.path().join("input.pdf")).unwrap(), original);
        std::fs::write(&req.output, "previous result").unwrap();
        cancel.store(true, Ordering::Relaxed);
        assert!(publish(&req.input, &req.output, &cancel).is_err());
        assert_eq!(std::fs::read_to_string(&req.output).unwrap(), "previous result");
        cancel.store(false, Ordering::Relaxed);
        publish(&req.input, &req.output, &cancel).unwrap();
        assert_eq!(Document::load(&req.output).unwrap().get_pages().len(), 2);
    }

    #[test]
    fn text_selection_needs_no_helper_and_rejects_empty_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let mut req = request(dir.path(), "txt", DocumentUnit::Words, 2, 3);
        std::fs::write(&req.input, "one two\nthree four").unwrap();
        prepare(
            &mut req,
            &Engine::new(Default::default()),
            &Arc::new(AtomicBool::new(false)),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(req.input).unwrap(), "two\nthree");
        let mut empty = request(dir.path(), "txt", DocumentUnit::Words, 5, 8);
        assert!(prepare(
            &mut empty,
            &Engine::new(Default::default()),
            &Arc::new(AtomicBool::new(false)),
            &mut |_| {}
        )
        .is_err());
    }

    #[test]
    fn words_are_one_based_inclusive_and_keep_internal_whitespace() {
        assert_eq!(select_words("one  two\nthree four", 2, 3).unwrap(), "two\nthree");
        assert_eq!(select_words("one two", 1, 10).unwrap(), "one two");
        assert!(select_words("one two", 3, 5).is_err());
        assert_eq!(select_words("你好 世界 café", 2, 3).unwrap(), "世界 café");
    }
    #[test]
    fn numeric_ranges_are_validated() {
        let mut c = CropSettings::default();
        assert!(c.validate().is_err());
        c.media = Some(MediaCrop { start_secs: 10.0, length_secs: 40.0 });
        assert!(c.validate().is_ok());
        c.media.as_mut().unwrap().length_secs = f64::NAN;
        assert!(c.validate().is_err());
        c.media = None;
        c.image = Some(ImageCrop { x: 0, y: 0, width: 0, height: 100 });
        assert!(c.validate().is_err());
    }
    #[test]
    fn precise_media_selection_disables_packet_copy_and_does_not_mutate_preferences() {
        let mut settings = Settings::default();
        settings.video.codec = VideoCodec::Copy;
        settings.crop = Some(CropSettings {
            media: Some(MediaCrop { start_secs: 10.0, length_secs: 40.0 }),
            ..Default::default()
        });
        let effective = media_settings(&settings, Category::Video);
        assert_eq!(effective.trim.length_secs, 40.0);
        assert_eq!(effective.video.codec, VideoCodec::Auto);
        assert_eq!(settings.video.codec, VideoCodec::Copy);
        assert!(!media_settings(&settings, Category::Image).trim.enabled);
    }
}
