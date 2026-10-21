//! Divergence reports: when Analyze finds no compatible table or mismatching
//! sites, a machine-readable trace is produced so offsets for new builds can
//! be rebuilt quickly. Three sinks, best-effort and never blocking the UI:
//!
//! 1. Local file under app-data `reports/` (always).
//! 2. Discord webhook (when `STEREO_WEBHOOK` is set at build time or runtime).
//! 3. GitHub issue via token (when `STEREO_GH_TOKEN` is set).
//!
//! Trace format: one JSON object per report — what, where, which goal,
//! per-site evidence (expected vs found bytes), binary identity and a small
//! fixed-size digest of surrounding bytes so new offsets can be located by
//! signature. No user identifiers are collected.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Webhook default (report channel); runtime env `STEREO_WEBHOOK` overrides.
const WEBHOOK_URL: Option<&str> = match option_env!("STEREO_WEBHOOK") {
    Some(url) => Some(url),
    None => Some(
        "ENV_ONLY_STEREO_WEBHOOK",
    ),
};
const GH_REPO: &str = "sittiev/discord_stereo";


#[derive(Serialize)]
pub struct SiteTrace {
    pub name: String,
    pub expected_orig: String,
    pub expected_patch: String,
    pub found: String,
    /// Hex of `[offset - 16, offset + 16)` around the site, when known.
    pub neighborhood: Option<String>,
    pub offset: Option<u32>,
}

#[derive(Serialize)]
pub struct Report {
    pub app_version: String,
    pub schema: u32,
    pub ts_unix: u64,
    /// RFC 3339 local-offset timestamp (logs need a timezone, not just epoch).
    pub ts_iso: String,
    /// Stable dedupe key across sinks: `{md5}-{ts}`.
    pub report_id: String,
    pub os: String,
    pub arch: String,
    pub goal: String,
    pub node_path: String,
    pub node_md5: String,
    pub node_size: u64,
    pub discord_dir: String,
    pub db_version: u32,
    pub table_kind: String,
    pub sites: Vec<SiteTrace>,
}

impl Report {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// One-line problem statement, first line of every rendering.
    pub fn summary(&self) -> String {
        format!(
            "{}: {} of {} sites diverged on {}/{} ({})",
            self.goal,
            self.sites.iter().filter(|s| !s.found.is_empty()).count(),
            self.sites.len(),
            self.os,
            self.arch,
            self.discord_dir
        )
    }

    /// Compact human-readable rendering for webhook embeds and the
    /// clipboard copy. Redacts the home directory — anything that leaves
    /// the machine must not carry the username.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("{}\n", self.summary()));
        s.push_str(&format!(
            "app={} report={} at {}\n",
            self.app_version, self.report_id, self.ts_iso
        ));
        s.push_str(&format!(
            "goal={} os={}/{} discord={} md5={} size={} db=v{}\n",
            self.goal, self.os, self.arch, redact_home(&self.discord_dir),
            self.node_md5, self.node_size, self.db_version
        ));
        s.push_str(&format!(
            "node: {}\n",
            redact_home(&self.node_path)
        ));
        for site in &self.sites {
            s.push_str(&format!(
                "- {}\n  expected {}\n  found    {}\n",
                site.name, site.expected_orig, site.found
            ));
            if let Some(n) = &site.neighborhood {
                s.push_str(&format!("  around   {n}\n"));
            }
        }
        s
    }
}

/// Mask the OS username in paths: `/home/<user>/x` → `~/x`. Discord dirs
/// like `app-1.0.161` pass through unchanged.
fn redact_home(s: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.len() > 1 && s.starts_with(&home) {
        return format!("~{}", &s[home.len()..]);
    }
    if let Some(rest) = s.strip_prefix("C:\\Users\\") {
        let mut it = rest.splitn(2, '\\');
        let _user = it.next();
        return format!("~\\{}", it.next().unwrap_or_default());
    }
    s.to_owned()
}


/// Epoch seconds to "YYYY-MM-DDTHH:MM:SSZ" (UTC). Days-from-epoch to
/// civil date per Howard Hinnant's `civil_from_days`; valid for all u64.
pub fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Persist the report and fire the network sinks. Returns the path of the
/// local file so the UI can offer a copy/open action. Never panics: every
/// sink is best-effort and failures are folded into `Err` only for the
/// local write, which is the one sink the user directly sees.
pub fn submit(report: &Report, reports_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(reports_dir).map_err(|e| e.to_string())?;
    let name = format!("div-{}.json", report.report_id);
    let path = reports_dir.join(name);
    fs::write(&path, report.to_json()).map_err(|e| e.to_string())?;

    if let Some(url) = webhook_url() {
        send_webhook(&url, report);
    }
    if let Ok(token) = std::env::var("STEREO_GH_TOKEN") {
        send_github_issue(&token, report);
    }
    Ok(path)
}

