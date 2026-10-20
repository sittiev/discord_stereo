//! Stereo patch engine: Patch-DB sites, verify-then-write, backup, revert.
//!
//! Tables come from [`crate::tables::PatchDb`]: a table with `binary_md5` is
//! surgical (fixed offset); a pattern-only table resolves via AOB signature
//! with a unique match and `expect`. A binary without a resolvable table is
//! never written.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::tables::{PatchDb, Table, TableSite};

/// Resolved site: pattern and offset converge to the same shape.
///
/// `reason` set = pattern did not match (0/2+ matches, invalid token);
/// the site counts as mismatch and is never written.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub name: String,
    pub offset: u32,
    pub orig: Vec<u8>,
    pub patch: Vec<u8>,
    pub reason: Option<String>,
}

/// Per-site analysis outcome.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum SiteState {
    /// Bytes match `orig` — patchable.
    Ok,
    /// Bytes match `patch` — already applied.
    Patched,
    /// Bytes differ from both — binary mismatch at this site.
    Mismatch { found: String },
}

/// Full node analysis for a goal.
#[derive(Debug, Serialize)]
pub struct Analysis {
    /// Node path analyzed.
    pub path: PathBuf,
    /// Node size in bytes.
    pub size: u64,
    /// MD5 hex.
    pub md5: String,
    /// Whether a table exists for this binary (md5-keyed or pattern-only).
    pub known_pristine: bool,
    /// Per-site states for the selected goal.
    pub sites: Vec<(String, SiteState)>,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("hex decode in {site}: {source}")]
    Hex {
        site: String,
        source: hex::FromHexError,
    },
    #[error("validation failed on {n} site(s); nothing was written")]
    Validation { n: usize },
    #[error("no backup found to revert")]
    NoBackup,
    #[error("no table for goal {goal} (md5 {md5}); nothing was written")]
    NoTable { md5: String, goal: String },
}

fn decode_hex(s: &str, site_name: &str) -> Result<Vec<u8>, EngineError> {
    let compact: String = s.split_whitespace().collect();
    hex::decode(&compact).map_err(|source| EngineError::Hex {
        site: site_name.to_owned(),
        source,
    })
}

fn parse_offset(s: &str, site_name: &str) -> Result<u32, EngineError> {
    let body = s.trim().strip_prefix("0x").unwrap_or(s);
    u32::from_str_radix(body, 16).map_err(|_| EngineError::Hex {
        site: format!("{site_name}: offset {s}"),
        source: hex::FromHexError::InvalidHexCharacter { c: '?', index: 0 },
    })
}

/// Pattern tokens: `??` = wildcard (any byte), hex = exact byte.
struct Aob {
    pat: Vec<u8>,
    mask: Vec<bool>,
}

fn parse_aob(pattern: &str) -> Option<Aob> {
    let mut aob = Aob {
        pat: Vec::new(),
        mask: Vec::new(),
    };
    for token in pattern.split_whitespace() {
        if token == "??" {
            aob.pat.push(0);
            aob.mask.push(false);
        } else {
            match hex::decode(token) {
                Ok(b) if b.len() == 1 => {
                    aob.pat.push(b[0]);
                    aob.mask.push(true);
                }
                _ => return None,
            }
        }
    }
    if aob.pat.is_empty() {
        return None;
    }
    Some(aob)
}

/// AOB scan: returns the offset of the single match, or None (0 or 2+ matches).
fn scan_pattern(bytes: &[u8], aob: &Aob) -> Option<usize> {
    if aob.pat.len() > bytes.len() {
        return None;
    }
    let mut found: Option<usize> = None;
    'outer: for i in 0..=(bytes.len() - aob.pat.len()) {
        for (j, (&p, &m)) in aob.pat.iter().zip(aob.mask.iter()).enumerate() {
            if m && bytes[i + j] != p {
                continue 'outer;
            }
        }
        if found.is_some() {
            return None;
        }
        found = Some(i);
    }
    found
}

