//! Multi-client, multi-OS Discord installation detection.
//!
//! Finds `discord_voice.node` for every supported client flavor on
//! Linux, Windows and macOS without launching anything or touching the
//! user's session. Read-only by design.

use std::collections::BTreeMap;
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
    let mut out = newest_per_kind(out);
    out.sort_by_key(|c| c.kind_label_key());
    out
}

/// One client per flavor: Discord keeps old `app-1.0.*` directories after an
/// update, and showing every folder looks like duplicate clients.
fn newest_per_kind(clients: Vec<FoundClient>) -> Vec<FoundClient> {
    let mut newest: BTreeMap<u8, FoundClient> = BTreeMap::new();
    for client in clients {
        let rank = kind_rank(client.kind);
        match newest.get(&rank) {
            Some(existing) if app_version(&existing.version_dir) >= app_version(&client.version_dir) => {}
            _ => {
                newest.insert(rank, client);
            }
        }
    }
    newest.into_values().collect()
}

fn kind_rank(kind: ClientKind) -> u8 {
    match kind {
        ClientKind::Stable => 0,
        ClientKind::Ptb => 1,
        ClientKind::Canary => 2,
        ClientKind::Vencord => 3,
    }
}

impl FoundClient {
    fn kind_label_key(&self) -> (u8, String) {
        (kind_rank(self.kind), self.version_dir.clone())
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
///
/// One client per `app-*` directory: if the updater left more than one
/// `discord_voice-*` module behind, keep the highest module version.
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
        let mut newest: Option<(u64, PathBuf)> = None;
        for m in mods.flatten() {
            let node = m.path().join("discord_voice").join("discord_voice.node");
            if !node.is_file() {
                continue;
            }
            let module_name = m.file_name().to_string_lossy().into_owned();
            let version = module_version(&module_name);
            if newest
                .as_ref()
                .is_none_or(|(current, _)| version > *current)
            {
                newest = Some((version, node));
            }
        }
        if let Some((_, node)) = newest {
            found.push(FoundClient {
                label: kind.label().to_owned(),
                kind,
                version_dir: name,
                node_path: node,
                process_running: discord_process_running(),
            });
        }
    }
    found
}

fn module_version(name: &str) -> u64 {
    name.rsplit('-')
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn app_version(version_dir: &str) -> u64 {
    version_dir
        .rsplit('.')
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn module_version_uses_the_numeric_suffix() {
        assert_eq!(module_version("discord_voice-1"), 1);
        assert_eq!(module_version("discord_voice-12"), 12);
        assert_eq!(module_version("discord_voice"), 0);
    }

    #[test]
    fn app_version_parses_the_last_dotted_number() {
        assert_eq!(app_version("app-1.0.9267"), 9267);
        assert_eq!(app_version("app-1.0.10000"), 10000);
        assert_eq!(app_version("app-1.0.161"), 161);
    }

    #[test]
    fn newest_per_kind_keeps_one_entry_per_client() {
        let client = |kind: ClientKind, version: &str| FoundClient {
            label: kind.label().to_owned(),
            kind,
            version_dir: version.to_owned(),
            node_path: PathBuf::from("/tmp/discord_voice.node"),
            process_running: false,
        };
        let out = newest_per_kind(vec![
            client(ClientKind::Canary, "app-1.0.9266"),
            client(ClientKind::Canary, "app-1.0.9267"),
            client(ClientKind::Stable, "app-1.0.9267"),
        ]);
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|c| c.kind == ClientKind::Canary && c.version_dir == "app-1.0.9267"));
        assert!(out.iter().any(|c| c.kind == ClientKind::Stable));
    }

    #[test]
    fn one_client_per_app_keeps_the_newest_module() {
        let root = std::env::temp_dir().join(format!(
            "sw-detect-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let modules = root.join("app-1.0.9267").join("modules");
        for name in ["discord_voice-1", "discord_voice-2"] {
            let dir = modules.join(name).join("discord_voice");
            fs::create_dir_all(&dir).expect("create fake module");
            fs::write(dir.join("discord_voice.node"), b"node").expect("write fake node");
        }

        let found = scan_root(ClientKind::Canary, &root);
        assert_eq!(found.len(), 1, "one client per app-* directory");
        assert!(
            found[0]
                .node_path
                .to_string_lossy()
                .contains("discord_voice-2"),
            "newest module wins: {:?}",
            found[0].node_path
        );

        let _ = fs::remove_dir_all(&root);
    }
}
