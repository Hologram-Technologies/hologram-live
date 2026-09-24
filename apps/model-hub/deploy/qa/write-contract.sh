#!/usr/bin/env bash
# Exercise the write surfaces of the hub with the tokens that already live on this host.
#
#   ./write-contract.sh            run every check
#   ./write-contract.sh --dry      say what it would do and what it would leave behind, then stop
#
# Every read path of this endpoint is now heavily tested. The write paths were verified only to the extent that
# they refuse anonymous callers — which proves the door is locked, not that the key works or that what is behind it
# keeps its promises. The one that matters most is the first assertion below: that the address the server hands
# back when you publish is genuinely the BLAKE3 of the bytes you sent. Everything this hub claims rests on that,
# and it has only ever been checked for objects that were already published.
#
# No secret is printed. Tokens are read from the files that already hold them and passed in a variable.
#
# What it leaves behind, permanently: ONE small object, about 200 bytes, published under the kind
# `qa.write-contract` so it is obvious what it is. Objects are immutable and cannot be deleted; running this again
# republishes the same bytes, which resolves to the same address and adds nothing. The registry upload session it
# opens is cancelled, so the registry keeps nothing.
set -uo pipefail

HUB=${HUB:-/root/hub}
HOST=${HOST:-gethologram.ai}
DRY=${1:-}
pass=0; fail=0
ok() { pass=$((pass + 1)); printf '  ok   %-54s %s\n' "$1" "${2:-}"; }
no() { fail=$((fail + 1)); printf '  FAIL %-54s %s\n' "$1" "${2:-}"; }
skip() { printf '  ··   %-54s %s\n' "$1" "${2:-}"; }

# ---- credentials, by name only -----------------------------------------------------------------
#
# What the EDGE checks is not what pub.env holds. publish.sh posts to hub-server directly over the docker network,
# so pub.env carries the server's own token and gets a 401 at the edge. The edge's credentials live in the front
# Caddyfile: the object publish gate is a Bearer token, and the registry gate is HTTP Basic as user `publisher`,
# not the Bearer that deploy/Caddyfile.hub still describes.
CADDYFILE=${CADDYFILE:-/root/twenty/Caddyfile}
PUBLISH_TOKEN=""; REGISTRY_USER=""; REGISTRY_PASS=""

if [ -r "$CADDYFILE" ]; then
	PUBLISH_TOKEN=$(grep -E '@denied[[:space:]]+not[[:space:]]+header[[:space:]]+Authorization' "$CADDYFILE" \
		| sed -E 's/.*Bearer[[:space:]]+([^"]+)".*/\1/' | head -1)
fi
[ -z "$PUBLISH_TOKEN" ] && [ -f "$HUB/pub.env" ] && PUBLISH_TOKEN=$(grep -E '^HUB_PUBLISH_TOKEN=' "$HUB/pub.env" | cut -d= -f2-)

if [ -f "$HUB/registry-push-credentials" ]; then
	REGISTRY_USER=$(grep -E '^username:' "$HUB/registry-push-credentials" | awk '{print $2}')
	REGISTRY_PASS=$(grep -E '^password:' "$HUB/registry-push-credentials" | awk '{print $2}')
fi

echo "# credentials found: publish=$([ -n "$PUBLISH_TOKEN" ] && echo yes || echo NO) registry=$([ -n "$REGISTRY_PASS" ] && echo "yes (basic, user ${REGISTRY_USER:-?})" || echo NO)"
echo "# host: https://$HOST"
if [ "$DRY" = "--dry" ]; then
	echo
	echo "would publish one ~200 byte object of kind qa.write-contract (permanent, immutable, re-runs add nothing),"
	echo "open and then cancel one registry upload session (leaves nothing), and post 9 MiB once to find the edge cap."
	exit 0
fi

BODY=$(mktemp); OUT=$(mktemp); HEAD=$(mktemp)
trap 'rm -f "$BODY" "$OUT" "$HEAD" /tmp/.wc-big' EXIT

# A body that is obviously a test, and stable so that re-running resolves to the same address.
printf '{"format":"hologram.qa.write-contract/v1","what":"a write-path contract check","stable":true}\n' > "$BODY"

