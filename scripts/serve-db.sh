#!/usr/bin/env bash
# Serves the repo root over HTTP so STEREO_DB_URL can point at
# patchdb/tables.json (dev override for live Patch-DB adoption tests).
# Usage: serve-db.sh [port]
set -euo pipefail

PORT="${1:-8123}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "export STEREO_DB_URL=http://127.0.0.1:$PORT/patchdb/tables.json"
exec python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$ROOT"
