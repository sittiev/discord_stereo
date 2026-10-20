//! Stereo Wizard backend: wizard commands exposed to the webview.
//!
//! All commands are read-only or explicitly requested by the user via
//! button press; nothing here runs at app startup beyond `detect` and the
//! cached Patch-DB bootstrap.

mod detect;
mod patch_engine;
mod tables;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use detect::{FoundClient, scan_all};
use patch_engine::{Analysis, analyze, apply, revert};
use serde::Serialize;
use tauri::Manager;

/// Scan for installed Discord clients (read-only).
#[tauri::command]
fn detect_clients() -> Vec<FoundClient> {
    scan_all()
}

/// Goal metadata for the UI (titles, descriptions, site counts).
#[tauri::command]
fn goals() -> Vec<GoalInfo> {
    let db = tables::snapshot();
    db.goals
        .into_iter()
        .map(|g| GoalInfo {
            sites: db
                .tables
                .iter()
                .filter(|t| t.goal == g.id)
                .map(|t| t.sites.len())
                .sum(),
            id: g.id,
            title: g.title,
            description: g.description,
        })
        .collect()
}

/// UI-facing goal metadata.
#[derive(Serialize)]
struct GoalInfo {
    id: String,
    title: String,
    description: String,
    sites: usize,
}

/// Analyze a node for a goal without writing (dry-run).
#[tauri::command]
fn analyze_node(node: String, goal: String) -> Result<Analysis, String> {
    analyze(Path::new(&node), &goal, &tables::snapshot()).map_err(|e| e.to_string())
}

/// Apply a goal to a node (verify-then-write, backup, idempotent).
#[tauri::command]
fn apply_goal(node: String, goal: String) -> Result<Analysis, String> {
    let path = PathBuf::from(&node);
    if discord_running() {
        return Err("Discord is running — close it before applying the patch".into());
    }
    apply(&path, &goal, &tables::snapshot()).map_err(|e| e.to_string())
}

/// Revert a node from the newest `.bak-*` backup.
#[tauri::command]
fn revert_node(node: String) -> Result<String, String> {
    let path = PathBuf::from(&node);
    if discord_running() {
        return Err("Discord is running — close it before reverting".into());
    }
    revert(&path)
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

/// Generic troubleshooting/info payload.
#[tauri::command]
fn system_info() -> SystemInfo {
    let db = tables::snapshot();
    SystemInfo {
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        known_pristine_md5: db
            .tables
            .iter()
            .filter_map(|t| t.binary_md5.clone())
            .next()
            .unwrap_or_default(),
    }
}

#[derive(Serialize)]
struct SystemInfo {
    os: String,
    arch: String,
    known_pristine_md5: String,
}

/// Patch-DB summary shown in the about step.
#[tauri::command]
fn patch_db_info() -> PatchDbInfo {
    db_info(tables::snapshot())
}

#[derive(Serialize)]
struct PatchDbInfo {
    db_version: u32,
    updated: String,
    goals: usize,
    tables: usize,
    remote_url: String,
}

fn db_info(db: tables::PatchDb) -> PatchDbInfo {
    let remote_url = match std::env::var("STEREO_DB_URL") {
        Ok(url) => format!("{url} (dev override)"),
        Err(_) => tables::REMOTE_DB.to_owned(),
    };
    PatchDbInfo {
        db_version: db.db_version,
        updated: db.updated,
        goals: db.goals.len(),
        tables: db.tables.len(),
        remote_url,
    }
}

#[derive(Serialize)]
struct DbCheck {
    adopted: bool,
    info: Option<PatchDbInfo>,
    error: Option<String>,
}

/// Downloads the Patch-DB remote and adopts it when `db_version` is newer.
/// Blocking work runs on a separate thread pool; capped at 2 MiB and 15s.
#[tauri::command]
async fn check_db_update() -> DbCheck {
    tauri::async_runtime::spawn_blocking(fetch_adopt_db)
        .await
        .unwrap_or(DbCheck {
            adopted: false,
            info: None,
            error: Some("check_db_update: task was cancelled".into()),
        })
}

fn fetch_adopt_db() -> DbCheck {
    let denied = |what: String| DbCheck {
        adopted: false,
        info: Some(db_info(tables::snapshot())),
        error: Some(what),
    };
    let url = std::env::var("STEREO_DB_URL").unwrap_or_else(|_| tables::REMOTE_DB.to_owned());
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();
    let response = match agent.get(&url).call() {
        Ok(resp) => resp,
        Err(e) => return denied(format!("fetching patchdb: {e}")),
    };
    let mut json = String::new();
    if let Err(e) = response
        .into_reader()
        .take(tables::DOWNLOAD_CAP_BYTES)
        .read_to_string(&mut json)
    {
        return denied(format!("parsing patchdb: {e}"));
    }
    match tables::adopt_remote(&json) {
        Ok(adopted) => DbCheck {
            adopted,
            info: Some(db_info(tables::snapshot())),
            error: None,
        },
        Err(e) => DbCheck {
            adopted: false,
            info: None,
            error: Some(e.to_string()),
        },
    }
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
        .setup(|app| {
            if let Some(dir) = app.path().app_data_dir().ok() {
                tables::init(dir);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            detect_clients,
            goals,
            analyze_node,
            apply_goal,
            revert_node,
            system_info,
            patch_db_info,
            check_db_update
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
