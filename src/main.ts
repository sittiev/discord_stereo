// Import per component, never `@m3e/web/all`: the `all.js` bundle is a
// pre-bundle that embeds a second copy of icon.js, and the SVG registry
// of `m3e-icon` gets orphaned — icons fall back to ligatures.
import "@m3e/web/theme";
import "@m3e/web/button";
import "@m3e/web/callout";
import "@m3e/web/card";
import "@m3e/web/chips";
import "@m3e/web/dialog";
import "@m3e/web/expansion-panel";
import "@m3e/web/heading";
import "@m3e/web/icon-button";
import "@m3e/web/list";
import "@m3e/web/progress-indicator";
import "@m3e/web/segmented-button";
import "@m3e/web/snackbar";
import "@m3e/web/stepper";
import "@m3e/web/switch";

import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

import confetti from "canvas-confetti";

type ClientKind = "stable" | "ptb" | "canary" | "vencord";

interface FoundClient {
  label: string;
  kind: ClientKind;
  version_dir: string;
  node_path: string;
  process_running: boolean;
}

interface Localized {
  pt: string;
  en: string;
}

// UI copy is English (see index.html); pt stays as fallback.
function loc(l: Localized): string {
  return l.en || l.pt;
}

interface GoalInfo {
  id: string;
  title: Localized;
  description: Localized;
  sites: number;
}

type SiteState =
  | { state: "ok" }
  | { state: "patched" }
  | { state: "mismatch"; found: string };
interface Analysis {
  path: string;
  size: number;
  md5: string;
  known_pristine: boolean;
  sites: Array<[string, SiteState]>;
  expected?: Array<[string, string]>;
}

interface SystemInfo {
  os: string;
  arch: string;
  known_pristine_md5: string;
}

// ---------------------------------------------------------------- state
let clients: FoundClient[] = [];
let selectedClient: FoundClient | null = null;
let goals: GoalInfo[] = [];
let selectedGoal = "v4";
let analysis: Analysis | null = null;

// ---------------------------------------------------------------- steps
// Header is the native `m3e-stepper`. Panels live outside it: in
// @m3e/web 2.10.x the internal `slot name="panel"` collapses to height 0, so
// panel switching is done here and the stepper only reflects the current step.
const ORDER = ["s-clients", "s-goal", "s-apply", "s-info"] as const;
const ON_ENTER: Record<string, () => void> = {
  "s-clients": () => void refreshClients(),
  "s-goal": () => void loadGoals(),
  "s-apply": () => renderApply(),
  "s-info": () => {
    void loadSysInfo();
    renderAbout();
  },
};
const steps = [...document.querySelectorAll<HTMLElement>("m3e-step")];
let currentStep: (typeof ORDER)[number] | null = null;

function gotoStep(id: string): void {
  if (!(ORDER as readonly string[]).includes(id)) return;
  const from = currentStep === null ? -1 : ORDER.indexOf(currentStep);
  const to = ORDER.indexOf(id as (typeof ORDER)[number]);
  const outEl = from >= 0 ? document.getElementById(ORDER[from]) : null;
  const inEl = document.getElementById(id);
  currentStep = id as (typeof ORDER)[number];
  for (const s of steps) {
    const idx = ORDER.indexOf((s.getAttribute("for") ?? "") as (typeof ORDER)[number]);
    s.toggleAttribute("selected", idx === to);
    s.toggleAttribute("completed", idx !== -1 && idx < to);
  }
  document.querySelector("main")!.scrollTo(0, 0);
  ON_ENTER[id]?.();
  // Animated step change; without animation (or first entry), plain swap.
  if (animPref && from !== -1 && from !== to && outEl && inEl && !outEl.hidden) {
    slide(outEl, inEl, to > from);
  } else {
    for (const panel of document.querySelectorAll<HTMLElement>("section.pane")) {
      panel.hidden = panel.id !== id;
    }
  }
}

// Step change as a carousel (the Motion "item offset" pattern): the current
// step slides out and fades away, the new one enters from the opposite side
// and appears, synchronized and animated via WAAPI (cancel/await is handled
// straight in JS). The #panels height slides along, from the current step to
// the new one, and ends at the natural value — no jump when the swap ends.
const SLIDE_MS = 340;
// Fast start, soft settle, no bounce.
const SLIDE_EASE = "cubic-bezier(0.32, 0.72, 0, 1)";
let slideCleanup: (() => void) | null = null;

