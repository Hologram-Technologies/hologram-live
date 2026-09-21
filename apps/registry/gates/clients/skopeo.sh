#!/usr/bin/env bash
# skopeo: us to a peer registry and back with the digest unchanged, then delete.
# Needs gate-c/docker:v1, which docker.sh pushes.
source "$(dirname "$0")/lib.sh"

copy() { skopeo copy -q --all --preserve-digests --src-tls-verify=false --dest-tls-verify=false "docker://$1" "docker://$2"; }
digest() { skopeo inspect --tls-verify=false --format '{{.Digest}}' "docker://$1"; }

copy "$US/gate-c/docker:v1" "$PEER/gate-c/from-us:v1"
copy "$PEER/gate-c/from-us:v1" "$US/gate-c/round-trip:v1"
origin=$(digest "$US/gate-c/docker:v1")
same "$origin" "$(digest "$PEER/gate-c/from-us:v1")" "the digest on the peer"
same "$origin" "$(digest "$US/gate-c/round-trip:v1")" "the digest after the round trip"

skopeo delete --tls-verify=false "docker://$US/gate-c/round-trip:v1"
if digest "$US/gate-c/round-trip:v1" > /dev/null 2>&1; then fail "a deleted image still answers"; fi
printf 'skopeo: us -> peer -> us with one digest, then delete: ok (%s)\n' "$origin"
