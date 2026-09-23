#!/usr/bin/env bash
# Rehearse the one Caddy change that /openapi.json needs, off the host.
#
# The change moves /openapi.json from the Hologram Server to the site, and adds /.well-known/openapi.json as a rewrite
# onto it. Getting that wrong takes the endpoint's own contract off the air, so it is proven here first: a throwaway
# Caddy runs the real deploy/Caddyfile.hub against a stub for every upstream and a file server holding the built site.
#
#   ./rehearse-openapi-route.sh            run it, print one line per assertion, exit non-zero on the first failure
#   KEEP=1 ./rehearse-openapi-route.sh     leave the containers up for poking at http://127.0.0.1:8971
#
# Needs Docker. Nothing here touches the live host.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
WEB="$HERE/../web"
WORK="$(mktemp -d)"
NET=hub-openapi-rehearsal
PORT=8971
trap 'rc=$?; if [ "${KEEP:-}" != "1" ]; then docker rm -f rehearse-front rehearse-site rehearse-stub >/dev/null 2>&1 || true; docker network rm "$NET" >/dev/null 2>&1 || true; fi; rm -rf "$WORK"; exit $rc' EXIT

test -f "$WEB/public/openapi.json" || { echo "build the document first: (cd $WEB && node scripts/openapi.build.mjs)"; exit 1; }

# ---- the site the real one serves, reduced to the files this change touches
mkdir -p "$WORK/site/.well-known"
cp "$WEB/public/openapi.json" "$WORK/site/openapi.json"
cp "$WEB/public/robots.txt" "$WORK/site/robots.txt"
cp "$WEB/public/agent.md" "$WORK/site/agent.md"
cp "$WEB/public/.well-known/agent-card.json" "$WORK/site/.well-known/agent-card.json"
printf '{"format":"hologram.model-hub.descriptor/v1","catalog":"blake3:%064d"}\n' 0 > "$WORK/site/.well-known/model-hub.json"
printf '# guide\n' > "$WORK/site/llms.txt"
printf '<!doctype html><title>site</title>\n' > "$WORK/site/index.html"
printf '{"days":[]}\n' > "$WORK/site/archive.json"
cat > "$WORK/site/Caddyfile" <<'SITE'
{
	auto_https off
	admin off
}
:8080 {
	root * /srv
	file_server
}
SITE

# ---- one stub standing in for every upstream, on each port they really listen on
mkdir -p "$WORK/stub"
cat > "$WORK/stub/Caddyfile" <<'STUB'
{
	auto_https off
	admin off
}
:11435 {
	respond "hologram-server {path}" 200
}
:8090 {
	respond "hub-resolve {path}" 200
}
:5000 {
	respond "kappa {path}" 200
}
STUB

# ---- the real hub block, with only the host line and the TLS-only bits replaced
mkdir -p "$WORK/front"
{
	printf '{\n\tauto_https off\n\tadmin off\n}\n'
	sed -e "1s|^hub.uor.foundation {|:80 {|" "$HERE/Caddyfile.hub"
} > "$WORK/front/Caddyfile"
grep -q '^:80 {' "$WORK/front/Caddyfile" || { echo "Caddyfile.hub no longer starts with the hub.uor.foundation site block"; exit 1; }

docker network create "$NET" >/dev/null 2>&1 || true
docker rm -f rehearse-front rehearse-site rehearse-stub >/dev/null 2>&1 || true
docker run -d --name rehearse-stub --network "$NET" \
	--network-alias hub-server --network-alias hub-resolve --network-alias hub-kappa \
	-v "$WORK/stub/Caddyfile:/etc/caddy/Caddyfile:ro" caddy:2-alpine >/dev/null
docker run -d --name rehearse-site --network "$NET" --network-alias hub-site \
	-v "$WORK/site:/srv:ro" -v "$WORK/site/Caddyfile:/etc/caddy/Caddyfile:ro" caddy:2-alpine >/dev/null
docker run -d --name rehearse-front --network "$NET" -p "127.0.0.1:$PORT:80" \
	-e HUB_REGISTRY_TOKEN=rehearsal -e HUB_SERVER_TOKEN=rehearsal -e HUB_PUBLISH_TOKEN=rehearsal \
	-v "$WORK/front/Caddyfile:/etc/caddy/Caddyfile:ro" caddy:2-alpine >/dev/null

# caddy validates on start; a bad file exits the container.
sleep 2
for c in rehearse-stub rehearse-site rehearse-front; do
	[ "$(docker inspect -f '{{.State.Running}}' "$c")" = true ] || { echo "FAIL $c did not start"; docker logs "$c" 2>&1 | tail -20; exit 1; }
