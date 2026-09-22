#!/usr/bin/env bash
# Helm: a chart pushed to oci://, pulled back byte for byte, and read by
# `helm show` and `helm template` from the registry (interop matrix row 13).
# `helm install` needs a cluster; it runs in the Kubernetes journey (J3).
source "$(dirname "$0")/lib.sh"
work=$(mktemp -d)
cd "$work"
export HELM_CACHE_HOME="$work/cache" HELM_CONFIG_HOME="$work/config" HELM_DATA_HOME="$work/data"

helm create gate-chart > /dev/null
sed -i 's/^version: .*/version: 0.1.0/' gate-chart/Chart.yaml
helm package -d . gate-chart > /dev/null
pushed=$(helm push gate-chart-0.1.0.tgz "oci://$US/gate-c/charts" --plain-http 2>&1)
digest=$(awk '/^Digest:/ { print $2 }' <<< "$pushed")
[ -n "$digest" ] || fail "helm push printed no digest: $pushed"

mkdir pulled
helm pull "oci://$US/gate-c/charts/gate-chart" --version 0.1.0 --plain-http -d pulled > /dev/null
cmp gate-chart-0.1.0.tgz pulled/gate-chart-0.1.0.tgz || fail "the pulled chart differs from the pushed one"
helm show chart "oci://$US/gate-c/charts/gate-chart" --version 0.1.0 --plain-http | grep -q '^name: gate-chart' ||
  fail "helm show chart"
helm template gate "oci://$US/gate-c/charts/gate-chart" --version 0.1.0 --plain-http | grep -q 'kind: Deployment' ||
  fail "helm template from the registry"
curl -fsS "http://$US/v2/gate-c/charts/gate-chart/tags/list" | grep -q '"0.1.0"' || fail "the chart version is not a tag"
printf 'helm: push, pull, show, template from oci://: ok (%s)\n' "$digest"
