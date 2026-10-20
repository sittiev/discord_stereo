//! Stereo patch engine: goal tables, verify-then-write, backup, revert, status.
//!
//! Sites validated against Discord app-1.0.161 linux x64
//! (`discord_voice.node`, MD5 `0bd81054ee44228a03928e84044a4d32`).
//! Unknown binaries are never patched — `analyze` reports them instead.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A single byte-range patch site.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Site {
    /// File offset (== VA in the validated ELF build).
    pub offset: u32,
    /// Expected original bytes (hex).
    pub orig: String,
    /// Patched bytes (hex).
    pub patch: String,
    /// Human-readable site name.
    pub name: String,
}

/// Patch goal: which table to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Goal {
    /// v3 — stereo core + dead filters (12 sites).
    #[serde(rename = "v3")]
    V3,
    /// v4 — full stereo, 510 kbps CBR lock (18 sites). Recommended.
    #[serde(rename = "v4")]
    V4,
    /// v5 — v4 + Opus kAudio: deeper stereo, lower spectral centroid (19 sites).
    #[serde(rename = "v5")]
    V5,
}

impl Goal {
    /// All selectable goals, in wizard order.
    pub const ALL: [Goal; 3] = [Goal::V3, Goal::V4, Goal::V5];

    /// Wire id / serde tag exchanged with the webview.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Goal::V3 => "v3",
            Goal::V4 => "v4",
            Goal::V5 => "v5",
        }
    }

    /// Marketing title shown in the UI.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Goal::V3 => "Filtros Removido",
            Goal::V4 => "Stereo Full",
            Goal::V5 => "Stereo Profundo",
        }
    }

    /// One-line description shown in the UI.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Goal::V3 => "Stereo core sem krisp/highpass/downmix; bitrate do servidor (~64k+boost).",
            Goal::V4 => {
                "Recomendado: v3 + 510 kbps CBR travado + force-stereo + RemixAndResample fix. Melhor CENTROID no bench."
            }
            Goal::V5 => {
                "v4 + Opus kAudio: mais profundidade e cantos; centroide espectral mais baixo (trade-off do modo música)."
            }
        }
    }
}

/// Build a [`Site`] from hex strings (plain fn: `to_owned` is not const).
fn site(offset: u32, orig: &str, patch: &str, name: &str) -> Site {
    Site {
        offset,
        orig: orig.to_owned(),
        patch: patch.to_owned(),
        name: name.to_owned(),
    }
}

/// v3 base table (12 sites).
#[must_use]
pub fn v3_sites() -> Vec<Site> {
    vec![
        site(0x003CB8D0, "02", "00", "StereoCheck1 CommitAudioCodec"),
        site(0x003C76D5, "02", "00", "StereoCheck2 CreateAudioStream"),
        site(
            0x003BF2E0,
            "4939d44c0f43e2",
            "49c7c402000000",
            "CreateAudioFrame clamp1->2",
        ),
        site(
            0x003BF2EF,
            "4939d44c0f43e2",
            "49c7c402000000",
            "CreateAudioFrame clamp2->2",
        ),
        site(
            0x0038C075,
            "84c0740d83bb30020000090f8f",
            "909090909090909090909090e9",
            "MonoDownmix bypass (NOP+JMP)",
        ),
        site(0x0099F7DD, "01", "02", "MCOpusImpl ctor num_channels"),
        site(0x009E3629, "01", "02", "SdpToConfig num_channels"),
        site(0x009CCF80, "55", "c3", "downmix_and_resample ret"),
        site(0x00696220, "55", "c3", "HighPassFilter::Process ret"),
        site(
            0x009E3190,
            "554889e58b0f31c0",
            "48c7c001000000c3",
            "OpusConfig IsOk true",
        ),
        site(0x00376240, "55", "c3", "NoiseCanceller::Process ret"),
        site(
            0x0038CC20,
            "0fb687f80100002401c3",
            "31c0c390909090909090",
            "ShouldRunNoiseCancellation false",
        ),
    ]
}

/// v4 = v3 + bitrate/force/CBR/remix (18 sites).
#[must_use]
pub fn v4_sites() -> Vec<Site> {
    let mut sites = v3_sites();
    sites.extend([
        site(
            0x0099F7E7,
            "007d0000",
            "30c80700",
            "MCOpusImpl ctor bitrate 510k",
        ),
        site(
            0x009E3633,
            "007d0000",
            "30c80700",
            "SdpToConfig bitrate 510k",
        ),
        site(
            0x006A2135,
            "554889e589f2",
            "55ba30c80700",
            "WebRtcOpus_SetBitRate lock 510k",
        ),
        site(
            0x006A2525,
            "4883fa02772e",
            "ba0200000090",
            "SetForceChannels->2",
        ),
        site(0x006A23E0, "48", "c3", "DisableCbr->ret"),
        site(0x00666CE3, "765c", "eb5c", "RemixAndResample skip downmix"),
    ]);
    sites
}

/// v5 = v4 + kAudio (19 sites).
#[must_use]
pub fn v5_sites() -> Vec<Site> {
    let mut sites = v4_sites();
    sites.push(site(
        0x009E3682,
        "00",
        "01",
        "OpusConfig application kAudio",
    ));
    sites
}