/// Readable reason when the pattern did not produce a unique match.
fn pattern_reason(bytes: &[u8], pattern: &str) -> String {
    let Some(aob) = parse_aob(pattern) else {
        return "invalid pattern".into();
    };
    if aob.pat.len() > bytes.len() {
        return "0 matches".into();
    }
    let mut hits = 0usize;
    'outer: for i in 0..=(bytes.len() - aob.pat.len()) {
        for (j, (&p, &m)) in aob.pat.iter().zip(aob.mask.iter()).enumerate() {
            if m && bytes[i + j] != p {
                continue 'outer;
            }
        }
        hits += 1;
    }
    format!("{hits} matches")
}

/// Table applicable to (goal, md5): the exact-md5 table first; otherwise the
/// first table for the goal (pattern-only or another build's md5). The
/// verify-then-write pass rejects unexpected bytes in either case, so a
/// foreign binary is never written.
fn table_for<'a>(db: &'a PatchDb, goal: &str, md5: &str) -> Option<&'a Table> {
    db.tables
        .iter()
        .find(|t| t.goal == goal && t.binary_md5.as_deref() == Some(md5))
        .or_else(|| db.tables.iter().find(|t| t.goal == goal))
}

fn resolve_sites(table: &Table, bytes: &[u8]) -> Result<Vec<Resolved>, EngineError> {
    let mut out = Vec::with_capacity(table.sites.len());
    for site in &table.sites {
        match site {
            TableSite::Offset {
                offset,
                orig,
                patch,
                name,
            } => out.push(Resolved {
                name: name.clone(),
                offset: parse_offset(offset, name)?,
                orig: decode_hex(orig, name)?,
                patch: decode_hex(patch, name)?,
                reason: None,
            }),
            TableSite::Pattern {
                name,
                pattern,
                patch,
                expect,
            } => {
                let patched = decode_hex(patch, name)?;
                let Some(off) = parse_aob(pattern).and_then(|aob| scan_pattern(bytes, &aob))
                else {
                    out.push(Resolved {
                        name: name.clone(),
                        offset: 0,
                        orig: Vec::new(),
                        patch: patched,
                        reason: Some(pattern_reason(bytes, pattern)),
                    });
                    continue;
                };
                let end = off + patched.len();
                if end > bytes.len() {
                    out.push(Resolved {
                        name: name.clone(),
                        offset: 0,
                        orig: Vec::new(),
                        patch: patched,
                        reason: Some("pattern beyond binary end".into()),
                    });
                    continue;
                }
                let orig = match expect {
                    Some(expected) => decode_hex(expected, name)?,
                    None => bytes[off..end].to_vec(),
                };
                out.push(Resolved {
                    name: name.clone(),
                    offset: off as u32,
                    orig,
                    patch: patched,
                    reason: None,
                });
            }
        }
    }
    Ok(out)
}

fn site_state(resolved: &Resolved, bytes: &[u8]) -> SiteState {
    if let Some(reason) = &resolved.reason {
        return SiteState::Mismatch {
            found: reason.clone(),
        };
    }
    let start = resolved.offset as usize;
    let end = start + resolved.orig.len().max(resolved.patch.len());
    if end > bytes.len() {
        return SiteState::Mismatch { found: "eof".into() };
    }
    let cur = &bytes[start..end];
    if cur == resolved.orig.as_slice() {
        SiteState::Ok
    } else if cur == resolved.patch.as_slice() {
        SiteState::Patched
    } else {
        SiteState::Mismatch {
            found: hex::encode(cur),
        }
    }
}

/// Analyze a node against a goal without writing anything.
///
/// # Errors
/// - [`EngineError::Io`]: the node cannot be read.
/// - [`EngineError::Hex`]: a table entry is malformed (bug in the DB).
pub fn analyze(node: &Path, goal: &str, db: &PatchDb) -> Result<Analysis, EngineError> {
    let bytes = fs::read(node)?;
    analyze_bytes(node, &bytes, goal, db)
}

/// Pure variant of [`analyze`] over an in-memory node.
///
/// # Errors
/// [`EngineError::Hex`] if a table entry is malformed (bug in the DB).
pub fn analyze_bytes(
    node: &Path,
    bytes: &[u8],
    goal: &str,
    db: &PatchDb,
) -> Result<Analysis, EngineError> {
    let md5 = md5_hex(bytes);
    let table = table_for(db, goal, &md5);
    let sites = table
        .map(|t| resolve_sites(t, bytes))
        .transpose()?
        .map(|resolved| {
            resolved
                .iter()
                .map(|r| (r.name.clone(), site_state(r, bytes)))
                .collect()
        })
        .unwrap_or_default();
    Ok(Analysis {
        path: node.to_path_buf(),
        size: bytes.len() as u64,
        md5,
        known_pristine: table.is_some(),
        sites,
    })
}

