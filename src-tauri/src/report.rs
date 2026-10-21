//! Divergence reports: when Analyze finds no compatible table or mismatching
//! sites, a machine-readable trace is produced so offsets for new builds can
//! be rebuilt quickly. Three sinks, best-effort and never blocking the UI:
//!
//! 1. Local file under app-data `reports/` (always).
//! 2. Discord webhook (when `STEREO_WEBHOOK` is set at build time or runtime).
//! 3. GitHub issue via token (when `STEREO_GH_TOKEN` is set).
//!
//! Trace format: one JSON object per report - what, where, which goal,
//! per-site evidence (expected vs found bytes), binary identity and a small
//! fixed-size digest of surrounding bytes so new offsets can be located by
//! signature. No user identifiers are collected.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Webhook endpoint comes from the environment only (`STEREO_WEBHOOK`);
/// it is a secret and must never be compiled into the public binary.
const WEBHOOK_URL: Option<&str> = option_env!("STEREO_WEBHOOK");
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
    /// Full localized goal name ("Deep Stereo" / "Stereo Profundo").
    pub goal_title: String,
    /// UI language when the report was sent ("pt" / "en").
    pub lang: String,
    pub node_path: String,
    pub node_md5: String,
    pub node_size: u64,
    pub discord_dir: String,
    /// Flavor of the reported client (stable/ptb/canary/vencord).
    pub client_kind: String,
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
    /// clipboard copy. Redacts the home directory - anything that leaves
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


/// OS display name for the content line.
fn os_name(os: &str) -> &'static str {
    match os {
        "windows" => "Windows",
        "macos" => "macOS",
        _ => "Linux",
    }
}

/// Icon of the reported client flavor - Discord, Canary, PTB or Vencord -
/// via icon.horse (serves real PNGs, no proxy 301 games).
fn client_icon(kind: &str) -> String {
    let domain = match kind {
        "canary" => "canary.discord.com",
        "ptb" => "ptb.discord.com",
        "vencord" => "vencord.dev",
        _ => "discord.com",
    };
    format!("https://icon.horse/icon/{domain}")
}

/// Discord brand color (Blurple).
const EMBED_COLOR: u32 = 0x5865F2;

fn field(name: &str, value: String, inline: bool) -> serde_json::Value {
    serde_json::json!({"name": name, "value": value, "inline": inline})
}

/// Bot identity of the reporting webhook, from the user's template.
const BOT_AVATAR: &str = "https://encrypted-tbn0.gstatic.com/images?q=tbn:ANd9GcQ1El38iW3yk3YIvYakO7ZJMcPc_4l_OTLyQBDflIpzJg&s=10";

