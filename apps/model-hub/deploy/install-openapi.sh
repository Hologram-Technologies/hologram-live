#!/usr/bin/env bash
# Put the endpoint's own OpenAPI document on the air. Run on the host, as root.
#
#   ./install-openapi.sh check      say what would change and what is already true; changes nothing
#   ./install-openapi.sh install    apply, validate, reload, verify, and roll back by itself if verification fails
#   ./install-openapi.sh verify     probe the live endpoint only
#   ./install-openapi.sh rollback   restore the newest backup this script made and reload
#
# What it changes, and nothing else: three surgical edits inside the existing gethologram.ai block of the front
# Caddyfile. The block is edited IN PLACE, never rewritten and never renamed: Caddy bind-mounts that single file and
# the tokens live inline in it.
#   1. /openapi.json leaves the Hologram Server's route list, so it falls through to the site, which now ships the
#      document that describes the whole endpoint rather than the server's own six paths.
#   2. /.well-known/openapi.json is rewritten onto /openapi.json, and /openapi.json and /robots.txt are made readable
#      from another origin.
#   3. GET / answers agent.md to anything that is neither a browser nor asking for JSON, so `curl gethologram.ai`
#      returns the hub in one screen instead of 77 KB of markup; and the front door becomes readable cross-origin.
#   4. GET / declares Vary: Accept, so a shared cache cannot serve one caller's representation to another.
#   5. A malformed object address refuses in the documented JSON shape instead of the catch-all's text/plain.
#   6. /models and /registry answer their own brief to anything that is not a browser, so the endpoint the site
#      shows beside each section heading is a line you can run rather than a name you have to interpret.
#      /docs carries its slash: /docs without one is the server's own API reference, on a different upstream.
#
# Each edit is applied only if it is missing, so this is safe to run against a host that has had an earlier version
# of this script: it adds what is absent and leaves the rest alone.
# /docs is untouched. It loads the spec from /openapi.json, so it renders the new document with no change to it.
#
# The route change itself is rehearsed off the host by ./rehearse-openapi-route.sh, which runs the real
# Caddyfile.hub against stubs. This script is the part that can only be done here.
set -euo pipefail

CADDYFILE=${CADDYFILE:-/root/twenty/Caddyfile}
SITE=${SITE:-/root/hub/site}
HOST=${HOST:-gethologram.ai}
STAMP=$(date -u +%Y-%m-%d)
BACKUP="$CADDYFILE.bak-$STAMP-openapi"
MARK='rewrite /.well-known/openapi.json /openapi.json'
MARK_ARRIVING='rewrite @arriving /agent.md'
MARK_VARY='header / Vary Accept'
MARK_MALFORMED='@object_malformed'
MARK_SECTIONS='@section_brief'

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
	grep -qE "^([^{]*, )?$HOST(, [^{]*)? \{" "$CADDYFILE" || die "$CADDYFILE has no site block naming $HOST: this script only edits that block"
	[ -f "$SITE/openapi.json" ] || die "$SITE/openapi.json is missing. Build the site from a revision that contains apps/model-hub/web/public/openapi.json first (build-site.sh), or the flip would take /openapi.json off the air."
	grep -q '"openapi": "3.1.0"' "$SITE/openapi.json" || die "$SITE/openapi.json is not an OpenAPI 3.1 document"
	grep -q '"title": "Hologram Model Hub"' "$SITE/openapi.json" || die "$SITE/openapi.json is not the hub's document: the site build is older than this change"
	[ -f "$SITE/robots.txt" ] || die "$SITE/robots.txt is missing: the site build is older than this change"
	[ -f "$SITE/agent.md" ] || die "$SITE/agent.md is missing: the site build is older than this change, and the bare name would answer markup to an arriving agent"
	[ -f "$SITE/.well-known/agent-card.json" ] || die "$SITE/.well-known/agent-card.json is missing: the site build is older than this change"
	[ -n "$CADDY" ] || CADDY=$(caddy_container)
}

# All three markers, not one: an earlier version of this script applied only the first two, and a host in that
# state must still be treated as needing work rather than as done.
applied() {
	grep -qF "$MARK" "$CADDYFILE" && grep -qF "$MARK_ARRIVING" "$CADDYFILE" \
		&& grep -qF "$MARK_VARY" "$CADDYFILE" && grep -qF "$MARK_MALFORMED" "$CADDYFILE" \
		&& grep -qF "$MARK_SECTIONS" "$CADDYFILE" \
		&& ! grep -qE 'path /healthz /openapi\.json ' "$CADDYFILE"
}

