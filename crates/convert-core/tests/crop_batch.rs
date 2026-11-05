//! Exercises batch selection with real files. CI supplies bundled FFmpeg (or system FFmpeg).
use convert_core::{
    crop::{CropSettings, DocumentCrop, DocumentUnit, ImageCrop, MediaCrop},
    format::Tool,
    queue::{run_batch, BatchEvent, BatchItem},
    tools::ToolRegistry,
    Engine, Settings,
};
use std::{
    path::PathBuf,
    process::Command,
    sync::{atomic::AtomicBool, Arc, Mutex},
};

fn tools() -> ToolRegistry {
    let mut tools = ToolRegistry::discover(None);
    let triple = if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        }
    } else {
        "x86_64-pc-windows-msvc"
    };
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    for (tool, name) in [(Tool::Ffmpeg, "ffmpeg"), (Tool::Ffprobe, "ffprobe")] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../src-tauri/binaries")
            .join(format!("{name}-{triple}{suffix}"));
        if path.is_file() {
            tools.set(tool, path);
        }
    }
    tools
}

fn batch(engine: Engine, items: Vec<BatchItem>, settings: Settings) -> Vec<BatchEvent> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let record = events.clone();
    run_batch(
        items,
        settings,
        Arc::new(engine),
        Arc::new(AtomicBool::new(false)),
        Arc::new(move |event| record.lock().unwrap().push(event)),
    );
    Arc::try_unwrap(events).unwrap().into_inner().unwrap()
}

#[test]
fn mixed_batch_crops_pixels_and_media_but_keeps_sources_intact() {
    let tools = tools();
    let (Some(ffmpeg), Some(_)) = (tools.path(Tool::Ffmpeg), tools.path(Tool::Ffprobe)) else {
        eprintln!("Skipping media crop: FFmpeg/ffprobe unavailable");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("source.ppm");
    let mut ppm = b"P6\n200 150\n255\n".to_vec();
    ppm.extend(std::iter::repeat_n(0x7f, 200 * 150 * 3));
    std::fs::write(&image, &ppm).unwrap();
    let audio = dir.path().join("source.wav");
    let status = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=3",
        ])
        .arg(&audio)
        .status()
        .unwrap();
    assert!(status.success());
    let mut settings = Settings {
        crop: Some(CropSettings {
            image: Some(ImageCrop { x: 10, y: 20, width: 100, height: 80 }),
            media: Some(MediaCrop { start_secs: 0.5, length_secs: 1.0 }),
            document: None,
        }),
        ..Default::default()
    };
    let engine = Engine::new(tools.clone());
    let events = batch(
        engine,
        vec![
            BatchItem::file("crop-image", &image, "png"),
            BatchItem::file("crop-audio", &audio, "wav"),
        ],
        settings.clone(),
    );
    assert!(
        events.iter().any(|e| matches!(e, BatchEvent::BatchFinished { ok: 2, failed: 0, .. })),
        "{events:?}"
    );
    let engine = Engine::new(tools);
    let image_result = engine.probe(&dir.path().join("Converted/source.png")).unwrap();
    assert_eq!((image_result.width, image_result.height), (Some(100), Some(80)));
    let audio_result = engine.probe(&dir.path().join("Converted/source.wav")).unwrap();
    assert!((audio_result.duration_secs.unwrap() - 1.0).abs() < 0.03);
    assert_eq!(std::fs::read(&image).unwrap(), ppm);
    // A literal printf-like filename must not become an image sequence.
    let literal = dir.path().join("frame%04d.png");
    std::fs::copy(dir.path().join("Converted/source.png"), &literal).unwrap();
    let mut literal_settings = settings.clone();
    literal_settings.crop.as_mut().unwrap().image =
        Some(ImageCrop { x: 0, y: 0, width: 51, height: 39 });
    let events = batch(
        Engine::new(engine.tools.clone()),
        vec![BatchItem::file("crop-literal", &literal, "png")],
        literal_settings,
    );
    assert!(
        events.iter().any(|e| matches!(e, BatchEvent::BatchFinished { ok: 1, failed: 0, .. })),
        "{events:?}"
    );
    let literal_result = dir.path().join("literal-result.png");
    std::fs::copy(dir.path().join("Converted/frame%04d.png"), &literal_result).unwrap();
    let result = engine.probe(&literal_result).unwrap();
    assert_eq!((result.width, result.height), (Some(51), Some(39)));
    settings.crop.as_mut().unwrap().image.as_mut().unwrap().width = 999;
    let events = batch(engine, vec![BatchItem::file("crop-invalid", &image, "png")], settings);
    assert!(
        events.iter().any(
            |e| matches!(e, BatchEvent::Failed { message, .. } if message.contains("exceeds"))
        ),
        "{events:?}"
    );
    assert!(!dir.path().join("Converted/source (2).png").exists());
}

