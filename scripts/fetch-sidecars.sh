#!/bin/sh
#
# fetch-sidecars.sh - download the static FFmpeg + ffprobe binaries Flint ships.
#
# Tauri's `externalBin` expects the files to be named `<name>-<rust target triple>` inside
# `src-tauri/binaries/`, e.g. `ffmpeg-aarch64-apple-darwin`. The triple is detected with
# `rustc -vV`, so cross-compiling only means running this script with a different toolchain
# default (or setting TARGET_TRIPLE yourself).
#
#   ./scripts/fetch-sidecars.sh            # download whatever is missing
#   ./scripts/fetch-sidecars.sh --check    # report only, download nothing
#   FORCE=1 ./scripts/fetch-sidecars.sh    # re-download even if the binaries are already there
#   FFMPEG_URL=... FFPROBE_URL=... ./scripts/fetch-sidecars.sh
#   SIDECAR_CACHE_DIR=... ./scripts/fetch-sidecars.sh
# Partial archives survive failures; running again resumes them.
#
# ---------------------------------------------------------------------------------------------
# LICENCE, READ THIS BEFORE YOU SHIP
# ---------------------------------------------------------------------------------------------
# FFmpeg remains a separate process with its own licence. Redistributing either GPL or LGPL
# binaries requires the applicable notices and corresponding source; our application stays MIT.
#
# Review the publisher's source evidence before distribution. You do not have to
# supply any licence text yourself: `src-tauri/licenses/` already carries the verbatim GPL-2.0,
# GPL-3.0 and LGPL-2.1 texts and a written source offer, and the release workflow works out which
# licence the build you shipped is under from its own configure line, records that in a generated
# FFMPEG-BUILD.txt, and attaches the applicable licence and the matching upstream FFmpeg source
# tarball to the release (see the README there).
# ---------------------------------------------------------------------------------------------

set -eu
# pipefail is not in POSIX; enable it where the shell supports it (bash/zsh, i.e. macOS /bin/sh).
# shellcheck disable=SC3040
(set -o pipefail 2>/dev/null) && set -o pipefail

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH='' cd -- "$SCRIPT_DIR/.." && pwd)
BIN_DIR="$PROJECT_ROOT/src-tauri/binaries"

CHECK_ONLY=0
[ "${1:-}" = "--check" ] && CHECK_ONLY=1
if [ -n "${1:-}" ] && [ "$1" != "--check" ]; then
  echo "$0: unrecognised argument '$1'" >&2
  echo "usage: $0 [--check]" >&2
  # A leading '#' means a pasted comment was parsed as an argument: zsh (the default macOS shell)
  # only strips '#' in interactive shells when `setopt interactive_comments` is on. Worth naming,
  # because the same paste also makes `npm install  # deps` fail with EINVALIDTAGNAME.
  if [ "$1" = "#" ]; then
    echo >&2
    echo "It looks like you pasted a trailing '# comment' along with the command." >&2
    echo "Interactive zsh does not treat '#' as a comment, so it became an argument." >&2
    echo "Re-run the command on its own:" >&2
    echo "    $0" >&2
  fi
  exit 2
fi

die() {
  echo "" >&2
  echo "  ✗ $1" >&2
  [ -n "${2:-}" ] && echo "    $2" >&2
  exit 1
}

info() { echo "  $1"; }

# --------------------------------------------------------------------------------------------
# Which platform are we building for?
# --------------------------------------------------------------------------------------------
if command -v rustc >/dev/null 2>&1; then
  DETECTED_TRIPLE=$(rustc -vV | sed -n 's/^host: //p')
else
  DETECTED_TRIPLE=""
fi
TARGET_TRIPLE="${TARGET_TRIPLE:-$DETECTED_TRIPLE}"
[ -n "$TARGET_TRIPLE" ] || die "could not determine the Rust target triple" \
  "install Rust (https://rustup.rs) or set TARGET_TRIPLE=aarch64-apple-darwin yourself."

OS=$(uname -s)
ARCH=$(uname -m)

# Defaults are the two builds the macOS community keeps current:
#   * evermeet.cx  - Intel (x86_64) snapshots, one binary per download
#   * osxexperts   - Apple Silicon (arm64) builds
# Both are GPL builds. Override the URLs to point at an LGPL build you produced yourself.
DEFAULT_FFMPEG_URL=""
DEFAULT_FFPROBE_URL=""
case "$TARGET_TRIPLE" in
  aarch64-apple-darwin)
    DEFAULT_FFMPEG_URL="https://www.osxexperts.net/ffmpeg711arm.zip"
    DEFAULT_FFPROBE_URL="https://www.osxexperts.net/ffprobe711arm.zip"
    ;;
  x86_64-apple-darwin)
    DEFAULT_FFMPEG_URL="https://evermeet.cx/ffmpeg/getrelease/ffmpeg/zip"
    DEFAULT_FFPROBE_URL="https://evermeet.cx/ffmpeg/getrelease/ffprobe/zip"
    ;;
