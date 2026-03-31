// Test fixture helpers — create temporary repositories for integration tests.

use std::path::Path;
use tempfile::TempDir;

/// A temporary repository with a known set of files.
pub struct TempRepo {
    pub dir: TempDir,
}

impl TempRepo {
    /// Create a minimal repo with a few Rust source files.
    pub fn minimal() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        write_file(root, "src/main.rs", MAIN_RS);
        write_file(root, "src/lib.rs", LIB_RS);
        write_file(root, "src/utils.rs", UTILS_RS);
        write_file(root, "README.md", README_MD);

        Self { dir }
    }

    /// Create a repo with duplicate files (same content, different paths).
    pub fn with_duplicates() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        let content = "fn duplicate() { /* same content */ }";
        write_file(root, "src/a.rs", content);
        write_file(root, "src/b.rs", content); // exact duplicate
        write_file(root, "src/c.rs", "fn unique() {}");

        Self { dir }
    }

    /// Create a larger repo for budget/selection tests.
    pub fn larger(n_files: usize) -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        for i in 0..n_files {
            let content = format!(
                "/// File {i}\npub fn function_{i}() -> usize {{\n    {i}\n}}\n"
            );
            write_file(root, &format!("src/module_{i}.rs"), &content);
        }

        Self { dir }
    }

    /// Create a repo with near-duplicate files (very similar content, small differences).
    pub fn with_near_duplicates() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // These files share ~95% identical content — only one variable name differs.
        // The more shared tokens, the lower the SimHash Hamming distance.
        let base = r#"
use std::collections::HashMap;

pub fn process_data(input: &str) -> Result<String, Box<dyn std::error::Error>> {
    let trimmed = input.trim();
    let parsed: Vec<&str> = trimmed.split(',').collect();
    let mut result = String::new();
    for item in &parsed {
        if !item.is_empty() {
            result.push_str(item.trim());
            result.push('\n');
        }
    }
    Ok(result)
}

pub fn validate(input: &str) -> bool {
    !input.is_empty() && input.len() < 1000
}

pub fn count_items(input: &str) -> usize {
    input.split(',').filter(|s| !s.is_empty()).count()
}
"#;
        let variant = r#"
use std::collections::HashMap;

pub fn process_data(input: &str) -> Result<String, Box<dyn std::error::Error>> {
    let trimmed = input.trim();
    let parsed: Vec<&str> = trimmed.split(',').collect();
    let mut result = String::new();
    for item in &parsed {
        if !item.is_empty() {
            result.push_str(item.trim());
            result.push('\n');
        }
    }
    Ok(result)
}

pub fn validate(value: &str) -> bool {
    !value.is_empty() && value.len() < 1000
}

