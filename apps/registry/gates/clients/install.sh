#!/usr/bin/env bash
# The clients gate C runs that the runner does not carry, each pinned by
# version and by the sha256 its own release publishes. Installed into $1 (a
# directory on PATH afterwards). Linux x86_64.
# Usage: install.sh <bin directory>
set -euo pipefail
bin=$1
mkdir -p "$bin"
work=$(mktemp -d)

fetch() {
  local url=$1 sum=$2 out="$work/$(basename "$1")"
  curl -fsSL --retry 5 -o "$out" "$url"
  printf '%s  %s\n' "$sum" "$out" | sha256sum -c --quiet - || { echo "FAIL: $url does not match its pinned sha256" >&2; exit 1; }
  printf '%s' "$out"
}

# crane, go-containerregistry v0.22.1 (checksums.txt of the release).
tar -xzf "$(fetch https://github.com/google/go-containerregistry/releases/download/v0.22.1/go-containerregistry_Linux_x86_64.tar.gz \
  0ab7a1d6932a213aed964ce97666c3077fe691c8606413674a8b3e0b9ec4cda0)" -C "$bin" crane
# ORAS v1.3.4 (oras_1.3.4_checksums.txt).
tar -xzf "$(fetch https://github.com/oras-project/oras/releases/download/v1.3.4/oras_1.3.4_linux_amd64.tar.gz \
  f27adb935022d94df8dc77719c322dda592c78a0d57a6f7dcdd8d900b248c454)" -C "$bin" oras
# Helm v3.22.0 (get.helm.sh's .sha256sum), the 3.x line most charts target.
tar -xzf "$(fetch https://get.helm.sh/helm-v3.22.0-linux-amd64.tar.gz \
  1e4ab49e429626cf6c6958d914248b78c9730803c2751b87627e171dc800e7bb)" -C "$work" linux-amd64/helm
mv "$work/linux-amd64/helm" "$bin/helm"
"$bin/crane" version
"$bin/oras" version | head -n 1
"$bin/helm" version --short
