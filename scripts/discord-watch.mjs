#!/usr/bin/env node
// Discord build watcher. Primary source: Discord's own distribution manifests
// per platform (module versions without any download). Bonus source: the
// xHyroM/discord-datamining feed (Windows). Compares against a committed
// state file and posts one webhook embed per changed channel.
//
// Usage:
//   node scripts/discord-watch.mjs check --state state/versions.json \
//        [--report <path>] [--notify] [--apply-state]
//   node scripts/discord-watch.mjs note pr --host 1.0.X --md5 H --run-url U \
//        --auto-v6 <log> --auto-v7 <log> [--platform win|linux --channel stable]
//   node scripts/discord-watch.mjs note issue --host 1.0.X --md5 H \
//        [--report R | --reason text] [--platform win|linux --channel stable]
//   node scripts/discord-watch.mjs tables-differ --head A --tables B   (exit 1 = same patch bytes)
//
// First run (no state file): primary channels count as changes (one notify
// each); bonus channels only seed state - same data, no duplicate spam.
// State schema 2 nests channels under their platform; schema 1 states are
// migrated automatically (flat channels were Windows-only).
// Exit codes: 0 ok (report carries the changes), 1 fatal (manifest batch or
// webhook refused). datamining being down is only a warning (bonus feed).

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const PLATFORM_CHANNELS = { win: ["stable", "ptb", "canary"], linux: ["stable"] };
const DATAMINING_CHANNELS = ["stable", "ptb", "canary"];
const MANIFEST_URL = (platform, channel) =>
  `https://canary.discord.com/api/updates/distributions/app/manifests/latest?platform=${platform}&channel=${channel}&arch=x64`;
const DATAMINING_URL = (channel) =>
  `https://raw.githubusercontent.com/xHyroM/discord-datamining/master/data/client/channels/${channel}/host/host_build.json`;
const COLOR = { discord: 0x5865f2, datamining: 0x99aab5 };
const HEADERS = {
  "user-agent": "StereoWizard-Watch/0.3 (+https://github.com/sittiev/discord_stereo)",
};

const log = (msg) => process.stderr.write(`watch: ${msg}\n`);
const short = (sha) => (sha ? String(sha).slice(0, 12) : "-");
const modulePriority = (n) => (n === "discord_voice" ? 0 : n === "discord_krisp" ? 1 : 2);

function watchTasks() {
  const list = DATAMINING_CHANNELS.map((channel) => ({ source: "datamining", platform: null, channel }));
  for (const [platform, channels] of Object.entries(PLATFORM_CHANNELS)) {
    for (const channel of channels) list.push({ source: "discord", platform, channel });
  }
  return list;
}

async function getJson(url, label, { critical }) {
  try {
    const res = await fetch(url, { headers: HEADERS, signal: AbortSignal.timeout(15_000) });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return await res.json();
  } catch (e) {
    if (critical) throw new Error(`${label}: ${e.message}`);
    return null;
  }
}

function normalizeManifest(raw) {
  const modules = {};
  for (const [name, m] of Object.entries(raw?.modules ?? {})) {
    const f = m?.full ?? {};
    modules[name] = {
      v: Number.isInteger(f.module_version) ? f.module_version : 0,
      sha: typeof f.package_sha256 === "string" ? f.package_sha256 : "",
      url: typeof f.url === "string" ? f.url : "",
    };
  }
  const full = raw?.full ?? {};
  const host = Array.isArray(full.host_version) ? full.host_version.join(".") : String(full.host_version ?? "");
  return { host, modules };
}

function normalizeDatamining(raw) {
  const modules = {};
  for (const m of Array.isArray(raw?.modules) ? raw.modules : []) {
    modules[m.name] = {
      v: Number.isInteger(m.version) ? m.version : 0,
      sha: typeof m.package_sha256 === "string" ? m.package_sha256 : "",
      url: typeof m.url === "string" ? m.url : "",
    };
  }
  return { host: typeof raw?.host_version === "string" ? raw.host_version : "", modules };
}

function changedModules(prev, next) {
  const names = new Set([...Object.keys(prev?.modules ?? {}), ...Object.keys(next.modules)]);
  const changed = [];
  for (const name of names) {
    const before = prev?.modules?.[name];
    const after = next.modules[name];
    if (!before || !after || before.v !== after.v || before.sha !== after.sha) {
      changed.push({ name, from: before ?? null, to: after });
    }
  }
  changed.sort((a, b) => modulePriority(a.name) - modulePriority(b.name));
  return changed;
}

