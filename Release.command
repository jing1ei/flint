#!/bin/bash
#
# Release.command - double-click this file in Finder to publish a new release of
# Flint on GitHub.
#
# What it does, in order, stopping at the first problem with an explanation:
#   1. checks git, the GitHub CLI (installed AND logged in), and perl
#   2. checks the repository: an origin remote on GitHub, the right branch, a clean tree,
#      nothing waiting to be pulled
#   3. asks for the version number, defaulting to a sensible next one, and refuses one that is
#      already tagged
#   4. writes that version into every file that carries it - package.json, package-lock.json,
#      src-tauri/tauri.conf.json, Cargo.toml, Cargo.lock - so they cannot drift
#   5. shows you exactly what it is about to do and waits for you to type "yes"
#   6. commits, tags, pushes, then watches the GitHub Actions run and opens the release page
#
# Nothing before step 5 touches GitHub, and nothing before step 5 cannot be undone.
# The build itself happens on GitHub (both Mac architectures) - see docs/RELEASING.md.
#
#   ./Release.command              # normal, interactive
#   RELEASE_DRY_RUN=1 ./Release.command   # do everything except commit, tag and push
#   VERSION=0.2.0 ./Release.command       # skip the version question
#
# Deliberately not set -e: every failure below is reported by hand, with the fix.
set -uo pipefail

# Finder runs a double-clicked .command from the user's home directory, not from the file's own
# folder, so every relative path below would miss without this.
cd "$(dirname "$0")" || exit 1

printf '\033]0;Flint - release\007' # name the Terminal window

BOLD=$'\033[1m'
DIM=$'\033[2m'
RED=$'\033[31m'
GREEN=$'\033[32m'
YELLOW=$'\033[33m'
OFF=$'\033[0m'

step() { printf '\n%s==> %s%s\n' "$BOLD" "$1" "$OFF"; }
note() { printf '%s    %s%s\n' "$DIM" "$1" "$OFF"; }
good() { printf '%s ✓ %s%s\n' "$GREEN" "$1" "$OFF"; }
warn() { printf '%s ! %s%s\n' "$YELLOW" "$1" "$OFF"; }
fail() { printf '\n%s ✗ %s%s\n' "$RED" "$1" "$OFF"; }

DRY_RUN=${RELEASE_DRY_RUN:-0}
BACKUP_DIR="" # set once the version files are about to change

usage() {
  sed -n '3,22p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
}
case "${1:-}" in
-h | --help | help) usage ;;
"") ;;
*)
  fail "unrecognised argument '$1'"
  printf '    Run it with no arguments, or --help.\n'
  exit 2
  ;;
esac

# Double-clicked windows close the instant the script ends, taking the output with them.
pause() {
  printf '\n%sPress Return to close this window.%s ' "$DIM" "$OFF"
  read -r _ || true
}

# Put the version files back exactly as they were. Used when you decline the confirmation and
# whenever something fails before the commit.
restore_backup() {
  [ -n "$BACKUP_DIR" ] && [ -d "$BACKUP_DIR" ] || return 0
  local rel
  while IFS= read -r rel; do
    [ -n "$rel" ] || continue
    cp "$BACKUP_DIR/$(printf '%s' "$rel" | tr / _)" "$rel" 2>/dev/null
  done <"$BACKUP_DIR/manifest"
  note "the version files were put back the way they were"
}

die() {
  fail "$1"
  shift
  for line in "$@"; do printf '    %s\n' "$line"; done
  restore_backup
  pause
  exit 1
}

trap 'printf "\n"; warn "interrupted"; restore_backup; pause; exit 130' INT

# read_answer PROMPT [DEFAULT] -> sets ANSWER
ANSWER=""
read_answer() {
  local prompt=$1 default=${2:-}
  printf '\n%s%s%s ' "$BOLD" "$prompt" "$OFF"
  if ! read -r ANSWER; then
    ANSWER="" # end of input (piped, or the window was closed): fall back to the default
    printf '\n'
  fi
  [ -n "$ANSWER" ] || ANSWER=$default
}

printf '%s\n' "$BOLD"
printf 'Flint - release\n'
printf '%s' "$OFF"
note "$(pwd)"
[ "$DRY_RUN" = "1" ] && warn "RELEASE_DRY_RUN=1: nothing will be committed, tagged or pushed"