function slide(outEl: HTMLElement, inEl: HTMLElement, forward: boolean): void {
  slideCleanup?.();
  const wrap = document.getElementById("panels")!;
  const dx = forward ? 1 : -1;
  const hFrom = wrap.offsetHeight;
  outEl.style.position = "absolute";
  outEl.style.top = "0";
  outEl.style.left = "0";
  outEl.style.right = "0";
  outEl.style.pointerEvents = "none";
  outEl.style.willChange = "transform, opacity";
  inEl.hidden = false;
  inEl.style.willChange = "transform, opacity";
  const hTo = inEl.offsetHeight;
  // Height: leaves the current value and lands on the new step's with the
  // same duration and curve; the final value is natural, so cleanup moves
  // nothing.
  wrap.style.transition = "none";
  wrap.style.height = `${hFrom}px`;
  void wrap.offsetHeight;
  wrap.style.transition = `height ${SLIDE_MS}ms ${SLIDE_EASE}`;
  wrap.style.height = `${hTo}px`;
  const opts: KeyframeAnimationOptions = {
    duration: SLIDE_MS,
    easing: SLIDE_EASE,
    fill: "both",
  };
  const outAnim = outEl.animate(
    [
      { transform: "translateX(0)", opacity: 1 },
      // Fade reaches 0 at 70% (the slide runs to the end, but invisibly):
      // the compositor delays a few frames and, with the fade stretched to
      // 100%, the trail showed the previous panel visible at the end — the "flicker".
      { offset: 0.7, transform: `translateX(${-dx * 70}%)`, opacity: 0 },
      { transform: `translateX(${-dx * 100}%)`, opacity: 0 },
    ],
    opts,
  );
  const inAnim = inEl.animate(
    [
      { offset: 0, transform: `translateX(${dx * 100}%)`, opacity: 0.25 },
      { offset: 0.5, transform: `translateX(${dx * 25}%)`, opacity: 1 },
      { offset: 1, transform: "translateX(0)", opacity: 1 },
    ],
    opts,
  );
  const done = () => {
    if (slideCleanup !== done) return;
    slideCleanup = null;
    outEl.hidden = true;
    outAnim.cancel();
    inAnim.cancel();
    outEl.style.cssText = "";
    inEl.style.willChange = "";
    wrap.style.height = "";
    wrap.style.transition = "";
  };
  slideCleanup = done;
  // Fast navigation cancels `finished`; allSettled covers both cases.
  Promise.allSettled([outAnim.finished, inAnim.finished]).then(done);
  window.setTimeout(done, SLIDE_MS + 400);
  window.setTimeout(() => {
    if (slideCleanup !== done) return;
    outAnim.cancel();
    outEl.style.opacity = "0";
  }, (SLIDE_MS * 7) / 10 + 20);
}

// Stepper clicks navigate; Back/Continue from the footers use `data-step`
// (handler in boot).
for (const step of steps) {
  step.addEventListener("click", () => {
    const id = step.getAttribute("for");
    if (id) gotoStep(id);
  });
}

// ---------------------------------------------------------------- helpers
// Theme: "auto" follows the system (m3e-theme observes prefers-color-scheme);
// "light"/"dark" pin the scheme. Animations: off, the step slide and the
// confetti drop out and the theme falls back to motion="standard".
const PREF_THEME = "sw.theme";
const PREF_ANIM = "sw.anim";
const themeEl = document.querySelector("m3e-theme")!;

type ThemePref = "auto" | "light" | "dark";
// Cycles the footer button; the segmented control on Info covers all three
// directly.
const THEME_ORDER: ThemePref[] = ["auto", "light", "dark"];
const THEME_LABEL: Record<ThemePref, string> = {
  auto: "system",
  light: "light",
  dark: "dark",
};
// Embedded Material Symbols (offline, like the client icons).
const THEME_ICON: Record<ThemePref, string> = {
  auto: `<svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true"><path fill="currentColor" d="M10.85 12.65h2.3L12 9l-1.15 3.65zM20 8.69V4h-4.69L12 .69 8.69 4H4v4.69L.69 12 4 15.31V20h4.69L12 23.31 15.31 20H20v-4.69L23.31 12 20 8.69zM14.3 16l-.7-2h-3.2l-.7 2H7.8L11 7h2l3.2 9h-1.9z"/></svg>`,
  light: `<svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true"><path fill="currentColor" d="M12 7c-2.76 0-5 2.24-5 5s2.24 5 5 5 5-2.24 5-5-2.24-5-5-5zM2 13h2c.55 0 1-.45 1-1s-.45-1-1-1H2c-.55 0-1 .45-1 1s.45 1 1 1zm18 0h2c.55 0 1-.45 1-1s-.45-1-1-1h-2c-.55 0-1 .45-1 1s.45 1 1 1zM11 2v2c0 .55.45 1 1 1s1-.45 1-1V2c0-.55-.45-1-1-1s-1 .45-1 1zm0 18v2c0 .55.45 1 1 1s1-.45 1-1v-2c0-.55-.45-1-1-1s-1 .45-1 1zM5.99 4.58c-.39-.39-1.03-.39-1.41 0-.39.39-.39 1.03 0 1.41l1.06 1.06c.39.39 1.03.39 1.41 0s.39-1.03 0-1.41L5.99 4.58zm12.37 12.37c-.39-.39-1.03-.39-1.41 0-.39.39-.39 1.03 0 1.41l1.06 1.06c.39.39 1.03.39 1.41 0 .39-.39.39-1.03 0-1.41l-1.06-1.06zm1.06-10.96c.39-.39.39-1.03 0-1.41-.39-.39-1.03-.39-1.41 0l-1.06 1.06c-.39.39-.39 1.03 0 1.41s1.03.39 1.41 0l1.06-1.06zM7.05 18.36c.39-.39.39-1.03 0-1.41-.39-.39-1.03-.39-1.41 0l-1.06 1.06c-.39.39-.39 1.03 0 1.41s1.03.39 1.41 0l1.06-1.06z"/></svg>`,
  dark: `<svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true"><path fill="currentColor" d="M12 3c-4.97 0-9 4.03-9 9s4.03 9 9 9 9-4.03 9-9c0-.46-.04-.92-.1-1.36-.98 1.37-2.58 2.26-4.4 2.26-2.98 0-5.4-2.42-5.4-5.4 0-1.81.89-3.42 2.26-4.4-.44-.06-.9-.1-1.36-.1z"/></svg>`,
};

