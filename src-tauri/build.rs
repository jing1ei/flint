fn main() {
    // The sidecar binaries are named `ffmpeg-<target triple>` on disk (Tauri's convention), so the
    // runtime needs to know which triple it was built for. `TARGET` is only set for build scripts,
    // hence this re-export into the crate's compile-time environment (see `sidecar.rs`).
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=BUILD_TARGET_TRIPLE={target}");

    ensure_sidecars(&target);
    tauri_build::build();
}

/// Fail the build with the command that fixes it.
///
/// `tauri.conf.json` declares these two files as `externalBin`, so a fresh checkout cannot compile
/// until they exist - correct, because an app that silently ships without its conversion engine is
/// worse than one that refuses to build. Tauri's own error for this is
/// `resource path binaries/ffmpeg-<triple> doesn't exist`, which says nothing about the script that
/// downloads them; this check runs first purely so the message is actionable.
fn ensure_sidecars(target: &str) {
    // Cargo caches build scripts aggressively: without this, deleting a sidecar would not re-trigger
    // the check and the failure would surface much later, during bundling.
    println!("cargo:rerun-if-changed=binaries");

    let dir = std::path::Path::new("binaries");
    let suffix = if target.contains("windows") { ".exe" } else { "" };
    let fetch = if target.contains("windows") {
        "powershell -NoProfile -File scripts/fetch-sidecars.ps1"
    } else {
        "./scripts/fetch-sidecars.sh"
    };
    let missing: Vec<String> = ["ffmpeg", "ffprobe"]
        .iter()
        .map(|name| format!("{name}-{target}{suffix}"))
        .filter(|file| !dir.join(file).exists())
        .collect();

    if missing.is_empty() {
        return;
    }

    panic!(
        "\n\n\
         Flint needs its FFmpeg sidecars before it can build.\n\
         Missing from src-tauri/binaries/: {}\n\n\
         Fix it from the project root:\n\
         \x20   {fetch}\n\n\
         Then re-run your build.\n\n\
         Only `cargo test -p convert-core` (the portable engine) runs without them.\n\n",
        missing.join(", ")
    );
}
