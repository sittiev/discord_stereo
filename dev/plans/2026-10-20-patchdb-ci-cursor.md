# Stereo Wizard — Patch-DB (A) + Pattern-scan (B), CI/CD e política de cursor

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tornar os patches sobreviventes a updates do Discord (Patch-DB remota + fallback por assinatura de bytes), equipar o repo com auto-release GitHub Actions, e limpar o cursor para "mão só no que abre link".

**Architecture:** As tabelas de sites saem do código Rust e viram `patchdb/tables.json` (fonte única, embutida no binário via `include_str!` e atualizável por download). O engine passa a resolver sites por `(goal, md5)` com fallback para tabelas pattern-only (AOB), mantendo verify-then-write idempotente com backup e conferência de bytes originais. CI/CD redige a app na tag `v*` (3 SO) e empacota snapshot do JSON como asset de release. Cursor: override externo das `:host` rules do m3e.

**Tech Stack:** Rust edition 2024 + Tauri 2.0, vanilla TS, `@m3e/web` 2.10.1, `ureq` (fetch), GitHub Actions (`tauri-apps/tauri-action@v0`), Node 22 + pnpm 11, Vite 8.

**Decisões fechadas no grill (contexto para o executor):**

1. **A e B em camadas:** Patch-DB (A) é o caminho primário; pattern-scan (B) é a segunda camada dentro do mesmo schema. Tabelas cirúrgicas têm `binary_md5` obrigatório; tabelas pattern-only podem viver sem md5 e atravessam builds menores do Discord enquanto as assinaturas continuam únicas e batendo com `expect`.
2. **Entrega:** `patchdb/tables.json` no `main` é a fonte viva (push = usuários atualizados em segundos). Workflow anexa o mesmo JSON como asset da release `patchdb` (arquivo/rollback). O app lê a raw do `main`; trocar para ler a release futura = mudar 1 constante.
3. **Confiança:** HTTPS + verify-then-write nativo. Sem assinatura (usuário escolheu "nenhum"). **Risco residual documentado:** quem conseguir commitar no repo pode publicar tabela deliberadamente errada; a defesa que permanece é a conferência de `orig`/`expect` byte a byte + md5 unique-match — writes nunca acontecem em bytes que não batem.
4. **Goals data-driven:** `tables.json` dono de id/título/descrição dos goals; Rust mantém o bootstrap embutido para primeira execução offline.
5. **CI/CD:** `release.yml` full-matrix (AppImage+deb+rpm, dmg universal, NSIS+msi), `ci.yml` gate (pnpm build + cargo check --locked + validador do JSON), sem assinatura de código por enquanto (macOS right-click-open / Windows SmartScreen, nota para o Info do app em task posterior).
6. **Cursor:** `pointer` só em elementos que abrem link externo (`[data-url]`); tudo mais arrow; `cursor: text` em `.path` fica (affordance de seleção, não é hover); `src/styles.css` é lixo de template Vite e some.
7. **Escopo fora (dito no grill):** updater de binário do app (tauri-plugin-updater) fica para fase futura, fora deste plano.
8. **Plano salvo em** `dev/plans/` por escolha explícita do usuário (normal aqui do projeto).

---

## Estrutura de arquivos (mapa antes de mexer)

| Arquivo | Ação | Responsabilidade |
|---|---|---|
| `.gitignore` | criar | lixo de build fora do git |
| `src/styles.css` | deletar | template Vite sem referência |
| `index.html` | editar | política de cursor + nó `#dbinfo` + hint `#db-stale` |
| `patchdb/tables.json` | criar | fonte única de tabelas (schema v1) |
| `dev/patchdb-HOWTO.md` | criar | processo manual do mantenedor para novas versões do Discord |
| `dev/validate-patchdb.mjs` | criar | validador sem dependências rodado no CI |
| `.github/workflows/ci.yml` | criar | gate de PR/push |
| `.github/workflows/release.yml` | criar | auto-release na tag `v*` |
| `.github/workflows/patchdb-archive.yml` | criar | snapshot do JSON como asset |
| `src-tauri/src/tables.rs` | criar | schema + cache + merge + URL |
| `src-tauri/src/patch_engine.rs` | reescrever | resolve sites do DB (+ scan de pattern) |
| `src-tauri/src/lib.rs` | editar | comandos `patch_db_info`, `check_db_update`, setup |
| `src-tauri/Cargo.toml` | editar | dependência `ureq` |
| `src/main.ts` | editar | card de DB no Info, check no boot, hint em Aplicar |
| `dev/mock-ipc.mjs` | editar | fixtures dos 2 comandos novos |
| `AGENTS.md` | editar | paths/commands novos |

Ordem das tasks: cursor (ganho rápido independente) → dados do patchdb → CI → engine → lib → frontend → archive → docs. Cada task termina em commit (mensagens Conventional ≤50 chars).

---

### Task 0: git init (pré-requisito)

**Files:**
- Create: `.gitignore`

A pasta hoje **não é** repositório git (remoto alvo: `github.com/sittiev/discord_stereo`). Sem isso, Tasks de commit falham.

- [ ] **Step 1: init + .gitignore**

```bash
cd /home/rafael/Projetos/stereo-wizard
git init -b main
cat > .gitignore <<'EOF'
node_modules/
dist/
src-tauri/target/
src-tauri/gen/schemas/
.omp/
EOF
```

- [ ] **Step 2: primeiro commit**

```bash
git add -A && git commit -m "chore: baseline do stereo-wizard"
```

 Esperado: commit criado. (push/remote fica com o usuário; CI não precisa hoje.)

> Se `git rev-parse --is-inside-work-tree` já responder `true`, pule esta task inteira.

---

### Task 1: política de cursor (pointer só no que abre link)

**Files:**
- Modify: `index.html` (bloco `<style>`)
- Delete: `src/styles.css`

Motivação técnica: `cursor: pointer` de botões/cards/chips/segmentos/stepper/switch/expansão vem de **`:host` rules dentro do shadow DOM do m3e** (ex.: `button.js: :host(:not(:disabled)...) { cursor: pointer }`). Regras no documento externo para o mesmo elemento **vencem** `:host` na cascata — então um bloco externo no `index.html` resolve sem tocar no pacote.

- [ ] **Step 1: confirmar que styles.css é morto**

```bash
grep -rn "styles.css" src index.html vite.config.ts src-tauri 2>/dev/null
```

Esperado: **nenhuma linha** (nenhum import). Se aparecer, pare e reavalie.

- [ ] **Step 2: remover o pointer dos cards + inserir o bloco de política**

No `<style>` do `index.html`, **apague** este bloco:

```css
      m3e-card.selectable {
        cursor: pointer;
      }
```

E **adicione** num lugar próximo (fica visível de propósito, é política do app):

```css
      /* Política de cursor: mão só no que abre link. As rules :host do m3e
         (pointer em botão/card/chip etc.) perdem para estas do documento. */
      m3e-button,
      m3e-icon-button,
      m3e-card,
      m3e-chip,
      m3e-expansion-panel,
      m3e-step,
      m3e-button-segment,
      m3e-segmented-button,
      m3e-switch,
      m3e-list-item {
        cursor: default;
      }
      [data-url] {
        cursor: pointer;
      }
```

Não mexa em `.path { cursor: text }` (fica, é affordance de seleção, não é hover). Estados visuais de hover (relevo/cor) não mudam — só cursor.

- [ ] **Step 3: deletar styles.css**

```bash
rm src/styles.css
```

- [ ] **Step 4: verificar**

```bash
pnpm build
```

Esperado: `tsc && vite build` compilam sem erro.

- [ ] **Step 5: smoke visual no harness (opcional, barato)**

```bash
pnpm dev &        # porta 1420
# abrir http://127.0.0.1:1420 com mock injetado por Playwright
```

Esperado: cursor arrow sobre cards/botões/switch/stepper/segmentos; mão no GitHub/Discord/Doar; texto I-beam nos paths de instalação.

- [ ] **Step 6: commit**

```bash
git add -A && git commit -m "feat(ui): cursor padronizado — mao so no link"
```

---

### Task 2: `patchdb/tables.json` (fonte única) + HOWTO

**Files:**
- Create: `patchdb/tables.json`
- Create: `dev/patchdb-HOWTO.md`

A conversão aqui **move** as tabelas hoje escritas em Rust para JSON; a Task 5 apaga as funções `v3_sites/v4_sites/v5_sites` do Rust e passa a ler este arquivo (embutido via `include_str!`).Offsets ficam como **string hex** (`"0x003CB8D0"`) — sem erro de conversão, legível na re-derivação de offsets.

- [ ] **Step 1: criar `patchdb/tables.json` (db_version 1, conteúdo atual do engine)**

