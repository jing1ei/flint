# Flint 1.0.0

Deliverable: clean Flint 1.0.0 source with the pink-blue theme and MIDI piano rendering.
The source archive contains no build output or Git history. Existing installers must be rebuilt.

## Verified

- Production frontend and Rust checks; conversion, cancellation, crop, settings and recovery tests.
- Chromium and WebKit journeys, including legacy dialog/focus fallbacks and narrow layouts.
- Intel application minimum: macOS 11.0. Bundled Intel tools minimum: macOS 10.13.
- Earlier Intel build: native launch and image conversion through Rosetta on the development Mac.
  The current source has not been packaged for minimum-OS acceptance testing.

## Remaining shipping gates

- Apple Silicon packages require macOS 12, matching the bundled tools. Validate launch and conversion on Monterey.
- Test installation on actual Intel Big Sur and Windows 10. The automatic release builds a Windows x64 installer.
- Developer ID signing/notarization and Windows publisher signing are not configured.
- Supply complete corresponding source and build scripts for redistributed FFmpeg dependencies.
  An upstream tarball and binary metadata alone do not satisfy that requirement.

See [Compatibility](COMPATIBILITY.md) and [Release procedure](RELEASING.md).