apply_edits() {
	python3 - "$CADDYFILE" "$HOST" <<'PY'
import io, re, sys
path, host = sys.argv[1], sys.argv[2]
text = io.open(path, encoding="utf-8").read()

# The label may carry several names (`gethologram.ai, hub.uor.foundation {`); find the block whose label names this host.
label = re.search(r"^(?:[^{\n]*, )?" + re.escape(host) + r"(?:, [^{\n]*)? \{", text, re.M)
if not label:
    raise SystemExit(f"no site block names {host}")
start = label.start()
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

# Three independent edits, each applied only when missing, so a host that got an earlier version of this script
# is brought the rest of the way rather than refused outright.
done = []

# 1. /openapi.json leaves the Hologram Server's route list.
block, n = re.subn(r"(path /healthz )(/openapi\.json )", r"\1", block, count=1)
if n:
    done.append("moved /openapi.json to the site")

# 2. the well-known alias and the cross-origin headers, next to the ones already there.
anchor = '\t\theader /llms.txt Access-Control-Allow-Origin "*"\n'
if anchor not in block:
    raise SystemExit("the site handler does not carry the llms.txt header line; refusing to guess where to insert")
if "rewrite /.well-known/openapi.json" not in block:
    block = block.replace(anchor, anchor + (
        '\t\t# One document, two paths: an agent handed nothing but the host name looks under /.well-known first.\n'
        '\t\trewrite /.well-known/openapi.json /openapi.json\n'
        '\t\t# The whole endpoint in one OpenAPI document, built with the site. A browser agent reads it cross-origin.\n'
        '\t\theader /openapi.json Access-Control-Allow-Origin "*"\n'
        '\t\theader /openapi.json Cache-Control "public, max-age=300"\n'
        '\t\theader /robots.txt Access-Control-Allow-Origin "*"\n'
    ), 1)
    done.append("added the /.well-known alias and the cross-origin headers")

# 3. the arriving agent. curl, node fetch and python requests all send Accept: */*, so without this the bare name
#    answers markup to every one of them.
agent_anchor = '\t\trewrite @agent /.well-known/model-hub.json\n'
if agent_anchor not in block:
    raise SystemExit("the site handler does not carry the descriptor rewrite; refusing to guess where to insert")
if "rewrite @arriving /agent.md" not in block:
    block = block.replace(agent_anchor, agent_anchor + (
        '\t\t@arriving {\n'
        '\t\t\tpath /\n'
        '\t\t\tnot header Accept *text/html*\n'
        '\t\t\tnot header Accept *application/json*\n'
        '\t\t\t# Link unfurlers send */* too, and a preview of markdown is a worse card than a preview of the page.\n'
        '\t\t\tnot header User-Agent *bot*\n'
        '\t\t\tnot header User-Agent *Bot*\n'
        '\t\t\tnot header User-Agent *Slack*\n'
        '\t\t\tnot header User-Agent *Twitter*\n'
        '\t\t\tnot header User-Agent *Discord*\n'
        '\t\t\tnot header User-Agent *facebookexternalhit*\n'
        '\t\t}\n'
        '\t\trewrite @arriving /agent.md\n'
        '\t\t# The front door has to be readable from another origin, or a browser-resident agent cannot start at all.\n'
        '\t\theader / Access-Control-Allow-Origin "*"\n'
        '\t\theader /agent.md Access-Control-Allow-Origin "*"\n'
    ), 1)
    done.append("made the bare name answer agent.md")

# 4. Vary: Accept, next to the rewrite that makes it necessary.
arriving_anchor = '\t\trewrite @arriving /agent.md\n'
if arriving_anchor in block and "header / Vary Accept" not in block:
    block = block.replace(arriving_anchor, arriving_anchor + (
        '\t\t# Three representations chosen by Accept means caches have to be told, or one caller\'s answer is served to\n'
        '\t\t# the next caller who asked for something else. Silent, intermittent, and invisible from here.\n'
        '\t\theader / Vary Accept\n'
    ), 1)
    done.append("declared Vary: Accept on the front door")

# 5. A malformed object address refuses in the documented shape.
read_anchor = '\t@read {\n'
if read_anchor not in block:
    raise SystemExit("the server read matcher is not where expected; refusing to guess")
if "@object_malformed" not in block:
    block = block.replace(read_anchor, (
        '\t# An address is blake3: and 64 hex characters. Anything else under this prefix is a client error, and it has to\n'
        '\t# refuse in the shape the document promises rather than falling through to the catch-all\'s text/plain.\n'
        '\t@object_malformed {\n'
        '\t\tpath /api/v1/objects/*\n'
        '\t\tnot path_regexp ^/api/v1/objects/blake3:[0-9a-f]{64}$\n'
        '\t\tnot path /api/v1/objects/search\n'
        '\t}\n'
        '\thandle @object_malformed {\n'
        '\t\theader Content-Type "application/json"\n'
        '\t\theader Access-Control-Allow-Origin "*"\n'
        '\t\trespond `{"code":"LIVE_BAD_REQUEST","message":"an object address is blake3: followed by 64 hexadecimal characters"}` 400\n'
        '\t}\n\n'
    ) + read_anchor, 1)
    done.append("gave a malformed address the documented error shape")

# 6. A brief per section, at the section's own URL.
robots_anchor = '\t\theader /robots.txt Access-Control-Allow-Origin "*"\n'
if robots_anchor not in block:
    raise SystemExit("the robots header is not where expected; refusing to guess where to insert")
if "@section_brief" not in block:
    block = block.replace(robots_anchor, robots_anchor + (
        '\t\t# Each section answers the way the root does: the page to a browser, the section\'s own brief to everything\n\t\t# else. The site shows an endpoint beside every section heading, and a name is not usable -- an agent that\n\t\t# follows it gets the browse page, a quarter of a megabyte of markup. This adds no API: every route the briefs\n\t\t# name already existed. /v2/ is deliberately untouched, because it is a protocol endpoint and OCI clients\n\t\t# depend on exactly what it returns.\n\t\t@section_brief {\n\t\t\tpath /models /models/ /registry /registry/ /spaces /spaces/ /buckets /buckets/ /docs/\n\t\t\tnot header Accept *text/html*\n\t\t\tnot header User-Agent *bot*\n\t\t\tnot header User-Agent *Bot*\n\t\t\tnot header User-Agent *Slack*\n\t\t\tnot header User-Agent *Twitter*\n\t\t\tnot header User-Agent *Discord*\n\t\t\tnot header User-Agent *facebookexternalhit*\n\t\t}\n\t\trewrite @section_brief /{path.0}.md\n\t\theader /models* Vary Accept\n\t\theader /registry* Vary Accept\n\t\theader /spaces* Vary Accept\n\t\theader /buckets* Vary Accept\n\t\theader /docs/* Vary Accept\n\t\theader /models.md Access-Control-Allow-Origin "*"\n\t\theader /registry.md Access-Control-Allow-Origin "*"\n\t\theader /spaces.md Access-Control-Allow-Origin "*"\n\t\theader /buckets.md Access-Control-Allow-Origin "*"\n\t\theader /docs.md Access-Control-Allow-Origin "*"\n'
    ), 1)
    done.append("gave /models and /registry their own briefs")
else:
    # Already there, from when only two sections had one. Widen it to all five rather than leave the three new
    # tags pointing at an address that answers a page.
    old_paths = "\t\t\tpath /models /models/ /registry /registry/\n"
    new_paths = "\t\t\tpath /models /models/ /registry /registry/ /spaces /spaces/ /buckets /buckets/ /docs/\n"
    if old_paths in block:
        block = block.replace(old_paths, new_paths, 1)
        done.append("widened the section briefs to spaces, buckets and docs")
    old_cors = '\t\theader /registry.md Access-Control-Allow-Origin "*"\n'
    new_cors = ('\t\theader /registry.md Access-Control-Allow-Origin "*"\n'
                '\t\theader /spaces.md Access-Control-Allow-Origin "*"\n'
                '\t\theader /buckets.md Access-Control-Allow-Origin "*"\n'
                '\t\theader /docs.md Access-Control-Allow-Origin "*"\n')
    if old_cors in block and "/spaces.md Access-Control" not in block:
        block = block.replace(old_cors, new_cors, 1)
    old_vary = "\t\theader /registry* Vary Accept\n"
    new_vary = ("\t\theader /registry* Vary Accept\n"
                "\t\theader /spaces* Vary Accept\n"
                "\t\theader /buckets* Vary Accept\n"
                "\t\theader /docs/* Vary Accept\n")
    if old_vary in block and "/spaces* Vary" not in block:
        block = block.replace(old_vary, new_vary, 1)

if not done:
    raise SystemExit("already applied; nothing to change")
# Written in place: Caddy bind-mounts this single file, so a rename would break the mount.
with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(text[:start] + block + text[i + 1:])
print("edited the block in place: " + "; ".join(done))
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
	probe "vary on the root"    /                             200 'vary: Accept'
	probe "malformed address"   /api/v1/objects/notanaddress  400 'LIVE_BAD_REQUEST'
	probe "the models brief"    /models                       200 "$HOST/models"
	probe "the registry brief"  /registry                     200 "$HOST/registry"
	probe "the spaces brief"    /spaces                       200 "$HOST/spaces"
	probe "the buckets brief"   /buckets                      200 "$HOST/buckets"
	probe "the docs brief"      /docs/                        200 "$HOST/docs"
	# The headline claim: what curl actually gets from the bare name.
	if curl -s --max-time 20 -H 'accept: */*' "https://$HOST/" | head -1 | grep -q "^# $HOST"; then
		echo "  ok   the bare name answers the brief"
	else
		echo "  FAIL the bare name still answers markup: curl https://$HOST returns HTML, not agent.md"
		fails=$((fails + 1))
	fi
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