/// Table for a goal.
#[must_use]
pub fn sites_for(goal: Goal) -> Vec<Site> {
    match goal {
        Goal::V3 => v3_sites(),
        Goal::V4 => v4_sites(),
        Goal::V5 => v5_sites(),
    }
}

/// MD5 of the pristine validated node (app-1.0.161 linux x64).
pub const KNOWN_PRISTINE_MD5: &str = "0bd81054ee44228a03928e84044a4d32";

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
    /// Whether this MD5 is the known-good pristine table base.
    pub known_pristine: bool,
    /// Per-site states for the selected goal.
    pub sites: Vec<(String, SiteState)>,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("hex decode em {site}: {source}")]
    Hex {
        site: String,
        source: hex::FromHexError,
    },
    #[error("validacao falhou em {n} site(s); nada foi escrito")]
    Validation { n: usize },
    #[error("nenhum backup encontrado para reverter")]
    NoBackup,
}

fn decode_hex(s: &str, site_name: &str) -> Result<Vec<u8>, EngineError> {
    hex::decode(s).map_err(|source| EngineError::Hex {
        site: site_name.to_owned(),
        source,
    })
}

/// Analyze a node against a goal without writing anything.
///
/// # Errors
/// Returns [`EngineError::Io`] if the node cannot be read, or
/// [`EngineError::Hex`] if a table entry is malformed (bug in tables).
pub fn analyze(node: &Path, goal: Goal) -> Result<Analysis, EngineError> {
    let bytes = fs::read(node)?;
    analyze_bytes(node, &bytes, goal)
}

fn analyze_bytes(node: &Path, bytes: &[u8], goal: Goal) -> Result<Analysis, EngineError> {
    let md5 = md5_hex(bytes);
    let mut sites = Vec::with_capacity(sites_for(goal).len());

    for s in sites_for(goal) {
        let orig = decode_hex(&s.orig, &s.name)?;
        let patch = decode_hex(&s.patch, &s.name)?;
        let end = s.offset as usize + orig.len().max(patch.len());
        let state = if end > bytes.len() {
            SiteState::Mismatch {
                found: "eof".to_owned(),
            }
        } else {
            let cur = &bytes[s.offset as usize..end];
            if cur == orig.as_slice() {
                SiteState::Ok
            } else if cur == patch.as_slice() {
                SiteState::Patched
            } else {
                SiteState::Mismatch {
                    found: hex::encode(cur),
                }
            }
        };
        sites.push((s.name, state));
    }

    let known_pristine = md5 == KNOWN_PRISTINE_MD5;
    Ok(Analysis {
        path: node.to_path_buf(),
        size: bytes.len() as u64,
        md5,
        known_pristine,
        sites,
    })
}

/// Apply a goal to a node: verify-then-write with backup.
///
/// Idempotent: already-patched sites are skipped. Any unknown-byte site
/// aborts the whole operation before a single byte is written.
///
/// # Errors
/// - [`EngineError::Validation`]: `n` sites do not match `orig` nor `patch`.
/// - [`EngineError::Io`]: the node cannot be read or written.
pub fn apply(node: &Path, goal: Goal) -> Result<Analysis, EngineError> {
    let mut bytes = fs::read(node)?;
    let analysis = analyze_bytes(node, &bytes, goal)?;
    let mismatched = analysis
        .sites
        .iter()
        .filter(|(_, st)| matches!(st, SiteState::Mismatch { .. }))
        .count();
    if mismatched > 0 {
        return Err(EngineError::Validation { n: mismatched });
    }

    let mut todo: Vec<(u32, Vec<u8>)> = Vec::new();
    for s in sites_for(goal) {
        let patch = decode_hex(&s.patch, &s.name)?;
        let cur = &bytes[s.offset as usize..s.offset as usize + patch.len()];
        if cur != patch.as_slice() {
            todo.push((s.offset, patch));
        }
    }
    if todo.is_empty() {
        return Ok(analysis);
    }

    let backup = backup_path(node, goal);
    if !backup.exists() {
        fs::copy(node, &backup)?;
    }
    for (offset, patch) in &todo {
        let off = *offset as usize;
        bytes[off..off + patch.len()].copy_from_slice(patch);
    }
    fs::write(node, &bytes)?;
    let final_state = analyze_bytes(node, &bytes, goal)?;
    debug_assert_eq!(
        final_state
            .sites
            .iter()
            .filter(|(_, st)| matches!(st, SiteState::Ok))
            .count(),
        0,
        "apply left unpatched sites behind; verify-then-write invariant broken"
    );
    Ok(final_state)
}

/// Revert a node from the most recent `.bak-*` backup.
///
/// # Errors
/// [`EngineError::NoBackup`] when no backup exists.
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
        let rest = name.strip_prefix(&format!("{stem}{ext}.bak-"));
        if let Some(_goal) = rest {
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

fn backup_path(node: &Path, goal: Goal) -> PathBuf {
    let mut p = node.as_os_str().to_os_string();
    p.push(format!(".bak-{}", goal.id()));
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
