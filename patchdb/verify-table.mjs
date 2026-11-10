#!/usr/bin/env node
// Verifies one Patch-DB table against a clean discord_voice.node: every site's
// orig bytes must match, every patch must write, and reverting the patches must
// restore the original md5. Exits 0 only when the whole apply/revert cycle
// passes, so the watcher can publish without a human gate.
// Usage: verify-table.mjs <node> <md5> <goal> [tables.json]

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

const [nodePath, md5, goal, tablesPath = "patchdb/tables.json"] = process.argv.slice(2);
if (!nodePath || !md5 || !goal) {
  console.error("usage: verify-table.mjs <node> <md5> <goal> [tables.json]");
  process.exit(2);
}

const db = JSON.parse(readFileSync(tablesPath, "utf8"));
const table = db.tables.find((t) => t.goal === goal && t.binary_md5 === md5.toLowerCase());
if (!table) {
  console.error(`no ${goal} table for ${md5} in ${tablesPath}`);
  process.exit(1);
}

const data = Buffer.from(readFileSync(nodePath));
const clean = createHash("md5").update(data).digest("hex");
if (clean !== md5.toLowerCase()) {
  console.error(`node md5 ${clean} does not match ${md5}`);
  process.exit(1);
}

for (const site of table.sites) {
  if (site.kind !== "offset") {
    console.error(`${site.name}: unsupported site kind ${site.kind}`);
    process.exit(1);
  }
  const offset = Number.parseInt(site.offset, 16);
  const orig = Buffer.from(site.orig, "hex");
  if (!data.subarray(offset, offset + orig.length).equals(orig)) {
    console.error(`${site.name}: clean bytes differ at ${site.offset}`);
    process.exit(1);
  }
  Buffer.from(site.patch, "hex").copy(data, offset);
}

for (const site of table.sites) {
  const offset = Number.parseInt(site.offset, 16);
  const patch = Buffer.from(site.patch, "hex");
  if (!data.subarray(offset, offset + patch.length).equals(patch)) {
    console.error(`${site.name}: patched bytes differ at ${site.offset}`);
    process.exit(1);
  }
}

const reverted = Buffer.from(data);
for (const site of table.sites) {
  const offset = Number.parseInt(site.offset, 16);
  Buffer.from(site.orig, "hex").copy(reverted, offset);
}
if (createHash("md5").update(reverted).digest("hex") !== clean) {
  console.error("revert md5 mismatch");
  process.exit(1);
}

console.log(`${goal} ${md5}: ${table.sites.length} sites verified (apply + revert)`);
