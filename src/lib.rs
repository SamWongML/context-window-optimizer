//! # Context Window Optimizer
//!
//! A Rust library and MCP server that intelligently selects and packs code context
//! for LLM agents. Solves the context window optimization problem: given a token
//! budget, select the most valuable code fragments while respecting deduplication
//! and diversity constraints.
//!
//! ## Quick Start
//!
//! ```no_run
//! use ctx_optim::{config::Config, types::Budget, pack_files};
//! use std::path::PathBuf;
//!
//! let config = Config::default();
//! let budget = Budget::standard(128_000);
//! let result = pack_files(".", &budget, &[], &config).unwrap();
//!
//! println!("Selected {} files using {} tokens",
//!     result.stats.files_selected,
//!     result.stats.tokens_used);
//! ```
//!
//! ## Architecture
//!
//! ```text
//! Delivery (MCP/CLI) → Selection (Knapsack) → Scoring → Indexing (Files/AST/Dedup)
//! ```
//!
//! - [`index`] — file discovery, token counting, deduplication
//! - [`scoring`] — per-signal scoring (recency, size, proximity)
//! - [`selection`] — greedy knapsack solver
//! - [`output`] — L1/L2/L3 hierarchical formatting
//! - [`mcp`] — MCP server tools and stdio handler

pub mod config;
pub mod error;
pub mod index;
pub mod mcp;
pub mod output;
pub mod scoring;
pub mod selection;
pub mod types;

use config::Config;
use error::OptimError;
use index::{
    dedup::{dedup_by_hash, dedup_near_duplicates},
    discovery::{DiscoveryOptions, discover_files},
};
use output::format::{format_l1, format_l2, format_l3};
use scoring::score_entries;
use selection::knapsack::select_items;
use std::path::{Path, PathBuf};
use types::{Budget, PackResult, PackStats};

/// Run the full optimization pipeline for a repository.
///
/// This is the main entry point for both CLI and MCP usage.
///
/// ## Pipeline
///
/// 1. **Discover** files via the `ignore` walker (respects `.gitignore`)
/// 2. **Deduplicate** by MD5 hash
/// 3. **Score** each file using recency, size, and proximity signals
/// 4. **Select** using greedy knapsack within the L3 token budget
/// 5. **Format** L1/L2/L3 output layers
///
/// # Errors
///
/// Returns `OptimError::EmptyRepo` if no files pass the discovery filters.
///
/// # Examples
///
/// ```no_run
/// use ctx_optim::{config::Config, types::Budget, pack_files};
///
/// let config = Config::default();
/// let budget = Budget::standard(128_000);
/// let result = pack_files(".", &budget, &[], &config).unwrap();
/// assert!(!result.selected.is_empty());
/// ```
pub fn pack_files(
    repo: impl AsRef<Path>,
    budget: &Budget,
    focus_paths: &[PathBuf],
    config: &Config,
) -> Result<PackResult, OptimError> {
    pack_files_with_options(repo, budget, focus_paths, config, false)
}

/// Like [`pack_files`] but with additional formatting options.
///
/// When `include_signatures` is `true`, the L1 output will include extracted
/// function/struct/trait signatures beneath each file entry (requires the `ast` feature).
pub fn pack_files_with_options(
    repo: impl AsRef<Path>,
    budget: &Budget,
    focus_paths: &[PathBuf],
    config: &Config,
    include_signatures: bool,
) -> Result<PackResult, OptimError> {
    let repo = repo.as_ref();
    let opts = DiscoveryOptions::from_config(config, repo);

    // Step 1: Discover
    let files = discover_files(&opts)?;
    let total_files_scanned = files.len();

    tracing::info!(
        "pack: discovered {total_files_scanned} files in {}",
        repo.display()
    );

    // Step 2: Deduplicate by MD5
    let (files, duplicates_removed) = if config.dedup.exact {
        let tagged: Vec<([u8; 16], _)> = files.into_iter().map(|f| (f.hash, f)).collect();
        dedup_by_hash(tagged)
    } else {
        (files, 0)
    };

    // Step 2b: Near-duplicate dedup (SimHash + Hamming distance)
    let (files, near_duplicates_removed) = if config.dedup.near {
        dedup_near_duplicates(
            files,
            config.dedup.hamming_threshold,
            config.dedup.shingle_size,
        )
    } else {
        (files, 0)
    };

    tracing::info!(
        "pack: {duplicates_removed} exact + {near_duplicates_removed} near duplicates removed, {} files remaining",
        files.len()
    );

    // Step 2.5: Build dependency graph from AST imports
    let dep_graph = if !focus_paths.is_empty() {
        let graph = index::depgraph::DependencyGraph::build(&files, repo);
        Some(graph)
    } else {
        None
    };

    // Step 3: Score in parallel
    let scored = score_entries(&files, &config.weights, focus_paths, dep_graph.as_ref());

    // Step 4: Select within L3 budget (greedy, KKT, or auto with diversity)
    let l3_budget = budget.l3_tokens();
    let diversity_config = if config.selection.diversity.enabled {
        Some(&config.selection.diversity)
    } else {
        None
    };
    let result = select_items(
        scored,
        l3_budget,
        &config.selection.solver,
        diversity_config,
    );
    let selected = result.selected;

    let tokens_used = result.tokens_used;
    let total_tokens_in_files: usize = files.iter().map(|f| f.token_count).sum();
    let compression_ratio = if total_tokens_in_files > 0 {
        total_tokens_in_files as f32 / tokens_used.max(1) as f32
    } else {
        1.0
    };

    tracing::info!(
        "pack: selected {} files, {tokens_used}/{l3_budget} tokens, {:.1}x compression",
        selected.len(),
        compression_ratio
    );

    // Step 5: Format outputs
    let l1_output = format_l1(&selected, include_signatures);
    let l2_output = format_l2(&selected);
    let l3_output = format_l3(&selected);

    let stats = PackStats {
        total_files_scanned,
        duplicates_removed,
        near_duplicates_removed,
        files_selected: selected.len(),
        tokens_used,
        tokens_budget: budget.total_tokens,
        compression_ratio,
        solver_used: config.selection.solver.clone(),
    };

    Ok(PackResult {
        selected,
        l1_output,
        l2_output,
        l3_output,
        stats,
    })
}