# =============================================================================================
# 1. Tools
# =============================================================================================
step "Checking the tools this needs"

command -v git >/dev/null 2>&1 || die "git is not installed." \
  "Install Apple's command line tools, then run this again:" \
  "    xcode-select --install"

command -v perl >/dev/null 2>&1 || die "perl is not installed." \
  "It ships with macOS, so something is unusual about this machine." \
  "Install Apple's command line tools and try again:  xcode-select --install"

if ! command -v gh >/dev/null 2>&1; then
  die "the GitHub CLI (gh) is not installed." \
    "It is what creates the release. Install it with Homebrew:" \
    "    brew install gh" \
    "" \
    "No Homebrew? Get it from https://cli.github.com (there is a .pkg installer)," \
    "then open a new Terminal window and run this script again."
fi

if ! gh auth status >/dev/null 2>&1; then
  die "the GitHub CLI is installed but not logged in." \
    "Log in once - it opens your browser and takes a minute:" \
    "    gh auth login" \
    "" \
    "Choose: GitHub.com  ->  HTTPS  ->  authenticate with a browser." \
    "Then run this script again. Check it worked with:  gh auth status"
fi

good "git $(git --version | awk '{print $3}'), gh $(gh --version | head -n 1 | awk '{print $3}') (logged in)"

# =============================================================================================
# 2. The repository
# =============================================================================================
step "Checking the repository"

toplevel=$(git rev-parse --show-toplevel 2>/dev/null)
[ -n "$toplevel" ] || die "this folder is not a git repository." \
  "Release.command has to sit in the root of the Flint checkout." \
  "If you downloaded a .zip of the source rather than cloning it, there is no git" \
  "history to release from - clone the repository instead:" \
  "    git clone https://github.com/YOUR-USER/flint.git"

if [ "$toplevel" != "$(pwd -P)" ]; then
  die "Release.command is not in the root of the repository." \
    "It is in : $(pwd -P)" \
    "Root is  : $toplevel" \
    "Move it to the root and run it from there."
fi

[ -f .github/workflows/release.yml ] || die "there is no .github/workflows/release.yml in this checkout." \
  "That workflow is what builds and publishes the release; without it, pushing a tag" \
  "would do nothing. Update your checkout (git pull) and try again."

origin=$(git remote get-url origin 2>/dev/null)
if [ -z "$origin" ]; then
  die "this repository has no 'origin' remote, so there is nowhere to publish to." \
    "Create the repository on GitHub, then point this checkout at it:" \
    "    git remote add origin https://github.com/YOUR-USER/flint.git" \
    "    git push -u origin HEAD"
fi
case "$origin" in
*github.com*) ;;
*)
  die "the 'origin' remote is not on GitHub:" \
    "    $origin" \
    "This script publishes GitHub Releases with the GitHub CLI, so origin has to be a" \
    "GitHub repository. Point it at one:" \
    "    git remote set-url origin https://github.com/YOUR-USER/flint.git"
  ;;
esac

repo_slug=$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null)
if [ -z "$repo_slug" ]; then
  die "the GitHub CLI cannot see the repository behind 'origin'." \
    "    $origin" \
    "Either it does not exist yet, or your account has no access to it. Check with:" \
    "    gh repo view" \
    "If the repository is not created yet:  gh repo create --source . --public --push"
fi
good "origin: $repo_slug"

branch=$(git symbolic-ref --short HEAD 2>/dev/null)
[ -n "$branch" ] || die "you are not on a branch (detached HEAD)." \
  "Switch to the branch you release from and try again, for example:" \
  "    git switch main"

default_branch=$(gh repo view --json defaultBranchRef --jq .defaultBranchRef.name 2>/dev/null)
[ -n "$default_branch" ] || default_branch=$branch

if [ "$branch" != "$default_branch" ]; then
  warn "you are on '$branch', but the repository's default branch is '$default_branch'."
  note "The release is built from whatever the tag points at, so this does work -"
  note "it is just rarely what you meant."
  read_answer "Release from '$branch' anyway? [y/N]" "n"
  case "$ANSWER" in
  [yY] | [yY][eE][sS]) note "continuing on '$branch'" ;;
  *) die "stopped. Switch branch and run this again:  git switch $default_branch" ;;
  esac
