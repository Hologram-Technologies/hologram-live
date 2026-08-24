#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
REMOTE=${DOCS_RELEASE_REMOTE:-origin}
VERSION=${1:-}

if [ -z "$VERSION" ]; then
  VERSION=$(sed -n 's/^[[:space:]]*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$ROOT/apps/docs/package.json" | head -n 1)
fi
TAG="docs-v$VERSION"

"$ROOT/scripts/check-release-tag.sh" docs "$TAG"

if [ -n "$(git -C "$ROOT" status --porcelain)" ]; then
  echo "error: commit or stash all changes before releasing documentation" >&2
  exit 1
fi

if git -C "$ROOT" rev-parse --verify --quiet "refs/tags/$TAG" >/dev/null; then
  echo "error: local tag $TAG already exists" >&2
  exit 1
fi

if git -C "$ROOT" ls-remote --exit-code --tags "$REMOTE" "refs/tags/$TAG" >/dev/null 2>&1; then
  echo "error: tag $TAG already exists on $REMOTE" >&2
  exit 1
fi

(
  cd "$ROOT/apps/docs"
  npm ci
  GITHUB_PAGES=true npm run build
)

git -C "$ROOT" tag -a "$TAG" -m "Hologram documentation $VERSION"
if ! git -C "$ROOT" push "$REMOTE" "$TAG"; then
  git -C "$ROOT" tag -d "$TAG" >/dev/null
  echo "error: failed to push $TAG; removed the local tag" >&2
  exit 1
fi

printf 'pushed %s; GitHub Pages deployment started\n' "$TAG"
