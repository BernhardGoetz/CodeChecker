//! Model management: fixed catalog (Qwen2.5-Coder 7B, pinned to a Hugging
//! Face revision with SHA-256 and size), hardware check (enough RAM for it),
//! download with resume and hash check, import of local `.gguf` files,
//! selection (persisted in `ai.json` in the app config directory).
//!
//! The download is the only network access of the AI feature. It accepts
//! nothing but https URLs on huggingface.co and Hugging Face's own CDN hosts
//! (`*.huggingface.co`, `hf.co`, `*.hf.co`), also for redirects.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use super::{hex, llama, AiError};

pub const CONFIG_FILE: &str = "ai.json";
pub const MODELS_SUBDIR: &str = "models";
/// Files imported for an unpinned catalog entry (no hash to check against)
/// live in `models/unverified/<catalog id>/<original name>`: separate from
/// the verified catalog files, and with their original names so llama.cpp
/// finds the other parts of a split GGUF.
pub const UNVERIFIED_SUBDIR: &str = "unverified";
/// Below this much RAM the 7B model (about 4.7 GB, plus context, Docker
/// and the system) does not fit: the AI feature is not available. OS-reported
/// RAM of a "12 GB" machine is ~11.7 GiB, of an "8 GB" one ~7.7 GiB.
pub const MIN_AI_RAM_BYTES: u64 = 11 * GIB;
pub const LARGE_MODEL: &str = "qwen2.5-coder-7b";
/// Catalog entries the app offers; other entries of `catalog.json` (e.g. a
/// removed model before the catalog is pinned again) are ignored. The same
/// list is in `scripts/pin-models.sh` (`SUPPORTED_MODELS`).
pub const SUPPORTED_MODELS: &[&str] = &[LARGE_MODEL];
/// Upper bound for imported files.
pub const MAX_IMPORT_BYTES: u64 = 32 * GIB;
/// Solutions per task for imported (unknown) models.
const IMPORTED_SOLUTIONS: u32 = 3;
const GIB: u64 = 1024 * 1024 * 1024;
const CHUNK: usize = 8 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
const MAX_REDIRECTS: usize = 10;

static CATALOG_JSON: &str = include_str!("catalog.json");
static CATALOG: OnceLock<Vec<CatalogModel>> = OnceLock::new();
static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
/// Serializes read-modify-write of `ai.json`.
static CONFIG_LOCK: Mutex<()> = Mutex::new(());
static TRANSFER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Sets the app config and data directories (called once at app start).
pub fn init(config_dir: PathBuf, data_dir: PathBuf) {
    let _ = CONFIG_DIR.set(config_dir);
    let _ = DATA_DIR.set(data_dir);
}

// ---------------------------------------------------------------------------
// Catalog

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    pub repo: String,
    pub quant: String,
    pub license: String,
    /// Candidate solutions generated per task.
    pub solutions: u32,
    /// Commit SHA of the Hugging Face repo.
    pub revision: String,
    /// One file, or the parts of a split GGUF (first part is loaded).
    /// Older pins may list both the single file and its parts; only
    /// [`effective_files`] is downloaded.
    pub files: Vec<CatalogFile>,
}

#[derive(Deserialize)]
struct CatalogRoot {
    models: Vec<CatalogModel>,
}

pub fn parse_catalog(json: &str) -> Result<Vec<CatalogModel>, String> {
    serde_json::from_str::<CatalogRoot>(json)
        .map(|c| c.models)
        .map_err(|e| e.to_string())
}

/// The supported entries of a catalog ([`SUPPORTED_MODELS`]), in order.
pub fn supported(models: Vec<CatalogModel>) -> Vec<CatalogModel> {
    models
        .into_iter()
        .filter(|m| SUPPORTED_MODELS.contains(&m.id.as_str()))
        .collect()
}

pub fn catalog() -> &'static [CatalogModel] {
    CATALOG.get_or_init(|| supported(parse_catalog(CATALOG_JSON).expect("catalog.json is valid")))
}

/// A model the app no longer supports: users who downloaded it get a note
/// and can delete it; it is never used or selected again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetiredModel {
    pub id: &'static str,
    pub name: &'static str,
    /// Files the download stored in the models directory.
    pub files: &'static [&'static str],
    /// Their SHA-256 (an import of such a file is refused).
    pub sha256: &'static [&'static str],
}

/// The 1.5B model was removed in 1.1.1: on tasks without input it was right
/// in 2 of 12 runs (KI-Benchmark 1.1.1); the 7B model is the only one.
pub const RETIRED_MODELS: &[RetiredModel] = &[RetiredModel {
    id: "qwen2.5-coder-1.5b",
    name: "Qwen2.5-Coder 1.5B Instruct (Q5_K_M)",
    files: &["qwen2.5-coder-1.5b-instruct-q5_k_m.gguf"],
    sha256: &["f15d02e8e51a5c6f9448b972819acfa66aee4d8bb7d881a8b8ba3d90da08ef09"],
}];

pub fn retired(id: &str) -> Option<&'static RetiredModel> {
    RETIRED_MODELS.iter().find(|r| r.id == id)
}

/// German note for a retired model.
pub fn retired_note(r: &RetiredModel) -> String {
    format!(
        "„{}“ wird nicht mehr unterstützt: Es lieferte zu oft falsche Testfälle. Die KI \
         verwendet nur noch das 7B-Modell.",
        r.name
    )
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The files of a catalog entry that make up the model: the single file if
/// there is exactly one, otherwise the parts of the split GGUF. Pins made
/// before the fix in `scripts/pin-models.sh` list both variants of the same
/// model (single file and its parts); then only the single file counts.
/// Several single files (not a valid pin) are returned unchanged.
pub fn effective_files(files: &[CatalogFile]) -> Vec<&CatalogFile> {
    let singles: Vec<&CatalogFile> = files
        .iter()
        .filter(|f| split_part(&f.name).is_none())
        .collect();
    if singles.len() == 1 {
        singles
    } else {
        files.iter().collect()
    }
}

/// Every complete variant of a catalog entry, preferred one first: the
/// effective files and, for an entry listing both, the split parts (an
/// earlier download of the parts stays usable).
fn variants(files: &[CatalogFile]) -> Vec<Vec<&CatalogFile>> {
    let effective = effective_files(files);
    let parts: Vec<&CatalogFile> = files
        .iter()
        .filter(|f| split_part(&f.name).is_some())
        .collect();
    let mut v = vec![effective];
    if !parts.is_empty() && v[0].len() == 1 && split_part(&v[0][0].name).is_none() {
        let names: Vec<&str> = parts.iter().map(|f| f.name.as_str()).collect();
        if matches!(part_set(&names), PartSet::Complete { .. }) {
            v.push(parts);
        }
    }
    v
}

/// The file llama.cpp loads from a variant: the single file or part 1.
fn start_file<'a>(files: &[&'a CatalogFile]) -> Option<&'a CatalogFile> {
    files
        .iter()
        .copied()
        .find(|f| split_part(&f.name).is_some_and(|(_, i, _)| i == 1))
        .or_else(|| files.first().copied())
}

impl CatalogModel {
    /// Revision, sizes and hashes are filled in (`scripts/pin-models.sh`);
    /// at most one single file (an additional set of parts is tolerated,
    /// see [`effective_files`]).
    pub fn is_pinned(&self) -> bool {
        is_hex(&self.revision, 40)
            && !self.files.is_empty()
            && self
                .files
                .iter()
                .filter(|f| split_part(&f.name).is_none())
                .count()
                <= 1
            && self
                .files
                .iter()
                .all(|f| f.size > 0 && is_hex(&f.sha256, 64) && safe_file_name(&f.name))
    }

    /// Download size (effective files only).
    pub fn size(&self) -> u64 {
        effective_files(&self.files).iter().map(|f| f.size).sum()
    }

    pub fn url(&self, file: &CatalogFile) -> String {
        download_url(&self.repo, &self.revision, &file.name)
    }
}

pub fn download_url(repo: &str, revision: &str, file: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/{revision}/{file}")
}

/// A plain file name (no path separators, no dot files), ending in `.gguf`.
pub fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':'])
        && name.to_ascii_lowercase().ends_with(".gguf")
}

/// File header of every GGUF file.
pub fn is_gguf_magic(head: &[u8]) -> bool {
    head.starts_with(b"GGUF")
}

/// The file name contains the expected quantization (e.g. `q4_k_m`).
pub fn name_matches_quant(name: &str, quant: &str) -> bool {
    name.to_ascii_lowercase()
        .contains(&quant.to_ascii_lowercase())
}

/// Part of a split GGUF (`<prefix>-00001-of-00003.gguf`, the naming of
/// llama.cpp's gguf-split): `(prefix, index, count)`.
pub fn split_part(name: &str) -> Option<(&str, u32, u32)> {
    const TAIL: usize = "-00001-of-00003".len();
    let lower = name.to_ascii_lowercase();
    if !lower.ends_with(".gguf") || name.len() < 5 + TAIL + 1 {
        return None;
    }
    let stem = name.get(..name.len() - 5)?;
    let tail = stem.get(stem.len() - TAIL..)?;
    let b = tail.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if b[0] != b'-' || !digits(1..6) || &b[6..10] != b"-of-" || !digits(10..15) {
        return None;
    }
    let index: u32 = tail[1..6].parse().ok()?;
    let count: u32 = tail[10..15].parse().ok()?;
    (index >= 1 && index <= count).then_some((&stem[..stem.len() - TAIL], index, count))
}

/// Completeness of the files imported for one model.
#[derive(Debug, PartialEq)]
pub enum PartSet {
    /// Everything is there; `first` is the file llama.cpp loads.
    Complete { first: String },
    /// File names of the parts still missing (empty: no file at all).
    Missing(Vec<String>),
    /// The files do not belong together (German reason).
    Invalid(String),
}

pub fn part_set(names: &[&str]) -> PartSet {
    let mut names: Vec<&str> = names.to_vec();
    names.sort_unstable();
    names.dedup();
    let parts: Vec<_> = names.iter().filter_map(|n| split_part(n)).collect();
    if parts.is_empty() {
        return match names.as_slice() {
            [] => PartSet::Missing(Vec::new()),
            [one] => PartSet::Complete {
                first: one.to_string(),
            },
            _ => PartSet::Invalid(
                "Mehrere einzelne .gguf-Dateien gewählt – zu einem Modell gehört genau eine \
                 Datei oder alle Teile einer aufgeteilten Datei (…-00001-of-0000N.gguf)."
                    .into(),
            ),
        };
    }
    let (prefix, _, count) = parts[0];
    if parts.len() != names.len() || parts.iter().any(|&(p, _, c)| p != prefix || c != count) {
        return PartSet::Invalid(
            "Die gewählten Dateien gehören nicht zur selben aufgeteilten Datei.".into(),
        );
    }
    let name = |i: u32| format!("{prefix}-{i:05}-of-{count:05}.gguf");
    let missing: Vec<String> = (1..=count)
        .filter(|i| !parts.iter().any(|&(_, idx, _)| idx == *i))
        .map(name)
        .collect();
    if missing.is_empty() {
        PartSet::Complete { first: name(1) }
    } else {
        PartSet::Missing(missing)
    }
}

/// Warnings (German) for files chosen for catalog entry `m`.
pub fn name_warnings(m: &CatalogModel, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .filter(|n| !name_matches_quant(n, &m.quant))
        .map(|n| {
            format!(
                "„{n}“ enthält nicht „{}“ im Namen – vermutlich nicht die richtige Datei für \
                 „{}“ (erwartet: *{}*.gguf).",
                m.quant, m.name, m.quant
            )
        })
        .collect()
}

