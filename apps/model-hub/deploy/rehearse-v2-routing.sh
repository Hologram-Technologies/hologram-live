#!/usr/bin/env bash
# Rehearse the /v2 routing of deploy/Caddyfile.hub off the host, with no Docker: a local `caddy` binary runs the real
# Caddyfile against one stub for every upstream, and the real hub-resolve.mjs runs with an empty index in front of
# the registry stub, so the fallback is exercised for real.
#
#   ./rehearse-v2-routing.sh            one line per assertion; exit non-zero if any fails
#   CADDY=/path/to/caddy               a caddy binary other than the one on PATH
#
# Every assertion says which upstream answered: the stubs echo their own name and the path they were asked for.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
CADDY=${CADDY:-caddy}
WORK="$(mktemp -d)"
FRONT=8971; RESOLVE=8090; STUB_REG=5000; STUB_SERVER=11435; STUB_SITE=8080
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done; rm -rf "$WORK"; }
trap cleanup EXIT

command -v "$CADDY" >/dev/null || { echo "need caddy (https://caddyserver.com/api/download)"; exit 1; }
command -v node >/dev/null || { echo "need node"; exit 1; }

# ---- stubs: each answers with its own name and the path, on the port the real service listens on
cat > "$WORK/stubs.Caddyfile" <<STUB
{
	auto_https off
	admin off
}
:$STUB_SERVER {
	respond "hologram-server {path}" 200
}
:$STUB_SITE {
	respond "hub-site {path}" 200
}
:$STUB_REG {
	# what the registry answers for an image somebody pushed: a manifest with its digest header
	@manifest path /v2/you/app/manifests/*
	handle @manifest {
		header Docker-Content-Digest "sha256:9c3724af63a7e72cbc154a7c87f5d8c51f453481fa90b21b7824c75bc7817616"
		header Content-Type "application/vnd.oci.image.manifest.v1+json"
		respond "kappa manifest {path}" 200
	}
	respond "kappa {path}" 200
}
STUB
"$CADDY" run --config "$WORK/stubs.Caddyfile" --adapter caddyfile >"$WORK/stubs.log" 2>&1 &
pids+=($!)

# ---- the real resolver, empty index, pointed at the registry stub
mkdir -p "$WORK/data" "$WORK/state"
printf '[]\n' > "$WORK/data/models.json"
HUB_DATA="$WORK/data" HUB_STATE="$WORK/state" PORT=$RESOLVE HUB_REGISTRY="http://127.0.0.1:$STUB_REG" \
	node "$HERE/hub-resolve.mjs" >"$WORK/resolve.log" 2>&1 &
pids+=($!)

# ---- the real Caddyfile.hub, on a local port, with every upstream name pointed at the stubs
sed -e "s#^hub.uor.foundation {#:$FRONT {#" \
	-e "s#hub-resolve:8090#127.0.0.1:$RESOLVE#g" -e "s#hub-kappa:5000#127.0.0.1:$STUB_REG#g" \
	-e "s#hub-server:11435#127.0.0.1:$STUB_SERVER#g" -e "s#hub-site:8080#127.0.0.1:$STUB_SITE#g" \
	-e "s#hub-account:8091#127.0.0.1:$STUB_SERVER#g" \
	"$HERE/Caddyfile.hub" > "$WORK/front.Caddyfile"
printf '{\n\tauto_https off\n\tadmin off\n}\n' | cat - "$WORK/front.Caddyfile" > "$WORK/front.full" && mv "$WORK/front.full" "$WORK/front.Caddyfile"
"$CADDY" validate --config "$WORK/front.Caddyfile" --adapter caddyfile >"$WORK/validate.log" 2>&1 || { echo "FAIL Caddyfile.hub does not validate"; cat "$WORK/validate.log"; exit 1; }
HUB_SERVER_TOKEN=st HUB_PUBLISH_TOKEN=pt HUB_REGISTRY_TOKEN=${HUB_REGISTRY_TOKEN:-rt} "$CADDY" run --config "$WORK/front.Caddyfile" --adapter caddyfile >"$WORK/front.log" 2>&1 &
pids+=($!)
for _ in $(seq 1 50); do curl -s -o /dev/null "http://127.0.0.1:$FRONT/v2/" && curl -s -o /dev/null "http://127.0.0.1:$RESOLVE/api/models" && break; sleep 0.2; done

fails=0
check() { # name method path expected-status grep-pattern [extra curl args...]
	local name=$1 method=$2 path=$3 want=$4 pattern=$5; shift 5
	local code
	code=$(curl -s -o "$WORK/body" -D "$WORK/head" -w '%{http_code}' -X "$method" "$@" "http://127.0.0.1:$FRONT$path" || true)
	if [ "$code" != "$want" ]; then echo "FAIL $name: $path answered $code, expected $want"; fails=$((fails + 1)); return; fi
	if ! grep -qi -- "$pattern" "$WORK/head" "$WORK/body"; then echo "FAIL $name: $path answered $code but nothing matched /$pattern/"; fails=$((fails + 1)); return; fi
	echo "ok   $name: $method $path -> $code ($pattern)"
}

echo "== an anonymous write is refused, under every name, before it reaches the registry"
check "anonymous upload refused"   POST  /v2/you/app/blobs/uploads/          401 "write requires a registry token"
check "anonymous manifest refused" PUT   /v2/you/app/manifests/v1            401 "write requires a registry token"
check "anonymous delete refused"   DELETE /v2/you/app/manifests/sha256:0     401 "write requires a registry token"
check "the hub's namespace too"    PUT   /v2/model-hub/index/manifests/today 401 "write requires a registry token"

echo "== with the token, every write under any name reaches the registry"
TOK=(-H "authorization: Bearer ${HUB_REGISTRY_TOKEN:-rt}")
check "start an upload"           POST  /v2/you/app/blobs/uploads/          200 "kappa /v2/you/app/blobs/uploads/" "${TOK[@]}"
check "append to it"              PATCH /v2/you/app/blobs/uploads/abc       200 "kappa /v2/you/app/blobs/uploads/abc" "${TOK[@]}"
check "finish it"                 PUT   "/v2/you/app/blobs/uploads/abc?digest=sha256:0" 200 "kappa /v2/you/app/blobs/uploads/abc" "${TOK[@]}"
check "push a manifest"           PUT   /v2/you/app/manifests/v1            200 "kappa manifest /v2/you/app/manifests/v1" "${TOK[@]}"
check "delete a manifest"         DELETE /v2/you/app/manifests/sha256:0     200 "kappa manifest /v2/you/app/manifests/sha256:0" "${TOK[@]}"
check "delete a blob"             DELETE /v2/you/app/blobs/sha256:0         200 "kappa /v2/you/app/blobs/sha256:0" "${TOK[@]}"
check "the hub's own namespace"   PUT   /v2/model-hub/index/manifests/today 200 "kappa /v2/model-hub/index/manifests/today" "${TOK[@]}"

echo "== reads of a pushed image go through the resolver's fallback to the registry, headers intact"
check "manifest of a pushed image" GET  /v2/you/app/manifests/v1             200 "kappa manifest /v2/you/app/manifests/v1"
check "its digest header survives" GET  /v2/you/app/manifests/v1             200 "docker-content-digest: sha256:9c3724af"
check "HEAD too"                   HEAD /v2/you/app/manifests/v1             200 "docker-content-digest: sha256:9c3724af"
check "a blob of it"               GET  /v2/you/app/blobs/sha256:0           200 "kappa /v2/you/app/blobs/sha256:0"
check "its tags"                   GET  /v2/you/app/tags/list                200 "kappa /v2/you/app/tags/list"
check "the query string survives"  GET  "/v2/you/app/tags/list?n=1"          200 "kappa /v2/you/app/tags/list"

echo "== what must not change"
check "the base route"            GET  /v2/                                 200 "kappa /v2/"
check "the catalogue"             GET  /v2/_catalog                         200 "kappa /v2/_catalog"
check "referrers"                 GET  /v2/you/app/referrers/sha256:0       200 "kappa /v2/you/app/referrers/sha256:0"
check "the hub's index, reads"    GET  /v2/model-hub/index/tags/list        200 "kappa /v2/model-hub/index/tags/list"
check "models still on resolve"   GET  /api/models                          200 "\\[\\]"
check "an unindexed model, HF dialect, is still resolve's 404" GET /api/models/you/app 404 "RepoNotFound"
check "publish still shut"        GET  /api/v1/objects                      401 "publish requires a publisher token"
check "grpc still shut"           GET  /hologram.live.v1.HologramLive/Handshake 404 "not found"
check "docs still on the server"  GET  /docs                                200 "hologram-server /docs"

echo
if [ "$fails" -gt 0 ]; then echo "$fails failed"; echo "--- resolve.log ---"; tail -20 "$WORK/resolve.log"; exit 1; fi
echo "all assertions passed against the real Caddyfile.hub and the real hub-resolve.mjs"
