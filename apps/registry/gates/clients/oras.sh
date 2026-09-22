#!/usr/bin/env bash
# ORAS: an artifact with its own artifactType pushed and pulled back byte for
# byte; a signature attached to an image and found through the referrers
# API; and `cp -r` through the reference and back with the referrer kept
# (interop matrix row 12). Needs gate-c/docker:v1, which docker.sh pushes.
source "$(dirname "$0")/lib.sh"
work=$(mktemp -d)
cd "$work"

printf 'an artifact, not an image\n' > artifact.txt
oras push -q --plain-http --artifact-type application/vnd.gate.example "$US/gate-c/oras:v1" artifact.txt:text/plain
mkdir pulled
oras pull -q --plain-http -o pulled "$US/gate-c/oras:v1"
cmp artifact.txt pulled/artifact.txt || fail "the pulled artifact differs"
oras manifest fetch --plain-http "$US/gate-c/oras:v1" | grep -q '"artifactType":"application/vnd.gate.example"' ||
  fail "the artifactType did not survive"

subject="$US/gate-c/docker:v1"
printf 'a signature\n' > signature.txt
oras attach -q --plain-http --artifact-type application/vnd.gate.signature "$subject" signature.txt:text/plain
found=$(oras discover --plain-http --format json --artifact-type application/vnd.gate.signature "$subject")
grep -q 'application/vnd.gate.signature' <<< "$found" || fail "oras discover does not find the signature: $found"

# The image and its referrer, to the reference and back. Where a registry
# has no referrers API, oras keeps the referrer under the fallback tag.
oras cp -r --from-plain-http --to-plain-http "$subject" "$PEER/gate-c/from-oras:v1"
oras cp -r --from-plain-http --to-plain-http "$PEER/gate-c/from-oras:v1" "$US/gate-c/oras-round-trip:v1"
back=$(oras discover --plain-http --format json --artifact-type application/vnd.gate.signature "$US/gate-c/oras-round-trip:v1")
grep -q 'application/vnd.gate.signature' <<< "$back" || fail "the referrer was lost in the round trip: $back"
same "$(oras resolve --plain-http "$US/gate-c/oras-round-trip:v1")" "$(oras resolve --plain-http "$subject")" \
  "the image digest after oras's round trip"
printf 'oras: artifact, attach, discover, cp -r round trip: ok\n'
