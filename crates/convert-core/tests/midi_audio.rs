//! Real MIDI → PCM → encoded audio, including cancellation and queue integration.
use convert_core::{
    format::{by_id, Tool},
    plan::{plan, PlanRequest},
    queue::{run_batch, BatchEvent, BatchItem},
    settings::{AudioCodec, ConflictPolicy, TrimSettings},
    Engine, Settings, ToolRegistry,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

fn tools() -> Option<ToolRegistry> {
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
    for (tool, name) in [(Tool::Ffmpeg, "ffmpeg"), (Tool::Ffprobe, "ffprobe")] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../src-tauri/binaries")
            .join(format!("{name}-{triple}{}", if cfg!(windows) { ".exe" } else { "" }));
        if path.is_file() {
            tools.set(tool, path);
        }
    }
    if !tools.has(Tool::Ffmpeg) || !tools.has(Tool::Ffprobe) {
        eprintln!("SKIP: FFmpeg and ffprobe required");
        return None;
    }
    Some(tools)
}
fn fixture() -> Vec<u8> {
    let track = [
        0, 0x90, 60, 100, 0, 0x90, 64, 80, 0x87, 0x40, 0x80, 60, 0, 0, 0x80, 64, 0, 0, 0xff, 0x2f,
        0,
    ];
    let mut bytes = b"MThd\0\0\0\x06\0\0\0\x01\x01\xe0MTrk".to_vec();
    bytes.extend((track.len() as u32).to_be_bytes());
    bytes.extend(track);
    bytes
}
#[test]
fn midi_queue_writes_piano_mp3_and_wav_and_keeps_original() {
    let Some(tools) = tools() else { return };
    let engine = Engine::new(tools);
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("Piano notes.mid");
    std::fs::write(&input, fixture()).unwrap();
    for target in ["wav", "mp3", "flac"] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let record = events.clone();
        run_batch(
            vec![BatchItem::file(target, input.clone(), target)],
            Settings::default(),
            Arc::new(Engine::new(engine.tools.clone())),
            Arc::new(AtomicBool::new(false)),
            Arc::new(move |e| record.lock().unwrap().push(e)),
        );
        let result = dir.path().join(format!("Converted/Piano notes.{target}"));
        assert!(result.is_file(), "{target}: {:?}", events.lock().unwrap());
        let info = engine.probe(&result).unwrap();
        assert!(info.has_audio && !info.has_video);
        assert!((info.duration_secs.unwrap() - 2.0).abs() < 0.15);
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, BatchEvent::BatchFinished { ok: 1, failed: 0, .. })));
    }
    assert_eq!(std::fs::read(input).unwrap(), fixture());
}
#[test]
fn midi_copy_option_encodes_and_trim_applies_after_synthesis() {
    let Some(tools) = tools() else { return };
    let engine = Engine::new(tools);
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("song.midi");
    std::fs::write(&input, fixture()).unwrap();
    let mut settings = Settings::default();
    settings.audio.codec = AudioCodec::Copy;
    settings.trim = TrimSettings { enabled: true, start_secs: 0.25, length_secs: 0.5 };
    let req = PlanRequest {
        input,
        output: dir.path().join("cut.mp3"),
        source: by_id("midi").unwrap(),
        target: by_id("mp3").unwrap(),
        settings,
        temp_dir: dir.path().join("scratch"),
        source_is_animated: false,
    };
    let p = plan(&req, &engine.tools).unwrap();
    engine.run(&p, Some(0.5), &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap();
    assert!((engine.probe(&req.output).unwrap().duration_secs.unwrap() - 0.5).abs() < 0.1);
    assert!(!req.temp_dir.join("midi-piano.wav").exists());
}
#[test]
fn cancelled_piano_render_preserves_output_and_removes_intermediate() {
    let Some(tools) = tools() else { return };
    let engine = Engine::new(tools);
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("song.mid");
    std::fs::write(&input, fixture()).unwrap();
    let mut settings = Settings::default();
    settings.output.on_conflict = ConflictPolicy::Overwrite;
    let req = PlanRequest {
        input,
        output: dir.path().join("keep.wav"),
        source: by_id("midi").unwrap(),
        target: by_id("wav").unwrap(),
        settings,
        temp_dir: dir.path().join("scratch"),
        source_is_animated: false,
    };
    std::fs::write(&req.output, b"previous output").unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let result = engine.run(&plan(&req, &engine.tools).unwrap(), Some(2.0), &cancel, &mut |_| {
        cancel.store(true, Ordering::Relaxed)
    });
    assert!(matches!(result, Err(convert_core::engine::EngineError::Cancelled)));
    assert_eq!(std::fs::read(&req.output).unwrap(), b"previous output");
    assert!(!req.temp_dir.join("midi-piano.wav").exists());
}

#[test]
fn midi_crop_batch_keeps_only_requested_time_range() {
    let Some(tools) = tools() else { return };
    let engine = Arc::new(Engine::new(tools));
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("range.mid");
    std::fs::write(&input, fixture()).unwrap();
    let settings = Settings {
        crop: Some(convert_core::crop::CropSettings {
            media: Some(convert_core::crop::MediaCrop { start_secs: 0.2, length_secs: 0.4 }),
            image: None,
            document: None,
        }),
        ..Settings::default()
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let record = events.clone();
    run_batch(
        vec![BatchItem::file("crop", input, "wav")],
        settings,
        engine.clone(),
        Arc::new(AtomicBool::new(false)),
        Arc::new(move |e| record.lock().unwrap().push(e)),
    );
    let output = dir.path().join("Converted/range.wav");
    assert!(output.is_file(), "{:?}", events.lock().unwrap());
    assert!((engine.probe(&output).unwrap().duration_secs.unwrap() - 0.4).abs() < 0.01);
}

#[test]
fn corrupt_midi_fails_without_replacing_existing_audio() {
    let Some(tools) = tools() else { return };
    let engine = Engine::new(tools);
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("damaged.mid");
    let mut bytes = fixture();
    bytes.truncate(bytes.len() - 3);
    std::fs::write(&input, bytes).unwrap();
    let req = PlanRequest {
        input,
        output: dir.path().join("keep.wav"),
        source: by_id("midi").unwrap(),
        target: by_id("wav").unwrap(),
        settings: Settings::default(),
        temp_dir: dir.path().join("scratch"),
        source_is_animated: false,
    };
    std::fs::write(&req.output, b"previous result").unwrap();
    let result = engine.run(
        &plan(&req, &engine.tools).unwrap(),
        None,
        &Arc::new(AtomicBool::new(false)),
        &mut |_| {},
    );
    assert!(matches!(result, Err(convert_core::engine::EngineError::Selection(_))));
    assert_eq!(std::fs::read(&req.output).unwrap(), b"previous result");
    assert!(!req.temp_dir.join("midi-piano.wav").exists());
}
