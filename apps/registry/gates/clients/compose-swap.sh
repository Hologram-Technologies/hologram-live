#!/usr/bin/env bash
# P5 T5, the drop-in claim: the reference's deployment guide, run with only the
# image name changed. distribution/distribution v3.1.1,
# docs/content/about/deploying.md.
#
# The guide's plain deployments are four `docker run` commands (lines 115 to
# 171); each must start, take a `docker push` and give it back to `docker
# pull`, within 300 s of the start (the image already pulled: SC-001). Its one
# compose file, TLS and htpasswd login, must do the same by DNS name, trusted
# through the Docker daemon's certs.d and with no insecure-registries entry.
#
# The compose part changes the machine it runs on, so it runs only where
# SWAP_TRUST=sudo says it may (the CI runner): it adds `registry.gate` to
# /etc/hosts and the test CA to /etc/docker/certs.d/registry.gate:5000/.
# Usage: compose-swap.sh <our image>
set -euo pipefail
image=$1
here=$(cd "$(dirname "$0")" && pwd)
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
work=$(mktemp -d)
# The build context is a folder of its own: the compose leg mounts the
# registry's volume under $work, and its state directory is root-only.
ctx="$work/ctx"
mkdir -p "$ctx"
printf 'compose swap\n' > "$ctx/hello.txt"
printf 'FROM scratch\nCOPY hello.txt /hello.txt\n' > "$ctx/Dockerfile"

# Push and pull through host port $1, timed from $2 (seconds since the epoch).
round_trip() {
  local port=$1 started=$2 tag="127.0.0.1:$1/swap/hello:v1"
  for _ in $(seq 1 60); do
    curl -fsS "http://127.0.0.1:$port/v2/" > /dev/null 2>&1 && break
    sleep 1
  done
  docker build -q -t "$tag" "$ctx" > /dev/null
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
  name=$(docker run "${@/#registry:3/$image}")
  round_trip "$port" "$started"
  docker rm -f "$name" > /dev/null
}

run 5000 -d -p 5000:5000 --restart=always --name registry registry:3
run 5001 -d -p 5001:5000 --name registry-test registry:3
run 5001 -d -e REGISTRY_HTTP_ADDR=0.0.0.0:5001 -p 5001:5001 --name registry-test registry:3
mnt=${SWAP_MNT:-/mnt/registry}
run 5000 -d -p 5000:5000 --restart=always --name registry -v "$mnt:/var/lib/registry" registry:3
echo "the guide's four docker run deployments: ok"

# The compose file, as the guide gives it: TLS from /certs, login from /auth.
if [ "${SWAP_TRUST:-}" != sudo ]; then
  echo "the guide's compose file: skipped (set SWAP_TRUST=sudo where this may edit /etc/hosts and certs.d)"
  echo "compose swap: ok"
  exit 0
fi
compose="$work/docker-compose.yml"
mkdir -p "$work/path/data" "$work/path/certs" "$work/path/auth"
sed -e "s|image: registry:3|image: $image|" -e "s|/path/|$work/path/|" "$here/../compose/deploying-tls-htpasswd.yml" > "$compose"
extra=$(diff <(grep -v '^#' "$here/../compose/deploying-tls-htpasswd.yml") <(grep -v '^#' "$compose") \
  | grep '^[<>]' | grep -vE 'image:|/path/|'"$work" || true)
[ -z "$extra" ] || fail "compose-swap changed more than the image and /path: $extra"

# What the guide has the reader make: a certificate and key as domain.crt and
# domain.key, and a password file from `htpasswd -Bbn` (its own httpd:2 recipe).
tls=$(cd "$here/../../../../tests/fixtures/tls" && pwd)
cp "$tls/chain.crt" "$work/path/certs/domain.crt"
cp "$tls/server.key" "$work/path/certs/domain.key"
docker run --rm --entrypoint htpasswd httpd:2 -Bbn gate gate-password > "$work/path/auth/htpasswd"
# The client trusts the CA by name, as the guide's "use a certificate" section
# says; a non-loopback address, so Docker's default insecure range does not apply.
address=$(ip route get 1 | awk '{for (i = 1; i < NF; i++) if ($i == "src") print $(i + 1)}')
grep -q ' registry.gate$' /etc/hosts || printf '%s registry.gate\n' "$address" | sudo tee -a /etc/hosts > /dev/null
sudo mkdir -p /etc/docker/certs.d/registry.gate:5000
sudo cp "$tls/ca.crt" /etc/docker/certs.d/registry.gate:5000/ca.crt

printf 'guide: docker compose up -d (TLS, htpasswd)\n'
started=$(date +%s)
down() { docker compose -f "$compose" -p swap down -v > /dev/null 2>&1 || true; }
trap down EXIT
docker compose -f "$compose" -p swap up -d > /dev/null
for _ in $(seq 1 60); do
  [ "$(curl -s -o /dev/null -w '%{http_code}' --cacert "$tls/ca.crt" https://registry.gate:5000/v2/)" = 401 ] && break
  sleep 1
done
challenge=$(curl -s -o /dev/null -w '%{http_code} %header{www-authenticate}' --cacert "$tls/ca.crt" https://registry.gate:5000/v2/)
[ "$challenge" = '401 Basic realm="Registry Realm"' ] || { docker compose -f "$compose" -p swap logs 2>&1 | tail -n 20 || true; fail "the TLS compose file: $challenge"; }
# Plain HTTP on the TLS port gets Go's 400, never the registry.
plain=$(curl -s "http://registry.gate:5000/v2/" || true)
grep -q "Client sent an HTTP request to an HTTPS server" <<<"$plain" || fail "plain HTTP on the TLS port: $plain"

tag="registry.gate:5000/swap/hello:v1"
docker build -q -t "$tag" "$ctx" > /dev/null
docker logout registry.gate:5000 > /dev/null 2>&1 || true
# Refused for the right reason: the login, not TLS or the network.
if refused=$(docker push -q "$tag" 2>&1); then fail "an anonymous push went through"; fi
grep -qiE "no basic auth credentials|unauthorized" <<<"$refused" || fail "the anonymous push failed, but not for the login: $refused"
printf 'gate-password' | docker login -u gate --password-stdin registry.gate:5000 > /dev/null || fail "docker login over TLS"
docker push -q "$tag" > /dev/null || fail "docker push over TLS"
pushed=$(docker inspect --format '{{index .RepoDigests 0}}' "$tag")
took=$(( $(date +%s) - started ))
docker image rm "$tag" > /dev/null
docker pull -q "$tag" > /dev/null || fail "docker pull over TLS"
[ "$(docker inspect --format '{{index .RepoDigests 0}}' "$tag")" = "$pushed" ] || fail "pull differs from push over TLS"
docker logout registry.gate:5000 > /dev/null
[ "$took" -le 300 ] || fail "${took}s from compose up to a pushed image, over 300 s"
printf '  start to pushed: %ss (%s)\n' "$took" "$pushed"
echo "the guide's compose file: TLS by name, login, push and pull: ok"
echo "compose swap: ok"
