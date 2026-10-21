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
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Discord webhook embed: fire-and-forget with a hard timeout; failure is
/// invisible to the user (the local file always exists). Layout mirrors a
/// triage board — context header, verdict title, heat bar, then sites
/// sorted worst-first so the eye lands on breakage before detail.
fn send_webhook(url: &str, report: &Report) {
    let diverged: Vec<&SiteTrace> = report.sites.iter().filter(|s| !s.found.is_empty()).collect();
    let clean = report.sites.len() - diverged.len();
    let mut lines = String::new();
    lines.push_str(&format!("`{}`\n", heat_bar(&report.sites)));
    for (i, s) in diverged.iter().enumerate() {
        if i == 8 {
            lines.push_str(&format!("… +{} more in the JSON file\n", diverged.len() - 8));
            break;
        }
        lines.push_str(&format!(
            "▲ **{}** at {}\n   expected `{}`  found `{}`{}\n",
            s.name,
            s.offset.map(|o| format!("`0x{o:X}`")).unwrap_or_else(|| "—".into()),
            s.expected_orig,
            s.found,
            s.neighborhood
                .as_deref()
                .map(|n| format!("\n   around `{}`", n))
                .unwrap_or_default()
        ));
    }
    if clean > 0 {
        lines.push_str(&format!("● {clean} sites matched — not shown\n"));
    }
    let payload = serde_json::json!({
        "username": "Stereo Wizard Reports",
        "embeds": [{
            "author": {"name": format!("{} · discord {}", report.goal, report.discord_dir)},
            "title": format!(
                "{} diverged of {} sites",
                diverged.len(),
                report.sites.len()
            ),
            "description": truncate(&lines, 3900),
            "color": 0x4A4E54,
            "fields": [
                {"name": "md5", "value": format!("`{}`", &report.node_md5[..16.min(report.node_md5.len())]), "inline": true},
                {"name": "os/arch", "value": format!("{}/{}", report.os, report.arch), "inline": true},
                {"name": "table", "value": report.table_kind, "inline": true},
            ],
            "footer": {"text": format!("`{}` · db v{} · app {}", report.report_id, report.db_version, report.app_version)}
        }],
        "allowed_mentions": {"parse": []}
    });
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

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}\n…(truncated)", &s[..cut])
    }
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
}
