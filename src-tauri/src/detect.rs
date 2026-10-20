//! Multi-client, multi-OS Discord installation detection.
//!
//! Finds `discord_voice.node` for every supported client flavor on
//! Linux, Windows and macOS without launching anything or touching the
//! user's session. Read-only by design.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Supported Discord client flavors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    /// Official stable channel.
    Stable,
    /// Official PTB channel.
    Ptb,
    /// Official canary channel.
    Canary,
    /// Vencord desktop app (uses official Discord directories).
    Vencord,
}

impl ClientKind {
    /// Display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ClientKind::Stable => "Discord Stable",
            ClientKind::Ptb => "Discord PTB",
            ClientKind::Canary => "Discord Canary",
            ClientKind::Vencord => "Vencord Desktop",
        }
    }

    /// All flavors scanned by the wizard.
    pub const ALL: [ClientKind; 4] = [
        ClientKind::Stable,
        ClientKind::Ptb,
        ClientKind::Canary,
        ClientKind::Vencord,
    ];
}

/// A discovered installation.
#[derive(Debug, Clone, Serialize)]
pub struct FoundClient {
    /// Display label (e.g. "Discord Stable") for the UI.
    pub label: String,
    /// Which flavor this is.
    pub kind: ClientKind,
    /// Version directory name (e.g. `app-1.0.161`), when discoverable.
    pub version_dir: String,
    /// Absolute path to `discord_voice.node`.
    pub node_path: PathBuf,
    /// Whether the host Discord process appears to be running.
    pub process_running: bool,
}

/// Scan every known install root for every flavor.
///
/// Never errors: unreadable roots are skipped, the caller renders what
/// exists. Returns installations sorted by flavor then version (newest
/// last so callers can prefer `Vec::pop`).
#[must_use]
pub fn scan_all() -> Vec<FoundClient> {
    let mut out = Vec::new();
    for kind in ClientKind::ALL {
        for root in install_roots(kind) {
            out.extend(scan_root(kind, &root));
        }
    }
    out.sort_by_key(|c| c.kind_label_key());
    out
}

impl FoundClient {
    fn kind_label_key(&self) -> (u8, String) {
        (
            match self.kind {
                ClientKind::Stable => 0,
                ClientKind::Ptb => 1,
                ClientKind::Canary => 2,
                ClientKind::Vencord => 3,
            },
            self.version_dir.clone(),
        )
    }
}

/// Home directory with fallback to the OS-reported home.
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn install_roots(kind: ClientKind) -> Vec<PathBuf> {
    let home = home();
    let cfg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData/Local"));
    let app_support = home.join("Library/Application Support");

    let dir_for = |flavor: &str| match std::env::consts::OS {
        "linux" => match flavor {
            "stable" => cfg.join("discord"),
            "ptb" => cfg.join("discordptb"),
            "canary" => cfg.join("discordcanary"),
            _ => cfg.join("Vencord"),
        },
        "windows" => match flavor {
            "stable" => local.join("Discord"),
            "ptb" => local.join("DiscordPTB"),
            "canary" => local.join("DiscordCanary"),
            _ => local.join("Vencord"),
        },
        _ => match flavor {
            "stable" => app_support.join("discord"),
            "ptb" => app_support.join("discordptb"),
            "canary" => app_support.join("discordcanary"),
            _ => app_support.join("Vencord"),
        },
    };

    let flavor = match kind {
        ClientKind::Stable => "stable",
        ClientKind::Ptb => "ptb",
        ClientKind::Canary => "canary",
        ClientKind::Vencord => "vencord",
    };
    vec![dir_for(flavor)]
}

/// Scan one install root for `app-*/modules/discord_voice-*/discord_voice/discord_voice.node`.
fn scan_root(kind: ClientKind, root: &Path) -> Vec<FoundClient> {
    let mut found = Vec::new();
    let Ok(app_dirs) = std::fs::read_dir(root) else {
        return found;
    };
    for app in app_dirs.flatten() {
        let name = app.file_name().to_string_lossy().into_owned();
        if !name.starts_with("app-") {
            continue;
        }
        let modules = app.path().join("modules");
        let Ok(mods) = std::fs::read_dir(&modules) else {
            continue;
        };
        for m in mods.flatten() {
            let node = m.path().join("discord_voice").join("discord_voice.node");
            if node.is_file() {
                found.push(FoundClient {
                    label: kind.label().to_owned(),
                    kind,
                    version_dir: name.clone(),
                    node_path: node,
                    process_running: discord_process_running(),
                });
            }
        }
    }
    found
}

/// Best-effort check for a running Discord host on this OS.
pub fn discord_process_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().as_bytes()[0].is_ascii_digit())
        .any(|e| {
            let cmdline = e.path().join("cmdline");
            let Ok(content) = std::fs::read(cmdline) else {
                return false;
            };
            content.windows(7).any(|w| w == b"Discord")
        })
}
