#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/hologram-private-registry.XXXXXX")
auth_dir="$work_dir/registry-auth"
authenticated_config="$work_dir/docker-authenticated"
anonymous_config="$work_dir/docker-anonymous"
project_dir="$work_dir/python-hello"
registry_container="hologram-private-registry-$$"
registry_volume="hologram-private-registry-data-$$"
registry_host=""
seed_base=""
private_base=""
compiled_image_id=""

cleanup() {
  if [[ -n "$registry_host" ]]; then
    DOCKER_CONFIG="$authenticated_config" docker logout "$registry_host" >/dev/null 2>&1 || true
  fi
  if [[ -n "$private_base" ]]; then
    docker image rm --force "$private_base" >/dev/null 2>&1 || true
  fi
  if [[ -n "$seed_base" ]]; then
    docker image rm --force "$seed_base" >/dev/null 2>&1 || true
  fi
  if [[ -n "$compiled_image_id" ]]; then
    docker image rm --force "$compiled_image_id" >/dev/null 2>&1 || true
  fi
  docker container rm --force "$registry_container" >/dev/null 2>&1 || true
  docker volume rm --force "$registry_volume" >/dev/null 2>&1 || true
  rm -rf -- "$work_dir"
}
trap cleanup EXIT

for command in cargo curl docker python3; do
  command -v "$command" >/dev/null || {
    printf 'error: authenticated private-registry proof requires %s\n' "$command" >&2
    exit 1
  }
done

docker version --format '{{.Server.Version}}' >/dev/null
docker buildx version >/dev/null
mkdir -p "$auth_dir" "$authenticated_config" "$anonymous_config"

prepare_isolated_docker_config() {
  local config=$1
  if DOCKER_CONFIG="$config" docker buildx version >/dev/null 2>&1; then
    return
  fi

  local default_config=${DOCKER_CONFIG:-${HOME:?HOME is required to locate Docker CLI plugins}/.docker}
  local candidate
  for candidate in \
    "$default_config/cli-plugins/docker-buildx" \
    /usr/local/lib/docker/cli-plugins/docker-buildx \
    /usr/local/libexec/docker/cli-plugins/docker-buildx \
    /usr/lib/docker/cli-plugins/docker-buildx \
    /usr/libexec/docker/cli-plugins/docker-buildx \
    /opt/homebrew/lib/docker/cli-plugins/docker-buildx \
    /Applications/Docker.app/Contents/Resources/cli-plugins/docker-buildx \
    /Applications/OrbStack.app/Contents/MacOS/xbin/docker-buildx; do
    if [[ -x "$candidate" ]]; then
      mkdir -p "$config/cli-plugins"
      ln -s "$candidate" "$config/cli-plugins/docker-buildx"
      DOCKER_CONFIG="$config" docker buildx version >/dev/null
      return
    fi
  done

  printf 'error: could not expose Docker Buildx in isolated DOCKER_CONFIG=%s\n' "$config" >&2
  exit 1
}

prepare_isolated_docker_config "$authenticated_config"
prepare_isolated_docker_config "$anonymous_config"

# These conspicuous fixture-only values make accidental disclosure detectable.
# They authorize only the disposable loopback registry created below.
registry_username="hologram-registry-fixture-user"
registry_password="hologram-registry-fixture-password-7f3492"
registry_auth=$(printf '%s:%s' "$registry_username" "$registry_password" | base64 | tr -d '\r\n')

docker run --rm --entrypoint htpasswd httpd:2.4-alpine \
  -Bbn "$registry_username" "$registry_password" >"$auth_dir/htpasswd"

docker volume create "$registry_volume" >/dev/null
docker run --detach --name "$registry_container" \
  --publish 127.0.0.1::5000 \
  --volume "$registry_volume:/var/lib/registry" \
  registry:2.8.3 >/dev/null