esac

FFMPEG_URL="${FFMPEG_URL:-$DEFAULT_FFMPEG_URL}"
FFPROBE_URL="${FFPROBE_URL:-$DEFAULT_FFPROBE_URL}"

# --------------------------------------------------------------------------------------------
# --check: report and exit
# --------------------------------------------------------------------------------------------
read_version() {
  # Capture the command itself before selecting its first line. A pipeline would
  # report head's success even when the binary printed a banner and then failed.
  version_output=$("$1" -version 2>/dev/null) || return 1
  version=$(printf '%s\n' "$version_output" | head -n 1)
  [ -n "$version" ]
}

report_one() {
  name=$1
  path="$BIN_DIR/$name-$TARGET_TRIPLE"
  if [ ! -f "$path" ]; then
    echo "  ✗ $name-$TARGET_TRIPLE   missing"
    return 1
  fi
  if [ ! -x "$path" ]; then
    echo "  ✗ $name-$TARGET_TRIPLE   present but not executable (chmod +x it)"
    return 1
  fi
  if ! read_version "$path"; then
    echo "  ✗ $name-$TARGET_TRIPLE   present but does not run (wrong architecture? quarantined?)"
    return 1
  fi
  echo "  ✓ $name-$TARGET_TRIPLE   $version"
  return 0
}

echo "Flint · FFmpeg sidecars"
echo "  target triple : $TARGET_TRIPLE"
echo "  destination   : $BIN_DIR"
echo ""

if [ "$CHECK_ONLY" -eq 1 ]; then
  status=0
  report_one ffmpeg || status=1
  report_one ffprobe || status=1
  echo ""
  if [ "$status" -eq 0 ]; then
    echo "Both sidecars are ready. Next: npm run tauri dev"
  else
    echo "Run ./scripts/fetch-sidecars.sh to download the missing binaries."
  fi
  exit "$status"
fi

[ -n "$FFMPEG_URL" ] && [ -n "$FFPROBE_URL" ] || die \
  "no default download URL for $OS/$ARCH" \
  "set FFMPEG_URL and FFPROBE_URL to static builds for this platform, e.g. FFMPEG_URL=https://…/ffmpeg.zip"

command -v curl >/dev/null 2>&1 || die "curl is required but was not found on PATH"

mkdir -p "$BIN_DIR"
CACHE_DIR="${SIDECAR_CACHE_DIR:-${XDG_CACHE_HOME:-${HOME:-/tmp}/Library/Caches}/flint/sidecars}"
mkdir -p "$CACHE_DIR"
DOWNLOAD_TIMEOUT="${SIDECAR_DOWNLOAD_TIMEOUT:-900}"
case "$DOWNLOAD_TIMEOUT" in
  ''|*[!0-9]*|0) die "SIDECAR_DOWNLOAD_TIMEOUT must be a positive number of seconds" ;;
esac

