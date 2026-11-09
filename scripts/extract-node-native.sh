#!/usr/bin/env bash
# Extracts discord_voice.node by installing the official Discord installer
# natively on Windows (Git Bash on the runners) - no wine involved. Mirrors
# extract-windows-node.sh's output contract:
# <outdir>/app-<ver>/discord_voice.node + metadata.json {source, app_version,
# md5, size}. Ends with parseable "host=..." / "md5=..." lines for CI.
# Fallback when the module is missing: read the discord_voice full.distro URL
# from the updater log and fetch it directly (brotli + tar via node core).
# Usage: extract-node-native.sh <stable|ptb|canary> [outdir]
set -euo pipefail

usage() {
  echo "usage: $0 <stable|ptb|canary> [outdir]" >&2
  exit 2
}

CHANNEL="${1:-}"
[ -n "$CHANNEL" ] || usage
case "$CHANNEL" in stable | ptb | canary) ;; *) usage ;; esac
OUTDIR="${2:-$PWD/node-out}"
case "$CHANNEL" in
  stable) INSTALLER_URL="https://discord.com/api/download/stable?platform=win"; DIR="Discord" ;;
  ptb) INSTALLER_URL="https://discord.com/api/download/ptb?platform=win"; DIR="DiscordPTB" ;;
  canary) INSTALLER_URL="https://discord.com/api/download/canary?platform=win"; DIR="discordcanary" ;;
esac
LOCALAPP="$LOCALAPPDATA"
[ -n "$LOCALAPP" ] || { echo "error: LOCALAPPDATA not set (run under Git Bash on Windows)" >&2; exit 1; }
mkdir -p "$OUTDIR"

# Extracts one tar member ("files/discord_voice.node") from a brotli stream
# without GNU tar/Python deps: node core (zlib + header walk) only.
tar_member() { # $1 = .distro path, $2 = node output path
  node - "$1" "$2" <<'EOF'
const fs = require("fs");
const { brotliDecompressSync } = require("zlib");
const data = brotliDecompressSync(fs.readFileSync(process.argv[2]));
for (let i = 0; i + 512 <= data.length; ) {
  const rawName = data.subarray(i, i + 100);
  const end = rawName.indexOf(0);
  const name = String(rawName.subarray(0, end === -1 ? 100 : end));
  const sizeStr = data.subarray(i + 124, i + 136).toString("latin1").replace(/\0/g, "").trim();
  const size = parseInt(sizeStr, 8) || 0;
  const type = String.fromCharCode(data[i + 156]);
  const body = i + 512;
  const pad = (512 - (size % 512)) % 512;
  if ((type === "0" || type === "\0") && name === "files/discord_voice.node") {
    fs.writeFileSync(process.argv[3], data.subarray(body, body + size));
    console.error(`tar_member: extracted ${name} (${size}b)`);
    process.exit(0);
  }
  i = body + size + pad;
}
console.error("tar_member: member files/discord_voice.node not found");
process.exit(1);
EOF
}

catalog() { # $1 = node path; $2 = explicit "app-<ver>" override (distro fallback)
  local node="$1" base="${2:-}" dest
  [ -n "$base" ] || base="$(printf '%s' "$node" | awk -F/ '{print $(NF-4)}')"
  [[ "$base" =~ ^app-[0-9]+\.[0-9]+\.[0-9]+$ ]] || base="unknown-$(date +%Y%m%d)"
  dest="$OUTDIR/$base"
  mkdir -p "$dest"
  cp -f "$node" "$dest/discord_voice.node"
  node - "$dest/discord_voice.node" "$base" "$CHANNEL" "$dest/metadata.json" <<'EOF'
const fs = require("fs");
const crypto = require("crypto");
const [path, appVersion, source, metaPath] = process.argv.slice(2);
const data = fs.readFileSync(path);
const md5 = crypto.createHash("md5").update(data).digest("hex");
fs.writeFileSync(metaPath, `${JSON.stringify({ source, app_version: appVersion, md5, size: data.length }, null, 2)}\n`);
console.log(`${source}: ${appVersion} md5=${md5} size=${data.length} -> ${path}`);
console.log(`md5=${md5}`);
console.log(`host=${appVersion}`);
EOF
}

main() {
  local tmp_root="${TMPDIR:-${TEMP:-/tmp}}" setup="" node="" catalog_base=""
  setup="$tmp_root/Discord-${CHANNEL}Setup.exe"
  echo "downloading Discord ${CHANNEL} installer..."
  curl -fsSL --retry 2 "$INSTALLER_URL" -o "$setup"
  echo "installing silently (native, ~1-2 min)..."
  MSYS_NO_PATHCONV=1 "$setup" /S >/dev/null 2>&1 || true
  for _ in $(seq 1 150); do
    node="$(find "$LOCALAPP/$DIR" -type f -path '*/modules/discord_voice-1/discord_voice/discord_voice.node' 2>/dev/null | sort -V | tail -1)"
    [ -n "$node" ] && break
    sleep 4
  done
  if [ -z "$node" ]; then
    echo "module absent after install; trying discord_voice full.distro fallback..."
    local log url ver tmp
    log="$(find "$LOCALAPP/$DIR" -maxdepth 3 -name '*updater_rCURRENT.log' 2>/dev/null | head -1 || true)"
    url="$(grep -o 'https://[^"]*discord_voice[^"]*full\.distro' "$log" 2>/dev/null | head -1 || true)"
    [ -n "$url" ] || { echo "error: discord_voice.node not found under $LOCALAPP/$DIR" >&2; exit 1; }
    ver="$(printf '%s' "$url" | sed -n 's|.*/x64/\([0-9.]*\)/discord_voice.*|\1|p')"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    curl -fsSL "$url" -o "$tmp/voice.distro"
    tar_member "$tmp/voice.distro" "$tmp/discord_voice.node"
    mkdir -p "$OUTDIR/app-$ver"
    node="$OUTDIR/app-$ver/discord_voice.node"
    cp -f "$tmp/discord_voice.node" "$node"
    catalog_base="app-$ver"
  fi
  catalog "$node" "$catalog_base"
}

main