registry_binding=$(docker port "$registry_container" 5000/tcp | head -n 1)
registry_port=${registry_binding##*:}
if [[ -z "$registry_port" || "$registry_port" == "$registry_binding" ]]; then
  printf 'error: could not determine seed-registry port from %s\n' "$registry_binding" >&2
  exit 1
fi
seed_host="127.0.0.1:$registry_port"

registry_ready=false
for _ in {1..30}; do
  status=$(curl --silent --output /dev/null --write-out '%{http_code}' "http://$seed_host/v2/" || true)
  if [[ "$status" == 200 ]]; then
    registry_ready=true
    break
  fi
  sleep 1
done
if [[ "$registry_ready" != true ]]; then
  printf 'error: seed registry did not become ready\n' >&2
  docker logs "$registry_container" >&2 || true
  exit 1
fi

seed_base="$seed_host/hologram/python:private"
docker pull python:3.12-slim >/dev/null
docker tag python:3.12-slim "$seed_base"
DOCKER_CONFIG="$anonymous_config" docker push "$seed_base" >/dev/null
docker image rm --force "$seed_base" >/dev/null
docker container rm --force "$registry_container" >/dev/null

# Restart the same registry storage behind authentication on a fresh port. The
# anonymous proof therefore cannot reuse an earlier login or registry endpoint.
docker run --detach --name "$registry_container" \
  --publish 127.0.0.1::5000 \
  --volume "$registry_volume:/var/lib/registry" \
  --volume "$auth_dir:/auth:ro" \
  --env REGISTRY_AUTH=htpasswd \
  --env REGISTRY_AUTH_HTPASSWD_REALM='Hologram private-registry fixture' \
  --env REGISTRY_AUTH_HTPASSWD_PATH=/auth/htpasswd \
  registry:2.8.3 >/dev/null

registry_binding=$(docker port "$registry_container" 5000/tcp | head -n 1)
registry_port=${registry_binding##*:}
if [[ -z "$registry_port" || "$registry_port" == "$registry_binding" ]]; then
  printf 'error: could not determine private-registry port from %s\n' "$registry_binding" >&2
  exit 1
fi
registry_host="127.0.0.1:$registry_port"

registry_ready=false
for _ in {1..30}; do
  status=$(curl --silent --output /dev/null --write-out '%{http_code}' "http://$registry_host/v2/" || true)
  if [[ "$status" == 401 ]]; then
    registry_ready=true
    break
  fi
  sleep 1
done
if [[ "$registry_ready" != true ]]; then
  printf 'error: authenticated registry did not become ready\n' >&2
  docker logs "$registry_container" >&2 || true
  exit 1
fi

private_base="$registry_host/hologram/python:private"

cp -R "$repo_root/examples/python-hello" "$project_dir"
python3 - "$project_dir/hologram.json" "$private_base" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
manifest = json.loads(path.read_text())
manifest["layers"][0]["source"]["base"] = sys.argv[2]
path.write_text(json.dumps(manifest, indent=2) + "\n")
PY

target_dir=${CARGO_TARGET_DIR:-"$repo_root/target"}
cargo build --release --locked --package hologram-live --bin hologram \
  --manifest-path "$repo_root/Cargo.toml"
binary="$target_dir/release/hologram"
manifest="$project_dir/hologram.json"
archive="$work_dir/private-registry.holo"
anonymous_archive="$work_dir/anonymous.holo"
check_report="$work_dir/check.json"
anonymous_error="$work_dir/anonymous-error.json"
anonymous_diagnostic="$work_dir/anonymous-error.stderr"
compile_report="$work_dir/compile.json"
run_report="$work_dir/run.json"
hologram_env=(
  env
  "HOLOGRAM_CONFIG_DIR=$work_dir/hologram/config"
  "HOLOGRAM_DATA_DIR=$work_dir/hologram/data"
  "HOLOGRAM_STATE_DIR=$work_dir/hologram/state"
  "HOLOGRAM_CACHE_DIR=$work_dir/hologram/cache"
)

# The registry is live and requires authentication, but --check must remain
# offline and must not require registry credentials.
"${hologram_env[@]}" DOCKER_CONFIG="$anonymous_config" \
  "$binary" --json compile "$manifest" --check >"$check_report"

if "${hologram_env[@]}" DOCKER_CONFIG="$anonymous_config" \
  "$binary" --json compile "$manifest" \
  --output "$anonymous_archive" >"$anonymous_error" 2>"$anonymous_diagnostic"; then
  printf 'error: private rootfs compilation unexpectedly succeeded without credentials\n' >&2
  exit 1
fi
if [[ -e "$anonymous_archive" ]]; then
  printf 'error: failed anonymous compilation left an archive behind\n' >&2
  exit 1
fi

printf '%s\n' "$registry_password" \
  | DOCKER_CONFIG="$authenticated_config" docker login \
      --username "$registry_username" --password-stdin "$registry_host" >/dev/null
"${hologram_env[@]}" DOCKER_CONFIG="$authenticated_config" \
  "$binary" --json compile "$manifest" \
  --output "$archive" >"$compile_report"
compiled_image_id=$(python3 -c \
  'import json,sys; print(json.load(open(sys.argv[1]))["build_provenance"]["layers"][0]["source"]["output"]["image_id"])' \
  "$compile_report")
"${hologram_env[@]}" DOCKER_CONFIG="$anonymous_config" \
  "$binary" --json run "$archive" \
  --input-text Registry >"$run_report"

for secret in "$registry_username" "$registry_password" "$registry_auth"; do
  for artifact in \
    "$check_report" "$anonymous_error" "$anonymous_diagnostic" \
    "$compile_report" "$archive"; do
    if LC_ALL=C grep -a -F -q "$secret" "$artifact"; then
      printf 'error: private-registry test artifact exposed credentials: %s\n' "$artifact" >&2
      exit 1
    fi
  done
done

python3 - \
  "$check_report" "$anonymous_error" "$compile_report" "$run_report" \
  "$private_base" "$registry_username" "$registry_password" "$registry_auth" <<'PY'
import json
import pathlib
import sys

check = json.loads(pathlib.Path(sys.argv[1]).read_text())
anonymous_error = json.loads(pathlib.Path(sys.argv[2]).read_text())
compile_report = json.loads(pathlib.Path(sys.argv[3]).read_text())
run_report = json.loads(pathlib.Path(sys.argv[4]).read_text())
private_base, username, password, encoded_auth = sys.argv[5:]

planned = check["build_provenance"]["layers"][0]["source"]
if planned["base_image"] != {"reference": private_base, "digest_pinned": False}:
    raise SystemExit(f"offline check unexpectedly resolved the private base: {planned!r}")
if planned["reproducibility"].get("reproducible") is not False:
    raise SystemExit(f"offline check claimed an unresolved mutable base: {planned!r}")

if anonymous_error.get("code") != "LIVE_CONFLICT" or private_base not in anonymous_error.get("message", ""):
    raise SystemExit(f"unexpected anonymous registry failure: {anonymous_error!r}")

source = compile_report["build_provenance"]["layers"][0]["source"]
base = source["base_image"]
repository = private_base.rsplit(":", 1)[0]
resolved = base.get("resolved_reference", "")
if base.get("reference") != private_base or base.get("digest_pinned") is not False:
    raise SystemExit(f"completed provenance lost requested private base identity: {base!r}")
if not resolved.startswith(repository + "@sha256:") or len(resolved.removeprefix(repository + "@sha256:")) != 64:
    raise SystemExit(f"completed provenance did not bind a private registry digest: {base!r}")
if source.get("reproducibility") != {"reproducible": True}:
    raise SystemExit(f"authenticated build did not satisfy the rootfs contract: {source!r}")

for report_name, report in (
    ("check", check),
    ("anonymous error", anonymous_error),
    ("compile", compile_report),
):
    encoded = json.dumps(report, separators=(",", ":"))
    for secret in (username, password, encoded_auth):
        if secret in encoded:
            raise SystemExit(f"{report_name} report exposed registry credentials")

def credential_keys(value):
    if isinstance(value, dict):
        for key, child in value.items():
            normalized = key.lower().replace("_", "").replace("-", "")
            if normalized in {
                "auth", "authorization", "credential", "credentials",
                "identitytoken", "password", "registrytoken", "token", "username",
            }:
                yield key
            yield from credential_keys(child)
    elif isinstance(value, list):
        for child in value:
            yield from credential_keys(child)

leaked_keys = list(credential_keys(compile_report["build_provenance"]))
if leaked_keys:
    raise SystemExit(f"build provenance contains credential-bearing fields: {leaked_keys!r}")

output = json.loads(bytes(run_report["outputs"][0]).decode())
expected = {"message": "Hello, Registry!", "name": "Registry", "runtime": "python"}
if output != expected:
    raise SystemExit(f"unexpected private-registry application output: {output!r}")

summary = {
    "schema_version": 1,
    "status": "ok",
    "requested_base": private_base,
    "resolved_base": resolved,
    "offline_check": True,
    "anonymous_rejected": True,
    "credentials_absent_from_provenance": True,
    "application_kappa": compile_report["application_kappa"],
    "archive_kappa": compile_report["archive_kappa"],
}
print(json.dumps(summary, separators=(",", ":")))
PY
