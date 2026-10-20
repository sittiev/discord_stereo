// Dev-only harness for break-ui. Never imported by the app.
// Injected by Playwright via addInitScript; it mocks the Tauri IPC so the
// wizard renders in a plain browser at http://127.0.0.1:1420.
//
// Fixtures are swapped at the data boundary (the invoke response), never by
// editing markup. Dataset is chosen by ?data= or by the segmented control.
const HOSTS = [
  "m3e-navigation-rail.m3e.dev",
  "cdn.discordapp.com",
  "discord.com",
  "discord.com/api/v9/voice",
  "ptb.discord.com",
  "canary.discord.com",
];

function sitesFor(n) {
  return Array.from({ length: n }, (_, i) => [
    HOSTS[i % HOSTS.length],
    i === 0 ? { state: "patched" } : { state: "ok" },
  ]);
}

const DEMO = {
  clients: [
    {
      label: "Discord Stable",
      kind: "stable",
      version_dir: "app-1.0.161",
      node_path:
        "/home/rafael/.config/discord/app-1.0.161/modules/discord_voice-1/discord_voice/discord_voice.node",
      process_running: false,
    },
  ],
  goals: [
    {
      id: "v3",
      title: "Filtros Mortos",
      description: "Stereo core sem krisp/highpass/downmix; bitrate do servidor.",
      sites: 12,
    },
    {
      id: "v4",
      title: "Stereo Full",
      description: "Recomendado: v3 + 510 kbps CBR travado + force-stereo.",
      sites: 18,
    },
    {
      id: "v5",
      title: "Stereo Profundo",
      description: "v4 + kAudio: mais profundidade, centroide mais baixo.",
      sites: 19,
    },
  ],
  sysinfo: {
    os: "linux",
    arch: "x86_64",
    known_pristine_md5: "e7d4b1c0a95f3d2688bb41f7c30e6a59",
  },
  analysis: {
    path: "/home/rafael/.config/discord/app-1.0.161/modules/discord_voice-1/discord_voice/discord_voice.node",
    size: 24803328,
    md5: "9f2c7b41aa0e5d38219f6c04be7d1a823",
    known_pristine: false,
    sites: sitesFor(18),
  },
};

// Every value here is plausible for this product: real Discord install roots,
// a real Windows user profile, a macOS home with an accented full name, and the
// prerelease version dirs Discord actually ships.
const WORST = {
  clients: [
    {
      label: "Discord Stable",
      kind: "stable",
      version_dir: "app-1.0.9163-beta.2",
      node_path:
        "C:\\Users\\Rafael Almeida da Silva Filho\\AppData\\Local\\Discord\\app-1.0.9163-beta.2\\modules\\discord_voice-3\\discord_voice\\discord_voice.node",
      process_running: true,
    },
    {
      label: "Discord Canary",
      kind: "canary",
      version_dir: "app-1.0.9170-rc.1+build.202610161204",
      node_path:
        "/Users/Francisco de Almeida 山田/Library/Application Support/discordcanary/app-1.0.9170-rc.1+build.202610161204/modules/discord_voice-1/discord_voice/discord_voice.node",
      process_running: false,
    },
    {
      label: "Vencord Desktop",
      kind: "vencord",
      version_dir: "app-1.0.161 (Vencord 1.5.4-rc.2 \"beta\")",
      node_path:
        "/home/مستخدم-تجربة/.config/Vencord/app-1.0.161 (Vencord 1.5.4-rc.2 \"beta\")/modules/discord_voice-1/discord_voice/discord_voice.node",
      process_running: false,
    },
    {
      label: "Discord PTB",
      kind: "ptb",
      version_dir: "app-1.0.162",
      node_path:
        "/mnt/data/Backups/DiscordAntigosBackupDefinitivo2026/app-1.0.162/modules/discord_voice-1/discord_voice/discord_voice.node",
      process_running: false,
    },
  ],
  goals: [
    {
      id: "v1",
      title: "Somente identificação de codec e canais",
      description:
        "Patch mínimo: apenas força o Opus a declarar dois canais e remove o downmix do caminho de recepção. Não altera taxa de bitrate, não toca nos filtros de krisp e preserva integralmente a resposta de frequência original do sinal gravado, o que mantém o timbre exatamente como veio da fonte mas deixa o timbre mais fino e menos presente, com menos grave e menos brilho percebidos na escuta.",
      sites: 1,
    },
    {
      id: "v4",
      title: "Stereo Full (recomendado)",
      description: "v3 + 510 kbps CBR travado.",
      sites: 18,
    },
    {
      id: "v5",
      title: "Stereo Profundo",
      description: "v4 + kAudio.",
      sites: 19,
    },
  ],
  sysinfo: {
    os: "linux-gnu (x86_64-unknown-linux-gnu)",
    arch: "x86_64",
    known_pristine_md5: "e7d4b1c0a95f3d2688bb41f7c30e6a59",
  },
  analysis: {
    path: "C:\\Users\\Rafael Almeida da Silva Filho\\AppData\\Local\\Discord\\app-1.0.9163-beta.2\\modules\\discord_voice-3\\discord_voice\\discord_voice.node",
    size: 24803328,
    md5: "9f2c7b41aa0e5d38219f6c04be7d1a823",
    known_pristine: false,
    sites: [
      [
        "stream-secure.voice-prod-a.b.discordvoice.net.edgekey.net.east-aws-edge.net.discord.gg",
        { state: "patched" },
      ],
      [
        "discord.com/api/v9/voice/connections/1000000000000000000/…",
        { state: "mismatch", found: "0x00007f3a2c11_expected_0x00007f3a2c40" },
      ],
      ["localhost", { state: "ok" }],
      [
        "a-very-long-single-token-hostname-that-should-wrap-instead-of-overflowing-the-list-item.voice.example.invalid",
        { state: "ok" },
      ],
    ],
  },
};

