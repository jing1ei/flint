# Platform compatibility

Runtime targets are macOS 11 Big Sur or newer on Intel x86_64, macOS 12 Monterey or newer on Apple Silicon arm64, and Windows 10/11 x64 with Microsoft Edge WebView2. Development tools can require a newer host than the packaged app.

## What is enforced

- Frontend syntax targets Safari 14 and Edge 109. Dialog and `inert` polyfills are bundled locally for older WebKit. Crop sizing uses `vh` fallbacks; split-button styling does not require `:has()`.
- `.cargo/config.toml` sets `MACOSX_DEPLOYMENT_TARGET=11.0` for Cargo and native dependency builds. Intel's bundle minimum is also 11.0. `npm run build:app` and `npm run build:dmg` use `scripts/build-macos.mjs`; on arm64 it sets the deployment target to 12.0 and merges `src-tauri/tauri.arm64.conf.json` so the bundle also declares 12.0.
- Release jobs build on separate native Intel and Apple Silicon runners. The macOS compatibility gate checks the actual Mach-O architecture and minimum OS of the application, FFmpeg and ffprobe. A successful check is necessary but does not prove all runtime APIs work on the minimum OS.
- Windows builds target `x86_64-pc-windows-msvc`. The per-user NSIS installer installs WebView2 when missing and requests an update for versions older than 109. First installation can need internet access; an offline Windows 10 machine needs WebView2 provisioned separately.

## Current verification and blockers

| Target | Evidence | Remaining validation |
| --- | --- | --- |
| Intel Big Sur | Core/tests and Tauri shell cross-check on x86_64; inspected default FFmpeg and ffprobe declare macOS 10.13 minimum | Launch and conversion on a real Intel Big Sur installation |
| Apple Silicon Monterey | Package minimum and compatibility gate both require macOS 12.0, matching the bundled FFmpeg/ffprobe | Launch and conversion on a real macOS 12 installation |
| Windows 10 x64 | Windows core and test targets cross-check; native Windows build/smoke workflow exists | Run the native workflow, install the result and test on Windows 10; cross-compilation does not verify installer or Explorer behavior |

Modern Chromium/WebKit tests remove native dialog and inert APIs before startup to exercise fallbacks. This is feature-removal testing, not Safari 14 or Windows 10 emulation. Actual minimum-OS launch tests remain required before declaring those builds verified.

Do not change Mach-O version headers to make a binary appear compatible. That cannot remove newer OS symbols or requirements from its linked libraries. Optional helpers have their own OS requirements; use versions supporting the installed OS.

## Minimum-OS acceptance test

On each actual target: install and launch; add files through the chooser and drag/drop; convert image/audio/video fixtures; crop then cancel and retry; use keyboard navigation through Settings and crop; open/reveal outputs; test spaces and Unicode in paths; cancel a running conversion and confirm originals survive. On Windows additionally test a machine without WebView2 and verify the installer provisions it. On Intel test a CPU representative of the oldest supported hardware.

References: [Tauri webview versions](https://v2.tauri.app/reference/webview-versions/), [Intel FFmpeg publisher](https://evermeet.cx/ffmpeg/), [Windows FFmpeg publisher](https://www.gyan.dev/ffmpeg/builds/).
