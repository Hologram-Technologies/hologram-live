#!/usr/bin/env bash
# P5 T5, the drop-in claim: the reference's deployment guide, run with only the
# image name changed. distribution/distribution v3.1.1,
# docs/content/about/deploying.md.
#
# The guide's plain deployments are four `docker run` commands (lines 115 to
# 171); each must start, take a `docker push` and give it back to `docker
# pull`, within 300 s of the start (the image already pulled: SC-001). Its one
# compose file needs TLS and htpasswd login (plan P6): until P6 it must refuse
# to start and name why, never come up without TLS. P6 turns that into a pass.
# Usage: compose-swap.sh <our image>
set -euo pipefail
image=$1
here=$(cd "$(dirname "$0")" && pwd)
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
work=$(mktemp -d)
printf 'compose swap\n' > "$work/hello.txt"
printf 'FROM scratch\nCOPY hello.txt /hello.txt\n' > "$work/Dockerfile"

# Push and pull through host port $1, timed from $2 (seconds since the epoch).
round_trip() {
  local port=$1 started=$2 tag="127.0.0.1:$1/swap/hello:v1"
  for _ in $(seq 1 60); do
    curl -fsS "http://127.0.0.1:$port/v2/" > /dev/null 2>&1 && break
    sleep 1
  done
  docker build -q -t "$tag" "$work" > /dev/null
  docker push -q "$tag" > /dev/null || fail "docker push to $port"
  local pushed took
  pushed=$(docker inspect --format '{{index .RepoDigests 0}}' "$tag")
  took=$(( $(date +%s) - started ))
  docker image rm "$tag" > /dev/null
  docker pull -q "$tag" > /dev/null || fail "docker pull from $port"
  [ "$(docker inspect --format '{{index .RepoDigests 0}}' "$tag")" = "$pushed" ] || fail "pull differs from push on $port"
  docker image rm "$tag" > /dev/null
  [ "$took" -le 300 ] || fail "${took}s from start to a pushed image, over 300 s"
  printf '  start to pushed: %ss\n' "$took"
}

# One of the guide's commands, as written, with `registry:3` replaced.
run() {
  local port=$1; shift
  local name
  printf 'guide: docker run %s\n' "$*"
  local started
  started=$(date +%s)
  docker run "${@/#registry:3/$image}" > /dev/null
  name=$(docker ps -lq)
  round_trip "$port" "$started"
  docker rm -f "$name" > /dev/null
}

run 5000 -d -p 5000:5000 --restart=always --name registry registry:3
run 5001 -d -p 5001:5000 --name registry-test registry:3
run 5001 -d -e REGISTRY_HTTP_ADDR=0.0.0.0:5001 -p 5001:5001 --name registry-test registry:3
mnt=${SWAP_MNT:-/mnt/registry}
run 5000 -d -p 5000:5000 --restart=always --name registry -v "$mnt:/var/lib/registry" registry:3
echo "the guide's four docker run deployments: ok"

# The compose file: TLS and login are P6.
compose="$work/docker-compose.yml"
mkdir -p "$work/path/data" "$work/path/certs" "$work/path/auth"
sed -e "s|image: registry:3|image: $image|" -e "s|/path/|$work/path/|" "$here/../compose/deploying-tls-htpasswd.yml" > "$compose"
extra=$(diff <(grep -v '^#' "$here/../compose/deploying-tls-htpasswd.yml") <(grep -v '^#' "$compose") \
  | grep '^[<>]' | grep -vE 'image:|/path/|'"$work" || true)
[ -z "$extra" ] || fail "compose-swap changed more than the image and /path: $extra"
docker compose -f "$compose" -p swap up -d > /dev/null 2>&1
sleep 5
logs=$(docker compose -f "$compose" -p swap logs 2>&1 || true)
docker compose -f "$compose" -p swap down -v > /dev/null 2>&1 || true
if curl -fsS "http://127.0.0.1:5000/v2/" > /dev/null 2>&1; then fail "the TLS compose file came up over plain HTTP"; fi
printf '%s' "$logs" | grep -q "not built yet" || { printf '%s\n' "$logs" | tail -n 20; fail "the TLS compose file did not refuse by name"; }
printf '%s\n' "$logs" | grep -m1 "not built yet" | sed 's/^/  refused: /'
echo "the guide's compose file: refused by name until P6 (TLS, htpasswd)"
echo "compose swap: ok"