function nextTheme(v: ThemePref): ThemePref {
  return THEME_ORDER[(THEME_ORDER.indexOf(v) + 1) % THEME_ORDER.length];
}

function isThemePref(v: string | null): v is ThemePref {
  return v === "auto" || v === "light" || v === "dark";
}

function storeGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function storeSet(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* no localStorage: the preference lasts only for this session */
  }
}

function loadThemePref(): ThemePref {
  const v = storeGet(PREF_THEME);
  return isThemePref(v) ? v : "auto";
}

function loadAnimPref(): boolean {
  const v = storeGet(PREF_ANIM);
  if (v === "on") return true;
  if (v === "off") return false;
  // Default: on, except when the system asks for less motion.
  return !matchMedia("(prefers-reduced-motion: reduce)").matches;
}

let themePref = loadThemePref();
let animPref = loadAnimPref();

function applyThemePref(): void {
  themeEl.setAttribute("scheme", themePref);
  document.documentElement.style.colorScheme =
    themePref === "auto" ? "light dark" : themePref;
}

function applyAnimPref(): void {
  themeEl.setAttribute("motion", animPref ? "expressive" : "standard");
}

function setThemePref(v: ThemePref): void {
  themePref = v;
  storeSet(PREF_THEME, v);
  applyThemePref();
  syncThemeCtl();
}

function syncThemeCtl(): void {
  for (const seg of document.querySelectorAll<HTMLElement & { checked: boolean }>(
    "#pref-theme m3e-button-segment",
  )) {
    const want = seg.getAttribute("value") === themePref;
    seg.toggleAttribute("checked", want);
    seg.checked = want;
  }
  const btn = document.getElementById("btn-theme");
  if (btn) {
    btn.innerHTML = THEME_ICON[themePref];
    btn.setAttribute(
      "aria-label",
      `Theme: ${THEME_LABEL[themePref]} — click for ${THEME_LABEL[nextTheme(themePref)]}`,
    );
  }
}

// Escapes for interpolation into HTML: covers text and attribute values
// (a path with quotes cannot break `title="…"`).
const HTML_ESCAPES: Record<string, string> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
};

function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => HTML_ESCAPES[c]!);
}

// Install paths are told apart by their root (stable, PTB, canary,
// Vencord); the rest is `app-*/modules/discord_voice/...` on every one.
function installRoot(p: string): string {
  const i = p.search(/[\\/]app-/);
  return i > 0 ? p.slice(0, i) : p;
}