```json
{
  "schema": 1,
  "db_version": 1,
  "updated": "2026-10-20",
  "goals": [
    {
      "id": "v3",
      "title": "Filtros Removido",
      "description": "Stereo core sem krisp/highpass/downmix; bitrate do servidor (~64k+boost)."
    },
    {
      "id": "v4",
      "title": "Stereo Full",
      "description": "Recomendado: v3 + 510 kbps CBR travado + force-stereo + RemixAndResample fix. Melhor CENTROID no bench."
    },
    {
      "id": "v5",
      "title": "Stereo Profundo",
      "description": "v4 + Opus kAudio: mais profundidade e cantos; centroide espectral mais baixo (trade-off do modo música)."
    }
  ],
  "tables": [
    {
      "goal": "v3",
      "binary_md5": "0bd81054ee44228a03928e84044a4d32",
      "discord": "app-1.0.161",
      "os": "linux",
      "arch": "x86_64",
      "sites": [
        { "kind": "offset", "offset": "0x003CB8D0", "orig": "02", "patch": "00", "name": "StereoCheck1 CommitAudioCodec" },
        { "kind": "offset", "offset": "0x003C76D5", "orig": "02", "patch": "00", "name": "StereoCheck2 CreateAudioStream" },
        { "kind": "offset", "offset": "0x003BF2E0", "orig": "4939d44c0f43e2", "patch": "49c7c402000000", "name": "CreateAudioFrame clamp1->2" },
        { "kind": "offset", "offset": "0x003BF2EF", "orig": "4939d44c0f43e2", "patch": "49c7c402000000", "name": "CreateAudioFrame clamp2->2" },
        { "kind": "offset", "offset": "0x0038C075", "orig": "84c0740d83bb30020000090f8f", "patch": "909090909090909090909090e9", "name": "MonoDownmix bypass (NOP+JMP)" },
        { "kind": "offset", "offset": "0x0099F7DD", "orig": "01", "patch": "02", "name": "MCOpusImpl ctor num_channels" },
        { "kind": "offset", "offset": "0x009E3629", "orig": "01", "patch": "02", "name": "SdpToConfig num_channels" },
        { "kind": "offset", "offset": "0x009CCF80", "orig": "55", "patch": "c3", "name": "downmix_and_resample ret" },
        { "kind": "offset", "offset": "0x00696220", "orig": "55", "patch": "c3", "name": "HighPassFilter::Process ret" },
        { "kind": "offset", "offset": "0x009E3190", "orig": "554889e58b0f31c0", "patch": "48c7c001000000c3", "name": "OpusConfig IsOk true" },
        { "kind": "offset", "offset": "0x00376240", "orig": "55", "patch": "c3", "name": "NoiseCanceller::Process ret" },
        { "kind": "offset", "offset": "0x0038CC20", "orig": "0fb687f80100002401c3", "patch": "31c0c390909090909090", "name": "ShouldRunNoiseCancellation false" }
      ]
    },
    {
      "goal": "v4",
      "binary_md5": "0bd81054ee44228a03928e84044a4d32",
      "discord": "app-1.0.161",
      "os": "linux",
      "arch": "x86_64",
      "sites": [
        { "kind": "offset", "offset": "0x003CB8D0", "orig": "02", "patch": "00", "name": "StereoCheck1 CommitAudioCodec" },
        { "kind": "offset", "offset": "0x003C76D5", "orig": "02", "patch": "00", "name": "StereoCheck2 CreateAudioStream" },
        { "kind": "offset", "offset": "0x003BF2E0", "orig": "4939d44c0f43e2", "patch": "49c7c402000000", "name": "CreateAudioFrame clamp1->2" },
        { "kind": "offset", "offset": "0x003BF2EF", "orig": "4939d44c0f43e2", "patch": "49c7c402000000", "name": "CreateAudioFrame clamp2->2" },
        { "kind": "offset", "offset": "0x0038C075", "orig": "84c0740d83bb30020000090f8f", "patch": "909090909090909090909090e9", "name": "MonoDownmix bypass (NOP+JMP)" },
        { "kind": "offset", "offset": "0x0099F7DD", "orig": "01", "patch": "02", "name": "MCOpusImpl ctor num_channels" },
        { "kind": "offset", "offset": "0x009E3629", "orig": "01", "patch": "02", "name": "SdpToConfig num_channels" },
        { "kind": "offset", "offset": "0x009CCF80", "orig": "55", "patch": "c3", "name": "downmix_and_resample ret" },
        { "kind": "offset", "offset": "0x00696220", "orig": "55", "patch": "c3", "name": "HighPassFilter::Process ret" },
        { "kind": "offset", "offset": "0x009E3190", "orig": "554889e58b0f31c0", "patch": "48c7c001000000c3", "name": "OpusConfig IsOk true" },
        { "kind": "offset", "offset": "0x00376240", "orig": "55", "patch": "c3", "name": "NoiseCanceller::Process ret" },
        { "kind": "offset", "offset": "0x0038CC20", "orig": "0fb687f80100002401c3", "patch": "31c0c390909090909090", "name": "ShouldRunNoiseCancellation false" },
        { "kind": "offset", "offset": "0x0099F7E7", "orig": "007d0000", "patch": "30c80700", "name": "MCOpusImpl ctor bitrate 510k" },
        { "kind": "offset", "offset": "0x009E3633", "orig": "007d0000", "patch": "30c80700", "name": "SdpToConfig bitrate 510k" },
        { "kind": "offset", "offset": "0x006A2135", "orig": "554889e589f2", "patch": "55ba30c80700", "name": "WebRtcOpus_SetBitRate lock 510k" },
        { "kind": "offset", "offset": "0x006A2525", "orig": "4883fa02772e", "patch": "ba0200000090", "name": "SetForceChannels->2" },
        { "kind": "offset", "offset": "0x006A23E0", "orig": "48", "patch": "c3", "name": "DisableCbr->ret" },
        { "kind": "offset", "offset": "0x00666CE3", "orig": "765c", "patch": "eb5c", "name": "RemixAndResample skip downmix" }
      ]
    },
    {
      "goal": "v5",
      "binary_md5": "0bd81054ee44228a03928e84044a4d32",
      "discord": "app-1.0.161",
      "os": "linux",
      "arch": "x86_64",
      "sites": [
        { "kind": "offset", "offset": "0x003CB8D0", "orig": "02", "patch": "00", "name": "StereoCheck1 CommitAudioCodec" },
        { "kind": "offset", "offset": "0x003C76D5", "orig": "02", "patch": "00", "name": "StereoCheck2 CreateAudioStream" },
        { "kind": "offset", "offset": "0x003BF2E0", "orig": "4939d44c0f43e2", "patch": "49c7c402000000", "name": "CreateAudioFrame clamp1->2" },
        { "kind": "offset", "offset": "0x003BF2EF", "orig": "4939d44c0f43e2", "patch": "49c7c402000000", "name": "CreateAudioFrame clamp2->2" },
        { "kind": "offset", "offset": "0x0038C075", "orig": "84c0740d83bb30020000090f8f", "patch": "909090909090909090909090e9", "name": "MonoDownmix bypass (NOP+JMP)" },
        { "kind": "offset", "offset": "0x0099F7DD", "orig": "01", "patch": "02", "name": "MCOpusImpl ctor num_channels" },
        { "kind": "offset", "offset": "0x009E3629", "orig": "01", "patch": "02", "name": "SdpToConfig num_channels" },
        { "kind": "offset", "offset": "0x009CCF80", "orig": "55", "patch": "c3", "name": "downmix_and_resample ret" },
        { "kind": "offset", "offset": "0x00696220", "orig": "55", "patch": "c3", "name": "HighPassFilter::Process ret" },
        { "kind": "offset", "offset": "0x009E3190", "orig": "554889e58b0f31c0", "patch": "48c7c001000000c3", "name": "OpusConfig IsOk true" },
        { "kind": "offset", "offset": "0x00376240", "orig": "55", "patch": "c3", "name": "NoiseCanceller::Process ret" },
        { "kind": "offset", "offset": "0x0038CC20", "orig": "0fb687f80100002401c3", "patch": "31c0c390909090909090", "name": "ShouldRunNoiseCancellation false" },
        { "kind": "offset", "offset": "0x0099F7E7", "orig": "007d0000", "patch": "30c80700", "name": "MCOpusImpl ctor bitrate 510k" },
        { "kind": "offset", "offset": "0x009E3633", "orig": "007d0000", "patch": "30c80700", "name": "SdpToConfig bitrate 510k" },
        { "kind": "offset", "offset": "0x006A2135", "orig": "554889e589f2", "patch": "55ba30c80700", "name": "WebRtcOpus_SetBitRate lock 510k" },
        { "kind": "offset", "offset": "0x006A2525", "orig": "4883fa02772e", "patch": "ba0200000090", "name": "SetForceChannels->2" },
        { "kind": "offset", "offset": "0x006A23E0", "orig": "48", "patch": "c3", "name": "DisableCbr->ret" },
        { "kind": "offset", "offset": "0x00666CE3", "orig": "765c", "patch": "eb5c", "name": "RemixAndResample skip downmix" },
        { "kind": "offset", "offset": "0x009E3682", "orig": "00", "patch": "01", "name": "OpusConfig application kAudio" }
      ]
    }
  ]
}
```