/// Hosts the model download may talk to (also after redirects).
pub fn allowed_url(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" || url.port().is_some_and(|p| p != 443) {
        return false;
    }
    match url.host_str() {
        Some(h) => {
            let h = h.to_ascii_lowercase();
            h == "huggingface.co"
                || h.ends_with(".huggingface.co")
                || h == "hf.co"
                || h.ends_with(".hf.co")
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Hardware

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hardware {
    pub total_ram_bytes: u64,
    pub physical_cores: usize,
    pub apple_silicon: bool,
    /// `None` on non-x86 CPUs.
    pub avx2: Option<bool>,
    /// Enough RAM for the 7B model (or the RAM is unknown). Otherwise the
    /// AI feature is not available; checking code without it still is.
    pub ai_available: bool,
    /// `None` if the AI is not available on this machine.
    pub suggested_model: Option<String>,
    /// German notes (slow CPU, too little RAM).
    pub warnings: Vec<String>,
}

/// Whether the AI can run with this much RAM (0 = unknown: not blocked).
pub fn ai_available(total_ram_bytes: u64) -> bool {
    total_ram_bytes == 0 || total_ram_bytes >= MIN_AI_RAM_BYTES
}

/// The 7B model if it fits into the RAM, else none.
pub fn suggest(total_ram_bytes: u64) -> Option<&'static str> {
    ai_available(total_ram_bytes).then_some(LARGE_MODEL)
}

/// German message when the RAM is too small for the 7B model.
pub fn ram_too_small(total_ram_bytes: u64) -> String {
    format!(
        "Die KI-Funktion ist auf diesem Rechner nicht verfügbar: Das KI-Modell braucht \
         mindestens 12 GB Arbeitsspeicher, dieser Rechner hat {} GB. Code prüfen und eigene \
         Testfälle funktionieren weiterhin ohne KI.",
        gib(total_ram_bytes)
    )
}

/// Error if the AI cannot run on this machine (too little RAM).
fn ensure_ai_available() -> Result<(), AiError> {
    let ram = total_ram();
    if ai_available(ram) {
        Ok(())
    } else {
        Err(AiError::Other(ram_too_small(ram)))
    }
}

pub fn gib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / GIB as f64).replace('.', ",")
}

pub fn hardware_warnings(total_ram_bytes: u64, avx2: Option<bool>) -> Vec<String> {
    let mut w = Vec::new();
    if avx2 == Some(false) {
        w.push(
            "Der Prozessor unterstützt kein AVX2 – die KI läuft damit sehr langsam.".to_string(),
        );
    }
    if !ai_available(total_ram_bytes) {
        w.push(ram_too_small(total_ram_bytes));
    }
    w
}

fn detect_avx2() -> Option<bool> {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        Some(std::arch::is_x86_feature_detected!("avx2"))
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        None
    }
}

/// Total RAM in bytes; 0 = unknown (never blocks, see [`ai_available`]).
fn total_ram() -> u64 {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    System::new_with_specifics(RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()))
        .map(|sys| sys.total_memory())
        .unwrap_or(0)
}

pub fn hardware() -> Hardware {
    let ram = total_ram();
    let avx2 = detect_avx2();
    Hardware {
        total_ram_bytes: ram,
        physical_cores: llama::thread_count(),
        apple_silicon: cfg!(all(target_os = "macos", target_arch = "aarch64")),
        avx2,
        ai_available: ai_available(ram),
        suggested_model: suggest(ram).map(str::to_string),
        warnings: hardware_warnings(ram, avx2),
    }
}

// ---------------------------------------------------------------------------
// Config (selection, imported models)

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedModel {
    /// `import-<first 12 hex of sha256>`.
    pub id: String,
    /// Original file name (display).
    pub name: String,
    /// File name in the models directory.
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
    /// Imported via the button of this (unpinned) catalog entry: not
    /// hash-verified, stored in `unverified/<catalog id>/<name>` and shown
    /// under that entry instead of as an unknown model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    #[serde(default)]
    pub selected: Option<String>,
    #[serde(default)]
    pub imported: Vec<ImportedModel>,
}

pub fn parse_config(json: &str) -> AiConfig {
    serde_json::from_str(json).unwrap_or_default()
}

fn read_config_from(dir: &Path) -> AiConfig {
    std::fs::read_to_string(dir.join(CONFIG_FILE))
        .map(|s| parse_config(&s))
        .unwrap_or_default()
}

fn write_config_to(dir: &Path, c: &AiConfig) -> Result<(), AiError> {
    let json = serde_json::to_string_pretty(c).map_err(|e| AiError::Other(e.to_string()))?;
    std::fs::create_dir_all(dir)
        .and_then(|_| std::fs::write(dir.join(CONFIG_FILE), json))
        .map_err(|e| AiError::Other(format!("Einstellung konnte nicht gespeichert werden: {e}")))
}

fn config_dir() -> Result<&'static Path, AiError> {
    CONFIG_DIR
        .get()
        .map(PathBuf::as_path)
        .ok_or_else(|| AiError::Other("Konfigurationsordner ist nicht bekannt.".into()))
}

fn read_config() -> AiConfig {
    CONFIG_DIR
        .get()
        .map(|d| read_config_from(d))
        .unwrap_or_default()
}

fn update_config(f: impl FnOnce(&mut AiConfig)) -> Result<AiConfig, AiError> {
    let dir = config_dir()?;
    let _lock = CONFIG_LOCK.lock();
    let mut c = read_config_from(dir);
    f(&mut c);
    write_config_to(dir, &c)?;
    Ok(c)
}

/// Measured llama-server speeds of the last run per model (start value of
/// the remaining-time estimate, see `eta`), next to `ai.json`.
pub const SPEEDS_FILE: &str = "ai-speed.json";
/// Serializes read-modify-write of [`SPEEDS_FILE`].
static SPEEDS_LOCK: Mutex<()> = Mutex::new(());

fn read_speeds_from(dir: &Path) -> std::collections::BTreeMap<String, super::eta::Speeds> {
    std::fs::read_to_string(dir.join(SPEEDS_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_speeds_to(dir: &Path, id: &str, speeds: super::eta::Speeds) -> Result<(), AiError> {
    let _lock = SPEEDS_LOCK.lock();
    let mut all = read_speeds_from(dir);
    all.insert(id.to_string(), speeds);
    let json = serde_json::to_string_pretty(&all).map_err(|e| AiError::Other(e.to_string()))?;
    std::fs::create_dir_all(dir)
        .and_then(|_| std::fs::write(dir.join(SPEEDS_FILE), json))
        .map_err(|e| AiError::Other(format!("Messwerte konnten nicht gespeichert werden: {e}")))
}

/// Speeds measured in the last run of model `id` (implausible values are
/// ignored).
pub fn load_speeds(id: &str) -> Option<super::eta::Speeds> {
    let dir = CONFIG_DIR.get()?;
    read_speeds_from(dir)
        .get(id)
        .copied()
        .filter(|s| s.prompt_tps.is_finite() && s.gen_tps.is_finite())
        .filter(|s| {
            (0.01..100_000.0).contains(&s.prompt_tps) && (0.01..10_000.0).contains(&s.gen_tps)
        })
}

/// Stores the speeds measured for model `id`.
pub fn save_speeds(id: &str, speeds: super::eta::Speeds) -> Result<(), AiError> {
    write_speeds_to(config_dir()?, id, speeds)
}

pub fn models_dir() -> Result<PathBuf, AiError> {
    DATA_DIR
        .get()
        .map(|d| d.join(MODELS_SUBDIR))
        .ok_or_else(|| AiError::Other("Datenordner ist nicht bekannt.".into()))
}

// ---------------------------------------------------------------------------
// Status

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub size_bytes: u64,
    pub license: Option<String>,
    /// From the catalog; `false` = imported unknown file.
    pub known: bool,
    /// Catalog entry has revision and hashes (download possible).
    pub pinned: bool,
    pub installed: bool,
    /// Installed and checked against the catalog hashes. `installed &&
    /// !verified` for a catalog entry: imported without a hash to check.
    pub verified: bool,
    /// Hugging Face repo and quantization (catalog entries).
    pub repo: Option<String>,
    pub quant: Option<String>,
    /// Unverified files already imported for this entry.
    pub imported_parts: u32,
    /// Parts of a split GGUF that are still missing.
    pub missing_parts: Vec<String>,
    /// Bytes of an interrupted download (resumable).
    pub partial_bytes: u64,
    pub suggested: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    pub hardware: Hardware,
    pub models: Vec<ModelInfo>,
    /// Models no longer supported whose files are still on disk.
    pub retired: Vec<RetiredInfo>,
    pub selected: Option<String>,
    /// A model is selected and installed.
    pub model_ready: bool,
    pub runtime_available: bool,
    pub runtime_error: Option<String>,
    pub llama_version: String,
    pub models_dir: Option<String>,
}

/// A retired model still on disk (see [`RETIRED_MODELS`]).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RetiredInfo {
    pub id: String,
    pub name: String,
    /// Bytes deleting it frees (files, interrupted downloads, imports).
    pub size_bytes: u64,
    /// German note: no longer supported, can be deleted.
    pub note: String,
}

/// Files of a retired model: its downloads (with interrupted `.part` files)
/// and the files imported for it.
fn retired_files(dir: &Path, config: &AiConfig, r: &RetiredModel) -> Vec<PathBuf> {
    r.files
        .iter()
        .flat_map(|f| {
            let p = dir.join(f);
            [part_path(&p), p]
        })
        .chain(
            config
                .imported
                .iter()
                .filter(|i| i.catalog_id.as_deref() == Some(r.id))
                .map(|i| imported_path(dir, i)),
        )
        .collect()
}

/// Retired models with files on disk (or imports recorded for them).
pub fn retired_on_disk(dir: &Path, config: &AiConfig) -> Vec<RetiredInfo> {
    RETIRED_MODELS
        .iter()
        .filter_map(|r| {
            let files = retired_files(dir, config, r);
            let present: Vec<u64> = files.iter().filter_map(|p| file_len(p)).collect();
            let recorded = config
                .imported
                .iter()
                .any(|i| i.catalog_id.as_deref() == Some(r.id));
            (!present.is_empty() || recorded).then(|| RetiredInfo {
                id: r.id.to_string(),
                name: r.name.to_string(),
                size_bytes: present.iter().sum(),
                note: retired_note(r),
            })
        })
        .collect()
}

/// A selected model that is neither a supported catalog entry nor an
/// imported file (e.g. the retired 1.5B model) falls back to the 7B model
/// if it is installed, else to no selection; the change is stored. Returns
/// the retired model that was selected, if any.
fn repair_selection(config: &mut AiConfig, dir: Option<&Path>) -> Option<&'static RetiredModel> {
    let id = config.selected.clone()?;
    let known = catalog().iter().any(|m| m.id == id)
        || config
            .imported
            .iter()
            .any(|i| i.id == id && i.catalog_id.is_none());
    if known {
        return None;
    }
    let fallback = dir.and_then(|d| {
        catalog()
            .iter()
            .find(|m| m.id == LARGE_MODEL)
            .filter(|m| {
                catalog_installed(d, m) || unverified_parts(d, config, &m.id).first().is_some()
            })
            .map(|m| m.id.clone())
    });
    config.selected = fallback.clone();
    if CONFIG_DIR.get().is_some() {
        let _ = update_config(|c| {
            if c.selected.as_deref() == Some(id.as_str()) {
                c.selected = fallback;
            }
        });
    }
    retired(&id)
}

