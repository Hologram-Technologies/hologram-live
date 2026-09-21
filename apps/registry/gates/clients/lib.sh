#!/usr/bin/env bash
# Gate C: clients nobody modified, against Hologram Registry. Shared settings.
set -euo pipefail
US=${US:-127.0.0.1:5000}      # Hologram Registry
PEER=${PEER:-127.0.0.1:5001}  # the reference registry, as a peer to copy through

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
same() { [ "$1" = "$2" ] || fail "$3: $1 is not $2"; }
