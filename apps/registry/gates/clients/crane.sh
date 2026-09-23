#!/usr/bin/env bash
# crane (go-containerregistry): copy, digest, ls, catalog, delete, and a copy
# through the reference and back with one digest (interop matrix row 11).
# Needs gate-c/docker:v1, which docker.sh pushes.
source "$(dirname "$0")/lib.sh"

origin=$(crane digest --insecure "$US/gate-c/docker:v1")
crane copy --insecure "$US/gate-c/docker:v1" "$US/gate-c/crane:v1"
same "$(crane digest --insecure "$US/gate-c/crane:v1")" "$origin" "crane copy within us"
crane tag --insecure "$US/gate-c/crane:v1" v2
same "$(crane ls --insecure "$US/gate-c/crane" | sort | tr '\n' ' ')" "v1 v2 " "crane ls"
crane catalog --insecure "$US" | grep -qx 'gate-c/crane' || fail "crane catalog does not list gate-c/crane"

# Through the reference and back.
crane copy --insecure "$US/gate-c/crane:v1" "$PEER/gate-c/from-crane:v1"
crane copy --insecure "$PEER/gate-c/from-crane:v1" "$US/gate-c/crane-round-trip:v1"
same "$(crane digest --insecure "$US/gate-c/crane-round-trip:v1")" "$origin" "the digest after crane's round trip"

# The config and every layer, fetched whole and hashed by crane.
crane validate --insecure --remote "$US/gate-c/crane:v1" > /dev/null || fail "crane validate"

crane delete --insecure "$US/gate-c/crane-round-trip@$origin"
if crane digest --insecure "$US/gate-c/crane-round-trip:v1" > /dev/null 2>&1; then fail "a deleted manifest still answers"; fi
printf 'crane: copy, tag, ls, catalog, round trip, validate, delete: ok (%s)\n' "$origin"