# ---- A. the publisher token --------------------------------------------------------------------
echo
echo "== publishing objects"

code=$(curl -s -o "$OUT" -w '%{http_code}' --max-time 30 -X POST "https://$HOST/api/v1/objects" --data-binary @"$BODY")
[ "$code" = 401 ] && ok "anonymous publish is refused" "401" || no "anonymous publish is refused" "$code"

code=$(curl -s -o "$OUT" -w '%{http_code}' --max-time 30 -X POST "https://$HOST/api/v1/objects" \
	-H "Authorization: Bearer not-the-token" --data-binary @"$BODY")
[ "$code" = 401 ] && ok "a wrong token is refused" "401" || no "a wrong token is refused" "$code"

if [ -z "$PUBLISH_TOKEN" ]; then
	skip "publishing with a valid token" "no publisher token found; the rest of this section cannot run"
else
	code=$(curl -s -o "$OUT" -D "$HEAD" -w '%{http_code}' --max-time 60 -X POST "https://$HOST/api/v1/objects" \
		-H "Authorization: Bearer $PUBLISH_TOKEN" -H "x-hologram-kind: qa.write-contract" \
		-H "x-hologram-filename: write-contract.json" -H "content-type: application/json" --data-binary @"$BODY")
	if [ "$code" != 201 ] && [ "$code" != 200 ]; then
		no "publishing with a valid token" "$code $(head -c 120 "$OUT")"
	else
		ID=$(node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{try{console.log(JSON.parse(s).id||"")}catch{console.log("")}})' < "$OUT")
		ok "publishing with a valid token" "$code, id ${ID:0:26}…"

		# THE assertion. Everything this hub claims rests on the name being the hash of the bytes.
		# ESM resolution ignores NODE_PATH, so run from a directory that already has hash-wasm: publish.sh installs
		# it into $HUB/pub, and a global install works too.
		HASHDIR=$HUB/pub; [ -d "$HASHDIR/node_modules/hash-wasm" ] || HASHDIR=$PWD
		COMPUTED=$(cd "$HASHDIR" && node -e '
			const fs=require("fs");
			import("hash-wasm").then(async h=>{const b=await h.createBLAKE3();b.update(fs.readFileSync(process.argv[1]));console.log("blake3:"+b.digest("hex"))})
				.catch(()=>{console.log("NO-HASH-WASM")});' "$BODY" 2>/dev/null)
		if [ "$COMPUTED" = "NO-HASH-WASM" ] || [ -z "$COMPUTED" ]; then
			skip "the returned address is the BLAKE3 of the bytes sent" "hash-wasm not installed on this host: npm i -g hash-wasm, or run from $HUB/pub"
		elif [ "$COMPUTED" = "$ID" ]; then
			ok "the returned address IS the BLAKE3 of the bytes sent" "verified locally"
		else
			no "the returned address IS the BLAKE3 of the bytes sent" "server said ${ID:0:26}…, the bytes hash to ${COMPUTED:0:26}…"
		fi

		# And the bytes come back, to anyone, unchanged.
		if [ -n "$ID" ]; then
			code=$(curl -s -o "$OUT" -w '%{http_code}' --max-time 30 "https://$HOST/api/v1/objects/$ID")
			if [ "$code" = 200 ] && cmp -s "$OUT" "$BODY"; then ok "it reads back anonymously, byte for byte" "$(wc -c < "$OUT") bytes"
			else no "it reads back anonymously, byte for byte" "$code, $(cmp "$OUT" "$BODY" 2>&1 | head -1)"; fi
		fi

		# Publishing the same bytes again must resolve to the same name, or the address is not a function of content.
		ID2=$(curl -s --max-time 60 -X POST "https://$HOST/api/v1/objects" -H "Authorization: Bearer $PUBLISH_TOKEN" \
			-H "x-hologram-kind: qa.write-contract" --data-binary @"$BODY" \
			| node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{try{console.log(JSON.parse(s).id||"")}catch{console.log("")}})')
		[ -n "$ID" ] && [ "$ID2" = "$ID" ] && ok "republishing the same bytes gives the same address" "no second object" \
			|| no "republishing the same bytes gives the same address" "first ${ID:0:20}…, second ${ID2:0:20}…"

		code=$(curl -s -o "$OUT" -w '%{http_code}' --max-time 60 "https://$HOST/api/v1/objects" -H "Authorization: Bearer $PUBLISH_TOKEN")
		[ "$code" = 200 ] && ok "a token holder can list objects" "$code" || no "a token holder can list objects" "$code"

		# One call only: this path is about 1 rps and must never be hammered.
		code=$(curl -s -o "$OUT" -w '%{http_code}' --max-time 120 "https://$HOST/api/v1/objects/search?limit=1" -H "Authorization: Bearer $PUBLISH_TOKEN")
		[ "$code" = 200 ] && ok "a token holder can search objects" "$code (one call: this path is ~1 rps)" || no "a token holder can search objects" "$code"

		# The edge caps the body at 8 MB and the server at 32 MiB. A caller meeting that should learn so clearly.
		head -c 9437184 /dev/urandom > /tmp/.wc-big
		code=$(curl -s -o "$OUT" -w '%{http_code}' --max-time 120 -X POST "https://$HOST/api/v1/objects" \
			-H "Authorization: Bearer $PUBLISH_TOKEN" -H "x-hologram-kind: qa.write-contract" --data-binary @/tmp/.wc-big)
		case "$code" in
			413) ok "9 MiB is refused at the edge" "413, the documented 8 MB cap" ;;
			201|200) no "9 MiB is refused at the edge" "accepted: the 8 MB edge cap is not in force" ;;
			*) no "9 MiB is refused at the edge" "$code $(head -c 100 "$OUT") — neither accepted nor a clean 413" ;;
		esac
	fi
