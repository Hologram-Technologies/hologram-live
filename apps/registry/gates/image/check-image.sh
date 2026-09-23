#!/usr/bin/env bash
# The image, judged from outside: its metadata equals the reference image's,
# it serves /v2/ with its own default file, nothing else faces the network,
# docker push and pull work, `docker stop` is clean, and a restart on the same
# volume keeps what was pushed.
# Usage: check-image.sh <our image>   (the reference is gates/reference.env)
set -euo pipefail
ours=$1
here=$(cd "$(dirname "$0")" && pwd)
source "$here/../reference.env"
reference="${REGISTRY_REF/:3@/@}"
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
port=5002
name=gate-image

docker pull -q "$reference" > /dev/null
field() { docker image inspect --format "{{json .Config.$2}}" "$1"; }
for key in Entrypoint Cmd ExposedPorts Volumes; do
  [ "$(field "$ours" "$key")" = "$(field "$reference" "$key")" ] \
    || fail "$key: ours $(field "$ours" "$key"), the reference's $(field "$reference" "$key")"
done
field "$ours" Env | grep -q '"OTEL_TRACES_EXPORTER=none"' || fail "Env lacks OTEL_TRACES_EXPORTER=none"
echo "metadata: entry point, command, port, volume and environment equal the reference's"

