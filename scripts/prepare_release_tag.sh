#!/bin/bash
# Create a missing release tag, or resume the same commit after a failed build.
set -euo pipefail
: "${TAG:?}" "${SOURCE_SHA:?}"
git check-ref-format "refs/tags/$TAG"
head=$(git rev-parse HEAD)
[ "$head" = "$SOURCE_SHA" ] || { echo 'Checkout does not match triggering commit' >&2; exit 1; }
if git show-ref --verify --quiet "refs/tags/$TAG"; then
  tagged=$(git rev-parse "refs/tags/$TAG^{commit}")
  if [ "$tagged" != "$SOURCE_SHA" ]; then
    echo "$TAG belongs to another commit; leave it unchanged."
    echo 'release=false' >> "$GITHUB_OUTPUT"
    exit 0
  fi
else
  git tag -a "$TAG" -m "Flint ${TAG#v}" "$SOURCE_SHA"
  git push origin "refs/tags/$TAG"
fi
echo 'release=true' >> "$GITHUB_OUTPUT"
