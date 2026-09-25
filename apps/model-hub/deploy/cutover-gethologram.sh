#!/usr/bin/env bash
# cutover-gethologram.sh: give the hub its new name, gethologram.ai, on the host that already serves it.
#
# The hub is one Caddy site block in front of six backends (site, resolve, registry, server, account, kappa). Nothing
# about that changes. This script does the four things the rename needs on the host, each applied only when missing:
#
#   1. the front door answers the new name too: the site label `hub.uor.foundation {` becomes
#      `gethologram.ai, hub.uor.foundation {`, and a `www.gethologram.ai` block redirects to the apex;
#   2. /benches/* is served: the benchmark JSON that Hologram-Technologies/hologram pushes into the hologram-website
#      repository every run, refreshed by a 10-minute `git pull` instead of the daily site build;
#   3. the manifest for the MCP registry names the new domain (the listing itself is `listing`, after DNS);
#   4. everything is verified from outside, on both names, and rolled back if it does not verify.
#
# Run order:   cutover-gethologram.sh check      (no changes; says what it would do)
#              cutover-gethologram.sh install    (edits, validates, reloads, verifies the OLD name; prints the DNS step)
#              -- point the gethologram.ai A record at this host, drop the GitHub Pages records --
#              cutover-gethologram.sh verify     (both names, over real TLS)
#              cutover-gethologram.sh listing    (publish the MCP registry entry for the new domain)
#              cutover-gethologram.sh rollback   (restore the Caddyfile and compose backups this script made)
#
# It never prints the Caddyfile: the live one carries tokens inline. Backups are dated and left next to the originals.
set -euo pipefail