function diffEntry(task, prev, next) {
  const firstRun = prev == null;
  const changed = firstRun
    ? Object.keys(next.modules)
        .sort((a, b) => modulePriority(a) - modulePriority(b))
        .map((name) => ({ name, from: null, to: next.modules[name] }))
    : changedModules(prev, next);
  const hostChanged = firstRun || prev.host !== next.host;
  return {
    source: task.source,
    platform: task.platform,
    channel: task.channel,
    firstRun,
    hostChanged,
    from: prev ?? null,
    to: next,
    changed,
    voiceChanged: changed.some((c) => c.name === "discord_voice"),
    krispChanged: changed.some((c) => c.name === "discord_krisp"),
  };
}

function moduleLines(entry) {
  const lines = entry.changed.map(({ name, from, to }) => {
    const fromText = from ? `\`${short(from.sha)}\`` : "_novo_";
    return `- **${name}** ${fromText} → \`${short(to.sha)}\``;
  });
  const cap = 12;
  if (lines.length > cap) {
    const overflow = `_+${lines.length - cap} módulos_`;
    lines.splice(cap, lines.length - cap, overflow);
  }
  return lines.join("\n") || "_sem mudança de módulos_";
}

function embedFor(entry) {
  const first = !entry.from;
  const hostValue = first
    ? `monitor iniciado em **${entry.to.host}**`
    : `${entry.from.host || "?"} → **${entry.to.host}**`;
  const title =
    entry.source === "discord"
      ? `Discord ${entry.platform} ${entry.channel} • nova build`
      : `Datamining • ${entry.channel}`;
  const embed = {
    title,
    color: COLOR[entry.source],
    fields: [
      { name: "Host", value: hostValue, inline: false },
      { name: `Módulos (${entry.changed.length})`, value: moduleLines(entry), inline: false },
    ],
    footer: {
      text:
        entry.source === "discord"
          ? "Stereo Wizard watch • primária"
          : "Stereo Wizard watch • bônus: xHyroM/discord-datamining",
    },
  };
  if (!first && entry.hostChanged && entry.source === "discord") {
    embed.url = `https://xhyrom.dev/discord/client?channel=${entry.channel}&platform=${entry.platform}`;
  }
  return { username: "Stereo Wizard watch", embeds: [embed] };
}

async function postWebhook(webhook, entry) {
  const payload = JSON.stringify(embedFor(entry));
  let lastError = null;
  for (let attempt = 1; attempt <= 2; attempt++) {
    try {
      const res = await fetch(webhook, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: payload,
        signal: AbortSignal.timeout(15_000),
      });
      if (res.ok) return;
      if (res.status >= 400 && res.status < 500 && res.status !== 429) {
        throw new Error(`webhook recusou ${entry.source}/${entry.platform ?? ""}/${entry.channel}: HTTP ${res.status}`);
      }
      lastError = new Error(`HTTP ${res.status}`);
    } catch (e) {
      lastError = e;
    }
    if (attempt < 2) await sleep(5000);
  }
  throw new Error(
    lastError instanceof Error ? lastError.message : `webhook falhou: ${entry.source}/${entry.channel}`,
  );
}

function markdownSummary(changes) {
  const rows = changes.slice(0, 20).map((c) => {
    const from = c.firstRun ? "—" : c.from?.host || "?";
    const mods = c.changed.filter((m) => m.name === "discord_voice" || m.name === "discord_krisp").map((m) => m.name);
    return `| ${c.source} | ${c.platform ?? "—"} | ${c.channel} | ${from} → ${c.to.host} | ${c.changed.length}${mods.length ? ` (voz/krisp: ${mods.join(", ")})` : ""} |`;
  });
  return [
    "| fonte | plataforma | canal | host | módulos |",
    "| --- | --- | --- | --- | --- |",
    ...rows,
    changes.length > 20 ? `\n(+${changes.length - 20} mudanças ocultas)` : "",
  ]
    .filter(Boolean)
    .join("\n");
}

function loadState(path) {
  if (!existsSync(path)) return { schema: 2, sources: {} };
  const parsed = JSON.parse(readFileSync(path, "utf8"));
  if (parsed.schema === 2) return parsed;
  const legacyWindows = {};
  for (const [channel, entry] of Object.entries(parsed.sources?.discord ?? {})) {
    if (entry && typeof entry === "object" && "host" in entry) legacyWindows[channel] = entry;
  }
  return { schema: 2, sources: { discord: { win: legacyWindows }, datamining: parsed.sources?.datamining ?? {} } };
}

function previousEntry(state, task) {
  const node = state.sources?.[task.source];
  if (!node) return null;
  return task.platform ? node[task.platform]?.[task.channel] ?? null : node[task.channel] ?? null;
}

function storeChecked(checked, task, value) {
  checked[task.source] ??= {};
  if (task.platform) {
    checked[task.source][task.platform] ??= {};
    checked[task.source][task.platform][task.channel] = value;
  } else {
    checked[task.source][task.channel] = value;
  }
}

