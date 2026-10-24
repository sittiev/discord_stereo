#!/usr/bin/env bash
# Extracts the Windows discord_voice.node from the official Discord installer
# (installed silently under wine) and/or the AudioCord NSIS installer (via 7z).
# Each node is cataloged into <outdir>/app-<ver>/discord_voice.node with a
# metadata.json {source, app_version, md5, size}. Fails (exit 1) if a node is
# missing - never continues without the binary.
# Usage: extract-windows-node.sh <discord|audiocord|canary|both> [outdir]
set -euo pipefail

usage() {
  echo "usage: $0 <discord|audiocord|canary|both> [outdir]" >&2
  exit 1
}
TARGET="${1:-}"
[ -n "$TARGET" ] || usage
case "$TARGET" in discord | audiocord | canary | both) ;; *) usage ;; esac

OUTDIR="${2:-$HOME/Downloads/discord-voice-node-reverse/windows}"
AUDIOCORD_EXE="$HOME/Downloads/discord-voice-node-reverse/Audiocord Setup 2.68.0.exe"
WINEPREFIX_DIR="${WINEPREFIX:-$HOME/Games/Programas-Gerais}"
mkdir -p "$OUTDIR"

catalog() { # $1 = source name, $2 = node path
  local ver
  if [[ "$2" =~ app-[0-9]+\.[0-9]+\.[0-9]+ ]]; then
    ver="${BASH_REMATCH[0]}"
  else
    ver="unknown-$(date +%Y%m%d)"
  fi
  local dest="$OUTDIR/$ver"
  mkdir -p "$dest"
  cp -f "$2" "$dest/discord_voice.node"
  python3 - "$1" "$ver" "$dest/discord_voice.node" "$dest/metadata.json" <<'PY'
import hashlib, json, sys
source, ver, path, out = sys.argv[1:5]
data = open(path, "rb").read()
md5 = hashlib.md5(data).hexdigest()
json.dump({"source": source, "app_version": ver, "md5": md5, "size": len(data)},
          open(out, "w"), indent=2)
print(f"{source}: {ver} md5={md5} size={len(data)} -> {path}")
PY
}

# The Squirrel bootstrap crashes under wine before downloading the voice
# module on first launch. Fallback: read the module URL from the updater
# log and fetch the .distro (brotli stream -> tar) directly - no wine needed.
fetch_module_distro() { # $1 = source, $2 = app dir (contains *updater log), $3 = dest dir for the node
  local log url ver tmp node
  log="$(ls "$2"/*updater_rCURRENT.log 2>/dev/null | head -1 || true)"
  [ -n "$log" ] || return 1
  url="$(grep -o 'https://[^"]*discord_voice[^"]*full\.distro' "$log" | head -1 || true)"
  [ -n "$url" ] || return 1
  ver="$(printf '%s' "$url" | sed -n 's|.*/x64/\([0-9.]*\)/discord_voice.*|\1|p')"
  echo "wine launch did not fetch the module; downloading distro directly: $url"
  tmp="$(mktemp -d)"
  curl -fsSL "$url" -o "$tmp/voice.distro"
  python3 - "$tmp" <<'PY'
import brotli, io, os, sys, tarfile
tmp = sys.argv[1]
raw = open(os.path.join(tmp, "voice.distro"), "rb").read()
tar = tarfile.open(fileobj=io.BytesIO(brotli.decompress(raw)))
node = tar.extractfile("files/discord_voice.node").read()
open(os.path.join(tmp, "discord_voice.node"), "wb").write(node)
PY
  mkdir -p "$3/app-$ver"
  cp -f "$tmp/discord_voice.node" "$3/app-$ver/discord_voice.node"
  rm -rf "$tmp"
  printf '%s\n' "$3/app-$ver/discord_voice.node"
  return 0
}