// Real brand mark of each flavor at 24px (Simple Icons, embedded paths so
// it works offline). Colors only to tell them apart in the list.
const CLIENT_ICON: Record<ClientKind, { path: string; color: string }> = {
  stable: { path: "M20.317 4.3698a19.7913 19.7913 0 00-4.8851-1.5152.0741.0741 0 00-.0785.0371c-.211.3753-.4447.8648-.6083 1.2495-1.8447-.2762-3.68-.2762-5.4868 0-.1636-.3933-.4058-.8742-.6177-1.2495a.077.077 0 00-.0785-.037 19.7363 19.7363 0 00-4.8852 1.515.0699.0699 0 00-.0321.0277C.5334 9.0458-.319 13.5799.0992 18.0578a.0824.0824 0 00.0312.0561c2.0528 1.5076 4.0413 2.4228 5.9929 3.0294a.0777.0777 0 00.0842-.0276c.4616-.6304.8731-1.2952 1.226-1.9942a.076.076 0 00-.0416-.1057c-.6528-.2476-1.2743-.5495-1.8722-.8923a.077.077 0 01-.0076-.1277c.1258-.0943.2517-.1923.3718-.2914a.0743.0743 0 01.0776-.0105c3.9278 1.7933 8.18 1.7933 12.0614 0a.0739.0739 0 01.0785.0095c.1202.099.246.1981.3728.2924a.077.077 0 01-.0066.1276 12.2986 12.2986 0 01-1.873.8914.0766.0766 0 00-.0407.1067c.3604.698.7719 1.3628 1.225 1.9932a.076.076 0 00.0842.0286c1.961-.6067 3.9495-1.5219 6.0023-3.0294a.077.077 0 00.0313-.0552c.5004-5.177-.8382-9.6739-3.5485-13.6604a.061.061 0 00-.0312-.0286zM8.02 15.3312c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9555-2.4189 2.157-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.9555 2.4189-2.1569 2.4189zm7.9748 0c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9554-2.4189 2.1569-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.946 2.4189-2.1568 2.4189Z", color: "#5865F2" },
  ptb: { path: "M20.317 4.3698a19.7913 19.7913 0 00-4.8851-1.5152.0741.0741 0 00-.0785.0371c-.211.3753-.4447.8648-.6083 1.2495-1.8447-.2762-3.68-.2762-5.4868 0-.1636-.3933-.4058-.8742-.6177-1.2495a.077.077 0 00-.0785-.037 19.7363 19.7363 0 00-4.8852 1.515.0699.0699 0 00-.0321.0277C.5334 9.0458-.319 13.5799.0992 18.0578a.0824.0824 0 00.0312.0561c2.0528 1.5076 4.0413 2.4228 5.9929 3.0294a.0777.0777 0 00.0842-.0276c.4616-.6304.8731-1.2952 1.226-1.9942a.076.076 0 00-.0416-.1057c-.6528-.2476-1.2743-.5495-1.8722-.8923a.077.077 0 01-.0076-.1277c.1258-.0943.2517-.1923.3718-.2914a.0743.0743 0 01.0776-.0105c3.9278 1.7933 8.18 1.7933 12.0614 0a.0739.0739 0 01.0785.0095c.1202.099.246.1981.3728.2924a.077.077 0 01-.0066.1276 12.2986 12.2986 0 01-1.873.8914.0766.0766 0 00-.0407.1067c.3604.698.7719 1.3628 1.225 1.9932a.076.076 0 00.0842.0286c1.961-.6067 3.9495-1.5219 6.0023-3.0294a.077.077 0 00.0313-.0552c.5004-5.177-.8382-9.6739-3.5485-13.6604a.061.061 0 00-.0312-.0286zM8.02 15.3312c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9555-2.4189 2.157-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.9555 2.4189-2.1569 2.4189zm7.9748 0c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9554-2.4189 2.1569-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.946 2.4189-2.1568 2.4189Z", color: "#949CF7" },
  canary: { path: "M20.317 4.3698a19.7913 19.7913 0 00-4.8851-1.5152.0741.0741 0 00-.0785.0371c-.211.3753-.4447.8648-.6083 1.2495-1.8447-.2762-3.68-.2762-5.4868 0-.1636-.3933-.4058-.8742-.6177-1.2495a.077.077 0 00-.0785-.037 19.7363 19.7363 0 00-4.8852 1.515.0699.0699 0 00-.0321.0277C.5334 9.0458-.319 13.5799.0992 18.0578a.0824.0824 0 00.0312.0561c2.0528 1.5076 4.0413 2.4228 5.9929 3.0294a.0777.0777 0 00.0842-.0276c.4616-.6304.8731-1.2952 1.226-1.9942a.076.076 0 00-.0416-.1057c-.6528-.2476-1.2743-.5495-1.8722-.8923a.077.077 0 01-.0076-.1277c.1258-.0943.2517-.1923.3718-.2914a.0743.0743 0 01.0776-.0105c3.9278 1.7933 8.18 1.7933 12.0614 0a.0739.0739 0 01.0785.0095c.1202.099.246.1981.3728.2924a.077.077 0 01-.0066.1276 12.2986 12.2986 0 01-1.873.8914.0766.0766 0 00-.0407.1067c.3604.698.7719 1.3628 1.225 1.9932a.076.076 0 00.0842.0286c1.961-.6067 3.9495-1.5219 6.0023-3.0294a.077.077 0 00.0313-.0552c.5004-5.177-.8382-9.6739-3.5485-13.6604a.061.061 0 00-.0312-.0286zM8.02 15.3312c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9555-2.4189 2.157-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.9555 2.4189-2.1569 2.4189zm7.9748 0c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9554-2.4189 2.1569-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.946 2.4189-2.1568 2.4189Z", color: "#FAA61A" },
  vencord: { path: "M.613.6v.587h2.4V.6h-1.2zm2.4.587v8.4H3.6v-8.4h-.293zm.587 8.4v.613h.613v-.613h-.306zm.613.613v.587H4.8V10.2h-.293zm.587.587v.613h.613v-.613h-.306zm.613.613v.587H6V11.4h-.293zm.587.587v.613h1.2v-.613h-.6zm1.2 0h.613V11.4H7.2v.293zm.613-.587H8.4V15h-.587v.587H8.4v4.8h.613v-9.6h2.4v-2.4H10.8V10.2H9.6v-.613h-.587v.613H8.4v.587h-.587v.306zM9.6 9.587h.613v-8.4H9.6v4.2zm.613-8.4h2.4V.6h-2.4v.293zm2.4 0V7.8h-1.2v.587H15.6v.32H18v-.32h.614v.32h2.106v-.32h.294V6.893h-.294v-.906h-.32v-.88h-.293V5.4h-.293v.907h-.294V6.6h.294v.587h.293v-.88h.293v.88h.32v.906h-.32v.294h-.586v-.294h.293V7.8h.293v-.613h-.293v.32h-.293V7.8h-.294v.587h-.613V7.8h-.293v-.293h-.294V7.8h-1.813v.293h1.2v.294H15.92v-.294h-1.813V7.8h-.293V6.307h-.294V7.8h-.32V1.187h-.293zm1.2 5.12h.294v-.32h-.294v.16zm.294-.32h.293V5.4h-.293v.293zM14.4 5.4h.614v-.293H14.4v.146zm.614-.293h.586V5.4h.32v-.613h-.906v.16zm.906-.32h.293v-.294h-.293v.147zm.293-.294h.294V4.2h-.294v.147zm.294-.293h.293v-.293h-.293v.146zm.293 0v.587h.32v.32h.293v-.32h.294v.32h-.294V5.4H18v-.613h.32V4.2H18v.293h-.88V4.2h-.16zm1.52 0h.294v-.293h-.294v.146zm.294-.293h.293v-.32h-.293v.16zm.293-.32h.293v-.294h-.293v.147zm.293 0v1.52h.32v-.32h.293v-.294h-.293v-.906h-.16zm.613 1.2v.32h.294v-.32h-.147zm0 .32h-.293V5.4h.293v-.147zM15.6 5.4h-.293v1.2h.293V6zm0 1.2v.587h.32V6.6h-.16zm.32.587v.32h.293v-.32h-.146zm.293.32V7.8h.294v-.293h-.147zm2.694.293h.613v-.293h-.613v.146zm.613-.293h.293v-.32h-.293v.16zm0-.907h-.613v.293h.613v-.146zm-.613.293h-.293v.294h.293V7.04zm2.107 1.494v2.4h2.4V10.2H21.6V8.387h-.293zm2.4 2.4v2.4H24v-2.4h-.293zm0 2.4h-4.8v.613h4.8v-.307zm-4.8 0v-2.4h-4.8v9.6h4.8v-2.4H18V19.8h-3.6v-8.4H18v1.787h.307zm0 4.8h4.8V17.4h-4.8v.293zm4.8 0v2.4H24v-2.4h-.293zm0 2.4h-2.4v2.4h.586V21h1.814v-.306zm-2.4 2.4h-9.6v.613h9.6v-.307zm-9.6 0v-2.4h-2.4V21H10.8v1.787h.307zm-3.6-7.2h-2.4v.613h2.4v-.307zm-2.4 0V15H4.8v.587h.307zM4.8 15v-.613h-.586V15h.293zm-.586-.613V13.8H3.6v.587h.307zM3.6 13.8v-.613h-.587v.613h.294zm-.587-.613V12.6H2.4v.587h.307zM2.4 12.6v-.613h-.587v.613h.294zm-.587-.613V11.4H1.2v.587h.307zM1.2 11.4v-.613H.614v.613h.293zm-.586-.613v-9.6H0v9.6h.307zM13.52.6v.293h1.493v.293h.293V.893h.294V.6h-1.04zm1.493.586h-.293v.32h.293v-.16zm-.293.32h-.32V1.8h.32v-.147zm0 .294v.293h.293V1.8h-.147zm-.32 0h-.294v.293h.294v-.147zm-.294.293h-.293v.293h-.293v.32h2.08v-.32h-1.494V2.24zm.294-.587v-.32h-.294v.32h.147zm2.106.294v.293h.907v.293h.293v-.293H18V1.8h-.747zm.907.586h-.293v.32h-.614V3H18v-.294h-.587v-.16zm-1.2 4.214v.293h.586V6.6h-.293zm.586.293v.293h.907v-.293h-.453zm.907.293v.32H18v-.32h-.146z", color: "#3DB389" },
};

