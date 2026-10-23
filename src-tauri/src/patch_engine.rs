//! Stereo patch engine: Patch-DB sites, verify-then-write, backup, revert.
//!
//! Tables come from [`crate::tables::PatchDb`]: a table with `binary_md5` is
//! surgical (fixed offset); a pattern-only table resolves via AOB signature
//! with a unique match and `expect`. A binary without a resolvable table is
//! never written.

use std::collections::HashSet;
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
    /// Bytes match `orig` - patchable.
    Ok,
    /// Bytes match `patch` - already applied.
    Patched,
    /// Bytes differ from both - binary mismatch at this site.
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
    /// Expected bytes per site (hex), for divergence reports.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub expected: Vec<(String, String)>,
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

/// Table applicable to (goal, md5): the exact-md5 table first, then the
/// same-goal table whose md5 matches the binary with every site's patch
/// already applied (recognizes a patched binary, keeping apply idempotent
/// and analyze truthful). A pattern-only fallback comes last - an offset
/// table carries a fixed file offset that is meaningless in a different
/// binary, so it must never be tried against a foreign md5 (that produced
/// wall-of-"diverged" analyses).
fn table_for<'a>(db: &'a PatchDb, goal: &str, md5: &str, bytes: &[u8]) -> Option<&'a Table> {
    if let Some(t) = db
        .tables
        .iter()
        .find(|t| t.goal == goal && t.binary_md5.as_deref() == Some(md5))
    {
        return Some(t);
    }
    for t in db.tables.iter().filter(|t| t.goal == goal && t.binary_md5.is_some()) {
        if patched_md5(t, bytes).as_deref() == Some(md5) {
            return Some(t);
        }
    }
    db.tables
        .iter()
        .find(|t| t.goal == goal && t.binary_md5.is_none())
}

/// md5 of the binary with every offset site's patch applied in memory;
/// None when any site falls outside the binary.
fn patched_md5(table: &Table, bytes: &[u8]) -> Option<String> {
    let mut copy = bytes.to_vec();
    for site in &table.sites {
        let TableSite::Offset { offset, patch, .. } = site else {
            return None;
        };
        let start = u32::from_str_radix(offset.trim_start_matches("0x"), 16).ok()?;
        let patch = decode_hex(patch, "site").ok()?;
        let end = start as usize + patch.len();
        copy.get_mut(start as usize..end)?;
        copy[start as usize..end].copy_from_slice(&patch);
    }
    Some(md5_hex(&copy))
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
    let table = table_for(db, goal, &md5, bytes);
    let resolved = table.map(|t| resolve_sites(t, bytes)).transpose()?;
    let sites = resolved
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|r| (r.name.clone(), site_state(r, bytes)))
        .collect();
    let expected = resolved
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|r| (r.name.clone(), hex::encode(&r.orig)))
        .collect();
    Ok(Analysis {
        path: node.to_path_buf(),
        size: bytes.len() as u64,
        md5,
        known_pristine: table.is_some(),
        sites,
        expected,
    })
}

/// Per-site relocation evidence for divergence reports.
#[derive(Debug, Clone)]
pub struct TracedSite {
    pub name: String,
    pub state: SiteState,
    pub orig: Vec<u8>,
    pub patch: Vec<u8>,
    /// Resolved offset (source table or scan hit).
    pub offset: Option<u32>,
    /// Relocation match, mismatch sites only.
    pub new_offset: Option<u32>,
    pub unique: bool,
}

/// Server-side recomputed divergence evidence: per-site states with
/// relocation data, independent of what the UI observed.
#[derive(Debug)]
pub struct DivergenceTrace {
    /// A source table existed (drives `table_kind`).
    pub table_matched: bool,
    /// `binary_md5` of the source table, if any.
    pub source_md5: Option<String>,
    pub sites: Vec<TracedSite>,
}

