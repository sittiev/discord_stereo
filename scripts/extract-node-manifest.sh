#!/usr/bin/env bash
# Extracts discord_voice.node straight from Discord's distribution manifest
# module package (brotli tar) for any platform/channel - no installer, no
# wine, runs on a plain Linux runner. Same catalog contract as the other
# extractors: <outdir>/app-<host>/discord_voice.node + metadata.json.
# Prints parseable md5=/host=/platform=/channel= lines for CI.
# Usage: extract-node-manifest.sh <win|linux> <stable|ptb|canary> [outdir]
set -euo pipefail

usage() {
  echo "usage: $0 <win|linux> <stable|ptb|canary> [outdir]" >&2
  exit 2
}

PLATFORM="${1:-}"
CHANNEL="${2:-}"
[ -n "$PLATFORM" ] && [ -n "$CHANNEL" ] || usage
case "$PLATFORM" in win | linux) ;; *) usage ;; esac
case "$CHANNEL" in stable | ptb | canary) ;; *) usage ;; esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTDIR="${3:-$PWD/node-out}"
UA='StereoWizard-Watch/0.3 (+https://github.com/sittiev/discord_stereo)'
MANIFEST="https://canary.discord.com/api/updates/distributions/app/manifests/latest?platform=$PLATFORM&channel=$CHANNEL&arch=x64"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "fetching manifest $PLATFORM/$CHANNEL..."
curl -fsSL --retry 2 -A "$UA" "$MANIFEST" -o "$tmp/manifest.json"
read -r host url sha < <(node - "$tmp/manifest.json" <<'EOF'
const fs = require("node:fs");
const manifest = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const full = manifest?.modules?.discord_voice?.full ?? {};
const host = Array.isArray(full.host_version) ? full.host_version.join(".") : "";
console.log([host || "-", full.url || "-", full.package_sha256 || "-"].join(" "));
EOF
)
[ "$host" != "-" ] && [ "$url" != "-" ] || { echo "error: manifest has no discord_voice module" >&2; exit 1; }

echo "downloading discord_voice package ($host)..."
curl -fsSL --retry 2 -A "$UA" "$url" -o "$tmp/voice.distro"
node "$ROOT/scripts/node-from-distro.mjs" "$tmp/voice.distro" "$tmp/discord_voice.node"

dest="$OUTDIR/app-$host"
mkdir -p "$dest"
cp -f "$tmp/discord_voice.node" "$dest/discord_voice.node"

node - "$dest/discord_voice.node" "$dest/metadata.json" "$PLATFORM" "$CHANNEL" "app-$host" "$sha" "$url" <<'EOF'
const fs = require("node:fs");
const crypto = require("node:crypto");
const [nodePath, metaPath, platform, channel, appVersion, moduleSha, moduleUrl] = process.argv.slice(2);
const data = fs.readFileSync(nodePath);
const md5 = crypto.createHash("md5").update(data).digest("hex");
const metadata = { source: "discord-manifest", platform, channel, app_version: appVersion, md5, size: data.length, module_sha256: moduleSha, module_url: moduleUrl };
fs.writeFileSync(metaPath, `${JSON.stringify(metadata, null, 2)}\n`);
console.log(`${platform}/${channel}: ${appVersion} md5=${md5} size=${data.length}`);
console.log(`md5=${md5}`);
console.log(`host=${appVersion}`);
console.log(`platform=${platform}`);
console.log(`channel=${channel}`);
EOF