function applyChecked(state, checked) {
  for (const [source, scopes] of Object.entries(checked)) {
    state.sources[source] ??= {};
    for (const [scope, value] of Object.entries(scopes)) {
      if (value && typeof value === "object" && "host" in value) {
        state.sources[source][scope] = value;
      } else if (value && typeof value === "object") {
        state.sources[source][scope] = { ...state.sources[source][scope], ...value };
      }
    }
  }
}

function noteArguments(argv) {
  const get = (flag) => {
    const i = argv.indexOf(flag);
    if (i === -1) return null;
    return argv[i + 1];
  };
  const kind = argv[0];
  if (kind !== "pr" && kind !== "issue" && kind !== "report") {
    throw new Error("usage: discord-watch.mjs note pr|issue|report ...");
  }
  const out = {
    kind,
    host: get("--host"),
    md5: get("--md5"),
    platform: get("--platform"),
    channel: get("--channel"),
  };
  if (kind !== "report" && (!out.host || !out.md5)) throw new Error("note: --host and --md5 are required");
  if (kind === "pr") {
    out.runUrl = get("--run-url");
    out.autoV6 = get("--auto-v6");
    out.autoV7 = get("--auto-v7");
    if (!out.runUrl || !out.autoV6 || !out.autoV7) {
      throw new Error("note pr: --run-url, --auto-v6 and --auto-v7 are required");
    }
  } else if (kind === "report") {
    out.report = get("--report");
    out.runUrl = get("--run-url");
    out.autoLog = get("--auto");
    if (!out.report || !out.runUrl || !out.autoLog) {
      throw new Error("note report: --report, --run-url and --auto are required");
    }
  } else {
    out.report = get("--report");
    out.reason = get("--reason");
  }
  return out;
}

function noteCommand(argv) {
  const args = noteArguments(argv);
  const md5Ref = `\`${args.md5}\``;
  const target = [args.platform, args.channel].filter(Boolean).join("/");
  if (args.kind === "pr") {
    const v6 = JSON.parse(readFileSync(args.autoV6, "utf8"));
    const v7 = JSON.parse(readFileSync(args.autoV7, "utf8"));
    return [
      "### Tabela candidata automática",
      "",
      `- Build: Discord ${args.platform ?? ""} ${args.host} (node md5 ${md5Ref})`,
      `- Cobertura: v6 ${v6.sites} sites · v7 ${v7.sites} sites (autopatch exit 0)`,
      `- db_version: ${v7.db_version}`,
      `- Run: ${args.runUrl} (artifact \`discord-watch-reports-*\`)`,
      "",
      "Revisão (humano): Reverter → Analisar → Aplicar no wizard e testar de verdade (hard L/R) na plataforma alvo.",
      "O merge atualiza o asset via patchdb-archive; os apps adotam no próximo boot por db_version.",
      "",
    ].join("\n");
  }
  if (args.kind === "report") {
    const report = JSON.parse(readFileSync(args.report, "utf8"));
    const auto = JSON.parse(readFileSync(args.autoLog, "utf8"));
    return [
      "### Tabela candidata a partir de report do app",
      "",
      `- report: \`${report.report_id}\` · goal ${report.goal} · build ${report.os} ${report.discord_dir} · node md5 \`${report.node_md5}\``,
      `- autopatch: ${auto.sites} sites (exit 0) · db_version ${auto.db_version}`,
      `- Run: ${args.runUrl}`,
      "",
      "Revisão (humano): Reverter → Analisar → Aplicar no wizard e testar hard L/R de verdade.",
      "O merge atualiza o asset via patchdb-archive; os apps adotam no próximo boot por db_version.",
      "",
    ].join("\n");
  }
  const lines = [`### Build ${args.platform ?? ""} ${args.host} precisa de RE manual`, "", `- node md5: ${md5Ref}`];
  if (target) lines.push(`- target: ${target}`);
  if (args.reason) lines.push(`- motivo: ${args.reason}`);
  lines.push(`- Reports: artifact \`discord-watch-reports-*\` da run (evidência por site).`);
  if (args.report && existsSync(args.report)) {
    const report = JSON.parse(readFileSync(args.report, "utf8"));
    const failing = report.sites
      .filter((s) => s.state !== "ok")
      .slice(0, 12)
      .map((s) => `- ${s.name}: state=${s.state} found=${(s.found || "?").slice(0, 16)}`);
    lines.push("", "Sites que falharam (v6):", ...(failing.length ? failing : ["- (nenhum no report v6; ver artifact)"]));
  } else {
    lines.push("- O finder não conseguiu gerar report para este binário.");
  }
  lines.push("");
  return lines.join("\n");
}

