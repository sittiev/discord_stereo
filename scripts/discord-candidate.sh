#!/usr/bin/env bash
# Turns extracted node artifacts into Patch-DB tables:
#   <dir>/discord-node-<platform>-<channel>/app-<ver>/{discord_voice.node,metadata.json}
# Runs the v6/v7 finder per build, lets autopatch merge only reports with full
# coverage, verifies apply/revert on the clean node and pushes the table to
# main. Opens an issue when a goal needs manual RE or the gate fails.
# Env: GH_TOKEN (push/issue; absent = dry run), REPO (owner/name), RUN_URL.
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
  echo "discord-candidate: dry run (no GH_TOKEN / DRY_RUN=1) - no push or issue will be created"
fi

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

  git -C "$ROOT" checkout -q HEAD -- patchdb/tables.json
  # One decision per goal: v6 always, v7 only where tables exist for the platform
  # (there is no Linux v7 table yet, so Linux builds skip v7).
  goals=(v6)
  [ "$platform" = "win" ] && goals=(v6 v7)
  for goal in "${goals[@]}"; do
    GOAL="$goal" REPORT="$WORK/$goal-$tag.json" \
      bash "$ROOT/scripts/find-offsets.sh" "$node" "$WORK/$goal-$tag.txt" \
      >"$WORK/$goal-$tag.finder.log" 2>&1 || true
  done
  declare -A rc_of=()
  cp "$ROOT/patchdb/tables.json" "$WORK/head-$tag.json"

  for goal in "${goals[@]}"; do
    report="$WORK/$goal-$tag.json"
    if [ ! -f "$report" ]; then
      rc_of[$goal]="missing"
      continue
    fi
    if node "$ROOT/patchdb/autopatch.mjs" "$report" "$ROOT/patchdb/tables.json" >"$WORK/$goal-$tag.auto.log" 2>&1; then
      rc_of[$goal]=0
    else
      rc_of[$goal]=$?
    fi
  done
  echo "   autopatch v6=${rc_of[v6]:-n/a} v7=${rc_of[v7]:-n/a}"

  merged=0
  merged_count=0
  for goal in "${goals[@]}"; do
    if [ "${rc_of[$goal]}" = "0" ]; then
      merged=1
      merged_count=$((merged_count + 1))
    fi
  done

  if [ "$merged" = "1" ]; then
    if node "$ROOT/scripts/discord-watch.mjs" tables-differ --head "$WORK/head-$tag.json" --tables "$ROOT/patchdb/tables.json" >/dev/null 2>&1; then
      gate_ok=1
      for goal in "${goals[@]}"; do
        [ "${rc_of[$goal]}" = "0" ] || continue
        if ! node "$ROOT/patchdb/verify-table.mjs" "$node" "$md5" "$goal" "$ROOT/patchdb/tables.json" \
          >"$WORK/$goal-$tag.gate.log" 2>&1; then
          gate_ok=0
          rc_of[$goal]="gate"
          echo "   gate failed for $goal: $(tail -1 "$WORK/$goal-$tag.gate.log")"
        fi
      done
      if [ "$gate_ok" = "1" ]; then
        if [ "$dry" = "1" ]; then
          echo "   [dry] would commit patchdb/tables.json and push to main"
        else
          git -C "$ROOT" config user.name "stereo-watch-bot"
          git -C "$ROOT" config user.email "actions@users.noreply.github.com"
          git -C "$ROOT" add patchdb/tables.json
          word=table
          [ "$merged_count" -gt 1 ] && word=tables
          git -C "$ROOT" commit -m "patchdb: add $platform $host $word"
          if ! git -C "$ROOT" push origin HEAD:main; then
            git -C "$ROOT" fetch origin main
            git -C "$ROOT" rebase origin/main
            git -C "$ROOT" push origin HEAD:main
          fi
          echo "   pushed to main"
        fi
      fi
    else
      echo "   no real byte change (db_version only); push skipped"
    fi
  fi

  for goal in "${goals[@]}"; do
    rc="${rc_of[$goal]:-missing}"
    [ "$rc" = "0" ] && continue
    title="patchdb: manual RE for $platform $host (${md5:0:8}) - $goal"
    if [ "$dry" = "0" ]; then
      open_count="$(gh issue list --repo "$REPO" --state open --search "$title in:title" --json number --jq 'length' || echo 0)"
      if [ "$open_count" != "0" ]; then
        echo "   open issue already exists for $goal; skip"
        continue
      fi
    fi
    body="$WORK/issue-$tag-$goal.md"
    if [ "$rc" = "gate" ]; then
      node "$ROOT/scripts/discord-watch.mjs" note issue --host "$host" --md5 "$md5" --platform "$platform" --channel "$channel" \
        --run-url "$RUN_URL" --reason "apply/revert gate failed on the clean node" >"$body"
    elif [ -f "$WORK/$goal-$tag.json" ]; then
      node "$ROOT/scripts/discord-watch.mjs" note issue --host "$host" --md5 "$md5" --platform "$platform" --channel "$channel" \
        --run-url "$RUN_URL" --report "$WORK/$goal-$tag.json" >"$body"
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
done

git -C "$ROOT" checkout -q HEAD -- patchdb/tables.json