extract_discord() {
  local setup=/tmp/DiscordSetup.exe node=""
  echo "downloading Discord stable installer..."
  curl -fsSL "https://discord.com/api/download/stable?platform=win" -o "$setup"
  mkdir -p "$WINEPREFIX_DIR"
  export WINEPREFIX="$WINEPREFIX_DIR"
  echo "installing under wine prefix $WINEPREFIX_DIR (silent, ~1-3 min)..."
  wine "$setup" /S >/dev/null 2>&1 || true
  for _ in $(seq 1 72); do
    node="$(ls "$WINEPREFIX_DIR"/drive_c/users/*/AppData/Local/Discord/app-*/modules/discord_voice-1/discord_voice/discord_voice.node 2>/dev/null | head -1 || true)"
    [ -n "$node" ] && break
    # NSIS hands off to the Squirrel updater, which launches Discord.exe; the
    # app crashes under wine and pops a scary "installation has failed" dialog
    # even though the files extracted fine. Kill it so the dialog never shows.
    pkill -f 'Discord\.exe' >/dev/null 2>&1 || true
    pkill -f 'update\.exe' >/dev/null 2>&1 || true
    sleep 5
  done
  pkill -f 'Discord\.exe' >/dev/null 2>&1 || true
  wineserver -k >/dev/null 2>&1 || true
  if [ -z "$node" ]; then
    echo "error: discord_voice.node not found under $WINEPREFIX_DIR/drive_c/users/*/AppData/Local/Discord/app-*" >&2
    exit 1
  fi
  catalog discord "$node"
}

extract_canary() {
  local setup=/tmp/DiscordCanarySetup.exe node="" glob
  echo "downloading Discord canary installer..."
  curl -fsSL "https://discord.com/api/download/canary?platform=win" -o "$setup"
  mkdir -p "$WINEPREFIX_DIR"
  export WINEPREFIX="$WINEPREFIX_DIR"
  echo "installing under wine prefix $WINEPREFIX_DIR (silent, ~1-3 min)..."
  wine "$setup" /S >/dev/null 2>&1 || true
  for _ in $(seq 1 72); do
    # Canary installs into %LOCALAPPDATA%\discordcanary (distinct from the
    # stable %LOCALAPPDATA%\Discord); both coexist in the same prefix.
    glob="$WINEPREFIX_DIR"/drive_c/users/*/AppData/Local/discordcanary/app-*/modules/discord_voice-1/discord_voice/discord_voice.node
    node="$(ls $glob 2>/dev/null | head -1 || true)"
    [ -n "$node" ] && break
    pkill -f 'DiscordCanary\.exe' >/dev/null 2>&1 || true
    pkill -f 'Discord\.exe' >/dev/null 2>&1 || true
    pkill -f 'update\.exe' >/dev/null 2>&1 || true
    sleep 5
  done
  pkill -f 'DiscordCanary\.exe' >/dev/null 2>&1 || true
  pkill -f 'Discord\.exe' >/dev/null 2>&1 || true
  wineserver -k >/dev/null 2>&1 || true
  if [ -z "$node" ]; then
    local appdir fetched
    appdir="$(dirname "$(dirname "$glob")")"
    fetched="$(fetch_module_distro canary "$appdir" "$OUTDIR" || true)"
    [ -n "$fetched" ] && node="$fetched"
  fi
  if [ -z "$node" ]; then
    echo "error: discord_voice.node not found under $WINEPREFIX_DIR/drive_c/users/*/AppData/Local/discordcanary/app-*" >&2
    exit 1
  fi
  catalog canary "$node"
}

extract_audiocord() {
  if [ ! -f "$AUDIOCORD_EXE" ]; then
    echo "error: $AUDIOCORD_EXE not found" >&2
    exit 1
  fi
  local tmp=/tmp/audiocord node=""
  rm -rf "$tmp"
  mkdir -p "$tmp"
  echo "extracting $AUDIOCORD_EXE..."
  7z x -y "$AUDIOCORD_EXE" -o"$tmp" >/dev/null
  local p7="$tmp/\$PLUGINSDIR/app-64.7z"
  if [ ! -f "$p7" ]; then
    echo "error: \$PLUGINSDIR/app-64.7z not found inside $AUDIOCORD_EXE" >&2
    exit 1
  fi
  7z x -y "$p7" -o"$tmp/app" >/dev/null
  node="$(find "$tmp/app" -path '*/modules/discord_voice-1/discord_voice/discord_voice.node' | head -1 || true)"
  if [ -z "$node" ]; then
    echo "error: AudioCord does not ship or use discord_voice.node - it runs voice" >&2
    echo "through Chromium WebRTC instead (see resources/app.asar main.js), so there" >&2
    echo "is nothing to byte-patch here. Use 'discord' (official client builds, which" >&2
    echo "modded clients like Vencord/Equicord also use)." >&2
    exit 1
  fi
  catalog audiocord "$node"
}

case "$TARGET" in
discord) extract_discord ;;
audiocord) extract_audiocord ;;
canary) extract_canary ;;
both)
  extract_discord
  extract_audiocord
  ;;
esac
