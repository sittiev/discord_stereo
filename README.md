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

<p align="center">
  <a href="https://discord.gg/jb72VayAzN" title="Entrar no servidor do Discord">
    <img src="https://invidget.switchblade.xyz/jb72VayAzN" width="430" alt="Discord Server">
  </a>
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
  <a href="#how-patches-work">How patches work</a> •
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

## How patches work

> [!TIP]
> **Offsets and sites.** An *offset* is where a byte lives inside the
> `discord_voice.node` file. A *site* is one small change at that place, such as forcing
> stereo or locking the bitrate. A *goal* (v3–v6) groups sites, and a *table* is that group
> for one exact Discord build. The app checks every byte before writing, keeps a backup, and
> stops if the file doesn't match. Builds change often, so check which build the Info step
> shows.

## Client support

The engine patches any client whose `discord_voice.node` still matches a Patch-DB table.
Branch channels on the same OS ship the same voice binary; other platforms compile their
own node, so they need their own tables. Current coverage:

<p align="left">
  <img src="https://img.shields.io/badge/Discord%20Stable-tested%20on%20Windows%20native%2FWine%20%2B%20Linux-success?logo=discord" alt="Discord Stable">
  <img src="https://img.shields.io/badge/Discord%20PTB-untested-orange?logo=discord" alt="Discord PTB">
  <img src="https://img.shields.io/badge/Discord%20Canary-untested-orange?logo=discord" alt="Discord Canary">
  <img src="https://img.shields.io/badge/Vencord-same%20binary%20as%20Stable-orange?logo=vencord" alt="Vencord">
</p>

| Client | Windows | Linux | macOS |
|---|---|---|---|
| Discord Stable | **tested** (native + Wine) | **tested** | no tables |
| Discord PTB | untested* | untested* | no tables |
| Discord Canary | untested* | untested* | no tables |
| Vencord | same official Stable binary; expected to work | same as Stable | no tables |

- **Tested** = analyzed, applied and reverted with every byte check passing and a
  remote-listener audio pass on a current build. Builds rotate; the Info step
  shows which build the wizard found, and **Windows** covers both native
  Windows and Wine.
- \* Cross-channel cells should behave like the stable row — the voice binary family is the
  same; re-verify before relying on it.
- The wizard is the referee: when a table covers your build, Apply lights up; otherwise it
  analyzes only and never writes.

## Checking your version

Open the wizard's Info step. It shows the Discord install path and which build it found.
You do not need to look up the version manually. If the build has no verified table, the app
analyzes it and never writes.

## Goals

Each goal is a byte-site table matched to a Discord binary generation. v3–v5
improve filters, quality, and bitrate; only v6 gives true L/R separation.

| Goal | What it does |
|---|---|
| v3 — Filters Removed | Removes filters and improves quality; no true L/R separation like v6. |
| v4 — Full Quality | v3 + 510 kbps CBR lock + force-stereo + RemixAndResample fix. Improves quality; no true L/R separation like v6. |
| v5 — Deep Quality | v4 + Opus kAudio: more depth and quality; no true L/R separation like v6. |
| v6 — True Stereo 1:1 (recommended) | The only true 1:1 L/R profile: SDP force-stereo, 510 kbps CBR, Opus frame copy/gain, CELT; high/low-pass bypassed. Windows Stable. |
| v7 — 8ch Surround (experimental) | Forces 8 channels at 208 kbps CBR. Windows Stable only; audio untested and may break voice. Revert if silent. |

## Patch-DB

The patch tables live in [`patchdb/tables.json`](patchdb/tables.json). The app embeds the
current table, so it works offline, and downloads a newer one when available.

- A known build gets exact offsets. An unknown build gets a warned compatibility attempt,
  and the app writes only when every site verifies.
- A build without a verified table is analyzed and never written.
- CI validates the table on every change.

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
| v6 true stereo | Stereo path tested on Windows native + Wine; the 510 kbps change is static-apply verified (audio re-test pending) |
| v7 8ch surround | Static apply/revert only; audio untested; experimental and may break voice |
| v3/v4/v5 legacy tables | Tested on Linux and Windows legacy builds; current tables in the Patch-DB |
| Cross-build compatibility | Unknown builds get a warned compatibility attempt; apply only after every site verifies |
| Patch matching | Requires a unique match; refuses 0 or 2+ matches |
| Apply/revert cycle | Unit-tested: verify-then-write, idempotent apply, backup restore |
| CI gate + release builds | Run on GitHub Actions for all three OSes |
| macOS node builds | No tables for those binaries yet; analyze-only |

## Limits

> [!IMPORTANT]
> Builds are unsigned. This project is unofficial and not affiliated with Discord Inc. — it
> modifies local app files at your own risk and never automates Discord.

- The offset tables pin specific `discord_voice.node` builds. Other client versions get a
  warned compatibility attempt: the app resolves sites through the cross-build profile bank
  and only writes when every site verifies. If any site is unresolved, the build is analyzed
  and never patched.
- Windows Stable is tested on **native Windows and Wine**; Linux legacy
  tables are tested. Builds rotate, so the Info step identifies the build
  before you apply.
- v7 is experimental: it forces 8 channels at 208 kbps and is not audio-tested.
  It can leave the call silent or unstable; revert if there is no audio.
- The v6 Windows tables now use 510 kbps CBR. Their stereo bytes are unchanged,
  but the bitrate change is static-apply verified; re-test audio after updating.
- macOS has no byte tables yet and is analyze-only. PTB and Canary have no v6
  table yet.
- On native Windows the app still relies on byte verification; the running-
  Discord guard is Linux-only. Close Discord before apply/revert.
- The CI has no word on how patched audio behaves in closed Discord builds. The bench that
  picked the flags is a local measurement setup, not a Discord endorsement.
- The app interface is in Portuguese (pt-BR).
