#!/usr/bin/env bash
# Every 5 minutes from cron: restart a hub service that stops answering. The registry has a known failure mode
# (an aborted chunked upload can wedge its accept loop), so a live probe matters more than "container running".
# Probes go through the shared docker network from the Caddy container, so they work regardless of DNS or TLS.
set -uo pipefail

HUB=/root/hub
LOG="$HUB/logs/health.log"
mkdir -p "$HUB/logs"
probe() { docker exec twenty-caddy-1 wget -q -T 10 -O /dev/null "$1" >/dev/null 2>&1; }

if ! probe http://hub-site:8080/index.html; then
  echo "$(date -u +%FT%TZ) site not answering, restarting" >>"$LOG"
  docker compose -f "$HUB/docker-compose.yml" restart site >/dev/null 2>&1
fi
if ! probe http://hub-kappa:5000/v2/; then
  echo "$(date -u +%FT%TZ) kappa not answering, restarting" >>"$LOG"
  docker compose -f "$HUB/docker-compose.yml" restart kappa >/dev/null 2>&1
fi
if ! probe http://hub-server:11435/healthz; then
  echo "$(date -u +%FT%TZ) hologram server not answering, restarting" >>"$LOG"
  docker compose -f "$HUB/docker-compose.yml" restart hologram >/dev/null 2>&1
fi
CATALOG=$(python3 -c 'import json;print(json.load(open("/root/hub/state/model-hub.json")).get("catalog") or "")' 2>/dev/null || true)
if [ -n "$CATALOG" ] && ! docker exec twenty-caddy-1 wget -q -T 10 -O /dev/null "http://hub-site:8080/.well-known/model-hub.json" >/dev/null 2>&1; then
  echo "$(date -u +%FT%TZ) descriptor missing from the site, restoring" >>"$LOG"
  mkdir -p "$HUB/site/.well-known" && cp -f "$HUB/state/model-hub.json" "$HUB/site/.well-known/model-hub.json"
fi
if [ -n "$CATALOG" ] && ! docker exec twenty-caddy-1 wget -q -T 10 -O /dev/null --header "Authorization: Bearer $(cat "$HUB/server-token")" "http://hub-server:11435/api/v1/objects/$CATALOG" >/dev/null 2>&1; then
  echo "$(date -u +%FT%TZ) catalog $CATALOG not served by the hologram server" >>"$LOG"
fi