function clientIcon(kind: ClientKind): string {
  const ic = CLIENT_ICON[kind];
  return `<span class="client-icon" style="color:${ic.color}"><svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="${ic.path}"/></svg></span>`;
}

// ---------------------------------------------------------------- clients
// One card selected at a time: the `filled` relief + `aria-pressed`; the
// others go back to `outlined`. (`m3e-card` has no `selected` attribute.)
function selectCard(container: HTMLElement, card: HTMLElement): void {
  for (const sib of container.querySelectorAll("m3e-card")) {
    sib.setAttribute("variant", "outlined");
    sib.setAttribute("aria-pressed", "false");
  }
  card.setAttribute("variant", "filled");
  card.setAttribute("aria-pressed", "true");
}

async function refreshClients(): Promise<void> {
  const list = document.getElementById("client-list")!;
  if (list.childElementCount === 0) {
    const progress = document.createElement(
      "m3e-linear-progress-indicator",
    );
    progress.setAttribute("indeterminate", "");
    list.append(progress);
  }
  try {
    clients = await invoke<FoundClient[]>("detect_clients");
  } catch (err) {
    const msg = `invoke detect_clients failed: ${String(err)}`;
    snack(msg);
    list.innerHTML = `<m3e-callout variant="warning">${esc(msg)}</m3e-callout>`;
    return;
  }
  list.innerHTML = "";
  document.getElementById("no-clients")!.hidden = clients.length !== 0;
  for (const c of clients) {
    const card = document.createElement("m3e-card");
    card.className = "selectable";
    card.setAttribute("actionable", "");
    card.setAttribute("role", "button");
    const isSel = selectedClient?.node_path === c.node_path;
    card.setAttribute("variant", isSel ? "filled" : "outlined");
    card.setAttribute("aria-pressed", String(isSel));
    card.innerHTML = `
      <div class="client-row">
        ${clientIcon(c.kind)}
        <div class="client-text">
          <span class="overline cell" title="${esc(c.version_dir)}">${esc(c.version_dir)}</span>
          <span class="cell">${esc(c.label)}</span>
          <span class="cell path" title="${esc(c.node_path)}">${esc(installRoot(c.node_path))}</span>
        </div>
      </div>`;
    card.addEventListener("click", () => {
      selectedClient = c;
      selectCard(list, card);
      analysis = null;
      (document.getElementById("btn-to-goal") as HTMLButtonElement).disabled = false;
      updateApplyButtons();
    });
    list.append(card);
  }
}

