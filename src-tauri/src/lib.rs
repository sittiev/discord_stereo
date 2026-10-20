//! Stereo Wizard backend: wizard commands exposed to the webview.
//!
//! All commands are read-only or explicitly requested by the user via
//! button press; nothing here runs at app startup beyond `detect`.

mod detect;
mod patch_engine;

use std::path::{Path, PathBuf};

use detect::{FoundClient, scan_all};
use patch_engine::{Analysis, Goal, analyze, apply, revert};
use serde::Serialize;

/// Scan for installed Discord clients (read-only).
#[tauri::command]
fn detect_clients() -> Vec<FoundClient> {
    scan_all()
}

/// Goal metadata for the UI (titles, descriptions, site counts).
#[tauri::command]
fn goals() -> Vec<GoalInfo> {
    Goal::ALL
        .iter()
        .map(|g| GoalInfo {
            id: g.id(),
            title: g.title(),
            description: g.description(),
            sites: patch_engine::sites_for(*g).len(),
        })
        .collect()
}

/// UI-facing goal metadata.
#[derive(Serialize)]
struct GoalInfo {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    sites: usize,
}

/// Analyze a node for a goal without writing (dry-run).
#[tauri::command]
fn analyze_node(node: String, goal: String) -> Result<Analysis, String> {
    let goal: Goal = serde_json::from_value(serde_json::Value::String(goal))
        .map_err(|e| e.to_string())?;
    analyze(Path::new(&node), goal).map_err(|e| e.to_string())
}

/// Apply a goal to a node (verify-then-write, backup, idempotent).
#[tauri::command]
fn apply_goal(node: String, goal: String) -> Result<Analysis, String> {
    let goal: Goal = serde_json::from_value(serde_json::Value::String(goal))
        .map_err(|e| e.to_string())?;
    let path = PathBuf::from(&node);
    if discord_running() {
        return Err("o Discord está rodando — feche-o antes de aplicar o patch".into());
    }
    apply(&path, goal).map_err(|e| e.to_string())
}

/// Revert a node from the newest `.bak-*` backup.
#[tauri::command]
fn revert_node(node: String) -> Result<String, String> {
    let path = PathBuf::from(&node);
    if discord_running() {
        return Err("o Discord está rodando — feche-o antes de reverter".into());
    }
    revert(&path)
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

/// Generic troubleshooting/info payload.
#[tauri::command]
fn system_info() -> SystemInfo {
    SystemInfo {
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        known_pristine_md5: patch_engine::KNOWN_PRISTINE_MD5.to_owned(),
    }
}

#[derive(Serialize)]
struct SystemInfo {
    os: String,
    arch: String,
    known_pristine_md5: String,
}

/// Whether a Discord host process is running. Best-effort: only Linux is
/// inspected (`/proc`); on other OSes the guard passes and the engine's
/// verify-then-write is the remaining safety layer.
fn discord_running() -> bool {
    if std::env::consts::OS != "linux" {
        return false;
    }
    detect::discord_process_running()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    // The MCP bridge is compiled only with the `dev-tools` feature (never in
    // release builds) and only listens when STEREO_MCP is set.
    #[cfg(feature = "dev-tools")]
    let builder = if std::env::var_os("STEREO_MCP").is_some() {
        builder.plugin(tauri_plugin_mcp_bridge::init())
    } else {
        builder
    };
    builder
        .invoke_handler(tauri::generate_handler![
            detect_clients,
            goals,
            analyze_node,
            apply_goal,
            revert_node,
            system_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
