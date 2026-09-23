#!/usr/bin/env bash
# Put the endpoint's own OpenAPI document on the air. Run on the host, as root.
#
#   ./install-openapi.sh check      say what would change and what is already true; changes nothing
#   ./install-openapi.sh install    apply, validate, reload, verify, and roll back by itself if verification fails
#   ./install-openapi.sh verify     probe the live endpoint only
#   ./install-openapi.sh rollback   restore the newest backup this script made and reload
#
# What it changes, and nothing else: two surgical edits inside the existing hub.uor.foundation block of the front
# Caddyfile. The block is edited IN PLACE, never rewritten and never renamed: Caddy bind-mounts that single file and
# the tokens live inline in it.
#   1. /openapi.json leaves the Hologram Server's route list, so it falls through to the site, which now ships the
#      document that describes the whole endpoint rather than the server's own six paths.
#   2. /.well-known/openapi.json is rewritten onto /openapi.json, and /openapi.json and /robots.txt are made readable
#      from another origin.
# /docs is untouched. It loads the spec from /openapi.json, so it renders the new document with no change to it.
#
# The route change itself is rehearsed off the host by ./rehearse-openapi-route.sh, which runs the real
# Caddyfile.hub against stubs. This script is the part that can only be done here.
set -euo pipefail

CADDYFILE=${CADDYFILE:-/root/twenty/Caddyfile}
SITE=${SITE:-/root/hub/site}
HOST=${HOST:-hub.uor.foundation}
STAMP=$(date -u +%Y-%m-%d)
BACKUP="$CADDYFILE.bak-$STAMP-openapi"
MARK='rewrite /.well-known/openapi.json /openapi.json'

die() { echo "FAIL $*" >&2; exit 1; }
say() { echo "  $*"; }

caddy_container() {
	local found
	found=$(docker ps --format '{{.Names}}' | grep -i caddy || true)
	[ "$(echo "$found" | grep -c .)" = 1 ] || die "expected exactly one running caddy container, found: ${found:-none}. Set CADDY=<name>."
	echo "$found"
}
CADDY=${CADDY:-}

preconditions() {
	[ "$(id -u)" = 0 ] || die "run as root"
	[ -f "$CADDYFILE" ] || die "no Caddyfile at $CADDYFILE"
	grep -q "^$HOST {" "$CADDYFILE" || die "$CADDYFILE has no '$HOST {' block: this script only edits that block"
	[ -f "$SITE/openapi.json" ] || die "$SITE/openapi.json is missing. Build the site from a revision that contains apps/model-hub/web/public/openapi.json first (build-site.sh), or the flip would take /openapi.json off the air."
	grep -q '"openapi": "3.1.0"' "$SITE/openapi.json" || die "$SITE/openapi.json is not an OpenAPI 3.1 document"
	grep -q '"title": "Hologram Model Hub"' "$SITE/openapi.json" || die "$SITE/openapi.json is not the hub's document: the site build is older than this change"
	[ -f "$SITE/robots.txt" ] || die "$SITE/robots.txt is missing: the site build is older than this change"
	[ -f "$SITE/agent.md" ] || die "$SITE/agent.md is missing: the site build is older than this change, and the bare name would answer markup to an arriving agent"
	[ -f "$SITE/.well-known/agent-card.json" ] || die "$SITE/.well-known/agent-card.json is missing: the site build is older than this change"
	[ -n "$CADDY" ] || CADDY=$(caddy_container)
}

applied() { grep -qF "$MARK" "$CADDYFILE"; }

apply_edits() {
	python3 - "$CADDYFILE" "$HOST" <<'PY'
import io, re, sys
path, host = sys.argv[1], sys.argv[2]
text = io.open(path, encoding="utf-8").read()

start = text.index(f"{host} {{")
depth, i = 0, start
while True:
    if text[i] == "{":
        depth += 1
    elif text[i] == "}":
        depth -= 1
        if depth == 0:
            break
    i += 1
block = text[start:i + 1]
before = block

# Applying twice would duplicate the rewrite and both headers. The caller guards this too; guard it here as well,
# because this half is the half that writes.
if 'rewrite /.well-known/openapi.json' in block:
    raise SystemExit('already applied; refusing to apply twice')

# 1. /openapi.json leaves the Hologram Server's route list.
block, n = re.subn(r"(path /healthz )(/openapi\.json )", r"\1", block, count=1)
if n == 0 and "/openapi.json" in block.split("handle {")[0]:
    raise SystemExit("the server route list does not look as expected; refusing to guess")

# 2. the well-known alias and the two cross-origin headers, next to the ones already there.
anchor = '\t\theader /llms.txt Access-Control-Allow-Origin "*"\n'
if anchor not in block:
    raise SystemExit("the site handler does not carry the llms.txt header line; refusing to guess where to insert")
addition = (
    '\t\t# One document, two paths: an agent handed nothing but the host name looks under /.well-known first.\n'
    '\t\trewrite /.well-known/openapi.json /openapi.json\n'
    '\t\t# The whole endpoint in one OpenAPI document, built with the site. A browser agent reads it cross-origin.\n'
    '\t\theader /openapi.json Access-Control-Allow-Origin "*"\n'
    '\t\theader /openapi.json Cache-Control "public, max-age=300"\n'
    '\t\theader /robots.txt Access-Control-Allow-Origin "*"\n'
)
block = block.replace(anchor, anchor + addition, 1)

if block == before:
    raise SystemExit("nothing changed; refusing to write")
# Written in place: Caddy bind-mounts this single file, so a rename would break the mount.
with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(text[:start] + block + text[i + 1:])
print("edited the block in place")
PY
}

