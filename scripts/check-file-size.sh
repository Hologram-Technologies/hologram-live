#!/usr/bin/env bash
set -euo pipefail

limit="${HOLOGRAM_MAX_SOURCE_LINES:-1500}"
failed=0

while IFS= read -r file; do
  case "$file" in
    tests/*|*/tests/*|features/*|*/generated/*|*/gen/*|apps/docs/public/openapi.json|*Cargo.lock|*package-lock.json)
      continue
      ;;
  esac

  case "$file" in
    *.rs)
      lines="$(awk '/^[[:space:]]*#\[cfg\(test\)\]/{exit} {count++} END{print count+0}' "$file")"
      ;;
    *.ts|*.tsx|*.js|*.mjs|*.astro|*.css|*.html|*.proto|*.sh|*.toml|*.md|*.json|*.yml|*.yaml)
      lines="$(wc -l < "$file" | tr -d ' ')"
      ;;
    *)
      continue
      ;;
  esac

  if (( lines > limit )); then
    printf '%s: %s production lines (limit %s)\n' "$file" "$lines" "$limit" >&2
    failed=1
  fi
done < <(git ls-files --cached --others --exclude-standard)

if (( failed != 0 )); then
  exit 1
fi

printf 'source file size gate passed (maximum %s production lines)\n' "$limit"
