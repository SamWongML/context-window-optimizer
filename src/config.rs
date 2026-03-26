use crate::error::OptimError;
use std::path::{Path, PathBuf};

/// Weights for the composite scoring function.
///
/// All weights are relative — they are normalized before use.
/// Set a weight to `0.0` to disable a signal entirely.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScoringWeights {
    /// Weight for file recency (git commit age).
    pub recency: f32,
    /// Weight for inverse-size scoring.
    pub size: f32,
    /// Weight for path proximity to focus files.
    pub proximity: f32,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            recency: 0.5,
            size: 0.2,
            proximity: 0.3,
        }
    }
}

/// SimHash deduplication configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DedupConfig {
    /// Enable exact (MD5) deduplication.
    pub exact: bool,
    /// Enable near-duplicate (SimHash Hamming) deduplication.
    pub near: bool,
    /// Hamming distance threshold for near-dedup (default: 3).
    pub hamming_threshold: u32,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            exact: true,
            near: false, // Phase 4 feature
            hamming_threshold: 3,
        }
    }
}

/// Top-level configuration loaded from `ctx-optim.toml`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Config {
    /// Paths (relative or absolute) to ignore in addition to `.gitignore`.
    pub extra_ignore: Vec<PathBuf>,
    /// Maximum file size to index (bytes). Files larger than this are skipped.
    pub max_file_bytes: u64,
    /// Maximum token count per file. Files over this limit are truncated or skipped.
    pub max_file_tokens: usize,
    /// Scoring weights.
    pub weights: ScoringWeights,
    /// Deduplication settings.
    pub dedup: DedupConfig,
    /// Extensions to include (empty = all non-binary).
    pub include_extensions: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            extra_ignore: vec![
                PathBuf::from("target"),
                PathBuf::from("node_modules"),
                PathBuf::from(".git"),
                PathBuf::from("dist"),
                PathBuf::from("build"),
            ],
            max_file_bytes: 512 * 1024, // 512 KB
            max_file_tokens: 8_000,
            weights: ScoringWeights::default(),
            dedup: DedupConfig::default(),
            include_extensions: vec![],
        }
    }
}

impl Config {
    /// Load config from a TOML file, falling back to defaults if not found.
    ///
    /// # Examples
    /// ```no_run
    /// use ctx_optim::config::Config;
    /// let cfg = Config::load("ctx-optim.toml").unwrap();
    /// ```
    pub fn load(path: impl AsRef<Path>) -> Result<Self, OptimError> {
        let path = path.as_ref();
        if !path.exists() {
            tracing::debug!("no config file at {}, using defaults", path.display());
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        toml::from_str(&text)
            .map_err(|e| OptimError::Config(format!("parse error in {}: {e}", path.display())))
    }

    /// Find and load config by searching upward from `start_dir`.
    ///
    /// Returns defaults if no `ctx-optim.toml` is found.
    pub fn find_and_load(start_dir: impl AsRef<Path>) -> Result<Self, OptimError> {
        let mut dir = start_dir.as_ref().to_path_buf();
        loop {
            let candidate = dir.join("ctx-optim.toml");
            if candidate.exists() {
                tracing::debug!("loading config from {}", candidate.display());
                return Self::load(candidate);
            }
            if !dir.pop() {
                break;
            }
        }
        tracing::debug!("no ctx-optim.toml found, using defaults");
        Ok(Self::default())
    }
}