done

fails=0
check() { # name method path expected-status [grep-pattern] [extra curl args...]
	local name=$1 method=$2 path=$3 want=$4 pattern=${5:-}
	shift 5 2>/dev/null || shift $#
	local out code
	# No default Accept: curl would send both headers and the first one would decide the answer, which is exactly
	# the thing under test here. Each case states the Accept it means.
	out=$(curl -s -o "$WORK/body" -D "$WORK/head" -w '%{http_code}' -X "$method" "$@" "http://127.0.0.1:$PORT$path" || true)
	code=$out
	if [ "$code" != "$want" ]; then echo "FAIL $name: $path answered $code, expected $want"; fails=$((fails + 1)); return; fi
	if [ -n "$pattern" ] && ! grep -qi -- "$pattern" "$WORK/head" "$WORK/body"; then
		echo "FAIL $name: $path answered $code but nothing matched /$pattern/"; fails=$((fails + 1)); return
	fi
	echo "ok   $name: $path -> $code${pattern:+ (${pattern})}"
}

# The three readers of the bare name. `check` sends Accept: application/json by default, so each case overrides it.
echo "== one URL, three readers"
check "a browser gets the page"    GET / 200 "<title>site</title>"       -H "accept: text/html,application/xhtml+xml"
check "json by name: descriptor"   GET / 200 "descriptor/v1"             -H "accept: application/json"
check "curl gets the brief"        GET / 200 "# hub.uor.foundation"      -H "accept: */*"
check "no Accept at all: brief"    GET / 200 "Hash what arrives"         -H "accept:"
check "an unfurler still gets html" GET / 200 "<title>site</title>"      -H "accept: */*" -A "Slackbot-LinkExpanding 1.0"
check "the front door is cors-open" GET / 200 "access-control-allow-origin: \*" -H "accept: */*"
check "the brief by its own path"  GET /agent.md 200 "# hub.uor.foundation" -H "accept: */*"
# The document promises text/markdown here; a file server that guessed octet-stream would make agents download it.
check "the brief is markdown"      GET /agent.md 200 "content-type: text/markdown" -H "accept: */*"

echo "== what the change adds"
check "openapi from the site"      GET /openapi.json                  200 '"openapi": "3.1.0"'
check "openapi is cross-origin"    GET /openapi.json                  200 'access-control-allow-origin: \*'
check "openapi is cached briefly"  GET /openapi.json                  200 'cache-control: public, max-age=300'
check "well-known alias"           GET /.well-known/openapi.json      200 '"openapi": "3.1.0"'
check "agent card"                 GET /.well-known/agent-card.json   200 'Hologram Model Hub'
check "robots"                     GET /robots.txt                    200 'Disallow: /via/'

echo "== what the change must not disturb"
check "docs still on the server"   GET /docs                          200 'hologram-server /docs'
check "healthz still on it"        GET /healthz                       200 'hologram-server /healthz'
check "capabilities still on it"   GET /api/v1/capabilities           200 'hologram-server /api/v1/capabilities'
check "objects still on it"        GET "/api/v1/objects/blake3:$(printf '0%.0s' $(seq 64))" 200 'hologram-server'
check "models still on resolve"    GET /api/models                    200 'hub-resolve /api/models'
check "resolve still on resolve"   GET /a/b/resolve/main/c.json       200 'hub-resolve'
check "mcp still on resolve"       GET /mcp                           200 'hub-resolve /mcp'
check "registry still on kappa"    GET /v2/                           200 'kappa'
check "descriptor still rewrites"  GET /                              200 'hologram.model-hub.descriptor/v1' -H "accept: application/json"
check "llms.txt still served"      GET /llms.txt                      200 '# guide'
# A write under /v2/<owner>/<name>/ never reaches the registry: the @ollama matcher sends it to the read-only
# shim, which refuses anything but GET and HEAD. The token gate guards the hub's own namespace and /v2/ itself.
check "registry write still shut"  PUT /v2/model-hub/index/blobs/uploads/ 401 'write requires a registry token'
check "model write hits the shim"  PUT /v2/x/y/blobs/uploads/         200 'hub-resolve'
check "publish still shut"         GET /api/v1/objects                401 'publish requires a publisher token'
check "grpc still shut"            GET /hologram.live.v1.HologramLive/Handshake 404 'not found'

echo
if [ "$fails" -gt 0 ]; then echo "$fails failed"; exit 1; fi
echo "all assertions passed against the real Caddyfile.hub"
[ "${KEEP:-}" = "1" ] && echo "containers left up on http://127.0.0.1:$PORT"
exit 0