else
  good "on '$branch' (the default branch)"
fi

dirty=$(git status --porcelain)
if [ -n "$dirty" ]; then
  fail "you have uncommitted changes, and a release has to be made from a known state."
  printf '\n'
  printf '%s\n' "$dirty" | head -n 20 | sed 's/^/      /'
  count=$(printf '%s\n' "$dirty" | wc -l | tr -d ' ')
  [ "$count" -gt 20 ] && note "... and $((count - 20)) more"
  printf '\n    Either commit them:\n'
  printf '        git add -A && git commit -m "describe what changed"\n'
  printf '\n    or put them aside for later:\n'
  printf '        git stash\n'
  printf '\n    Then run this script again.\n'
  pause
  exit 1
fi
good "working tree is clean"

note "fetching from GitHub"
if ! git fetch --tags --quiet origin 2>/dev/null; then
  die "could not fetch from origin." \
    "Check your network and that you have access to $repo_slug:" \
    "    git fetch origin" \
    "A release needs to reach GitHub, so this has to work first."
fi

if git rev-parse --verify --quiet "refs/remotes/origin/$branch" >/dev/null; then
  behind=$(git rev-list --count "HEAD..origin/$branch")
  ahead=$(git rev-list --count "origin/$branch..HEAD")
  if [ "$behind" -gt 0 ]; then
    die "your '$branch' is $behind commit(s) behind origin/$branch." \
      "Releasing now would tag older code than what is on GitHub. Catch up first:" \
      "    git pull --ff-only" \
      "Then run this script again."
  fi
  [ "$ahead" -gt 0 ] && note "$ahead local commit(s) will be pushed along with the release commit"
  good "up to date with origin/$branch"
else
  note "origin/$branch does not exist yet; it will be created by the push"
fi

# =============================================================================================
# 3. Version
#
# Three files carry the version independently (package.json, src-tauri/tauri.conf.json and the
# Cargo workspace), and the release workflow refuses to publish if they disagree with the tag.
# =============================================================================================
step "Version"

# --- readers -----------------------------------------------------------------------------------
# The first top-level "version" key of a JSON file.
json_version() {
  perl -0777 -ne 'print $1 and last if /^\s*"version"\s*:\s*"([^"]+)"/m' "$1" 2>/dev/null
}

# src-tauri/Cargo.toml normally says `version.workspace = true`, which means the real number is in
# the root Cargo.toml under [workspace.package]. Both spellings are handled, here and when writing.
# (Detection is its own function because $(cargo_version) runs in a subshell, where an assignment
# to CARGO_VERSION_FILE would be thrown away.)
CARGO_VERSION_FILE=""
detect_cargo_version_file() {
  if perl -0777 -ne 'exit(/^version\s*=\s*"/m ? 0 : 1)' src-tauri/Cargo.toml 2>/dev/null; then
    CARGO_VERSION_FILE="src-tauri/Cargo.toml"
  else
    CARGO_VERSION_FILE="Cargo.toml"
  fi
}

cargo_version() {
  if [ "$CARGO_VERSION_FILE" = "Cargo.toml" ]; then
    perl -0777 -ne 'print $1 if /\[workspace\.package\].*?\nversion\s*=\s*"([^"]+)"/s' Cargo.toml 2>/dev/null
  else
    perl -0777 -ne 'print $1 and last if /^version\s*=\s*"([^"]+)"/m' src-tauri/Cargo.toml 2>/dev/null
  fi
}

[ -f src-tauri/Cargo.toml ] || die "src-tauri/Cargo.toml is missing - this is not a Flint checkout."
[ -f package.json ] || die "package.json is missing - this is not a Flint checkout."
[ -f src-tauri/tauri.conf.json ] || die "src-tauri/tauri.conf.json is missing - this is not a Flint checkout."
detect_cargo_version_file

pkg_version=$(json_version package.json)
tauri_version=$(json_version src-tauri/tauri.conf.json)
rust_version=$(cargo_version)

[ -n "$pkg_version" ] || die "could not read the version out of package.json." \
  "Expected a line like:  \"version\": \"1.0.0\"" \
  "Fix that file, or set the version by hand in all three manifests."
