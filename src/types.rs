use std::path::PathBuf;
use std::time::SystemTime;

/// Detected programming language for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Other,
}

impl Language {
    /// Infer the language from a file extension.
    ///
    /// # Examples
    /// ```
    /// use ctx_optim::types::Language;
    /// assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
    /// assert_eq!(Language::from_extension("xyz"), None);
    /// ```
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "rs" => Some(Language::Rust),
            "ts" | "tsx" => Some(Language::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "py" | "pyi" => Some(Language::Python),
            "go" => Some(Language::Go),
            _ => None,
        }
    }
}

/// Git-extracted metadata for a file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitMetadata {
    /// Days since last commit touching this file (0 = today).
    pub age_days: f64,
    /// Number of commits that have modified this file.
    pub commit_count: u32,
}

/// All metadata associated with a discovered file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileMetadata {
    /// File size in bytes.
    pub size_bytes: u64,
    /// Last filesystem modification time.
    pub last_modified: SystemTime,
    /// Git metadata, if available.
    pub git: Option<GitMetadata>,
    /// Detected language.
    pub language: Option<Language>,
}

/// Classification of a symbol extracted from AST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Interface,
    Class,
    TypeAlias,
    Impl,
}

/// A function/type signature extracted from AST via tree-sitter.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Signature {
    /// The kind of symbol.
    pub kind: SymbolKind,
    /// The extracted signature text (without body).
    pub text: String,
    /// Line number in the source file (1-indexed).
    pub line: usize,
}

/// An import/dependency reference extracted from AST.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportRef {
    /// The raw import path string (e.g., `crate::scoring::signals`, `./utils`).
    pub raw_path: String,
    /// Line number in the source file (1-indexed).
    pub line: usize,
}

/// AST analysis results for a single file.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AstData {
    /// Extracted function/type signatures.
    pub signatures: Vec<Signature>,
    /// Extracted import references.
    pub imports: Vec<ImportRef>,
}

/// A discovered file with its token count and content hash.
///
/// This is the raw unit processed by the scoring and selection pipeline.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileEntry {
    /// Absolute path to the file.
    pub path: PathBuf,
    /// Estimated token count (cl100k_base encoding).
    pub token_count: usize,
    /// MD5 hash of the file content (for exact deduplication).
    pub hash: [u8; 16],
    /// File metadata.
    pub metadata: FileMetadata,
    /// AST analysis data, if tree-sitter parsing succeeded.
    #[serde(default)]
    pub ast: Option<AstData>,
    /// 64-bit SimHash fingerprint for near-duplicate detection.
    /// `None` if near-dedup is disabled or fingerprint was not computed.
    #[serde(default)]
    pub simhash: Option<u64>,
}

/// Per-signal score breakdown, all values normalized to `[0.0, 1.0]`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ScoreSignals {
    /// How recently this file was modified (higher = more recent).
    pub recency: f32,
    /// Inverse size penalty: smaller files score higher (reduces filler).
    pub size_score: f32,
    /// Path-based proximity to focus files (1.0 = same dir, 0.0 = root).
    pub proximity: f32,
    /// Import-graph distance to focus files (1.0 = focus file, 0.0 = unreachable).
    #[serde(default)]
    pub dependency: f32,
}

/// A `FileEntry` augmented with composite and per-signal scores.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScoredEntry {
    /// The underlying file.
    pub entry: FileEntry,
    /// Weighted composite score in `[0.0, 1.0]`.
    pub composite_score: f32,
    /// Per-signal breakdown used to compute `composite_score`.
    pub signals: ScoreSignals,
}

