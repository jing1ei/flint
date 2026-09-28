# Windows PC Builds

Target: **Windows 10/11 x64**, Rust `x86_64-pc-windows-msvc`, Tauri 2, WebView2.
Windows ARM64 and 32-bit installers are not configured.

## Build Locally On Windows

Install:

- Node.js 22 LTS and npm.
- Rust stable MSVC, version 1.88 or later, from rustup.rs.
- Visual Studio 2022 Build Tools with **Desktop development with C++** and the Windows SDK.
- 7-Zip from https://www.7-zip.org.
- Microsoft Edge WebView2 Runtime for development/testing.

From PowerShell in the source folder:

```powershell
powershell -NoProfile -File .\Build-Windows.ps1
```

The script installs locked npm dependencies, verifies/downloads Windows FFmpeg and ffprobe,
runs frontend tests, checks the Rust workspace, runs native Windows smoke tests, and builds NSIS.
If your execution policy blocks local scripts, inspect the script and use your organization's
approved signing or execution-policy process; the build does not change your system policy.

The output is `target\release\bundle\nsis\*-setup.exe`.
The installer is per-user. It downloads the WebView2 bootstrapper if the runtime is missing, so
first-time installation may need internet access. Conversion itself remains local.

For development after fetching the sidecars:

```powershell
npm run tauri dev
```

## GitHub Actions

Push the source to a GitHub repository and run **Windows PC build**, or allow its push/PR trigger.
The job uses `windows-2022`, runs the local build script, checks Clippy, and uploads
**Flint-Windows-x64**. Download it from the workflow run's Artifacts section.
It contains the installer, checksums, FFmpeg build/license evidence and upstream source archive.

The existing **Release** workflow still publishes only macOS artifacts. No Windows release is
published automatically. Installer signing is not configured, so Windows may warn about an unknown
publisher. Do not disable SmartScreen globally; verify the origin and checksums before proceeding.

## Runtime Differences

- Native Windows shell APIs open/reveal files without invoking `cmd.exe`.
- Background conversions do not open console windows.
- Explorer's configured Downloads folder is used for pasted-link outputs by default.
- Browser detection checks Windows installation and cookie-profile locations. Chromium
  app-bound cookie encryption can still prevent extraction; use Firefox or an exported cookies
  file when yt-dlp refuses browser access. No Safari permission flow applies on Windows.
- Optional tools use Windows-specific instructions in Settings. Install them manually and
  restart the app after changing PATH. Homebrew auto-install remains macOS-only.
- Software encoding is used; automatic NVENC/QSV/AMF selection is not implemented.
- macOS-only `sips` formats need ImageMagick or another compatible helper on Windows.

## Sidecars And Distribution

`scripts/fetch-sidecars.ps1` pins Gyan's **FFmpeg 9.0.1 full build** archive and SHA-256.
The full build includes encoders required by the shared catalog, including SVT-AV1.
Tauri expects `ffmpeg-x86_64-pc-windows-msvc.exe` and
`ffprobe-x86_64-pc-windows-msvc.exe` under `src-tauri/binaries`.

The binaries are GPL-3.0-or-later and run as separate processes. Build evidence and GPL text
are included in the installer. The workflow also includes upstream FFmpeg source.
That archive is **not** proof of complete corresponding source for the publisher's patches and
all linked dependencies. Complete the corresponding-source/signing review before public
redistribution; do not interpret a successful build as license-compliance certification.

See [Platform compatibility](COMPATIBILITY.md) for the minimum-OS validation matrix.

## Verification Status

On the macOS development host, the Windows core and `windows_smoke` target were cross-checked
successfully, and the Windows shell API module compiled separately against its Windows dependencies.
The desktop cross-check accepted the Windows configuration and verified sidecar
filenames, but stopped at the missing `llvm-rc` resource compiler. No Windows installer was produced
or launched on this host. A successful native Windows workflow run is still required.

The Unix shell-fixture suite stays on macOS/Linux. Dedicated Windows smoke tests exercise a real
bundled conversion/probe, filenames containing spaces/Unicode/shell characters, source hard-link
protection, Windows output paths, and LibreOffice file-URL construction. Installer installation,
native dialogs, Explorer integration, authenticated downloads, and optional helpers still need
manual Windows validation before release.
