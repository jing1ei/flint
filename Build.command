#!/bin/bash
#
# Build.command - double-click this file in Finder to build Flint.
#
# It does every step in order and stops at the first real problem with an explanation:
#   1. checks Xcode command line tools, Node and Rust
#   2. installs the frontend dependencies if they are missing
#   3. downloads the FFmpeg sidecars if they are missing   <- the step that is easiest to forget
#   4. builds the macOS .app
#   5. opens the folder it landed in
#
# Set DMG=1 to also build the .dmg installer (see the note where that happens).
#
# Windows: see the section at the end. A Windows app cannot be built on a Mac, so it is not a step
# this script can perform - it explains what to do instead rather than pretending.

# Deliberately not set -e: every failure below is reported by hand, with the fix, and then `pause`
# holds the Finder window open long enough to read it. An -e exit would skip that pause and the
# window would vanish with the reason in it - the one failure mode a double-clicker cannot recover
# from.
set -uo pipefail

# Finder runs a double-clicked .command from the user's home directory, not from the file's own
# folder, so every relative path below would miss without this.
cd "$(dirname "$0")" || exit 1

BOLD=$'\033[1m'
DIM=$'\033[2m'
RED=$'\033[31m'
GREEN=$'\033[32m'
YELLOW=$'\033[33m'
OFF=$'\033[0m'

step() { printf '\n%s==> %s%s\n' "$BOLD" "$1" "$OFF"; }
note() { printf '%s    %s%s\n' "$DIM" "$1" "$OFF"; }
fail() { printf '\n%s✗ %s%s\n' "$RED" "$1" "$OFF"; }
good() { printf '%s✓ %s%s\n' "$GREEN" "$1" "$OFF"; }

# Double-clicked windows close the instant the script ends, taking the output with them.
pause() {
  printf '\n%sPress Return to close this window.%s ' "$DIM" "$OFF"
  read -r _ || true
}

die() {
  fail "$1"
  shift
  for line in "$@"; do printf '    %s\n' "$line"; done
  pause
  exit 1
}

printf '%s\n' "$BOLD"
printf 'Flint - build\n'
printf '%s' "$OFF"
note "$(pwd)"

# ---------------------------------------------------------------------------------------------
# 1. Prerequisites. Checked all at once so you fix everything in one trip, not one per run.
# ---------------------------------------------------------------------------------------------
step "Checking prerequisites"

missing=""
xcode-select -p >/dev/null 2>&1 || missing="$missing xcode"
command -v node >/dev/null 2>&1 || missing="$missing node"
command -v cargo >/dev/null 2>&1 || missing="$missing rust"

if [ -n "$missing" ]; then
  fail "Some tools this build needs are not installed yet."
  case "$missing" in
  *xcode*) printf '\n  Xcode command line tools (compilers and linkers):\n      xcode-select --install\n' ;;
  esac
  case "$missing" in
  *node*) printf '\n  Node 20 or newer (the frontend build):\n      brew install node\n' ;;
  esac
  case "$missing" in
  *rust*) printf '\n  Rust (the app itself):\n      curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh\n' ;;
  esac
  printf '\n  Install what is listed, open a new terminal window, then run this again.\n'
  pause
  exit 1
fi

good "Xcode tools, Node $(node --version), $(cargo --version | cut -d' ' -f1-2)"

# ---------------------------------------------------------------------------------------------
# 2. Frontend dependencies. `tauri` itself lives in node_modules, so nothing works without these.
# ---------------------------------------------------------------------------------------------
step "Frontend dependencies"

  note "running npm ci with the checked-in lockfile"
  npm ci || die "npm ci failed." \
    "Scroll up for the reason. Retry npm ci after checking the network and Node version." \
    "Keep package-lock.json; it pins the versions used by GitHub Actions."
  good "installed"