- [ ] **Step 2: criar `dev/patchdb-HOWTO.md` (processo do mantenedor)**

```markdown
# Patch-DB: como atualizar tabelas quando o Discord atualiza

As tabelas moram em `patchdb/tables.json`. O app embute o arquivo do commit
bootstrap e baixa a raw do `main` como atualização; um workflow anexa o mesmo
JSON como asset de release para (roll back)/(arquivo).

## Schema (v1)

- `schema`: versão do schema, hoje `1`.
- `db_version`: contadone monotônico do DB; o app só adota remoto com version
  maior que o local. A cada edição relevante, **suba o `db_version`**.
- `updated`: data legível (YYYY-MM-DD).
- `goals`: id/título/descrição exibidos na etapa Patchs. Novo goal
  (ex.: "v6") = criar entrada aqui — sem build novo do app.
- `tables[]`:
  - `goal`: id do goal (tem que existir em `goals`).
  - `binary_md5`: md5 do `discord_voice.node` alvo. **Obrigatório** se a
    tabela tiver sites `offset`. Tabelas só de pattern podem omitir.
  - `discord`: version dir, ex. `app-1.0.161` (informativo; use o nome real).
  - `os`, `arch`: informativo.
  - `sites[]`:
    - `kind: "offset"`: cirúrgico. `offset` hex (`0x...`), `orig` e `patch`
      hex de MESMO comprimento. Só válido em tabela com `binary_md5`.
    - `kind: "pattern"`: scan AOB. `pattern` hex com `??` como wildcard,
      `patch` do mesmo comprimento do span do pattern, `expect` (opcional,
      mas recomendado) = bytes atuais esperados no match. Regras do engine:
      exatamente 1 match; `expect` (se presente) tem que bater; qualquer
      dúvida, recusa e nada é escrito.

## Fluxo para versão nova do Discord

1. Pegue o `discord_voice.node` novo (instale a versão nova do cliente).
2. Calcule o md5: `md5sum discord_voice.node`.
3. Derive os offsets (Ghidra/IDA, igual hoje) e escreva as tabelas [v3/v4/v5]
   correspondentes com `binary_md5` do passo 2.
4. Baixe o `db_version` e a `updated`.
5. `node dev/validate-patchdb.mjs` — precisa passar.
6. Commit + push. O CI valida; o asset de release se atualiza sozinho.

## Quando usar pattern em vez de offset

- Função pode mudar de endereço entre builds próximos → escreva `pattern`
  uma vez, sem offset, e (opcionalmente sem `binary_md5`) a tabela
  atravessa builds enquanto a assinatura é única e `expect` bate.
- Offset exato na mão de uma tabela md5 específica → `offset` é mais
  barato e cirúrgico.
```

- [ ] **Step 3: commit**

```bash
git add patchdb dev/patchdb-HOWTO.md
git commit -m "feat(patchdb): tabela inicial v1 + howto"
```

---

### Task 3: validador + CI gate + auto-release

**Files:**
- Create: `dev/validate-patchdb.mjs`
- Create: `.github/workflows/ci.yml`
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: `dev/validate-patchdb.mjs`** (Node puro, zero deps; CI trava JSON ruim antes de chegar no app)

```js
#!/usr/bin/env node
// Valida patchdb/tables.json (schema v1) sem dependências. Uso:
//   node dev/validate-patchdb.mjs [caminho]
import { readFileSync } from "node:fs";
import { argv, exit } from "node:process";

const file = argv[2] ?? "patchdb/tables.json";
const fail = (m) => { console.error(`${file}: ${m}`); exit(1); };

const HEX = (s) => /^[0-9a-fA-F]+$/.test(s) && s.length % 2 === 0;
const HEX_OR_STAR = (s) => {
  if (!/^([0-9a-fA-F]{2}|\?\?)(\s*([0-9a-fA-F]{2}|\?\?))*$/.test(s)) return false;
  return s.replace(/\s/g, "").length % 2 === 0;
};

let db;
try { db = JSON.parse(readFileSync(file, "utf8")); } catch (e) { fail(`JSON inválido: ${e.message}`); }

if (db.schema !== 1) fail(`schema desconhecido: ${db.schema}`);
if (!Number.isInteger(db.db_version)) fail("db_version inteiro");
if (typeof db.updated !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(db.updated)) fail("updated YYYY-MM-DD");

const goalIds = new Set((db.goals ?? []).map((g) => g.id));
if (goalIds.size !== db.goals.length) fail("goal duplicado em goals");
for (const g of db.goals) {
  if (typeof g.id !== "string" || !g.id) fail("goal.id obrigatório");
  if (typeof g.title !== "string" || !g.title) fail(`goal ${g.id}.title obrigatório`);
  if (typeof g.description !== "string" || !g.description) fail(`goal ${g.id}.description obrigatório`);
}

const seenPairs = new Set();
const tables = db.tables ?? [];
if (tables.length === 0) fail("nenhuma tabela");
const span = (p) => p.replace(/\s/g, "").length / 2;

for (const t of tables) {
  if (!goalIds.has(t.goal)) fail(`tabela com goal desconhecido: ${t.goal}`);
  const hasOffsetSites = t.sites.some((s) => s.kind === "offset");
  if (hasOffsetSites && typeof t.binary_md5 !== "string") {
    fail(`tabela ${t.goal} (updated ${t.discord}) com sites offset exige binary_md5`);
  }
  if (t.binary_md5 !== undefined && !/^[0-9a-f]{32}$/.test(t.binary_md5)) {
    fail(`table ${t.goal}/${t.discord}: binary_md5 deve ser md5 hex minúsculo de 32`);
  }
  const pair = `${t.goal}:${t.binary_md5 ?? "*"}:${t.discord ?? "*"}`;
  if (seenPairs.has(pair)) fail(`tabela duplicada: ${pair}`);
  seenPairs.add(pair);
  for (const s of t.sites) {
    if (s.kind !== "offset" && s.kind !== "pattern") fail(`site com kind inválido: ${s.kind} (${s.name})`);
    if (typeof s.name !== "string" || !s.name) fail(sitePattern("name obrigatório"));
    if (s.kind === "offset") {
      if (!/^0x[0-9a-f]{1,8}$/.test(s.offset)) fail(kindOffset(`${s.name}: offset hex 0x...`));
      if (!HEX(s.orig) || !HEX(s.patch)) fail(stateMsg(`${s.name}: orig/patch hex par`));
      if (s.orig.length !== s.patch.length) fail(`${s.name}: orig e patch de mesmo comprimento`);
    } else {
      if (!HEX_OR_STAR(s.pattern)) fail(`${s.name}: pattern hex com ??`);
      if (!HEX(s.patch)) fail(`${s.name}: patch hex par`);
      if (span(s.pattern) !== s.patch.length / 2) fail(`${s.name}: patch deve cobrir o span inteiro do pattern`);
      if (s.expect !== undefined) {
        if (!HEX(s.expect)) fail(`${s.name}: expect hex par`);
        if (s.expect.length !== s.patch.length) fail(`${s.name}: expect mesmo comprimento do patch`);
      }
    }
  }
}

console.log(`patchdb ok: ${db.goals.length} goals, ${tables.length} tabelas, db_version ${db.db_version}`);
```

> **Correção de montagem:** as chamadas `fail(kindOffset(...))` e `fail(sitePattern(...))`/`stateMsg(...)` acima são erradas por construção — o correto é sempre `fail("<mensagem>")` com template literal direto. Ao escrever o arquivo, use `fail(\`site ${s.name}: ...\`)` em todos esses pontos (schema: strings diretas, sem helper). A mensagem que define é a do template literal, não o helper.

- [ ] **Step 2: `.github/workflows/ci.yml`**

```yaml
name: CI
on:
  push:
    branches: [main]
  pull_request:

jobs:
  gate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 11
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: pnpm
      - run: pnpm install --frozen-lockfile
      - run: pnpm build
      - run: node dev/validate-patchdb.mjs
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
      - run: cargo check --locked
        working-directory: src-tauri
```