impl ScoredEntry {
    /// Efficiency ratio used by the greedy knapsack: score per token.
    ///
    /// # Examples
    /// ```
    /// use ctx_optim::types::{ScoredEntry, FileEntry, FileMetadata, ScoreSignals};
    /// use std::time::SystemTime;
    /// use std::path::PathBuf;
    /// let entry = ScoredEntry {
    ///     entry: FileEntry {
    ///         path: PathBuf::from("src/lib.rs"),
    ///         token_count: 100,
    ///         hash: [0u8; 16],
    ///         metadata: FileMetadata {
    ///             size_bytes: 200,
    ///             last_modified: SystemTime::now(),
    ///             git: None,
    ///             language: None,
    ///         },
    ///         ast: None,
    ///         simhash: None,
    ///     },
    ///     composite_score: 0.8,
    ///     signals: ScoreSignals::default(),
    /// };
    /// assert!((entry.efficiency() - 0.008).abs() < 1e-6);
    /// ```
    pub fn efficiency(&self) -> f32 {
        if self.entry.token_count == 0 {
            0.0
        } else {
            self.composite_score / self.entry.token_count as f32
        }
    }
}

/// Token budget allocation across the three output levels.
///
/// Percentages must sum to `<= 1.0`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Budget {
    /// Total token allowance for the packed output.
    pub total_tokens: usize,
    /// Fraction allocated to L1 (skeleton map of all files).
    pub l1_pct: f32,
    /// Fraction allocated to L2 (dependency cluster expansion).
    pub l2_pct: f32,
    /// Fraction allocated to L3 (full content fragments).
    pub l3_pct: f32,
}

impl Budget {
    /// Standard budget with `5% / 25% / 70%` split.
    ///
    /// # Examples
    /// ```
    /// use ctx_optim::types::Budget;
    /// let b = Budget::standard(128_000);
    /// assert_eq!(b.l1_tokens(), 6_400);
    /// ```
    pub fn standard(total_tokens: usize) -> Self {
        Self {
            total_tokens,
            l1_pct: 0.05,
            l2_pct: 0.25,
            l3_pct: 0.70,
        }
    }

    /// Tokens available for L1 output.
    pub fn l1_tokens(&self) -> usize {
        (self.total_tokens as f32 * self.l1_pct) as usize
    }

    /// Tokens available for L2 output.
    pub fn l2_tokens(&self) -> usize {
        (self.total_tokens as f32 * self.l2_pct) as usize
    }

