#!/usr/bin/env node
// Turns a divergence report (schema 2) into a candidate patchdb/tables.json
// table: offsets come from the report's relocation data, never from hand
// hunting. No dependencies; node built-ins + child_process only. Usage:
//   node patchdb/autopatch.mjs <report.json> [tables.json path]
// Exit codes: 0 merged+validated (summary JSON on stdout), 2 hard-validate
// failure, 3 manual RE needed (markdown on stdout), 4 site-rule violation,
// 5 merged tables.json failed patchdb/validate.mjs (stderr on stderr).
import { readFileSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { argv, exit } from "node:process";

const reportPath = argv[2];
const tablesPath = argv[3] ?? "patchdb/tables.json";
if (!reportPath) {
  console.error("usage: node patchdb/autopatch.mjs <report.json> [tables.json path]");
  exit(2);
}

const HEX = (s) => typeof s === "string" && /^[0-9a-fA-F]+$/.test(s) && s.length % 2 === 0;

// 1. Parse + hard-validate the report.
let report;
try {
  report = JSON.parse(readFileSync(reportPath, "utf8"));
} catch (e) {
  console.error(`report parse failed: ${e.message}`);
  exit(2);
}
if (typeof report !== "object" || report === null || Array.isArray(report)) {
  console.error("report must be a JSON object");
  exit(2);
}
if (report.schema !== 2) {
  console.error(`unknown report schema: ${report.schema}`);
  exit(2);
}
if (typeof report.node_md5 !== "string" || !/^[0-9a-f]{32}$/.test(report.node_md5)) {
  console.error("node_md5 must be lowercase 32-char hex");
  exit(2);
}
if (typeof report.goal !== "string" || !/^[a-z0-9_-]{1,32}$/.test(report.goal)) {
  console.error(`goal must match [a-z0-9_-]{1,32}: ${report.goal}`);
  exit(2);
}
if (typeof report.report_id !== "string" || !/^[0-9]{10,}-[0-9a-f]{12}$/.test(report.report_id)) {
  console.error(`report_id malformed: ${report.report_id}`);
  exit(2);
}
if (!Array.isArray(report.sites)) {
  console.error("sites must be an array");
  exit(2);
}

// 2. Eligibility: every site must be resolvable (ok+offset, or uniquely
// relocated), otherwise the patch needs human reverse engineering.
if (report.sites.length === 0) {
  console.log(
    "Autopatch has nothing to patch - the report carries no sites (the goal likely has no tables in the patch db). Manual RE needed.",
  );
  exit(3);
}
const failing = [];
for (const s of report.sites) {
  const resolvable =
    (s.state === "ok" && Number.isInteger(s.offset)) ||
    (s.unique === true && Number.isInteger(s.new_offset) && HEX(s.expected_patch));
  if (!resolvable) failing.push(s);
}
if (failing.length > 0) {
  const lines = failing
    .map((s) => `- \`${s.name}\`: found \`${s.found}\``)
    .join("\n");
  console.log(
    `Autopatch could not relocate every site - manual RE needed for ${failing.length} site(s):\n\n${lines}`,
  );
  exit(3);
}

// 3. Build the candidate table (report site order preserved).
const sites = report.sites.map((s) => {
  const n = Number(s.new_offset ?? s.offset);
  const site = {
    kind: "offset",
    name: s.name,
    offset: `0x${n.toString(16).toUpperCase().padStart(8, "0")}`,
    orig: s.expected_orig,
    patch: s.expected_patch,
  };
  if (!/^0x[0-9A-F]{1,8}$/.test(site.offset)) {
    console.error(`site ${site.name}: offset out of range: ${site.offset}`);
    exit(4);
  }
  if (!HEX(site.orig) || !HEX(site.patch)) {
    console.error(`site ${site.name}: orig/patch must be even-length hex`);
    exit(4);
  }
  if (site.orig.length !== site.patch.length) {
    console.error(`site ${site.name}: orig and patch must be the same length`);
    exit(4);
  }
  return site;
});

const candidate = {
  goal: report.goal,
  binary_md5: report.node_md5,
  ...(typeof report.discord_dir === "string" && report.discord_dir
    ? { discord: report.discord_dir }
    : {}),
  sites,
};

// 4. Merge into tables.json: replace the same (goal, md5) table in place,
// else append. db_version and updated follow the db-file conventions.
let db;
try {
  db = JSON.parse(readFileSync(tablesPath, "utf8"));
} catch (e) {
  console.error(`tables.json read failed: ${e.message}`);
  exit(2);
}
if (!Array.isArray(db.tables)) {
  console.error("tables.json: tables must be an array");
  exit(2);
}
const idx = db.tables.findIndex(
  (t) => t.goal === report.goal && t.binary_md5 === report.node_md5,
);
const added = idx === -1;
if (added) {
  db.tables.push(candidate);
} else {
  db.tables[idx] = candidate;
}
db.db_version += 1;
db.updated = new Date().toISOString().slice(0, 10);
writeFileSync(tablesPath, `${JSON.stringify(db, null, 2)}\n`);

// 5. The merged db must pass the CI validator.
try {
  execFileSync("node", ["patchdb/validate.mjs", tablesPath], { stdio: "pipe" });
} catch (e) {
  console.error(`merged tables.json failed validation: ${e.stderr?.toString() ?? e}`);
  exit(5);
}

// 6. One-line summary for the workflow's PR step.
console.log(
  JSON.stringify({
    report_id: report.report_id,
    goal: report.goal,
    md5: report.node_md5,
    sites: sites.length,
    added,
    db_version: db.db_version,
  }),
);
