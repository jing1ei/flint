#!/bin/bash
#
# Release.command - double-click in Finder (or run from a terminal) to cut a new Flint release.
#
# Releases are immutable tags: this script bumps the version into every manifest, commits that
# single change, tags it v<version> and pushes. The tag push starts the Release workflow, which
# runs the full CI gates, builds both Mac architectures and Windows, and publishes the GitHub
# Release only if every check passes. A published release is never moved or overwritten - a fix
# means a new version number.
#
#   ./Release.command                    interactive
#   RELEASE_DRY_RUN=1 ./Release.command  align and verify, but commit/tag/push nothing
#   VERSION=1.2.3 ./Release.command      skip the version question
#
set -uo pipefail
cd "$(dirname "$0")" || exit 1

printf '\033]0;Flint - release\033'

BOLD=$'\033[1m'; DIM=$'\033[2m'; RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; OFF=$'\033[0m'
step() { printf '\n%s==> %s%s\n' "$BOLD" "$1" "$OFF"; }
note() { printf '%s    %s%s\n' "$DIM" "$1" "$OFF"; }
good() { printf '%s ✓ %s%s\n' "$GREEN" "$1" "$OFF"; }
warn() { printf '%s ! %s%s\n' "$YELLOW" "$1" "$OFF"; }
fail() { printf '\n%s ✗ %s%s\n' "$RED" "$1" "$OFF"; }

DRY_RUN=${RELEASE_DRY_RUN:-0}
BACKUP_DIR=""

pause() { printf '\n%sPress Return to close this window.%s ' "$DIM" "$OFF"; read -r _ || true; }
die() { fail "$1"; shift; for line in "$@"; do printf '    %s\n' "$line"; done; pause; exit 1; }

case "${1:-}" in
  "") ;;
  -h|--help)
    sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
    exit 0 ;;
  *) die "unrecognised argument '$1'" "Run it with no arguments, or --help." ;;
esac

# -------------------------------------------------------------------------------- prerequisites
step "Checking the tools this needs"
command -v git  >/dev/null 2>&1 || die "git is not installed." "Run: xcode-select --install"
command -v gh   >/dev/null 2>&1 || die "the GitHub CLI (gh) is not installed." "brew install gh   (or get it from https://cli.github.com)"
command -v perl >/dev/null 2>&1 || die "perl is not installed." "Run: xcode-select --install"
command -v python3 >/dev/null 2>&1 || die "python3 is not installed." "It ships with macOS and with the Xcode command line tools."
gh auth status >/dev/null 2>&1 || die "gh is installed but not logged in." "Run: gh auth login"
good "git, gh (logged in), perl, python3"

# -------------------------------------------------------------------------------- the repository
step "Checking the repository"
toplevel=$(git rev-parse --show-toplevel 2>/dev/null)
[ -n "$toplevel" ] || die "this folder is not a git repository."
[ "$toplevel" = "$(pwd -P)" ] || die "Release.command must sit at the repository root."
origin=$(git remote get-url origin 2>/dev/null)
case "$origin" in *github.com*) ;; *) die "origin is not a GitHub remote:" "    $origin" ;; esac
repo_slug=$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null)
[ -n "$repo_slug" ] || die "gh cannot see the repository behind origin:" "    $origin"
good "origin: $repo_slug"

branch=$(git symbolic-ref --short HEAD 2>/dev/null) || die "you are on a detached HEAD." "Check out main first."
[ -n "$(git status --porcelain)" ] && die "the working tree has uncommitted changes." "Commit or stash them first - a release is one known commit."
git fetch --tags --quiet origin 2>/dev/null || die "could not fetch from origin."
if git rev-parse --verify --quiet "refs/remotes/origin/$branch" >/dev/null; then
  behind=$(git rev-list --count "HEAD..origin/$branch")
  [ "$behind" -eq 0 ] || die "'$branch' is $behind commit(s) behind origin/$branch." "Run: git pull --ff-only"
fi
good "on '$branch', clean and up to date"

# -------------------------------------------------------------------------------- version
step "Version"
json_version() { perl -0777 -ne 'print $1 and last if /^\s*"version"\s*:\s*"([^"]+)"/m' "$1" 2>/dev/null; }
current=$(json_version package.json)
[ -n "$current" ] || die "could not read the version from package.json."

tag_exists() { git rev-parse --verify --quiet "refs/tags/$1" >/dev/null || git ls-remote --exit-code --tags origin "refs/tags/$1" >/dev/null 2>&1; }
if tag_exists "v$current"; then suggest=$(printf '%s' "$current" | perl -ne 'if (/^(\d+)\.(\d+)\.(\d+)/) { print "$1.$2.", $3 + 1 }'); else suggest=$current; fi
note "current: $current    suggestion: $suggest"

if [ -n "${VERSION:-}" ]; then version=$VERSION; else
  printf '%sVersion to release [%s]:%s ' "$BOLD" "$suggest" "$OFF"; read -r version || version=""
  [ -n "$version" ] && version=$version || version=$suggest
