#!/usr/bin/env bash
set -euo pipefail
# No layer in memory, and no Kappa type outside the adapter.
#
# The store calls that return or take a whole blob are allowed for manifests
# only (4 MiB at most). Every Kappa type stays inside src/oci_store/, so a
# store swap touches one directory (ADR 025).
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
fail=0
# grep exits 2 for a directory that does not exist yet, which under pipefail
# would make the whole test false and the gate pass. Search what exists, and
# judge by what was printed.
dirs=()
for dir in "${root}/src/oci_store" "${root}/src/modules/oci"; do
  [[ -d "${dir}" ]] && dirs+=("${dir}")
done
found=$(grep -rnE 'blob_get(_range|_verified)?[[:space:]]*\(|to_bytes[[:space:]]*\(' "${dirs[@]}" | grep -v '/manifests.rs:' || true)
if [[ -n "${found}" ]]; then
  printf '%s\n' "${found}"
  echo 'error: a whole-buffer call outside manifests.rs' >&2
  fail=1
fi
if grep -rnE 'kappa_core|kappa_store_redb' "${root}/src" | grep -v "^${root}/src/oci_store/"; then
  echo 'error: a Kappa type outside src/oci_store/' >&2
  fail=1
fi
(( fail == 0 )) && printf 'oci streaming gate passed\n'
exit "${fail}"