[ -n "$tauri_version" ] || die "could not read the version out of src-tauri/tauri.conf.json."
[ -n "$rust_version" ] || die "could not read the version out of $CARGO_VERSION_FILE."

printf '    %-32s %s\n' "package.json" "$pkg_version"
printf '    %-32s %s\n' "src-tauri/tauri.conf.json" "$tauri_version"
printf '    %-32s %s\n' "$CARGO_VERSION_FILE" "$rust_version"

current=$pkg_version
if [ "$pkg_version" != "$tauri_version" ] || [ "$pkg_version" != "$rust_version" ]; then
  warn "those three do not agree - they have drifted."
  note "This release will set all of them to the version you choose, which fixes it."
else
  good "current version: $current"
fi

tag_exists() {
  git rev-parse --verify --quiet "refs/tags/$1" >/dev/null && return 0
  git ls-remote --exit-code --tags origin "refs/tags/$1" >/dev/null 2>&1 && return 0
  return 1
}

# A sensible default: if the current version was never tagged, release it as it is; otherwise
# bump the patch number.
if tag_exists "v$current"; then
  suggest=$(printf '%s' "$current" | perl -ne 'if (/^(\d+)\.(\d+)\.(\d+)/) { print "$1.$2.", $3 + 1 }')
  reason="v$current is already released, so this bumps the patch number"
else
  suggest=$current
  reason="v$current has never been tagged, so it can be released as it is"
fi
[ -n "$suggest" ] || suggest=$current
note "$reason"

if [ -n "${VERSION:-}" ]; then
  version=$VERSION
  note "using VERSION=$version from the environment"
else
  read_answer "Version to release [$suggest]:" "$suggest"
  version=$ANSWER
fi

# Friendly about the two ways people type it.
case "$version" in
v*)
  version=${version#v}
  note "dropped the leading 'v': tags are written as v$version"
  ;;
esac
version=$(printf '%s' "$version" | tr -d '[:space:]')

if ! printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'; then
  die "'$version' is not a version number this project can use." \
    "It has to be MAJOR.MINOR.PATCH, optionally with a pre-release suffix:" \
    "    0.2.0      1.0.0      1.0.0-beta.1" \
    "No 'v' prefix, no build metadata (+something): Cargo and macOS both reject those."
fi

tag="v$version"
if tag_exists "$tag"; then
  die "$tag already exists, so it cannot be released again." \
    "Pick a higher version number. What is already out there:" \
    "    git tag --list 'v*' | tail" \
    "    gh release list" \
    "If $tag was a mistake and nobody has it yet, remove it first:" \
    "    git tag -d $tag && git push origin :refs/tags/$tag"
fi

# Going backwards is legal but almost always a typo, so it needs a second look.
if [ "$version" != "$current" ]; then
  lowest=$(printf '%s\n%s\n' "$current" "$version" | sort -V | head -n 1)
  if [ "$lowest" = "$version" ]; then
    warn "$version is lower than the current version ($current)."
    read_answer "Really go backwards? [y/N]" "n"
    case "$ANSWER" in
    [yY] | [yY][eE][sS]) ;;
    *) die "stopped. Nothing was changed." ;;
    esac
  fi
fi
good "releasing $tag"

# =============================================================================================
# 4. Write the version everywhere
# =============================================================================================
step "Setting the version to $version in every file that carries it"

FILES=(package.json src-tauri/tauri.conf.json "$CARGO_VERSION_FILE")
[ -f package-lock.json ] && FILES+=(package-lock.json)
[ -f Cargo.lock ] && FILES+=(Cargo.lock)

BACKUP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/cc-release-XXXXXX") || die "could not create a temporary folder"
: >"$BACKUP_DIR/manifest"
for f in "${FILES[@]}"; do
  cp "$f" "$BACKUP_DIR/$(printf '%s' "$f" | tr / _)" || die "could not back up $f"
  printf '%s\n' "$f" >>"$BACKUP_DIR/manifest"
done
note "originals copied to $BACKUP_DIR (put back automatically if anything goes wrong)"