verify() {
	local fails=0
	probe() { # name path expected-status [pattern]
		local name=$1 path=$2 want=$3 pattern=${4:-} code head
		head=$(mktemp); code=$(curl -s -o /tmp/.hub-verify-body -D "$head" -w '%{http_code}' -H 'origin: https://example.com' "https://$HOST$path" || echo 000)
		if [ "$code" != "$want" ]; then echo "  FAIL $name: $path -> $code, expected $want"; fails=$((fails + 1)); rm -f "$head"; return; fi
		if [ -n "$pattern" ] && ! grep -qi -- "$pattern" "$head" /tmp/.hub-verify-body; then echo "  FAIL $name: $path -> $code but no /$pattern/"; fails=$((fails + 1)); rm -f "$head"; return; fi
		echo "  ok   $name: $path -> $code"; rm -f "$head"
	}
	probe "the document"        /openapi.json                 200 '"title": "Hologram Model Hub"'
	probe "cross-origin"        /openapi.json                 200 'access-control-allow-origin: \*'
	probe "well-known alias"    /.well-known/openapi.json     200 '"openapi": "3.1.0"'
	probe "agent card"          /.well-known/agent-card.json  200 'Hologram Model Hub'
	probe "robots"              /robots.txt                   200 'Disallow: /via/'
	probe "the brief"           /agent.md                     200 'Hash what arrives'
	probe "docs still render"   /docs                         200 'openapi.json'
	probe "health untouched"    /healthz                      200 'ready'
	probe "capabilities"        /api/v1/capabilities          200 'operations'
	probe "descriptor"          /.well-known/model-hub.json   200 'descriptor/v1'
	probe "models dialect"      "/api/models?limit=1"         200 'modelId'
	probe "registry"            /v2/                          200 ''
	probe "guide"               /llms.txt                     200 'HF_ENDPOINT'
	rm -f /tmp/.hub-verify-body
	return $fails
}

restore() {
	[ -f "$BACKUP" ] || die "no backup at $BACKUP to restore"
	cat "$BACKUP" > "$CADDYFILE"          # in place: never mv onto a bind-mounted file
	docker exec "$CADDY" caddy reload --config /etc/caddy/Caddyfile >/dev/null 2>&1 || docker restart "$CADDY" >/dev/null
	say "restored $CADDYFILE from $BACKUP and reloaded"
}

case "${1:-check}" in
check)
	preconditions
	say "caddy container: $CADDY"
	say "site document:   $(grep -o '"version": "[^"]*"' "$SITE/openapi.json" | head -1), $(wc -c <"$SITE/openapi.json") bytes"
	if applied; then say "already applied: nothing to do"; else say "would apply both edits to the $HOST block of $CADDYFILE"; fi
	say "verifying the live endpoint as it stands:"
	verify || true
	;;
install)
	preconditions
	if applied; then say "already applied"; verify || die "the endpoint does not verify even though the edits are in place"; say "nothing to do"; exit 0; fi
	cp -a "$CADDYFILE" "$BACKUP"
	say "backed up to $BACKUP"
	apply_edits
	if ! docker exec "$CADDY" caddy validate --config /etc/caddy/Caddyfile >/dev/null 2>&1; then
		echo "FAIL the edited Caddyfile does not validate" >&2
		restore
		exit 1
	fi
	say "validated"
	docker exec "$CADDY" caddy reload --config /etc/caddy/Caddyfile >/dev/null || { restore; die "reload failed"; }
	say "reloaded"
	sleep 2
	if ! verify; then
		echo "FAIL verification failed after the reload" >&2
		restore
		exit 1
	fi
	say "installed and verified"
	;;
verify)
	verify || die "the live endpoint does not verify"
	;;
rollback)
	[ -n "$CADDY" ] || CADDY=$(caddy_container)
	BACKUP=$(ls -1t "$CADDYFILE".bak-*-openapi 2>/dev/null | head -1 || true)
	[ -n "$BACKUP" ] || die "no backup made by this script was found"
	restore
	;;
*)
	die "usage: $0 {check|install|verify|rollback}"
	;;
esac
