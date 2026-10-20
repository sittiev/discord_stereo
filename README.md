# Stereo Wizard

[![CI](https://github.com/sittiev/discord_stereo/actions/workflows/ci.yml/badge.svg)](https://github.com/sittiev/discord_stereo/actions/workflows/ci.yml)
[![Release](https://github.com/sittiev/discord_stereo/actions/workflows/release.yml/badge.svg)](https://github.com/sittiev/discord_stereo/actions/workflows/release.yml)

Wizard desktop que patchea o `discord_voice.node` do Discord para melhorar a qualidade do áudio do microfone (codec Opus / WebRTC). Rust + Tauri 2, interface TypeScript com Material 3 Expressive.

## O que ele faz

- Detecta instalações do Discord no disco (stable, PTB, canary, Vencord).
- Analisa o binário de voz e mostra o estado de cada site de patch (ok / aplicado / divergente).
- Aplica e reverte com verificação byte a byte antes de escrever e backup automático.
- Recusa escrever com o Discord aberto.

Read-only por design: só toca no binário quando você clica em aplicar ou reverter.

## Goals

Cada goal é um conjunto de patches para uma família de versão do Discord:

| Goal | O que faz |
| --- | --- |
| v3 — Filtros Removido | Stereo core sem krisp/highpass/downmix; bitrate do servidor (~64k + boost). |
| v4 — Stereo Full (recomendado) | v3 + 510 kbps CBR travado + force-stereo + fix de RemixAndResample. Melhor centroid no bench. |
| v5 — Stereo Profundo | v4 + Opus kAudio: mais profundidade e cantos; trade-off do modo música. |

## Patch-DB: à prova de update do Discord

Os offsets vivem em `patchdb/tables.json`, com `db_version` incremental:

- O app embute a tabela do momento do build como bootstrap offline.
- Ao iniciar, baixa a versão mais recente do `main` deste repo e cacheia no app data.
- Binário desconhecido é só analisado, nunca gravado. Tabela por md5 é cirúrgica; tabela pattern-only resolve por assinatura AOB com match único e `expect`.

Versão nova do Discord quebrou os offsets? O processo do mantenedor está em [`dev/patchdb-HOWTO.md`](dev/patchdb-HOWTO.md).

## Download

Instaladores em [Releases](https://github.com/sittiev/discord_stereo/releases):

| SO | Assets |
| --- | --- |
| Linux | AppImage, deb, rpm |
| macOS | dmg universal (Intel + Apple Silicon) |
| Windows | setup.exe (NSIS), msi e zip portátil (exe solto) |

Builds unsigned: no macOS, clique direito > Abrir; no Windows, o SmartScreen pode avisar. O portátil exige WebView2 Runtime, presente por padrão no Windows 10/11.

## Compilar do zero

Requisitos: Node 22, pnpm 11, Rust stable. No Linux, as libs que o Tauri pede:

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev
```

```bash
pnpm install --frozen-lockfile
pnpm tauri dev          # app desktop completo
pnpm build              # gate do frontend (tsc + vite)
cd src-tauri && cargo check --locked
```

## Estrutura

- `patchdb/tables.json` — fonte única das tabelas, validada no CI (`dev/validate-patchdb.mjs`).
- `src-tauri/src/patch_engine.rs` — verify-then-write, backup, revert.
- `dev/mock-ipc.mjs` — harness de browser com fixtures de IPC.
- `dev/plans/` — planos de trabalho.

## Aviso

Projeto não-oficial, sem vínculo com o Discord Inc. Modifica arquivos locais do app; use por sua conta e risco.
