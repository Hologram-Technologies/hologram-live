#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CHANNEL=${1:-}
TAG=${2:-}

cargo_version() {
  awk '
    /^\[package\]$/ { package = 1; next }
    /^\[/ && package { exit }
    package && /^version[[:space:]]*=/ {
      value = $0
      sub(/^[^=]*=[[:space:]]*"/, "", value)
      sub(/"[[:space:]]*$/, "", value)
      print value
      exit
    }
  ' "$1"
}

json_version() {
  sed -n 's/^[[:space:]]*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$1" | head -n 1
}

case "$CHANNEL" in
  server)
    PREFIX=server-v
    EXPECTED=$(cargo_version "$ROOT/Cargo.toml")
    ;;
  desktop)
    PREFIX=desktop-v
    EXPECTED=$(json_version "$ROOT/apps/desktop/src-tauri/tauri.conf.json")
    DESKTOP_CARGO=$(cargo_version "$ROOT/apps/desktop/src-tauri/Cargo.toml")
    DESKTOP_PACKAGE=$(json_version "$ROOT/apps/desktop/package.json")
    if [ "$EXPECTED" != "$DESKTOP_CARGO" ] || [ "$EXPECTED" != "$DESKTOP_PACKAGE" ]; then
      echo "error: desktop versions differ: tauri=$EXPECTED cargo=$DESKTOP_CARGO package=$DESKTOP_PACKAGE" >&2
      exit 1
    fi
    ;;
  docs)
    PREFIX=docs-v
    EXPECTED=$(json_version "$ROOT/apps/docs/package.json")
    DOCS_LOCK=$(json_version "$ROOT/apps/docs/package-lock.json")
    if [ "$EXPECTED" != "$DOCS_LOCK" ]; then
      echo "error: docs versions differ: package=$EXPECTED lock=$DOCS_LOCK" >&2
      exit 1
    fi
    ;;
  *)
    echo "usage: $0 server|desktop|docs <release-tag>" >&2
    exit 2
    ;;
esac

case "$TAG" in
  "$PREFIX"*) VERSION=${TAG#"$PREFIX"} ;;
  *)
    echo "error: $CHANNEL releases require a ${PREFIX}<version> tag; got $TAG" >&2
    exit 1
    ;;
esac

if ! printf '%s\n' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'; then
  echo "error: release tag version is not valid semver: $VERSION" >&2
  exit 1
fi

if [ "$VERSION" != "$EXPECTED" ]; then
  echo "error: $TAG does not match the $CHANNEL manifest version $EXPECTED" >&2
  exit 1
fi

printf '%s release tag matches version %s\n' "$CHANNEL" "$VERSION"