pub fn count_items(input: &str) -> usize {
    input.split(',').filter(|s| !s.is_empty()).count()
}
"#;

        write_file(root, "src/processor_a.rs", base);
        write_file(root, "src/processor_b.rs", variant); // near-duplicate
        write_file(root, "src/unique.rs", "pub fn unique_function() -> u32 { 42 }");

        Self { dir }
    }

    /// Create a repo with files spread across multiple directories.
    pub fn with_directory_structure() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // 3 files in src/scoring/
        write_file(root, "src/scoring/signals.rs", "pub fn recency() -> f32 { 0.9 }");
        write_file(root, "src/scoring/weights.rs", "pub fn default_weights() -> f32 { 0.5 }");
        write_file(root, "src/scoring/mod.rs", "pub mod signals;\npub mod weights;");

        // 3 files in src/index/
        write_file(root, "src/index/discovery.rs", "pub fn discover() -> Vec<String> { vec![] }");
        write_file(root, "src/index/tokenizer.rs", "pub fn count_tokens() -> usize { 0 }");
        write_file(root, "src/index/mod.rs", "pub mod discovery;\npub mod tokenizer;");

        // 2 files in tests/
        write_file(root, "tests/test_scoring.rs", "fn test_score() { assert!(true); }");
        write_file(root, "tests/test_index.rs", "fn test_index() { assert!(true); }");

        Self { dir }
    }

    /// Root path of this repo.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    // ── Real-world simulation fixtures ────────────────────────────────────────

    /// A realistic mid-size Rust project (~20 files) with module hierarchy.
    ///
    /// Simulates a context-window optimizer-style codebase so tests can
    /// verify that focus-path boosting selects the right modules.
    /// Directory layout:
    ///   src/{lib,main,config,error,types}.rs
    ///   src/scoring/{mod,signals,weights}.rs
    ///   src/index/{mod,discovery,tokenizer,dedup}.rs
    ///   src/selection/{mod,knapsack}.rs
    ///   src/output/{mod,format}.rs
    ///   tests/integration.rs  docs/architecture.md  README.md  Cargo.toml
    pub fn realistic_project() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        write_file(root, "src/lib.rs", REALISTIC_LIB_RS);
        write_file(root, "src/main.rs", REALISTIC_MAIN_RS);
        write_file(root, "src/config.rs", REALISTIC_CONFIG_RS);
        write_file(root, "src/error.rs", REALISTIC_ERROR_RS);
        write_file(root, "src/types.rs", REALISTIC_TYPES_RS);

        write_file(root, "src/scoring/mod.rs", REALISTIC_SCORING_MOD_RS);
        write_file(root, "src/scoring/signals.rs", REALISTIC_SCORING_SIGNALS_RS);
        write_file(root, "src/scoring/weights.rs", REALISTIC_SCORING_WEIGHTS_RS);

        write_file(root, "src/index/mod.rs", REALISTIC_INDEX_MOD_RS);
        write_file(root, "src/index/discovery.rs", REALISTIC_INDEX_DISCOVERY_RS);
        write_file(root, "src/index/tokenizer.rs", REALISTIC_INDEX_TOKENIZER_RS);
        write_file(root, "src/index/dedup.rs", REALISTIC_INDEX_DEDUP_RS);

        write_file(root, "src/selection/mod.rs", REALISTIC_SELECTION_MOD_RS);
        write_file(root, "src/selection/knapsack.rs", REALISTIC_SELECTION_KNAPSACK_RS);

        write_file(root, "src/output/mod.rs", REALISTIC_OUTPUT_MOD_RS);
        write_file(root, "src/output/format.rs", REALISTIC_OUTPUT_FORMAT_RS);

        write_file(root, "tests/integration.rs", REALISTIC_TESTS_RS);
        write_file(root, "docs/architecture.md", REALISTIC_ARCH_MD);
        write_file(root, "README.md", REALISTIC_README_MD);
        write_file(root, "Cargo.toml", REALISTIC_CARGO_TOML);

        Self { dir }
    }

    /// A polyglot monorepo with Rust, Python, TypeScript, and Go files.
    ///
    /// Used to verify language-aware scoring and that the tool works
    /// across extension types without choking on unknown file types.
    pub fn polyglot_monorepo() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // Rust backend
        write_file(root, "backend/src/main.rs", POLYGLOT_RUST_MAIN);
        write_file(root, "backend/src/api.rs", POLYGLOT_RUST_API);
        write_file(root, "backend/src/models.rs", POLYGLOT_RUST_MODELS);
        write_file(root, "backend/Cargo.toml", POLYGLOT_RUST_CARGO);

        // Python data pipeline
        write_file(root, "pipeline/etl.py", POLYGLOT_PYTHON_ETL);
        write_file(root, "pipeline/transform.py", POLYGLOT_PYTHON_TRANSFORM);
        write_file(root, "pipeline/utils.py", POLYGLOT_PYTHON_UTILS);
        write_file(root, "pipeline/requirements.txt", "pandas==2.1.0\nnumpy==1.26.0\n");

        // TypeScript frontend
        write_file(root, "frontend/src/App.tsx", POLYGLOT_TS_APP);
        write_file(root, "frontend/src/api.ts", POLYGLOT_TS_API);
        write_file(root, "frontend/src/types.ts", POLYGLOT_TS_TYPES);
        write_file(root, "frontend/package.json", POLYGLOT_TS_PKG);

        // Go microservice
        write_file(root, "svc/main.go", POLYGLOT_GO_MAIN);
        write_file(root, "svc/handler.go", POLYGLOT_GO_HANDLER);

        // Shared config / docs
        write_file(root, "docker-compose.yml", POLYGLOT_COMPOSE);
        write_file(root, "README.md", "# Polyglot Monorepo\n\nMulti-language project.\n");

        Self { dir }
    }

    /// A repo where every file has the exact same content (all duplicates).
    ///
    /// After exact dedup, only 1 unique file should remain regardless of `n`.
    pub fn all_duplicates(n: usize) -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();
        let content = "pub fn shared() -> u32 { 42 }\n";
        for i in 0..n {
            write_file(root, &format!("src/copy_{i:04}.rs"), content);
        }
        Self { dir }
    }

    /// A repo with one very large file (> 4 000 tokens) and a few small ones.
    ///
    /// Tests that oversized files are correctly handled: they may be skipped by
    /// the knapsack when the budget is tight, but still counted in total_files_scanned.
    pub fn with_oversized_file() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // Generate a file with ~5 000 tokens worth of content (~4 bytes/token × 5 000 ≈ 20 KB)
        let big_content: String = (0..500)
            .map(|i| {
                format!(
                    "/// Documentation for function {i}.\npub fn generated_fn_{i}(x: u64, y: u64) -> u64 {{\n    x.wrapping_add(y).wrapping_mul({i})\n}}\n\n"
                )
            })
            .collect();
        write_file(root, "src/generated.rs", &big_content);

        // A few small files that are easy to pack
        write_file(root, "src/lib.rs", "pub mod generated;\n");
        write_file(root, "src/small_a.rs", "pub fn a() -> u8 { 1 }");
        write_file(root, "src/small_b.rs", "pub fn b() -> u8 { 2 }");

        Self { dir }
    }

    /// A repo with files at 5 levels of directory nesting.
    ///
    /// Verifies the ignore-crate walker recurses correctly and that
    /// proximity scoring degrades gracefully with depth.
    pub fn deeply_nested() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // Flat entry point
        write_file(root, "src/lib.rs", "pub mod level1;");

        // 5-level deep chain
        write_file(root, "src/level1/mod.rs", "pub mod level2;");
        write_file(root, "src/level1/level2/mod.rs", "pub mod level3;");
        write_file(root, "src/level1/level2/level3/mod.rs", "pub mod level4;");
        write_file(root, "src/level1/level2/level3/level4/mod.rs", "pub mod level5;");
        write_file(
            root,
            "src/level1/level2/level3/level4/level5/deep.rs",
            "pub fn deep_fn() -> usize { 42 }",
        );

        // Sibling at each level
        write_file(root, "src/level1/sibling.rs", "pub fn l1_sibling() {}");
        write_file(root, "src/level1/level2/sibling.rs", "pub fn l2_sibling() {}");
        write_file(root, "src/level1/level2/level3/sibling.rs", "pub fn l3_sibling() {}");

        Self { dir }
    }

    /// A repo mixing source files, docs, config, and binary-looking data files.
    ///
    /// Verifies that non-source files are handled gracefully (either skipped or
    /// tokenized) without panicking.
    pub fn mixed_content_types() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        write_file(root, "src/lib.rs", "pub fn run() {}");
        write_file(root, "src/utils.rs", "pub fn helper() -> bool { true }");
        write_file(root, "README.md", "# Project\n\nSee docs.\n");
        write_file(root, "Cargo.toml", "[package]\nname = \"example\"\nversion = \"0.1.0\"\n");
        write_file(root, ".env.example", "DATABASE_URL=postgres://localhost/db\nSECRET=changeme\n");
        write_file(root, "config/default.toml", "[server]\nport = 8080\n");
        write_file(root, "scripts/deploy.sh", "#!/bin/bash\necho \"Deploying...\"\n");
        write_file(root, "data/schema.sql", "CREATE TABLE users (id INTEGER PRIMARY KEY);\n");
        // A JSON config file
        write_file(
            root,
            "frontend/tsconfig.json",
            r#"{"compilerOptions":{"target":"ES2020","strict":true}}"#,
        );

        Self { dir }
    }

    /// A large realistic repo with `n_files` source files in varied subdirectories.
    ///
    /// Files vary in size (50–800 tokens), simulating a real project better than
    /// the uniform-size `larger()`. Used for performance and compression-ratio tests.
    pub fn large_realistic(n_files: usize) -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        let modules = ["core", "api", "db", "auth", "util", "config", "proto", "cli"];
        for i in 0..n_files {
            let module = modules[i % modules.len()];
            // Vary content length based on file index to get realistic token distribution
            let line_count = 5 + (i * 7 + 13) % 80; // 5-84 lines
            let content: String = std::iter::once(format!(
                "//! Auto-generated module {i} in {module}.\n\nuse std::collections::HashMap;\n\n"
            ))
            .chain((0..line_count).map(|j| {
                format!(
                    "pub fn {module}_fn_{i}_{j}(param: &str) -> Option<String> {{\n    if param.is_empty() {{ return None; }}\n    Some(param.to_string())\n}}\n\n"
                )
            }))
            .collect();

            write_file(root, &format!("src/{module}/file_{i:04}.rs"), &content);
        }

        // Add a few config / root files that are low-value
        write_file(root, "src/lib.rs", "// Library root\n");
        write_file(root, "README.md", "# Large Realistic Repo\n");
        write_file(root, "Cargo.toml", "[package]\nname = \"large-project\"\nversion = \"0.1.0\"\n");

        Self { dir }
    }

    /// A repo where all files are just barely under the configured token limit.
    ///
    /// Used to ensure that files near the max_file_tokens boundary are included
    /// in discovery (not filtered), while still exercising the knapsack under load.
    pub fn near_token_limit() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // Each file has ~350 tokens (well within the default 8000 limit, but
        // meaningful enough to force selection when budget is ~1000 tokens).
        for i in 0..10 {
            let content: String = (0..40)
                .map(|j| {
                    format!(
                        "pub fn file_{i}_fn_{j}(a: i32, b: i32) -> i32 {{ a + b + {j} }}\n"
                    )
                })
                .collect();
            write_file(root, &format!("src/module_{i}.rs"), &content);
        }

        Self { dir }
    }
}