function tablesDiffer(argv) {
  const get = (flag) => {
    const i = argv.indexOf(flag);
    if (i === -1) throw new Error(`tables-differ: missing ${flag}`);
    return argv[i + 1];
  };
  const digest = (db) =>
    JSON.stringify(
      (db.tables ?? [])
        .map((t) => ({
          goal: t.goal,
          md5: t.binary_md5,
          os: t.os ?? "",
          discord: t.discord ?? "",
          sites: (t.sites ?? [])
            .map((s) => [s.name, s.offset ?? "", s.orig, s.patch, s.expect ?? ""])
            .sort((a, b) => String(a[0]).localeCompare(String(b[0]))),
        }))
        .sort((a, b) => `${a.goal}|${a.md5}`.localeCompare(`${b.goal}|${b.md5}`)),
    );
  const a = digest(JSON.parse(readFileSync(get("--head"), "utf8")));
  const b = digest(JSON.parse(readFileSync(get("--tables"), "utf8")));
  const equal = a === b;
  log(`tables-differ: ${equal ? "idênticas" : "diferentes"}`);
  process.exitCode = equal ? 1 : 0;
}

function parseArgs(argv) {
  if (argv[0] !== "check") {
    throw new Error("usage: discord-watch.mjs check --state <path> [--report <path>] [--notify] [--apply-state]");
  }
  const args = { state: "state/versions.json" };
  for (let i = 1; i < argv.length; i += 1) {
    if (argv[i] === "--state") args.state = argv[++i];
    else if (argv[i] === "--report") args.report = argv[++i];
    else if (argv[i] === "--notify") args.notify = true;
    else if (argv[i] === "--apply-state") args.applyState = true;
    else throw new Error(`unknown argument: ${argv[i]}`);
  }
  return args;
}

async function main() {
  if (process.argv[2] === "note") {
    process.stdout.write(`${noteCommand(process.argv.slice(3))}\n`);
    return;
  }
  if (process.argv[2] === "tables-differ") {
    tablesDiffer(process.argv.slice(3));
    return;
  }
  const args = parseArgs(process.argv.slice(2));
  const state = loadState(args.state);

  const checked = {};
  const changes = [];
  for (const task of watchTasks()) {
    const label = task.platform ? `${task.source}/${task.platform}/${task.channel}` : `${task.source}/${task.channel}`;
    const raw = await getJson(
      task.source === "discord" ? MANIFEST_URL(task.platform, task.channel) : DATAMINING_URL(task.channel),
      label,
      { critical: task.source === "discord" },
    );
    if (!raw) {
      log(`${label}: fonte bônus indisponível (aviso, ignoro)`);
      continue;
    }
    const next = task.source === "discord" ? normalizeManifest(raw) : normalizeDatamining(raw);
    storeChecked(checked, task, next);
    const entry = diffEntry(task, previousEntry(state, task), next);
    if (entry.firstRun || entry.hostChanged || entry.changed.length > 0) changes.push(entry);
  }

  const notified = [];
  if (args.notify) {
    const webhook = process.env.DISCORD_WEBHOOK_UPDATES;
    if (!webhook) throw new Error("--notify sem DISCORD_WEBHOOK_UPDATES");
    for (const entry of changes) {
      if (entry.source === "datamining" && entry.firstRun) {
        log(`datamining/${entry.channel}: primeira run só planta estado`);
        continue;
      }
      await postWebhook(webhook, entry);
      notified.push(`${entry.source}/${entry.platform ?? ""}/${entry.channel}`);
    }
  }

  if (args.applyState) {
    applyChecked(state, checked);
    mkdirSync(dirname(args.state), { recursive: true });
    writeFileSync(args.state, `${JSON.stringify(state, null, 2)}\n`);
  }

  if (args.report) {
    mkdirSync(dirname(args.report), { recursive: true });
    writeFileSync(args.report, `${JSON.stringify({ schema: 2, checked, changes, notified }, null, 2)}\n`);
  }

  const summaryPath = process.env.GITHUB_STEP_SUMMARY;
  if (summaryPath && changes.some((c) => !c.firstRun)) {
    writeFileSync(summaryPath, `## Discord watch\n\n${markdownSummary(changes)}\n`);
  }

  log(`ok: ${changes.length} mudança(s), ${notified.length} embed(s) enviado(s)`);
  console.log(
    JSON.stringify({
      schema: 2,
      changes: changes.length,
      notified,
      detail: changes.map((c) => ({
        source: c.source,
        platform: c.platform,
        channel: c.channel,
        firstRun: c.firstRun,
        hostChanged: c.hostChanged,
        host: c.to.host,
        modules: c.changed.length,
        voiceChanged: c.voiceChanged,
      })),
    }),
  );
}

main().catch((e) => {
  log(`fatal: ${e.message}`);
  process.exit(1);
});
