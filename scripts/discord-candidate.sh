#!/usr/bin/env bash
# Turns extracted node artifacts into Patch-DB candidates:
#   <dir>/discord-node-<platform>-<channel>/app-<ver>/{discord_voice.node,metadata.json}
# Runs the v6/v7 finder per build, lets autopatch merge only reports with full
# coverage, opens a PR when the merged tables changed bytes for real and an
# issue when the build needs manual RE. Never publishes directly.
# Env: GH_TOKEN (PR/issue; absent = dry run), REPO (owner/name), RUN_URL.
# Usage: discord-candidate.sh <artifacts-dir> [work-dir]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACTS="${1:?usage: discord-candidate.sh <artifacts-dir> [work-dir]}"
WORK="${2:-${RUNNER_TEMP:-/tmp}/candidate}"
REPO="${REPO:?REPO is required}"
RUN_URL="${RUN_URL:-local}"
mkdir -p "$WORK"

dry=0
if [ -z "${GH_TOKEN:-}" ] || [ "${DRY_RUN:-0}" = "1" ]; then
  dry=1
  echo "discord-candidate: dry run (no GH_TOKEN / DRY_RUN=1) - no PR or issue will be created"
fi

base="$(git -C "$ROOT" rev-parse HEAD)"
shopt -s nullglob
builds=("$ARTIFACTS"/discord-node-*)
if [ "${#builds[@]}" -eq 0 ]; then
  echo "discord-candidate: no node artifacts found in $ARTIFACTS"
  exit 0
fi

for dir in "${builds[@]}"; do
  meta="$(find "$dir" -name metadata.json | head -1)"
  node="$(find "$dir" -name discord_voice.node | head -1)"
  if [ -z "$meta" ] || [ -z "$node" ]; then
    echo "discord-candidate: skip incomplete artifact $dir"
    continue
  fi
  read -r platform channel host md5 < <(node -e '
    const m = JSON.parse(require("node:fs").readFileSync(process.argv[1], "utf8"));
    console.log([m.platform, m.channel, String(m.app_version).replace(/^app-/, ""), m.md5].join(" "));
  ' "$meta")
  if [ -z "$platform" ] || [ -z "$md5" ]; then
    echo "discord-candidate: skip unreadable metadata $meta"
    continue
  fi
  tag="$platform-$channel-$host"
  echo "== $tag (md5 $md5)"

  git -C "$ROOT" checkout -q "$base" -- patchdb/tables.json
  for goal in v6 v7; do
    GOAL="$goal" REPORT="$WORK/$goal-$tag.json" \
      bash "$ROOT/scripts/find-offsets.sh" "$node" "$WORK/$goal-$tag.txt" \
      >"$WORK/$goal-$tag.finder.log" 2>&1 || true
  done

  rc6=skip
  rc7=skip
  if [ -f "$WORK/v6-$tag.json" ]; then
    node "$ROOT/patchdb/autopatch.mjs" "$WORK/v6-$tag.json" "$ROOT/patchdb/tables.json" >"$WORK/v6-$tag.auto.log" 2>&1 && rc6=0 || rc6=$?
  fi
  if [ "$rc6" = 0 ] && [ -f "$WORK/v7-$tag.json" ]; then
    node "$ROOT/patchdb/autopatch.mjs" "$WORK/v7-$tag.json" "$ROOT/patchdb/tables.json" >"$WORK/v7-$tag.auto.log" 2>&1 && rc7=0 || rc7=$?
  fi
  echo "   autopatch v6=$rc6 v7=$rc7"

  if [ "$rc6" = 0 ] && [ "$rc7" = 0 ]; then
    git -C "$ROOT" show "$base":patchdb/tables.json >"$WORK/head-$tag.json" 2>/dev/null || printf '{}\n' >"$WORK/head-$tag.json"
    if ! node "$ROOT/scripts/discord-watch.mjs" tables-differ --head "$WORK/head-$tag.json" --tables "$ROOT/patchdb/tables.json" >/dev/null 2>&1; then
      echo "   no real byte change (db_version only); PR skipped"
      continue
    fi
    branch="db/auto-$tag"
    if [ "$dry" = "1" ]; then
      echo "   [dry] would commit patchdb/tables.json and open PR on $branch"
      continue
    fi
    (
      cd "$ROOT"
      git config user.name "stereo-watch-bot"
      git config user.email "actions@users.noreply.github.com"
      git switch -C "$branch" HEAD
      git add patchdb/tables.json
      git commit -m "patchdb: tables for $platform $host"
      git push --force origin "$branch"
    )
    body="$WORK/pr-$tag.md"
    node "$ROOT/scripts/discord-watch.mjs" note pr --host "$host" --md5 "$md5" --platform "$platform" --channel "$channel" \
      --run-url "$RUN_URL" --auto-v6 "$WORK/v6-$tag.auto.log" --auto-v7 "$WORK/v7-$tag.auto.log" >"$body"
    if ! gh pr create --repo "$REPO" --title "patchdb: tables for $platform $host" --body-file "$body" --base main --head "$branch"; then
      echo "   PR already exists?"
      gh pr list --repo "$REPO" --head "$branch" --json url --jq '.[].url' || true
    fi
    continue
  fi

  title="patchdb: manual RE for $platform $host (${md5:0:8})"
  if [ "$dry" = "0" ]; then
    open_count="$(gh issue list --repo "$REPO" --state open --search "$title in:title" --json number --jq 'length' || echo 0)"
    if [ "$open_count" != "0" ]; then
      echo "   open issue already exists; skip"
      continue
    fi
  fi
  body="$WORK/issue-$tag.md"
  if [ -f "$WORK/v6-$tag.json" ]; then
    node "$ROOT/scripts/discord-watch.mjs" note issue --host "$host" --md5 "$md5" --platform "$platform" --channel "$channel" \
      --run-url "$RUN_URL" --report "$WORK/v6-$tag.json" >"$body"
  else
    node "$ROOT/scripts/discord-watch.mjs" note issue --host "$host" --md5 "$md5" --platform "$platform" --channel "$channel" \
      --run-url "$RUN_URL" --reason "the finder produced no report for this binary" >"$body"
  fi
  if [ "$dry" = "1" ]; then
    echo "   [dry] would open issue: $title"
    continue
  fi
  gh issue create --repo "$REPO" --title "$title" --body-file "$body" || echo "   issue create failed"
done

git -C "$ROOT" checkout -q "$base" -- patchdb/tables.json