fn write_file(root: &Path, rel_path: &str, content: &str) {
    let full = root.join(rel_path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).expect("create dirs");
    }
    std::fs::write(full, content).expect("write fixture file");
}

// ── Static fixture content ──

const MAIN_RS: &str = r#"
fn main() {
    println!("Context Window Optimizer");
}
"#;

const LIB_RS: &str = r#"
/// Library root.
pub mod utils;

/// Pack context for an LLM.
pub fn pack() -> Vec<String> {
    vec![]
}
"#;

const UTILS_RS: &str = r#"
/// Utility functions.

/// Compute the score of a file based on its age.
pub fn recency_score(age_days: f64) -> f64 {
    (-0.023 * age_days).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recency_score() {
        assert!(recency_score(0.0) > 0.99);
    }
}
"#;

const README_MD: &str = r#"
# Context Window Optimizer

A Rust-based MCP server that intelligently packs code context for LLM agents.
"#;

// ── Realistic project fixture content ─────────────────────────────────────────

const REALISTIC_LIB_RS: &str = r#"
//! Context Window Optimizer library.
//!
//! Selects the highest-value code fragments for LLM context windows.

pub mod config;
pub mod error;
pub mod index;
pub mod output;
pub mod scoring;
pub mod selection;
pub mod types;

