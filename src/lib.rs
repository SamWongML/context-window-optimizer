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

/// Feedback loop: session tracking, utilization scoring, weight learning.
#[cfg(feature = "feedback")]
pub mod feedback;

/// File watcher for incremental re-indexing.
#[cfg(feature = "watch")]
pub mod watch;

use config::Config;
use error::OptimError;
use index::{
    dedup::{dedup_by_hash, dedup_near_duplicates},
    discovery::{DiscoveryOptions, discover_files},
};
use output::format::{format_l1, format_l2, format_l3};
use scoring::score_entries_with_feedback;
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
    // Fingerprints are pre-computed during discovery when config.dedup.near is enabled.
    let (files, near_duplicates_removed) = if config.dedup.near {
        dedup_near_duplicates(files, config.dedup.hamming_threshold)
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

    // Step 3: Score in parallel (with feedback multipliers if available)
    #[cfg(feature = "feedback")]
    let (effective_weights, feedback_multipliers) = {
        let db_path = std::path::Path::new(&repo).join(&config.feedback.db_path);
        load_feedback_context(&db_path, &config.weights)
    };
    #[cfg(not(feature = "feedback"))]
    let (effective_weights, feedback_multipliers) =
        { (config.weights.clone(), std::collections::HashMap::new()) };

    let scored = score_entries_with_feedback(
        &files,
        &effective_weights,
        focus_paths,
        dep_graph.as_ref(),
        &feedback_multipliers,
    );

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

    // Generate session ID for feedback tracking
    #[cfg(feature = "feedback")]
    let session_id = feedback::generate_session_id(&repo.to_string_lossy());
    #[cfg(not(feature = "feedback"))]
    let session_id = String::new();

    let result = PackResult {
        session_id: session_id.clone(),
        selected,
        l1_output,
        l2_output,
        l3_output,
        stats,
    };

    // Store session in feedback database if feature is enabled
    #[cfg(feature = "feedback")]
    {
        if let Ok(store) = feedback::store::FeedbackStore::open(
            std::path::Path::new(&repo).join(&config.feedback.db_path),
        ) {
            let session = feedback::store::Session {
                id: session_id,
                repo: repo.to_string_lossy().to_string(),
                budget: budget.total_tokens,
                created_at: feedback::unix_now(),
            };
            if let Err(e) = store.create_session(&session) {
                tracing::warn!("failed to store feedback session: {e}");
            }

            let records: Vec<feedback::store::FeedbackRecord> = result
                .selected
                .iter()
                .map(|s| feedback::store::FeedbackRecord {
                    file_path: s.entry.path.to_string_lossy().to_string(),
                    token_count: s.entry.token_count,
                    composite_score: s.composite_score,
                    was_selected: true,
                    utilization: None,
                })
                .collect();
            if let Err(e) = store.record_feedback(&session.id, &records) {
                tracing::warn!("failed to record initial feedback: {e}");
            }
        }
    }

    Ok(result)
}

#[cfg(feature = "feedback")]
fn load_feedback_context(
    db_path: &Path,
    default_weights: &config::ScoringWeights,
) -> (
    config::ScoringWeights,
    std::collections::HashMap<PathBuf, f32>,
) {
    use feedback::store::FeedbackStore;

    let store = match FeedbackStore::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!("no feedback store: {e}");
            return (default_weights.clone(), std::collections::HashMap::new());
        }
    };

    // Load learned weights if available
    let weights = match store.latest_weights() {
        Ok(Some(snap)) => config::ScoringWeights {
            recency: snap.recency,
            size: snap.size,
            proximity: snap.proximity,
            dependency: snap.dependency,
        },
        _ => default_weights.clone(),
    };

    // Build per-file feedback multipliers from historical utilization
    let multipliers = match store.all_feedback_with_utilization() {
        Ok(records) => {
            let mut file_utils: std::collections::HashMap<String, Vec<f32>> =
                std::collections::HashMap::new();
            for r in &records {
                if let Some(u) = r.utilization {
                    file_utils.entry(r.file_path.clone()).or_default().push(u);
                }
            }
            file_utils
                .into_iter()
                .map(|(path, utils)| {
                    let avg: f32 = utils.iter().sum::<f32>() / utils.len() as f32;
                    // Map utilization 0.0–1.0 to multiplier 0.5–1.5
                    let multiplier = 0.5 + avg;
                    (PathBuf::from(path), multiplier)
                })
                .collect()
        }
        Err(e) => {
            tracing::debug!("cannot load feedback multipliers: {e}");
            std::collections::HashMap::new()
        }
    };

    (weights, multipliers)
}
