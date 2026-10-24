#!/usr/bin/env bash
# Wrapper for the autopatch pipeline step: merges a divergence report into
# tables.json (candidate table from relocation data only) and re-runs the CI
# validator. autopatch prints a one-line summary JSON; exit code is repassed.
# Usage: table-from-report.sh <report.json> [tables.json]
set -euo pipefail

REPORT="${1:?usage: table-from-report.sh <report.json> [tables.json]}"
TABLES="${2:-patchdb/tables.json}"

node patchdb/autopatch.mjs "$REPORT" "$TABLES"
node patchdb/validate.mjs "$TABLES"