use config::Config;
use error::OptimError;
use types::{Budget, PackResult, PackStats};

/// Run the full optimization pipeline.
pub fn pack_files(
    repo: &std::path::Path,
    budget: &Budget,
    focus_paths: &[std::path::PathBuf],
    config: &Config,
) -> Result<PackResult, OptimError> {
    let files = index::discovery::discover(repo, config)?;
    let (files, dups) = index::dedup::dedup_exact(files);
    let scored = scoring::score_all(&files, &config.weights, focus_paths);
    let selected = selection::knapsack::greedy(&scored, budget.l3_tokens);
    let l1 = output::format::l1(&scored, budget.l1_tokens);
    let l2 = output::format::l2(&selected);
    let l3 = output::format::l3(&selected);
    let tokens_used: usize = selected.iter().map(|s| s.entry.token_count).sum();
    Ok(PackResult {
        selected,
        l1_output: l1,
        l2_output: l2,
        l3_output: l3,
        stats: PackStats {
            total_files_scanned: files.len() + dups,
            duplicates_removed: dups,
            files_selected: 0,
            tokens_used,
            tokens_budget: budget.total,
            compression_ratio: 1.0,
            ..Default::default()
        },
        ..Default::default()
    })
}
"#;

const REALISTIC_MAIN_RS: &str = r#"
//! CLI entry point.

use anyhow::Result;
use clap::Parser;
use ctx_optim::{config::Config, pack_files, types::Budget};

#[derive(Parser)]
#[command(name = "ctx-optim", about = "Pack code context for LLM agents")]
struct Cli {
    /// Repository path to analyse.
    #[arg(long, default_value = ".")]
    repo: std::path::PathBuf,

    /// Token budget.
    #[arg(long, default_value_t = 128_000)]
    budget: usize,

    /// Output level: l1, l2, l3, stats.
    #[arg(long, default_value = "l3")]
    output: String,

    /// Focus paths to boost (comma-separated).
    #[arg(long)]
    focus: Vec<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::find_and_load(&cli.repo)?;
    let budget = Budget::standard(cli.budget);
    let result = pack_files(&cli.repo, &budget, &cli.focus, &config)?;
    match cli.output.as_str() {
        "l1" => print!("{}", result.l1_output),
        "l2" => print!("{}", result.l2_output),
        "stats" => eprintln!("{:?}", result.stats),
        _ => print!("{}", result.l3_output),
    }
    Ok(())
}
"#;

const REALISTIC_CONFIG_RS: &str = r#"
//! Configuration loaded from `ctx-optim.toml`.

use serde::{Deserialize, Serialize};

/// Composite scoring weights (all relative — normalized before use).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringWeights {
    pub recency: f32,
    pub size: f32,
    pub proximity: f32,
    pub dependency: f32,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self { recency: 0.4, size: 0.15, proximity: 0.2, dependency: 0.25 }
    }
}

/// Top-level configuration for the optimizer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub extra_ignore: Vec<std::path::PathBuf>,
    pub max_file_bytes: u64,
    pub max_file_tokens: usize,
    pub weights: ScoringWeights,
}

impl Config {
    /// Load from a TOML file, falling back to defaults if not found.
    pub fn find_and_load(dir: &std::path::Path) -> Result<Self, crate::error::OptimError> {
        let candidate = dir.join("ctx-optim.toml");
        if candidate.exists() {
            let text = std::fs::read_to_string(&candidate)?;
            toml::from_str(&text).map_err(|e| crate::error::OptimError::Config(e.to_string()))
        } else {
            Ok(Self::default())
        }
    }
}
"#;

