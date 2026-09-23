#!/usr/bin/env bash
# Swap in a new hub-resolve.mjs, prove it answers, and put the old one back by itself if it does not.
#
#   ./install-shim.sh check              what is running now, and what would change
#   ./install-shim.sh install <file>     back up, swap, restart, verify, roll back on any failure
#   ./install-shim.sh rollback           restore the newest backup this script made
#
# The shim is the busiest thing on the endpoint: the Hugging Face dialect, the Ollama and OCI dialects, /mcp and
# every file redirect all run through it. It is one file with no dependencies, bind-mounted read-only into the
# `resolve` container, so a swap is a copy and a restart — and a bad copy takes all of that down at once, which is
# why nothing here is done without a backup and a verification.
#
# Contracts are proven off the host first by deploy/qa/shim-contract.mjs, which runs the same file against
# fixtures. This script checks that the deployed thing is alive and answering, not that it is correct.
set -euo pipefail

HUB=${HUB:-/root/hub}
TARGET="$HUB/bin/hub-resolve.mjs"
HOST=${HOST:-hub.uor.foundation}
STAMP=$(date -u +%Y-%m-%dT%H%M%SZ)

die() { echo "FAIL $*" >&2; exit 1; }
say() { echo "  $*"; }

compose() { docker compose -f "$HUB/docker-compose.yml" "$@"; }

verify() {
	local fails=0
	probe() { # name path expected-status [pattern]
		local name=$1 path=$2 want=$3 pattern=${4:-} code head
		head=$(mktemp)
		code=$(curl -s -o /tmp/.shim-verify -D "$head" -w '%{http_code}' --max-time 20 "https://$HOST$path" || echo 000)
		if [ "$code" != "$want" ]; then echo "  FAIL $name: $path -> $code, expected $want"; fails=$((fails + 1)); rm -f "$head"; return; fi
		if [ -n "$pattern" ] && ! grep -qi -- "$pattern" "$head" /tmp/.shim-verify; then echo "  FAIL $name: $path -> $code but no /$pattern/"; fails=$((fails + 1)); rm -f "$head"; return; fi
		echo "  ok   $name"; rm -f "$head"
	}
	# One probe per dialect the shim owns, so a swap that breaks any of them is caught here rather than by a user.
	probe "the model list"      "/api/models?limit=1"                                   200 'modelId'
	probe "one model"           "/api/models/sentence-transformers/all-MiniLM-L6-v2"     200 '"sha"'
	probe "its files"           "/api/models/sentence-transformers/all-MiniLM-L6-v2/tree/main" 200 '"oid"'
	probe "a file redirect"     "/sentence-transformers/all-MiniLM-L6-v2/resolve/main/config.json" 302 'x-hub-source'
	probe "the checksum file"   "/sentence-transformers/all-MiniLM-L6-v2/resolve/main/SHA256SUMS"  200 'config.json'
	probe "source health"       "/api/hub/health"                                        200 'huggingface'
	probe "the OCI manifest"    "/v2/sentence-transformers/all-minilm-l6-v2/tags/list"   200 'tags'
	probe "a pin is honoured"   "/via/huggingface/sentence-transformers/all-MiniLM-L6-v2/resolve/main/config.json" 302 'x-hub-source: huggingface.co'
	probe "a bad parameter"     "/api/models?limit=0"                                    400 'BadParameter'
	# MCP is a POST, so it does not fit the probe helper.
	if curl -s --max-time 20 -X POST "https://$HOST/mcp" -H 'content-type: application/json' \
		-H 'accept: application/json, text/event-stream' \
		-d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | grep -q search_models; then
		echo "  ok   the MCP tools"
	else echo "  FAIL the MCP tools: tools/list did not list search_models"; fails=$((fails + 1)); fi
	rm -f /tmp/.shim-verify
	return $fails
}

case "${1:-check}" in
check)
	[ -f "$TARGET" ] || die "no shim at $TARGET"
	say "running: $(md5sum "$TARGET" | cut -c1-12)  $(stat -c '%s bytes, %y' "$TARGET" | cut -c1-40)"
	say "container: $(docker ps --filter name=resolve --format '{{.Names}} {{.Status}}')"
	say "verifying the live endpoint as it stands:"
	verify || true
	;;
install)
	NEW=${2:-}
	[ -n "$NEW" ] && [ -f "$NEW" ] || die "usage: $0 install <new hub-resolve.mjs>"
	node --check "$NEW" || die "$NEW does not parse; refusing to deploy it"
	grep -q "hub-resolve" "$NEW" || grep -q "HUB_DATA" "$NEW" || die "$NEW does not look like the shim"
	if cmp -s "$NEW" "$TARGET"; then say "identical to what is running; nothing to do"; exit 0; fi

	BACKUP="$HUB/bin/hub-resolve.mjs.bak-$STAMP"
	cp -a "$TARGET" "$BACKUP"
	say "backed up to $BACKUP"
	# Written in place: the container bind-mounts this exact path, so a rename would break the mount.
	cat "$NEW" > "$TARGET"
	say "swapped in $(md5sum "$TARGET" | cut -c1-12)"
	compose restart resolve >/dev/null
	say "restarted"
	sleep 4

	if ! verify; then
		echo "FAIL verification failed after the swap" >&2
		cat "$BACKUP" > "$TARGET"
		compose restart resolve >/dev/null
		sleep 4
		say "rolled back to $(md5sum "$TARGET" | cut -c1-12)"
		verify && say "the previous shim is answering again" || echo "  the rollback did not verify either — look at: docker logs \$(docker ps -qf name=resolve)" >&2
		exit 1
	fi
	say "installed and verified"
	;;
verify)
	verify || die "the live endpoint does not verify"
	;;
rollback)
	BACKUP=$(ls -1t "$HUB"/bin/hub-resolve.mjs.bak-* 2>/dev/null | head -1 || true)
	[ -n "$BACKUP" ] || die "no backup made by this script was found"
	cat "$BACKUP" > "$TARGET"
	compose restart resolve >/dev/null
	sleep 4
	say "restored $BACKUP"
	verify || die "the restored shim does not verify"
	;;
*)
	die "usage: $0 {check|install <file>|verify|rollback}"
	;;
esac