fn file_len(p: &Path) -> Option<u64> {
    std::fs::metadata(p)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn unverified_dir(dir: &Path, catalog_id: &str) -> PathBuf {
    dir.join(UNVERIFIED_SUBDIR).join(catalog_id)
}

/// Path of an imported file (attributed ones live in `unverified/<id>/`).
fn imported_path(dir: &Path, imp: &ImportedModel) -> PathBuf {
    match &imp.catalog_id {
        Some(c) => unverified_dir(dir, c).join(&imp.file_name),
        None => dir.join(&imp.file_name),
    }
}

/// Unverified files imported for a catalog entry.
#[derive(Debug, PartialEq)]
pub struct UnverifiedParts {
    /// Files present with the recorded size.
    pub present: Vec<String>,
    pub state: PartSet,
}

impl UnverifiedParts {
    pub fn first(&self) -> Option<&str> {
        match &self.state {
            PartSet::Complete { first } => Some(first),
            _ => None,
        }
    }
}

pub fn unverified_parts(dir: &Path, config: &AiConfig, catalog_id: &str) -> UnverifiedParts {
    let present: Vec<String> = config
        .imported
        .iter()
        .filter(|i| i.catalog_id.as_deref() == Some(catalog_id))
        .filter(|i| file_len(&imported_path(dir, i)) == Some(i.size))
        .map(|i| i.file_name.clone())
        .collect();
    let names: Vec<&str> = present.iter().map(String::as_str).collect();
    let state = part_set(&names);
    UnverifiedParts { present, state }
}

/// The installed variant of a catalog entry: all its files present with the
/// catalog size (files only get there by an atomic rename after the hash
/// check). The single file is preferred; for an entry that also lists the
/// parts, a complete set of parts counts as well.
pub fn installed_files<'a>(dir: &Path, m: &'a CatalogModel) -> Option<Vec<&'a CatalogFile>> {
    variants(&m.files).into_iter().find(|v| {
        !v.is_empty()
            && v.iter()
                .all(|f| file_len(&dir.join(&f.name)) == Some(f.size))
    })
}

pub fn catalog_installed(dir: &Path, m: &CatalogModel) -> bool {
    installed_files(dir, m).is_some()
}

pub fn status() -> AiStatus {
    let hw = hardware();
    let mut config = read_config();
    let dir = models_dir().ok();
    repair_selection(&mut config, dir.as_deref());
    let selected = config.selected.clone();
    let mut models: Vec<ModelInfo> = catalog()
        .iter()
        .map(|m| {
            let installed = dir.as_ref().and_then(|d| installed_files(d, m));
            let verified = installed.is_some();
            let partial = match &dir {
                Some(d) if !verified => effective_files(&m.files)
                    .iter()
                    .filter_map(|f| file_len(&part_path(&d.join(&f.name))))
                    .sum(),
                _ => 0,
            };
            let parts = dir.as_ref().map(|d| unverified_parts(d, &config, &m.id));
            let unverified_size: u64 = config
                .imported
                .iter()
                .filter(|i| i.catalog_id.as_deref() == Some(m.id.as_str()))
                .map(|i| i.size)
                .sum();
            let (imported_parts, missing_parts, unverified_ok) = match &parts {
                Some(p) => (
                    p.present.len() as u32,
                    match &p.state {
                        PartSet::Missing(v) => v.clone(),
                        _ => Vec::new(),
                    },
                    p.first().is_some(),
                ),
                None => (0, Vec::new(), false),
            };
            ModelInfo {
                id: m.id.clone(),
                name: m.name.clone(),
                size_bytes: if let Some(files) = &installed {
                    files.iter().map(|f| f.size).sum()
                } else if m.is_pinned() {
                    m.size()
                } else {
                    unverified_size
                },
                license: Some(m.license.clone()),
                known: true,
                pinned: m.is_pinned(),
                installed: verified || unverified_ok,
                verified,
                repo: Some(m.repo.clone()),
                quant: Some(m.quant.clone()),
                imported_parts,
                missing_parts,
                partial_bytes: partial,
                suggested: hw.suggested_model.as_deref() == Some(m.id.as_str()),
                selected: selected.as_deref() == Some(m.id.as_str()),
            }
        })
        .collect();
    for imp in config.imported.iter().filter(|i| i.catalog_id.is_none()) {
        let installed = dir
            .as_ref()
            .is_some_and(|d| file_len(&imported_path(d, imp)) == Some(imp.size));
        models.push(ModelInfo {
            id: imp.id.clone(),
            name: imp.name.clone(),
            size_bytes: imp.size,
            license: None,
            known: false,
            pinned: false,
            installed,
            verified: false,
            repo: None,
            quant: None,
            imported_parts: 0,
            missing_parts: Vec::new(),
            partial_bytes: 0,
            suggested: false,
            selected: selected.as_deref() == Some(imp.id.as_str()),
        });
    }
    let model_ready = models.iter().any(|m| m.selected && m.installed);
    let (runtime_available, runtime_error) = match llama::server_path() {
        Ok(_) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };
    let retired = dir
        .as_ref()
        .map(|d| retired_on_disk(d, &config))
        .unwrap_or_default();
    AiStatus {
        hardware: hw,
        models,
        retired,
        selected,
        model_ready,
        runtime_available,
        runtime_error,
        llama_version: llama::LLAMA_CPP_VERSION.to_string(),
        models_dir: dir.map(|d| d.display().to_string()),
    }
}

/// The selected, installed model: path of the (first) GGUF file and the
/// number of solutions to generate.
pub struct SelectedModel {
    pub id: String,
    pub path: PathBuf,
    pub solutions: u32,
    /// The file's SHA-256 matched the (pinned) catalog. Otherwise
    /// (imported unknown model, unpinned catalog entry) llama-server uses
    /// its built-in chat template instead of the file's.
    pub verified: bool,
}

impl SelectedModel {
    pub fn chat_template(&self) -> llama::ChatTemplate {
        if self.verified {
            llama::ChatTemplate::FromModel
        } else {
            llama::ChatTemplate::BuiltinChatml
        }
    }

    /// llama-server slots: one per solution plus one for the test inputs,
    /// which are generated at the same time. OCR correction uses the same
    /// number, so it reuses the running server.
    pub fn server_slots(&self) -> usize {
        self.solutions as usize + 1
    }
}

pub fn selected_model() -> Result<SelectedModel, AiError> {
    let not_ready = || {
        AiError::Other(
            "Kein KI-Modell eingerichtet. Bitte unter „KI-Modell“ ein Modell herunterladen \
             oder importieren und auswählen."
                .into(),
        )
    };
    ensure_ai_available()?;
    let mut config = read_config();
    let dir = models_dir()?;
    if let Some(r) = repair_selection(&mut config, Some(&dir)) {
        if config.selected.is_none() {
            return Err(AiError::Other(format!(
                "{} Bitte unter „KI-Modell“ das 7B-Modell herunterladen und auswählen.",
                retired_note(r)
            )));
        }
    }
    let id = config.selected.clone().ok_or_else(not_ready)?;
    if let Some(m) = catalog().iter().find(|m| m.id == id) {
        let start = installed_files(&dir, m).and_then(|v| start_file(&v));
        let verified = start.is_some();
        let path = if let Some(f) = start {
            dir.join(&f.name)
        } else {
            let parts = unverified_parts(&dir, &config, &m.id);
            let first = parts.first().ok_or_else(not_ready)?;
            unverified_dir(&dir, &m.id).join(first)
        };
        return Ok(SelectedModel {
            id,
            path,
            solutions: m.solutions,
            verified,
        });
    }
    let imp = config
        .imported
        .iter()
        .find(|m| m.id == id && m.catalog_id.is_none())
        .ok_or_else(not_ready)?;
    let path = imported_path(&dir, imp);
    if file_len(&path) != Some(imp.size) {
        return Err(not_ready());
    }
    Ok(SelectedModel {
        id,
        path,
        solutions: IMPORTED_SOLUTIONS,
        verified: false,
    })
}

pub fn select(id: &str) -> Result<AiStatus, AiError> {
    ensure_ai_available()?;
    if let Some(r) = retired(id) {
        return Err(AiError::Other(retired_note(r)));
    }
    let st = status();
    match st.models.iter().find(|m| m.id == id) {
        Some(m) if m.installed => {}
        Some(_) => {
            return Err(AiError::Other(
                "Das Modell ist noch nicht vollständig vorhanden – bitte erst importieren oder \
                 herunterladen."
                    .into(),
            ))
        }
        None => return Err(AiError::Other(format!("Unbekanntes Modell: {id}"))),
    }
    update_config(|c| c.selected = Some(id.to_string()))?;
    Ok(status())
}

/// Selects `id` if no model is selected yet.
fn select_if_none(id: &str) {
    let _ = update_config(|c| {
        if c.selected.is_none() {
            c.selected = Some(id.to_string());
        }
    });
}

pub async fn delete(id: &str) -> Result<AiStatus, AiError> {
    let dir = models_dir()?;
    let config = read_config();
    let files: Vec<PathBuf> = if let Some(r) = retired(id) {
        retired_files(&dir, &config, r)
    } else if let Some(m) = catalog().iter().find(|m| m.id == id) {
        m.files
            .iter()
            .map(|f| dir.join(&f.name))
            .chain(
                config
                    .imported
                    .iter()
                    .filter(|i| i.catalog_id.as_deref() == Some(id))
                    .map(|i| imported_path(&dir, i)),
            )
            .collect()
    } else if let Some(imp) = config
        .imported
        .iter()
        .find(|m| m.id == id && m.catalog_id.is_none())
    {
        vec![imported_path(&dir, imp)]
    } else {
        return Err(AiError::Other(format!("Unbekanntes Modell: {id}")));
    };
    if TRANSFER_RUNNING.load(Ordering::SeqCst) {
        return Err(AiError::Other(
            "Bitte warten, bis Download oder Import beendet ist.".into(),
        ));
    }
    // The server keeps the file open (Windows cannot delete it then).
    llama::shutdown().await;
    for f in &files {
        for p in [f.clone(), part_path(f)] {
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(AiError::Other(format!(
                        "{} konnte nicht gelöscht werden: {e}",
                        p.display()
                    )))
                }
            }
        }
    }
    let _ = std::fs::remove_dir(unverified_dir(&dir, id));
    update_config(|c| {
        c.imported
            .retain(|m| m.id != id && m.catalog_id.as_deref() != Some(id));
        if c.selected.as_deref() == Some(id) {
            c.selected = None;
        }
    })?;
    Ok(status())
}

// ---------------------------------------------------------------------------
// Download and import

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsProgress {
    pub model_id: String,
    /// German status line.
    pub message: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub percent: Option<u8>,
}

pub fn percent(done: u64, total: u64) -> Option<u8> {
    (total > 0).then(|| ((done.min(total) as u128 * 100) / total as u128) as u8)
}

/// Only one download/import at a time.
struct TransferGuard;

impl TransferGuard {
    fn acquire() -> Result<Self, AiError> {
        if TRANSFER_RUNNING.swap(true, Ordering::SeqCst) {
            return Err(AiError::Other(
                "Es läuft bereits ein Download oder Import.".into(),
            ));
        }
        Ok(TransferGuard)
    }
}

impl Drop for TransferGuard {
    fn drop(&mut self) {
        TRANSFER_RUNNING.store(false, Ordering::SeqCst);
    }
}

fn io_err(what: &str, e: std::io::Error) -> AiError {
    AiError::Other(format!("{what}: {e} (Speicherplatz voll?)"))
}