# --- writers -----------------------------------------------------------------------------------
# Every one of these is a targeted, single-occurrence edit: no JSON or TOML reformatting, so the
# diff stays one line per file wherever possible.
set_json_version() {
  NEW=$version perl -0777 -pi -e 's/^(\s*"version"\s*:\s*")[^"]*(")/$1$ENV{NEW}$2/m' "$1"
}

# package-lock.json carries it twice at the top (the root object and packages[""]), and again for
# every dependency further down. Only the first two are ours.
set_lock_versions() {
  NEW=$version perl -0777 -pi -e 'my $n = 0; s/"version"\s*:\s*"[^"]*"/++$n <= 2 ? "\"version\": \"$ENV{NEW}\"" : $&/ge' package-lock.json
}

set_cargo_version() {
  if [ "$CARGO_VERSION_FILE" = "Cargo.toml" ]; then
    NEW=$version perl -0777 -pi -e 's/(\[workspace\.package\].*?\nversion\s*=\s*")[^"]*(")/$1$ENV{NEW}$2/s' Cargo.toml
  else
    NEW=$version perl -0777 -pi -e 's/^(version\s*=\s*")[^"]*(")/$1$ENV{NEW}$2/m' src-tauri/Cargo.toml
  fi
}

# Cargo.lock lists the two workspace crates with their own version lines.
set_cargo_lock_versions() {
  local crate
  for crate in convert-core flint; do
    CRATE=$crate NEW=$version perl -0777 -pi -e 's/(name = "\Q$ENV{CRATE}\E"\nversion = ")[^"]*(")/$1$ENV{NEW}$2/' Cargo.lock
  done
}

set_json_version package.json || die "could not update package.json"
set_json_version src-tauri/tauri.conf.json || die "could not update src-tauri/tauri.conf.json"
set_cargo_version || die "could not update $CARGO_VERSION_FILE"
[ -f package-lock.json ] && { set_lock_versions || die "could not update package-lock.json"; }
[ -f Cargo.lock ] && { set_cargo_lock_versions || die "could not update Cargo.lock"; }

# --- and prove it --------------------------------------------------------------------------------
check_one() {
  local label=$1 got=$2
  if [ "$got" = "$version" ]; then
    printf '    %s ✓ %-34s %s%s\n' "$GREEN" "$label" "$got" "$OFF"
    return 0
  fi
  printf '    %s ✗ %-34s %s (expected %s)%s\n' "$RED" "$label" "${got:-<unreadable>}" "$version" "$OFF"
  return 1
}

problems=0
check_one "package.json" "$(json_version package.json)" || problems=1
check_one "src-tauri/tauri.conf.json" "$(json_version src-tauri/tauri.conf.json)" || problems=1
check_one "$CARGO_VERSION_FILE" "$(cargo_version)" || problems=1
if [ -f package-lock.json ]; then
  lock_top=$(perl -0777 -ne 'my @v = /"version"\s*:\s*"([^"]+)"/g; print join(",", @v[0, 1])' package-lock.json)
  check_one "package-lock.json (both entries)" "$(printf '%s' "$lock_top" | perl -pe 's/^([^,]+),\1$/$1/')" || problems=1
fi
if [ -f Cargo.lock ]; then
  for crate in convert-core flint; do
    got=$(CRATE=$crate perl -0777 -ne 'print $1 if /name = "\Q$ENV{CRATE}\E"\nversion = "([^"]+)"/' Cargo.lock)
    check_one "Cargo.lock ($crate)" "$got" || problems=1
  done
fi
[ "$problems" -eq 0 ] || die "the version could not be written to every file consistently." \
  "Nothing was committed. The originals have been put back, so the repository is as it was." \
  "This means one of those files does not look the way this script expects - open the ones" \
  "marked ✗ above and set the version by hand, or report it."

# A stale Cargo.lock fails CI, so catch it here if cargo is around to tell us.
if command -v cargo >/dev/null 2>&1; then
  cargo_out=$(cargo metadata --locked --offline --format-version 1 2>&1 >/dev/null)
  case "$cargo_out" in
  *"needs to be updated"* | *"--locked"*)
    die "Cargo.lock no longer matches Cargo.toml after the version change." \
      "Run this, commit nothing, and then start again:" \
      "    cargo update --workspace --offline" \
      "The originals have been put back."
    ;;
  *) [ -z "$cargo_out" ] && good "Cargo.lock is consistent (cargo metadata --locked)" ;;
  esac
