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
#
# One wrinkle an operator does not have: the four plain deployments above
# taught this daemon that 127.0.0.1:5000 speaks plain HTTP, and docker
# keeps that verdict for a while - the login ping then goes straight to
# http:// and meets a TLS listener that refuses it. A daemon restart
# clears the memory, the way a machine that only ever ran TLS would see
# the registry.
compose="$work/docker-compose.yml"
mkdir -p "$work/path/data" "$work/path/certs" "$work/path/auth"
sudo systemctl restart docker
for _ in $(seq 1 30); do docker info > /dev/null 2>&1 && break; sleep 1; done
docker info > /dev/null 2>&1 || fail "the docker daemon did not come back"
# A certificate the way the guide's operator has one: a test CA signs a
# server certificate, and the daemon trusts the CA through certs.d. The
# name the daemon uses is registry.local - a DNS name, not 127.0.0.1:
# docker treats a loopback registry as insecure, retries it over plain
# HTTP, and dockerd 28's push pipeline degrades to http:// - the same
# TLS-only port that refuses it. An operator's registry has a name in
# its certificate; the test borrows that shape.
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -keyout "$work/path/ca.key" -out "$work/path/ca.crt" \
  -days 2 -nodes -subj "//CN=registry-test-ca" \
  -addext "basicConstraints=critical,CA:TRUE" > /dev/null 2>&1 \
  || fail "openssl could not make the test CA"
openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -keyout "$work/path/certs/domain.key" -out "$work/path/domain.csr" \
  -nodes -subj "//CN=registry.local" \
  -addext "subjectAltName=DNS:registry.local,DNS:localhost,IP:127.0.0.1" > /dev/null 2>&1 \
  || fail "openssl could not make the test key"
openssl x509 -req -in "$work/path/domain.csr" \
  -CA "$work/path/ca.crt" -CAkey "$work/path/ca.key" -CAcreateserial \
  -days 2 -out "$work/path/certs/domain.crt" \
  -extfile <(printf 'subjectAltName=DNS:registry.local,DNS:localhost,IP:127.0.0.1\n') > /dev/null 2>&1 \
  || fail "openssl could not sign the test certificate"
echo "127.0.0.1 registry.local" | sudo tee -a /etc/hosts > /dev/null
# The daemon trusts the test CA the one way docker supports: certs.d.
sudo mkdir -p /etc/docker/certs.d/registry.local:5000
sudo cp "$work/path/ca.crt" /etc/docker/certs.d/registry.local:5000/ca.crt
cp "$here/../differential/fixtures/htpasswd" "$work/path/auth/htpasswd"
sed -e "s|image: registry:3|image: $image|" -e "s|/path/|$work/path/|" "$here/../compose/deploying-tls-htpasswd.yml" > "$compose"
extra=$(diff <(grep -v '^#' "$here/../compose/deploying-tls-htpasswd.yml") <(grep -v '^#' "$compose") \
  | grep '^[<>]' | grep -vE 'image:|/path/|'"$work" || true)