fn webhook_url() -> Option<String> {
    std::env::var("STEREO_WEBHOOK")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| WEBHOOK_URL.map(str::to_owned))
        .filter(|s| s.starts_with("https://"))
}

/// Heat bar: one char per site — solid for diverged, dot for clean. Gives
/// a size-and-density read before any detail line.
fn heat_bar(sites: &[SiteTrace]) -> String {
    let mut bar = String::new();
    for (i, s) in sites.iter().enumerate() {
        if i % 40 == 0 && i > 0 {
            bar.push('\n');
        }
        bar.push(if s.found.is_empty() { '·' } else { '█' });
    }
    bar
}

/// Zero-width space — Discord rejects empty field names, but renders ZWSP
/// as one; the Discohook trick for invisible full-width section dividers.
const ZW: &str = "\u{200B}";

/// Discord brand color (Blurple).
const EMBED_COLOR: u32 = 0x5865F2;

/// Overview embed body: the heat bar only — facts live in the field grid.
fn overview_description(report: &Report) -> String {
    if report.sites.is_empty() {
        return "No compatible table for this binary — offsets unknown. \
Send the report so the table can be rebuilt."
            .into();
    }
    format!("`{}`", heat_bar(&report.sites))
}

/// Evidence embed body: one block per diverged site, worst-first, blocks
/// separated by `_ _` (Discord's blank line). Capped at 8 blocks.
fn evidence_description(report: &Report) -> String {
    let diverged: Vec<&SiteTrace> = report.sites.iter().filter(|s| !s.found.is_empty()).collect();
    let mut out = String::new();
    for (i, s) in diverged.iter().enumerate() {
        if i == 8 {
            out.push_str(&format!("… +{} more in the JSON report", diverged.len() - 8));
            break;
        }
        if i > 0 {
            out.push_str("\n_ _\n");
        }
        out.push_str(&format!(
            "▸ **{}** — {}\n  ✗ expected `{}`\n  found `{}`",
            s.name,
            s.offset.map(|o| format!("`0x{o:X}`")).unwrap_or_else(|| "—".into()),
            s.expected_orig,
            s.found,
        ));
        if let Some(n) = &s.neighborhood {
            out.push_str(&format!("\n  around `{n}`"));
        }
    }
    out
}

fn field(name: &str, value: String) -> serde_json::Value {
    serde_json::json!({"name": name, "value": value, "inline": true})
}

fn divider() -> serde_json::Value {
    serde_json::json!({"name": ZW, "value": ZW})
}

/// Pure payload builder — everything testable without touching the
/// network. Two embeds, Discohook-style: an overview card (verdict,
/// heat bar, fact grid with invisible section dividers) plus an evidence
/// card only when something diverged. `send_webhook` is the thin I/O shell.
#[must_use]
fn embed_payload(report: &Report) -> serde_json::Value {
    let diverged = report.sites.iter().filter(|s| !s.found.is_empty()).count();
    let clean = report.sites.len() - diverged;
    let title = if report.sites.is_empty() {
        "📦 Binary not in the patch db".to_owned()
    } else {
        format!("🛠 {diverged} of {} sites diverged", report.sites.len())
    };
    let overview = serde_json::json!({
        "author": {"name": "Stereo Wizard — Divergence Report"},
        "title": title,
        "description": truncate(&overview_description(report), 3900),
        "color": EMBED_COLOR,
        "fields": [
            divider(),
            field("Build", report.discord_dir.clone()),
            field("Goal", report.goal.clone()),
            field("Table", report.table_kind.clone()),
            divider(),
            field("md5", format!("`{}`", &report.node_md5[..16.min(report.node_md5.len())])),
            field("Platform", format!("{}/{}", report.os, report.arch)),
            field("Sites", format!("{} · {} ✗ · {} ✓", report.sites.len(), diverged, clean)),
        ],
        "footer": {"text": format!("`{}` · db v{} · app {}", report.report_id, report.db_version, report.app_version)},
        "timestamp": report.ts_iso,
    });
    let mut embeds = vec![overview];
    if diverged > 0 {
        embeds.push(serde_json::json!({
            "author": {"name": "Stereo Wizard — Site Evidence"},
            "title": "🔎 Diverged site bytes",
            "description": truncate(&evidence_description(report), 3900),
            "color": EMBED_COLOR,
            "footer": {"text": "expected vs found · neighborhood ±16 bytes"},
            "timestamp": report.ts_iso,
        }));
    }
    serde_json::json!({
        "username": "Stereo Wizard Reports",
        "embeds": embeds,
        "allowed_mentions": {"parse": []}
    })
}