CADDYFILE=${CADDYFILE:-/root/twenty/Caddyfile}
HUB=${HUB:-/root/hub}
COMPOSE=${COMPOSE:-$HUB/docker-compose.yml}
SITE=${SITE:-$HUB/site}
NEW=${NEW:-gethologram.ai}
OLD=${OLD:-hub.uor.foundation}
WEBSITE_REPO=${WEBSITE_REPO:-https://github.com/Hologram-Technologies/hologram-website.git}
BENCHES=${BENCHES:-$HUB/benches}
STAMP=$(date -u +%Y-%m-%d)
BACKUP="$CADDYFILE.bak-$STAMP-gethologram"
COMPOSE_BACKUP="$COMPOSE.bak-$STAMP-gethologram"
CRON_LINE="*/10 * * * * git -C $BENCHES pull -q --ff-only >/dev/null 2>&1"

die() { echo "FAIL $*" >&2; exit 1; }
say() { echo "  $*"; }

caddy_container() {
	local found
	found=$(docker ps --format '{{.Names}}' | grep -i caddy || true)
	[ "$(echo "$found" | grep -c .)" = 1 ] || die "expected exactly one running caddy container, found: ${found:-none}. Set CADDY=<name>."
	echo "$found"
}
CADDY=${CADDY:-}

preconditions() {
	[ "$(id -u)" = 0 ] || die "run as root"
	[ -f "$CADDYFILE" ] || die "no Caddyfile at $CADDYFILE"
	grep -qE "^([^{]*, )?$OLD(, [^{]*)? \{" "$CADDYFILE" || die "$CADDYFILE has no site block naming $OLD: this script only edits that block"
	[ -f "$COMPOSE" ] || die "no compose file at $COMPOSE"
	grep -q './archive:/srv/archive:ro' "$COMPOSE" || die "$COMPOSE has no archive mount on the site service; refusing to guess where the benches mount goes"
	[ -f "$SITE/agent.md" ] || die "$SITE/agent.md is missing: build the site first (build-site.sh)"
	head -1 "$SITE/agent.md" | grep -q "^# $NEW\$" || die "$SITE/agent.md still opens with the old name: build the site from a revision that carries the rename (apps/model-hub/web/src/origin.mjs) before the front door learns the new name, or the hero would advertise a lie"
	command -v git >/dev/null || die "git is needed for the benches checkout"
	[ -n "$CADDY" ] || CADDY=$(caddy_container)
}

label_applied()   { grep -qE "^$NEW, $OLD \{" "$CADDYFILE"; }
www_applied()     { grep -qE "^www\.$NEW \{" "$CADDYFILE"; }
benches_applied() { grep -q 'header /benches/\* Access-Control-Allow-Origin' "$CADDYFILE" && grep -q './benches/public/benches:/srv/benches:ro' "$COMPOSE" && [ -d "$BENCHES/.git" ] && crontab -l 2>/dev/null | grep -qF "$BENCHES pull"; }
applied() { label_applied && www_applied && benches_applied; }

apply_edits() {
	python3 - "$CADDYFILE" "$NEW" "$OLD" <<'PY'
import io, re, sys
path, new, old = sys.argv[1], sys.argv[2], sys.argv[3]
text = io.open(path, encoding="utf-8").read()
done = []

# 1. the label. Only the hub's own block, matched at the start of a line.
if not re.search(rf"^{re.escape(new)}, {re.escape(old)} \{{", text, re.M):
    text, n = re.subn(rf"^{re.escape(old)} \{{", f"{new}, {old} {{", text, count=1, flags=re.M)
    if not n:
        raise SystemExit(f"no line reads exactly '{old} {{'; the block label has a shape this script does not know")
    done.append(f"front door now answers {new} and {old}")

# 2. the benches header, next to the archive one inside the site handler.
anchor = '\t\theader /archive.json Access-Control-Allow-Origin "*"\n'
if "header /benches/* Access-Control-Allow-Origin" not in text:
    if anchor not in text:
        raise SystemExit("the site handler does not carry the archive.json header line; refusing to guess where to insert")
    text = text.replace(anchor, anchor + '\t\t# Benchmark results, pushed into the website repository by the hologram benchmarks workflow; a data endpoint.\n\t\theader /benches/* Access-Control-Allow-Origin "*"\n', 1)
    done.append("added the /benches/* header")

# 3. www redirects to the apex. Its own block, appended, so nothing inside the hub block moves.
if not re.search(rf"^www\.{re.escape(new)} \{{", text, re.M):
    if not text.endswith("\n"):
        text += "\n"
    text += f"\nwww.{new} {{\n\tredir https://{new}{{uri}} permanent\n}}\n"
    done.append(f"added the www.{new} redirect block")

io.open(path, "w", encoding="utf-8", newline="\n").write(text)
for d in done:
    print("  " + d)
if not done:
    print("  Caddyfile already carried every edit")
PY
}

apply_compose() {
	if grep -q './benches/public/benches:/srv/benches:ro' "$COMPOSE"; then say "compose already mounts the benches"; return; fi
	python3 - "$COMPOSE" <<'PY'
import io, sys
path = sys.argv[1]
text = io.open(path, encoding="utf-8").read()
anchor = "      - ./archive:/srv/archive:ro\n"
if anchor not in text:
    raise SystemExit("the site service's archive mount line has a shape this script does not know")
text = text.replace(anchor, anchor + "      # /benches/*: benchmark JSON from the hologram-website repository, pulled every 10 minutes (cutover-gethologram.sh).\n      - ./benches/public/benches:/srv/benches:ro\n", 1)
io.open(path, "w", encoding="utf-8", newline="\n").write(text)
print("  compose: the site service mounts ./benches/public/benches on /srv/benches")
PY
}

apply_benches() {
	if [ ! -d "$BENCHES/.git" ]; then
		git clone --quiet --depth 1 --filter=blob:none --sparse "$WEBSITE_REPO" "$BENCHES"
		git -C "$BENCHES" sparse-checkout set public/benches
		say "cloned public/benches from $WEBSITE_REPO into $BENCHES"
	else
		git -C "$BENCHES" pull -q --ff-only || die "$BENCHES does not fast-forward; fix it by hand"
		say "benches checkout is current"
	fi
	[ -f "$BENCHES/public/benches/current.json" ] || die "$BENCHES/public/benches/current.json is missing after the checkout"
	# The site container mounts /srv/benches inside the read-only site; the mount point must exist (same trap as archive).
	mkdir -p "$SITE/benches"
	if ! crontab -l 2>/dev/null | grep -qF "$BENCHES pull"; then
		( crontab -l 2>/dev/null; echo "$CRON_LINE" ) | crontab -
		say "cron: benches pull every 10 minutes"
	fi
}

apply_manifest() {
	# The MCP registry manifest on the host is what publish-mcp-listing.sh sends; it must name the new domain.
	local m="$HUB/mcp-server.json"
	[ -f "$m" ] || { say "no $m on the host; \`listing\` will fetch one"; return; }
	if grep -q "\"$OLD" "$m"; then
		cp -a "$m" "$m.bak-$STAMP-gethologram"
		sed -i "s#\"foundation\\.uor\\.hub/model-hub\"#\"ai.gethologram/model-hub\"#; s#https://$OLD#https://$NEW#g" "$m"
		say "mcp-server.json now names $NEW (backup $m.bak-$STAMP-gethologram)"
	fi
}

# Verification runs from this host through the public front door. Before DNS moves, the new name is reached by
# pinning it to this host and skipping the certificate (Caddy cannot have issued one yet); after DNS, over real TLS.
public_ip() { curl -s --max-time 10 https://api.ipify.org || curl -s --max-time 10 https://ifconfig.me || true; }
resolves_here() { [ "$(getent ahostsv4 "$1" | awk 'NR==1{print $1}')" = "$(public_ip)" ]; }

verify_host() { # host [pre-dns]
	local host=$1 mode=${2:-tls} fails=0 extra=()
	if [ "$mode" = pin ]; then extra=(-k --resolve "$host:443:127.0.0.1"); fi
	probe() { # name path expected-status [pattern] [curl args...]
		local name=$1 path=$2 want=$3 pattern=${4:-}; shift 4 || shift $#
		local head code
		head=$(mktemp); code=$(curl -s -o /tmp/.hub-cutover-body -D "$head" -w '%{http_code}' --max-time 20 "${extra[@]}" "$@" "https://$host$path" || echo 000)
		if [ "$code" != "$want" ]; then echo "  FAIL $host$path -> $code, expected $want ($name)"; fails=$((fails + 1)); rm -f "$head"; return; fi
		if [ -n "$pattern" ] && ! grep -qi -- "$pattern" "$head" /tmp/.hub-cutover-body; then echo "  FAIL $host$path -> $code but no /$pattern/ ($name)"; fails=$((fails + 1)); rm -f "$head"; return; fi
		echo "  ok   $host$path -> $code ($name)"; rm -f "$head"
	}
	probe "the brief"            /                              200 "^# $NEW"                     -H 'accept: */*'
	probe "the page"             /                              200 '<!doctype html'              -H 'accept: text/html'
	probe "the descriptor"       /                              200 'descriptor/v1'               -H 'accept: application/json'
	probe "vary"                 /                              200 'vary: Accept'                -H 'accept: */*'
	probe "models brief"         /models                        200 "^# $NEW/models"              -H 'accept: */*'
	probe "a model page"         /models/hexgrad/Kokoro-82M/    200 '<!doctype html'              -H 'accept: text/html'
	probe "well-known alias"     /.well-known/openapi.json      200 "\"url\": \"https://$NEW\""
	probe "agent card"           /.well-known/agent-card.json   200 "https://$NEW"
	probe "guide"                /llms.txt                      200 "HF_ENDPOINT=https://$NEW"
	probe "registry"             /v2/                           200 ''
	probe "index tags"           /v2/model-hub/index/tags/list  200 '"tags"'
	probe "models dialect"       "/api/models?limit=1"          200 'modelId'
	probe "account, own origin"  /api/account/health            200 '"ok":true'
	probe "benches"              /benches/current.json          200 'access-control-allow-origin: \*'
	probe "mcp key"              /.well-known/mcp-registry-auth 200 'ed25519'
	if [ "$mode" = tls ]; then
		if ! curl -sI --max-time 20 "https://$host/" >/dev/null; then echo "  FAIL $host: no valid certificate yet (Caddy issues one within a minute of the name resolving here)"; fails=$((fails + 1)); fi
	fi
	rm -f /tmp/.hub-cutover-body
	return $fails
}

verify() {
	local fails=0
	say "old name, over TLS:"
	verify_host "$OLD" tls || fails=$((fails + $?))
	if resolves_here "$NEW"; then
		say "new name resolves to this host: verifying over TLS"
		verify_host "$NEW" tls || fails=$((fails + $?))
		code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 "https://www.$NEW/x" || echo 000)
		loc=$(curl -s -o /dev/null -D - --max-time 20 "https://www.$NEW/x" | grep -i '^location:' | tr -d '\r' || true)
		if [ "$code" = 301 ] && echo "$loc" | grep -q "https://$NEW/x"; then say "ok   www.$NEW -> 301 https://$NEW/x"; else echo "  FAIL www.$NEW/x -> $code $loc (expected 301 to the apex; DNS for www must point here too)"; fails=$((fails + 1)); fi
	else
		say "new name does not resolve to this host yet: verifying routing by pinning it here (certificate check skipped)"
		verify_host "$NEW" pin || fails=$((fails + $?))
	fi
	return $fails
}

restore() {
	if [ -f "$BACKUP" ]; then
		cat "$BACKUP" > "$CADDYFILE"          # in place: never mv onto a bind-mounted file
		docker exec "$CADDY" caddy reload --config /etc/caddy/Caddyfile >/dev/null 2>&1 || docker restart "$CADDY" >/dev/null
		say "restored $CADDYFILE from $BACKUP and reloaded"
	fi
	if [ -f "$COMPOSE_BACKUP" ]; then
		cat "$COMPOSE_BACKUP" > "$COMPOSE"
		docker compose -f "$COMPOSE" up -d site >/dev/null 2>&1 || true
		say "restored $COMPOSE from $COMPOSE_BACKUP"
	fi
}

case "${1:-check}" in
check)
	preconditions
	say "caddy container: $CADDY"
	say "site brief:      $(head -1 "$SITE/agent.md")"
	label_applied   && say "label:   done" || say "label:   would add $NEW to the $OLD site block"
	www_applied     && say "www:     done" || say "www:     would add the www.$NEW redirect block"
	benches_applied && say "benches: done" || say "benches: would clone public/benches, mount it, add the header and the 10-minute pull"
	resolves_here "$NEW" && say "dns:     $NEW resolves to this host" || say "dns:     $NEW does not resolve here yet ($(getent ahostsv4 "$NEW" | awk 'NR==1{print $1}' || echo unresolved))"
	;;