[ -z "$extra" ] || fail "compose-swap changed more than the image and /path: $extra"
cleanup() { sudo sed -i '/registry\.local/d' /etc/hosts; sudo rm -rf /etc/docker/certs.d/registry.local:5000; }
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
printf 'gate-password' | docker login -u gate --password-stdin registry.local:5000 > /dev/null 2>"$work/login.err" \
  || {
    cat "$work/login.err" >&2
    echo '--- the docker daemon on the HTTPS attempt ---' >&2
    sudo journalctl -u docker --no-pager 2>/dev/null | tail -n 8 >&2 || true
    echo '--- the same registry on 5007, debug logs, daemon against it ---' >&2
    docker run -d --name swap-diag --network host \
      -e REGISTRY_HTTP_ADDR=0.0.0.0:5007 \
      -e REGISTRY_HTTP_TLS_CERTIFICATE=/certs/domain.crt \
      -e REGISTRY_HTTP_TLS_KEY=/certs/domain.key \
      -e REGISTRY_AUTH=htpasswd -e REGISTRY_AUTH_HTPASSWD_PATH=/auth/htpasswd \
      -e REGISTRY_AUTH_HTPASSWD_REALM="Registry Realm" \
      -e REGISTRY_LOG_LEVEL=debug \
      -v "$work/path/certs:/certs" -v "$work/path/auth:/auth" \
      "$image" > /dev/null 2>&1 || true
    sleep 2
    curl -k -s -o /dev/null -w 'curl over TLS on 5007: %{http_code}\n' "https://registry.local:5007/v2/" 2>&1 | tail -n 1 >&2 || true
    printf 'gate-password' | docker login -u gate --password-stdin registry.local:5007 2>&1 | head -n 3 >&2 || true
    docker logs swap-diag 2>&1 | tail -n 30 >&2
    docker rm -f swap-diag > /dev/null 2>&1 || true
    echo '--- the compose registry itself, after the failed login ---' >&2
    docker compose -f "$compose" -p swap ps -a --format '{{.Name}} {{.State}} restarts?{{.Health}}' 2>&1 | tail -n 5 >&2 || true
    docker compose -f "$compose" -p swap logs --tail 30 registry 2>&1 | tail -n 30 >&2 || true
    docker compose -f "$compose" -p swap down -v > /dev/null 2>&1 || true
    fail "docker login through TLS"
  }
tag="registry.local:5000/swap/hello:v1"
docker build -q -t "$tag" "$work" > /dev/null
docker push -q "$tag" > /dev/null 2>"$work/push.err" \
  || {
    cat "$work/push.err" >&2
    # The control: the same push, same certificate, same daemon, against
    # the reference registry. If the reference fails too, the daemon or
    # the runner is the variable; if it succeeds, the difference is ours.
    echo '--- the same push against the reference registry on 5009 ---' >&2
    echo "127.0.0.1 registry-tls" | sudo tee -a /etc/hosts > /dev/null
    sudo mkdir -p /etc/docker/certs.d/registry-tls:5009
    sudo cp "$work/path/ca.crt" /etc/docker/certs.d/registry-tls:5009/ca.crt
    docker run -d --name ref-tls -p 5009:5000 \
      -e REGISTRY_HTTP_TLS_CERTIFICATE=/certs/domain.crt \
      -e REGISTRY_HTTP_TLS_KEY=/certs/domain.key \
      -v "$work/path/certs:/certs" \
      registry:3 > /dev/null 2>&1 || true
    sleep 2
    tag9="registry-tls:5009/swap/hello:v1"
    docker build -q -t "$tag9" "$work" > /dev/null 2>&1 || true
    docker push -q "$tag9" 2>&1 | head -n 3 >&2 || true
    docker rm -f ref-tls > /dev/null 2>&1 || true
    sudo sed -i '/registry-tls/d' /etc/hosts
    sudo rm -rf /etc/docker/certs.d/registry-tls:5009
    docker compose -f "$compose" -p swap down -v > /dev/null 2>&1 || true
    fail "docker push through the TLS compose file"
  }
pushed=$(docker inspect --format '{{index .RepoDigests 0}}' "$tag")
docker image rm "$tag" > /dev/null
docker pull -q "$tag" > /dev/null || fail "docker pull through the TLS compose file"
[ "$(docker inspect --format '{{index .RepoDigests 0}}' "$tag")" = "$pushed" ] || fail "the digest pulled differs from the one pushed"
docker logout registry.local:5000 > /dev/null
docker image rm "$tag" > /dev/null
took=$(( $(date +%s) - started ))
[ "$took" -le 300 ] || fail "${took}s from start to a pushed image behind TLS, over 300 s"
printf '  start to pushed behind TLS: %ss\n' "$took"
docker compose -f "$compose" -p swap down -v > /dev/null 2>&1
echo "the guide's compose file: TLS and login, one image line changed: ok"
echo "compose swap: ok"
