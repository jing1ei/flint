# Release Flint

Check [release status](RELEASE-STATUS.md) and [compatibility](COMPATIBILITY.md) first.
A successful build does not establish minimum-OS support, signing or FFmpeg source completeness.

## Validate

Run the checks in [Verification](VERIFICATION.md). Test the packaged app on each supported OS:
launch, file chooser, drag/drop, conversion, crop, stop/retry, output-folder access and settings.

Version numbers must agree in `package.json`, `package-lock.json`, `Cargo.toml`, `Cargo.lock`
and `src-tauri/tauri.conf.json`. The desktop crate inherits its version from the workspace.

## Build without publishing

- **macOS:** `npm run build:dmg`, or run **Release** in GitHub Actions with an empty tag.
  The workflow builds Intel and Apple Silicon separately and rejects binaries whose minimum
  OS or architecture conflicts with the bundle configuration.
- **Windows:** `Build-Windows.ps1` or **Windows PC build**. It produces a per-user NSIS installer
  and provisions WebView2. Windows artifacts are not automatically published to the Mac release.

Each distribution needs the application, applicable licenses, exact FFmpeg build information,
complete corresponding source and build scripts for the redistributed components, and checksums.
An upstream FFmpeg tarball alone is not a complete source package for third-party static builds.
Keep private acceptance candidates clearly marked until these requirements are resolved.

## Publish

`Release.command` checks the repository, aligns version files, shows the diff and requires `yes`
before committing, tagging and pushing. Use `RELEASE_DRY_RUN=1 ./Release.command` to inspect the
planned release without publishing. GitHub CLI authentication is required.

Alternatively, push an annotated `v<version>` tag or run **Release** with an existing tag.
The **Auto release** workflow validates manifests and lockfiles, then tags the exact commit
pushed to main. Re-running the workflow resumes an existing tag only when it points to that same
commit; a version tag on another commit is left unchanged. New GitHub Releases remain drafts until
all assets upload successfully. Both routes publish
only after workflow checks pass. Do not push release changes while shipping gates remain open.

Use a Developer ID certificate and notarization for public Mac downloads. Windows installers
should be signed by the publisher. Do not disable OS security protections to make a release work.

## Recovery

Inspect the failing workflow and retain its logs. Do not overwrite an existing release tag or
force-push shared history to repair a failed build. Correct the source and issue a new version.
