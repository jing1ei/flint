# Third-party licences

Flint's own code is proprietary; all rights reserved. Everything else in this document is
somebody else's work, and it falls into three categories that carry genuinely different
obligations. The distinction that matters is **do we hand the user a copy of it?**

| Category | Do we redistribute it? | What that obliges us to do |
| --- | --- | --- |
| [A. Bundled sidecars](#a-what-we-redistribute-ffmpeg-and-ffprobe) — `ffmpeg`, `ffprobe` | **Yes** — inside the `.app`, inside the `.dmg`, attached to every GitHub Release | Ship the full licence text **and** offer the corresponding source. See the obligations below. |
| [B. Optional helper packages](#b-what-the-user-installs-the-seven-helper-packages) — LibreOffice, Pandoc, ImageMagick, Poppler, Ruffle, yt-dlp, Deno | **No** — the user installs them from Homebrew; we detect and spawn them | Nothing beyond attribution. We are a user of these programs, not a distributor of them. |
| [C. Build and runtime dependencies](#c-build-and-runtime-dependencies) — Rust crates, npm packages, Tauri | Partly — compiled/bundled into our own binary | Retain the copyright and permission notices. All permissive. |

This file describes what is true of **this repository as it stands**. It is deliberately not
exhaustive: a general statement that is correct is worth more than a detailed one that is wrong.

---

## A. What we redistribute: FFmpeg and ffprobe

`scripts/fetch-sidecars.sh` downloads two prebuilt static binaries into `src-tauri/binaries/`.
Tauri's `externalBin` copies them into `Flint.app`, so they are inside every `.dmg`
and every `.app.zip` the release workflow attaches to a GitHub Release. **That is redistribution**,
and it is the one place in this project where a licence creates real work for us.

### Which builds, and under which licence

Neither default is our build; both are community builds, and the licence follows the configure flags
the publisher used. The script picks by the requested Rust target triple, so the release workflow — which builds natively
on one Apple Silicon runner and one Intel runner — produces **two artifacts under two different
licences**:

| Artifact | Default source | Relevant configure flags | Licence |
| --- | --- | --- | --- |
| `…-macOS-apple-silicon.dmg` | [osxexperts.net](https://www.osxexperts.net/) (`ffmpeg711arm.zip`, `ffprobe711arm.zip`) | `--enable-gpl`, `--enable-libx264`, `--enable-libx265`, `--enable-libvidstab`, `--enable-libkvazaar`, `--enable-libbluray` — **no** `--enable-version3` | **GPL-2.0-or-later** |
| `…-macOS-intel.dmg` | [evermeet.cx](https://evermeet.cx/ffmpeg/) (`getrelease/ffmpeg/zip`, `getrelease/ffprobe/zip`) | `--enable-gpl` **and** `--enable-version3` (`--enable-libopencore-amrnb/wb`, `--enable-libvo-amrwbenc`, `--enable-libxvid`, `--enable-libx264`, `--enable-libx265`) | **GPL-3.0-or-later** |

FFmpeg itself is LGPL-2.1-or-later at its core. `--enable-gpl` links GPL components (x264, x265,
vid.stab, postproc) into the binary, which makes the whole binary GPL. Adding `--enable-version3`
pulls in components that are (L)GPL **v3**-only, which upgrades the result to GPL-3.0. Both flags
are present in the Intel build; only the first is present in the Apple Silicon build.

One piece of good news: **neither build uses `--enable-nonfree`.** A `--enable-nonfree` FFmpeg may
not be redistributed at all, under any licence, so the defaults being "merely" GPL is what keeps a
public release legally possible. The release workflow re-checks this on every build and aborts if
it ever sees that flag.

The flags in the table above are what those publishers ship *today*. They are not what the release
relies on: the licence for a given release is read off the binary that release actually bundles and
written to `FFMPEG-BUILD.txt` inside the `.app` and next to the download. If this table and that
file ever disagree, **the file is right and this table is stale**.

Upstream source and licence text:

- Source: <https://git.ffmpeg.org/ffmpeg.git> · release tarballs at <https://ffmpeg.org/download.html>
- Licence terms: <https://ffmpeg.org/legal.html> and `LICENSE.md` in the source tree
- The authoritative statement of what *your* copy is: `ffmpeg -version`, which prints the version
  and the full configure line.

### What this obliges us to do, and what the release pipeline now actually does

Distributing a GPL binary is allowed and normal. It is not free of conditions. Every one of these
is now performed by `.github/workflows/release.yml` on each build runner, and the job fails rather
than publishing if any of them cannot be satisfied:

1. **Ship the licence text.** `src-tauri/licenses/` holds the verbatim `GPL-2.0.txt`, `GPL-3.0.txt`
   and `LGPL-2.1.txt`, and `bundle.resources` (`["licenses/*"]` in `src-tauri/tauri.conf.json`)
   copies them into `Flint.app/Contents/Resources/licenses/`. The workflow refuses to
   build if the text for the licence it just detected is missing, and after the build it re-opens
   the `.app` and checks the file is there and byte-identical to the tracked copy.
2. **Record which build it was.** Before bundling, the workflow runs the freshly downloaded sidecar
   and writes `FFMPEG-BUILD.txt` into that same folder: the full `ffmpeg -version` and
   `ffprobe -version` output (version *and* configure line), the SHA-256 of both binaries, the URLs
   they came from, and the resulting verdict — which of the three licence files applies. The
   verdict is derived from the configure line, not from a table in a document, so an overridden
   URL or an upstream change cannot silently invalidate it. `--enable-nonfree` aborts the release.
3. **Offer the corresponding source** (GPL-2.0 §3, GPL-3.0 §6). Each release carries:
   - the matching **upstream FFmpeg source archive** for exactly the version the binary reports —
     `ffmpeg.org/releases` for a numbered release, the upstream commit from git.ffmpeg.org or the
     FFmpeg GitHub mirror for a snapshot — plus upstream's detached `.asc` signature when one
     exists. If it cannot be fetched, the release stops;
   - **`WRITTEN-OFFER.txt`**, a three-year written offer naming the contact route (the repository's
     issue tracker), rendered with the real repository and release URLs;
   - the publisher's complete **configure line**, which is what makes the build reproducible in
     principle from that source.

   Where this is imperfect, we say so in `WRITTEN-OFFER.txt` and in the release notes rather than
   claiming more: we did not compile these binaries, and if a publisher patched their tree on top
   of upstream we do not hold those patches and do not claim the tarball rebuilds the exact binary.
   An honest, documented best effort is defensible; a false claim of compliance is not.
4. **Tell the recipient which of the two licences is theirs.** The release notes carry a generated
   per-architecture table (version, licence, licence file, source archive) built from what the two
   runners measured, and every user can confirm it themselves with
   `"/Applications/Flint.app/Contents/MacOS/ffmpeg" -version`: `--enable-version3` in
   the configure line means GPL-3.0-or-later, `--enable-gpl` without it means GPL-2.0-or-later.

**The application source remains proprietary.** Flint spawns `ffmpeg` as a separate process
over `argv` and a pipe; it does not link against libavcodec and shares no address space with it.
The `.app` is an aggregate of two independent programs, so the GPL attaches to the FFmpeg binary we
pass along, not to the Rust and TypeScript in this repository. Nothing in this paragraph relieves
us of items 1–4 above.

### Two caveats about the Apple Silicon default specifically

Worth knowing before relying on it as the shipping default:

- The publisher's page states the binaries are provided **"for educational purposes only"**, and its
  *License* link serves the **Apache-2.0 licence of the build scripts**, not FFmpeg's GPL. A
  publisher who has not themselves shipped the GPL text is a weak foundation for our own compliance.
- Its stated "source used to compile" points at FFmpeg `release/6.1` while the download it sits
  beside is labelled a much later version. We cannot honour the source obligation by pointing at
  that page.

These two facts are exactly why the release attaches the upstream source itself, records the
publisher's configure line, and carries our own written offer, instead of forwarding a link to the
publisher. They are restated in `WRITTEN-OFFER.txt` so the recipient can weigh them too.

### The alternative, if the obligations are unwelcome — and why it buys less than it looks like

Both URLs are overridable — `FFMPEG_URL`/`FFPROBE_URL` for a local build, and the four repository
variables the release workflow reads. Building FFmpeg **without** `--enable-gpl` and without
`--enable-version3` yields an **LGPL-2.1-or-later** binary. Before treating that as the easy way
out, two corrections to the intuition:

- **It does not remove the source obligation.** LGPL-2.1 §4 attaches to conveying the *object code*
  just as GPL-2.0 §3 does: we would still have to ship the LGPL text and still have to provide the
  corresponding source or a written offer. What LGPL changes is linking — and we never link, we
  spawn a separate process. The paperwork saved is close to zero. What genuinely improves is that
  an LGPL build would be *ours*, so "the corresponding source" becomes a tree and a recipe we can
  publish exactly, which is the one soft spot in the current position.
- **It costs the software H.264/H.265 path.** No `--enable-gpl` means no libx264 and no libx265.
  MP4/H.264 output survives, because `crates/convert-core/src/plan.rs` already defaults to
  `h264_videotoolbox`/`hevc_videotoolbox` (Apple's system encoders, LGPL-compatible) whenever
  hardware acceleration is `Auto`. What disappears is the fallback the app uses when the user turns
  hardware acceleration **off**: CRF rate control, x264 presets and `-profile high`/`-level 4.1`
  tuning, better quality per bit at low bitrates, and HEVC on the older Intel Macs whose hardware
  cannot encode it. Everything else the app can emit — VP9 (libvpx), AV1 (libsvtav1), ProRes, WebP,
  MP3/Opus/Vorbis/AAC — is already LGPL-compatible.

`docs/RELEASING.md` and `src-tauri/licenses/README.md` are where that decision would get recorded.

---

## B. What the user installs: the seven helper packages

These unlock formats FFmpeg cannot do — or, in yt-dlp's and Deno's case, a *source* it cannot
fetch. **We redistribute none of them.** The app looks for them on disk, greys out what it cannot
reach without them, and — if you click Install — runs a Homebrew command that downloads them from
the upstream project to your machine. No bytes of these projects pass through our release, our
repository, or our `.app`.

Because we distribute nothing, their licences do not attach to Flint, and copyleft
among them creates no obligation for us. Attribution is simply the right thing to do:

| Package | Licence (as published upstream) | Upstream |
| --- | --- | --- |
| LibreOffice | MPL-2.0 | <https://www.libreoffice.org/about-us/licenses/> |
| Pandoc | GPL-2.0-or-later | <https://github.com/jgm/pandoc/blob/main/COPYRIGHT> |
| ImageMagick | ImageMagick License (Apache-2.0 derived) | <https://imagemagick.org/script/license.php> |
| Poppler | GPL-2.0-or-later | <https://poppler.freedesktop.org/> |
| Ruffle | MIT **or** Apache-2.0 | <https://github.com/ruffle-rs/ruffle> |
| yt-dlp | Unlicense | <https://github.com/yt-dlp/yt-dlp/blob/master/LICENSE> |
| Deno | MIT | <https://github.com/denoland/deno/blob/main/LICENSE.md> |

Three more programs the app invokes without shipping: **Homebrew** (BSD-2-Clause,
<https://brew.sh>), which performs those installs; **Node.js** (MIT,
<https://github.com/nodejs/node/blob/main/LICENSE>), which we never install but will use as
yt-dlp's JavaScript runtime if you already have it, in place of Deno; and **`sips`**, which is part
of macOS and governed by your Apple software licence agreement. The system **WebKit** that renders
the UI is likewise the operating system's, not ours — Tauri uses it instead of bundling a browser
engine.

If a future version ever *bundles* one of these, it moves to category A and inherits category A's
obligations. GPL-2.0-or-later for Pandoc and Poppler in particular would then apply to the bundle.

---

## C. Build and runtime dependencies

Our own binary is a compiled work that contains other people's code. All of it is permissive
(MIT / Apache-2.0 / BSD-family), so the obligation is to retain the copyright and permission
notices — which is what this section does.

**Inside the shipped app.** Statically linked into the Rust binary, or bundled into the JavaScript
in `dist/`:

| Dependency | Licence |
| --- | --- |
| [Tauri](https://tauri.app) 2 (`tauri`, `tauri-build`, `tauri-plugin-dialog`, `@tauri-apps/api`, `@tauri-apps/plugin-dialog`) | MIT or Apache-2.0 |
| `serde`, `serde_json` | MIT or Apache-2.0 |
| `anyhow`, `thiserror` | MIT or Apache-2.0 |
| [lopdf](https://github.com/J-F-Liu/lopdf) 0.36 (PDF page selection) | MIT; full notice bundled in `src-tauri/licenses/LOPDF-MIT.txt` |
| [React](https://react.dev) 19, `react-dom` | MIT |
| [Zustand](https://github.com/pmndrs/zustand) | MIT |

**Build-time only** — never shipped, present in a contributor's checkout:
TypeScript (Apache-2.0), Vite (MIT), `@vitejs/plugin-react` (MIT), `@tauri-apps/cli` (MIT or
Apache-2.0), the Rust toolchain (MIT or Apache-2.0), Playwright (Apache-2.0, used by the Python
drivers in `scripts/`) and Pillow (MIT-CMU, used only by `scripts/make_icons.py`).

**On the transitive graph, honestly:** Tauri's dependency tree is several hundred crates and the
frontend has its own. We have *not* audited every one, and this file does not claim to. The checks,
for anyone who wants to:

```bash
cargo tree --workspace                # what is actually linked
cargo install cargo-about && cargo about generate   # per-crate licence report
npm ls --all                          # the JS side
```

If that audit turns up anything that is not MIT / Apache-2.0 / BSD-family, it belongs in this file
and it belongs in a release note.

---

## Where the licence text physically lives

| Path | Contents |
| --- | --- |
| Application source | Proprietary; all rights reserved. |
| `src-tauri/licenses/GPL-2.0.txt`, `GPL-3.0.txt`, `LGPL-2.1.txt` | The verbatim FSF texts, as FFmpeg itself distributes them. Tracked in git, copied into the `.app` by `bundle.resources`, and the applicable one is attached to each release. |
| `src-tauri/licenses/WRITTEN-OFFER.txt` | Our written source offer, with its limits stated. Rendered with the real repository/release URLs at release time. |
| `src-tauri/licenses/FFMPEG-BUILD.txt` | **Generated per build, not committed.** The `ffmpeg -version` record of the binary in that specific artifact, and the verdict of which licence applies. Inside the `.app` and attached to the release as `…-macOS-<arch>-FFMPEG-BUILD.txt`. |
| `src-tauri/licenses/README.md` | Why the folder exists, how it is enforced, and the per-architecture difference. |
| Release assets | The applicable GPL text(s), the written offer, both `FFMPEG-BUILD.txt` records, and the matching upstream FFmpeg source archive (with upstream's `.asc` when published) — all covered by `SHA256SUMS.txt`. |
| This file | The map. Not a substitute for the texts above. |

Corrections are welcome as issues — a wrong licence claim is a bug, and it is the kind of bug that
is hardest to fix after publication.

## Browser compatibility dependencies

- `dialog-polyfill` 0.5.6 (The Chromium Authors), BSD-3-Clause: provides crop dialogs where native `showModal` is absent. Full notice: [DIALOG-POLYFILL-BSD.txt](src-tauri/licenses/DIALOG-POLYFILL-BSD.txt).
- `wicg-inert` 3.1.3, W3C Software and Document License: keeps covered controls out of keyboard focus on older WebKit. Full notice: [WICG-INERT-W3C.txt](src-tauri/licenses/WICG-INERT-W3C.txt).

Both are bundled locally. No polyfill code is fetched from a CDN at runtime.

## MIDI parser

`midly` 0.5.3 is distributed under the Unlicense. It parses MIDI events; Flint’s basic piano
synthesis uses original procedural audio and includes no sampled instruments or soundfont.
Source: https://github.com/negamartin/midly