/// Apply a goal to a node: verify-then-write with backup.
///
/// Idempotent: already-patched sites are skipped. Any mismatch (bytes
/// differing or a pattern without a unique match) aborts before a single
/// byte is written. A binary where an already-applied pattern broke its own
/// signature needs `revert` before apply.
///
/// # Errors
/// - [`EngineError::NoTable`]: the goal has no tables at all.
/// - [`EngineError::Validation`]: `n` sites are not patchable.
/// - [`EngineError::Io`]: the node cannot be read or written.
pub fn apply(node: &Path, goal: &str, db: &PatchDb) -> Result<Analysis, EngineError> {
    let mut bytes = fs::read(node)?;
    let md5 = md5_hex(&bytes);
    let Some(table) = table_for(db, goal, &md5) else {
        return Err(EngineError::NoTable { md5, goal: goal.into() });
    };
    let analysis = analyze_bytes(node, &bytes, goal, db)?;
    let mismatched = analysis
        .sites
        .iter()
        .filter(|(_, st)| matches!(st, SiteState::Mismatch { .. }))
        .count();
    if mismatched > 0 {
        return Err(EngineError::Validation { n: mismatched });
    }

    let resolved = resolve_sites(table, &bytes)?;
    let todo: Vec<&Resolved> = resolved
        .iter()
        .filter(|r| r.reason.is_none())
        .filter(|r| {
            let start = r.offset as usize;
            bytes.get(start..start + r.patch.len()) != Some(r.patch.as_slice())
        })
        .collect();
    if todo.is_empty() {
        return Ok(analysis);
    }

    let backup = backup_path(node, goal);
    if !backup.exists() {
        fs::copy(node, &backup)?;
    }
    for r in &todo {
        let start = r.offset as usize;
        bytes[start..start + r.patch.len()].copy_from_slice(&r.patch);
    }
    fs::write(node, &bytes)?;
    let sites: Vec<(String, SiteState)> = analysis
        .sites
        .into_iter()
        .map(|(name, state)| match state {
            SiteState::Ok => (name, SiteState::Patched),
            state => (name, state),
        })
        .collect();
    debug_assert_eq!(
        sites
            .iter()
            .filter(|(_, st)| matches!(st, SiteState::Ok))
            .count(),
        0,
        "apply left unpatched sites behind; verify-then-write invariant broken"
    );
    Ok(Analysis {
        path: node.to_path_buf(),
        size: bytes.len() as u64,
        md5: md5_hex(&bytes),
        known_pristine: true,
        sites,
    })
}

/// Revert a node from the most recent `.bak-*` backup.
///
/// # Errors
/// [`EngineError::NoBackup`] when no backup exists, [`EngineError::Io`]
/// when directory reads or file copies fail.
pub fn revert(node: &Path) -> Result<PathBuf, EngineError> {
    let parent = node.parent().unwrap_or_else(|| Path::new("."));
    let stem = node
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = node
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();

    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    let entries = fs::read_dir(parent)?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&format!("{stem}{ext}.bak-")) {
            let Ok(meta) = entry.metadata() else { continue };
            let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().is_none_or(|(t, _)| mtime > *t) {
                best = Some((mtime, entry.path()));
            }
        }
    }

    let (_, backup) = best.ok_or(EngineError::NoBackup)?;
    fs::copy(&backup, node)?;
    Ok(backup)
}

fn backup_path(node: &Path, goal: &str) -> PathBuf {
    let mut p = node.as_os_str().to_os_string();
    p.push(format!(".bak-{goal}"));
    PathBuf::from(p)
}

