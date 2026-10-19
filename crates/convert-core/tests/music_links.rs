//! Deterministic music contracts plus opt-in live source checks.
use convert_core::{
    format::{by_id, Category, Tool},
    link::{self, FetchOptions, Link},
    tools::ToolRegistry,
    Engine,
};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

const MUSIC: &[&str] = &[
    "https://y.qq.com/n/ryqq/songDetail/004Ti8rT003TaZ",
    "https://music.163.com/song?id=17241424",
    "https://soundcloud.com/the80m/the-following",
    "https://benprunty.bandcamp.com/track/lanius-battle",
];

fn tools() -> ToolRegistry {
    let mut tools = ToolRegistry::discover(None);
    let triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", _) => "x86_64-apple-darwin",
        ("windows", _) => "x86_64-pc-windows-msvc",
        _ => return tools,
    };
    for (tool, name) in [(Tool::Ffmpeg, "ffmpeg"), (Tool::Ffprobe, "ffprobe")] {
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../src-tauri/binaries")
            .join(format!("{name}-{triple}{suffix}"));
        if path.is_file() {
            tools.set(tool, path);
        }
    }
    tools
}

#[test]
fn music_source_contract_is_audio_only_and_config_independent() {
    for url in MUSIC {
        let link = Link::parse(url).unwrap();
        assert_eq!(link.site().category(), Category::Audio);
        let options = FetchOptions::for_target(
            PathBuf::from("/tmp/music-test"),
            by_id("mp3").unwrap(),
            None,
            None,
            None,
        );
        let args = link::fetch_args(&link, &options);
        assert!(args.contains(&"--ignore-config".into()));
        assert!(args.contains(&"--embed-metadata".into()));
        assert!(args.iter().any(|arg| arg.contains("format_id!*=preview")));
        assert_eq!(args.last().unwrap(), url);
    }
}

#[test]
#[ignore = "Uses public source sites; run explicitly to check current availability without media output"]
fn live_music_metadata_resolves_on_each_source() {
    let engine = Engine::new(tools());
    assert!(engine.tools.has(Tool::YtDlp), "install yt-dlp to run live checks");
    for url in MUSIC {
        let link = Link::parse(url).unwrap();
        let info = engine
            .probe_link_cancellable(&link, None, &Arc::new(AtomicBool::new(false)))
            .unwrap_or_else(|e| panic!("{url}: {e}"));
        assert!(!info.title.is_empty());
        eprintln!("{}: {} ({:?} seconds)", link.site(), info.title, info.duration_secs);
    }
}

#[test]
#[ignore = "Retrieves the artist-enabled Bandcamp test track; run explicitly, never during offline CI"]
fn live_bandcamp_source_converts_and_retains_audio_metadata() {
    use convert_core::{
        plan::{plan, PlanRequest},
        Settings,
    };
    let engine = Engine::new(tools());
    let dir = tempfile::tempdir().unwrap();
    let link = Link::parse(MUSIC[3]).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let metadata = engine.probe_link_cancellable(&link, None, &cancel).unwrap();
    let options = FetchOptions::for_target(
        dir.path().join("source"),
        by_id("mp3").unwrap(),
        engine.tools.path(Tool::Ffmpeg).map(|p| p.to_path_buf()),
        engine.js_runtime(),
        None,
    );
    let input = engine.fetch_link(&link, &options, &cancel, &mut |_| {}).unwrap();
    let source = convert_core::format::by_path(&input).unwrap();
    let req = PlanRequest {
        input,
        output: dir.path().join("result.mp3"),
        source,
        target: by_id("mp3").unwrap(),
        settings: Settings::default(),
        temp_dir: dir.path().join("scratch"),
        source_is_animated: false,
    };
    let planned = plan(&req, &engine.tools).unwrap();
    engine.run(&planned, metadata.duration_secs, &cancel, &mut |_| {}).unwrap();
    let result = engine.probe(&req.output).unwrap();
    assert!(result.has_audio);
    assert!(!link::incomplete_music(metadata.duration_secs, result.duration_secs));
    let output = std::process::Command::new(engine.tools.path(Tool::Ffprobe).unwrap())
        .args(["-v", "error", "-show_entries", "format_tags=title,artist,album", "-of", "json"])
        .arg(&req.output)
        .output()
        .unwrap();
    let tags: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(tags["format"]["tags"]["title"].as_str().is_some_and(|t| !t.is_empty()), "{tags}");
}
