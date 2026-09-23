#!/usr/bin/env bash
# P5 T5, the drop-in claim: the reference's deployment guide, run with only the
# image name changed. distribution/distribution v3.1.1,
# docs/content/about/deploying.md.
#
# The guide's plain deployments are four `docker run` commands (lines 115 to
# 171); each must start, take a `docker push` and give it back to `docker
# pull`, within 300 s of the start (the image already pulled: SC-001). Its one
# compose file needs TLS and htpasswd login (plan P6): the test provisions
# the certificate and the password file the way the guide's operator would,
# and the file comes up behind TLS with only the image line changed. The
# registry is addressed by DNS name (registry.local), as the spec's tls.sh
# design has it, because docker treats a loopback registry as insecure and
# retries plain HTTP against the TLS-only port. Nothing answers over plain
# HTTP, ever.
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

# The compose file, as the guide intends since P6: the operator brings a
# certificate and a password file, and only `image:` changes. The test
# brings its own the same way.
compose="$work/docker-compose.yml"
mkdir -p "$work/path/data" "$work/path/certs" "$work/path/auth"
# A certificate the way the guide's operator has one: a test CA signs a
# A certificate the way the guide's operator has one: a test CA signs a
# server certificate, and the client trusts the CA. The client is a
# skopeo container on the registry's own Docker network, addressing the
# registry by its service name - the spec's tls.sh shape ("two containers
# on one Docker network... by DNS name with no insecure-registries").
# The host daemon's push pipeline for a loopback address retries plain
# HTTP against the TLS-only port (dockerd 28 on the runner, observed
# against the reference too), so the host daemon is not the TLS client.
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -keyout "$work/path/ca.key" -out "$work/path/ca.crt" \
  -days 2 -nodes -subj "//CN=registry-test-ca" \
  -addext "basicConstraints=critical,CA:TRUE" > /dev/null 2>&1 \
  || fail "openssl could not make the test CA"
openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -keyout "$work/path/certs/domain.key" -out "$work/path/domain.csr" \
  -nodes -subj "//CN=registry" \
  -addext "subjectAltName=DNS:registry,DNS:localhost,IP:127.0.0.1" > /dev/null 2>&1 \
  || fail "openssl could not make the test key"
openssl x509 -req -in "$work/path/domain.csr" \
  -CA "$work/path/ca.crt" -CAkey "$work/path/ca.key" -CAcreateserial \
  -days 2 -out "$work/path/certs/domain.crt" \
  -extfile <(printf 'subjectAltName=DNS:registry,DNS:localhost,IP:127.0.0.1\n') > /dev/null 2>&1 \
  || fail "openssl could not sign the test certificate"
cp "$here/../differential/fixtures/htpasswd" "$work/path/auth/htpasswd"
sed -e "s|image: registry:3|image: $image|" -e "s|/path/|$work/path/|" "$here/../compose/deploying-tls-htpasswd.yml" > "$compose"
extra=$(diff <(grep -v '^#' "$here/../compose/deploying-tls-htpasswd.yml") <(grep -v '^#' "$compose") \
  | grep '^[<>]' | grep -vE 'image:|/path/|'"$work" || true)
[ -z "$extra" ] || fail "compose-swap changed more than the image and /path: $extra"
cleanup() { docker rm -f swap-client > /dev/null 2>&1 || true; }
trap cleanup EXIT
started=$(date +%s)
docker compose -f "$compose" -p swap up -d > /dev/null 2>&1
serving=1
for _ in $(seq 1 60); do
  # The self-signed test certificate is trusted through certs.d by the
  # docker daemon; curl here only waits. The registry behind the guide's
  # file answers 401 until login: any HTTP status is a serving registry,
  # 000 is no answer.
  code=$(curl -k -s -o /dev/null -w '%{http_code}' --max-time 5 "https://127.0.0.1:5000/v2/" 2>/dev/null || printf 000)
  [ "$code" != "000" ] && { serving=0; break; }
  sleep 1
done
state=$(docker compose -f "$compose" -p swap ps --format '{{.Name}} {{.State}}' 2>/dev/null || true)
printf '  registry container: %s\n' "$state"
logs_before_login=$(docker compose -f "$compose" -p swap logs --tail 5 2>/dev/null || true)
printf '%s\n' "$logs_before_login" | tail -n 5
if [ "$serving" != 0 ]; then
  printf '%s\n' 'diagnostics: the TLS compose file never answered /v2/ over HTTPS' >&2
  printf '%s\n' '--- curl, verbose ---' >&2
  curl -kv --max-time 5 "https://127.0.0.1:5000/v2/" 2>&1 | tail -n 25 >&2 || true
  printf '%s\n' '--- openssl, the raw handshake ---' >&2
  openssl s_client -connect 127.0.0.1:5000 -servername localhost </dev/null 2>&1 | head -n 25 >&2 || true
  docker compose -f "$compose" -p swap logs 2>&1 | tail -n 20 >&2
  fail "the TLS compose file never answered /v2/ over HTTPS"
fi
# While it is up: nothing may answer over plain HTTP.
if curl -fsS "http://127.0.0.1:5000/v2/" > /dev/null 2>&1; then
  docker compose -f "$compose" -p swap down -v > /dev/null 2>&1 || true
  fail "the TLS compose file answered over plain HTTP"
fi
# The login and the round trip, through the TLS port, within SC-001's 300 s.

# The round trip, through TLS, within SC-001's 300 s. The client is a
# skopeo container on the registry's own Docker network: it trusts the
# test CA from its own certs.d, addresses the registry by the service's
# DNS name, and verifies the certificate (--tls-verify). The image comes
# from, and returns to, the host daemon through its socket (docker-daemon:);
# the TLS conversations happen between the two containers on the network.
docker build -q -t swap/hello:v1 "$work" > /dev/null
skopeo() {
  docker run --rm --name swap-client --network swap_default \
    -v /var/run/docker.sock:/var/run/docker.sock \
    -v "$work/path/ca.crt:/etc/containers/certs.d/registry:5000/ca.crt" \
    quay.io/skopeo/stable:latest "$@"
}
skopeo copy -q --dest-creds gate:gate-password --dest-tls-verify \
  docker-daemon:swap/hello:v1 docker://registry:5000/swap/hello:v1 \
  || { docker compose -f "$compose" -p swap down -v > /dev/null 2>&1 || true; fail "skopeo push through the TLS compose file"; }
pushed=$(skopeo inspect --tls-verify --format '{{.Digest}}' docker://registry:5000/swap/hello:v1)
skopeo copy -q --src-creds gate:gate-password --src-tls-verify \
  docker://registry:5000/swap/hello:v1 docker-daemon:swap/hello:v2 \
  || { docker compose -f "$compose" -p swap down -v > /dev/null 2>&1 || true; fail "skopeo pull through the TLS compose file"; }
pulled=$(skopeo inspect --format '{{.Digest}}' docker-daemon:swap/hello:v2)
[ "$pulled" = "$pushed" ] || fail "the digest pulled differs from the one pushed"
docker image rm swap/hello:v1 swap/hello:v2 > /dev/null
took=$(( $(date +%s) - started ))
[ "$took" -le 300 ] || fail "${took}s from start to a pushed image behind TLS, over 300 s"
printf '  start to pushed behind TLS: %ss\n' "$took"
docker compose -f "$compose" -p swap down -v > /dev/null 2>&1
echo "the guide's compose file: TLS and login, one image line changed: ok"
echo "compose swap: ok"