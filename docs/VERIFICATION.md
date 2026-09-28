# Verification

Run checks before publishing. Source-only handoffs contain no build output or dependencies.
Install prerequisites as described in the [README](../README.md).

## Automated Checks

| Command | Coverage |
| --- | --- |
| `cargo test --workspace --locked` | Core and native IPC contracts; path conflicts, overwrite rollback, cancellation, settings and conversion planning |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Rust static checks |
| `cargo fmt --all --check` | Rust formatting |
| `npm test` | Frontend state, crop settings and skins |
| `npm run build` | TypeScript and production bundle |
| `python3 scripts/test_sidecar_download.py` | Offline timeout, resume, cache and archive recovery tests |
| `python3 scripts/ui_behaviour.py` | Queue, settings, keyboard navigation and recovery |
| `python3 scripts/ui_crop.py` | Numeric crop workflow in Chromium |
| `python3 scripts/ui_crop.py --browser webkit` | Crop workflow, macOS pointer/focus behavior and legacy API recovery |
| `python3 scripts/ui_theme.py` | Default light palette, glass, gradient contrast and focus |
| `python3 scripts/ui_midi.py` | MIDI input discovery, audio targets and preview queue |
| `cargo test -p convert-core --test midi_audio --locked` | Real piano WAV/MP3/FLAC, trim, crop and cancellation |
| `python3 scripts/ui_music.py` | Music-link validation, audio defaults and batch cropping |
| `python3 scripts/ui_experience.py` | Toolbar, completed results, focus and responsive recovery |
| `python3 scripts/ui_legacy.py` | Dialog/inert fallback with native APIs removed |
| `python3 scripts/test_macos_compat.py` | Binary compatibility gate |
| `python3 scripts/ui_skin.py` | Skin preview, persistence and reset |
| `python3 scripts/deadcss.py` | CSS selector coverage |

Browser checks require the preview server and Playwright browsers. CI regenerates `FORMATS.md`
and `src/lib/mock-catalog.ts` and rejects drift. Native Windows checks run in `windows.yml`.

## Validation Limits

- Browser tests use a mock backend. Real image/audio/video crop tests use bundled FFmpeg.
  PDF/text selection tests use local fixtures.
- Optional-helper tests skip when their tools are absent. Passing tests do not imply every
  source/target combination or optional helper has been exercised.
- Live music tests in `music_links` are opt-in and excluded from offline CI. Public sample
  success does not prove access to private, paid, regional or future service content.
- Windows core cross-checks on macOS do not validate installation, launch or shell integration.
  Run the Windows workflow and test its installer on Windows before distribution.
- Crop options include locally bundled dialog and inert fallbacks for older WebKit;
  ordinary conversion remains available.
- Rollback handles reported conversion failures and cancellation. Forced process termination,
  device loss and power failure are not transactional. A failed restore retains a recovery file.
- Installer signing and complete FFmpeg corresponding-source review remain release obligations.
  See [Windows](WINDOWS.md), [Releasing](RELEASING.md) and the third-party notices.

Minimum-OS compatibility evidence and remaining blockers are tracked in [COMPATIBILITY.md](COMPATIBILITY.md). Feature-removal tests do not replace an actual Big Sur or Windows 10 launch.
