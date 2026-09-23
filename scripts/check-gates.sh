#!/usr/bin/env sh
# Every gate must be green on the commit being released.
#
# A tag publishes one commit, so one commit must be judged. "Green somewhere in
# the history" is not the same claim, and neither is "never ran": a workflow
# with no run for this commit fails here, because an absent gate blocks nothing.
#
# Usage:  check-gates.sh <sha>
# Reads:  GH_TOKEN (or GITHUB_TOKEN), GITHUB_REPOSITORY, RELEASE_GATES
set -eu

SHA=${1:-}
if [ -z "$SHA" ]; then
  echo "usage: check-gates.sh <sha>" >&2
  exit 2
fi

REPO=${GITHUB_REPOSITORY:-Hologram-Technologies/hologram-live}
# The workflows that judge a release. `gates` carries A, B, C and the image
# check; `registry-os` runs the store on all three systems; `ci` is the rest.
REQUIRED=${RELEASE_GATES:-"gates registry-os ci"}

if [ -z "${GH_TOKEN:-}" ] && [ -n "${GITHUB_TOKEN:-}" ]; then
  GH_TOKEN=$GITHUB_TOKEN
  export GH_TOKEN
fi

# The runs API filters on the full 40-character sha, and this also proves the
# commit exists in the repository being released.
SHA=$(gh api "repos/$REPO/commits/$SHA" --jq .sha)

runs=$(gh api --paginate "repos/$REPO/actions/runs?head_sha=$SHA&per_page=100" \
  --jq '.workflow_runs[] | [.name, .status, (.conclusion // "pending"), .created_at, (.html_url)] | @tsv')

failed=0
for name in $REQUIRED; do
  # The newest run of this workflow for this commit: a re-run supersedes.
  line=$(printf '%s\n' "$runs" | awk -F'\t' -v want="$name" '$1 == want' | sort -t"$(printf '\t')" -k4,4r | head -n 1)

  if [ -z "$line" ]; then
    echo "gate $name: NO RUN on $SHA" >&2
    failed=1
    continue
  fi

  status=$(printf '%s' "$line" | cut -f2)
  conclusion=$(printf '%s' "$line" | cut -f3)
  url=$(printf '%s' "$line" | cut -f5)

  if [ "$status" != "completed" ]; then
    echo "gate $name: $status, not completed  $url" >&2
    failed=1
  elif [ "$conclusion" != "success" ]; then
    echo "gate $name: $conclusion  $url" >&2
    failed=1
  else
    echo "gate $name: success  $url"
  fi
done

if [ "$failed" -ne 0 ]; then
  echo "" >&2
  echo "refusing to release $SHA: the gates above are not green on this commit" >&2
  exit 1
fi

echo "every required gate is green on $SHA"