// ---------------------------------------------------------------- goals
async function loadGoals(): Promise<void> {
  try {
    goals = await invoke<GoalInfo[]>("goals");
  } catch (err) {
    snack(`Error loading goals: ${String(err)}`);
    return;
  }
  const grid = document.getElementById("goal-grid")!;
  grid.innerHTML = "";
  for (const g of goals) {
    const card = document.createElement("m3e-card");
    card.className = "selectable";
    card.setAttribute("actionable", "");
    card.setAttribute("role", "button");
    const isSel = g.id === selectedGoal;
    card.setAttribute("variant", isSel ? "filled" : "outlined");
    card.setAttribute("aria-pressed", String(isSel));
    card.innerHTML = `
      <div class="goal-body">
        <strong>${esc(loc(g.title))}</strong>
        <div class="muted">${esc(loc(g.description))}</div>
        <div><m3e-chip variant="assist">${g.sites} ${g.sites === 1 ? "patch" : "patches"}</m3e-chip></div>
      </div>`;
    card.addEventListener("click", () => {
      selectedGoal = g.id;
      selectCard(grid, card);
      analysis = null;
      updateApplyButtons();
    });
    grid.append(card);
  }
}

// ---------------------------------------------------------------- apply
function renderApply(): void {
  const summary = document.getElementById("apply-summary")!;
  if (!selectedClient) {
    summary.innerHTML =
      '<m3e-callout variant="warning">Pick a client on the Clients tab first.</m3e-callout>';
  } else {
    const goal = goals.find((g) => g.id === selectedGoal);
    summary.innerHTML = `
      <m3e-callout variant="info">
        <div class="row-between">
          <span><strong>${esc(goal ? loc(goal.title) : selectedGoal)}</strong> on ${esc(selectedClient.label)}</span>
          <span class="muted path">${esc(selectedClient.version_dir)}</span>
        </div>
      </m3e-callout>`;
  }
  const sites = document.getElementById("site-list")!;
  sites.innerHTML = "";
  if (analysis) {
    for (const [name, st] of analysis.sites) {
      const li = document.createElement("m3e-list-item");
      li.innerHTML = `
        <span slot="leading" class="state-${st.state}">${stateIcon(st)}</span>
        <span class="cell" title="${esc(name)}">${esc(name)}</span>
        <span slot="supporting-text">${esc(stateText(st))}</span>`;
      sites.append(li);
    }
  } else {
    const li = document.createElement("m3e-list-item");
    li.innerHTML = selectedClient
      ? `<span class="muted">Run Analyze to see each site's state before applying.</span>`
      : `<span class="muted">No client selected. Go back to the Clients tab and pick one.</span>`;
    sites.append(li);
  }
  updateApplyButtons();
}

function stateIcon(st: SiteState): string {
  if (st.state === "ok") return "○";
  if (st.state === "patched") return "●";
  return "✕";
}

function stateText(st: SiteState): string {
  if (st.state === "ok") return "ready to patch";
  if (st.state === "patched") return "already applied";
  return `diverged from expected (${st.found}); the patch aborts before writing`;
}

function updateApplyButtons(): void {
  const sites = analysis?.sites ?? [];
  const hasMismatch = sites.some(([, st]) => st.state === "mismatch");
  const allPatched =
    sites.length > 0 && sites.every(([, st]) => st.state === "patched");
  const reportable =
    analysis !== null && (hasMismatch || !analysis.known_pristine);
  (document.getElementById("btn-analyze") as HTMLButtonElement).disabled =
    !selectedClient;
  (document.getElementById("btn-revert") as HTMLButtonElement).disabled =
    !selectedClient;
  (document.getElementById("btn-apply") as HTMLButtonElement).disabled =
    !selectedClient || analysis === null || hasMismatch || allPatched;
  document.getElementById("btn-report")!.hidden = !reportable;
}

async function doAnalyze(): Promise<void> {
  if (!selectedClient) return;
  try {
    analysis = await invoke<Analysis>("analyze_node", {
      node: selectedClient.node_path,
      goal: selectedGoal,
    });
    document.getElementById("discord-running")!.hidden =
      !selectedClient.process_running;
    const patched = analysis.sites.filter(
      ([, s]) => s.state === "patched",
    ).length;
    snack(`Analysis: ${patched}/${analysis.sites.length} patches already applied`);
  } catch (err) {
    snack(`Error: ${String(err)}`);
  }
  renderApply();
}

interface ReportOutcome {
  path: string;
  text: string;
}