fi

# ---- B. the registry token ---------------------------------------------------------------------
echo
echo "== writing to the registry"

code=$(curl -s -o "$OUT" -w '%{http_code}' --max-time 30 -X POST "https://$HOST/v2/model-hub/qa-write-contract/blobs/uploads/")
[ "$code" = 401 ] && ok "an anonymous registry write is refused" "401" || no "an anonymous registry write is refused" "$code"

code=$(curl -s -o "$OUT" -w '%{http_code}' --max-time 30 -X POST "https://$HOST/v2/model-hub/qa-write-contract/blobs/uploads/" \
	-u "wrong:credentials")
[ "$code" = 401 ] && ok "wrong registry credentials are refused" "401" || no "wrong registry credentials are refused" "$code"

if [ -z "$REGISTRY_PASS" ]; then
	skip "starting an upload with valid credentials" "no registry password found in $HUB/registry-push-credentials"
else
	code=$(curl -s -o "$OUT" -D "$HEAD" -w '%{http_code}' --max-time 30 -X POST \
		"https://$HOST/v2/model-hub/qa-write-contract/blobs/uploads/" -u "$REGISTRY_USER:$REGISTRY_PASS")
	LOC=$(grep -i '^location:' "$HEAD" | tr -d '\r' | cut -d' ' -f2-)
	if [ "$code" = 202 ] && [ -n "$LOC" ]; then
		ok "a credential holder can start an upload" "202, session opened"
		# Cancel it: this check is about the door, not about leaving anything in the room.
		case "$LOC" in http*) URL="$LOC" ;; *) URL="https://$HOST$LOC" ;; esac
		ccode=$(curl -s -o /dev/null -w '%{http_code}' --max-time 30 -X DELETE "$URL" -u "$REGISTRY_USER:$REGISTRY_PASS")
		case "$ccode" in 202|204|200) ok "and the session can be cancelled" "$ccode, nothing left behind" ;;
			*) no "and the session can be cancelled" "$ccode — an upload session may be left open" ;; esac
	else
		no "a credential holder can start an upload" "$code $(head -c 100 "$OUT")"
	fi
fi

# Reads must stay anonymous no matter what: a token must never be required to read.
code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 30 "https://$HOST/v2/_catalog")
[ "$code" = 200 ] && ok "reads stay anonymous" "200 without any token" || no "reads stay anonymous" "$code"

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ] || exit 1
