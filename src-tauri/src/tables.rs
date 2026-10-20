//! Patch-DB: schema do `tables.json`, bootstrap embutido, cache e adoção.
//!
//! Fonte única: `patchdb/tables.json` na raiz do repo. O binário embute o
//! arquivo (`include_str!`) como bootstrap offline; o download da raw do
//! `main` entra como cache no app_data e vence o bootstrap com `db_version`
//! maior. Nota serde: `deny_unknown_fields` em enums com `tag` é ignorado
//! pela crate — o gate real de schema é o validador do CI
//! (`dev/validate-patchdb.mjs`).

use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

/// Fonte viva das tabelas. Override de dev: `STEREO_DB_URL` aponta para um
/// JSON local (harness `python3 -m http.server`), senão usa a raw do main.
pub const REMOTE_DB: &str =
    "https://raw.githubusercontent.com/sittiev/discord_stereo/main/patchdb/tables.json";

const BUILTIN_DB: &str = include_str!("../../patchdb/tables.json");

/// Teto de tamanho do download; o arquivo hoje tem ~30 KB.
const DOWNLOAD_CAP: u64 = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("patchdb: {0}")]
    Json(#[from] serde_json::Error),
    #[error("patchdb: campo inválido em {where_}: {why}")]
    Field { where_: String, why: String },
}

/// Teto de leitura para o download remoto; exposto para o comando de fetch.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalMeta {
    pub id: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    /// md5 do binário alvo. Obrigatório quando há sites `offset`;
    /// tabela pattern-only pode omitir e atravessa builds menores.
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
    /// Cirúrgico: offset fixo no binário apontado por `binary_md5`.
    Offset {
        offset: String,
        orig: String,
        patch: String,
        name: String,
    },
    /// Scan por assinatura (AOB), `??` = wildcard. Exige único match;
    /// `expect` confere os bytes correntes antes de escrever.
    Pattern {
        name: String,
        pattern: String,
        patch: String,
        #[serde(default)]
        expect: Option<String>,
    },
}

/// Valida estrutura lógica além do serde: ids únicos, goal conhecido e
/// sites offset exigindo `binary_md5` na tabela.
pub fn parse_db(json: &str) -> Result<PatchDb, DbError> {
    let db: PatchDb = serde_json::from_str(json)?;
    let mut ids = db.goals.iter().map(|g| g.id.as_str()).collect::<Vec<_>>();
    ids.sort_unstable();
    if ids.windows(2).any(|w| w[0] == w[1]) {
        return Err(DbError::Field {
            where_: "goals".into(),
            why: "id duplicado".into(),
        });
    }
    for t in &db.tables {
        if !db.goals.iter().any(|g| g.id == t.goal) {
            return Err(DbError::Field {
                where_: "tables".into(),
                why: format!("goal desconhecido {}", t.goal),
            });
        }
        if t.sites.iter().any(|s| matches!(s, TableSite::Offset { .. })) && t.binary_md5.is_none() {
            return Err(DbError::Field {
                where_: format!("table {}", t.goal),
                why: "site offset exige binary_md5".into(),
            });
        }
    }
    Ok(db)
}

// ---- estado global (app de janela única: uma DB por processo) ----
static DB: OnceLock<RwLock<PatchDb>> = OnceLock::new();
static CACHE_FILE: OnceLock<PathBuf> = OnceLock::new();

fn cell() -> &'static RwLock<PatchDb> {
    DB.get_or_init(|| {
        RwLock::new(parse_db(BUILTIN_DB).expect("patchdb embutido inválido"))
    })
}

/// Bootstrap: embutido sempre carrega; cache no app_data vence se
/// `db_version` maior. Erro de cache é silencioso (volta pro embutido).
pub fn init(cache_dir: PathBuf) {
    cell();
    let cache = cache_dir.join("patchdb.json");
    if let Ok(json) = std::fs::read_to_string(&cache) {
        if let Ok(cached) = parse_db(&json) {
            let mut db = cell().write().expect("db lock");
            if cached.db_version > db.db_version {
                *db = cached;
            }
        }
    }
    let _ = CACHE_FILE.set(cache);
}

/// Snapshot para os comandos.
pub fn snapshot() -> PatchDb {
    cell().read().expect("db lock").clone()
}

/// Adota um JSON baixado: valida, salva cache (tmp + rename) e troca o
/// global quando `db_version` é maior. Retorna se algo mudou.
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
    fn builtin_valido_com_goals_e_tabelas() {
        let db = parse_db(BUILTIN_DB).expect("embutido válido");
        assert_eq!(db.schema, 1);
        assert!(db.goals.iter().any(|g| g.id == "v4"));
        assert_eq!(db.tables.len(), 3);
    }

    #[test]
    fn offset_sem_md5_recusado() {
        let bad = r#"{"schema":1,"db_version":2,"updated":"2026-01-01","goals":[{"id":"v3","title":"a","description":"b"}],"tables":[{"goal":"v3","sites":[{"kind":"offset","offset":"0x0","orig":"02","patch":"00","name":"x"}]}]}"#;
        assert!(parse_db(bad).is_err());
    }

    #[test]
    fn pattern_sem_md5_aceito() {
        let ok = r#"{"schema":1,"db_version":2,"updated":"2026-01-01","goals":[{"id":"v3","title":"a","description":"b"}],"tables":[{"goal":"v3","sites":[{"kind":"pattern","name":"x","pattern":"55 ??","patch":"c3 c3","expect":"55 48"}]}]}"#;
        assert!(parse_db(ok).is_ok());
    }
}