/// Pure payload builder - everything testable without touching the
/// network. `content` one-liner (OS, md5, full goal name, client flavor);
/// single embed whose author carries the icon of the reported client and
/// one field per diverged site named after the site's own function, so a
/// reverser or an LLM reads what to change directly from the field list.
#[must_use]
fn embed_payload(report: &Report) -> serde_json::Value {
    let diverged: Vec<&SiteTrace> = report.sites.iter().filter(|s| !s.found.is_empty()).collect();
    let flavor = match report.client_kind.as_str() {
        "canary" => "Discord Canary",
        "ptb" => "Discord PTB",
        "vencord" => "Vencord Desktop",
        _ => "Discord Stable",
    };
    let icon = client_icon(&report.client_kind);
    let os = os_name(&report.os);

    let mut fields = vec![
        field("MD5", format!("`{}`", report.node_md5), false),
        field(
            "Binary",
            format!("{} · {} · `{}` · {}", flavor, report.discord_dir, report.arch,
                    if report.table_kind == "none" { "no patch table for this build" } else { "patch table matched" }),
            false,
        ),
        field(
            "Patch",
            format!("{} (id: {})", report.goal_title, report.goal),
            false,
        ),
    ];
    for (i, s) in diverged.iter().enumerate() {
        if i == 8 {
            fields.push(field(
                "… more sites",
                format!("… +{} more diverged sites - full evidence in the JSON report", diverged.len() - 8),
                false,
            ));
            break;
        }
        let mut body = format!("✗ expected `{}`\n→ found `{}`", s.expected_orig, s.found);
        if let Some(off) = s.offset {
            body = format!("`0x{off:X}` · {body}");
        }
        if let Some(n) = &s.neighborhood {
            body.push_str(&format!("\n±16 `{n}`"));
        }
        fields.push(field(&truncate(&s.name, 256), body, false));
    }
    if diverged.is_empty() && !report.sites.is_empty() {
        fields.push(field("Result", format!("✓ all {} sites matched", report.sites.len()), false));
    }

    let title = if report.sites.is_empty() {
        "📦 Binary not in the patch db".to_owned()
    } else if diverged.is_empty() {
        format!("✓ {flavor} on {os} · all {} sites matched", report.sites.len())
    } else {
        format!("🛠 {flavor} on {os} · {} of {} sites diverged", diverged.len(), report.sites.len())
    };

    serde_json::json!({
        "username": "Stereo Wizard Report",
        "avatar_url": BOT_AVATAR,
        "content": format!(
            "OS: {os} ({}) - MD5: `{}` - patch: {} - {}",
            report.arch, report.node_md5, report.goal_title, flavor
        ),
        "embeds": [{
            "title": title,
            "color": EMBED_COLOR,
            "author": {"name": format!("{flavor} · site evidence"), "icon_url": icon},
            "fields": fields,
            "footer": {"text": format!(
                "`{}` · patchdb v{} (offset tables) · app v{}",
                report.report_id, report.db_version, report.app_version
            )},
            "timestamp": report.ts_iso,
        }],
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

/// Clamp to `max` **including** the truncation marker, on a char boundary -
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
            goal_title: "Stereo Full".into(),
            lang: "en".into(),
            node_path: "/x/discord_voice.node".into(),
            node_md5: "ab".repeat(16),
            node_size: 123,
            discord_dir: "app-1.0.162".into(),
            client_kind: "stable".into(),
            db_version: 2,
            table_kind: "none".into(),
            sites: vec![SiteTrace {
                name: "MCOpusImpl ctor num_channels".into(),
                expected_orig: "02".into(),
                expected_patch: "00".into(),
                found: "81".into(),
                neighborhood: Some("ff".repeat(16)),
                offset: Some(0x10),
            }],
        }
    }

    #[test]
    fn payload_matches_user_template() {
        let p = embed_payload(&sample());
        assert_eq!(p["username"], "Stereo Wizard Report");
        assert!(p["avatar_url"].as_str().unwrap().starts_with("https://"));
        let content = p["content"].as_str().unwrap();
        assert!(content.starts_with("OS: Linux (x86_64) - MD5: `"), "got {content}");
        assert!(content.ends_with("- patch: Stereo Full - Discord Stable"), "goal/flavor missing: {content}");
        let e = &p["embeds"][0];
        assert_eq!(e["color"], 0x5865F2);
        assert_eq!(e["author"]["name"], "Discord Stable · site evidence");
        assert_eq!(e["author"]["icon_url"], "https://icon.horse/icon/discord.com");
        assert_eq!(e["title"], "🛠 Discord Stable on Linux · 1 of 1 sites diverged");
        let fields = e["fields"].as_array().unwrap();
        assert_eq!(fields[0]["name"], "MD5");
        assert_eq!(fields[1]["name"], "Binary");
        assert!(fields[1]["value"].as_str().unwrap().contains("no patch table for this build"));
        assert_eq!(fields[2]["name"], "Patch");
        assert_eq!(fields[2]["value"], "Stereo Full (id: v4)");
        assert_eq!(fields[3]["name"], "MCOpusImpl ctor num_channels",
            "field must be the site function, not a generic PATCH label");
        assert!(!fields[3]["inline"].as_bool().unwrap(), "site fields stack full-width");
        assert!(fields[3]["value"].as_str().unwrap().contains("✗ expected"));
        let footer = e["footer"]["text"].as_str().unwrap();
        assert!(footer.contains("patchdb v2 (offset tables)"), "db version must be self-explaining: {footer}");
        assert_eq!(p["allowed_mentions"]["parse"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn site_fields_dynamic_per_diverged_site() {
        let mut rep = sample();
        rep.sites = (0..12)
            .map(|i| SiteTrace {
                name: format!("fn_{i}"),
                expected_orig: "02".into(),
                expected_patch: "00".into(),
                found: "81".into(),
                neighborhood: Some("ff".repeat(16)),
                offset: Some(i),
            })
            .collect();
        let fields = embed_payload(&rep)["embeds"][0]["fields"].as_array().unwrap().clone();
        assert_eq!(fields[3]["name"], "fn_0");
        assert_eq!(fields[10]["name"], "fn_7", "cap at 8 site fields");
        assert_eq!(fields[11]["name"], "… more sites", "overflow field expected");
        assert!(fields[11]["value"].as_str().unwrap().contains("+4 more"));
    }

    #[test]
    fn clean_report_ships_result_field() {
        let mut rep = sample();
        rep.sites = vec![SiteTrace {
            name: "only".into(),
            expected_orig: "02".into(),
            expected_patch: "00".into(),
            found: String::new(),
            neighborhood: None,
            offset: None,
        }];
        let e = &embed_payload(&rep)["embeds"][0];
        assert_eq!(e["title"], "✓ Discord Stable on Linux · all 1 sites matched");
        let fields = e["fields"].as_array().unwrap();
        assert_eq!(fields[3]["name"], "Result");
        assert!(fields[3]["value"].as_str().unwrap().contains("✓ all 1 sites matched"));
    }

    #[test]
    fn no_table_report_titles_and_content() {
        let mut rep = sample();
        rep.sites.clear();
        let p = embed_payload(&rep);
        assert_eq!(p["embeds"][0]["title"], "📦 Binary not in the patch db");
        let fields = p["embeds"][0]["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 3, "MD5 + Binary + Patch only, no site fields");
    }

    #[test]
    fn client_flavor_icons() {
        let mut rep = sample();
        rep.client_kind = "canary".into();
        let e = &embed_payload(&rep)["embeds"][0];
        assert_eq!(e["author"]["icon_url"], "https://icon.horse/icon/canary.discord.com");
        assert!(e["title"].as_str().unwrap().contains("Discord Canary"));
        rep.client_kind = "ptb".into();
        let e = &embed_payload(&rep)["embeds"][0];
        assert!(e["author"]["icon_url"].as_str().unwrap().contains("ptb.discord.com"));
        rep.client_kind = "vencord".into();
        let e = &embed_payload(&rep)["embeds"][0];
        assert!(e["author"]["icon_url"].as_str().unwrap().contains("vencord.dev"));
        assert!(e["title"].as_str().unwrap().contains("Vencord Desktop"));
    }

    #[test]
    fn goal_title_localizes_in_content() {
        let mut rep = sample();
        rep.goal_title = "Stereo Profundo".into();
        rep.lang = "pt".into();
        let p = embed_payload(&rep);
        assert!(p["content"].as_str().unwrap().contains("- patch: Stereo Profundo -"),
            "full localized goal name must appear");
        let fields = p["embeds"][0]["fields"].as_array().unwrap();
        assert_eq!(fields[2]["value"], "Stereo Profundo (id: v4)");
    }


    #[test]
    fn os_name_in_title_and_content() {
        let mut rep = sample();
        rep.os = "windows".into();
        let p = embed_payload(&rep);
        assert!(p["embeds"][0]["title"].as_str().unwrap().contains("on Windows"));
        assert!(p["content"].as_str().unwrap().starts_with("OS: Windows"));
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