- [ ] **Step 3: `.github/workflows/release.yml`** (trigger: tag `v*`; dmg universal; sem assinatura)

```yaml
name: Release
on:
  push:
    tags: ["v*"]

jobs:
  build:
    permissions:
      contents: write
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
          - os: windows-latest
          - os: macos-latest
            args: "--target universal-apple-darwin"
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 11
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: pnpm
      - run: pnpm install --frozen-lockfile
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: "aarch64-apple-darwin,x86_64-apple-darwin"
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
      - name: Dependências do Linux
        if: runner.os == 'Linux'
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev build-essential curl wget file patchelf
      - name: Dependências do Windows
        if: runner.os == 'Windows'
        run: cargo install --locked --force cargo-wix || PowerShell -c Write-Host "sem cargo-wix; segue msi padrão que o tauri gera"
      - uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          tagName: v__VERSION__
          releaseName: "Stereo Wizard v__VERSION__"
          releaseBody: "Assets: Linux AppImage/deb/rpm, macOS dmg (universal, sem notarizar), Windows NSIS/msi (sem assinar)."
          releaseDraft: false
          prerelease: false
          args: ${{ matrix.args }}
```

Nota `--locked`: o `cargo check --locked` no CI gate já barrava drift do lockfile; no release, os runner São fresh e os lockfiles são commitados, então lock é honrado por default. O flag anterior era redundância.

- [ ] **Step 4: rodar o validador localmente**

```bash
node dev/validate-patchdb.mjs
```

Esperado: `patchdb ok: 3 goals, 3 tabelas, db_version 1`

- [ ] **Step 5: commit**

```bash
git add dev/validate-patchdb.mjs .github
git commit -m "ci: gate de build e auto-release na tag v*"
```

---

### Task 4: `src-tauri/src/tables.rs` — schema, bootstrap, cache, merge

**Files:**
- Create: `src-tauri/src/tables.rs`

Fonte única: o mesmo `patchdb/tables.json` do repo é embutido no binário por `include_str!`. **Sem duplicação.** Caminho relativo a `src-tauri/src/tables.rs`: `../../patchdb/tables.json`.

- [ ] **Step 1: criar `src-tauri/src/tables.rs`**

```rust
//! Patch-DB: schema do tables.json, bootstrap embutido, cache e update.
//!
//! Fonte única: `patchdb/tables.json` na raiz do repo. O binário embute
//! este arquivo (`include_str!`) como bootstrap offline; o download do
//! raw do main entra como cache no app_data e ganha do bootstrap se
//! `db_version` for maior.

use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

pub const BUILTIN_DB_PATH: &str = "../../patchdb/tables.json";

/// Fonte viva. Override de dev: `STEREO_DB_URL` aponta para um JSON local
/// (harness `python3 -m http.server`), senão usa o main do repo.
pub const REMOTE_DB: &str =
    "https://raw.githubusercontent.com/sittiev/discord_stereo/main/patchdb/tables.json";

pub const DB_URL_FALLBACK_NOTE: &str =
    "trocar para asset de release = mudar REMOTE_DB";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchDb {
    pub schema: u32,
    pub db_version: u32,
    pub updated: String,
    pub goals: Vec<GoalMeta>,
    pub tables: Vec<Table>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalMeta {
    pub id: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    /// Bite-site table do goal; `binary_md5` Some() é cirúrgica (sólida).
    pub goal: String,
    /// md5 do binário alvo. Obrigatório em tabela com sites `offset`;
    /// tabela pattern-only pode omitir (None) e atravessa builds menores.
    pub binary_md5: Option<String>,
    /// Version dir informativo, ex.: "app-1.0.161".
    pub discord: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub sites: Vec<TableSite>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TableSite {
    /// Site cirúrgico: offset fixo no binário apontado por binary_md5.
    Offset {
        offset: String,
        orig: String,
        patch: String,
        name: String,
    },
    /// Scan por assinatura (AOB), `??` = wildcard. Único match exigido;
    /// `expect` confere os bytes correntes antes de escrever.
    Pattern {
        name: String,
        pattern: String,
        patch: String,
        #[serde(default)]
        expect: Option<String>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("patchdb: `{0}`")]
    Json(#[from] serde_json::Error),
    #[error("patchdb: campo inválido em {where_}: {why}")]
    Field { where_: String, why: String },
}

pub fn parse_db(json: &str) -> Result<PatchDb, DbError> {
    let db: PatchDb = serde_json::from_str(json)?;
    let ids: std::collections::HashSet<&str> =
        db.goals.iter().map(|g| g.id.as_str()).collect();
    if ids.len() != db.goals.len() {
        return Err(DbError::Field { where_: "goals".into(), why: "id duplicado".into() });
    }
    for t in &db.tables {
        if !ids.contains(t.goal.as_str()) {
            return Err(DbError::Field { where_: "tables".into(), why: format!("goal desconhecido {}", t.goal) });
        }
        if t.sites.iter().any(|s| matches!(s, TableSite::Offset { .. })) && t.binary_md5.is_none() {
            return Err(DbError::Field {
                where_: format!("table {}", t.goal),
                why: "site offset sem binary_md5".into(),
            });
        }
    }
    Ok(db)
}

// ---- estado global (janela única, uma DB por processo) ----------------
static DB: OnceLock<RwLock<PatchDb>> = OnceLock::new();
static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

fn cell() -> &'static RwLock<PatchDb> {
    DB.get_or_init(|| RwLock::new(parse_db_str_or_panic()))
}

/// Bootstrap: parse do embutido; parse do cache vence se db_version maior.
/// Erros de parse do cache são silenciais — volta pro embutido.
pub fn init(cache_dir: PathBuf) -> Result<(), DbError> {
    let _ = cell();
    let cache = cache_dir.join("patchdb.json");
    if let Ok(json) = std::fs::read_to_string(&cache) {
        if let Ok(newer) = parse_db(&json) {
            if newer.db_version >= cell().read().expect("db lock").db_version {
                *cell().write().expect("db lock") = newer;
            }
        }
    }
    let _ = CACHE_DIR.set(cache_dir);
    Ok(())
}

fn parse_db_str_or_panic() -> PatchDb {
    // O arquivo embutido é validado no CI (Tarefa 3) e no teste da Tarefa 4.
    parse_db(include_str!("../../patchdb/tables.json")).unwrap_or_else(|e| panic!("patchdb embutido inválido: {e}"))
}

/// Snapshot legível para comandos do UI.
pub fn snapshot() -> PatchDb {
    cell().read().expect("db lock").clone()
}

/// Adoca um db baixado: valida, salva cache (tmp + rename) e troca o global.
pub fn adopt_remote(json: &str) -> Result<bool, DbError> {
    let new_db = parse_db(json)?;
    let handle = cell();
    let current = handle.read().expect("db lock").db_version;
    if new_db.db_version > current {
        *handle.write().expect("db lock") = new_db;
        write_cache(json)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn write_cache(json: &str) -> Result<(), DbError> {
    let Some(dir) = CACHE_DIR.get() else { return Ok(()) };
    let tmp = dir.join("patchdb.json.tmp");
    let dst = dir.join("patchdb.json");
    std::fs::write(&tmp, json).map_err(|e| DbError::Field { where_: "cache".into(), why: e.to_string() })?;
    std::fs::rename(&tmp, &dst).map_err(|e| DbError::Field { where_: "cache".into(), why: e.to_string() })
}
```

- [ ] **Step 2: testes de parse/merge (módulo `#[cfg(test)]` dentro de tables.rs)** — checked no fim da Tarefa 5 depois que o cargo teste roda. Não confie no `again copy-paste` do passo 2 acima: o código final precisa incluir:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_parse_ok() {
        let db = parse_db(include_str!("../../patchdb/tables.json")).expect("embutido válido");
        assert!(db.goals.iter().any(|g| g.id == "v4"));
        assert_eq!(db.schema, 1);
    }

    #[test]
    fn offsets_sites_sem_md5_são_recusados() {
        let bad = r#"{"schema":1,"db_version":2,"updated":"2026-01-01","goals":[{"id":"v3","title":"a","description":"b"}],"tables":[{"goal":"v3","sites":[{"kind":"offset","offset":"0x0","orig":"02","patch":"00","name":"x"}]}]}"#;
        assert!(parse_db(bad).is_err());
    }

    #[test]
    fn pattern_site_sem_md5_aceito() {
        let ok = r#"{"schema":1,"db_version":2,"updated":"2026-01-01","goals":[{"id":"v3","title":"a","description":"b"}],"tables":[{"goal":"v3","sites":[{"kind":"pattern","name":"x","pattern":"55 ??","patch":"c3 c3","expect":"55 48"}]}]}"#;
        assert!(parse_db(ok).is_ok());
    }
}
```

> Nota: nomes de testes em português com underline estão ok no rust (identificadores). Ajuste para `offset_sites_without_md5_rejected` se aparecer aviso de estilo nos lints de CI (não tem hoje).

---

### Task 5: `patch_engine.rs` — resolve por DB + scan de pattern (camada B)

**Files:**
- Modify: `src-tauri/src/patch_engine.rs` (substituição do embutido por consulta ao DB)

Refactor nesta ordem para o teste de TDD na frente: a função de scan é pura e testa sem runtime Tauri.

- [ ] **Step 1: substituir módulo pelas peças abaixo (arquivo inteiro)**

```rust
//! Stereo patch engine: resops por Patch-DB, verify-then-write, backup, revert.
//!
//! Sites vêm de `tables::PatchDb` (Tarefas 4): tabela por `(goal, md5)`
//! cirúrgica (offset) ou pattern-only (AOB). Binário sem tabelas
//! correspondentes nunca é gravado — `analyze` reporta `mismatch`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::tables::{PatchDb, TableSite};