async function doReport(): Promise<void> {
  if (!selectedClient || !analysis) return;
  const expected = new Map(analysis.expected ?? []);
  const sites: Array<[string, string, string]> = analysis.sites.map(
    ([name, st]) => [
      name,
      expected.get(name) ?? "",
      st.state === "mismatch" ? st.found : "",
    ],
  );
  const btn = document.getElementById("btn-report") as HTMLButtonElement;
  btn.disabled = true;
  try {
    const outcome = await invoke<ReportOutcome>("report_divergence", {
      node: selectedClient.node_path,
      goal: selectedGoal,
      sites,
      knownPristine: analysis.known_pristine,
      clientKind: selectedClient.kind,
    });
    await navigator.clipboard.writeText(outcome.text);
    snack(`Report copied & saved: ${outcome.path}`);
  } catch (err) {
    snack(`Report failed: ${String(err)}`);
  }
  btn.disabled = false;
}

async function doApply(): Promise<void> {
  if (!selectedClient) return;
  if (selectedClient.process_running) {
    snack("Close Discord before applying.");
    return;
  }
  try {
    const result = await invoke<Analysis>("apply_goal", {
      node: selectedClient.node_path,
      goal: selectedGoal,
    });
    analysis = result;
    snack(`Applied: ${result.sites.length} patches. Reopen Discord.`);
    // Subtle confetti on success; only when animations are on.
    if (animPref) {
      void confetti({ particleCount: 70, spread: 75, origin: { y: 0.6 } });
      for (const c of document.querySelectorAll("canvas")) {
        c.setAttribute("aria-hidden", "true");
      }
      setTimeout(() => {
        void confetti({ particleCount: 40, angle: 60, spread: 60, origin: { x: 0 } });
        void confetti({ particleCount: 40, angle: 120, spread: 60, origin: { x: 1 } });
      }, 250);
    }
    (document.getElementById("done-dialog") as HTMLElement & { open: boolean }).open = true;
  } catch (err) {
    snack(`Error applying: ${String(err)}`);
  }
  renderApply();
}

async function doRevert(): Promise<void> {
  if (!selectedClient) return;
  try {
    const bak = await invoke<string>("revert_node", {
      node: selectedClient.node_path,
    });
    analysis = null;
    snack(`Restored from ${bak}`);
  } catch (err) {
    snack(`Error reverting: ${String(err)}`);
  }
  renderApply();
}

// ---------------------------------------------------------------- about
// Cards of the Info page; same block saved at
// dev/backup/2026-10-16-stepper-sem-info/sobre-cards.md.
const LINKS: Array<{ title: string; desc: string; url: string; aria: string; svg: string }> = [
  {
    title: "GitHub",
    desc: "Kit source code, issues and releases.",
    url: "https://github.com/sittiev/discord_stereo",
    aria: "Open the project's GitHub",
    svg: `<svg viewBox="0 0 24 24" width="24" height="24" fill="currentColor" aria-hidden="true"><path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.11.79-.25.79-.55 0-.27-.01-1.17-.02-2.12-3.2.7-3.88-1.36-3.88-1.36-.52-1.33-1.28-1.68-1.28-1.68-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.19 1.76 1.19 1.03 1.75 2.69 1.25 3.34.95.1-.74.4-1.25.72-1.53-2.55-.29-5.24-1.28-5.24-5.69 0-1.26.45-2.28 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.18 1.18a11.1 11.1 0 0 1 5.8 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.83 1.19 3.09 0 4.42-2.7 5.39-5.27 5.68.41.36.78 1.05.78 2.12 0 1.53-.01 2.77-.01 3.15 0 .3.2.66.8.55A11.52 11.52 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5Z"/></svg>`,
  },
  {
    title: "Discord server",
    desc: "Support, discussion and offsets for new versions.",
    url: "https://discord.gg/jb72VayAzN",
    aria: "Open the Discord server",
    svg: `<svg viewBox="0 0 24 24" width="24" height="24" fill="#5865F2" aria-hidden="true"><path d="M20.32 4.37a19.8 19.8 0 0 0-4.89-1.52.07.07 0 0 0-.08.04c-.21.38-.44.87-.61 1.25a18.27 18.27 0 0 0-5.48 0 12.6 12.6 0 0 0-.62-1.25.08.08 0 0 0-.08-.04 19.74 19.74 0 0 0-4.88 1.52.07.07 0 0 0-.04.03C.53 9.05-.32 13.58.1 18.06c0 .03.02.05.04.07a19.9 19.9 0 0 0 5.99 3.03.08.08 0 0 0 .08-.03c.46-.63.87-1.29 1.22-1.99.02-.04 0-.08-.04-.09a13.1 13.1 0 0 1-1.87-.89.08.08 0 0 1 .01-.13c.13-.09.25-.19.37-.29a.07.07 0 0 1 .08 0 14.2 14.2 0 0 0 12.06 0 .07.07 0 0 1 .08 0c.12.1.24.2.37.29a.08.08 0 0 1 .01.13c-.6.35-1.22.65-1.87.89a.08.08 0 0 0-.04.09c.36.7.77 1.36 1.22 1.99a.08.08 0 0 0 .08.03 19.84 19.84 0 0 0 6-3.03.07.07 0 0 0 .04-.07c.5-5.18-.84-9.68-3.55-13.66a.06.06 0 0 0-.03-.03ZM8.02 15.33c-1.18 0-2.16-1.08-2.16-2.42 0-1.33.96-2.42 2.16-2.42 1.21 0 2.18 1.1 2.16 2.42 0 1.34-.96 2.42-2.16 2.42Zm7.98 0c-1.18 0-2.15-1.08-2.15-2.42 0-1.33.95-2.42 2.15-2.42 1.21 0 2.18 1.1 2.16 2.42 0 1.34-.95 2.42-2.16 2.42Z"/></svg>`,
  },
  {
    title: "Donate",
    desc: "Support development: Ko-fi, Pix or Sponsor.",
    url: "https://ko-fi.com/SEU-PERFIL",
    aria: "Open the donation page",
    svg: `<svg viewBox="0 0 24 24" width="24" height="24" fill="currentColor" aria-hidden="true"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35Z"/></svg>`,
  },
];

