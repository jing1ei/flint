# Release Flint

Every release is an immutable Git tag. Check [release status](RELEASE-STATUS.md) and
[compatibility](COMPATIBILITY.md) first: a successful build does not establish minimum-OS support,
signing or FFmpeg source completeness.

## Validate

Run the checks in [Verification](VERIFICATION.md) (they are the same gates the release workflow
runs), and test the packaged app on each supported OS: launch, file chooser, drag/drop,
conversion, crop, stop/retry, output-folder access and settings.

Version numbers must agree in `package.json`, `package-lock.json`, `Cargo.toml`, `Cargo.lock`
and `src-tauri/tauri.conf.json`. `scripts/release_version.py` enforces this; `Release.command`
keeps the files aligned automatically.

## Cut the release

```sh
./Release.command                                  # guided: align, commit, tag, push
RELEASE_DRY_RUN=1 ./Release.command                # preview without committing or pushing
git tag -a v1.2.3 -m "Flint 1.2.3" && git push origin v1.2.3   # equivalent, by hand
```

A tag push starts the **Release** workflow. It runs the complete CI suite from
`.github/workflows/ci.yml` on the tagged commit, builds Intel and Apple Silicon packages
natively, calls the **Windows PC build** workflow, and only then publishes:

- both `.dmg` files and both `.app.zip` files;
- the Windows x64 NSIS installer;
- the per-architecture `FFMPEG-BUILD.txt` records, the applicable GPL texts and `WRITTEN-OFFER.txt`;
- the matching upstream FFmpeg source archive per platform;
- one consolidated `SHA256SUMS.txt` covering every asset.

Every asset is downloaded back from GitHub and checksummed before the draft release is published.
A failed run leaves a draft (or no release) that can be retried from the same tag.

## If something fails

Fix the source, bump the version, and push a new tag. The publisher refuses to modify an
already-published tag and never moves a tag to another commit. Do not delete and re-push a tag
that anyone may already have downloaded.

## Building without publishing

- **macOS:** `npm run build:dmg` builds for the host architecture only. For the other Mac
  architecture use a native runner (or the tagged release workflow).
- **Windows:** run `Build-Windows.ps1` on Windows; prerequisites are in [Windows](WINDOWS.md).

Each distribution needs the application, applicable licences, exact FFmpeg build information,
complete corresponding source for the redistributed components, and checksums. An upstream
FFmpeg tarball alone is not a complete source package for third-party static builds.

Use a Developer ID certificate and notarization for public Mac downloads; sign the Windows
installer with a publisher certificate. Do not disable OS security protections to make a
release work.