/// Discord webhook embed: fire-and-forget with a hard timeout; failure is
/// invisible to the user (the local file always exists).
fn send_webhook(url: &str, report: &Report) {
    let payload = embed_payload(report);
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(8))
        .build();
    let _ = agent
        .post(url)
        .set("User-Agent", "stereo-wizard/0.1")
        .set("Content-Type", "application/json")
        .send_string(&payload.to_string());
}

fn send_github_issue(token: &str, report: &Report) {
    let title = format!("[report] {}", truncate(&report.summary(), 60));
    let body = format!(
        "```json\n{}\n```",
        truncate(&report.to_json(), 60_000)
    );
    let payload = serde_json::json!({"title": title, "body": body, "labels": ["divergence-report"]});
    let url = format!("https://api.github.com/repos/{GH_REPO}/issues");
    let auth = format!("Bearer {token}");
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build();
    let _ = agent
        .post(&url)
        .set("User-Agent", "stereo-wizard/0.1")
        .set("Authorization", &auth)
        .set("Accept", "application/vnd.github+json")
        .set("Content-Type", "application/json")
        .send_string(&payload.to_string());
}

/// Clamp to `max` **including** the truncation marker, on a char boundary —
/// `max` is the hard budget of the returned string, not just the cut point
/// (Discord counts code points, and the suffix used to overflow it).
fn truncate(s: &str, max: usize) -> String {
    const MARK: &str = "\n…(truncated)";
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let room = max.saturating_sub(MARK.chars().count());
    let cut = s
        .char_indices()
        .map(|(i, c)| i + c.len_utf8())
        .take_while(|end| *end <= room)
        .last()
        .unwrap_or(0);
    format!("{}{MARK}", &s[..cut])
}