fi

changed=$(git status --porcelain -- "${FILES[@]}")
if [ -z "$changed" ]; then
  note "the files already said $version - the release commit will only carry the tag"
fi

# =============================================================================================
# 5. Confirm. Everything up to here was local and reversible; past this point is not.
# =============================================================================================
step "Ready. This is the point of no return - read it before answering."

printf '\n'
printf '    Version      %s  ->  %s\n' "$current" "$version"
printf '    Tag          %s\n' "$tag"
printf '    Branch       %s\n' "$branch"
printf '    Repository   %s\n' "$repo_slug"
printf '    Remote       %s\n' "$origin"
printf '\n    Files this commit changes:\n'
if [ -n "$changed" ]; then
  git diff --stat -- "${FILES[@]}" | sed 's/^/      /'
else
  printf '      (none - the manifests already said %s)\n' "$version"
fi
printf '\n    Then it will:\n'
printf '      * commit them as   Release %s\n' "$tag"
printf '      * create the tag   %s\n' "$tag"
printf '      * push the branch and the tag to %s\n' "$repo_slug"
printf '      * which starts the Release workflow: it tests, builds the app for both Apple\n'
printf '        Silicon and Intel, and publishes a %sPUBLIC%s GitHub Release with the .dmg files.\n' "$BOLD" "$OFF"
printf '\n'
warn "the app is not code-signed or notarized, so anyone downloading it has to click"
note "through macOS Gatekeeper. The release notes explain how; docs/RELEASING.md has the detail."

if [ "$DRY_RUN" = "1" ]; then
  printf '\n'
  warn "RELEASE_DRY_RUN=1, so none of the above will actually happen."
fi

read_answer "Type 'yes' to go ahead, anything else to stop:" ""
case "$ANSWER" in
yes | YES | Yes) ;;
*)
  printf '\n'
  note "stopped - nothing was committed, tagged or pushed."
  restore_backup
  good "the repository is exactly as you found it"
  pause
  exit 0
  ;;
esac

if [ "$DRY_RUN" = "1" ]; then
  step "Dry run"
  note "would run: git add ${FILES[*]}"
  note "would run: git commit -m 'Release $tag'"
  note "would run: git tag -a $tag -m 'Flint $version'"
  note "would run: git push origin HEAD:refs/heads/$branch"
  note "would run: git push origin refs/tags/$tag"
  note "would then watch the Actions run and open the release page"
  restore_backup
  good "dry run finished; the repository is unchanged"
  pause
  exit 0
fi

# =============================================================================================
# 6. Commit, tag, push
# =============================================================================================
step "Committing and tagging"

undo_help() {
  printf '\n%s    How to undo this:%s\n' "$BOLD" "$OFF"
  printf '        git push origin :refs/tags/%s       # remove the tag from GitHub\n' "$tag"
  printf '        git tag -d %s                       # remove it locally\n' "$tag"
  printf '        gh release delete %s --yes          # only if a release was already created\n' "$tag"
  printf '\n    And to undo the version commit itself:\n'
  printf '        git revert HEAD                       # safe once it has been pushed\n'
  printf '        git reset --hard HEAD~1               # only if you have not pushed it\n'
  printf '\n%s    Anyone who already downloaded a file keeps it: deleting a public release\n' "$DIM"
  printf '    unpublishes it, it does not un-download it.%s\n' "$OFF"
}

if [ -n "$changed" ]; then
  git add -- "${FILES[@]}" || die "git add failed. Nothing was committed."
  if ! git commit -q -m "Release $tag" -m "Version $version in package.json, tauri.conf.json and the Cargo workspace."; then
    die "git commit failed - see above." \
      "Nothing was tagged or pushed. Your changes are still staged; undo that with:" \
      "    git restore --staged ${FILES[*]}"
  fi
  good "committed: $(git log -1 --oneline)"
  committed=1
else
  note "nothing to commit; tagging the current commit $(git rev-parse --short HEAD)"
  committed=0
fi
BACKUP_DIR="" # committed: the backup would now overwrite good work

if ! git tag -a "$tag" -m "Flint $version"; then
  fail "could not create the tag $tag."
  if [ "$committed" = "1" ]; then
    printf '    The version commit was made but nothing was pushed. To undo it:\n'
    printf '        git reset --hard HEAD~1\n'
  fi
  pause
  exit 1