cleanup() {
  docker rm -f "$name" gate-image-debug > /dev/null 2>&1 || true
  docker volume rm -f gate-image-data > /dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup
# As the reference is run: the image's own default file, a named volume.
docker run -d --name "$name" -p "127.0.0.1:$port:5000" -v gate-image-data:/var/lib/registry "$ours" > /dev/null
wait_up() {
  for _ in $(seq 1 60); do
    curl -fsS "http://127.0.0.1:$port/v2/" > /dev/null 2>&1 && return 0
    sleep 1
  done
  docker logs "$name" | tail -n 40; fail "the image did not answer /v2/"
}
wait_up
status() { curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$port$1"; }
[ "$(status /v2/)" = 200 ] || fail "/v2/ is not 200"
for path in /api/v1/modules /api/v1/capabilities /api/v1/objects; do
  [ "$(status "$path")" = 404 ] || fail "$path faces the network (ADR 028)"
done
published=$(docker port "$name")
[ "$(printf '%s\n' "$published" | wc -l)" = 1 ] && printf '%s' "$published" | grep -q '^5000/tcp' \
  || fail "only 5000 is published, got: $published"
docker exec "$name" hologram --version > /dev/null || fail "hologram is not on the path"
# verify reads the image's own config.yml; beside the running server it says so.
if out=$(docker exec "$name" hologram oci verify 2>&1); then fail "oci verify ran beside the live server"; fi
grep -q "the registry is running on /var/lib/registry" <<<"$out" || fail "hologram oci verify: $out"
# The other way operators reach the binary: /bin/registry, as in the reference.
version=$(docker run --rm --entrypoint /bin/registry "$ours" --version) || fail "/bin/registry --version"
grep -q "^registry " <<<"$version" || fail "/bin/registry --version: $version"
# The reference's own command line, through the entry point, on a fresh volume.
gc=$(docker run --rm "$ours" garbage-collect --dry-run /etc/distribution/config.yml 2>&1) \
  || fail "garbage-collect through the entry point: $gc"
grep -q "blobs marked, 0 blobs and 0 manifests eligible for deletion" <<<"$gc" || fail "garbage-collect: $gc"
# The Helm chart's job writes the flag with a value, as cobra takes it.
gc=$(docker run --rm "$ours" garbage-collect --delete-untagged=true /etc/distribution/config.yml 2>&1)   || fail "garbage-collect --delete-untagged=true, the Helm chart's form: $gc"
grep -q "blobs marked, 0 blobs and 0 manifests eligible for deletion" <<<"$gc" || fail "garbage-collect: $gc"
echo "surface: /v2/ in public, the module API is not, only 5000 published, hologram on the path"

# The image's own default file turns on the debug listener (:5001) with
# Prometheus and the storage check, as the reference's does. Published here
# only to look at it; the reference does not publish it either.
debug_name=gate-image-debug
docker rm -f "$debug_name" > /dev/null 2>&1 || true
docker run -d --name "$debug_name" -p 127.0.0.1::5001 "$ours" > /dev/null
debug_port=$(docker port "$debug_name" 5001/tcp | head -n1 | awk -F: '{print $NF}')
for _ in $(seq 1 60); do curl -fsS "http://127.0.0.1:$debug_port/debug/health" > /dev/null 2>&1 && break; sleep 1; done
health=$(curl -s "http://127.0.0.1:$debug_port/debug/health")
[ "$health" = "{}" ] || { docker logs "$debug_name" | tail -n 20; fail "/debug/health: $health"; }
curl -fsS "http://127.0.0.1:$debug_port/metrics" | grep -q '^# TYPE registry_http_requests_total counter' || fail "/metrics"
docker rm -f "$debug_name" > /dev/null
echo "debug listener: /debug/health {} and /metrics on :5001, from the image's own file"

# http.draintimeout: an upload that hangs open is waited on for the drain,
# and no longer (the reference's `server.Shutdown` with that deadline).
drain_name=gate-image-drain
docker rm -f "$drain_name" > /dev/null 2>&1 || true
docker run -d --name "$drain_name" -e REGISTRY_HTTP_DRAINTIMEOUT=3s -p 127.0.0.1::5000 "$ours" > /dev/null
drain_port=$(docker port "$drain_name" 5000/tcp | head -n1 | awk -F: '{print $NF}')
for _ in $(seq 1 60); do curl -fsS "http://127.0.0.1:$drain_port/v2/" > /dev/null 2>&1 && break; sleep 1; done
location=$(curl -fsS -o /dev/null -D - -X POST "http://127.0.0.1:$drain_port/v2/gate/drain/blobs/uploads/" |
  tr -d '\r' | awk 'tolower($1) == "location:" { print $2 }')
location=${location#/}
# A PATCH that promises a megabyte and sends three bytes, left open.
exec 3<> "/dev/tcp/127.0.0.1/$drain_port"
printf 'PATCH %s HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/octet-stream\r\nContent-Length: 1048576\r\n\r\nabc' \
  "/${location#http*://*/}" >&3
sleep 1
started=$(date +%s)
docker stop -t 30 "$drain_name" > /dev/null
took=$(( $(date +%s) - started ))
exec 3>&-
code=$(docker inspect --format '{{.State.ExitCode}}' "$drain_name")
docker rm -f "$drain_name" > /dev/null
[ "$took" -ge 2 ] && [ "$took" -le 7 ] || fail "docker stop with a 3 s drain and a hung upload took ${took}s"
[ "$code" = 0 ] || fail "exit code $code after the drain"
echo "drain: a hung upload was waited on for http.draintimeout (${took}s), then the stop was clean"

dir=$(mktemp -d)
printf 'gate image\n' > "$dir/hello.txt"
printf 'FROM scratch\nCOPY hello.txt /hello.txt\n' > "$dir/Dockerfile"
tag="127.0.0.1:$port/gate/image:v1"
docker build -q -t "$tag" "$dir" > /dev/null
docker push -q "$tag" > /dev/null
pushed=$(docker inspect --format '{{index .RepoDigests 0}}' "$tag")
echo "push: $pushed"

started=$(date +%s)
docker stop "$name" > /dev/null
took=$(( $(date +%s) - started ))
code=$(docker inspect --format '{{.State.ExitCode}}' "$name")
[ "$took" -lt 8 ] || fail "docker stop took ${took}s: SIGTERM was not a clean stop"
[ "$code" = 0 ] || { docker logs "$name" | tail -n 20; fail "exit code $code after docker stop"; }
echo "stop: clean, ${took}s, exit 0"

docker start "$name" > /dev/null
wait_up
docker image rm "$tag" > /dev/null
docker pull -q "$tag" > /dev/null
[ "$(docker inspect --format '{{index .RepoDigests 0}}' "$tag")" = "$pushed" ] || fail "the pull after a restart differs"
echo "restart: the volume kept the push; the stale lock did not stop the start"
echo "image gate: ok"