fn http_client() -> Result<reqwest::Client, AiError> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            attempt.error("zu viele Weiterleitungen")
        } else if allowed_url(attempt.url()) {
            attempt.follow()
        } else {
            let host = attempt.url().host_str().unwrap_or("?").to_string();
            attempt.error(format!("Weiterleitung zu nicht erlaubtem Server {host}"))
        }
    });
    reqwest::Client::builder()
        .redirect(policy)
        .https_only(true)
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .user_agent(concat!("Code-Pruefer/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AiError::Http(e.to_string()))
}

/// Hashes an existing file (resume) in chunks.
async fn hash_file(
    path: &Path,
    hasher: &mut Sha256,
    cancel: &CancellationToken,
) -> Result<u64, AiError> {
    let mut f = tokio::fs::File::open(path)
        .await
        .map_err(|e| io_err("Datei konnte nicht gelesen werden", e))?;
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Err(AiError::Cancelled);
        }
        let n = f
            .read(&mut buf)
            .await
            .map_err(|e| io_err("Datei konnte nicht gelesen werden", e))?;
        if n == 0 {
            return Ok(total);
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
}

/// Parses the start of `Content-Range: bytes <start>-<end>/<total>`.
pub fn content_range_start(header: &str) -> Option<u64> {
    header
        .trim()
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

/// Downloads one file to `dest` (via `dest.part`, resumable), verifying
/// size and SHA-256 before the atomic rename.
#[allow(clippy::too_many_arguments)]
async fn download_file(
    client: &reqwest::Client,
    url: reqwest::Url,
    dest: &Path,
    expected_size: u64,
    expected_sha256: &str,
    done_before: u64,
    total_all: u64,
    cancel: &CancellationToken,
    on_progress: &mut impl FnMut(u64, u64),
) -> Result<(), AiError> {
    let part = part_path(dest);
    let mut hasher = Sha256::new();
    let mut have = match file_len(&part) {
        Some(n) if n > 0 && n <= expected_size => hash_file(&part, &mut hasher, cancel).await?,
        Some(_) => {
            let _ = tokio::fs::remove_file(&part).await;
            0
        }
        None => 0,
    };

    if have < expected_size {
        let mut req = client.get(url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let resp = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AiError::Cancelled),
            r = req.send() => r.map_err(|e| AiError::Http(format!("Download fehlgeschlagen: {e}")))?,
        };
        let status = resp.status();
        let append = if status == reqwest::StatusCode::PARTIAL_CONTENT {
            let start = resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(content_range_start);
            start == Some(have)
        } else if status.is_success() {
            false
        } else {
            return Err(AiError::Http(format!(
                "Download fehlgeschlagen: HTTP {status}"
            )));
        };
        if !append {
            // Server ignored the range: start over.
            have = 0;
            hasher = Sha256::new();
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&part)
            .await
            .map_err(|e| io_err("Datei konnte nicht angelegt werden", e))?;
        let mut stream = resp.bytes_stream();
        let mut last = Instant::now();
        loop {
            let chunk = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    let _ = file.flush().await;
                    return Err(AiError::Cancelled);
                }
                c = stream.next() => c,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|e| AiError::Http(format!("Download unterbrochen: {e}")))?;
            have += chunk.len() as u64;
            if have > expected_size {
                drop(file);
                let _ = tokio::fs::remove_file(&part).await;
                return Err(AiError::Other(
                    "Die Datei ist größer als erwartet und wurde verworfen.".into(),
                ));
            }
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|e| io_err("Schreiben fehlgeschlagen", e))?;
            if last.elapsed() >= PROGRESS_INTERVAL {
                last = Instant::now();
                on_progress(done_before + have, total_all);
            }
        }
        file.flush()
            .await
            .map_err(|e| io_err("Schreiben fehlgeschlagen", e))?;
    }

    let actual = hex(&hasher.finalize());
    if have != expected_size || actual != expected_sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(AiError::Other(format!(
            "Die heruntergeladene Datei ist unvollständig oder beschädigt (Prüfsumme stimmt \
             nicht) und wurde verworfen. Bitte erneut herunterladen. ({} von {expected_size} Bytes)",
            have
        )));
    }
    tokio::fs::rename(&part, dest)
        .await
        .map_err(|e| io_err("Datei konnte nicht umbenannt werden", e))?;
    on_progress(done_before + have, total_all);
    Ok(())
}

/// Downloads a catalog model from huggingface.co (resumable).
pub async fn download(
    id: &str,
    cancel: &CancellationToken,
    mut on_progress: impl FnMut(ModelsProgress),
) -> Result<AiStatus, AiError> {
    ensure_ai_available()?;
    let m = catalog()
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| AiError::Other(format!("Unbekanntes Modell: {id}")))?;
    if !m.is_pinned() {
        return Err(AiError::Other(format!(
            "Für „{}“ sind im Modellkatalog dieser Version noch keine Prüfsummen eingetragen; \
             ein Download ist deshalb nicht möglich. Bitte die .gguf-Datei über „Datei \
             importieren …“ einlesen.",
            m.name
        )));
    }
    let _guard = TransferGuard::acquire()?;
    let dir = models_dir()?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| io_err("Modellordner konnte nicht angelegt werden", e))?;
    // Already there (e.g. the parts from a download before the catalog
    // listed only one variant): nothing to download twice.
    if catalog_installed(&dir, m) {
        select_if_none(&m.id);
        return Ok(status());
    }
    let client = http_client()?;
    let files = effective_files(&m.files);
    let total = m.size();
    let mut done = 0u64;
    let count = files.len();
    for (i, f) in files.into_iter().enumerate() {
        let dest = dir.join(&f.name);
        if file_len(&dest) == Some(f.size) {
            done += f.size;
            continue;
        }
        let part_note = if count > 1 {
            format!(" (Teil {} von {count})", i + 1)
        } else {
            String::new()
        };
        let mut report = |d: u64, t: u64| {
            on_progress(ModelsProgress {
                model_id: m.id.clone(),
                message: format!("Lade {}{part_note}: {} von {} GB", m.name, gib(d), gib(t)),
                done_bytes: d,
                total_bytes: t,
                percent: percent(d, t),
            })
        };
        report(done, total);
        let url = reqwest::Url::parse(&m.url(f))
            .ok()
            .filter(allowed_url)
            .ok_or_else(|| {
                AiError::Other(format!("Nicht erlaubte Download-Adresse: {}", m.url(f)))
            })?;
        download_file(
            &client,
            url,
            &dest,
            f.size,
            &f.sha256,
            done,
            total,
            cancel,
            &mut report,
        )
        .await?;
        done += f.size;
    }
    select_if_none(&m.id);
    Ok(status())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    /// Id of the imported model (`None` while confirmation is pending).
    pub model_id: Option<String>,
    /// The file matches a catalog file (SHA-256).
    pub known: bool,
    /// Unknown file: the user must confirm (`confirm_unknown`).
    pub needs_confirmation: bool,
    /// For split catalog models: other parts are still missing.
    pub incomplete: bool,
    /// Imported for a catalog entry without catalog hashes (not checked).
    pub unverified: bool,
    /// Reasons to confirm (German), e.g. unexpected file name.
    pub warnings: Vec<String>,
    /// Parts of a split GGUF still missing after this import.
    pub missing_parts: Vec<String>,
    /// SHA-256 of the (single or first) file; empty if not hashed yet.
    pub sha256: String,
    pub status: AiStatus,
}

/// What an imported file is, by its SHA-256.
#[derive(Debug, PartialEq)]
pub enum Match<'a> {
    Catalog(&'a CatalogModel, &'a CatalogFile),
    Unknown,
}

/// Matches against every pinned file, including the parts of an entry
/// whose effective file is the single file: they are verified as well and
/// count as installed once complete (see [`installed_files`]).
pub fn match_hash<'a>(models: &'a [CatalogModel], sha256: &str) -> Match<'a> {
    for m in models.iter().filter(|m| m.is_pinned()) {
        if let Some(f) = m.files.iter().find(|f| f.sha256 == sha256) {
            return Match::Catalog(m, f);
        }
    }
    Match::Unknown
}

pub fn imported_id(sha256: &str) -> String {
    format!("import-{}", &sha256[..12.min(sha256.len())])
}

/// Imports a local `.gguf` file: hashes it, copies it into the models
/// directory and marks it as catalog model (hash match) or, only with
/// `confirm_unknown`, as unknown model.
pub async fn import(
    path: &Path,
    confirm_unknown: bool,
    cancel: &CancellationToken,
    mut on_progress: impl FnMut(ModelsProgress),
) -> Result<ImportOutcome, AiError> {
    ensure_ai_available()?;
    let (name, size) = check_gguf_file(path).await?;

    let _guard = TransferGuard::acquire()?;
    let progress = |phase: &str, done: u64| ModelsProgress {
        model_id: String::new(),
        message: format!("{phase} „{name}“: {} von {} GB", gib(done), gib(size)),
        done_bytes: done,
        total_bytes: size,
        percent: percent(done, size),
    };

    // Pass 1: hash.
    let mut hasher = Sha256::new();
    {
        let mut f = tokio::fs::File::open(path)
            .await
            .map_err(|e| io_err("Datei konnte nicht gelesen werden", e))?;
        let mut buf = vec![0u8; CHUNK];
        let mut done = 0u64;
        let mut last = Instant::now();
        loop {
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled);
            }
            let n = f
                .read(&mut buf)
                .await
                .map_err(|e| io_err("Datei konnte nicht gelesen werden", e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            done += n as u64;
            if last.elapsed() >= PROGRESS_INTERVAL {
                last = Instant::now();
                on_progress(progress("Prüfe", done));
            }
        }
    }
    let sha256 = hex(&hasher.finalize());
    if let Some(r) = RETIRED_MODELS
        .iter()
        .find(|r| r.sha256.contains(&sha256.as_str()))
    {
        return Err(AiError::Other(format!(
            "{} Die Datei wurde nicht importiert.",
            retired_note(r)
        )));
    }

    let (target_name, model_id, known) = match match_hash(catalog(), &sha256) {
        Match::Catalog(m, f) => (f.name.clone(), m.id.clone(), true),
        Match::Unknown => {
            if !confirm_unknown {
                return Ok(ImportOutcome {
                    model_id: None,
                    known: false,
                    needs_confirmation: true,
                    incomplete: false,
                    unverified: false,
                    warnings: Vec::new(),
                    missing_parts: Vec::new(),
                    sha256,
                    status: status(),
                });
            }
            let id = imported_id(&sha256);
            (format!("{id}.gguf"), id, false)
        }
    };

    // Pass 2: copy (via .part, then atomic rename), hashing again so the
    // copy is verified as well.
    let dir = models_dir()?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| io_err("Modellordner konnte nicht angelegt werden", e))?;
    let dest = dir.join(&target_name);
    let part = part_path(&dest);
    let copied = copy_to_part(path, &part, cancel, |done| {
        on_progress(progress("Kopiere", done))
    })
    .await?;
    if copied != sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(AiError::Other(
            "Die Datei hat sich beim Kopieren verändert – bitte erneut importieren.".into(),
        ));
    }
    // A running server may hold the old file open.
    llama::shutdown().await;
    tokio::fs::rename(&part, &dest)
        .await
        .map_err(|e| io_err("Datei konnte nicht umbenannt werden", e))?;

    if !known {
        update_config(|c| {
            c.imported.retain(|m| m.id != model_id);
            c.imported.push(ImportedModel {
                id: model_id.clone(),
                name,
                file_name: target_name.clone(),
                size,
                sha256: sha256.clone(),
                catalog_id: None,
            });
        })?;
    }
    let incomplete = catalog()
        .iter()
        .find(|m| m.id == model_id)
        .is_some_and(|m| !catalog_installed(&dir, m));
    if !incomplete {
        select_if_none(&model_id);
    }
    Ok(ImportOutcome {
        model_id: Some(model_id),
        known,
        needs_confirmation: false,
        incomplete,
        unverified: false,
        warnings: Vec::new(),
        missing_parts: Vec::new(),
        sha256,
        status: status(),
    })
}

/// Checks name, size and GGUF header of a file to import; returns its
/// file name and size.
async fn check_gguf_file(path: &Path) -> Result<(String, u64), AiError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if !name.to_ascii_lowercase().ends_with(".gguf") {
        return Err(AiError::Other("Bitte eine .gguf-Datei wählen.".into()));
    }
    let size =
        file_len(path).ok_or_else(|| AiError::Other(format!("„{name}“ wurde nicht gefunden.")))?;
    if size > MAX_IMPORT_BYTES {
        return Err(AiError::Other(format!(
            "„{name}“ ist zu groß ({} GB, höchstens {} GB).",
            gib(size),
            gib(MAX_IMPORT_BYTES)
        )));
    }
    let mut magic = [0u8; 4];
    tokio::fs::File::open(path)
        .await
        .map_err(|e| io_err("Datei konnte nicht gelesen werden", e))?
        .read_exact(&mut magic)
        .await
        .map_err(|_| AiError::Other(format!("„{name}“ ist keine GGUF-Datei.")))?;
    if !is_gguf_magic(&magic) {
        return Err(AiError::Other(format!(
            "„{name}“ ist keine GGUF-Datei (Dateianfang ist nicht „GGUF“)."
        )));
    }
    // Metadata and tensor list within the ranges of real models, before
    // llama.cpp's native parser ever sees the file.
    let header_path = path.to_path_buf();
    let header = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&header_path).map_err(|e| e.to_string())?;
        super::gguf::check_header(std::io::BufReader::new(file))
    })
    .await
    .map_err(|e| AiError::Other(e.to_string()))?;
    if let Err(detail) = header {
        return Err(AiError::Other(format!(
            "„{name}“ hat ungewöhnliche Metadaten ({detail}) und wird nicht übernommen – \
             vermutlich ist die Datei beschädigt oder manipuliert."
        )));
    }
    Ok((name, size))
}

