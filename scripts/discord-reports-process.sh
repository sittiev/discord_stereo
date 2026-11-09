#!/usr/bin/env bash
# Processes downloaded schema-2 reports (app traces) into Patch-DB candidates
# with the same policy as the build pipeline: full coverage -> PR built from
# the trace; anything else -> issue with the report evidence. Never publishes.
# Env: GH_TOKEN (PR/issue; absent = dry run), REPO (owner/name), RUN_URL.
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
  echo "discord-reports-process: dry run (no GH_TOKEN / DRY_RUN=1) - no PR or issue will be created"
fi

base="$(git -C "$ROOT" rev-parse HEAD)"
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

  git -C "$ROOT" checkout -q "$base" -- patchdb/tables.json
  rc=skip
  node "$ROOT/patchdb/autopatch.mjs" "$report" "$ROOT/patchdb/tables.json" >"$WORK/$tag.auto.log" 2>&1 && rc=0 || rc=$?
  echo "   autopatch rc=$rc"

  if [ "$rc" = "0" ]; then
    git -C "$ROOT" show "$base":patchdb/tables.json >"$WORK/head-$tag.json" 2>/dev/null || printf '{}\n' >"$WORK/head-$tag.json"
    if ! node "$ROOT/scripts/discord-watch.mjs" tables-differ --head "$WORK/head-$tag.json" --tables "$ROOT/patchdb/tables.json" >/dev/null 2>&1; then
      echo "   no real byte change (db_version only); PR skipped"
      continue
    fi
    branch="db/report-$tag"
    if [ "$dry" = "1" ]; then
      echo "   [dry] would commit patchdb/tables.json and open PR on $branch"
      continue
    fi
    (
      cd "$ROOT"
      git config user.name "stereo-watch-bot"
      git config user.email "actions@users.noreply.github.com"
      git switch -C "$branch" "$base"
      git add patchdb/tables.json
      git commit -m "patchdb: tables from report $rid"
      git push --force origin "$branch"
    )
    body="$WORK/pr-$tag.md"
    node "$ROOT/scripts/discord-watch.mjs" note report --report "$report" --auto "$WORK/$tag.auto.log" --run-url "$RUN_URL" >"$body"
    if ! gh pr create --repo "$REPO" --title "patchdb: tables from report $rid" --body-file "$body" --base main --head "$branch"; then
      echo "   PR already exists?"
      gh pr list --repo "$REPO" --head "$branch" --json url --jq '.[].url' || true
    fi
    continue
  fi

  title="patchdb: manual RE for $platform $host ($short)"
  if [ "$dry" = "0" ]; then
    open_count="$(gh issue list --repo "$REPO" --state open --search "$title in:title" --json number --jq 'length' || echo 0)"
    if [ "$open_count" != "0" ]; then
      echo "   open issue already exists; skip"
      continue
    fi
  fi
  body="$WORK/issue-$tag.md"
  node "$ROOT/scripts/discord-watch.mjs" note issue --host "$host" --md5 "$md5" --platform "$platform" --report "$report" >"$body"
  if [ "$dry" = "1" ]; then
    echo "   [dry] would open issue: $title"
    continue
  fi
  gh issue create --repo "$REPO" --title "$title" --body-file "$body" || echo "   issue create failed"
done

git -C "$ROOT" checkout -q "$base" -- patchdb/tables.json
