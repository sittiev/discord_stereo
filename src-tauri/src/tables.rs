//! Patch-DB: `tables.json` schema, embedded bootstrap, cache and adoption.
//!
//! Single source: `patchdb/tables.json` at the repo root. The binary embeds
//! the file (`include_str!`) as the offline bootstrap; a download of the
//! `main` raw lands as cache in app_data and wins over the bootstrap with a
//! higher `db_version`. serde note: `deny_unknown_fields` on tagged enums
//! is ignored by the crate — the real schema gate is the CI validator
//! (`dev/validate-patchdb.mjs`).

use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

/// Live source for the tables. Dev override: `STEREO_DB_URL` points to a
/// local JSON (harness `python3 -m http.server`); otherwise uses the main raw.
pub const REMOTE_DB: &str =
    "https://raw.githubusercontent.com/sittiev/discord_stereo/main/patchdb/tables.json";

const BUILTIN_DB: &str = include_str!("../../patchdb/tables.json");

/// Download size cap; the file is ~30 KB today.
const DOWNLOAD_CAP: u64 = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("patchdb: {0}")]
    Json(#[from] serde_json::Error),
    #[error("patchdb: invalid field in {where_}: {why}")]
    Field { where_: String, why: String },
}

/// Read cap for the remote download; exposed for the fetch command.
pub const DOWNLOAD_CAP_BYTES: u64 = DOWNLOAD_CAP;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchDb {
    pub schema: u32,
    pub db_version: u32,
    pub updated: String,
    pub goals: Vec<GoalMeta>,
    pub tables: Vec<Table>,
}

/// Goal id plus bilingual user-visible strings (db_version 2 schema).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalMeta {
    pub id: String,
    pub title: Localized,
    pub description: Localized,
}

/// Localized pair for user-visible DB strings; UI picks by language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Localized {
    pub pt: String,
    pub en: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    /// md5 of the target binary. Required when there are `offset` sites;
    /// a pattern-only table may omit it and spans smaller builds.
    pub binary_md5: Option<String>,
    /// Version dir informativo, ex. `app-1.0.161`.
    pub discord: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub goal: String,
    pub sites: Vec<TableSite>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TableSite {
    /// Surgical: fixed offset in the binary pointed to by `binary_md5`.
    Offset {
        offset: String,
        orig: String,
        patch: String,
        name: String,
    },
    /// Signature scan (AOB), `??` = wildcard. Requires a unique match;
    /// `expect` checks the current bytes before writing.
    Pattern {
        name: String,
        pattern: String,
        patch: String,
        #[serde(default)]
        expect: Option<String>,
    },
}

impl TableSite {
    pub fn name(&self) -> &str {
        match self {
            TableSite::Offset { name, .. } | TableSite::Pattern { name, .. } => name,
        }
    }
}

/// Validates structure beyond serde: unique ids, known goals, and offset
/// sites requiring `binary_md5` on the table.
pub fn parse_db(json: &str) -> Result<PatchDb, DbError> {
    let db: PatchDb = serde_json::from_str(json)?;
    let mut ids = db.goals.iter().map(|g| g.id.as_str()).collect::<Vec<_>>();
    ids.sort_unstable();
    if ids.windows(2).any(|w| w[0] == w[1]) {
        return Err(DbError::Field {
            where_: "goals".into(),
            why: "duplicate id".into(),
        });
    }
    for t in &db.tables {
        if !db.goals.iter().any(|g| g.id == t.goal) {
            return Err(DbError::Field {
                where_: "tables".into(),
                why: format!("unknown goal {}", t.goal),
            });
        }
        if t.sites.iter().any(|s| matches!(s, TableSite::Offset { .. })) && t.binary_md5.is_none() {
            return Err(DbError::Field {
                where_: format!("table {}", t.goal),
                why: "offset site requires binary_md5".into(),
            });
        }
    }
    Ok(db)
}

// ---- global state (single-window app: one DB per process) ----
static DB: OnceLock<RwLock<PatchDb>> = OnceLock::new();
static CACHE_FILE: OnceLock<PathBuf> = OnceLock::new();

fn cell() -> &'static RwLock<PatchDb> {
    DB.get_or_init(|| {
        RwLock::new(parse_db(BUILTIN_DB).expect("invalid embedded patchdb"))
    })
}

/// Bootstrap: the embedded DB always loads; a cache in app_data wins when
/// its `db_version` is higher. Cache errors are silent (falls back to the
/// embedded DB).
pub fn init(cache_dir: PathBuf) {
    cell();
    let cache = cache_dir.join("patchdb.json");
    if let Ok(json) = std::fs::read_to_string(&cache)
        && let Ok(cached) = parse_db(&json)
        && cached.db_version > cell().read().expect("db lock").db_version
    {
        *cell().write().expect("db lock") = cached;
    }
    let _ = CACHE_FILE.set(cache);
}

/// Snapshot for the commands.
pub fn snapshot() -> PatchDb {
    cell().read().expect("db lock").clone()
}

/// Adopts a downloaded JSON: validates, saves the cache (tmp + rename) and
/// swaps the global when its `db_version` is higher. Returns whether
/// anything changed.
pub fn adopt_remote(json: &str) -> Result<bool, DbError> {
    let new_db = parse_db(json)?;
    let cell = cell();
    let current = cell.read().expect("db lock").db_version;
    if new_db.db_version <= current {
        return Ok(false);
    }
    *cell.write().expect("db lock") = new_db;
    write_cache(json)?;
    Ok(true)
}

fn write_cache(json: &str) -> Result<(), DbError> {
    let Some(path) = CACHE_FILE.get() else { return Ok(()) };
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| DbError::Field {
        where_: "cache".into(),
        why: e.to_string(),
    })?;
    std::fs::rename(&tmp, path).map_err(|e| DbError::Field {
        where_: "cache".into(),
        why: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_valid_with_goals_and_tables() {
        let db = parse_db(BUILTIN_DB).expect("embedded patchdb valid");
        assert_eq!(db.schema, 1);
        assert!(db.goals.iter().any(|g| g.id == "v4"));
        assert_eq!(db.tables.len(), 3);
    }

    #[test]
    fn offset_without_md5_rejected() {
        let bad = r#"{"schema":1,"db_version":2,"updated":"2026-01-01","goals":[{"id":"v3","title":{"pt":"a","en":"a"},"description":{"pt":"b","en":"b"}}],"tables":[{"goal":"v3","sites":[{"kind":"offset","offset":"0x0","orig":"02","patch":"00","name":"x"}]}]}"#;
        assert!(parse_db(bad).is_err());
    }

    #[test]
    fn pattern_without_md5_accepted() {
        let ok = r#"{"schema":1,"db_version":2,"updated":"2026-01-01","goals":[{"id":"v3","title":{"pt":"a","en":"a"},"description":{"pt":"b","en":"b"}}],"tables":[{"goal":"v3","sites":[{"kind":"pattern","name":"x","pattern":"55 ??","patch":"c3 c3","expect":"55 48"}]}]}"#;
        assert!(parse_db(ok).is_ok());
    }
}