install)
	preconditions
	if applied; then say "already applied"; verify || die "the hub does not verify even though every edit is in place"; exit 0; fi
	cp -a "$CADDYFILE" "$BACKUP"; say "backed up $CADDYFILE to $BACKUP"
	cp -a "$COMPOSE" "$COMPOSE_BACKUP"; say "backed up $COMPOSE to $COMPOSE_BACKUP"
	apply_benches
	apply_compose
	docker compose -f "$COMPOSE" up -d site >/dev/null || { restore; die "the site container did not come up with the benches mount"; }
	say "site container recreated with the benches mount"
	apply_edits
	if ! docker exec "$CADDY" caddy validate --config /etc/caddy/Caddyfile >/dev/null 2>&1; then
		echo "FAIL the edited Caddyfile does not validate" >&2; restore; exit 1
	fi
	say "validated"
	docker exec "$CADDY" caddy reload --config /etc/caddy/Caddyfile >/dev/null || { restore; die "reload failed"; }
	say "reloaded"
	apply_manifest
	sleep 3
	if ! verify; then echo "FAIL verification failed after the reload" >&2; restore; exit 1; fi
	say "installed and verified"
	if ! resolves_here "$NEW"; then
		echo
		echo "NEXT, at the registrar's DNS for $NEW (ns1-3.rrpproxy.net):"
		echo "  A     $NEW      -> $(public_ip)      (replace the four GitHub Pages A records; no AAAA)"
		echo "  CNAME www.$NEW  -> $NEW"
		echo "then remove the custom domain from the hologram-website Pages settings, and run:  $0 verify"
		echo "Caddy issues the certificate on its own once the name resolves here; expect up to a minute."
	fi
	;;