/// Site resolvido (pattern e offset terminam igual).
#[derive(Debug, Clone)]
pub struct Resolved {
    pub name: String,
    pub offset: u32,
    pub orig: Vec<u8>,
    pub patch: Vec<u8>,
}

/// Per-site analysis outcome.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum SiteState {
    /// Bytes batem com `orig` — patch-eável.
    Ok,
    /// Bytes batem com `patch` — já aplicado.
    Patched,
    /// Divergiu do esperado — inclui "0 matches"/"n matches" de pattern.
    Mismatch { found: String },
}

/// Full node analysis para um goal.
#[derive(Debug, Serialize)]
pub struct Analysis {
    pub path: PathBuf,
    pub size: u64,
    pub md5: String,
    /// true = existe tabela (md5-keyed ou pattern) resolvível para este binário.
    pub databases_avail: bool,
    pub sites: Vec<(String, SiteState)>,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("hex decode em {site}: {source}")]
    Hex {
        site: String,
        source: hex::FromHexError,
    },
    #[error("validação falhou em {n} site(s); nada foi escrito")]
    Validation { n: usize },
    #[error("nenhum backup encontrado para reverter")]
    NoBackup,
    #[error("nenhuma tabela de patch para esse binário (md5 {md5}, goal {goal}) — rode Verificar atualizações")]
    NoTable { md5: String, goal: String },
}

fn decode_hex(s: &str, site_name: &str) -> Result<Vec<u8>, EngineError> {
    hex::decode(s).map_err(|source| EngineError::Hex {
        site: site_name.to_owned(),
        source,
    })
}

fn parse_offset(hex: &str) -> Result<u32, EngineError> {
    let t = hex.trim();
    let body = t.strip_prefix("0x").unwrap_or(t);
    u32::from_str_radix(body, 16)
        .map_err(|e| EngineError::Hex { site: hex.into(), source: hex::FromHexError::InvalidStringLength })
        .map(|_| u32::from_str_radix(body, 16).expect("recheck ok")) // estilo; simplificar abaixo
}
```

> **Correção do Step acima:** `parse_offset` acima está convoluto. Versão correta e simples a usar no arquivo final:

```rust
fn parse_offset(hex: &str) -> Result<u32, EngineError> {
    let t = hex.trim();
    let body = t.strip_prefix("0x").unwrap_or(t);
    u32::from_str_radix(body, 16).map_err(|_| EngineError::Hex {
        site: format!("offset {hex}"),
        source: hex::FromHexError::InvalidHexCharacter { c: '?', index: 0 },
    })
}
```

Continuação obrigatória do patch_engine.rs:

```rust
/// Scan AOB: retorna offset do único match, ou None se 0/2+ matches.
/// Padrão: bytes hex separados por espaço, `??` = wildcard.
fn scan_pattern(bytes: &[u8], pattern: &str) -> Option<usize> {
    if pattern.len() % 3 != 0 && !pattern.isEmpty() { /* utils morrem aqui — ver nota */ }
}
```

> **Reescrevendo:** `scan_pattern` final e testado (usar ESTE, não o rascunho):

```rust
/// Scan AOB: retorna o offset (em bytes) do ÚNICO match, ou None.
/// Tokens hex separados por espaço; `??` = wildcard (1 byte).
fn scan_pattern(bytes: &[u8], pattern: &str) -> Option<usize> {
    let pat: Vec<u8> = Vec::new(); // construído abaixo
    let mask: Vec<bool> = Vec::new(); // true = byte exato; false = wildcard
    for token in pattern.split_whitespace() {
        if token == "??" {
            pat.push(0);
            mask.push(false);
        } else {
            match hex::decode(token) {
                Ok(b) if b.len() == 1 => {
                    pat.push(b[0]);
                    mask.push(true);
                }
                _ => return None, // token inválido — tabla não resolvível
            }
        }
    }
    if pat.is_empty() || bytes.len() < pat.len() {
        return None;
    }
    let mut found: Option<usize> = None;
    for i in 0..=(bytes.len() - pat.len()) {
        let hit = pat.Principal_if.zip(mask).all(|(&p, &m)| p == bytes[i] | !m);
        // Correção: ver versão final abaixo
    }
    found
}
```

> **Versão FINAL do scan_pattern** (correta, sem rascunho — é esta que entra no arquivo):

```rust
/// Scan AOB: retorna o offset (em bytes) do ÚNICO match, ou None.
/// Tokens hex separados por espaço; `??` = wildcard (1 byte).
fn scan_pattern(bytes: &[u8], pattern: &str) -> Option<usize> {
    let mut pat: Vec<u8> = Vec::new();
    let mut mask: Vec<bool> = Vec::new();
    for token in pattern.split_whitespace() {
        if token == "??" {
            pat.push(0);
            mask.push(false);
        } else {
            match hex::decode(token) {
                Ok(b) if b.len() == 1 => {
                    pat.push(b[0]);
                    mask.push(true);
                }
                _ => return None,
            }
        }
    }
    if pat.len() == 0 || pat.len() > bytes.len() {
        return None;
    }
    let mut found: Option<usize> = None;
    'outer: for i in 0..=(bytes.len() - pat.len()) {
        for (j, (&p, &m)) in pat.iter().zip(mask.iter()).enumerate() {
            if m && bytes[i + j] != p {
                continue 'outer;
            }
        }
        if found.is_some() {
            return None; // 2+ matches: recusa
        }
        found = Some(i);
    }
    found
}
```

- [ ] **Step 2: resolução de tabela e analyze novos**

```rust
/// Encontra a tabela para (goal, md5): md5-keyed primeiro; senão pattern-only.
/// None = sem tabela. sites são todos resolvidos (pattern scan na frente);
/// pattern sem unique match se torna Resolved com orig vazio e contagem no found.
pub fn resolve_table<'a>(
    db: &PatchDb,
    goal: &str,
    md5: &str,
    bytes: &[u8],
) -> Result<Vec<Resolved>, EngineError> {
    let needle = |t: &crate::tables::Table| t.goal == goal;
    let md5_t = db.tables.iter().find(|t| needle(t) && t.binary_md5.as_deref() == Some(md5));
    let table = match md5_t {
        Some(t) => t,
        None => db.tables.iter().find(|t| needle(t) && t.binary_md5.is_none()),
    };
    let Some(table) = table else { return Err(EngineError::NoTable { md5: md5.into(), goal: goal.into() }) };
    resolve_sites(table, goal, md5, bytes)
}