# --------------------------------------------------------------------------------------------
# Download + install one binary
# --------------------------------------------------------------------------------------------
fetch_one() (
  name=$1
  url=$2
  env_var=$3 # the variable a user overrides to change this download
  dest="$BIN_DIR/$name-$TARGET_TRIPLE"

  # Idempotent: a working binary is left alone unless FORCE=1.
  if [ "${FORCE:-0}" != "1" ] && [ -x "$dest" ] && read_version "$dest"; then
    info "✓ $name already installed ($version)"
    return 0
  fi

  key=$(printf '%s\n%s\n%s\n' "$name" "$TARGET_TRIPLE" "$url" | shasum -a 256 | cut -d' ' -f1)
  archive="$CACHE_DIR/$key.archive"
  partial="$CACHE_DIR/$key.part"
  lock="$CACHE_DIR/$key.lock"
  mkdir "$lock" 2>/dev/null || die "another download may be using $name's cache" \
    "Close the other build first. If none is running, remove this stale lock directory: $lock"
  tmp=""
  trap 'rm -rf "$tmp"; rmdir "$lock" 2>/dev/null || true' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/cc-sidecar-XXXXXX") || die "could not create a temp directory"

  info "↓ $name  <-  $url"
  if [ "${FORCE:-0}" = "1" ]; then
    rm -f "$archive" "$partial"
  fi
  if [ ! -s "$archive" ]; then
    attempt=1
    while [ "$attempt" -le 4 ]; do
      if [ -s "$partial" ]; then
        info "resuming $name from $(wc -c < "$partial" | tr -d ' ') bytes (attempt $attempt/4)"
      fi
      # Retry ourselves: curl's built-in retries rewind output instead of retaining progress.
      if curl --fail --location --continue-at - --connect-timeout 20 \
        --max-time "$DOWNLOAD_TIMEOUT" --speed-limit 1024 --speed-time 60 \
        --progress-bar --write-out '%{http_code}' --output "$partial" "$url" > "$tmp/http-status"; then
        status=0
      else
        status=$?
      fi
      # A server that ignores Range cannot resume. Retry from zero, with the larger time budget.
      # Some curl versions return success on 416, assuming the local file is already complete.
      if [ "$(cat "$tmp/http-status")" = "416" ]; then
        info "cached range no longer matches the server; restarting this archive"
        rm -f "$partial"
      elif [ "$status" -eq 0 ]; then
        mv "$partial" "$archive"
        break
      elif [ "$status" -eq 33 ]; then
        info "server does not support resuming; restarting this archive"
        rm -f "$partial"
      elif [ "$status" -eq 22 ]; then
        die "the server refused the $name request" \
          "Use another $env_var URL or a file:/// archive. Cached partial: $partial"
      fi
      attempt=$((attempt + 1))
      [ "$attempt" -gt 4 ] || sleep 2
    done
    [ -s "$archive" ] || die "download interrupted for $name; partial data was kept" \
      "Run ./scripts/fetch-sidecars.sh again to resume. Or set $env_var=file:///absolute/path/to/archive.zip. Cache: $partial"
  else
    info "using cached $name archive"
  fi

  [ -s "$archive" ] || die "downloaded an empty file for $name" "URL: $url"

  # The publishers ship .zip; .tar.xz/.tar.gz are supported so a custom LGPL build works too.
  kind=$(file -b "$archive" 2>/dev/null || echo unknown)
  extract="$tmp/extract"
  mkdir -p "$extract"
  case "$kind" in
    *Zip*|*ZIP*)
      command -v unzip >/dev/null 2>&1 || die "unzip is required to unpack $name"
      if ! unzip -q -o "$archive" -d "$extract"; then
        rm -f "$archive"
        die "the $name archive is incomplete or corrupt; it was removed from cache" \
          "Run this script again or supply another $env_var URL."
      fi
      ;;
    *XZ*|*gzip*|*tar*)
      if ! tar -xf "$archive" -C "$extract"; then
        rm -f "$archive"
        die "the $name archive is incomplete or corrupt; it was removed from cache"
      fi
      ;;
    *Mach-O*|*ELF*)
      cp "$archive" "$extract/$name" # already a bare binary
      ;;
    *)
      rm -f "$archive"
      die "unrecognised archive type for $name ($kind)" "URL: $url"
      ;;
  esac

  found=$(find "$extract" -type f -name "$name" -perm -u+x 2>/dev/null | head -n 1)
  [ -n "$found" ] || found=$(find "$extract" -type f -name "$name" 2>/dev/null | head -n 1)
  [ -n "$found" ] || die "the $name archive did not contain a binary called '$name'" \
    "unpacked into $extract - inspect it and adjust the URL"

  chmod +x "$found"
  # Gatekeeper flags anything downloaded; without this the app cannot spawn the sidecar.
  command -v xattr >/dev/null 2>&1 && xattr -dr com.apple.quarantine "$found" 2>/dev/null || true

  read_version "$found" || die "$name cannot be installed because it does not run" \
    "wrong architecture for $TARGET_TRIPLE? Try FORCE=1 with a different $env_var."
  mv "$found" "$dest" || die "could not write $dest"
  info "✓ $name  ->  $dest"
  info "   $version"

)

fetch_one ffmpeg "$FFMPEG_URL" FFMPEG_URL
fetch_one ffprobe "$FFPROBE_URL" FFPROBE_URL

cat <<'NEXT'

Done. Both sidecars are in src-tauri/binaries/ and are ignored by git.

  Licence: nothing to copy in by hand. src-tauri/licenses/ already holds the GPL-2.0,
  GPL-3.0 and LGPL-2.1 texts and a written source offer, and all of it is bundled into
  the .app. The release workflow reads the configure line of the build you shipped, writes
  what it found to FFMPEG-BUILD.txt, and attaches the licence that applies along with the
  matching upstream FFmpeg source - see src-tauri/licenses/README.md before distributing.

Next steps:
  npm install          # once
  npm run tauri dev    # run the app
  npm run tauri build  # produce a .dmg / .app
NEXT
