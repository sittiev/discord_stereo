#!/usr/bin/env bash
# Processes downloaded schema-2 reports (app traces) into Patch-DB tables with
# the same policy as the build pipeline: full coverage -> commit and push to
# main (autopatch validates the merged table); anything else -> issue with the
# report evidence. Reports carry no node binary, so the apply/revert gate runs
# only in the build pipeline.
# Env: GH_TOKEN (push/issue; absent = dry run), REPO (owner/name), RUN_URL.
# Usage: discord-reports-process.sh <reports-dir> [work-dir]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORTS="${1:?usage: discord-reports-process.sh <reports-dir> [work-dir]}"
WORK="${2:-${RUNNER_TEMP:-/tmp}/reports-work}"
REPO="${REPO:?REPO is required}"
RUN_URL="${RUN_URL:-local}"
mkdir -p "$WORK"

dry=0
if [ -z "${GH_TOKEN:-}" ] || [ "${DRY_RUN:-0}" = "1" ]; then
  dry=1
  echo "discord-reports-process: dry run (no GH_TOKEN / DRY_RUN=1) - no push or issue will be created"
fi

shopt -s nullglob
reports=("$REPORTS"/*.json)
if [ "${#reports[@]}" -eq 0 ]; then
  echo "discord-reports-process: no new reports"
  exit 0
fi

for report in "${reports[@]}"; do
  read -r rid goal os dir md5 < <(node -e '
    const r = JSON.parse(require("node:fs").readFileSync(process.argv[1], "utf8"));
    console.log([r.report_id, r.goal, r.os, r.discord_dir, r.node_md5].join(" "));
  ' "$report")
  if [ -z "$rid" ] || [ -z "$md5" ]; then
    echo "discord-reports-process: skip unreadable report $report"
    continue
  fi
  platform="$os"
  [ "$os" = "windows" ] && platform="win"
  host="${dir#app-}"
  short="${md5:0:8}"
  tag="$platform-$host-$short-$rid"
  echo "== report $rid ($platform $host, goal $goal, md5 $md5)"

  git -C "$ROOT" checkout -q HEAD -- patchdb/tables.json
  cp "$ROOT/patchdb/tables.json" "$WORK/head-$tag.json"
  rc=skip
  node "$ROOT/patchdb/autopatch.mjs" "$report" "$ROOT/patchdb/tables.json" >"$WORK/$tag.auto.log" 2>&1 && rc=0 || rc=$?
  echo "   autopatch rc=$rc"

  if [ "$rc" = "0" ]; then
    if node "$ROOT/scripts/discord-watch.mjs" tables-differ --head "$WORK/head-$tag.json" --tables "$ROOT/patchdb/tables.json" >/dev/null 2>&1; then
      if [ "$dry" = "1" ]; then
        echo "   [dry] would commit patchdb/tables.json and push to main"
      else
        git -C "$ROOT" config user.name "stereo-watch-bot"
        git -C "$ROOT" config user.email "actions@users.noreply.github.com"
        git -C "$ROOT" add patchdb/tables.json
        git -C "$ROOT" commit -m "patchdb: add $platform $host table"
        if ! git -C "$ROOT" push origin HEAD:main; then
          git -C "$ROOT" fetch origin main
          git -C "$ROOT" rebase origin/main
          git -C "$ROOT" push origin HEAD:main
        fi
        echo "   pushed to main"
      fi
    else
      echo "   no real byte change (db_version only); push skipped"
    fi
    continue
  fi

  title="patchdb: manual RE for $platform $host ($short) - $goal"
  if [ "$dry" = "0" ]; then
    open_count="$(gh issue list --repo "$REPO" --state open --search "$title in:title" --json number --jq 'length' || echo 0)"
    if [ "$open_count" != "0" ]; then
      echo "   open issue already exists; skip"
      continue
    fi
  fi
  body="$WORK/issue-$tag.md"
  node "$ROOT/scripts/discord-watch.mjs" note issue --host "$host" --md5 "$md5" --platform "$platform" --run-url "$RUN_URL" --report "$report" >"$body"
  if [ "$dry" = "1" ]; then
    echo "   [dry] would open issue: $title"
    continue
  fi
  gh issue create --repo "$REPO" --title "$title" --body-file "$body" || echo "   issue create failed"
done

git -C "$ROOT" checkout -q HEAD -- patchdb/tables.json
