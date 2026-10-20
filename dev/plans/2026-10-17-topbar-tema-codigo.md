# Plano — Topbar alinhado, toggle de tema no rodapé, animações on e passe de código (artigo 7 patterns + ponytail)

Data: 2026-10-17 · Arquivos: `index.html`, `src/main.ts` · Backup: `dev/backup/2026-10-17-topbar-tema-codigo/`

## Diagnóstico (feito)

- Seta medida no app real (WebKitGTK) e no harness (Chromium): **40×40 com glifo 24×24, pixel-idêntica em 480×620 e 1816×958** — não há distorção geométrica.
- Problema real: o `.topbar` ocupa a **largura total** (stepper esticado por 1756px em 1816) enquanto `.pane` é limitada a 520px centrada → a seta fica **encostada na borda (8px) e esmagada no canto**, desconectada do conteúdo. No tamanho em que a janela abre (GNOME restaurou 1816×958) isso fica evidente.
- `main` padda 16px; topbar paddava 8px com gap de 4px → apertado em qualquer tamanho.

## Etapa 1 — Topbar alinhado à coluna de conteúdo

`index.html` (`.topbar`): alinhar à coluna (520 + 16×2 = 552px centrado), gap 4→8, padding 8→16.

```css
.topbar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px 0;
  box-sizing: border-box;
  max-width: 552px;
  margin: 0 auto;
  flex-shrink: 0;
}
```

Verificar: em 1816 a seta fica na borda esquerda da coluna (x≈648); em 480/420 fica em x=16 e o stepper cabe (medir primeiro/último passo).

## Etapa 2 — Toggle de tema no rodapé (padrão: sistema)

- `index.html` rodapé: `m3e-icon-button#btn-theme` (ícone "auto" embutido como default, `aria-label`) antes dos links, empurrado à esquerda com `margin-right: auto`.
- `main.ts`: `type ThemePref`; `THEME_ORDER`, `THEME_ICON`, `THEME_LABEL`; `setThemePref(v)` (única via: valida → `themePref` → `storeSet` → `applyThemePref` → `syncThemeCtl`); `syncThemeCtl()` passa a atualizar **segmentado + botão** (ícone e aria).
- Clique no botão: ciclo auto → claro → escuro → auto. Segmentado do Info continua; ambos sincronizados.

## Etapa 3 — Animações por padrão

- `loadAnimPref()` já entrega padrão ligado (exceto `prefers-reduced-motion`). Nada a mudar no código.
- Limpar `sw.anim`="off" preso no localStorage do app real (sobrou de teste) para o padrão valer.

## Etapa 4 — Passe de código (artigo + ponytail)

1. `esc()`: escapar também `"` e `'` (usado em `title=`, `data-url=`, `aria-label=`; path com aspas quebra o atributo hoje). Trocar DOM-por-chamada por `replace` com mapa.
2. `updateApplyButtons()`: remover `?? true` enganoso e termo redundante; decisão explícita (`analysis === null || hasMismatch || allPatched`).
3. Extrair `selectCard(container, card)` — duplicação exata nas seleções de cliente e de patch (variante + `aria-pressed`).
4. `loadSysInfo()`: rótulos pt-BR (`Sistema`, `Arquitetura`, `MD5 do binário original`); remover `box.style.*` inline (duplica o CSS de `#sysinfo`).

Conferir cada item contra o código real antes de aplicar; pular o que não pagar (ex.: snake_case do IPC é fronteira própria estável — renomear = ruído).

## Etapa 5 — Verificação

1. `pnpm build` limpo.
2. Harness 480×620 + 420×480 + 1816×958: seta na coluna; toggle cicla e persiste (`sw.theme`); segmentado sincroniza; animações default ligadas (storage limpo) e desligadas com `sw.anim=off`; zero erro de console.
3. Build debug com `--features dev-tools` → app real (bridge 9223): medir topbar, ciclar tema, limpar `sw.anim`.
4. Loop de revisão: 2 agentes `general` (padrões+requisitos; caça-bugs de interação) → aplicar achados → re-revisar até limpo.
5. Release final: `pnpm tauri build -- --locked` (SEM dev-tools, conforme AGENTS.md) → relançar plain.

## Rollback

`dev/backup/2026-10-17-topbar-tema-codigo/` (index.html + main.ts pré-mudança + NOTA.md).