pub fn resolve_sites(
    table: &crate::tables::Table,
    goal: &str,
    md5: &str,
    bytes: &[u8],
) -> Result<Vec<Resolved>, EngineError> {
    let mut out = Vec::with_capacity(table.sites.len());
    for s in &table.sites {
        match s {
            TableSite::Offset { offset, orig, patch, name } => {
                out.push(Resolved {
                    name: name.clone(),
                    offset: parse_offset(offset)?,
                    orig: decode_hex(orig, name)?,
                    patch: decode_hex(patch, name)?,
                });
            }
            TableSite::Pattern { name, pattern, patch, expect } => {
                let Some(off) = scan_pattern(bytes, pattern) else {
                    return Err(EngineError::Validation {
                        n: 0, // sinaliza "pattern não achou/multi"; refined abaixo
                    });
                };
                let orig = if let Some(e) = expect {
                    decode_hex(e, name).map_err(ErrIntoHex(name))? // só bate se o span atual == expect
                } else {
                    bytes[off..off + patch.len() / 2].to_vec()
                };
                out.push(Resolved { name: name.clone(), offset: off as u32, orig, patch: decode_hex(patch, name)? });
            }
        }
    }
    Ok(out)
}
```

> **Correções a aplicar no arquivo final:**
> 1. O pattern sem unique match **não** vira `Validation { n: 0 }` — o analyze precisa mostrar "0 matches"/"n matches" por site e o apply precisar abortar. Implementação: `resolve_sites` acima cobra iterando `table.sites` retornando para cada site `(Resolved, Result<(), String>)`? Esta guia substitui por uma forma simplificada, e com testes cobertos. Se TDD deste função preferir retornar `Vec<(Resolved, Option<String>)>` (site resolvido + mismatch reason), o analyze mapeia para `SiteState::Mismatch` e o apply aborta com `Validation { n: mismatches }`. **Use esta forma.**
> 2. `ErrIntoHex` não existe — a única fonte de `EngineError::Hex` aqui é `decode_hex`.
> 3. Resolva todos os sites e convença-se: `mismatches.iter().filter(|(reason, _)| reason.isSome()).count()`.

- [ ] **Step 3: analyze/apply/revert finais**

```rust
pub fn analyze(node: &Path, goal: &str, db: &PatchDb) -> Result<Analysis, EngineError> {
    let bytes = fs::read(node)?;
    analyze_bytes(node, &bytes, goal, db)
}

pub fn analyze_bytes(node: &Path, bytes: &[u8], goal: &str, db: &PatchDb) -> Result<Analysis, EngineError> {
    let md5 = md5_hex(bytes);
    let sites: Vec<(String, SiteState)> = match try_table(db, goal, &md5, bytes) {
        Some((sites_resolved, pattern_only)) => {
            let _ = pattern_only;
            sites_resolved
                .iter()
                .map(|r| (r.name.clone(), site_state(&r, bytes)))
                .collect()
        }
        None => Vec::new(),
    };
    let databases_avail: bool = tables_here(db, goal, &md5);
    Ok(Analysis {
        path: node.to_path_buf(),
        size: bytes.len() as u64,
        md5: md5.clone(),
        databases_avail,
        sites,
    })
}
```

Helpers disponíveis no arquivo final:

```rust
/// tabela md5-keyed presente OU pattern-only hospedada para o goal.
fn tables_here(db: &PatchDb, goal: &str, md5: &str) -> bool { /* presente toda */ }

fn site_state(resolved: &Resolved, bytes: &[u8]) -> SiteState {
    let end = resolved.offset as usize + resolved.patch.len();
    match end_byte_match(bytes, resolved) {
        Current::OrigOk => SiteState::Ok,
        Current::PatchedAlready => SiteState::Patched,
        Current::Found { found } => SiteState::Mismatch { found },
    }
}
```

> Estes helpers (`tables_here`, `end_byte_match`, `Current`) definem por si mesmos ao implementar; o contrato é: `Ok` = bytes == `resolved.orig`; `Patched` = bytes == `resolved.patch`; `Mismatch { found }` = hex dos bytes correntes, mensagens "formato X.First one clear.

```rust
pub fn apply(node: &Path, goal: &str, db: &PatchDb) -> Result<Analysis, EngineError> {
    let mut bytes = fs::read(node)?;
    let analysis = analyze_bytes(node, &bytes, goal, db)?;
    let mismatched = analysis.sites.iter().filter(|(_, st)| matches!(st, SiteState::Mismatch { .. })).count();
    if analysis.sites.is_empty() {
        return Err(EngineError::NoTable { md5: analysis.md5.clone(), goal: goal.into() });
    }
    if mismatched > 0 {
        return Err(EngineError::Validation { n: mismatched });
    }
    let resolved = resolve_table(db, goal, &analysis.md5, &bytes)?;
    let mut todo = Vec::new();
    for r in resolved {
        let start = r.offset as usize;
        let end = start + r.patch.len();
        if bytes.get(start..end).expect("bounds do analyze") != r.patch.as_slice() {
            todo.push(r);
        }
    }
    if todo.is_empty() {
        return Ok(analysis);
    }
    let backup = backup_path(node, goal);
    if !backup.exists() {
        fs::copy(node, &backup)?;
    }
    for r in &todo {
        let off = r.offset as usize;
        bytes[off..off + r.patch.len()].copy_from_slice(&r.patch);
    }
    fs::write(node, &bytes)?;
    let final_state = analyze_bytes(node, &bytes, goal, db)?;
    debug_assert_eq!(
        final_state.sites.iter().filter(|(_, st)| matches!(st, SiteState::Ok)).count(),
        0,
        "apply left unpatched sites behind; verify-then-write invariant broken"
    );
    Ok(final_state)
}
```

`revert` e `backup_path` não muda (goal é String já? SIM: `backup_path(node, goal)` recebe `&str`).

```rust
fn backup_path(node: &Path, goal: &str) -> PathBuf {
    let mut p = node.as_os_str().to_os_string();
    p.push(format!(".bak-{}", goal));
    PathBuf::from(p)
}

/// md5 de um slice de bytes, hex minúsculo.
#[must_use]
pub fn md5_hex(bytes: &[u8]) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(bytes);
    let sum = hasher.finalize();
    hex::encode(sum)
}
```

- [ ] **Step 4: TDD antes do arquivo final — testes no início do processo**

Testes a escrever no `#[cfg(test)]` de `patch_engine.rs` (mix: scan puro + analyze em synthetic):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::parse_db;

    fn buf() -> Vec<u8> {
        // slot @ 0x00 orig "02" patch "00" já da tabela; bytearray sintético
        let mut b = vec![0u8; 64];
        b[0] = 0x02;
        b
    }

    const DB_JSON_MIN: &str = r#"{
        "schema":1,"db_version":1,"updated":"2026-10-20",
        "goals":[{"id":"t","title":"T","description":"d"}],
        "tables":[
          {"goal":"t","binary_md5":"__MD5__","sites":[
            {"kind":"offset","offset":"0x00","orig":"02","patch":"00","name":"slot0"}
          ]},
          {"goal":"t","sites":[
            {"kind":"pattern","name":"sig","pattern":"AA 00 ?? FF","patch":"AB 00 CD FF","expect":"AA 00 99 FF"}
          ],"discord":"app-x","os":"linux","arch":"x86_64"}
        ]
    }"#;

    #[test]
    fn md5_table_aplica_e_reporta_patched() {
        let mut b = buf();
        let md5 = md5_hex(&b);
        let json = DB_JSON_MIN.replace("__MD5__", &md5);
        let db = parse_db(&json).expect("parse db");
        // pré: bytes @0 == "02" → Ok
        let a = analyze_bytes(Path::new("n"), &b, "t", &db).expect("ancluir");
        assert!(a.databases_avail);
        assert!(a.sites.iter().any(|(n, s)| n == "slot0" && matches!(s, SiteState::Ok)));
        // pattern table depende da md5 table primeiro — sem pattern test aqui.
    }

    #[test]
    fn pattern_osc_nao_tr(double_hash) { /* ver nota */ }
}
```

> **Nota de correção:** os testes acima foram rascunhados com nomes quebrados. Formulário final dos testes obrigatórios para esta task (todos no arquivo; comportamento para cada um descrito, código final à época):

1. `offset_table_applies_and_reports_patched` — buffer 64 bytes, slot0 = `02`; analyze → `Ok`; subplots `bytes[0]` = `00`; analyze → `Patched`; apply → sem erro, `sites[0]` = Patched; revert em tmp file funciona (backup criado).
2. `offset_sem_md5_hash_prospects` — tabela com binary_md5 ____ != do buffer: analyze sites vazio e databases_avail false quando nem pattern table existir; no apply → `NoTable`.
3. `pattern_unique_resolves` — bytes `AA 00 99 FF` em bytes[10..14] únicos! signature "AA ?? 99"? Use o DB_JSON_MIN acima com pattern `AA 00 ?? FF` expect `AA 00 99 FF`; analyze → a site "sig" Ok; apply → bytes viram `AB 00 CD FF` e analyze volta `Patched`.
4. `pattern_multi_match_rejects` — mesma assinatura repetida 2× no buffer: analyze → site "sig" `Mismatch` found contém "matches"; apply → erro Validation.
5. `pattern_no_match_while_md5_table_s故居` — nenhum match: analyze → Mismatch; apply → Validation.
6. `backup_path_suffixed_with_goal` — `.bak-t` formado.

> Codificar os 6 testes ANTES do refactor, rodar `cargo test` (deve falhar: 'sites_for' not found — pointing to old API), escrever engine novo, re-rodar até green. Este é o loop TDD desta task.

- [ ] **Step 5: run + commit**

```bash
cd src-tauri && cargo test && cd ..
git add -A && git commit -m "feat(engine): sites do patchdb + scan de assinatura"
```

Esperado: todos os testes novos verdes; `cargo check --locked` limpo.

---

### Task 6: `lib.rs` — comandos `patch_db_info`/`check_db_update` + setup

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml` (adiciona `ureq`)

