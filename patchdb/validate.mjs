#!/usr/bin/env node
// Validates patchdb/tables.json (schema v1) with no dependencies. Usage:
//   node dev/validate-patchdb.mjs [path]
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
const locOk = (o) =>
  o !== null &&
  typeof o === "object" &&
  !Array.isArray(o) &&
  Object.keys(o).length === 2 &&
  typeof o.pt === "string" &&
  !!o.pt.trim() &&
  typeof o.en === "string" &&
  !!o.en.trim();

let db;
try {
  db = JSON.parse(readFileSync(file, "utf8"));
} catch (e) {
  fail(`invalid JSON: ${e.message}`);
}

if (db.schema !== 1) fail(`unknown schema: ${db.schema}`);
if (!Number.isInteger(db.db_version)) fail("db_version must be an integer");
if (typeof db.updated !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(db.updated)) {
  fail("updated must be YYYY-MM-DD");
}

const goalIds = new Set((db.goals ?? []).map((g) => g.id));
if (goalIds.size !== db.goals.length) fail("duplicate goal id in goals");
for (const g of db.goals) {
  if (typeof g.id !== "string" || !g.id) fail("goal.id is required");
  if (!locOk(g.title)) fail(`goal ${g.id}: title must contain pt and en strings`);
  if (!locOk(g.description)) {
    fail(`goal ${g.id}: description must contain pt and en strings`);
  }
}

const tables = db.tables ?? [];
if (tables.length === 0) fail("no tables");
const seen = new Set();
for (const t of tables) {
  if (!goalIds.has(t.goal)) fail(`table with unknown goal: ${t.goal}`);
  const hasOffsetSites = t.sites.some((s) => s.kind === "offset");
  if (hasOffsetSites && typeof t.binary_md5 !== "string") {
    fail(`table ${t.goal} (${t.discord ?? "no discord"}) with offset sites requires binary_md5`);
  }
  if (t.binary_md5 !== undefined && !/^[0-9a-f]{32}$/.test(t.binary_md5)) {
    fail(`table ${t.goal}/${t.discord ?? "any"}: binary_md5 must be lowercase 32-char md5 hex`);
  }
  const pair = `${t.goal}:${t.binary_md5 ?? "*"}:${t.discord ?? "*"}`;
  if (seen.has(pair)) fail(`duplicate table: ${pair}`);
  seen.add(pair);
  for (const s of t.sites) {
    if (s.kind !== "offset" && s.kind !== "pattern") {
      fail(`invalid site kind: ${s.kind} (${s.name})`);
    }
    if (typeof s.name !== "string" || !s.name) fail("site name is required");
    if (s.kind === "offset") {
      if (!/^0x[0-9a-fA-F]{1,8}$/.test(s.offset)) {
        fail(`site ${s.name}: offset must be 0x... hex`);
      }
      if (!HEX(s.orig) || !HEX(s.patch)) {
        fail(`site ${s.name}: orig/patch must be even-length hex`);
      }
      if (s.orig.length !== s.patch.length) {
        fail(`site ${s.name}: orig and patch must be the same length`);
      }
    } else {
      if (!HEX_OR_WILD(s.pattern)) {
        fail(`site ${s.name}: pattern must be hex with ?? (2 chars per token)`);
      }
      if (!HEX(s.patch)) fail(`site ${s.name}: patch must be even-length hex`);
      if (spanBytes(s.pattern) * 2 !== s.patch.length) {
        fail(`site ${s.name}: patch must cover the full pattern span`);
      }
      if (s.expect !== undefined) {
        if (!HEX(s.expect)) fail(`site ${s.name}: expect must be even-length hex`);
        if (s.expect.length !== s.patch.length) {
          fail(`site ${s.name}: expect must be the same length as patch`);
        }
      }
    }
  }
}

console.log(
  `patchdb ok: ${db.goals.length} goals, ${tables.length} tables, db_version ${db.db_version}`,
);