/// Recompute a goal's site evidence for a divergence report.
///
/// With a source table, every resolved site carries its state and, for a
/// byte-level mismatch, a relocation search of the expected bytes - so the
/// report pinpoints where the site moved. Without a source table (new
/// client builds), the goal's tables are scanned directly: every site is
/// an exact-byte or AOB search over the unknown binary.
///
/// # Errors
/// [`EngineError::Hex`] if a table entry is malformed (bug in the DB).
pub fn divergence_trace(
    node: &Path,
    bytes: &[u8],
    goal: &str,
    db: &PatchDb,
) -> Result<DivergenceTrace, EngineError> {
    let md5 = md5_hex(bytes);
    let table = table_for(db, goal, &md5, bytes);
    let _ = analyze_bytes(node, bytes, goal, db)?;

    let sites = if let Some(t) = table {
        resolve_sites(t, bytes)?
            .into_iter()
            .map(|r| {
                let state = site_state(&r, bytes);
                let offset = if r.reason.is_none() { Some(r.offset) } else { None };
                let (new_offset, unique) = if r.reason.is_none()
                    && matches!(state, SiteState::Mismatch { .. })
                    && !r.orig.is_empty()
                {
                    let aob = Aob {
                        pat: r.orig.clone(),
                        mask: vec![true; r.orig.len()],
                    };
                    match scan_pattern(bytes, &aob) {
                        Some(pos) => (Some(pos as u32), true),
                        None => (None, false),
                    }
                } else {
                    (None, false)
                };
                TracedSite {
                    name: r.name,
                    state,
                    orig: r.orig,
                    patch: r.patch,
                    offset,
                    new_offset,
                    unique,
                }
            })
            .collect()
    } else {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for t in db.tables.iter().filter(|t| t.goal == goal) {
            for site in &t.sites {
                if !seen.insert(site.name().to_owned()) {
                    continue;
                }
                match site {
                    TableSite::Offset { orig, patch, name, .. } => {
                        let orig_bytes = decode_hex(orig, name)?;
                        let patch_bytes = decode_hex(patch, name)?;
                        let aob = Aob {
                            pat: orig_bytes.clone(),
                            mask: vec![true; orig_bytes.len()],
                        };
                        let hit = scan_pattern(bytes, &aob).map(|pos| pos as u32);
                        out.push(TracedSite {
                            name: name.clone(),
                            state: SiteState::Mismatch { found: String::new() },
                            orig: orig_bytes,
                            patch: patch_bytes,
                            offset: hit,
                            new_offset: hit,
                            unique: hit.is_some(),
                        });
                    }
                    TableSite::Pattern { name, pattern, patch, expect } => {
                        let patch_bytes = decode_hex(patch, name)?;
                        let hit = parse_aob(pattern)
                            .and_then(|aob| scan_pattern(bytes, &aob))
                            .map(|pos| pos as u32);
                        let orig = match (hit, expect) {
                            (Some(_), Some(e)) => decode_hex(e, name)?,
                            (Some(pos), None) => {
                                let end = pos as usize + patch_bytes.len();
                                bytes.get(pos as usize..end).map(<[u8]>::to_vec).unwrap_or_default()
                            }
                            (None, _) => Vec::new(),
                        };
                        out.push(TracedSite {
                            name: name.clone(),
                            state: SiteState::Mismatch { found: String::new() },
                            orig,
                            patch: patch_bytes,
                            offset: hit,
                            new_offset: hit,
                            unique: hit.is_some(),
                        });
                    }
                }
            }
        }
        out
    };

    Ok(DivergenceTrace {
        table_matched: table.is_some(),
        source_md5: table.and_then(|t| t.binary_md5.clone()),
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
    let Some(table) = table_for(db, goal, &md5, &bytes) else {
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
        .clone()
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
        expected: analysis.expected,
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
            "goals": [{ "id": "t", "title": { "pt": "T", "en": "T" }, "description": { "pt": "d", "en": "d" } }],
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

    #[test]
    fn divergence_trace_relocates_shifted_site() {
        let mut bytes = vec![0u8; 64];
        let a = 10usize;
        bytes[a..a + 9].copy_from_slice(&[0x90, 0x90, 0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0x00]);
        // Orig must appear exactly once, relocated to B.
        let b = 40usize;
        bytes[b..b + 7].copy_from_slice(&[0x49, 0x39, 0xd4, 0x4c, 0x0f, 0x43, 0xe2]);
        let db = db_of(
            None,
            pattern_site("90 90 ?? ?? ?? ?? ?? ?? ??", "49 39 d4 4c 0f 43 e2", "49 c7 c4 02 00 00"),
        );
        let trace = divergence_trace(Path::new("mem"), &bytes, "t", &db).unwrap();
        assert!(trace.table_matched);
        let site = &trace.sites[0];
        assert!(matches!(site.state, SiteState::Mismatch { .. }), "got {:?}", site.state);
        assert_eq!(site.new_offset, Some(b as u32));
        assert!(site.unique);
    }

    #[test]
    fn divergence_trace_ambiguous_orig_stays_manual() {
        let mut bytes = vec![0u8; 64];
        let a = 10usize;
        bytes[a..a + 9].copy_from_slice(&[0x90, 0x90, 0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0x00]);
        // Orig appears twice: relocation is ambiguous.
        let b = 40usize;
        let c = 52usize;
        for off in [b, c] {
            bytes[off..off + 7].copy_from_slice(&[0x49, 0x39, 0xd4, 0x4c, 0x0f, 0x43, 0xe2]);
        }
        let db = db_of(
            None,
            pattern_site("90 90 ?? ?? ?? ?? ?? ?? ??", "49 39 d4 4c 0f 43 e2", "49 c7 c4 02 00 00"),
        );
        let trace = divergence_trace(Path::new("mem"), &bytes, "t", &db).unwrap();
        let site = &trace.sites[0];
        assert!(matches!(site.state, SiteState::Mismatch { .. }), "got {:?}", site.state);
        assert_eq!(site.new_offset, None);
        assert!(!site.unique);
    }

    #[test]
    fn divergence_trace_scans_unknown_binary() {
        let bytes = vec![0x03u8, 0x00, 0x02, 0x00];
        let db = db_of(
            Some(&"0".repeat(32)),
            serde_json::json!({
                "kind": "offset", "offset": "0x00", "orig": "02", "patch": "01", "name": "slot0"
            }),
        );
        let trace = divergence_trace(Path::new("mem"), &bytes, "t", &db).unwrap();
        assert!(!trace.table_matched);
        assert_eq!(trace.source_md5, None);
        assert!(analyze_bytes(Path::new("mem"), &bytes, "t", &db).unwrap().sites.is_empty(),
            "wire sites stay empty in scan mode");
        let site = &trace.sites[0];
        assert_eq!(site.name, "slot0");
        assert_eq!(site.offset, Some(2));
        assert_eq!(site.new_offset, Some(2));
        assert!(site.unique);
        assert_eq!(site.orig, vec![0x02]);
        assert_eq!(site.patch, vec![0x01]);
    }

    /// Dev-machine smoke: the real v5-patched node must NOT produce a wall
    /// of diverged sites (the bug this fix targets). Skips when absent.
    #[test]
    fn real_v5_node_analyze_is_not_wall_of_diverged() {
        let node = Path::new(
            "/home/rafael/.config/discord/app-1.0.161/modules/discord_voice-1/discord_voice/discord_voice.node",
        );
        let Ok(bytes) = fs::read(node) else {
            eprintln!("skipping: no real node on this machine");
            return;
        };
        let db_json = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("patchdb/tables.json");
        let Ok(db_json) = fs::read_to_string(db_json) else {
            eprintln!("skipping: no patchdb");
            return;
        };
        let db = crate::tables::parse_db(&db_json).unwrap();
        for goal in ["v3", "v4", "v5"] {
            let r = analyze_bytes(node, &bytes, goal, &db).unwrap();
            let mismatches = r
                .sites
                .iter()
                .filter(|(_, s)| matches!(s, SiteState::Mismatch { .. }))
                .count();
            assert_eq!(mismatches, 0, "{goal}: wall of diverged sites came back");
        }
    }
}
