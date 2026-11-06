#!/usr/bin/env bash
# Multi-tier offset finder wrapper. Default engine is patchdb/find_offsets.py;
# the old ProdHallow finder stays available as a fallback.
#
# Usage:
#   find-offsets.sh <node> [outfile]
#   find-offsets.sh --prodhallow <node> [outfile]
#
# Environment:
#   GOAL=v6          goal id from patchdb/tables.json
#   REF_MD5=...      force a reference table by md5
#   REPORT=path      schema-2 report output (default: <outfile>.json)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FINDER="$ROOT/patchdb/find_offsets.py"
PROFILES="$ROOT/patchdb/offset_profiles.json"

usage() {
  printf 'usage: %s [--prodhallow] <node> [outfile]\n' "$(basename "$0")" >&2
  exit 2
}

engine="ours"
if [ "${1:-}" = "--prodhallow" ]; then
  engine="prodhallow"
  shift
fi
NODE="${1:-}"
[ -n "$NODE" ] || usage
OUTFILE="${2:-$NODE.offsets.txt}"
REPORT="${REPORT:-${OUTFILE%.txt}.json}"
GOAL="${GOAL:-v6}"

if [ "$engine" = "prodhallow" ]; then
  FINDER_URL="https://raw.githubusercontent.com/ProdHallow/Discord-Stereo-Windows-MacOS-Linux/main/Updates/Offset%20Finder/discord_voice_node_offset_finder_v5.py"
  TMP_FINDER="/tmp/offset_finder_v5.py"
  curl -fsSL "$FINDER_URL" -o "$TMP_FINDER"
  python3 "$TMP_FINDER" "$NODE" | tee "$OUTFILE"
  exit 0
fi

[ -f "$FINDER" ] || { printf 'missing finder: %s\n' "$FINDER" >&2; exit 2; }
[ -f "$PROFILES" ] || {
  printf 'missing profile bank: %s\n' "$PROFILES" >&2
  printf 'generate it with: python3 patchdb/find_offsets.py learn --sample <md5>=<node> ...\n' >&2
  exit 2
}

args=(scan "$NODE" --goal "$GOAL" --report "$REPORT")
if [ -n "${REF_MD5:-}" ]; then
  args+=(--ref-md5 "$REF_MD5")
fi
python3 "$FINDER" "${args[@]}" | tee "$OUTFILE"