#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Report {
        Report {
            app_version: "0.1.0".into(),
            schema: 1,
            ts_unix: 1_730_000_000,
            ts_iso: iso_utc(1_730_000_000),
            report_id: "x".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            goal: "v4".into(),
            node_path: "/x/discord_voice.node".into(),
            node_md5: "ab".repeat(16),
            node_size: 123,
            discord_dir: "app-1.0.162".into(),
            db_version: 2,
            table_kind: "none".into(),
            sites: vec![SiteTrace {
                name: "site".into(),
                expected_orig: "02".into(),
                expected_patch: "00".into(),
                found: "81".into(),
                neighborhood: Some("ff".repeat(16)),
                offset: Some(0x10),
            }],
        }
    }

    #[test]
    fn heat_bar_marks_diverged_only() {
        let mut rep = sample();
        rep.sites = (0..45)
            .map(|i| SiteTrace {
                name: format!("s{i}"),
                expected_orig: "02".into(),
                expected_patch: "00".into(),
                found: if i % 10 == 0 { "81".into() } else { String::new() },
                neighborhood: None,
                offset: Some(i),
            })
            .collect();
        let bar = heat_bar(&rep.sites);
        assert_eq!(bar.matches('█').count(), 5);
        assert_eq!(bar.matches('·').count(), 40);
        assert_eq!(bar.lines().count(), 2, "40 sites per heat row");
    }

    #[test]
    fn text_renders_sites() {
        let t = sample().to_text();
        assert!(t.contains("site"));
        assert!(t.contains("expected 02"));
    }

    #[test]
    fn summary_counts_diverging_sites() {
        assert_eq!(sample().summary(), "v4: 1 of 1 sites diverged on linux/x86_64 (app-1.0.162)");
    }

    #[test]
    fn text_redacts_home() {
        let mut rep = sample();
        rep.node_path = format!("{}/.config/discord/x.node", std::env::var("HOME").unwrap());
        let t = rep.to_text();
        assert!(t.contains("~/.config"), "username leaked: {t}");
        assert!(!t.contains(&std::env::var("HOME").unwrap()), "home leaked: {t}");
    }

    #[test]
    fn iso_known_dates() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_730_000_000), "2024-10-27T03:33:20Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso_utc(4_102_444_800), "2100-01-01T00:00:00Z");
        assert_eq!(iso_utc(2_147_483_647), "2038-01-19T03:14:07Z");
    }
    #[test]
    fn truncate_respects_boundary() {
        let s = "ààà".repeat(40);
        let cut = truncate(&s, 10);
        assert!(cut.ends_with("(truncated)"));
    }

    #[test]
    fn truncate_passes_short_strings_through() {
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn evidence_description_blocks_and_cap() {
        let mut rep = sample();
        rep.sites = (0..12)
            .map(|i| SiteTrace {
                name: format!("s{i}"),
                expected_orig: "02".into(),
                expected_patch: "00".into(),
                found: "81".into(),
                neighborhood: Some("ff".repeat(16)),
                offset: Some(i),
            })
            .collect();
        let d = evidence_description(&rep);
        assert!(d.contains("▸ **s0**"), "first diverged site missing");
        assert!(d.contains("_ _"), "block separator missing");
        assert!(d.contains("around"), "neighborhood missing");
        assert!(!d.contains("▸ **s8**"), "9th site should be capped");
        assert!(d.contains("… +4 more in the JSON report"), "overflow line missing");
    }

    #[test]
    fn overview_description_collapses_when_no_table() {
        let mut rep = sample();
        rep.sites.clear();
        let d = overview_description(&rep);
        assert!(d.contains("No compatible table"), "guidance missing: {d}");
    }

    #[test]
    fn overview_description_is_heat_bar_when_sites_exist() {
        let d = overview_description(&sample());
        assert!(d.contains("█"), "heat bar missing: {d}");
    }

    #[test]
    fn embed_payload_shape_matches_discord_contract() {
        let p = embed_payload(&sample());
        assert_eq!(p["username"], "Stereo Wizard Reports");
        let embeds = p["embeds"].as_array().unwrap();
        assert_eq!(embeds.len(), 2, "overview + evidence expected");
        let over = &embeds[0];
        assert_eq!(over["title"], "🛠 1 of 1 sites diverged");
        assert_eq!(over["author"]["name"], "Stereo Wizard — Divergence Report");
        assert_eq!(over["color"], 0x5865F2, "Discord blurple");
        let fields = over["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 8, "divider + 2 rows of 3 + divider");
        assert_eq!(fields[0]["name"], "\u{200B}", "invisible divider");
        assert!(!fields[1]["name"].as_str().unwrap().is_empty());
        assert_eq!(over["timestamp"], "2024-10-27T03:33:20Z");
        assert!(over["footer"]["text"].as_str().unwrap().contains("db v2"));
        let ev = &embeds[1];
        assert_eq!(ev["title"], "🔎 Diverged site bytes");
        assert!(ev["description"].as_str().unwrap().contains("▸"));
        assert_eq!(p["allowed_mentions"]["parse"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn embed_payload_skips_evidence_when_clean() {
        let mut rep = sample();
        rep.sites = vec![SiteTrace {
            name: "only".into(),
            expected_orig: "02".into(),
            expected_patch: "00".into(),
            found: String::new(),
            neighborhood: None,
            offset: None,
        }];
        let p = embed_payload(&rep);
        let embeds = p["embeds"].as_array().unwrap();
        assert_eq!(embeds.len(), 1, "evidence card must not ship when clean");
        assert_eq!(embeds[0]["title"], "🛠 0 of 1 sites diverged");
    }

    #[test]
    fn embed_payload_titles_no_table_case() {
        let mut rep = sample();
        rep.sites.clear();
        let p = embed_payload(&rep);
        assert_eq!(p["embeds"][0]["title"], "📦 Binary not in the patch db");
    }

    #[test]
    fn embed_payload_truncates_huge_descriptions() {
        let mut rep = sample();
        rep.sites = (0..40)
            .map(|i| SiteTrace {
                name: format!("very-long-site-name-{i}-pad-pad-pad"),
                expected_orig: "aa".repeat(64),
                expected_patch: "bb".repeat(64),
                found: "cc".repeat(64),
                neighborhood: Some("dd".repeat(128)),
                offset: Some(i),
            })
            .collect();
        let p = embed_payload(&rep);
        for e in p["embeds"].as_array().unwrap() {
            let d = e["description"].as_str().unwrap();
            assert!(d.chars().count() <= 3900, "description over Discord limit");
        }
        assert!(p["embeds"][1]["description"]
            .as_str()
            .unwrap()
            .contains("(truncated)"));
    }

    #[test]
    fn submit_names_file_after_report_id() {
        let dir = std::env::temp_dir().join("stereo-wizard-test-reports");
        let mut rep = sample();
        rep.report_id = "1730000000-abababababab".into();
        let path = submit(&rep, &dir).unwrap();
        assert!(path.ends_with("div-1730000000-abababababab.json"), "got {path:?}");
        assert!(path.exists());
        std::fs::remove_file(&path).unwrap();
    }
}
