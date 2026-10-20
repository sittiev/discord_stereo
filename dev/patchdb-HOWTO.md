# Patch-DB: como atualizar tabelas quando o Discord atualiza

As tabelas moram em `patchdb/tables.json`. O app embute o arquivo do commit
bootstrap e baixa a raw do `main` como atualização; um workflow anexa o mesmo
JSON como asset da release `patchdb` (histórico/rollback).

## Schema (v1)

- `schema`: versão do schema, hoje `1`.
- `db_version`: contador monotônico do DB; o app só adota remoto com version
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
3. Derive os offsets (Ghidra/IDA, igual hoje) e escreva as tabelas v3/v4/v5
   correspondentes com `binary_md5` do passo 2.
4. Suba o `db_version` e a `updated`.
5. `node dev/validate-patchdb.mjs` — precisa passar.
6. Commit + push. O CI valida; o asset de release se atualiza sozinho.

## Quando usar pattern em vez de offset

- Função pode mudar de endereço entre builds próximos → escreva `pattern`
  uma vez, sem offset, e (opcionalmente sem `binary_md5`) a tabela
  atravessa builds enquanto a assinatura é única e `expect` bate.
- Offset exato na mão de uma tabela md5 específica → `offset` é mais
  barato e cirúrgico.