function renderAbout(): void {
  const box = document.getElementById("about-cards")!;
  if (box.childElementCount > 0) return;
  for (const link of LINKS) {
    const card = document.createElement("m3e-card");
    card.setAttribute("variant", "outlined");
    card.innerHTML = `
      <div class="about-body row-between">
        <div>
          <strong>${esc(link.title)}</strong>
          <div class="muted">${esc(link.desc)}</div>
        </div>
        <m3e-icon-button aria-label="${esc(link.aria)}" data-url="${esc(link.url)}">${link.svg}</m3e-icon-button>
      </div>`;
    card.querySelector("m3e-icon-button")!.addEventListener("click", (e) => {
      const url = (e.currentTarget as HTMLElement).dataset.url ?? "";
      openExternal(url);
    });
    box.append(card);
  }
}

// ---------------------------------------------------------------- misc
function snack(msg: string): void {
  const sb = document.getElementById("snackbar") as HTMLElement & { open: boolean };
  sb.textContent = msg;
  sb.open = true;
}

function openExternal(url: string): void {
  if (!url) return;
  void openUrl(url).catch((err: unknown) => snack(`Couldn't open: ${String(err)}`));
}

async function loadSysInfo(): Promise<void> {
  let info: SystemInfo;
  try {
    info = await invoke<SystemInfo>("system_info");
  } catch (err) {
    snack(`Error loading info: ${String(err)}`);
    return;
  }
  const box = document.getElementById("sysinfo")!;
  box.innerHTML = "";
  const rows: Array<[string, string]> = [
    ["OS", info.os],
    ["Architecture", info.arch],
    ["Original binary MD5", info.known_pristine_md5],
  ];
  for (const [label, value] of rows) {
    const chip = document.createElement("m3e-chip");
    chip.setAttribute("variant", "assist");
    chip.textContent = `${label}: ${value}`;
    box.append(chip);
  }
}

// ---------------------------------------------------------------- boot
window.addEventListener("DOMContentLoaded", () => {
  document.getElementById("btn-analyze")!.addEventListener("click", doAnalyze);
  document.getElementById("btn-apply")!.addEventListener("click", doApply);
  document.getElementById("btn-revert")!.addEventListener("click", doRevert);
  document.getElementById("btn-report")!.addEventListener("click", doReport);
  document.getElementById("btn-to-goal")!.addEventListener("click", () => {
    if (!selectedClient) {
      snack("Pick a client first.");
      return;
    }
    gotoStep("s-goal");
  });
  for (const b of document.querySelectorAll<HTMLElement>("[data-step]")) {
    b.addEventListener("click", () => {
      const id = b.dataset.step!;
      if (id === "s-goal" && !selectedClient) {
        snack("Pick a client first.");
        return;
      }
      gotoStep(id);
    });
  }
  // Titlebar is drag-only (decorations off, no window controls).
  // Footer and popup open external links; one delegate covers all.
  for (const b of document.querySelectorAll<HTMLElement>("[data-url]")) {
    b.addEventListener("click", () => openExternal(b.dataset.url ?? ""));
  }
  applyThemePref();
  applyAnimPref();
  syncThemeCtl();
  const themeCtl = document.getElementById("pref-theme")!;
  for (const seg of themeCtl.querySelectorAll<HTMLElement & { checked: boolean }>(
    "m3e-button-segment",
  )) {
    seg.addEventListener("change", () => {
      if (!seg.checked) return;
      const v = seg.getAttribute("value");
      if (isThemePref(v)) setThemePref(v);
    });
  }
  document.getElementById("btn-theme")!.addEventListener("click", () => {
    setThemePref(nextTheme(themePref));
  });
  const animCtl = document.getElementById("pref-anim") as HTMLElement & {
    checked: boolean;
  };
  animCtl.checked = animPref;
  animCtl.addEventListener("change", () => {
    animPref = animCtl.checked;
    storeSet(PREF_ANIM, animPref ? "on" : "off");
    applyAnimPref();
  });
  void loadGoals();
  void loadSysInfo();
  gotoStep("s-clients");
});
