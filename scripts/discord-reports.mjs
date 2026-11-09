#!/usr/bin/env node
// Discord report ingestion: pulls the div-*.json trace attachments that the
// app posts to the report channel (a webhook, so reading needs a bot token),
// dedupes by report_id against state/reports.json and downloads new traces
// for the candidate pipeline.
// Usage: node scripts/discord-reports.mjs fetch --state state/reports.json \
//        --out <dir> [--apply-state]
// Env: DISCORD_REPORT_TOKEN (bot), DISCORD_REPORT_CHANNEL (channel id).
// Prints one JSON line per downloaded report for the shell loop.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const API = "https://discord.com/api/v10";
const STATE_CAP = 500;

function parseArgs(argv) {
  if (argv[0] !== "fetch") {
    throw new Error("usage: discord-reports.mjs fetch --state <path> --out <dir> [--apply-state]");
  }
  const args = { state: "state/reports.json" };
  for (let i = 1; i < argv.length; i += 1) {
    if (argv[i] === "--state") args.state = argv[++i];
    else if (argv[i] === "--out") args.out = argv[++i];
    else if (argv[i] === "--apply-state") args.applyState = true;
    else throw new Error(`unknown argument: ${argv[i]}`);
  }
  if (!args.out) throw new Error("fetch: --out is required");
  return args;
}

async function discord(path, token) {
  const res = await fetch(`${API}${path}`, {
    headers: { authorization: `Bot ${token}`, "user-agent": "StereoWizard-Watch/0.3" },
    signal: AbortSignal.timeout(20_000),
  });
  if (!res.ok) throw new Error(`discord GET ${path}: HTTP ${res.status}`);
  return await res.json();
}

async function fetchMessages(channel, token) {
  const messages = [];
  let before = null;
  for (let page = 0; page < 3; page += 1) {
    const query = `?limit=100${before ? `&before=${before}` : ""}`;
    const batch = await discord(`/channels/${channel}/messages${query}`, token);
    messages.push(...batch);
    if (batch.length < 100) break;
    before = batch[batch.length - 1].id;
  }
  return messages;
}

async function download(url) {
  const res = await fetch(url, { signal: AbortSignal.timeout(30_000) });
  if (!res.ok) throw new Error(`attachment: HTTP ${res.status}`);
  return await res.text();
}

function loadState(path) {
  if (!existsSync(path)) return { schema: 1, processed: {} };
  const parsed = JSON.parse(readFileSync(path, "utf8"));
  parsed.processed ??= {};
  return parsed;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const token = process.env.DISCORD_REPORT_TOKEN;
  const channel = process.env.DISCORD_REPORT_CHANNEL;
  if (!token || !channel) throw new Error("DISCORD_REPORT_TOKEN and DISCORD_REPORT_CHANNEL are required");

  const state = loadState(args.state);
  mkdirSync(args.out, { recursive: true });
  const messages = await fetchMessages(channel, token);
  const reports = [];

  for (const message of messages) {
    for (const attachment of message.attachments ?? []) {
      const name = String(attachment.filename ?? "");
      if (!/^div-.+\.json$/i.test(name)) continue;
      const text = await download(attachment.url);
      let report;
      try {
        report = JSON.parse(text);
      } catch {
        process.stderr.write(`reports: ${name}: invalid JSON, skipped\n`);
        continue;
      }
      if (report?.schema !== 2 || !Array.isArray(report.sites) || typeof report.node_md5 !== "string") {
        process.stderr.write(`reports: ${name}: not a schema-2 report, skipped\n`);
        continue;
      }
      if (state.processed[report.report_id]) continue;
      const path = join(args.out, `${report.report_id}.json`);
      writeFileSync(path, `${JSON.stringify(report, null, 2)}\n`);
      state.processed[report.report_id] = report.node_md5;
      reports.push({
        path,
        report_id: report.report_id,
        goal: report.goal,
        os: report.os,
        arch: report.arch,
        discord_dir: report.discord_dir,
        node_md5: report.node_md5,
        sites: report.sites.length,
      });
    }
  }

  if (args.applyState) {
    const entries = Object.entries(state.processed);
    if (entries.length > STATE_CAP) {
      state.processed = Object.fromEntries(entries.slice(-STATE_CAP));
    }
    mkdirSync(join(args.state, ".."), { recursive: true });
    writeFileSync(args.state, `${JSON.stringify(state, null, 2)}\n`);
  }

  for (const report of reports) process.stdout.write(`${JSON.stringify(report)}\n`);
  process.stderr.write(`reports: ${reports.length} new report(s), ${messages.length} message(s) scanned\n`);
}

main().catch((e) => {
  process.stderr.write(`reports: fatal: ${e.message}\n`);
  process.exit(1);
});
