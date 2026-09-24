#!/usr/bin/env bash
# List the hub in the official MCP registry, or update the listing that is already there.
#
#   ./publish-mcp-listing.sh --dry     show exactly what would be published, and stop
#   ./publish-mcp-listing.sh           publish it
#
# This is the one outward-facing step in the whole deployment: it puts an entry under the organisation's name in a
# public directory that MCP clients browse. It is reversible — republishing replaces the entry and the registry
# supports deletion — but it is a publication, not a deploy.
#
# It needs no extra binary. The registry's HTTP domain method is two calls: sign an RFC3339 timestamp with the
# Ed25519 key whose public half this hub already serves at /.well-known/mcp-registry-auth, exchange the signature
# for a short-lived registry token, then POST the manifest. The private key never leaves this host and neither the
# key nor the token is ever printed.
set -euo pipefail

HUB=${HUB:-/root/hub}
KEY=${KEY:-$HUB/mcp-registry-key.pem}
MANIFEST=${MANIFEST:-$HUB/mcp-server.json}
DOMAIN=${DOMAIN:-hub.uor.foundation}
REGISTRY=${REGISTRY:-https://registry.modelcontextprotocol.io}

die() { echo "FAIL $*" >&2; exit 1; }

[ -f "$KEY" ] || die "no key at $KEY. Generate it first; the public half must already be served."
[ -f "$MANIFEST" ] || die "no manifest at $MANIFEST"
NAME=$(node -e 'console.log(JSON.parse(require("fs").readFileSync(process.argv[1],"utf8")).name)' "$MANIFEST")

# The registry fetches this to learn the public key. If it does not match the key we are about to sign with, the
# exchange fails with something unhelpful, so check it here where the message can be clear.
SERVED=$(curl -fsS --max-time 20 "https://$DOMAIN/.well-known/mcp-registry-auth" || die "the hub is not serving /.well-known/mcp-registry-auth")
LOCAL="v=MCPv1; k=ed25519; p=$(openssl pkey -in "$KEY" -pubout -outform DER | tail -c 32 | base64)"
[ "$SERVED" = "$LOCAL" ] || die "the served public key does not match $KEY. Re-copy mcp-registry-auth and rerun."

echo "  domain:   $DOMAIN"
echo "  name:     $NAME"
echo "  endpoint: $(node -e 'const m=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));console.log((m.remotes&&m.remotes[0]&&m.remotes[0].url)||"none")' "$MANIFEST")"
echo "  key:      matches the one this hub serves"

if [ "${1:-}" = "--dry" ]; then
	echo
	echo "would publish the manifest above to $REGISTRY as $NAME."
	echo "manifest:"
	sed 's/^/    /' "$MANIFEST"
	exit 0
fi

# ---- 1. sign a timestamp and exchange it for a registry token
TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)
# -rawin needs a file it can size: Ed25519 signs the whole message at once, so a pipe is refused.
TSFILE=$(mktemp); printf '%s' "$TS" > "$TSFILE"
SIG=$(openssl pkeyutl -sign -inkey "$KEY" -rawin -in "$TSFILE" | xxd -p -c 256 | tr -d '\n')
rm -f "$TSFILE"
[ -n "$SIG" ] || die "signing produced nothing; openssl 3.x is needed for -rawin"

TOKEN=$(curl -fsS --max-time 30 -X POST "$REGISTRY/v0/auth/http" -H 'content-type: application/json' \
	-d "$(node -e 'console.log(JSON.stringify({domain:process.argv[1],timestamp:process.argv[2],signed_timestamp:process.argv[3]}))' "$DOMAIN" "$TS" "$SIG")" \
	| node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{try{console.log(JSON.parse(s).registry_token||"")}catch{console.log("")}})')
[ -n "$TOKEN" ] || die "the registry would not exchange the signature for a token"
echo "  token:    obtained"

# ---- 2. publish the manifest
OUT=$(mktemp); trap 'rm -f "$OUT"' EXIT
CODE=$(curl -s -o "$OUT" -w '%{http_code}' --max-time 60 -X POST "$REGISTRY/v0/publish" \
	-H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' --data-binary @"$MANIFEST")

if [ "$CODE" = 200 ] || [ "$CODE" = 201 ]; then
	echo "  published: $CODE"
	node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const j=JSON.parse(s);const v=j.server||j;console.log("  listed as: "+(v.name||"?")+" "+(v.version||""))})' < "$OUT" || true
else
	echo "  the registry refused: $CODE"
	head -c 600 "$OUT"; echo
	exit 1
fi

# ---- 3. and confirm it is findable the way a client would find it
sleep 3
FOUND=$(curl -fsS --max-time 20 "$REGISTRY/v0/servers?search=hologram" 2>/dev/null \
	| node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{try{const j=JSON.parse(s);const a=j.servers||j.data||[];console.log(a.map(x=>(x.server&&x.server.name)||x.name).filter(Boolean).join(", "))}catch{console.log("")}})' || true)
[ -n "$FOUND" ] && echo "  searchable: $FOUND" || echo "  searchable: not indexed yet (the search index lags publication)"