verify)
	verify || die "the hub does not verify"
	;;
listing)
	[ -f "$HUB/publish-mcp-listing.sh" ] || die "$HUB/publish-mcp-listing.sh is missing; copy it from apps/model-hub/deploy"
	[ -f "$HUB/mcp-server.json" ] || curl -fsSL "https://raw.githubusercontent.com/Hologram-Technologies/hologram-live/${HUB_BRANCH:-main}/apps/model-hub/deploy/mcp-server.json" -o "$HUB/mcp-server.json"
	grep -q "https://$NEW/mcp" "$HUB/mcp-server.json" || die "$HUB/mcp-server.json does not point at https://$NEW/mcp"
	resolves_here "$NEW" || die "$NEW does not resolve to this host yet; the registry verifies the domain over HTTPS"
	curl -fsS "https://$NEW/.well-known/mcp-registry-auth" >/dev/null || die "https://$NEW/.well-known/mcp-registry-auth does not answer"
	DOMAIN=$NEW "$HUB/publish-mcp-listing.sh"
	;;
rollback)
	[ -n "$CADDY" ] || CADDY=$(caddy_container)
	BACKUP=$(ls -1t "$CADDYFILE".bak-*-gethologram 2>/dev/null | head -1 || true)
	COMPOSE_BACKUP=$(ls -1t "$COMPOSE".bak-*-gethologram 2>/dev/null | head -1 || true)
	[ -n "$BACKUP$COMPOSE_BACKUP" ] || die "no backup made by this script was found"
	restore
	;;
*)
	echo "usage: $0 check|install|verify|listing|rollback" >&2; exit 2
	;;
esac