/// Copies `src` to `part` (the caller renames it), returning the SHA-256
/// of the copied bytes. `part` is removed on error.
async fn copy_to_part(
    src: &Path,
    part: &Path,
    cancel: &CancellationToken,
    mut on_bytes: impl FnMut(u64),
) -> Result<String, AiError> {
    let copy = async {
        let mut src = tokio::fs::File::open(src)
            .await
            .map_err(|e| io_err("Datei konnte nicht gelesen werden", e))?;
        let mut out = tokio::fs::File::create(part)
            .await
            .map_err(|e| io_err("Datei konnte nicht angelegt werden", e))?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        let mut done = 0u64;
        let mut last = Instant::now();
        loop {
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled);
            }
            let n = src
                .read(&mut buf)
                .await
                .map_err(|e| io_err("Datei konnte nicht gelesen werden", e))?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            out.write_all(&buf[..n])
                .await
                .map_err(|e| io_err("Schreiben fehlgeschlagen", e))?;
            done += n as u64;
            if last.elapsed() >= PROGRESS_INTERVAL {
                last = Instant::now();
                on_bytes(done);
            }
        }
        out.flush()
            .await
            .map_err(|e| io_err("Schreiben fehlgeschlagen", e))?;
        Ok(hex(&h.finalize()))
    };
    let r = copy.await;
    if r.is_err() {
        let _ = tokio::fs::remove_file(part).await;
    }
    r
}