# ---------------------------------------------------------------------------------------------
# 3. FFmpeg sidecars. THE most common failure: they are ~140 MB of downloaded binaries, so they are
#    deliberately not in source control - which means a fresh copy of the source never has them, and
#    re-downloading the source loses them again. Hence doing it here rather than asking you to.
# ---------------------------------------------------------------------------------------------
step "FFmpeg engine"

if ./scripts/fetch-sidecars.sh --check >/dev/null 2>&1; then
  good "already present"
else
  note "downloading ffmpeg + ffprobe (~140 MB, once)"
  ./scripts/fetch-sidecars.sh || die "Could not download the FFmpeg sidecars." \
    "Partial downloads are cached. Run this again to resume instead of starting over." \
    "A browser-downloaded archive also works: FFPROBE_URL=file:///absolute/path/ffprobe.zip ./scripts/fetch-sidecars.sh" \
    "To see what is already there:" \
    "    ./scripts/fetch-sidecars.sh --check"
  good "downloaded"
fi

# ---------------------------------------------------------------------------------------------
# 4. Build.
#
#    Default is the .app alone. The .dmg step (bundle_dmg.sh) drives Finder over AppleScript, so it
#    needs a GUI session and permission to control Finder, and fails for reasons that have nothing
#    to do with the app - after the .app is already built. A disk image is only for handing the app
#    to someone else, so it is opt-in: DMG=1 ./Build.command
# ---------------------------------------------------------------------------------------------
if [ "${DMG:-0}" = "1" ]; then
  step "Building the app and disk image"
  target="dmg"
else
  step "Building the app"
  target="app"
fi
note "first build takes 5-10 minutes; later ones are much faster"

if ! npm run "build:$target"; then
  if [ "$target" = "dmg" ] && [ -d "target/release/bundle/macos" ]; then
    printf '\n%s! The app built, but the disk image step failed.%s\n' "$YELLOW" "$OFF"
    note "bundle_dmg.sh needs a GUI session and permission to control Finder:"
    note "System Settings > Privacy & Security > Automation > your terminal > Finder"
    note "A volume left mounted from an earlier attempt also breaks it:"
    note "    ls /Volumes    then    hdiutil detach \"/Volumes/Flint\""
  else
    die "The build failed." \
      "The last lines above say why. If it mentions missing sidecars, run this script again -" \
      "it downloads them. Anything else, send those lines on."
  fi
fi

# ---------------------------------------------------------------------------------------------
# 5. Show the result.
# ---------------------------------------------------------------------------------------------
app="target/release/bundle/macos/Flint.app"

if [ -d "$app" ]; then
  step "Done"
  good "$app"
  printf '\n    %sFirst launch:%s the app is unsigned, so double-clicking shows a warning.\n' "$BOLD" "$OFF"
  printf '    After the first launch attempt, use System Settings > Privacy & Security > Open Anyway.\n'
  printf '\n    %sUsing Safari sign-ins?%s macOS ties Full Disk Access to each build, so this\n' "$BOLD" "$OFF"
  printf '    fresh copy needs it again: switch it off and on in System Settings >\n'
  printf '    Privacy & Security > Full Disk Access, then open the app.\n'
  [ "$target" = "dmg" ] && [ -d target/release/bundle/dmg ] && note "installer: target/release/bundle/dmg/"
  open target/release/bundle/macos 2>/dev/null || true
else
  die "The build reported success but no .app is there." \
    "Look for errors above, or send the output on."
fi

# ---------------------------------------------------------------------------------------------
# Windows.
# ---------------------------------------------------------------------------------------------
printf '\n%s==> Windows%s\n' "$BOLD" "$OFF"
printf '    A Windows app cannot be built on a Mac: it needs the MSVC toolchain and\n'
printf '    WebView2, and the installer format (NSIS/MSI) is built by Windows tooling.\n'
printf '    Cross-compiling is not something Tauri supports here, so this script does\n'
printf '    not perform that build. Run the "Windows PC build" GitHub Actions workflow,\n'
printf '    or Build-Windows.ps1 on Windows. See docs/WINDOWS.md.\n'

pause
