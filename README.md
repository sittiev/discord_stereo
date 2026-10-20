<p align="center">
  <img src="src-tauri/icons/StereoWizardIcon.png" alt="Stereo Wizard" width="140">
</p>

<p align="center">
  <a href="https://github.com/sittiev/discord_stereo/actions/workflows/ci.yml">
    <img src="https://img.shields.io/github/actions/workflow/status/sittiev/discord_stereo/ci.yml?label=CI" alt="CI">
  </a>
  <a href="https://github.com/sittiev/discord_stereo/actions/workflows/release.yml">
    <img src="https://img.shields.io/github/actions/workflow/status/sittiev/discord_stereo/release.yml?label=Release" alt="Release">
  </a>
  <a href="https://v2.tauri.app/">
    <img src="https://img.shields.io/badge/built%20with-Tauri%202-24C4D8" alt="Tauri 2">
  </a>
  <img src="https://img.shields.io/badge/platforms-Windows%20%2F%20Linux%20%2F%20macOS-red" alt="Platforms">
</p>

<h1 align="center">Stereo Wizard</h1>

<p align="center">
  A desktop wizard that patches <code>discord_voice.node</code> (the Opus/WebRTC voice binary
  inside the Discord client) so a mono microphone stream plays as real stereo at locked
  bitrate. Tauri 2 + Rust, vanilla TypeScript on <code>@m3e/web</code> (Material 3 Expressive).
  Read-only by design: it writes only when you press apply or revert.
</p>

<p align="center">
  <a href="#what-it-does">What it does</a> •
  <a href="#client-support">Client support</a> •
  <a href="#checking-your-version">Version</a> •
  <a href="#goals">Goals</a> •
  <a href="#patch-db">Patch-DB</a> •
  <a href="#download">Download</a> •
  <a href="#build">Build</a> •
  <a href="#what-is-verified">What is verified</a> •
  <a href="#limits">Limits</a>
</p>

## What it does

Point it at your Discord install. Every action below is explicit:

- Detects Discord clients on disk (stable, PTB, canary, Vencord) on Windows, Linux and macOS.
- Analyzes `discord_voice.node` and reports each patch site as pristine, already applied, or
  diverged — including the bytes it actually found.
- Applies a goal with verify-then-write: every site must match its expected bytes before a
  single byte changes; a `.bak-<goal>` copy of the original is kept for one-click revert.
- Refuses to touch the binary while that Discord client is running (checked in `/proc` on
  Linux; on other platforms the byte verification is the safety layer).

## Client support

The engine patches any client whose `discord_voice.node` still matches a Patch-DB table.
Branch channels on the same OS ship the same voice binary; other platforms compile their
own node, so they need their own tables. Current coverage:

<p align="left">
  <img src="https://img.shields.io/badge/Discord%20Stable-tested%20on%20Linux-success?logo=discord" alt="Discord Stable">
  <img src="https://img.shields.io/badge/Discord%20PTB-untested-orange?logo=discord" alt="Discord PTB">
  <img src="https://img.shields.io/badge/Discord%20Canary-untested-orange?logo=discord" alt="Discord Canary">
  <img src="https://img.shields.io/badge/Vencord-untested%20but%20likely-orange?logo=vencord" alt="Vencord">
</p>

| Client | Windows | Linux | macOS |
|---|---|---|---|
| Discord Stable | needs own tables | **tested** | needs own tables |
| Discord PTB | needs own tables | untested* | needs own tables |
| Discord Canary | needs own tables | untested* | needs own tables |
| Vencord | needs own tables | untested — same binary as stable, likely works | needs own tables |

- **Tested** = analyzed, applied and reverted with every byte check passing against the real
  client.
- \* Cross-channel cells should behave like the tested row — the voice binary family is the
  same; re-verify before relying on it.
- The wizard is the referee: when a table covers your build, Apply lights up; otherwise it
  analyzes only and never writes.

## Checking your version

