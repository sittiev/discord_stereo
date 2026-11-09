#!/usr/bin/env node
// Extracts files/discord_voice.node from a Discord module package (.distro =
// brotli-compressed tar) using node core only - no GNU tar, no python.
// Verified: the manifest package yields the same bytes as the installed
// module (stable win 9267 and linux 1.0.168).
// Usage: node scripts/node-from-distro.mjs <input.distro> <output.node>

import { readFileSync, writeFileSync } from "node:fs";
import { brotliDecompressSync } from "node:zlib";

const [input, output] = process.argv.slice(2);
if (!input || !output) {
  console.error("usage: node scripts/node-from-distro.mjs <in.distro> <out.node>");
  process.exit(2);
}

const data = brotliDecompressSync(readFileSync(input));
for (let i = 0; i + 512 <= data.length; ) {
  const rawName = data.subarray(i, i + 100);
  const end = rawName.indexOf(0);
  const name = rawName.subarray(0, end === -1 ? 100 : end).toString();
  const size = Number.parseInt(data.subarray(i + 124, i + 136).toString("latin1").replace(/\0/g, "").trim(), 8) || 0;
  const type = String.fromCharCode(data[i + 156]);
  const body = i + 512;
  if ((type === "0" || type === "\0") && name === "files/discord_voice.node") {
    writeFileSync(output, data.subarray(body, body + size));
    console.log(`node-from-distro: extracted ${size} bytes`);
    process.exit(0);
  }
  i = body + size + ((512 - (size % 512)) % 512);
}
console.error("node-from-distro: files/discord_voice.node not found in package");
process.exit(1);