/// Imports one or more `.gguf` files via the button of catalog entry
/// `catalog_id` (all parts of a split model in one go).
///
/// - Pinned entry: every file must match a catalog hash (as `import`).
/// - Unpinned entry (no hash in this version): the files are attributed to
///   the entry, stored unverified and marked so; this always needs
///   `confirm`. The first call (without `confirm`) only checks name,
///   header and parts and returns the warnings.
pub async fn import_for(
    catalog_id: &str,
    paths: &[PathBuf],
    confirm: bool,
    cancel: &CancellationToken,
    mut on_progress: impl FnMut(ModelsProgress),
) -> Result<ImportOutcome, AiError> {
    ensure_ai_available()?;
    let m = catalog()
        .iter()
        .find(|m| m.id == catalog_id)
        .ok_or_else(|| AiError::Other(format!("Unbekanntes Modell: {catalog_id}")))?;
    if paths.is_empty() {
        return Err(AiError::Other("Keine Datei gewählt.".into()));
    }
    if m.is_pinned() {
        let mut last = None;
        for path in paths {
            let r = import(path, false, cancel, &mut on_progress).await?;
            if !r.known {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                return Err(AiError::Other(format!(
                    "„{name}“ passt nicht zur Prüfsumme von „{}“ und wurde nicht übernommen. \
                     Andere Modelle bitte über „Anderes Modell importieren …“ einlesen.",
                    m.name
                )));
            }
            last = Some(r);
        }
        return last.ok_or_else(|| AiError::Other("Keine Datei gewählt.".into()));
    }

    let mut files = Vec::new();
    for path in paths {
        let (name, size) = check_gguf_file(path).await?;
        if !safe_file_name(&name) {
            return Err(AiError::Other(format!("Ungültiger Dateiname: „{name}“")));
        }
        files.push((path.clone(), name, size));
    }
    let new_names: Vec<&str> = files.iter().map(|(_, n, _)| n.as_str()).collect();
    if let PartSet::Invalid(reason) = part_set(&new_names) {
        return Err(AiError::Other(reason));
    }
    let mut warnings = name_warnings(m, &new_names);
    // Parts imported earlier count, unless they do not fit the new ones
    // (then the new files replace them).
    let dir = models_dir()?;
    let config = read_config();
    let earlier = unverified_parts(&dir, &config, &m.id).present;
    let mut all: Vec<&str> = earlier.iter().map(String::as_str).collect();
    all.extend(&new_names);
    let replace = !earlier.is_empty() && matches!(part_set(&all), PartSet::Invalid(_));
    if replace {
        warnings.push(format!(
            "Die bisher für „{}“ importierten Dateien passen nicht dazu und werden ersetzt.",
            m.name
        ));
        all = new_names.clone();
    }
    let missing_parts = match part_set(&all) {
        PartSet::Missing(v) => v,
        _ => Vec::new(),
    };
    if !confirm {
        return Ok(ImportOutcome {
            model_id: None,
            known: true,
            needs_confirmation: true,
            incomplete: !missing_parts.is_empty(),
            unverified: true,
            warnings,
            missing_parts,
            sha256: String::new(),
            status: status(),
        });
    }

    let _guard = TransferGuard::acquire()?;
    let target = unverified_dir(&dir, &m.id);
    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|e| io_err("Modellordner konnte nicht angelegt werden", e))?;
    let total: u64 = files.iter().map(|(_, _, s)| s).sum();
    let count = files.len();
    let mut done_before = 0u64;
    let mut copied = Vec::new();
    for (i, (path, name, size)) in files.iter().enumerate() {
        let part_note = if count > 1 {
            format!(" (Teil {} von {count})", i + 1)
        } else {
            String::new()
        };
        let part = part_path(&target.join(name));
        let r = copy_to_part(path, &part, cancel, |d| {
            let d = done_before + d;
            on_progress(ModelsProgress {
                model_id: m.id.clone(),
                message: format!(
                    "Kopiere „{name}“{part_note}: {} von {} GB",
                    gib(d),
                    gib(total)
                ),
                done_bytes: d,
                total_bytes: total,
                percent: percent(d, total),
            })
        })
        .await;
        match r {
            Ok(sha) => copied.push((part, name.clone(), *size, sha)),
            Err(e) => {
                for (p, ..) in &copied {
                    let _ = tokio::fs::remove_file(p).await;
                }
                return Err(e);
            }
        }
        done_before += size;
    }
    // A running server may hold the old files open.
    llama::shutdown().await;
    if replace {
        for old in &earlier {
            let _ = tokio::fs::remove_file(target.join(old)).await;
        }
    }
    for (part, name, ..) in &copied {
        tokio::fs::rename(part, target.join(name))
            .await
            .map_err(|e| io_err("Datei konnte nicht umbenannt werden", e))?;
    }
    update_config(|c| {
        c.imported.retain(|i| {
            i.catalog_id.as_deref() != Some(m.id.as_str())
                || !(replace || copied.iter().any(|(_, n, ..)| *n == i.file_name))
        });
        for (_, name, size, sha) in &copied {
            c.imported.push(ImportedModel {
                id: format!("{}-{}", m.id, &sha[..12]),
                name: name.clone(),
                file_name: name.clone(),
                size: *size,
                sha256: sha.clone(),
                catalog_id: Some(m.id.clone()),
            });
        }
    })?;
    let parts = unverified_parts(&dir, &read_config(), &m.id);
    let complete = parts.first().is_some();
    if complete {
        select_if_none(&m.id);
    }
    Ok(ImportOutcome {
        model_id: Some(m.id.clone()),
        known: true,
        needs_confirmation: false,
        incomplete: !complete,
        unverified: true,
        warnings,
        missing_parts: match parts.state {
            PartSet::Missing(v) => v,
            _ => Vec::new(),
        },
        sha256: copied.first().map(|c| c.3.clone()).unwrap_or_default(),
        status: status(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_are_stored_per_model() {
        let dir = std::env::temp_dir().join(format!("cp-speeds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_speeds_from(&dir).is_empty());
        let a = super::super::eta::Speeds {
            prompt_tps: 7.5,
            gen_tps: 2.25,
        };
        let b = super::super::eta::Speeds {
            prompt_tps: 20.0,
            gen_tps: 5.0,
        };
        write_speeds_to(&dir, LARGE_MODEL, a).unwrap();
        write_speeds_to(&dir, "import-abc", b).unwrap();
        write_speeds_to(&dir, "import-abc", a).unwrap();
        let all = read_speeds_from(&dir);
        assert_eq!(all.len(), 2);
        assert_eq!(all[LARGE_MODEL], a);
        assert_eq!(all["import-abc"], a);
        let raw = std::fs::read_to_string(dir.join(SPEEDS_FILE)).unwrap();
        assert!(raw.contains("\"promptTps\": 7.5"), "{raw}");
        // A broken file counts as empty.
        std::fs::write(dir.join(SPEEDS_FILE), "{kaputt").unwrap();
        assert!(read_speeds_from(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn pinned(id: &str, files: &[(&str, u64, char)]) -> CatalogModel {
        CatalogModel {
            id: id.into(),
            name: id.into(),
            repo: "Qwen/X-GGUF".into(),
            quant: "q4_k_m".into(),
            license: "Apache-2.0".into(),
            solutions: 3,
            revision: "a".repeat(40),
            files: files
                .iter()
                .map(|(n, s, c)| CatalogFile {
                    name: n.to_string(),
                    size: *s,
                    sha256: c.to_string().repeat(64),
                })
                .collect(),
        }
    }

    #[test]
    fn bundled_catalog_is_valid() {
        // catalog.json is only changed by pin-models.yml: until it is pinned
        // again it may still list a retired model, which the app ignores.
        let raw = parse_catalog(CATALOG_JSON).unwrap();
        for m in &raw {
            if let Some(r) = retired(&m.id) {
                let names: Vec<&str> = m.files.iter().map(|f| f.name.as_str()).collect();
                let hashes: Vec<&str> = m.files.iter().map(|f| f.sha256.as_str()).collect();
                assert_eq!(names, r.files, "{}", m.id);
                assert_eq!(hashes, r.sha256, "{}", m.id);
            } else {
                assert!(SUPPORTED_MODELS.contains(&m.id.as_str()), "{}", m.id);
            }
        }
        let models = catalog();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, SUPPORTED_MODELS);
        assert_eq!(ids, vec![LARGE_MODEL]);
        for m in models {
            assert!(m.repo.starts_with("Qwen/Qwen2.5-Coder-"), "{}", m.repo);
            assert!(m.repo.ends_with("-Instruct-GGUF"), "{}", m.repo);
            assert_eq!(m.license, "Apache-2.0");
            // 3B is excluded (non-commercial license).
            assert!(!m.repo.contains("-3B-"), "{}", m.repo);
            // Pinned entries must be complete; unpinned ones stay empty.
            if !m.revision.is_empty() || !m.files.is_empty() {
                assert!(m.is_pinned(), "{} ist nur teilweise gepinnt", m.id);
                for f in &m.files {
                    assert!(f.name.contains(&m.quant), "{}", f.name);
                    assert!(allowed_url(&reqwest::Url::parse(&m.url(f)).unwrap()));
                }
                // Downloaded is exactly one variant: one single file or
                // the complete parts of one split file.
                let eff = effective_files(&m.files);
                let names: Vec<&str> = eff.iter().map(|f| f.name.as_str()).collect();
                match names.as_slice() {
                    [one] => assert!(split_part(one).is_none(), "{}: {one}", m.id),
                    _ => {
                        assert!(names.iter().all(|n| split_part(n).is_some()), "{}", m.id);
                        assert!(
                            matches!(part_set(&names), PartSet::Complete { .. }),
                            "{}: {names:?}",
                            m.id
                        );
                    }
                }
                assert_eq!(m.size(), eff.iter().map(|f| f.size).sum::<u64>());
            }
        }
        assert_eq!(models[0].solutions, 3);
    }

    #[test]
    fn unsupported_catalog_entries_are_ignored() {
        let models = vec![
            pinned("qwen2.5-coder-1.5b", &[("a-q4_k_m.gguf", 1, 'a')]),
            pinned(LARGE_MODEL, &[("b-q4_k_m.gguf", 1, 'b')]),
            pinned("unbekannt", &[("c-q4_k_m.gguf", 1, 'c')]),
        ];
        let ids: Vec<String> = supported(models).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![LARGE_MODEL.to_string()]);
        assert!(retired("qwen2.5-coder-1.5b").is_some());
        assert!(retired(LARGE_MODEL).is_none());
        assert!(!SUPPORTED_MODELS
            .iter()
            .any(|id| RETIRED_MODELS.iter().any(|r| r.id == *id)));
    }

    /// `SUPPORTED_MODELS` in `scripts/pin-models.sh` is the app's list.
    #[test]
    fn pin_script_lists_the_supported_models() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let script = std::fs::read_to_string(root.join("../scripts/pin-models.sh")).unwrap();
        let line = script
            .lines()
            .find_map(|l| l.strip_prefix("SUPPORTED_MODELS=\""))
            .and_then(|l| l.strip_suffix('"'))
            .expect("SUPPORTED_MODELS=\"…\" in pin-models.sh");
        let ids: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(ids, SUPPORTED_MODELS);
    }

    /// `scripts/pin-models.sh --check` (release workflow) and
    /// [`CatalogModel::is_pinned`] (app) accept and reject the same
    /// catalogs. Skipped without bash/jq.
    #[test]
    fn release_check_script_agrees_with_app() {
        let has = |cmd: &str| {
            std::process::Command::new(cmd)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !has("bash") || !has("jq") {
            eprintln!("übersprungen: bash/jq fehlen");
            return;
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let script = root.join("../scripts/pin-models.sh");
        let dir = std::env::temp_dir().join(format!("cp-pin-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let check = |name: &str, json: &serde_json::Value| -> bool {
            let path = dir.join(format!("{name}.json"));
            std::fs::write(&path, json.to_string()).unwrap();
            std::process::Command::new("bash")
                .arg(&script)
                .arg("--check")
                .env("CATALOG", &path)
                .output()
                .unwrap()
                .status
                .success()
        };
        let file = |name: &str, size: serde_json::Value, sha: &str| serde_json::json!({"name": name, "size": size, "sha256": sha});
        let model = |revision: &str, files: Vec<serde_json::Value>| {
            serde_json::json!({"models": [{
                "id": LARGE_MODEL, "name": "M", "repo": "Qwen/X-GGUF", "quant": "q4_k_m",
                "license": "Apache-2.0", "solutions": 3, "revision": revision, "files": files
            }]})
        };
        let rev = "a".repeat(40);
        let sha = "b".repeat(64);
        let cases = [
            (
                "ok",
                model(&rev, vec![file("m-q4_k_m.gguf", 10.into(), &sha)]),
            ),
            (
                "split",
                model(
                    &rev,
                    vec![
                        file("m-q4_k_m-00001-of-00002.gguf", 10.into(), &sha),
                        file("m-q4_k_m-00002-of-00002.gguf", 11.into(), &sha),
                    ],
                ),
            ),
            (
                "mixed",
                model(
                    &rev,
                    vec![
                        file("m-q4_k_m-00001-of-00002.gguf", 10.into(), &sha),
                        file("m-q4_k_m-00002-of-00002.gguf", 11.into(), &sha),
                        file("m-q4_k_m.gguf", 21.into(), &sha),
                    ],
                ),
            ),
            (
                "two-singles",
                model(
                    &rev,
                    vec![
                        file("m-q4_k_m.gguf", 10.into(), &sha),
                        file("n-q4_k_m.gguf", 11.into(), &sha),
                    ],
                ),
            ),
            (
                "no-revision",
                model("", vec![file("m-q4_k_m.gguf", 10.into(), &sha)]),
            ),
            (
                "branch",
                model("main", vec![file("m-q4_k_m.gguf", 10.into(), &sha)]),
            ),
            ("no-files", model(&rev, vec![])),
            (
                "short-sha",
                model(&rev, vec![file("m-q4_k_m.gguf", 10.into(), "abc")]),
            ),
            (
                "upper-sha",
                model(
                    &rev,
                    vec![file("m-q4_k_m.gguf", 10.into(), &"B".repeat(64))],
                ),
            ),
            (
                "size-0",
                model(&rev, vec![file("m-q4_k_m.gguf", 0.into(), &sha)]),
            ),
            (
                "size-text",
                model(&rev, vec![file("m-q4_k_m.gguf", "10".into(), &sha)]),
            ),
            (
                "path",
                model(&rev, vec![file("../m-q4_k_m.gguf", 10.into(), &sha)]),
            ),
            (
                "not-gguf",
                model(&rev, vec![file("m-q4_k_m.bin", 10.into(), &sha)]),
            ),
        ];
        for (name, json) in &cases {
            let app = parse_catalog(&json.to_string())
                .map(|ms| ms.iter().all(CatalogModel::is_pinned))
                .unwrap_or(false);
            let script_ok = check(name, json);
            assert_eq!(script_ok, app, "{name}: Skript {script_ok}, App {app}");
            assert_eq!(
                script_ok,
                matches!(*name, "ok" | "split" | "mixed"),
                "{name}"
            );
        }
        // An entry the app does not offer (retired, unknown) stops the
        // release until the catalog is pinned again (pin-models.yml).
        let mut retired_too = model(&rev, vec![file("m-q4_k_m.gguf", 10.into(), &sha)]);
        let mut old = retired_too["models"][0].clone();
        old["id"] = "qwen2.5-coder-1.5b".into();
        retired_too["models"].as_array_mut().unwrap().insert(0, old);
        assert!(!check("retired", &retired_too));
        // The committed catalog (pinned via pin-models.yml, or not yet):
        // script and app agree.
        let committed: serde_json::Value = serde_json::from_str(CATALOG_JSON).unwrap();
        let only_supported = parse_catalog(CATALOG_JSON)
            .unwrap()
            .iter()
            .all(|m| SUPPORTED_MODELS.contains(&m.id.as_str()));
        assert_eq!(
            check("committed", &committed),
            only_supported && catalog().iter().all(CatalogModel::is_pinned)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pinning_requires_revision_and_hashes() {
        let m = pinned("m", &[("m-q4_k_m.gguf", 10, 'b')]);
        assert!(m.is_pinned());
        assert_eq!(
            m.url(&m.files[0]),
            format!(
                "https://huggingface.co/Qwen/X-GGUF/resolve/{}/m-q4_k_m.gguf",
                "a".repeat(40)
            )
        );
        let mut bad = m.clone();
        bad.revision = "main".into();
        assert!(!bad.is_pinned());
        let mut bad = m.clone();
        bad.files[0].sha256 = "B".repeat(64);
        assert!(!bad.is_pinned());
        let mut bad = m.clone();
        bad.files[0].size = 0;
        assert!(!bad.is_pinned());
        let mut bad = m.clone();
        bad.files[0].name = "../x.gguf".into();
        assert!(!bad.is_pinned());
        let mut bad = m.clone();
        bad.files.clear();
        assert!(!bad.is_pinned());
        let two = pinned(
            "m",
            &[("m-q4_k_m.gguf", 10, 'b'), ("n-q4_k_m.gguf", 10, 'c')],
        );
        assert!(!two.is_pinned());
        let mixed = pinned(
            "m",
            &[
                ("m-q4_k_m-00001-of-00002.gguf", 4, 'b'),
                ("m-q4_k_m-00002-of-00002.gguf", 6, 'c'),
                ("m-q4_k_m.gguf", 10, 'd'),
            ],
        );
        assert!(mixed.is_pinned());
        assert_eq!(mixed.size(), 10);
    }

    #[test]
    fn effective_files_keep_one_variant() {
        let names = |m: &CatalogModel| -> Vec<String> {
            effective_files(&m.files)
                .iter()
                .map(|f| f.name.clone())
                .collect()
        };
        let single = pinned("m", &[("m-q4_k_m.gguf", 10, 'b')]);
        assert_eq!(names(&single), ["m-q4_k_m.gguf"]);
        let split = pinned(
            "m",
            &[
                ("m-q4_k_m-00001-of-00002.gguf", 4, 'b'),
                ("m-q4_k_m-00002-of-00002.gguf", 6, 'c'),
            ],
        );
        assert_eq!(
            names(&split),
            [
                "m-q4_k_m-00001-of-00002.gguf",
                "m-q4_k_m-00002-of-00002.gguf"
            ]
        );
        assert_eq!(split.size(), 10);
        // Old pin listing both variants: only the single file.
        let mixed = pinned(
            "m",
            &[
                ("m-q4_k_m-00001-of-00002.gguf", 4, 'b'),
                ("m-q4_k_m-00002-of-00002.gguf", 6, 'c'),
                ("m-q4_k_m.gguf", 10, 'd'),
            ],
        );
        assert_eq!(names(&mixed), ["m-q4_k_m.gguf"]);
        assert_eq!(mixed.size(), 10);
        assert!(effective_files(&[]).is_empty());
    }

    #[test]
    fn mixed_entry_is_installed_with_single_file_or_parts() {
        let dir = std::env::temp_dir().join(format!("cp-mixed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let m = pinned(
            "m",
            &[
                ("m-q4_k_m-00001-of-00002.gguf", 2, 'b'),
                ("m-q4_k_m-00002-of-00002.gguf", 3, 'c'),
                ("m-q4_k_m.gguf", 5, 'd'),
            ],
        );
        let start = |dir: &Path| {
            installed_files(dir, &m)
                .and_then(|v| start_file(&v))
                .map(|f| f.name.clone())
        };
        // Nothing.
        assert!(!catalog_installed(&dir, &m));
        assert_eq!(start(&dir), None);
        // Only part 1: incomplete.
        std::fs::write(dir.join("m-q4_k_m-00001-of-00002.gguf"), b"ab").unwrap();
        assert!(!catalog_installed(&dir, &m));
        // Only the parts (download before the fix).
        std::fs::write(dir.join("m-q4_k_m-00002-of-00002.gguf"), b"abc").unwrap();
        assert!(catalog_installed(&dir, &m));
        assert_eq!(start(&dir).as_deref(), Some("m-q4_k_m-00001-of-00002.gguf"));
        // Both: the single file is preferred.
        std::fs::write(dir.join("m-q4_k_m.gguf"), b"abcde").unwrap();
        assert_eq!(start(&dir).as_deref(), Some("m-q4_k_m.gguf"));
        assert_eq!(installed_files(&dir, &m).unwrap().len(), 1);
        // Only the single file.
        std::fs::remove_file(dir.join("m-q4_k_m-00001-of-00002.gguf")).unwrap();
        std::fs::remove_file(dir.join("m-q4_k_m-00002-of-00002.gguf")).unwrap();
        assert_eq!(start(&dir).as_deref(), Some("m-q4_k_m.gguf"));
        // Single file with wrong size and no parts: not installed.
        std::fs::write(dir.join("m-q4_k_m.gguf"), b"abc").unwrap();
        assert!(!catalog_installed(&dir, &m));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_hugging_face_hosts_are_allowed() {
        let ok = |u: &str| allowed_url(&reqwest::Url::parse(u).unwrap());
        assert!(ok("https://huggingface.co/Qwen/x/resolve/abc/f.gguf"));
        assert!(ok("https://cdn-lfs.huggingface.co/repos/x"));
        assert!(ok("https://cdn-lfs-us-1.hf.co/repos/x"));
        assert!(ok("https://cas-bridge.xethub.hf.co/xet-bridge-us/x"));
        assert!(ok("https://hf.co/x"));
        assert!(!ok("http://huggingface.co/x"));
        assert!(!ok("https://huggingface.co:8443/x"));
        assert!(!ok("https://evilhuggingface.co/x"));
        assert!(!ok("https://huggingface.co.evil.com/x"));
        assert!(!ok("https://hf.co.evil.com/x"));
        assert!(!ok("https://example.com/huggingface.co"));
        assert!(!ok("https://127.0.0.1/x"));
    }

    #[test]
    fn file_names_are_plain() {
        assert!(safe_file_name(
            "qwen2.5-coder-7b-instruct-q4_k_m-00001-of-00002.gguf"
        ));
        assert!(!safe_file_name("../a.gguf"));
        assert!(!safe_file_name("a\\b.gguf"));
        assert!(!safe_file_name("C:a.gguf"));
        assert!(!safe_file_name(".gguf"));
        assert!(!safe_file_name("a.bin"));
    }

    #[test]
    fn suggestion_by_ram() {
        // "8 GB" as reported by the OS: too little for the 7B model.
        assert_eq!(suggest(7_700_000_000), None);
        assert_eq!(suggest(11 * GIB - 1), None);
        assert!(!ai_available(8 * GIB));
        // "12 GB" (~11.7 GiB) and "16 GB" machines.
        assert_eq!(suggest(12_500_000_000), Some(LARGE_MODEL));
        assert_eq!(suggest(15_800_000_000), Some(LARGE_MODEL));
        assert_eq!(suggest(32 * GIB), Some(LARGE_MODEL));
        // Unknown RAM never blocks.
        assert_eq!(suggest(0), Some(LARGE_MODEL));
        assert!(ai_available(0));
    }

    #[test]
    fn hardware_warnings_are_german() {
        assert!(hardware_warnings(16 * GIB, Some(true)).is_empty());
        assert!(hardware_warnings(16 * GIB, None).is_empty());
        assert!(hardware_warnings(0, None).is_empty());
        let w = hardware_warnings(8 * GIB, Some(false));
        assert_eq!(w.len(), 2);
        assert!(w[0].contains("AVX2"));
        assert!(w[1].contains("8,0 GB"), "{}", w[1]);
        assert!(w[1].contains("nicht verfügbar"), "{}", w[1]);
        assert!(w[1].contains("ohne KI"), "{}", w[1]);
    }

    #[test]
    fn detected_hardware_is_plausible() {
        let h = hardware();
        assert!(h.total_ram_bytes > 0);
        assert!(h.physical_cores >= 1);
        assert_eq!(
            h.suggested_model.is_some(),
            h.ai_available,
            "{:?}",
            h.suggested_model
        );
    }

    #[test]
    fn hash_matching_uses_pinned_catalog_only() {
        let models = vec![
            pinned("a", &[("a-q4_k_m.gguf", 1, 'a')]),
            pinned(
                "b",
                &[
                    ("b-q4_k_m-00001-of-00002.gguf", 1, 'b'),
                    ("b-q4_k_m-00002-of-00002.gguf", 1, 'c'),
                ],
            ),
        ];
        match match_hash(&models, &"c".repeat(64)) {
            Match::Catalog(m, f) => {
                assert_eq!(m.id, "b");
                assert!(f.name.contains("00002"));
            }
            Match::Unknown => panic!("expected match"),
        }
        assert_eq!(match_hash(&models, &"d".repeat(64)), Match::Unknown);
        let mut unpinned = models.clone();
        unpinned[0].revision.clear();
        assert_eq!(match_hash(&unpinned, &"a".repeat(64)), Match::Unknown);
        assert_eq!(imported_id(&"e".repeat(64)), "import-eeeeeeeeeeee");
    }

    #[test]
    fn installed_needs_all_parts_with_size() {
        let dir = std::env::temp_dir().join(format!("cp-models-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (p1, p2) = (
            "p-q4_k_m-00001-of-00002.gguf",
            "p-q4_k_m-00002-of-00002.gguf",
        );
        let m = pinned("b", &[(p1, 3, 'b'), (p2, 2, 'c')]);
        assert!(!catalog_installed(&dir, &m));
        std::fs::write(dir.join(p1), b"abc").unwrap();
        assert!(!catalog_installed(&dir, &m));
        std::fs::write(dir.join(p2), b"abc").unwrap();
        assert!(!catalog_installed(&dir, &m), "wrong size");
        std::fs::write(dir.join(p2), b"ab").unwrap();
        assert!(catalog_installed(&dir, &m));
        assert_eq!(part_path(&dir.join("x.gguf")), dir.join("x.gguf.part"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn config_roundtrip_and_tolerance() {
        let dir = std::env::temp_dir().join(format!("cp-aicfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(read_config_from(&dir), AiConfig::default());
        let c = AiConfig {
            selected: Some("import-abc".into()),
            imported: vec![ImportedModel {
                id: "import-abc".into(),
                name: "x.gguf".into(),
                file_name: "import-abc.gguf".into(),
                size: 5,
                sha256: "f".repeat(64),
                catalog_id: None,
            }],
        };
        write_config_to(&dir, &c).unwrap();
        assert_eq!(read_config_from(&dir), c);
        let raw = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(raw.contains("\"fileName\""), "{raw}");
        assert_eq!(parse_config("kaputt"), AiConfig::default());
        assert_eq!(parse_config("{}"), AiConfig::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn content_range_and_percent() {
        assert_eq!(content_range_start("bytes 100-199/200"), Some(100));
        assert_eq!(content_range_start("bytes */200"), None);
        assert_eq!(content_range_start("items 1-2/3"), None);
        assert_eq!(percent(50, 200), Some(25));
        assert_eq!(percent(300, 200), Some(100));
        assert_eq!(percent(1, 0), None);
        assert_eq!(gib(GIB + GIB / 2), "1,5");
    }

    #[test]
    fn unpinned_download_is_refused_before_network() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let token = CancellationToken::new();
        for m in catalog().iter().filter(|m| !m.is_pinned()) {
            let e = rt
                .block_on(download(&m.id, &token, |_| {}))
                .unwrap_err()
                .to_string();
            assert!(e.contains("keine Prüfsummen"), "{e}");
        }
        let e = rt
            .block_on(download("gibt-es-nicht", &token, |_| {}))
            .unwrap_err()
            .to_string();
        assert!(e.contains("Unbekanntes Modell"), "{e}");
    }

    #[test]
    fn import_rejects_non_gguf_files() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = std::env::temp_dir().join(format!("cp-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let token = CancellationToken::new();
        let txt = dir.join("a.txt");
        std::fs::write(&txt, b"GGUF").unwrap();
        let e = rt
            .block_on(import(&txt, false, &token, |_| {}))
            .unwrap_err();
        assert!(e.to_string().contains(".gguf"), "{e}");
        let fake = dir.join("fake.gguf");
        std::fs::write(&fake, b"PK\x03\x04 kein gguf").unwrap();
        let e = rt
            .block_on(import(&fake, true, &token, |_| {}))
            .unwrap_err();
        assert!(e.to_string().contains("keine GGUF-Datei"), "{e}");
        let e = rt
            .block_on(import(&dir.join("fehlt.gguf"), true, &token, |_| {}))
            .unwrap_err();
        assert!(e.to_string().contains("nicht gefunden"), "{e}");
        // GGUF magic, but metadata outside the plausible ranges.
        let odd = dir.join("odd.gguf");
        std::fs::write(&odd, b"GGUF\x01\x00\x00\x00rest").unwrap();
        let e = rt
            .block_on(import(&odd, true, &token, |_| {}))
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("ungewöhnliche Metadaten (GGUF-Version 1)"),
            "{e}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gguf_magic_and_quant_in_file_name() {
        assert!(is_gguf_magic(b"GGUF\x03\x00\x00\x00"));
        assert!(!is_gguf_magic(b"PK\x03\x04"));
        assert!(!is_gguf_magic(b"GGU"));
        assert!(!is_gguf_magic(b"gguf"));
        assert!(name_matches_quant(
            "Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf",
            "q4_k_m"
        ));
        assert!(name_matches_quant(
            "qwen2.5-coder-7b-instruct-q4_k_m-00002-of-00002.gguf",
            "q4_k_m"
        ));
        assert!(!name_matches_quant(
            "qwen2.5-coder-7b-instruct-q5_k_m.gguf",
            "q4_k_m"
        ));
        let large = catalog().iter().find(|m| m.id == LARGE_MODEL).unwrap();
        assert!(name_warnings(large, &["qwen2.5-coder-7b-instruct-q4_k_m.gguf"]).is_empty());
        let w = name_warnings(
            large,
            &["qwen2.5-coder-7b-instruct-q4_k_m.gguf", "llama-q8_0.gguf"],
        );
        assert_eq!(w.len(), 1);
        assert!(
            w[0].contains("llama-q8_0.gguf") && w[0].contains("*q4_k_m*.gguf"),
            "{}",
            w[0]
        );
    }

    #[test]
    fn split_part_names() {
        assert_eq!(
            split_part("qwen2.5-coder-7b-instruct-q4_k_m-00001-of-00002.gguf"),
            Some(("qwen2.5-coder-7b-instruct-q4_k_m", 1, 2))
        );
        assert_eq!(split_part("X-00003-of-00003.GGUF"), Some(("X", 3, 3)));
        assert_eq!(split_part("x-00003-of-00002.gguf"), None);
        assert_eq!(split_part("x-00000-of-00002.gguf"), None);
        assert_eq!(split_part("x-0001-of-00002.gguf"), None);
        assert_eq!(split_part("-00001-of-00002.gguf"), None);
        assert_eq!(split_part("x-00001-of-00002.bin"), None);
        assert_eq!(split_part("model-q4_k_m.gguf"), None);
        // No panic on multibyte characters near the suffix.
        assert_eq!(split_part("ä-00001-öf-00002.gguf"), None);
        assert_eq!(split_part("äääääääääääääääääääää.gguf"), None);
    }

    #[test]
    fn part_sets() {
        let p = |i: u32| format!("m-q4_k_m-{i:05}-of-00003.gguf");
        assert_eq!(part_set(&[]), PartSet::Missing(vec![]));
        assert_eq!(
            part_set(&["a-q5_k_m.gguf"]),
            PartSet::Complete {
                first: "a-q5_k_m.gguf".into()
            }
        );
        assert!(matches!(
            part_set(&["a.gguf", "b.gguf"]),
            PartSet::Invalid(_)
        ));
        assert_eq!(part_set(&[&p(2)]), PartSet::Missing(vec![p(1), p(3)]));
        assert_eq!(
            part_set(&[&p(3), &p(1), &p(2), &p(1)]),
            PartSet::Complete { first: p(1) }
        );
        assert!(matches!(part_set(&[&p(1), "a.gguf"]), PartSet::Invalid(_)));
        assert!(matches!(
            part_set(&[&p(1), "other-00002-of-00003.gguf"]),
            PartSet::Invalid(_)
        ));
        assert!(matches!(
            part_set(&[&p(1), "m-q4_k_m-00002-of-00004.gguf"]),
            PartSet::Invalid(_)
        ));
    }

    #[test]
    fn unverified_parts_need_files_with_recorded_size() {
        let dir = std::env::temp_dir().join(format!("cp-unv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = |name: &str, size: u64, cat: Option<&str>| ImportedModel {
            id: format!("x-{name}"),
            name: name.into(),
            file_name: name.into(),
            size,
            sha256: "a".repeat(64),
            catalog_id: cat.map(Into::into),
        };
        let p1 = "m-q4_k_m-00001-of-00002.gguf";
        let p2 = "m-q4_k_m-00002-of-00002.gguf";
        let mut c = AiConfig {
            selected: None,
            imported: vec![rec(p1, 4, Some("b")), rec(p2, 2, Some("b"))],
        };
        let sub = unverified_dir(&dir, "b");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(p1), b"GGUF").unwrap();
        let u = unverified_parts(&dir, &c, "b");
        assert_eq!(u.present, vec![p1.to_string()]);
        assert_eq!(u.state, PartSet::Missing(vec![p2.to_string()]));
        assert_eq!(u.first(), None);
        std::fs::write(sub.join(p2), b"abc").unwrap();
        assert_eq!(unverified_parts(&dir, &c, "b").first(), None, "wrong size");
        std::fs::write(sub.join(p2), b"ab").unwrap();
        assert_eq!(unverified_parts(&dir, &c, "b").first(), Some(p1));
        // Other entries and unknown imports do not count.
        assert_eq!(
            unverified_parts(&dir, &c, "a").state,
            PartSet::Missing(vec![])
        );
        c.imported[0].catalog_id = None;
        assert_eq!(unverified_parts(&dir, &c, "b").first(), None);
        assert_eq!(imported_path(&dir, &c.imported[0]), dir.join(p1));
        assert_eq!(imported_path(&dir, &c.imported[1]), sub.join(p2));
        // Old configs without catalogId still load.
        let old = parse_config(
            r#"{"imported":[{"id":"i","name":"n","fileName":"f.gguf","size":1,"sha256":"x"}]}"#,
        );
        assert_eq!(old.imported[0].catalog_id, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Import via the button of an unpinned catalog entry: confirmation,
    /// attribution, parts imported one after another, selection, delete.
    #[test]
    fn import_for_unpinned_entry_attributes_unverified_files() {
        let large = catalog().iter().find(|m| m.id == LARGE_MODEL).unwrap();
        if large.is_pinned() {
            return;
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (root, _lock) = test_dirs();
        let src = root.join("src-impfor");
        std::fs::create_dir_all(&src).unwrap();
        let p1 = "qwen2.5-coder-7b-instruct-q4_k_m-00001-of-00002.gguf";
        let p2 = "qwen2.5-coder-7b-instruct-q4_k_m-00002-of-00002.gguf";
        let gguf = super::super::gguf::tests::minimal;
        std::fs::write(src.join(p1), gguf("part one")).unwrap();
        std::fs::write(src.join(p2), gguf("two")).unwrap();
        std::fs::write(src.join("other-q8_0.gguf"), gguf("other")).unwrap();
        std::fs::write(src.join("fake-q4_k_m.gguf"), b"PK\x03\x04").unwrap();
        let token = CancellationToken::new();
        let run = |paths: &[&str], confirm: bool| {
            let paths: Vec<PathBuf> = paths.iter().map(|p| src.join(p)).collect();
            rt.block_on(import_for(LARGE_MODEL, &paths, confirm, &token, |_| {}))
        };
        let entry = |s: &AiStatus| {
            s.models
                .iter()
                .find(|m| m.id == LARGE_MODEL)
                .unwrap()
                .clone()
        };

        // Header and set checks come first, also with confirm.
        let e = run(&["fake-q4_k_m.gguf"], true).unwrap_err().to_string();
        assert!(e.contains("keine GGUF-Datei"), "{e}");
        let e = run(&[p1, "other-q8_0.gguf"], true).unwrap_err().to_string();
        assert!(e.contains("nicht zur selben"), "{e}");

        // Without confirm: nothing is copied, the outcome explains why.
        let r = run(&[p1], false).unwrap();
        assert!(r.needs_confirmation && r.unverified && r.incomplete);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.missing_parts, vec![p2.to_string()]);
        assert!(!unverified_dir(&models_dir().unwrap(), LARGE_MODEL).exists());

        // Part 1 confirmed: attributed but not complete.
        let r = run(&[p1], true).unwrap();
        assert_eq!(r.model_id.as_deref(), Some(LARGE_MODEL));
        assert!(r.unverified && r.incomplete && !r.needs_confirmation);
        let m = entry(&r.status);
        assert!(!m.installed && !m.verified && m.known);
        assert_eq!(
            (m.imported_parts, m.missing_parts.clone()),
            (1, vec![p2.to_string()])
        );
        assert!(
            !r.status.models.iter().any(|m| !m.known),
            "no unknown entry"
        );
        let e = select(LARGE_MODEL).unwrap_err().to_string();
        assert!(e.contains("erst importieren"), "{e}");
        assert!(selected_model().is_err());

        // Part 2: complete, selected automatically, tier params of the entry.
        let r = run(&[p2], true).unwrap();
        assert!(!r.incomplete && r.missing_parts.is_empty());
        let m = entry(&r.status);
        assert!(m.installed && !m.verified && m.selected);
        assert_eq!(
            m.size_bytes,
            (gguf("part one").len() + gguf("two").len()) as u64
        );
        let sel = selected_model().unwrap();
        assert_eq!(sel.solutions, large.solutions);
        assert_eq!(
            sel.path,
            unverified_dir(&models_dir().unwrap(), LARGE_MODEL).join(p1)
        );

        // A file with an unexpected name: warning, and it replaces the parts.
        let r = run(&["other-q8_0.gguf"], false).unwrap();
        assert_eq!(r.warnings.len(), 2, "{:?}", r.warnings);
        assert!(r.warnings[0].contains("*q4_k_m*.gguf"));
        assert!(r.warnings[1].contains("ersetzt"));
        let r = run(&["other-q8_0.gguf"], true).unwrap();
        let m = entry(&r.status);
        assert!(m.installed && m.imported_parts == 1);
        let dir = unverified_dir(&models_dir().unwrap(), LARGE_MODEL);
        assert!(dir.join("other-q8_0.gguf").exists() && !dir.join(p1).exists());

        // Delete removes the files and the attribution.
        let st = rt.block_on(delete(LARGE_MODEL)).unwrap();
        let m = entry(&st);
        assert!(!m.installed && m.imported_parts == 0 && !m.selected);
        assert!(!dir.exists());
        assert!(read_config().imported.is_empty());
        std::fs::remove_dir_all(&src).unwrap();
    }

    /// The app's config and data directories for tests that need them
    /// (set once per process), held exclusively while the guard lives.
    fn test_dirs() -> (PathBuf, std::sync::MutexGuard<'static, ()>) {
        static LOCK: Mutex<()> = Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("cp-models-app-{}", std::process::id()));
        if CONFIG_DIR.get().is_none() {
            let _ = std::fs::remove_dir_all(&root);
            init(root.join("config"), root.join("data"));
        }
        let _ = std::fs::remove_dir_all(models_dir().unwrap());
        write_config_to(config_dir().unwrap(), &AiConfig::default()).unwrap();
        (root, guard)
    }

    /// Users who downloaded the removed 1.5B model: no crash, a note with
    /// the size, the selection falls back (7B if installed, else none),
    /// and deleting frees the space.
    #[test]
    fn retired_model_is_shown_and_can_be_deleted() {
        let (_root, _lock) = test_dirs();
        let old = &RETIRED_MODELS[0];
        let dir = models_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(old.files[0]);
        std::fs::write(&file, vec![0u8; 1000]).unwrap();
        std::fs::write(part_path(&file), vec![0u8; 24]).unwrap();
        let select_old = || {
            write_config_to(
                config_dir().unwrap(),
                &AiConfig {
                    selected: Some(old.id.into()),
                    imported: Vec::new(),
                },
            )
            .unwrap()
        };

        // Selected and the 7B model missing: a clear error, no model.
        select_old();
        let e = selected_model().err().unwrap().to_string();
        assert!(
            e.contains("nicht mehr unterstützt") && e.contains("7B"),
            "{e}"
        );
        assert_eq!(read_config().selected, None);
        select_old();
        let st = status();
        assert_eq!(st.selected, None);
        assert!(!st.model_ready);
        assert!(!st.models.iter().any(|m| m.id == old.id));
        assert_eq!(
            st.retired,
            vec![RetiredInfo {
                id: old.id.into(),
                name: old.name.into(),
                size_bytes: 1024,
                note: retired_note(old),
            }]
        );
        assert_eq!(read_config().selected, None);
        let e = select(old.id).unwrap_err().to_string();
        assert!(e.contains("nicht mehr unterstützt"), "{e}");

        // With the 7B model installed it takes over the selection.
        let large = catalog().iter().find(|m| m.id == LARGE_MODEL).unwrap();
        if large.is_pinned() {
            for f in effective_files(&large.files) {
                // Sparse file: only the size is checked here.
                std::fs::File::create(dir.join(&f.name))
                    .unwrap()
                    .set_len(f.size)
                    .unwrap();
            }
            select_old();
            let st = status();
            assert_eq!(st.selected.as_deref(), Some(LARGE_MODEL));
            assert!(st.model_ready);
            assert_eq!(read_config().selected.as_deref(), Some(LARGE_MODEL));
            select_old();
            assert_eq!(selected_model().unwrap().id, LARGE_MODEL);
            for f in effective_files(&large.files) {
                std::fs::remove_file(dir.join(&f.name)).unwrap();
            }
        }

        // Deleting removes the files and the note.
        select_old();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let st = rt.block_on(delete(old.id)).unwrap();
        assert!(st.retired.is_empty());
        assert!(!file.exists() && !part_path(&file).exists());
        assert_eq!(read_config().selected, None);
    }

    /// Minimal HTTP server for `download_file` (plain http on 127.0.0.1;
    /// the https/host checks are tested in `only_hugging_face_hosts_are_allowed`).
    async fn serve(body: Vec<u8>, honor_range: bool) -> reqwest::Url {
        use tokio::io::AsyncBufReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    let (r, mut w) = sock.into_split();
                    let mut lines = tokio::io::BufReader::new(r).lines();
                    let mut start = 0usize;
                    while let Ok(Some(l)) = lines.next_line().await {
                        if l.is_empty() {
                            break;
                        }
                        if let Some(v) = l.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            start = v.trim_end_matches('-').parse().unwrap();
                        }
                    }
                    let head = if honor_range && start > 0 {
                        format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                            body.len() - start, start, body.len() - 1, body.len()
                        )
                    } else {
                        start = 0;
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                    };
                    let _ = w.write_all(head.as_bytes()).await;
                    let _ = w.write_all(&body[start..]).await;
                    let _ = w.shutdown().await;
                });
            }
        });
        reqwest::Url::parse(&format!("http://{addr}/model.gguf")).unwrap()
    }

    #[test]
    fn download_resumes_verifies_and_renames() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let dir = std::env::temp_dir().join(format!("cp-dl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let sha = hex(&Sha256::digest(&body));
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let token = CancellationToken::new();
        rt.block_on(async {
            for honor_range in [true, false] {
                let url = serve(body.clone(), honor_range).await;
                let dest = dir.join(format!("m-{honor_range}.gguf"));
                // Interrupted download: the first 1000 bytes are already there.
                std::fs::write(part_path(&dest), &body[..1000]).unwrap();
                let mut last = 0;
                download_file(
                    &client,
                    url.clone(),
                    &dest,
                    body.len() as u64,
                    &sha,
                    0,
                    body.len() as u64,
                    &token,
                    &mut |d, _| last = d,
                )
                .await
                .unwrap();
                assert_eq!(std::fs::read(&dest).unwrap(), body);
                assert!(!part_path(&dest).exists());
                assert_eq!(last, body.len() as u64);

                // Wrong hash: the file is discarded.
                let bad = dir.join(format!("bad-{honor_range}.gguf"));
                let e = download_file(
                    &client,
                    url,
                    &bad,
                    body.len() as u64,
                    &"0".repeat(64),
                    0,
                    1,
                    &token,
                    &mut |_, _| {},
                )
                .await
                .unwrap_err();
                assert!(e.to_string().contains("Prüfsumme"), "{e}");
                assert!(!bad.exists() && !part_path(&bad).exists());
            }
            // Cancelled before the request.
            let url = serve(body.clone(), true).await;
            let cancelled = CancellationToken::new();
            cancelled.cancel();
            let e = download_file(
                &client,
                url,
                &dir.join("c.gguf"),
                body.len() as u64,
                &sha,
                0,
                1,
                &cancelled,
                &mut |_, _| {},
            )
            .await
            .unwrap_err();
            assert!(matches!(e, AiError::Cancelled));
        });
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
