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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Write a minimal valid TOML config to a temp dir, optionally with overrides.
    fn write_config(dir: &TempDir, extra: &str) -> std::path::PathBuf {
        let content = format!(
            r#"
extra_ignore = []
max_file_bytes = 524288
max_file_tokens = 8000
include_extensions = []

[weights]
recency = 0.5
size = 0.2
proximity = 0.3

[dedup]
exact = true
near = false
hamming_threshold = 3
{extra}
"#
        );
        let path = dir.path().join("ctx-optim.toml");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_default_has_expected_fields() {
        let cfg = Config::default();
        assert!(cfg.extra_ignore.contains(&PathBuf::from("target")));
        assert_eq!(cfg.max_file_bytes, 512 * 1024);
        assert_eq!(cfg.max_file_tokens, 8_000);
        assert!(cfg.dedup.exact);
        assert!(!cfg.dedup.near);
        assert!(cfg.include_extensions.is_empty());
    }

    #[test]
    fn test_load_nonexistent_path_returns_default() {
        let tmp = TempDir::new().unwrap();
        let cfg = Config::load(tmp.path().join("nonexistent.toml")).unwrap();
        assert_eq!(cfg.max_file_tokens, Config::default().max_file_tokens);
    }

    #[test]
    fn test_load_valid_toml_parses_correctly() {
        let tmp = TempDir::new().unwrap();
        let content = r#"
extra_ignore = ["dist"]
max_file_bytes = 1024
max_file_tokens = 500
include_extensions = ["rs", "toml"]

[weights]
recency = 0.8
size = 0.1
proximity = 0.1

[dedup]
exact = true
near = false
hamming_threshold = 3
"#;
        let path = tmp.path().join("ctx-optim.toml");
        std::fs::write(&path, content).unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.max_file_bytes, 1024);
        assert_eq!(cfg.max_file_tokens, 500);
        assert_eq!(cfg.include_extensions, vec!["rs", "toml"]);
        assert!((cfg.weights.recency - 0.8).abs() < 1e-6);
        assert!((cfg.weights.size - 0.1).abs() < 1e-6);
        assert_eq!(cfg.extra_ignore, vec![PathBuf::from("dist")]);
    }

    #[test]
    fn test_load_invalid_toml_returns_config_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("bad.toml");
        std::fs::write(&path, "this is ][[ not valid toml!").unwrap();

        let result = Config::load(&path);
        assert!(
            matches!(result, Err(OptimError::Config(_))),
            "expected Config error, got: {result:?}"
        );
    }

    #[test]
    fn test_find_and_load_finds_in_same_dir() {
        let tmp = TempDir::new().unwrap();
        write_config(&tmp, "max_file_tokens = 42");
        // Overwrite with specific token value
        let content = r#"
extra_ignore = []
max_file_bytes = 524288
max_file_tokens = 42
include_extensions = []

[weights]
recency = 0.5
size = 0.2
proximity = 0.3

[dedup]
exact = true
near = false
hamming_threshold = 3
"#;
        std::fs::write(tmp.path().join("ctx-optim.toml"), content).unwrap();

        let cfg = Config::find_and_load(tmp.path()).unwrap();
        assert_eq!(cfg.max_file_tokens, 42);
    }

    #[test]
    fn test_find_and_load_finds_in_parent_dir() {
        let tmp = TempDir::new().unwrap();
        let subdir = tmp.path().join("nested").join("deep");
        std::fs::create_dir_all(&subdir).unwrap();

        let content = r#"
extra_ignore = []
max_file_bytes = 524288
max_file_tokens = 999
include_extensions = []

[weights]
recency = 0.5
size = 0.2
proximity = 0.3

[dedup]
exact = true
near = false
hamming_threshold = 3
"#;
        std::fs::write(tmp.path().join("ctx-optim.toml"), content).unwrap();

        let cfg = Config::find_and_load(&subdir).unwrap();
        assert_eq!(cfg.max_file_tokens, 999, "should have found parent config");
    }

    #[test]
    fn test_find_and_load_no_file_returns_default() {
        let tmp = TempDir::new().unwrap();
        let cfg = Config::find_and_load(tmp.path()).unwrap();
        assert_eq!(cfg.max_file_tokens, Config::default().max_file_tokens);
    }

    #[test]
    fn test_scoring_weights_default_all_positive() {
        let w = ScoringWeights::default();
        assert!(w.recency > 0.0);
        assert!(w.size > 0.0);
        assert!(w.proximity > 0.0);
    }

    #[test]
    fn test_dedup_config_default_exact_enabled_near_disabled() {
        let d = DedupConfig::default();
        assert!(d.exact);
        assert!(!d.near);
        assert_eq!(d.hamming_threshold, 3);
    }
}