fi
case "$version" in v*) version=${version#v} ;; esac
version=$(printf '%s' "$version" | tr -d '[:space:]')
printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$' \
  || die "'$version' is not MAJOR.MINOR.PATCH[-pre]." "Examples: 1.2.0   2.0.0-rc.1"
tag="v$version"
tag_exists "$tag" && die "$tag already exists." "Releases are immutable - pick the next number: git tag --list 'v*'"
[ "$version" = "$current" ] || { lowest=$(printf '%s\n%s\n' "$current" "$version" | sort -V | head -n 1); [ "$lowest" != "$version" ] || warn "this goes backwards from $current to $version."; }
good "releasing $tag"

# ---------------------------------------------------------------------- align the version files
step "Writing $version into every version manifest"
FILES=(package.json package-lock.json Cargo.toml Cargo.lock src-tauri/tauri.conf.json)
BACKUP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/flint-release-XXXXXX")
: > "$BACKUP_DIR/manifest"
for f in "${FILES[@]}"; do cp "$f" "$BACKUP_DIR/$(printf '%s' "$f" | tr / _)"; printf '%s\n' "$f" >> "$BACKUP_DIR/manifest"; done

NEW=$version perl -0777 -pi -e 's/^(\s*"version"\s*:\s*")[^"]*(")/$1$ENV{NEW}$2/m' package.json src-tauri/tauri.conf.json
# package-lock.json carries the version twice at the top (root object and packages[""]).
NEW=$version perl -0777 -pi -e 'my $n = 0; s/"version"\s*:\s*"[^"]*"/++$n <= 2 ? "\"version\": \"$ENV{NEW}\"" : $&/ge' package-lock.json
NEW=$version perl -0777 -pi -e 's/(\[workspace\.package\].*?\nversion\s*=\s*")[^"]*(")/$1$ENV{NEW}$2/s' Cargo.toml
for crate in convert-core flint; do
  CRATE=$crate NEW=$version perl -0777 -pi -e 's/(name = "\Q$ENV{CRATE}\E"\nversion = ")[^"]*(")/$1$ENV{NEW}$2/' Cargo.lock
done

python3 scripts/release_version.py "$tag" >/dev/null || {
  for rel in $(cat "$BACKUP_DIR/manifest"); do cp "$BACKUP_DIR/$(printf '%s' "$rel" | tr / _)" "$rel"; done
  die "the manifests do not all agree after the edit; they were restored." "Check the files marked above and report it."
}
if command -v cargo >/dev/null 2>&1; then
  cargo metadata --locked --offline --format-version 1 >/dev/null 2>&1 \
    || die "Cargo.lock no longer matches Cargo.toml; files restored." "Run: cargo update --workspace --offline"
fi
good "all six manifests say $version"

changed=$(git status --porcelain -- "${FILES[@]}")

# -------------------------------------------------------------------------------- confirm
step "Ready"
printf '    Version     %s  ->  %s\n' "$current" "$version"
printf '    Tag         %s (triggers the Release workflow)\n' "$tag"
printf '    Branch      %s on %s\n' "$branch" "$repo_slug"
printf '\n    Files changed:\n'
git diff --stat -- "${FILES[@]}" | sed 's/^/      /'
printf '\n    Then it commits, tags and pushes. The Release workflow runs the full test suite, builds\n'
printf '    both Mac architectures and Windows, and publishes only if all checks pass.\n\n'
warn "the app is ad-hoc signed and not notarized; the release notes explain the Gatekeeper step."
[ "$DRY_RUN" = "1" ] && warn "RELEASE_DRY_RUN=1: nothing will be committed, tagged or pushed."
printf '%sType yes to go ahead, anything else stops:%s ' "$BOLD" "$OFF"; read -r answer || answer=""
[ "$answer" = "yes" ] || { for rel in $(cat "$BACKUP_DIR/manifest"); do cp "$BACKUP_DIR/$(printf '%s' "$rel" | tr / _)" "$rel"; done; good "stopped; files restored"; pause; exit 0; }

if [ "$DRY_RUN" = "1" ]; then
  warn "dry run: would git add/commit, tag $tag, and push."
  for rel in $(cat "$BACKUP_DIR/manifest"); do cp "$BACKUP_DIR/$(printf '%s' "$rel" | tr / _)" "$rel"; done
  pause; exit 0
fi

# ---------------------------------------------------------------------- commit, tag, push
git add -- "${FILES[@]}"
git commit -q -m "Release $tag" -m "Version $version across package.json, package-lock.json, the Cargo workspace/lock and tauri.conf.json." || die "commit failed."
good "committed $(git log -1 --oneline)"
BACKUP_DIR=""
git tag -a "$tag" -m "Flint $version" || die "could not create $tag."
good "tagged $tag"

git push origin "HEAD:refs/heads/$branch" || die "could not push the branch." "The tag is local only: git tag -d $tag after fixing the branch."
git push origin "refs/tags/$tag" || die "the branch pushed but the tag did not." "Push the tag manually: git push origin $tag"
good "pushed - the Release workflow is running"

# -------------------------------------------------------------------------------- watch
step "Watching the build"
note "tests, then one Mac build per architecture and Windows; roughly 20-40 minutes"
run_id=""
for _ in $(seq 1 30); do
  run_id=$(gh run list --workflow release.yml --limit 20 --json databaseId,headBranch,status \
    --jq "[.[] | select(.headBranch == \"$tag\")] | .[0].databaseId" 2>/dev/null)
  [ "$run_id" = "null" ] && run_id=""
  [ -n "$run_id" ] && break
  sleep 5
done
if [ -z "$run_id" ]; then
  warn "no workflow run appeared within 2.5 minutes; check https://github.com/$repo_slug/actions"
  pause; exit 0
fi
run_url="https://github.com/$repo_slug/actions/runs/$run_id"
gh run watch "$run_id" --exit-status --interval 20 || die "the Release workflow failed; nothing was published." "Logs: $run_url" "Fix on $branch and release a new version number."

release_url="https://github.com/$repo_slug/releases/tag/$tag"
good "Flint $version is published"
printf '\n    %s\n\n' "$release_url"
command -v open >/dev/null 2>&1 && open "$release_url" >/dev/null 2>&1
pause
