#!/usr/bin/env node
// Valida patchdb/tables.json (schema v1) sem dependências. Uso:
//   node dev/validate-patchdb.mjs [caminho]
import { readFileSync } from "node:fs";
import { argv, exit } from "node:process";

const file = argv[2] ?? "patchdb/tables.json";
const fail = (m) => {
  console.error(`${file}: ${m}`);
  exit(1);
};

const HEX = (s) => /^[0-9a-fA-F]+$/.test(s) && s.length % 2 === 0;
const HEX_OR_WILD = (s) =>
  /^([0-9a-fA-F]{2}|\?\?)(\s*([0-9a-fA-F]{2}|\?\?))*$/.test(s);
const spanBytes = (p) => p.replace(/\s/g, "").length / 2;

let db;
try {
  db = JSON.parse(readFileSync(file, "utf8"));
} catch (e) {
  fail(`JSON inválido: ${e.message}`);
}

if (db.schema !== 1) fail(`schema desconhecido: ${db.schema}`);
if (!Number.isInteger(db.db_version)) fail("db_version deve ser número inteiro");
if (typeof db.updated !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(db.updated)) {
  fail("updated deve ser YYYY-MM-DD");
}

const goalIds = new Set((db.goals ?? []).map((g) => g.id));
if (goalIds.size !== db.goals.length) fail("goal id duplicado em goals");
for (const g of db.goals) {
  if (typeof g.id !== "string" || !g.id) fail("goal.id obrigatório");
  if (typeof g.title !== "string" || !g.title) {
    fail(`goal ${g.id}.title obrigatório`);
  }
  if (typeof g.description !== "string" || !g.description) {
    fail(`goal ${g.id}.description obrigatório`);
  }
}

const tables = db.tables ?? [];
if (tables.length === 0) fail("nenhuma tabela");
const seen = new Set();
for (const t of tables) {
  if (!goalIds.has(t.goal)) fail(`tabela com goal desconhecido: ${t.goal}`);
  const hasOffsetSites = t.sites.some((s) => s.kind === "offset");
  if (hasOffsetSites && typeof t.binary_md5 !== "string") {
    fail(`tabela ${t.goal} (${t.discord ?? "sem discord"}) com sites offset exige binary_md5`);
  }
  if (t.binary_md5 !== undefined && !/^[0-9a-f]{32}$/.test(t.binary_md5)) {
    fail(`tabela ${t.goal}/${t.discord ?? "qualquer"}: binary_md5 deve ser md5 hex minúsculo de 32`);
  }
  const pair = `${t.goal}:${t.binary_md5 ?? "*"}:${t.discord ?? "*"}`;
  if (seen.has(pair)) fail(`tabela duplicada: ${pair}`);
  seen.add(pair);
  for (const s of t.sites) {
    if (s.kind !== "offset" && s.kind !== "pattern") {
      fail(`site com kind inválido: ${s.kind} (${s.name})`);
    }
    if (typeof s.name !== "string" || !s.name) fail("name obrigatório nos sites");
    if (s.kind === "offset") {
      if (!/^0x[0-9a-fA-F]{1,8}$/.test(s.offset)) {
        fail(`site ${s.name}: offset deve ser hex 0x...`);
      }
      if (!HEX(s.orig) || !HEX(s.patch)) {
        fail(`site ${s.name}: orig/patch devem ser hex pares`);
      }
      if (s.orig.length !== s.patch.length) {
        fail(`site ${s.name}: orig e patch de mesmo comprimento`);
      }
    } else {
      if (!HEX_OR_WILD(s.pattern)) {
        fail(`site ${s.name}: pattern deve ser hex com ?? (2 chars por token)`);
      }
      if (!HEX(s.patch)) fail(`site ${s.name}: patch deve ser hex par`);
      if (spanBytes(s.pattern) * 2 !== s.patch.length) {
        fail(`site ${s.name}: patch deve cobrir o span inteiro do pattern`);
      }
      if (s.expect !== undefined) {
        if (!HEX(s.expect)) fail(`site ${s.name}: expect deve ser hex par`);
        if (s.expect.length !== s.patch.length) {
          fail(`site ${s.name}: expect mesmo comprimento do patch`);
        }
      }
    }
  }
}

console.log(
  `patchdb ok: ${db.goals.length} goals, ${tables.length} tabelas, db_version ${db.db_version}`,
);
