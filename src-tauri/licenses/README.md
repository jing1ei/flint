# Licences shipped inside the app bundle

`bundle.resources` in `src-tauri/tauri.conf.json` is `["licenses/*"]`, so **every file in this
folder is copied into `Flint.app/Contents/Resources/licenses/`** and therefore into
every `.dmg` and `.app.zip` a release attaches. Windows bundles also include these resources.

## Why this folder exists

Flint ships prebuilt `ffmpeg` and `ffprobe` binaries as Tauri `externalBin` sidecars
(downloaded at build time by `scripts/fetch-sidecars.sh`). Those binaries are **GPL**. Putting them
in a `.dmg` and attaching it to a GitHub Release is redistribution of a GPL work, which obliges us
to hand the recipient the licence text and to make the corresponding source available. Our own code
remains proprietary: the app *spawns* FFmpeg as a separate process over argv and a pipe, never links it, so
the bundle is mere aggregation of two independent programs.

## What is in here

| File | Tracked in git? | What it is |
| --- | --- | --- |
| `GPL-2.0.txt` | yes | Verbatim GNU GPL version 2, June 1991. 18 092 bytes, SHA-256 `8177f975…b880643`. |
| `GPL-3.0.txt` | yes | Verbatim GNU GPL version 3, 29 June 2007. 35 147 bytes, SHA-256 `8ceb4b9e…eb65b903`. |
| `LGPL-2.1.txt` | yes | Verbatim GNU LGPL version 2.1, February 1999. 26 517 bytes, SHA-256 `246041b6…ab189c30`. |
| `LOPDF-MIT.txt` | yes | MIT copyright and permission notice for lopdf, used for PDF page selection. |
| `WRITTEN-OFFER.txt` | yes | Our written offer for the corresponding source (GPL-2.0 §3(b) / GPL-3.0 §6(b)), including a plain statement of where that position is imperfect. The release workflow renders its `__PLACEHOLDER__`s with the real repository and release URLs. |
| `FFMPEG-BUILD.txt` | **no — generated** | Written by `.github/workflows/release.yml` on the build runner from the sidecar that is about to be bundled: the full `ffmpeg -version` and `ffprobe -version` output (version **and** configure line), the SHA-256 of both binaries, the download URLs used, and the resulting verdict of which licence file above applies. |
| `README.md` | yes | This file. It ships too; that is deliberate. |

The three licence texts are the copies FFmpeg itself distributes (`COPYING.GPLv2`, `COPYING.GPLv3`,
`COPYING.LGPLv2.1` in the FFmpeg source tree), which are byte-identical to the same FSF texts as
shipped by other projects — GPL-2.0 matches VLC's `COPYING`, GPL-3.0 matches GCC's `COPYING3`. They
differ from today's gnu.org downloads only in cosmetic details the FSF has since edited (`http:` vs
`https:` links, the FSF postal address, the name in the example signature). Shipping the copy that
travels with FFmpeg is the defensible choice: it is the licence file of the work we redistribute.

`FFMPEG-BUILD.txt` is generated, not committed. If you ever produce one locally it will show up as
an untracked file — don't commit it, it describes your machine's download, not a release.

## Which licence applies is per architecture — the part that surprises people

`scripts/fetch-sidecars.sh` picks the download from `uname -m`, and the two community publishers
configure FFmpeg differently:

| Artifact | Publisher | Relevant flags | Licence | Text that applies |
| --- | --- | --- | --- | --- |
| `…-macOS-apple-silicon.*` | osxexperts.net | `--enable-gpl` (x264, x265, vid.stab), **no** `--enable-version3` | GPL-2.0-or-later | `GPL-2.0.txt` |
| `…-macOS-intel.*` | evermeet.cx | `--enable-gpl` **and** `--enable-version3` | GPL-3.0-or-later | `GPL-3.0.txt` |

So a single release ships two artifacts under two different licences. That is why both texts are
here and why `FFMPEG-BUILD.txt` exists: it is the per-download record that tells a recipient which
of the two they actually got, rather than making them guess from the file name.

`LGPL-2.1.txt` is here because FFmpeg's core is LGPL-2.1-or-later even inside a GPL build (the GPL
components are what make the *combined* binary GPL), and because the `FFMPEG_URL`/`FFPROBE_URL`
overrides make a genuine LGPL-only build a supported configuration. If you take that route,
`FFMPEG-BUILD.txt` will say so and point at `LGPL-2.1.txt`.

Neither default is configured `--enable-nonfree`. A `--enable-nonfree` build may not be
redistributed at all, under any licence, and the release workflow now fails hard if it ever sees
that flag.

## How this is enforced, not just documented

In `.github/workflows/release.yml`, on each build runner:

1. `Record the exact FFmpeg build…` runs the freshly downloaded sidecar, writes `FFMPEG-BUILD.txt`
   into this folder **before** `npm run build:dmg`, decides GPL-2.0 / GPL-3.0 / LGPL-2.1 from the
   configure line, aborts on `--enable-nonfree`, and aborts if the licence text it just decided on
   is missing from this folder.
2. `Fetch the matching upstream FFmpeg source…` downloads the upstream tarball for exactly that
   version (release tarball from ffmpeg.org, or the upstream commit for a snapshot build) and
   attaches it to the release; the job fails if it cannot get it.
3. `The licence files really are inside the .app` re-opens the built bundle and greps
   `Contents/Resources/licenses/` for the applicable text, `WRITTEN-OFFER.txt` and
   `FFMPEG-BUILD.txt`. A bundle that lost them does not ship.

## The seven helper packages are *not* redistributed

LibreOffice, Pandoc, ImageMagick, Poppler, Ruffle, yt-dlp and Deno are **detected and spawned,
never bundled**; the app can offer to install them with Homebrew, which downloads them from their
own projects onto the user's machine. No bytes of theirs pass through this repository, this folder
or a release asset, so their licences create no obligation for us and none of their texts belong
here. Node.js is in the same position one step further out: the app will use it as yt-dlp's
JavaScript runtime if the machine already has it, and never installs it.
Attribution for all of them lives in [`THIRD-PARTY-LICENSES.md`](../../THIRD-PARTY-LICENSES.md).

(An earlier version of this file listed Ghostscript. It is not one of our packages and never was.)

The browser fallbacks also ship `DIALOG-POLYFILL-BSD.txt` and `WICG-INERT-W3C.txt`.