#[test]
fn text_batch_selects_inclusive_words_without_external_tools() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.txt");
    std::fs::write(&input, "one two\nthree four five").unwrap();
    let settings = Settings {
        crop: Some(CropSettings {
            document: Some(DocumentCrop { unit: DocumentUnit::Words, start: 2, end: 4 }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let events = batch(
        Engine::new(Default::default()),
        vec![BatchItem::file("crop-words", &input, "txt")],
        settings,
    );
    assert!(
        events.iter().any(|e| matches!(e, BatchEvent::BatchFinished { ok: 1, failed: 0, .. })),
        "{events:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("Converted/input.txt")).unwrap(),
        "two\nthree four"
    );
    assert_eq!(std::fs::read_to_string(input).unwrap(), "one two\nthree four five");
}

#[test]
fn video_range_reencodes_between_keyframes_and_image_crop_keeps_animation() {
    use convert_core::settings::{GifSettings, HardwareAccel, VideoCodec, VideoSettings};
    let tools = tools();
    let (Some(ffmpeg), Some(_)) = (tools.path(Tool::Ffmpeg), tools.path(Tool::Ffprobe)) else {
        eprintln!("Skipping video/animation crop: FFmpeg/ffprobe unavailable");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("movie.mkv");
    let animation = dir.path().join("animated.gif");
    for output in [&video, &animation] {
        let mut command = Command::new(ffmpeg);
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x120:rate=10:duration=3",
        ]);
        if output == &video {
            command.args(["-c:v", "mpeg4", "-g", "100"]);
        }
        assert!(command.arg(output).status().unwrap().success());
    }
    let settings = Settings {
        video: VideoSettings {
            codec: VideoCodec::Copy,
            hardware_accel: HardwareAccel::Off,
            ..Default::default()
        },
        gif: GifSettings { fps: 10.0, ..Default::default() },
        crop: Some(CropSettings {
            image: Some(ImageCrop { x: 3, y: 7, width: 50, height: 40 }),
            media: Some(MediaCrop { start_secs: 0.5, length_secs: 1.0 }),
            document: None,
        }),
        ..Default::default()
    };
    let events = batch(
        Engine::new(tools.clone()),
        vec![
            BatchItem::file("crop-video", &video, "mp4"),
            BatchItem::file("crop-animation", &animation, "apng"),
        ],
        settings,
    );
    assert!(
        events.iter().any(|e| matches!(e, BatchEvent::BatchFinished { ok: 2, failed: 0, .. })),
        "{events:?}"
    );
    let engine = Engine::new(tools);
    let result = engine.probe(&dir.path().join("Converted/movie.mp4")).unwrap();
    assert!((result.duration_secs.unwrap() - 1.0).abs() < 0.11);
    let result = engine.probe(&dir.path().join("Converted/animated.png")).unwrap();
    assert_eq!((result.width, result.height), (Some(50), Some(40)));
    let output = Command::new(engine.tools.path(Tool::Ffprobe).unwrap())
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "json",
        ])
        .arg(dir.path().join("Converted/animated.png"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["streams"][0]["nb_read_frames"], "30");
}