- [ ] **Step 1: dependência**

```bash
cd src-tauri && cargo add ureq@2 && cd ..
```

`Cargo.lock` regenera pelo cargo. Ele é commitado no fim da task.

- [ ] **Step 2: `lib.rs` edits**

```rust
mod detect;
mod patch_engine;
mod tables; // novo

use std::path::{Path, PathBuf};
use tauri::Manager;

use detect::{FoundClient, scan_all};
use patch_engine::{Analysis, analyze, apply, revert};
use serde::Serialize;

/// Scan read-only...
#[tauri::command]
fn detect_clients() -> Vec<FoundClient> {
    scan_all()
}

/// Metadata do UI (goals data-driven do DB).
#[tauri::command]
fn goals() -> Vec<GoalInfo> {
    let db = tables::snapshot();
    db.goals
        .iter()
        .map(|g| GoalInfo {
            id: g.id.clone(),
            title: g.title.clone(),
            description: g.description.clone(),
            sites:-site_count(&db, &g.id),
        })
        .collect()
}

/// Contagem para o chip "N patchs": último site count da última table do goal.
fn site_count(db: &tables::PatchDb, goal_id: &str) -> usize {
    db.tables.iter().filter(|t| t.goal == goal_id).map(|t| t.sites.len()).last().unwrap_or(0)
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct GoalInfo { id: String, title: String, description: String, sites: usize }

#[tauri::command]
fn analyze_node(node: String, goal: String) -> Result<Analysis, String> {
    analyze(Path::new(&node), &goal, &tables::snapshot()).map_err(|e| e.to_string())
}

#[tauri::command]
fn apply_goal(node: String, goal: String) -> Result<Analysis, String> {
    let path = PathBuf::from(&node);
    if discord_running() {
        return Err("o Discord está rodando — feche-o antes de aplicar o patch".into());
    }
    apply(&path, &goal, &tables::snapshot()).map_err(|e| e.to_string())
}

#[tauri::command]
fn revert_node(node: String) -> Result<String, String> { /* igual hoje (já usa &str suffix) */ }
```

> Nesta task também: apague `Goal`, `Goal::ALL`, `sites_for` e todo o embutido antigo (`KNOWN_PRISTINE_MD5` sai de `system_info`? — read below).

`system_info` e `known_pristine_md5`: o uso da Info hoje mostra o md5 tabelado. New info card (Tarefa 7) mosta `db_version`/`updated`/`fonte`. Substitua `known_pristine_md5` do `SystemInfo` por nada? O ajuste mais minimal **dado que mock-ipc e Info UI já mostram**: mantenha `known_pristine_md5` mas alimético: `known_pristine_md5: tables::snapshot().tables.first().andThen(|t| t.binary_md5.clone()).unwrap_or_default()`. Assim a UI não quebra antes da Tarefa 7.

```rust
#[tauri::command]
fn system_info() -> SystemInfo {
    let db = tables::snapshot();
    SystemInfo {
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        known_pristine_md5: db.tables.iter().find_map(|t| t.binary_md5.clone()).unwrap_or_default(),
    }
}

/// Info do DB (card no Info).
#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct PatchDbInfo {
    db_version: u32,
    updated: String,
    goals: Vec<GoalInfo>,
    source: String,
}

#[tauri::command]
fn patch_db_info() -> PatchDbInfo {
    let db = tables::snapshot();
    let source = if std::env::var_os("STEREO_DB_URL").is_some() {
        "STEREO_DB_URL (dev)"
    } else {
        "raw main + builtin"
    };
    PatchDbInfo {
        db_version: db.db_version,
        updated: db.updated,
        goals: db.goals.iter().map(|g| GoalInfo {
            id: g.id.clone(),
            title: g.title.clone(),
            description: g.description.clone(),
            sites: site_count(&db, &g.id),
        }).collect(),
        source: source.into(),
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct DbCheck {
    updated: bool,
    info: PatchDbInfo,
}

/// Fetch do raw + merge no global + cache no app_data.
#[tauri::command]
async fn check_db_update(app: tauri::AppHandle) -> Result<DbCheck, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let url = std::env::var("STEREO_DB_URL").unwrap_or_else(|_| tables::REMOTE_DB.into());
        let resp = ureq::get(&url).timeout(std::time::Duration::from_secs(15)).call().map_err(|e| format!("baixando tables.json: {e}"))?;
        let mut body = String::new();
        resp.into_reader()
            .take(2usize.pow(21)) // 2 MB cap — arquivo é ~30KB hoje
            .read_to_string(&mut body)
            .map_err(|e| format!("lendo tables.json: {e}"))?;
        let updated = tables::adopt_remote(&body).map_err(|e| e.to_string())?;
        let db = tables::snapshot();
        let _ = &app; // AppHandle não precisa hoje; path já resolvido no setup
        Ok(DbCheck { updated, info: build_info(&db, Some(url)) })
    })
    .await
    .map_err(|e| format!("check_db_update: {e}"))?
}
```

Builder setup:

```rust
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("app data dir");
            tables::init(dir).expect("patchdb init");
            Ok(())
        });
    // ... resto igual (feature dev-tools, handler, run)
}
```

invoke_handler ganha `patch_db_info, check_db_update`.

`use std::io::Read;` no topo para o `read_to_string`.

- [ ] **Step 3: verificar**

```bash
cd src-tauri && cargo check --locked && cargo test && cd ..
pnpm build
```

Esperado: tudo verde. (`pnpm build` vai reclamar do TS só na Tarefa 7 — se reclamar desde `goals()` mudança de `&'static str` → String, corrije o `GoalInfo` TS na hora.)

- [ ] **Step 4: commit + locks**

```bash
git add -A
git commit -m "feat(ipc): comandos patch_db_info e check_db_update"
```

---

### Task 7: frontend — card do DB, check no boot, hint de binário desconhecido

**Files:**
- Modify: `src/main.ts`
- Modify: `index.html` (nós `#dbinfo`, `#db-stale`)
- Modify: `dev/mock-ipc.mjs` (fixtures)

Antes de mexer: snapshot de segurança (convenção AGENTS).

- [ ] **Step 0: snapshot**

```bash
mkdir -p "dev/backup/2026-10-20-patchdb-ui" && cp main.ts index.html dev/backup/2026-10-20-patchdb-ui/ && echo "ui: base p/ patchdb" > "dev/backup/2026-10-20-patchdb-ui/NOTA.md"
```

- [ ] **Step 1: tipos + state (`src/main.ts`)**

```ts
interface PatchDbInfo {
  db_version: number;
  updated: string;
  goals: Array<{ id: string; title: string; description: string; sites: number }>;
  source: string;
}

interface DbCheck {
  updated: boolean;
  info: PatchDbInfo;
}
```

O `GoalInfo {}` e `Analysis.known_pristine` estão **renomeados**: o Analysis wire novo chama `databases_avail`. insights:
  - Interface Analysis: troca `known_pristine: boolean` por `databases_avail: boolean`.
  - `SystemInfo` continua com `known_pristine_md5` (valor agora de db.tables[0], não quebra).

- [ ] **Step 2: ações**

```ts
async function loadDbInfo(): Promise<void> {
  try {
    info_db = await invoke<PatchDbInfo>("patch_db_info");
    renderDbInfo();
  } catch (err) {
    snack(`Erro ao carregar patchdb: ${String(err)}`);
  }
}

async function doDbCheck(reanalyze = false): Promise<void> {
  try {
    const saida = await invoke<DbCheck>("check_db_update");
    info_db = saida.info;
    renderDbInfo();
    snack(saida.updated
      ? `Patch-DB atualizada para v${saida.info.db_version}`
      : `Patch-DB v${saida.info.db_version} já era a mais recente`);
    if (saida.updated && reanalyze && selectedClient) void doAnalyze();
  } catch (err) {
    snack(`Verificar atualizações falhou: ${String(err)}`);
  }
}

function renderDbInfo(): void {
  const box = document.getElementById("dbinfo")!;
  if (!info_db) return;
  box.innerHTML = `
    <div class="row-between">
      <div class="pref-text">
        <span>Patch-DB: v${info_db.db_version} (${esc(info_db.updated)})</span>
        <span class="muted">Origem: ${esc(info_db.source)}. Sobrevive a updates do Discord.</span>
      </div>
      <m3e-button variant="tonal" id="btn-dbcheck">Verificar atualizações</m3e-button>
    </div>`;
  document.getElementById("btn-dbcheck")!.addEventListener("click", () => void doDbCheck(false));
}
```

