#!/usr/bin/env bash
set -euo pipefail
# The registry's store is vendored and auditable (ADR 032):
# - the vendored files are exactly the ones recorded in VENDORED.sha256, so an
#   edit to vendored code is deliberate and shows in review;
# - no store crate comes from anywhere but this repository;
# - every carried patch names where it was offered upstream;
# - nothing forbidden is in the registry build's dependency graph.
#
# After a deliberate edit or a re-vendor:  scripts/check-kappa-pin.sh --record
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
readme="${root}/third_party/kappa/README.md"
record="${root}/third_party/kappa/VENDORED.sha256"
fail=0

cd "${root}"
current=$(find third_party/kappa/crates third_party/dcbor -type f | LC_ALL=C sort | xargs sha256sum)
if [[ "${1:-}" == "--record" ]]; then
  printf '%s\n' "${current}" > "${record}"
  printf 'recorded %s vendored files\n' "$(wc -l < "${record}")"
  exit 0
fi
if ! diff <(printf '%s\n' "${current}") "${record}" > /dev/null; then
  printf 'kappa pin: the vendored files differ from %s:\n' "${record#"${root}/"}" >&2
  diff <(printf '%s\n' "${current}") "${record}" | head -20 >&2 || true
  fail=1
fi

for crate in kappa-core kappa-store-redb dcbor dcbor-derive; do
  if grep -A2 "^name = \"${crate}\"$" Cargo.lock | grep -q '^source = '; then
    printf 'kappa pin: %s is not the vendored copy (it has a source in Cargo.lock)\n' "${crate}" >&2
    fail=1
  fi
done

shopt -s nullglob
for patch in third_party/kappa/patches/*.patch; do
  name=$(basename "${patch}")
  if ! grep -F "${name}" "${readme}" | grep -qE 'https://github\.com/[^ |]+/(pull|issues)/[0-9]+'; then
    printf 'kappa pin: %s has no upstream link in the README\n' "${name}" >&2
    fail=1
  fi
done

tree=$(RUSTC_WRAPPER= cargo tree --manifest-path "${root}/Cargo.toml" --package hologram-live --features oci --edges normal --prefix none --locked)
if found=$(grep -iE '^(topcoat|veilid|openssl-sys|aws-lc|rekindle)' <<<"${tree}" | sort -u); then
  printf 'kappa pin: forbidden crates are in the registry graph:\n%s\n' "${found}" >&2
  fail=1
fi
if grep -E '^source = "git\+' Cargo.lock | grep -vqE '#[0-9a-f]{40}"$'; then
  printf 'kappa pin: a git dependency is not locked to a revision\n' >&2
  fail=1
fi

(( fail == 0 )) && printf 'kappa pin gate passed (%s vendored files)\n' "$(wc -l < "${record}")"
exit "${fail}"