/// MD5 of a byte slice, lowercase hex.
#[must_use]
pub fn md5_hex(bytes: &[u8]) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(bytes);
    let sum = hasher.finalize();
    hex::encode(sum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::parse_db;

    fn table_json(goal: &str, md5: Option<&str>, site: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "goal": goal,
            "binary_md5": md5.map(String::from),
            "sites": [site]
        })
    }

    fn db_of(md5: Option<&str>, site: serde_json::Value) -> PatchDb {
        let v = serde_json::json!({
            "schema": 1,
            "db_version": 1,
            "updated": "2026-10-20",
            "goals": [{ "id": "t", "title": "T", "description": "d" }],
            "tables": [table_json("t", md5, site)]
        });
        parse_db(&v.to_string()).expect("valid test db")
    }

    fn offset_site() -> serde_json::Value {
        serde_json::json!({
            "kind": "offset",
            "offset": "0x00",
            "orig": "02",
            "patch": "00",
            "name": "slot0"
        })
    }

    fn pattern_site(pattern: &str, expect: &str, patch: &str) -> serde_json::Value {
        serde_json::json!({
            "kind": "pattern",
            "name": "sig",
            "pattern": pattern,
            "patch": patch,
            "expect": expect
        })
    }

    fn node_created(tag: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("sw-engine-{}-{tag}.node", std::process::id()));
        fs::write(&path, bytes).expect("tmp node written");
        path
    }

    fn cleanup(path: &Path) {
        let pa = path.as_os_str().to_os_string();
        let _ = fs::remove_file(&*pa);
        let mut pb = pa.clone();
        pb.push(".bak-t");
        let _ = fs::remove_file(&*pb);
    }

    fn find_state<'a>(
        sites: &'a [(String, SiteState)],
        name: &str,
    ) -> &'a SiteState {
        &sites.iter().find(|(n, _)| n == name).expect("site").1
    }

    #[test]
    fn offset_state_ok_and_patched() {
        let bytes = vec![0x02u8, 0, 0, 0];
        let db = db_of(Some(&md5_hex(&bytes)), offset_site());
        let r = analyze_bytes(Path::new("mem"), &bytes, "t", &db).unwrap();
        assert!(r.known_pristine);
        assert!(matches!(find_state(&r.sites, "slot0"), SiteState::Ok));

        let patched = vec![0x00u8, 0, 0, 0];
        let r = analyze_bytes(Path::new("mem"), &patched, "t", &db_of(Some(&md5_hex(&patched)), offset_site())).unwrap();
        assert!(matches!(find_state(&r.sites, "slot0"), SiteState::Patched));
    }

    #[test]
    fn offset_mismatch_and_eof() {
        let bytes = vec![0x99u8, 0, 0, 0];
        let db = db_of(Some(&md5_hex(&bytes)), offset_site());
        let r = analyze_bytes(Path::new("mem"), &bytes, "t", &db).unwrap();
        match find_state(&r.sites, "slot0") {
            SiteState::Mismatch { found } => assert_eq!(found, "99"),
            other => panic!("expected mismatch, got {other:?}"),
        }

        let short = vec![0x02u8, 0, 0];
        let db_shifted = db_of(Some(&md5_hex(&short)), serde_json::json!({
            "kind": "offset", "offset": "0x10", "orig": "02", "patch": "00", "name": "slot0"
        }));
        let r = analyze_bytes(Path::new("mem"), &short, "t", &db_shifted).unwrap();
        match find_state(&r.sites, "slot0") {
            SiteState::Mismatch { found } => assert_eq!(found, "eof"),
            other => panic!("expected eof, got {other:?}"),
        }
    }

    #[test]
    fn no_table_analyze_only() {
        let bytes = vec![0x02u8, 0, 0, 0];
        let db = db_of(Some(&md5_hex(&bytes)), offset_site());
        let r = analyze_bytes(Path::new("mem"), &bytes, "sem-goal", &db).unwrap();
        assert!(!r.known_pristine);
        assert!(r.sites.is_empty());
    }

    #[test]
    fn pattern_zero_matches_reject_apply() {
        let bytes = vec![0u8; 64];
        let db = db_of(None, pattern_site("AA 00 ?? FF", "AA 00 99 FF", "AB 00 CD FF"));
        let r = analyze_bytes(Path::new("mem"), &bytes, "t", &db).unwrap();
        match find_state(&r.sites, "sig") {
            SiteState::Mismatch { found } => assert_eq!(found, "0 matches"),
            other => panic!("expected 0 matches, got {other:?}"),
        }
    }

    #[test]
    fn pattern_multi_matches_reject_apply() {
        let mut bytes = vec![0u8; 64];
        for off in [10usize, 40] {
            bytes[off] = 0xAA;
            bytes[off + 1] = 0x00;
            bytes[off + 3] = 0xFF;
        }
        let db = db_of(None, pattern_site("AA 00 ?? FF", "AA 00 99 FF", "AB 00 CD FF"));
        let r = analyze_bytes(Path::new("mem"), &bytes, "t", &db).unwrap();
        match find_state(&r.sites, "sig") {
            SiteState::Mismatch { found } => assert_eq!(found, "2 matches"),
            other => panic!("expected multi matches, got {other:?}"),
        }
    }

    #[test]
    fn pattern_unique_apply_and_revert() {
        let mut bytes = vec![0u8; 64];
        bytes[30] = 0xAA;
        bytes[31] = 0x00;
        bytes[32] = 0x99;
        bytes[33] = 0xFF;
        let db = db_of(None, pattern_site("AA 00 ?? FF", "AA 00 99 FF", "AB 00 CD FF"));

        let pre = analyze_bytes(Path::new("mem"), &bytes, "t", &db).unwrap();
        assert!(matches!(find_state(&pre.sites, "sig"), SiteState::Ok));

        let path = node_created("pattern", &bytes);
        let first = apply(&path, "t", &db).unwrap();
        assert!(matches!(find_state(&first.sites, "sig"), SiteState::Patched));

        let err = apply(&path, "t", &db).unwrap_err();
        assert!(matches!(err, EngineError::Validation { n: 1 }));
        let wrote = fs::read(&path).unwrap();
        assert_eq!(&wrote[30..34], &[0xAB, 0x00, 0xCD, 0xFF]);

        revert(&path).unwrap();
        let reverted = fs::read(&path).unwrap();
        assert_eq!(
            &reverted[30..34],
            &[0xAA, 0x00, 0x99, 0xFF],
            "revert restored original bytes"
        );
        cleanup(&path);
    }

    #[test]
    fn offset_apply_idempotent_and_revert() {
        let bytes = vec![0x02u8, 0, 0, 0];
        let db = db_of(Some(&md5_hex(&bytes)), offset_site());

        let path = node_created("offset", &bytes);
        let first = apply(&path, "t", &db).unwrap();
        assert!(matches!(find_state(&first.sites, "slot0"), SiteState::Patched));
        let second = apply(&path, "t", &db).unwrap();
        assert!(matches!(find_state(&second.sites, "slot0"), SiteState::Patched));

        let wrote = fs::read(&path).unwrap();
        assert_eq!(wrote[0], 0x00);

        revert(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap()[0], 0x02);
        cleanup(&path);
    }

    #[test]
    fn offset_unexpected_bytes_abort_without_write() {
        let bytes = vec![0x99u8, 0, 0, 0];
        let db = db_of(Some(&md5_hex(&bytes)), offset_site());
        let path = node_created("invalid", &bytes);
        let err = apply(&path, "t", &db).unwrap_err();
        assert!(matches!(err, EngineError::Validation { n: 1 }));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        cleanup(&path);
    }

    #[test]
    fn apply_no_table_rejected() {
        let bytes = vec![0x02u8, 0, 0, 0];
        let db = db_of(Some(&md5_hex(&bytes)), offset_site());
        let path = node_created("notable", &bytes);
        let err = apply(&path, "sem-goal", &db).unwrap_err();
        assert!(matches!(err, EngineError::NoTable { .. }));
        cleanup(&path);
    }

    #[test]
    fn pattern_no_expect_checks_current_bytes() {
        let mut bytes = vec![0u8; 64];
        bytes[10] = 0xAA;
        bytes[11] = 0x00;
        bytes[13] = 0xFF;
        let db = db_of(
            None,
            serde_json::json!({
                "kind": "pattern",
                "name": "sig",
                "pattern": "AA 00 ?? FF",
                "patch": "AB 00 CD FF"
            }),
        );
        let r = analyze_bytes(Path::new("mem"), &bytes, "t", &db).unwrap();
        match find_state(&r.sites, "sig") {
            SiteState::Ok => {}
            other => panic!("expected Ok without expect, got {other:?}"),
        }
    }
}