const SETS = {
  demo: DEMO,
  worst: WORST,
  empty: {
    clients: [],
    goals: DEMO.goals,
    sysinfo: DEMO.sysinfo,
    analysis: null,
  },
  one: {
    clients: [DEMO.clients[0]],
    goals: DEMO.goals,
    sysinfo: DEMO.sysinfo,
    analysis: DEMO.analysis,
  },
  many: {
    clients: Array.from({ length: 40 }, (_, i) => ({
      label: "Discord Stable",
      kind: "stable",
      version_dir: `app-1.0.${150 + i}`,
      node_path:
        `/home/rafael/.config/discord/app-1.0.${150 + i}/modules/discord_voice-1/discord_voice/discord_voice.node`,
      process_running: i % 7 === 0,
    })),
    goals: DEMO.goals,
    sysinfo: DEMO.sysinfo,
    analysis: null,
  },
};

let current = SETS[new URLSearchParams(location.search).get("data")] ?? DEMO;

const HANDLERS = {
  detect_clients: () => current.clients,
  goals: () => current.goals,
  system_info: () => current.sysinfo,
  analyze_node: () => current.analysis,
  apply_goal: () => current.analysis,
  revert_node: ({ node }) => `${node}.bak-20261016-120000`,
};

window.__TAURI_INTERNALS__ = {
  invoke(cmd, args) {
    const fn = HANDLERS[cmd];
    if (!fn) return Promise.reject(`mock: unknown command ${cmd}`);
    return new Promise((resolve) => setTimeout(() => resolve(fn(args || {})), 120));
  },
  transformCallback: (cb) => cb,
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
};

// Dev-only segmented control, fixed bottom-center, neutral chrome.
window.__mockData = {
  set(name) {
    current = SETS[name] ?? DEMO;
    const u = new URL(location.href);
    u.searchParams.set("data", name);
    history.replaceState(null, "", u);
    const items = document.querySelectorAll("m3e-nav-item");
    const active = document.querySelector("m3e-nav-item[selected]");
    if (active) active.click();
    void items;
  },
  mount() {
    const bar = document.createElement("div");
    bar.id = "breakui-toggle";
    bar.style.cssText =
      "position:fixed;bottom:10px;left:50%;transform:translateX(-50%);z-index:9999;display:flex;gap:2px;padding:3px;border-radius:8px;background:#2b2d31;border:1px solid #4e5058;font:11px/1 system-ui,sans-serif";
    for (const name of ["demo", "worst", "empty", "one", "many"]) {
      const seg = document.createElement("button");
      seg.textContent = name;
      seg.dataset.name = name;
      seg.style.cssText =
        "border:0;border-radius:6px;padding:5px 9px;background:transparent;color:#c8ccd4;cursor:pointer;font:inherit";
      seg.addEventListener("click", () => window.__mockData.set(name));
      bar.append(seg);
    }
    document.body.append(bar);
    const paint = () => {
      for (const b of bar.children) {
        b.style.background = b.dataset.name === current ? "#fff" : "transparent";
        b.style.color = b.dataset.name === current ? "#111" : "#c8ccd4";
      }
    };
    paint();
    window.__mockData.paint = paint;
    return paint;
  },
  set name(v) {
    window.__mockData.set(v);
  },
};

addEventListener("DOMContentLoaded", () => window.__mockData.mount());