- **First, the wizard**: the Info step prints the install paths it found plus the MD5 of the
  original voice binary; tables are keyed by MD5, not by the version label.
- **Windows**: look inside `%LOCALAPPDATA%\Discord\` — the `app-1.0.xXXX` folder name is the
  client version (`DiscordPTB\` / `DiscordCanary\` for the other channels).
- **Linux (deb/rpm)**: `dpkg -s discord | grep -i version` or `rpm -q discord`.
- **macOS**: right-click `/Applications/Discord.app` → Get Info → Version.
- **Vencord** is a client mod on a regular install — it has no version of its own; read the
  underlying client's.
- Current tables were cut against **`app-1.0.161`, linux x86_64** (`discord_voice.node`
  MD5 `0bd81054ee44228a03928e84044a4d32`).

## Goals

Each goal is a byte-site table matched to a Discord binary generation:

| Goal | What it does |
|---|---|
| v3 — Filters Out | Stereo core without krisp/highpass/downmix; server bitrate (~64k + boost). |
| v4 — Stereo Full (recommended) | v3 + 510 kbps CBR lock + force-stereo + RemixAndResample fix. Best centroid in the bench. |
| v5 — Deep Stereo | v4 + Opus kAudio: more depth and separation, lower spectral centroid (music-mode trade-off). |

## Patch-DB

The offsets live in [`patchdb/tables.json`](patchdb/tables.json), keyed by incremental
`db_version`, so old builds keep working after Discord updates:

- The running binary embeds the current table as an offline bootstrap.
- At boot it fetches the newest table from this repo (`main`), capped at 2 MiB with a 15 s
  timeout, and adopts it only when `db_version` is greater — then caches it to the app data
  dir. Dev override: `STEREO_DB_URL`.
- A binary matched by MD5 gets surgical offsets. Unknown binaries fall back to tables with
  AOB patterns (`??` wildcards) that require exactly one match and an `expect` check.
- A binary with no resolvable table is only ever analyzed, never written.
- The schema is validated on every push in CI (`patchdb/validate.mjs`).

## Download

Installers come from [Releases](https://github.com/sittiev/discord_stereo/releases):

| OS | Assets |
|---|---|
| Linux | `.AppImage`, `.deb`, `.rpm` |
| macOS | `.dmg` (universal: Intel + Apple Silicon) |
| Windows | `.exe` (NSIS), `.msi`, and a portable `.zip` with the bare exe |

<details>
<summary><b>Building from source</b></summary>

Requirements: Node 22, pnpm 11, Rust stable. On Linux, Tauri needs the GTK/WebKit dev
packages:

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev libxdo-dev libssl-dev
```

```bash
git clone https://github.com/sittiev/discord_stereo.git
cd discord_stereo
pnpm install --frozen-lockfile
pnpm tauri dev          # full desktop app
cd src-tauri && cargo check --locked && cargo test
```

</details>

## What is verified

| Part | Status |
|---|---|
| Offset tables v3/v4/v5 | Validated against `app-1.0.161` linux x86_64 `discord_voice.node`, MD5 `0bd81054ee44228a03928e84044a4d32` |
| Pattern scanner (AOB + `expect`) | Unit-tested in Rust; refuses 0 or 2+ matches |
| Apply/revert cycle | Unit-tested: verify-then-write, idempotent apply, backup restore |
| CI gate + release builds | Run on GitHub Actions for all three OSes |
| Windows/macOS node builds | No tables for those binaries yet; pattern-only tables are the planned path |

## Limits

> [!IMPORTANT]
> Builds are unsigned. This project is unofficial and not affiliated with Discord Inc. — it
> modifies local app files at your own risk and never automates Discord.

- The offset tables pin one Linux build of `discord_voice.node`. Other client versions get
  analyzed (never patched) until a matching table or a unique AOB pattern ships to the
  Patch-DB.
- The CI has no word on how patched audio behaves in closed Discord builds. The bench that
  picked the flags is a local measurement setup, not a Discord endorsement.
- The app interface is in English.
