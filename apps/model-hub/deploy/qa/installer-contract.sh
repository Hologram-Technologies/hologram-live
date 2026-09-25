#!/usr/bin/env bash
# Does install-openapi.sh actually produce the block we reviewed?
#
# The Caddyfile in this repo is the block as it should end up. install-openapi.sh is what edits the real one on
# the host, in place, and the two are written separately — which has already gone wrong once: a rule was added
# to Caddyfile.hub, rehearsed there, and never added to the installer, so it shipped to a host that did not get
# it. Rehearsing the config file is not rehearsing the installer.
#
# This runs the installer's own embedded Python — extracted from the script, not reimplemented — against two
# starting points, and asserts both land on exactly the reviewed block:
#
#   fresh    a host that has none of the edits
#   upgrade  a host that has an earlier version of them, which is what the live host actually is
#
# No Docker, no network, no host. Run it on every change to either file.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DEPLOY=$(dirname "$HERE")
# The host the installer itself defaults to, read from the installer, so this cannot drift from it -- the
# hub was renamed once already while this was being written.
HOST=${HOST:-$(sed -n 's/^HOST=${HOST:-\(.*\)}$/\1/p' "$DEPLOY/install-openapi.sh" | head -1)}
[ -n "$HOST" ] || { echo "could not read the default HOST out of install-openapi.sh"; exit 1; }
WANT="$DEPLOY/Caddyfile.hub"
INSTALLER="$DEPLOY/install-openapi.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# A Windows checkout can hold this file with CRLF while the installer writes LF, which would make every line
# of a correct result look changed. The comparison is about content, so both sides are normalised.
lf() { tr -d '\r' < "$1" > "$2"; }
lf "$WANT" "$WORK/want.caddy"
WANT="$WORK/want.caddy"

# The installer's edit step, exactly as the host runs it.
sed -n "/python3 - \"\$CADDYFILE\"/,/^PY$/p" "$INSTALLER" | sed '1d;$d' > "$WORK/edit.py"
[ -s "$WORK/edit.py" ] || { echo "could not extract the embedded editor from install-openapi.sh"; exit 1; }

fails=0
check() { # name before-file
	local name=$1 before=$2
	if ! python3 "$WORK/edit.py" "$before" "$HOST" > "$WORK/log" 2>&1; then
		echo "  FAIL $name: the editor refused"; sed 's/^/        /' "$WORK/log"; fails=$((fails + 1)); return
	fi
	lf "$before" "$WORK/got.caddy"
	if diff -u "$WANT" "$WORK/got.caddy" > "$WORK/diff"; then
		echo "  ok   $name  ($(sed -n 's/^edited the block in place: //p' "$WORK/log"))"
	else
		echo "  FAIL $name: the installer's result is not the reviewed Caddyfile"
		sed -n '1,40p' "$WORK/diff" | sed 's/^/        /'
		fails=$((fails + 1))
	fi
}

# ---- upgrade: the live host, which already has everything up to the section briefs
#
# Strip exactly what the newest edit adds, and nothing else, so what is left is the previous version of the
# block rather than a guess at one.
python3 - "$WANT" "$WORK/upgrade.caddy" <<'PY'
import io, re, sys
src, dst = sys.argv[1], sys.argv[2]
t = io.open(src, encoding="utf-8").read()
before = t
# the JSON matcher and its rewrite
t = re.sub(r"\t\t# A caller that asks for JSON by name.*?\t\trewrite @section_json /\{path\.0\}\.json\n", "", t, flags=re.S)
# the brief matcher's exclusion of JSON -- the one INSIDE @section_brief. The root's @arriving matcher has a
# line of its own that reads identically and is older than any of this.
i = t.index("\t\t@section_brief {")
line = "\t\t\tnot header Accept *application/json*\n"
if line not in t[i:]:
    raise SystemExit("@section_brief does not exclude JSON; this test no longer knows what the newest edit adds")
t = t[:i] + t[i:].replace(line, "", 1)
# the descriptors' CORS lines and the Link headers
t = re.sub(r'\t\theader /(models|registry|spaces|buckets|docs)\.json Access-Control-Allow-Origin "\*"\n', "", t)
t = re.sub(r"\t\t# RFC 8288, so an agent finds.*?\n", "", t)
t = re.sub(r'\t\theader /(models\*|registry\*|spaces\*|buckets\*|docs/\*) Link "[^"]*"\n', "", t)
if t == before:
    raise SystemExit("nothing was stripped: this test no longer knows what the newest edit adds")
if "@section_json" in t or "header /models* Link" in t:
    raise SystemExit("the strip left part of the newest edit behind")
io.open(dst, "w", encoding="utf-8", newline="\n").write(t)
PY
check "upgrade from the section briefs" "$WORK/upgrade.caddy"

echo
if [ "$fails" -ne 0 ]; then
	echo "$fails of the installer's paths do not reproduce Caddyfile.hub"
	exit 1
fi
echo "install-openapi.sh reproduces the reviewed block"