State: `let info_db: PatchDbInfo | null = null;` (bloco state). ON_ENTER de `s-info` ganha `void loadDbInfo();`.

Boot check diário: no fim do boot —

```ts
const LAST_DB_CHECK = "sw.dbcheck";
const DAY_MS = 24 * 60 * 60 * 1000;
const last = Number(storeGet(LAST_DB_CHECK) ?? 0);
if (Date.now() - last > 20 * 60 * 60 * 1000) {
  storeSet(LAST_DB_CHECK, String(Date.now()));
  void doDbCheck(false); // silencioso: falha só loga no snackbar
}
```

- [ ] **Step 3: hint de binário desconhecido (perdeu is-unknown tabela)**

Na `doAnalyze`, depois de `analysis = await invoke...`:

```ts
const no_table = analysis.databases_avail === false;
document.getElementById("db-stale")!.hidden = !no_table;
```

No `index.html` (etapa aplicar, perto do `#discord-running`):

```html
<m3e-callout variant="warning" id="db-stale" hidden>
  Versão do Discord sem tabela ainda. Rode "Verificar atualizações" na etapa
  Info; os patchs aparecem assim que a comunidade mapear os offsets novos.
</m3e-callout>
```

Consistir `updateApplyButtons()`: disable apply quando `databases_avail === false`.

`renderAbout`/`doApply`/`doRevert` não mudam.

- [ ] **Step 4: mock fixtures (`dev/mock-ipc.mjs`)**

Adicione em DEMO/WORST/etc. `dbinfo` + handlers:

```js
const mock_info_db = {
  db_version: 1,
  updated: "2026-10-20",
  goals: DEMO_GOALS,
  source: "raw main + builtin",
};

// dedo no HANDLERS:
patch_db_info: () => current.dbinfo ?? mock_info_db,
check_db_update: () => ({ updated: false, info: current.dbinfo ?? mock_info_db }),
```

Para `worst` fixture: `dbinfo: { ...mock_info_db, db_version: 99, updated: "2026-10-21", source: "STEREO_DB_URL (dev)" }`, e `analysis.databases_avail === false` — serve para o hint `#db-stale`.

> Ajuste: o mock atual de `sitesFor(18)` volta a ser tudo `ok`; para `worst`, force `databases_avail: false` para ver o callout db-stale.

- [ ] **Step 5: verificar**

```bash
pnpm build
cd src-tauri && cargo check --locked && cd ..
```

Manual: `pnpm dev` + mock; conferir card da Patch-DB no Info, `?data=worst` mostrando callout, botão "Verificar atualizações" com snack output.

Real fetch test harness (sem app redis begin):

```bash
cd patchdb && python3 -m http.server 8123 &   # serve tables.json
STEREO_DB_URL=http://127.0.0.1:8123/tables.json pnpm tauri dev -- --locked
```

Na UI, clicar "Verificar atualizações" → snack de Patch-DB; fechar e reabrir: cache ativo (app_data/patchdb.json existe, e "fonte" continua raw+cache local).

- [ ] **Step 6: commit**

```bash
git add -A
git commit -m "feat(ui): card patchdb, check no boot e hint binario novo"
```

---

### Task 8: snapshot do patchdb como asset de release

**Files:**
- Create: `.github/workflows/patchdb-archive.yml`

- [ ] **Step 1: workflow**

```yaml
name: PatchDB snapshot
on:
  push:
    branches: [main]
    paths: [patchdb/tables.json]

permissions:
  contents: write

jobs:
  archive:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: cria release patchdb e sobe tables.json (clobber = update)
        if jobs via gh CLI preinstalado
        run: |
          gh release create patchdb --title "Patch DB" --notes "Snapshot automático de cada push em patchdb/tables.json. O app lê o raw do main; este asset é histórico/rollback." || echo "já existe"
          gh release upload patchdb patchdb/tables.json --clobber
        env:
          GH_TOKEN: ${{ github.token }}
```

- [ ] **Step 2: commit**

```bash
git add .github && git commit -m "ci: snapshot do patchdb como asset"
```

---

### Task 9: documentação AGENTS.md + verificação final

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: AGENTS.md edits**

Point de integração mínima (5 mudanças):
1. Stack: linha `tauri-plugin-mcp-bridge` ganha vizinho `+ \`ureq\` — fetch da Patch-DB (stdint sans enable)`? Só o necessário: adiciona em **Stack**: "`ureq` — fetch da Patch-DB (remoto + cache)".
2. **Arquitetura**: "sete comandos" (não seis): `detect_clients`, `goals`, `analyze_node`, `apply_goal`, `revert_node`, `system_info`, **`patch_db_info`**, **`check_db_update`** → atualizar para oito comandos. `patch_engine::{analyze, apply, revert}` recebem `db` snapshot de `tables.rs` (fonte `patchdb/tables.json`, embutida por `include_str!` e atualizada por `check_db_update` no app_data cache).
3. **Segurança do patch (crítico)**: substitui o parágrafo do `KNOWN_PRISTINE_MD5` por: tabelas por `(goal, binary_md5)` opattern-only (AOB com unique-match + expect); DB viva no `main` (raw) e como asset da release `patchdb`; e sem `check_db_update` assinado (residual: commit mal-intencionado é a ameaça existente).
4. **Onde coisas novas vão**: adiciona `patchdb/` instrução: offsets novos vêm aqui via howto (com link para `dev/patchdb-HOWTO.md`), `dev/validate-patchdb.mjs` roda no CI. e workflows em `.github/workflows/`.
5. **Tooling**: mantém lockfiles; adiciona "`STEREO_DB_URL` para dev (override da URL da patchdb); **Release**: push de tag `v*` (dmg universal + installers 3 SO; sem assinatura de código)". Adiciona no porta "Testes": "`cargo test` roda os testes de engine/tables; PostgreSQL testes node script com node dev/validate-patchdb.mjs".

- [ ] **Step 2: loop final de verificação**

```bash
pnpm install --frozen-lockfile
pnpm build
node dev/validate-patchdb.mjs
cd src-tauri && cargo check --locked && cargo test && cd ..
```

Esperado: zero erro em tudo. Harness manual via `pnpm dev` + mock; app real via `STEREO_MCP=1 pnpm tauri dev --features dev-tools` (atarea Aplicar e Info revisado).

- [ ] **Step 3: commit final**

```bash
git add AGENTS.md
git commit -m "docs: patchdb, comandos novos e ci no agents"
```

---

## Critérios de sucesso (global do plano)

1. `pnpm build` limpo; `cargo check --locked` e `cargo test` limpos; `node dev/validate-patchdb.mjs` valida.
2. UI: cursor arrow em todos os interativos; pointer só em `[data-url]`; `.path` I-beam.
3. Motor: binário desconhecido **nunca** é escrito (NoTable/mismatch); binário tabelado patchee via Db; pattern un-match/2+ match recusa.
4. Patch-DB remoto: com `STEREO_DB_URL` apontando para JSON com `db_version` maior, app adota, cacheia e continua funcionando offline.
5. GitHub Actions: release na tag `v*` produz assets de 3 SO; snapshot do JSON na release `patchdb`.
6. `patchdb/tables.json` é single source: embutida (offline) e validada no CI.

## Riscos e anotações de tradeoff (pontos delicados)

- **Sem assinatura**: quem commita errado pode quebrar / enganar — opção de usuário. Upgrade posterior: assinatura ed25519 (uma crate + script) sem mudança no schema.
- **AOB**: sempre com `expect`; sem `expect`, só aumenta risco. Confiança em pattern depende `??` mínimo e uniqueness-check; documente no howto.
- **dmg universal**: precisa dos targets rust `aarch64-apple-darwin` + `x86_64-apple-darwin` — o workflow instala ambos.
- **runner os**: usar `macos-latest` (arm) com universal; `ubuntu-latest`/`windows-latest` padrão; sem `macos-13` separado (universal cobre intel via binary literal? Tauri universal = lipo dos dois binários, sim).
- **Cargo.lock**: regenerou pelo cargo da Tarefa 6 (`cargo add`); commit-o obrigatório.
- `--locked` no workflow de release **não** é setável via tauri-action; a meta-v204 fixado mesmo soo efetivo pelos locks commitados — decidia com AGENTS atualizado.

## Execução

Este plano armazena exatamente 10 tasks em ordem: T0 git, T1 cursor, T2 patchdb data, T3 CI, T4 tables module, T5 engine refactor (TDD), T6 comandos IPC, T7 frontend flow, T8 snapshot workflow, T9 docs. Total estimado: um dia dev; T5 é a mais grossa (90–150 min).
