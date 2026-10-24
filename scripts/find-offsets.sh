#!/usr/bin/env bash
# Downloads the ProdHallow offset finder (pinned path) and runs it on one
# node file; the full text output lands in <node>.offsets.txt by default.
# Usage: find-offsets.sh <node> [outfile]
set -euo pipefail

NODE="${1:?usage: find-offsets.sh <node> [outfile]}"
OUTFILE="${2:-$NODE.offsets.txt}"
FINDER_URL="https://raw.githubusercontent.com/ProdHallow/Discord-Stereo-Windows-MacOS-Linux/main/Updates/Offset%20Finder/discord_voice_node_offset_finder_v5.py"
FINDER=/tmp/offset_finder_v5.py

curl -fsSL "$FINDER_URL" -o "$FINDER"
python3 "$FINDER" "$NODE" | tee "$OUTFILE"
