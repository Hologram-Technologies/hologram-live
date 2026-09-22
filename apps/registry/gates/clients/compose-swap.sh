#!/usr/bin/env bash
# P5 T5, the drop-in claim: the reference's deployment guide, run with only the
# image name changed. distribution/distribution v3.1.1,
# docs/content/about/deploying.md.
#
# The guide's plain deployments are four `docker run` commands (lines 115 to
# 171); each must start, take a `docker push` and give it back to `docker
# pull`, within 300 s of the start (the image already pulled: SC-001). Its one
# compose file needs TLS and htpasswd login (plan P6): the test provisions the
# certificate and the password file the way the guide's operator would, and
# the file comes up behind TLS with only the image line changed. Nothing
# answers over plain HTTP, ever.
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
# A self-signed server certificate for 127.0.0.1, two days on purpose.
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -keyout "$work/path/certs/domain.key" -out "$work/path/certs/domain.crt" \
  -days 2 -nodes -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" > /dev/null 2>&1 \
  || fail "openssl could not make the test certificate"
# The guide's operator has a CA the daemon trusts; the test's self-signed
# certificate is trusted the one way docker supports: certs.d.
sudo mkdir -p /etc/docker/certs.d/127.0.0.1:5000
sudo cp "$work/path/certs/domain.crt" /etc/docker/certs.d/127.0.0.1:5000/ca.crt
cp "$here/../differential/fixtures/htpasswd" "$work/path/auth/htpasswd"
sed -e "s|image: registry:3|image: $image|" -e "s|/path/|$work/path/|" "$here/../compose/deploying-tls-htpasswd.yml" > "$compose"
extra=$(diff <(grep -v '^#' "$here/../compose/deploying-tls-htpasswd.yml") <(grep -v '^#' "$compose") \
  | grep '^[<>]' | grep -vE 'image:|/path/|'"$work" || true)
[ -z "$extra" ] || fail "compose-swap changed more than the image and /path: $extra"
cleanup() { sudo rm -rf /etc/docker/certs.d/127.0.0.1:5000; }
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
if [ "$serving" != 0 ]; then
  printf 'diagnostics: the TLS compose file never answered /v2/ over HTTPS\n' >&2
  printf '--- curl, verbose, h2 (the default curl offers) ---\n'
  curl -kv --max-time 5 "https://127.0.0.1:5000/v2/" 2>&1 | tail -n 25 >&2 || true
  printf '--- curl, forced HTTP/1.1 ---\n'
  curl -k --http1.1 -v --max-time 5 "https://127.0.0.1:5000/v2/" 2>&1 | tail -n 25 >&2 || true
  printf '--- openssl, the raw handshake ---\n'
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
printf 'gate-password' | docker login -u gate --password-stdin 127.0.0.1:5000 > /dev/null 2>"$work/login.err" \
  || { cat "$work/login.err" >&2; docker compose -f "$compose" -p swap down -v > /dev/null 2>&1 || true; fail "docker login through TLS"; }
tag="127.0.0.1:5000/swap/hello:v1"
docker build -q -t "$tag" "$work" > /dev/null
docker push -q "$tag" > /dev/null 2>"$work/push.err" \
  || { cat "$work/push.err" >&2; docker compose -f "$compose" -p swap down -v > /dev/null 2>&1 || true; fail "docker push through the TLS compose file"; }
pushed=$(docker inspect --format '{{index .RepoDigests 0}}' "$tag")
docker image rm "$tag" > /dev/null
docker pull -q "$tag" > /dev/null || fail "docker pull through the TLS compose file"
[ "$(docker inspect --format '{{index .RepoDigests 0}}' "$tag")" = "$pushed" ] || fail "the digest pulled differs from the one pushed"
docker logout 127.0.0.1:5000 > /dev/null
docker image rm "$tag" > /dev/null
took=$(( $(date +%s) - started ))
[ "$took" -le 300 ] || fail "${took}s from start to a pushed image behind TLS, over 300 s"
printf '  start to pushed behind TLS: %ss\n' "$took"
docker compose -f "$compose" -p swap down -v > /dev/null 2>&1
echo "the guide's compose file: TLS and login, one image line changed: ok"
echo "compose swap: ok"