    /// Tokens available for L3 output.
    pub fn l3_tokens(&self) -> usize {
        (self.total_tokens as f32 * self.l3_pct) as usize
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::standard(128_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Language ──────────────────────────────────────────────────────────────

    #[test]
    fn test_language_from_extension_all_known() {
        assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("js"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("jsx"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("mjs"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("cjs"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("pyi"), Some(Language::Python));
        assert_eq!(Language::from_extension("go"), Some(Language::Go));
    }

    #[test]
    fn test_language_from_extension_unknown_returns_none() {
        assert_eq!(Language::from_extension("xyz"), None);
        assert_eq!(Language::from_extension(""), None);
        assert_eq!(Language::from_extension("md"), None);
        assert_eq!(Language::from_extension("toml"), None);
        assert_eq!(Language::from_extension("json"), None);
        assert_eq!(Language::from_extension("RS"), None); // case-sensitive
    }

    // ── Budget ────────────────────────────────────────────────────────────────

    #[test]
    fn test_budget_standard_correct_split() {
        let b = Budget::standard(100_000);
        assert_eq!(b.l1_tokens(), 5_000);
        assert_eq!(b.l2_tokens(), 25_000);
        assert_eq!(b.l3_tokens(), 70_000);
    }

    #[test]
    fn test_budget_standard_percentages_sum_to_one() {
        let b = Budget::standard(128_000);
        let sum = b.l1_pct + b.l2_pct + b.l3_pct;
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "percentages sum to {sum}, expected 1.0"
        );
    }

    #[test]
    fn test_budget_levels_never_exceed_total() {
        for total in [1_000usize, 8_000, 128_000, 1_000_000] {
            let b = Budget::standard(total);
            assert!(b.l1_tokens() <= total);
            assert!(b.l2_tokens() <= total);
            assert!(b.l3_tokens() <= total);
        }
    }

    #[test]
    fn test_budget_default_is_128k() {
        assert_eq!(Budget::default().total_tokens, 128_000);
    }

    // ── ScoredEntry::efficiency ───────────────────────────────────────────────

    fn make_scored(token_count: usize, score: f32) -> ScoredEntry {
        ScoredEntry {
            entry: FileEntry {
                path: PathBuf::from("src/lib.rs"),
                token_count,
                hash: [0u8; 16],
                metadata: FileMetadata {
                    size_bytes: token_count as u64 * 4,
                    last_modified: SystemTime::now(),
                    git: None,
                    language: None,
                },
                ast: None,
                simhash: None,
            },
            composite_score: score,
            signals: ScoreSignals::default(),
        }
    }

    #[test]
    fn test_efficiency_normal_case() {
        let e = make_scored(200, 0.8);
        let eff = e.efficiency();
        assert!((eff - 0.004).abs() < 1e-6, "expected 0.004, got {eff}");
    }

    #[test]
    fn test_efficiency_zero_tokens_returns_zero() {
        let e = make_scored(0, 0.9);
        assert_eq!(
            e.efficiency(),
            0.0,
            "zero tokens should yield 0.0 efficiency"
        );
    }

    #[test]
    fn test_efficiency_higher_score_more_efficient_at_same_token_count() {
        let high = make_scored(100, 0.9);
        let low = make_scored(100, 0.3);
        assert!(high.efficiency() > low.efficiency());
    }

    #[test]
    fn test_efficiency_fewer_tokens_more_efficient_at_same_score() {
        let small = make_scored(50, 0.5);
        let large = make_scored(500, 0.5);
        assert!(small.efficiency() > large.efficiency());
    }

    // ── Property-based ────────────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn prop_budget_standard_l3_never_exceeds_total(total in 1usize..10_000_000) {
            let b = Budget::standard(total);
            prop_assert!(b.l3_tokens() <= total, "l3={} > total={total}", b.l3_tokens());
            prop_assert!(b.l1_tokens() <= total, "l1={} > total={total}", b.l1_tokens());
            prop_assert!(b.l2_tokens() <= total, "l2={} > total={total}", b.l2_tokens());
        }

        #[test]
        fn prop_efficiency_non_negative(tokens in 0usize..100_000, score in 0.0f32..=1.0) {
            let e = make_scored(tokens, score);
            prop_assert!(e.efficiency() >= 0.0);
        }
    }
}

/// Statistics about a completed pack operation.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PackStats {
    /// Number of files scanned by the walker.
    pub total_files_scanned: usize,
    /// Exact duplicates removed before scoring.
    pub duplicates_removed: usize,
    /// Near-duplicate files removed by SimHash dedup.
    #[serde(default)]
    pub near_duplicates_removed: usize,
    /// Files selected for inclusion.
    pub files_selected: usize,
    /// Tokens used by selected files.
    pub tokens_used: usize,
    /// Token budget (from `Budget::total_tokens`).
    pub tokens_budget: usize,
    /// Compression ratio: `tokens_used / tokens_in_all_files`.
    pub compression_ratio: f32,
    /// Selection solver used (`"greedy"`, `"kkt"`, or `"auto"`).
    #[serde(default)]
    pub solver_used: String,
}

/// The result of a pack operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PackResult {
    /// Unique session identifier for feedback tracking.
    #[serde(default)]
    pub session_id: String,
    /// Files chosen by the knapsack solver, ranked by composite score.
    pub selected: Vec<ScoredEntry>,
    /// L1 output: one-line skeleton for every file in the repo.
    pub l1_output: String,
    /// L2 output: paths + signatures for selected files.
    pub l2_output: String,
    /// L3 output: full content of selected files, XML-wrapped.
    pub l3_output: String,
    /// Aggregate statistics.
    pub stats: PackStats,
}
