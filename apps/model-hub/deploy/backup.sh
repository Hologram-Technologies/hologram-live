#!/usr/bin/env bash
# Nightly off-server backup of the hub registry to the private Filebase S3 bucket, plus a disk-pressure check.
#
# The registry store is content-addressed (blobs named by BLAKE3), so `rclone sync` moves only new blobs and the
# copy is complete by construction. A restore is `rclone sync fb:<bucket>/store /root/hub/store` and a restart.
set -uo pipefail

HUB=/root/hub
BUCKET=hologram-model-hub-backup
LOG=$HUB/logs/backup.log
mkdir -p "$HUB/logs"
exec >>"$LOG" 2>&1
echo "== $(date -u +%FT%TZ) backup start"

set -a; . "$HUB/filebase.env"; set +a
rc() {
  docker run --rm -v "$HUB:/hub:ro" -e RCLONE_CONFIG_FB_TYPE=s3 -e RCLONE_CONFIG_FB_PROVIDER=Other \
    -e RCLONE_CONFIG_FB_ENDPOINT=https://s3.filebase.io -e RCLONE_CONFIG_FB_REGION=auto \
    -e RCLONE_CONFIG_FB_ACCESS_KEY_ID="$FILEBASE_KEY" -e RCLONE_CONFIG_FB_SECRET_ACCESS_KEY="$FILEBASE_SECRET" \
    rclone/rclone:1.71 "$@" 2>&1 | grep -v NOTICE
}
rc sync /hub/store "fb:$BUCKET/store" --transfers 8 --checkers 16 --stats-one-line --stats 0 -v | tail -3
rc copyto /hub/pins.json "fb:$BUCKET/pins.json" -q
rc copyto /hub/registry-token "fb:$BUCKET/registry-token" -q
local_count=$(find "$HUB/store" -type f | wc -l)
remote_count=$(rc size "fb:$BUCKET/store" --json | grep -o '"count":[0-9]*' | cut -d: -f2)
echo "objects local=$local_count remote=$remote_count"
[ "$local_count" = "$remote_count" ] || echo "WARNING: counts differ"

use=$(df --output=pcent / | tail -1 | tr -dc '0-9')
[ "$use" -ge 80 ] && echo "WARNING: disk at ${use}%"
echo "== $(date -u +%FT%TZ) backup done (disk ${use}%)"
