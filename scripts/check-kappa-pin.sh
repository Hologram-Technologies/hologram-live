#!/usr/bin/env bash
set -euo pipefail
# The Kappa pin is auditable: the locked revision is the documented one, every
# carried patch names where it was offered upstream, and nothing forbidden or
# branch-floating is in the registry build's dependency graph.
#
# KAPPA_PIN_ALLOW_UNOPENED=1   spike only: a patch may lack its upstream link.
# KAPPA_PIN_ALLOW_AWS_LC=1     spike only: rekindle-aead still pulls aws-lc
#                              (see third_party/kappa/README.md, optional-aead).
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
readme="${root}/third_party/kappa/README.md"
fail=0

documented=$(grep -E '^\| Pin revision' "${readme}" | grep -oE '[0-9a-f]{40}' | head -1 || true)
locked=$(grep -A2 '^name = "kappa-core"$' "${root}/Cargo.lock" | grep -oE '#[0-9a-f]{40}' | tr -d '#' | head -1 || true)
if [[ -z "${documented}" || "${documented}" != "${locked}" ]]; then
  printf 'kappa pin: README says %s, Cargo.lock says %s\n' "${documented:-none}" "${locked:-none}" >&2
  fail=1
fi

shopt -s nullglob
for patch in "${root}"/third_party/kappa/patches/*.patch; do
  name=$(basename "${patch}")
  if ! grep -F "${name}" "${readme}" | grep -qE 'https://github\.com/[^ |]+/(pull|issues)/[0-9]+'; then
    if [[ "${KAPPA_PIN_ALLOW_UNOPENED:-0}" != "1" ]]; then
      printf 'kappa pin: %s has no upstream link in the README\n' "${name}" >&2
      fail=1
    fi
  fi
done

tree=$(RUSTC_WRAPPER= cargo tree --manifest-path "${root}/Cargo.toml" --package hologram-live --features oci --edges normal --prefix none --locked)
forbidden='^(topcoat|veilid|openssl-sys)'
if [[ "${KAPPA_PIN_ALLOW_AWS_LC:-0}" != "1" ]]; then
  forbidden='^(topcoat|veilid|openssl-sys|aws-lc)'
fi
if found=$(grep -iE "${forbidden}" <<<"${tree}" | sort -u); then
  printf 'kappa pin: forbidden crates are in the registry graph:\n%s\n' "${found}" >&2
  fail=1
fi
if grep -E '^source = "git\+' "${root}/Cargo.lock" | grep -vqE '#[0-9a-f]{40}"$'; then
  printf 'kappa pin: a git dependency is not locked to a revision\n' >&2
  fail=1
fi

(( fail == 0 )) && printf 'kappa pin gate passed (%s)\n' "${locked}"
exit "${fail}"
