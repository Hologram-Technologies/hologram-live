#!/usr/bin/env bash
set -euo pipefail
# No layer in memory, and no Kappa type outside the adapter.
#
# The store calls that return or take a whole blob are allowed for manifests
# only (4 MiB at most). Every Kappa type stays inside src/oci_store/, so a
# store swap touches one directory (ADR 025).
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
fail=0
if grep -rnE 'blob_get\(|blob_get_range\(|blob_get_verified\(|to_bytes\(' \
    "${root}/src/oci_store" "${root}/src/modules/oci" 2>/dev/null | grep -v '/manifests.rs:'; then
  echo 'error: a whole-buffer call outside manifests.rs' >&2
  fail=1
fi
if grep -rnE 'kappa_core|kappa_store_redb' "${root}/src" | grep -v "^${root}/src/oci_store/"; then
  echo 'error: a Kappa type outside src/oci_store/' >&2
  fail=1
fi
(( fail == 0 )) && printf 'oci streaming gate passed\n'
exit "${fail}"
