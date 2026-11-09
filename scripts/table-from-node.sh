#!/usr/bin/env bash
# Patch-DB intake for a new node build, end to end:
#   map the reference table by symbol/delta -> merge only full-coverage
#   reports through autopatch + validate -> verify apply/revert on a copy ->
#   optionally extend the offset-profile bank.
# It refuses to touch the tables while any site is unresolved (manual RE);
# on failure it keeps the workdir so the mapping/report can be inspected.
# Nothing is committed: the caller reviews the table, tests audio and opens the PR.
#
# Usage:
#   table-from-node.sh --node <path> --ref-node <path> --ref-md5 <md5> --goal v6
#     [--discord app-X] [--arch x86_64] [--tables patchdb/tables.json]
#     [--profiles patchdb/offset_profiles.json] [--no-learn] [--keep-work]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NODE=""
REF_NODE=""
REF_MD5=""
GOAL=""
DISCORD=""
ARCH="x86_64"
TABLES="$ROOT/patchdb/tables.json"
PROFILES="$ROOT/patchdb/offset_profiles.json"
LEARN=1
KEEP=0

while [ $# -gt 0 ]; do
  case "$1" in
    --node) NODE="$2"; shift 2 ;;
    --ref-node) REF_NODE="$2"; shift 2 ;;
    --ref-md5) REF_MD5="$2"; shift 2 ;;
    --goal) GOAL="$2"; shift 2 ;;
    --discord) DISCORD="$2"; shift 2 ;;
    --arch) ARCH="$2"; shift 2 ;;
    --tables) TABLES="$2"; shift 2 ;;
    --profiles) PROFILES="$2"; shift 2 ;;
    --no-learn) LEARN=0; shift ;;
    --keep-work) KEEP=1; shift ;;
    *) echo "table-from-node: unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ -z "$NODE" ] || [ -z "$REF_NODE" ] || [ -z "$REF_MD5" ] || [ -z "$GOAL" ]; then
  echo "usage: $0 --node <path> --ref-node <path> --ref-md5 <md5> --goal <goal> [--discord app-X] [--arch x86_64]" >&2
  exit 2
fi
for file in "$NODE" "$REF_NODE" "$TABLES" "$PROFILES"; do
  [ -f "$file" ] || { echo "table-from-node: not a file: $file" >&2; exit 2; }
done

WORK="$(mktemp -d)"
cleanup() { [ "$KEEP" = "1" ] || rm -rf "$WORK"; }
trap cleanup EXIT

map_args=(
  --tables "$TABLES" --goal "$GOAL" --ref-md5 "$REF_MD5"
  --ref-node "$REF_NODE" --node "$NODE" --arch "$ARCH"
  --mapping "$WORK/mapping.json" --report "$WORK/report.json"
)
[ -n "$DISCORD" ] && map_args+=(--discord-dir "$DISCORD")
python3 "$ROOT/patchdb/map_sites.py" "${map_args[@]}"

if ! python3 - "$WORK/mapping.json" <<'PY'
import json
import sys

mapping = json.load(open(sys.argv[1], encoding="utf-8"))
bad = [site for site in mapping["sites"] if not site["status"].startswith("verified-")]
for site in bad:
    print(f"  unresolved: {site['status']:<16} {site['name']}")
print(f"coverage: {len(mapping['sites']) - len(bad)}/{len(mapping['sites'])}")
sys.exit(3 if bad else 0)
PY
then
  KEEP=1
  echo "table-from-node: unresolved sites need manual RE; workdir kept at $WORK" >&2
  echo "table-from-node: tables/profile untouched" >&2
  exit 3
fi

node "$ROOT/patchdb/autopatch.mjs" "$WORK/report.json" "$TABLES"
node "$ROOT/patchdb/validate.mjs" "$TABLES"

python3 "$ROOT/patchdb/find_offsets.py" scan "$NODE" --goal "$GOAL" --tables "$TABLES" \
  --apply "$WORK/patched.node" >/dev/null

python3 - "$TABLES" "$NODE" "$WORK/patched.node" "$WORK/report.json" <<'PY'
import hashlib
import json
import sys

tables_path, node_path, patched_path, report_path = sys.argv[1:5]
report = json.load(open(report_path, encoding="utf-8"))
db = json.load(open(tables_path, encoding="utf-8"))
table = next(
    item
    for item in db["tables"]
    if item["goal"] == report["goal"] and item["binary_md5"] == report["node_md5"]
)
patched = bytearray(open(patched_path, "rb").read())
bad = []
for site in table["sites"]:
    offset = int(site["offset"], 16)
    patch = bytes.fromhex(site["patch"])
    if patched[offset : offset + len(patch)] != patch:
        bad.append(site["name"])
reverted = bytearray(patched)
for site in table["sites"]:
    offset = int(site["offset"], 16)
    orig = bytes.fromhex(site["orig"])
    reverted[offset : offset + len(orig)] = orig
clean = hashlib.md5(open(node_path, "rb").read()).hexdigest()
print(f"patched md5: {hashlib.md5(patched).hexdigest()}")
print(f"revert md5 : {hashlib.md5(reverted).hexdigest()} (clean {clean})")
if bad or hashlib.md5(reverted).hexdigest() != clean:
    print("apply/revert check failed: " + ", ".join(bad), file=sys.stderr)
    sys.exit(1)
PY

if [ "$LEARN" = "1" ]; then
  md5="$(md5sum "$NODE" | cut -d' ' -f1)"
  python3 "$ROOT/patchdb/find_offsets.py" learn --sample "$md5=$NODE" --goal "$GOAL" \
    --out "$WORK/profiles.json" >/dev/null
  python3 - "$PROFILES" "$WORK/profiles.json" <<'PY'
import json
import sys

bank = json.load(open(sys.argv[1], encoding="utf-8"))
learned = json.load(open(sys.argv[2], encoding="utf-8"))
known = {profile["id"] for profile in bank["profiles"]}
added = [profile for profile in learned["profiles"] if profile["id"] not in known]
bank["profiles"].extend(added)
for key, values in learned.get("aliases", {}).items():
    current = bank.setdefault("aliases", {}).setdefault(key, [])
    for value in values:
        if value not in current:
            current.append(value)
json.dump(bank, open(sys.argv[1], "w", encoding="utf-8"), indent=2)
open(sys.argv[1], "a", encoding="utf-8").write("\n")
print(f"profiles added: {[profile['id'] for profile in added] or 'none'}")
PY
fi

echo "table-from-node: table merged and verified; review $TABLES"
echo "table-from-node: next: audio test, README coverage, then PR"
