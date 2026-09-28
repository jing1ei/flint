#![cfg(windows)]

use convert_core::{
    format::{by_id, Tool},
    plan::{plan, PlanRequest},
    Engine, Settings, ToolRegistry,
};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

#[test]
fn bundled_windows_engine_converts_paths_with_spaces_unicode_and_shell_characters() {
    let binaries = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/binaries");
    let mut tools = ToolRegistry::default();
    for (tool, name) in [(Tool::Ffmpeg, "ffmpeg"), (Tool::Ffprobe, "ffprobe")] {
        let path = binaries.join(format!("{name}-x86_64-pc-windows-msvc.exe"));
        assert!(path.is_file(), "missing {}", path.display());
        tools.set(tool, path);
    }
    let scratch = std::env::temp_dir().join(format!("cc-win-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let input = scratch.join("photo & 100% \u{00e9}.ppm");
    std::fs::write(&input, b"P6\n2 2\n255\n\xff\0\0\0\xff\0\0\0\xff\xff\xff\xff").unwrap();
    let output = scratch.join("result & 100% \u{00e9}.png");
    let engine = Engine::new(tools);
    let request = PlanRequest {
        input: input.clone(),
        output: output.clone(),
        source: by_id("ppm").unwrap(),
        target: by_id("png").unwrap(),
        settings: Settings::default(),
        temp_dir: scratch.join("scratch"),
        source_is_animated: false,
    };
    let plan = plan(&request, &engine.tools).unwrap();
    engine.run(&plan, None, &Arc::new(AtomicBool::new(false)), &mut |_| {}).unwrap();
    assert!(std::fs::metadata(&output).unwrap().len() > 0);
    assert!(engine.probe(&output).is_some());
    let alias = scratch.join("alias.png");
    std::fs::hard_link(&output, &alias).unwrap();
    assert!(convert_core::paths::is_same_file(&output, &alias));
    std::fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn windows_paths_and_recovery_are_platform_appropriate() {
    assert!(convert_core::tools::home_dir().unwrap().is_absolute());
    let settings = Settings::default();
    assert!(convert_core::paths::link_output_dir(&settings.output).is_absolute());
    assert!(!Tool::Pandoc.install_hint().contains("brew"));
    assert!(Tool::Deno.install_hint().contains("winget"));

    let mut tools = ToolRegistry::default();
    tools
        .set(Tool::LibreOffice, PathBuf::from(r"C:\Program Files\LibreOffice\program\soffice.exe"));
    let request = PlanRequest {
        input: PathBuf::from(r"C:\input\document.docx"),
        output: PathBuf::from(r"C:\output\document.pdf"),
        source: by_id("docx").unwrap(),
        target: by_id("pdf").unwrap(),
        settings: Settings::default(),
        temp_dir: PathBuf::from(r"C:\scratch\space & hash#"),
        source_is_animated: false,
    };
    let planned = plan(&request, &tools).unwrap();
    let profile = planned
        .steps
        .iter()
        .flat_map(|step| &step.args)
        .find_map(|arg| arg.strip_prefix("-env:UserInstallation="))
        .unwrap();
    let decoded = url::Url::parse(profile).unwrap().to_file_path().unwrap();
    assert!(decoded.starts_with(&request.temp_dir), "{profile}");
}