fi
good "tagged $tag"

step "Pushing to $repo_slug"
note "the branch first, then the tag - the tag is what starts the release"

if ! git push origin "HEAD:refs/heads/$branch"; then
  fail "could not push the branch to origin."
  printf '    Nothing public has happened: the tag exists only on this machine.\n'
  printf '    Fix the reason above (network, permissions, or a newer origin/%s), then\n' "$branch"
  printf '    run this script again.\n'
  printf '\n    To roll this attempt back completely:\n'
  printf '        git tag -d %s\n' "$tag"
  [ "$committed" = "1" ] && printf '        git reset --hard HEAD~1\n'
  pause
  exit 1
fi
good "branch pushed"

if ! git push origin "refs/tags/$tag"; then
  fail "the branch was pushed, but the tag was not - so no release has started."
  printf '    Fix the reason above and push just the tag:\n'
  printf '        git push origin %s\n' "$tag"
  printf '\n    Or drop the attempt entirely:\n'
  printf '        git tag -d %s\n' "$tag"
  printf '    (the version commit is already on GitHub; git revert HEAD undoes it cleanly)\n'
  pause
  exit 1
fi
good "tag pushed - the release workflow is starting"

# =============================================================================================
# 7. Watch it
# =============================================================================================
step "Watching the build on GitHub"
note "roughly 20-40 minutes: tests, then one build per Mac architecture"
note "you can close this window - the build carries on without it"

actions_url="https://github.com/$repo_slug/actions"
run_id=""
for _ in $(seq 1 30); do
  run_id=$(gh run list --workflow release.yml --limit 20 \
    --json databaseId,headBranch,status \
    --jq "[.[] | select(.headBranch == \"$tag\")] | .[0].databaseId" 2>/dev/null)
  [ "$run_id" = "null" ] && run_id=""
  [ -n "$run_id" ] && break
  sleep 5
done

if [ -z "$run_id" ]; then
  warn "the tag is pushed, but no workflow run has appeared after two and a half minutes."
  note "It may just be queued. Check here:  $actions_url"
  note "If nothing ever appears, Actions may be disabled for this repository"
  note "(Settings > Actions > General > Allow all actions)."
  command -v open >/dev/null 2>&1 && open "$actions_url" >/dev/null 2>&1
  pause
  exit 0
fi

run_url="https://github.com/$repo_slug/actions/runs/$run_id"
good "run started: $run_url"
printf '\n'

gh run watch "$run_id" --exit-status --interval 20
watch_status=$?

if [ "$watch_status" -ne 0 ]; then
  fail "the release workflow did not finish successfully."
  printf '    Nothing was published (the release is only created after every job passes).\n'
  printf '\n    What failed, and why:\n'
  printf '        %s\n' "$run_url"
  printf '        gh run view %s --log-failed\n' "$run_id"
  printf '\n    Fix it on %s, then release again with a new version number.\n' "$branch"
  undo_help
  command -v open >/dev/null 2>&1 && open "$run_url" >/dev/null 2>&1
  pause
  exit 1
fi

step "Released"
release_url=$(gh release view "$tag" --json url --jq .url 2>/dev/null)
if [ -z "$release_url" ]; then
  warn "the workflow passed but the release page is not readable yet - give it a moment."
  release_url="https://github.com/$repo_slug/releases/tag/$tag"
fi
good "Flint $version is published"
printf '\n    %s\n' "$release_url"
printf '\n    Attached to it:\n'
printf '      Flint-%s-macOS-apple-silicon.dmg   (M1/M2/M3/M4 Macs)\n' "$version"
printf '      Flint-%s-macOS-intel.dmg           (Intel Macs)\n' "$version"
printf '      the same two as .app.zip, and SHA256SUMS.txt\n'
printf '\n    %sTell people what to expect:%s the app is unsigned, so macOS says it "cannot be\n' "$BOLD" "$OFF"
printf '    verified". The release notes walk them through System Settings > Privacy &\n'
printf '    Security > Open Anyway.\n'

command -v open >/dev/null 2>&1 && open "$release_url" >/dev/null 2>&1

pause
