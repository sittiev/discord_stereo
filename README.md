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
> stops if the file doesn't match. Builds change often, so check the MD5 shown in the Info
> step.

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
  shows your binary MD5, and **Windows** covers both native Windows and Wine.
- \* Cross-channel cells should behave like the stable row — the voice binary family is the
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
- Current **v6** tables cover the Windows Stable builds listed below. 9267
  passed native Windows and Wine audio tests; 9266 passed under Wine; 9265 is
  byte-validated. Re-check after every Discord update — the Info step prints
  your MD5.
- Legacy **v3/v4/v5** tables cover Linux `app-1.0.161`, Windows 9265, Windows
  1281 and older canary builds.

| Build | v6 clean MD5 | v6 patched MD5 |
|---|---|---|
| Windows 9265 | `6d8c7c6005f2c03454f2344e48cfcdc8` | `14387dedf0718e79b5ed43d8cc0e3866` |
| Windows 9266 | `0ebc96a7f0dde80d1866d4cf61eb49cd` | `85de3c99480de4e709e110d35efd7fb6` |
| Windows 9267 | `6f0c4c13d83e94b815211cb5867e8025` | `9b61496febfb2b441ba6b82785429896` |

## Goals

Each goal is a byte-site table matched to a Discord binary generation:

| Goal | What it does |
|---|---|
| v3 — Filters Out | Stereo core without krisp/highpass/downmix; server bitrate (~64k + boost). |
| v4 — Stereo Full (recommended) | v3 + 510 kbps CBR lock + force-stereo + RemixAndResample fix. Best centroid in the bench. |
| v5 — Deep Stereo | v4 + Opus kAudio: more depth and separation, lower spectral centroid (music-mode trade-off). |
| v6 — True Stereo 1:1 | Tested true-stereo set: SDP force-stereo, Opus frame copy/gain, CELT. Windows 9265/9266/9267. |

## Patch-DB

The offsets live in [`patchdb/tables.json`](patchdb/tables.json), keyed by incremental
`db_version`, so old builds keep working after Discord updates:

- The running binary embeds the current table as an offline bootstrap.
- At boot it fetches the newest table from this repo (`main`), capped at 2 MiB with a 15 s
  timeout, and adopts it only when `db_version` is greater — then caches it to the app data
  dir. Dev override: `STEREO_DB_URL`.
- A binary matched by MD5 gets surgical offsets. Unknown binaries fall back to tables with
  AOB patterns (`??` wildcards) that require exactly one match and an `expect` check.
- When no exact table matches, v6 also tries the cross-build profile bank. Every site must
  resolve to its expected original bytes (or already-patched bytes) before the UI offers a
  warned compatibility apply. Any unresolved site keeps the binary read-only.
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
| v6 true stereo | Tested on Windows native + Wine; current tables in the Patch-DB |
| v3/v4/v5 legacy tables | Tested on Linux and Windows legacy builds; current tables in the Patch-DB |
| Cross-build compatibility | v6 profile resolver; Windows 1281 and 9267 fixtures resolve 20/20; warned apply only after every site verifies |
| Pattern scanner (AOB + `expect`) | Unit-tested in Rust; refuses 0 or 2+ matches |
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
  tables are tested on 1.0.161. Builds rotate, so check the Info step for your
  MD5 before relying on a table.
- macOS has no byte tables yet and is analyze-only. PTB and Canary have no v6
  table yet.
- On native Windows the app still relies on byte verification; the running-
  Discord guard is Linux-only. Close Discord before apply/revert.
- The CI has no word on how patched audio behaves in closed Discord builds. The bench that
  picked the flags is a local measurement setup, not a Discord endorsement.
- The app interface is in Portuguese (pt-BR).