const REALISTIC_ERROR_RS: &str = r#"
//! Error types for the optimizer library.

/// All errors that can occur during optimization.
#[derive(Debug, thiserror::Error)]
pub enum OptimError {
    #[error("file discovery failed: {0}")]
    Discovery(#[from] ignore::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config error: {0}")]
    Config(String),
    #[error("tokenizer error: {0}")]
    Tokenizer(String),
    #[error("no files found in {path}")]
    EmptyRepo { path: String },
    #[error("budget exceeded: requested {requested}, max {max}")]
    BudgetExceeded { requested: usize, max: usize },
}
"#;

const REALISTIC_TYPES_RS: &str = r#"
//! Core data structures.

use std::path::PathBuf;
use std::time::SystemTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitMetadata {
    pub age_days: f64,
    pub commit_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub size_bytes: u64,
    pub last_modified: SystemTime,
    pub git: Option<GitMetadata>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub token_count: usize,
    pub hash: [u8; 16],
    pub metadata: FileMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreSignals {
    pub recency: f32,
    pub size_score: f32,
    pub proximity: f32,
    pub dependency: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredEntry {
    pub entry: FileEntry,
    pub composite_score: f32,
    pub signals: ScoreSignals,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub total: usize,
    pub l1_tokens: usize,
    pub l2_tokens: usize,
    pub l3_tokens: usize,
}

impl Budget {
    pub fn standard(total: usize) -> Self {
        Self {
            total,
            l1_tokens: (total as f32 * 0.05) as usize,
            l2_tokens: (total as f32 * 0.25) as usize,
            l3_tokens: (total as f32 * 0.70) as usize,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackStats {
    pub total_files_scanned: usize,
    pub duplicates_removed: usize,
    pub near_duplicates_removed: usize,
    pub files_selected: usize,
    pub tokens_used: usize,
    pub tokens_budget: usize,
    pub compression_ratio: f32,
    pub solver_used: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackResult {
    pub session_id: String,
    pub selected: Vec<ScoredEntry>,
    pub l1_output: String,
    pub l2_output: String,
    pub l3_output: String,
    pub stats: PackStats,
}
"#;

const REALISTIC_SCORING_MOD_RS: &str = r#"
//! Composite scoring pipeline.

pub mod signals;
pub mod weights;

use crate::{config::ScoringWeights, types::{FileEntry, ScoreSignals, ScoredEntry}};
use std::path::PathBuf;

/// Score a batch of files in parallel.
pub fn score_all(
    files: &[FileEntry],
    weights: &ScoringWeights,
    focus_paths: &[PathBuf],
) -> Vec<ScoredEntry> {
    files.iter().map(|f| score_one(f, weights, focus_paths)).collect()
}

fn score_one(f: &FileEntry, w: &ScoringWeights, focus: &[PathBuf]) -> ScoredEntry {
    let recency = signals::recency(f);
    let size_score = signals::size(f);
    let proximity = signals::proximity(f, focus);
    let dependency = 0.0_f32;
    let weight_sum = w.recency + w.size + w.proximity + w.dependency;
    let composite = if weight_sum > 0.0 {
        (w.recency * recency + w.size * size_score + w.proximity * proximity
            + w.dependency * dependency)
            / weight_sum
    } else {
        0.0
    };
    ScoredEntry {
        entry: f.clone(),
        composite_score: composite.clamp(0.0, 1.0),
        signals: ScoreSignals { recency, size_score, proximity, dependency },
    }
}
"#;

const REALISTIC_SCORING_SIGNALS_RS: &str = r#"
//! Per-signal computation, all normalized to [0.0, 1.0].

use crate::types::FileEntry;
use std::path::{Path, PathBuf};

/// Recency signal: exponential decay over git commit age.
///
/// Returns 1.0 for files modified today, approaching 0.0 for very old files.
pub fn recency(f: &FileEntry) -> f32 {
    match f.metadata.git.as_ref() {
        Some(g) => (-0.023_f32 * g.age_days as f32).exp(),
        None => 0.5, // no git metadata: use neutral score
    }
}

/// Size signal: prefer files around 500 tokens, penalise very small/large.
///
/// Peak is at `target = 500` tokens; falls off on both sides.
pub fn size(f: &FileEntry) -> f32 {
    let target = 500.0_f32;
    let t = f.token_count as f32;
    if t == 0.0 {
        return 0.0;
    }
    let ratio = if t <= target { t / target } else { target / t };
    ratio.clamp(0.0, 1.0)
}

/// Proximity signal: path-based distance to the nearest focus file.
///
/// Same directory → 1.0; no focus files → 0.0.
pub fn proximity(f: &FileEntry, focus_paths: &[PathBuf]) -> f32 {
    if focus_paths.is_empty() {
        return 0.0;
    }
    focus_paths
        .iter()
        .map(|fp| path_similarity(&f.path, fp))
        .fold(0.0_f32, f32::max)
}

fn path_similarity(a: &Path, b: &Path) -> f32 {
    let a_parts: Vec<_> = a.components().collect();
    let b_parts: Vec<_> = b.components().collect();
    let common = a_parts.iter().zip(b_parts.iter()).take_while(|(x, y)| x == y).count();
    let total = a_parts.len().max(b_parts.len());
    if total == 0 { 1.0 } else { common as f32 / total as f32 }
}
"#;

const REALISTIC_SCORING_WEIGHTS_RS: &str = r#"
//! Weight management utilities.

use crate::config::ScoringWeights;

/// Return a copy of weights normalised so they sum to 1.0.
pub fn normalize(w: &ScoringWeights) -> ScoringWeights {
    let sum = w.recency + w.size + w.proximity + w.dependency;
    if sum == 0.0 {
        return w.clone();
    }
    ScoringWeights {
        recency: w.recency / sum,
        size: w.size / sum,
        proximity: w.proximity / sum,
        dependency: w.dependency / sum,
    }
}

/// Focus weights: boost proximity and dependency for focused queries.
pub fn focus_weights() -> ScoringWeights {
    ScoringWeights { recency: 0.15, size: 0.05, proximity: 0.45, dependency: 0.35 }
}
"#;

const REALISTIC_INDEX_MOD_RS: &str = r#"
//! File discovery, token counting, and deduplication.

pub mod dedup;
pub mod discovery;
pub mod tokenizer;
"#;

const REALISTIC_INDEX_DISCOVERY_RS: &str = r#"
//! File walker using the `ignore` crate.

use crate::{config::Config, error::OptimError, types::FileEntry};
use ignore::WalkBuilder;
use std::path::Path;

/// Discover all indexable files in `repo`.
pub fn discover(repo: &Path, config: &Config) -> Result<Vec<FileEntry>, OptimError> {
    let mut entries = Vec::new();
    let walker = WalkBuilder::new(repo).hidden(false).build();
    for result in walker {
        let entry = result?;
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            if let Ok(meta) = entry.metadata() {
                if meta.len() > config.max_file_bytes && config.max_file_bytes > 0 {
                    continue;
                }
                let token_count = tokenizer::count(entry.path()).unwrap_or(0);
                entries.push(FileEntry {
                    path: entry.path().to_owned(),
                    token_count,
                    hash: [0u8; 16],
                    metadata: crate::types::FileMetadata {
                        size_bytes: meta.len(),
                        last_modified: meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                        git: None,
                        language: None,
                    },
                });
            }
        }
    }
    if entries.is_empty() {
        return Err(OptimError::EmptyRepo { path: repo.display().to_string() });
    }
    Ok(entries)
}
"#;

const REALISTIC_INDEX_TOKENIZER_RS: &str = r#"
//! Token counting via tiktoken (cl100k_base).

use std::path::Path;

/// Count tokens in a file using the cl100k_base BPE vocabulary.
pub fn count(path: &Path) -> Result<usize, std::io::Error> {
    let content = std::fs::read_to_string(path)?;
    // Approximate: 1 token ≈ 4 bytes for English code.
    Ok(content.len() / 4)
}

/// Count tokens in a string slice.
pub fn count_str(text: &str) -> usize {
    text.len() / 4
}
"#;

const REALISTIC_INDEX_DEDUP_RS: &str = r#"
//! Exact (MD5) and near (SimHash) deduplication.

use crate::types::FileEntry;
use std::collections::HashMap;

/// Remove exact duplicates by MD5 content hash.
///
/// Returns `(unique_files, n_removed)`.
pub fn dedup_exact(files: Vec<FileEntry>) -> (Vec<FileEntry>, usize) {
    let original_len = files.len();
    let mut seen: HashMap<[u8; 16], ()> = HashMap::new();
    let unique: Vec<FileEntry> = files
        .into_iter()
        .filter(|f| seen.insert(f.hash, ()).is_none())
        .collect();
    let removed = original_len - unique.len();
    (unique, removed)
}
"#;

const REALISTIC_SELECTION_MOD_RS: &str = r#"
//! Context selection algorithms.

pub mod knapsack;
"#;

const REALISTIC_SELECTION_KNAPSACK_RS: &str = r#"
//! Greedy knapsack context selector.

use crate::types::ScoredEntry;

/// Select items greedily by score/token efficiency until budget is exhausted.
pub fn greedy(items: &[ScoredEntry], budget: usize) -> Vec<ScoredEntry> {
    let mut sorted: Vec<&ScoredEntry> = items.iter().collect();
    sorted.sort_unstable_by(|a, b| {
        let ea = if a.entry.token_count > 0 { a.composite_score / a.entry.token_count as f32 } else { 0.0 };
        let eb = if b.entry.token_count > 0 { b.composite_score / b.entry.token_count as f32 } else { 0.0 };
        eb.partial_cmp(&ea).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut selected = Vec::new();
    let mut used = 0usize;
    for item in sorted {
        if used + item.entry.token_count <= budget {
            used += item.entry.token_count;
            selected.push(item.clone());
        }
    }
    selected
}
"#;

const REALISTIC_OUTPUT_MOD_RS: &str = r#"
//! L1/L2/L3 output formatters.

pub mod format;
"#;

const REALISTIC_OUTPUT_FORMAT_RS: &str = r#"
//! Hierarchical output formatting.

use crate::types::ScoredEntry;

/// L1: one-line skeleton per file (path + token count).
pub fn l1(scored: &[ScoredEntry], max_tokens: usize) -> String {
    let mut out = "Repository Skeleton:\n".to_string();
    let mut used = 0usize;
    for s in scored {
        let line = format!("  {} ({} tokens)\n", s.entry.path.display(), s.entry.token_count);
        if used + line.len() / 4 > max_tokens {
            break;
        }
        used += line.len() / 4;
        out.push_str(&line);
    }
    out
}

/// L2: path + metadata block per selected file.
pub fn l2(selected: &[ScoredEntry]) -> String {
    selected
        .iter()
        .map(|s| {
            let path = s.entry.path.display();
            format!("File: {path}\n  tokens: {}\n  score: {:.3}\n", s.entry.token_count, s.composite_score)
        })
        .collect()
}

/// L3: full XML-wrapped content per selected file.
pub fn l3(selected: &[ScoredEntry]) -> String {
    let mut out = "<context>\n".to_string();
    for s in selected {
        let path = s.entry.path.display();
        out.push_str(&format!("<file path={path:?}>\n(content)\n</file>\n"));
    }
    out.push_str("</context>\n");
    out
}
"#;

const REALISTIC_TESTS_RS: &str = r#"
//! Integration tests for the realistic project fixture.

#[cfg(test)]
mod tests {
    #[test]
    fn test_placeholder() {
        // Integration tests would exercise the CLI end-to-end here.
        assert!(true);
    }
}
"#;

const REALISTIC_ARCH_MD: &str = r#"
# Architecture

## Overview

The optimizer follows a pipeline architecture:

```
Discovery → Dedup → Score → Select → Format
```

## Modules

- `index`: File walking, token counting, deduplication
- `scoring`: Per-signal computation (recency, size, proximity, dependency)
- `selection`: Greedy and KKT knapsack solvers
- `output`: L1/L2/L3 hierarchical formatters
- `mcp`: MCP server over stdio

## Scoring Signals

| Signal     | Default Weight | Description                          |
|------------|---------------|--------------------------------------|
| recency    | 0.40          | Exponential decay on git commit age  |
| size       | 0.15          | Prefers ~500 token files             |
| proximity  | 0.20          | Path distance to focus files         |
| dependency | 0.25          | Import-graph distance to focus       |
"#;

const REALISTIC_README_MD: &str = r#"
# Context Window Optimizer

A Rust-based MCP server that intelligently selects code context for LLM agents.

## Quick Start

```bash
cargo run -- pack --budget 128000 --repo .
```

## MCP Usage

Add to your MCP config:

```json
{
  "mcpServers": {
    "ctx-optim": {
      "command": "ctx-optim",
      "args": ["--mcp"]
    }
  }
}
```

Then call `pack_context` with your repo path and token budget.
"#;

const REALISTIC_CARGO_TOML: &str = r#"
[package]
name = "ctx-optim-example"
version = "0.1.0"
edition = "2024"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
ignore = "0.4"
serde = { version = "1", features = ["derive"] }
thiserror = "2"
toml = "0.8"
"#;

// ── Polyglot monorepo fixture content ─────────────────────────────────────────

const POLYGLOT_RUST_MAIN: &str = r#"
//! Rust backend entry point.
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let addr: SocketAddr = "0.0.0.0:8080".parse().unwrap();
    tracing::info!("listening on {addr}");
}
"#;

const POLYGLOT_RUST_API: &str = r#"
//! REST API handlers.
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateUserRequest { pub name: String, pub email: String }

#[derive(Debug, Serialize, Deserialize)]
pub struct UserResponse { pub id: u64, pub name: String }

pub async fn create_user(req: CreateUserRequest) -> Result<UserResponse, String> {
    Ok(UserResponse { id: 1, name: req.name })
}
"#;

const POLYGLOT_RUST_MODELS: &str = r#"
//! Database models.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub id: u64,
    pub name: String,
    pub price_cents: u64,
}
"#;

const POLYGLOT_RUST_CARGO: &str = r#"
[package]
name = "backend"
version = "0.1.0"
edition = "2024"

[dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
tracing = "0.1"
"#;

const POLYGLOT_PYTHON_ETL: &str = r#"
"""ETL pipeline — extract, transform, load."""

import pandas as pd
from transform import normalize_records
from utils import setup_logging

logger = setup_logging(__name__)


def extract(source_path: str) -> pd.DataFrame:
    """Read raw CSV data."""
    logger.info("extracting from %s", source_path)
    return pd.read_csv(source_path)


def load(df: pd.DataFrame, dest: str) -> None:
    """Write processed data."""
    df.to_parquet(dest, index=False)
    logger.info("loaded %d rows to %s", len(df), dest)


def run(source: str, dest: str) -> None:
    raw = extract(source)
    transformed = normalize_records(raw)
    load(transformed, dest)
"#;

const POLYGLOT_PYTHON_TRANSFORM: &str = r#"
"""Data transformation functions."""

import pandas as pd
import numpy as np


def normalize_records(df: pd.DataFrame) -> pd.DataFrame:
    """Clean and normalize raw records."""
    df = df.copy()
    df.columns = [c.lower().strip() for c in df.columns]
    df = df.dropna(subset=["id"])
    df["amount"] = pd.to_numeric(df.get("amount", np.nan), errors="coerce").fillna(0.0)
    return df


def aggregate_by_category(df: pd.DataFrame) -> pd.DataFrame:
    """Group and sum by category."""
    return df.groupby("category")["amount"].sum().reset_index()
"#;

const POLYGLOT_PYTHON_UTILS: &str = r#"
"""Shared utilities."""

import logging


def setup_logging(name: str) -> logging.Logger:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(name)s %(message)s")
    return logging.getLogger(name)


def chunk(lst: list, size: int):
    """Yield successive chunks from lst."""
    for i in range(0, len(lst), size):
        yield lst[i : i + size]
"#;

const POLYGLOT_TS_APP: &str = r#"
import React, { useState, useEffect } from 'react';
import { fetchUsers } from './api';
import type { User } from './types';

export default function App() {
  const [users, setUsers] = useState<User[]>([]);

  useEffect(() => {
    fetchUsers().then(setUsers).catch(console.error);
  }, []);

  return (
    <div>
      <h1>Users</h1>
      <ul>
        {users.map(u => <li key={u.id}>{u.name}</li>)}
      </ul>
    </div>
  );
}
"#;

const POLYGLOT_TS_API: &str = r#"
import type { User } from './types';

const BASE = '/api/v1';

export async function fetchUsers(): Promise<User[]> {
  const res = await fetch(`${BASE}/users`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

export async function createUser(name: string, email: string): Promise<User> {
  const res = await fetch(`${BASE}/users`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name, email }),
  });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}
"#;

const POLYGLOT_TS_TYPES: &str = r#"
export interface User {
  id: number;
  name: string;
  email: string;
  createdAt: string;
}

export interface Product {
  id: number;
  name: string;
  priceCents: number;
}

export type ApiResponse<T> = { data: T; error: null } | { data: null; error: string };
"#;

const POLYGLOT_TS_PKG: &str = r#"
{
  "name": "frontend",
  "version": "0.1.0",
  "dependencies": {
    "react": "^18.0.0",
    "react-dom": "^18.0.0"
  },
  "devDependencies": {
    "typescript": "^5.0.0"
  }
}
"#;

const POLYGLOT_GO_MAIN: &str = r#"
package main

import (
	"log"
	"net/http"
)

func main() {
	mux := http.NewServeMux()
	mux.HandleFunc("/health", healthHandler)
	log.Println("starting on :9090")
	log.Fatal(http.ListenAndServe(":9090", mux))
}

func healthHandler(w http.ResponseWriter, r *http.Request) {
	w.WriteHeader(http.StatusOK)
	w.Write([]byte(`{"status":"ok"}`))
}
"#;

const POLYGLOT_GO_HANDLER: &str = r#"
package main

import (
	"encoding/json"
	"net/http"
)

type Event struct {
	ID      string `json:"id"`
	Type    string `json:"type"`
	Payload []byte `json:"payload"`
}

func eventHandler(w http.ResponseWriter, r *http.Request) {
	var evt Event
	if err := json.NewDecoder(r.Body).Decode(&evt); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]string{"id": evt.ID})
}
"#;

const POLYGLOT_COMPOSE: &str = r#"
version: "3.9"
services:
  backend:
    build: ./backend
    ports: ["8080:8080"]
  frontend:
    build: ./frontend
    ports: ["3000:3000"]
  svc:
    build: ./svc
    ports: ["9090:9090"]
"#;
