# Flint release status

Current deliverable: Flint 1.0.0 source with the pink-blue theme and MIDI piano rendering.
The source archive contains no build output or Git history.

## Verified

- Production frontend build, TypeScript check and unit tests (`tsc --noEmit`, `vitest`, `vite build`).
- Rust: `cargo fmt`, `cargo clippy --workspace -D warnings`, the convert-core suite and the
  native IPC/shell suite.
- Chromium and WebKit UI journeys, including legacy dialog/focus fallbacks and narrow layouts.
- Intel application minimum: macOS 11.0. Bundled Intel tools minimum: macOS 10.13.
- Apple Silicon package minimum: macOS 12.0, matching the bundled tools.
- The Apple Silicon app is built and smoke-tested locally on each source handoff.

## Remaining shipping gates

- Launch and convert on a real macOS 12 (Apple Silicon), macOS 11 (Intel) and Windows 10 x64
  machine. Feature-removal tests are not minimum-OS emulation.
- Developer ID signing/notarization and Windows publisher signing are not configured.
- The redistributed FFmpeg binaries are community builds. Upstream source, configure lines and
  written offers are attached per architecture; complete publisher-patch correspondence remains
  an open compliance item.

See [Compatibility](COMPATIBILITY.md) and [Release procedure](RELEASING.md).
