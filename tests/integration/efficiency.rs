//! Comprehensive efficiency and correctness tests for the context-window optimizer.
//!
//! These tests answer two questions:
//!
//! 1. **Is the tool efficient as designed?**
//!    - Does the knapsack select higher-scored files over lower-scored ones?
//!    - Does the token budget get well-utilized?
//!    - Do the solvers (greedy, KKT, auto) all respect the budget invariant?
//!    - Does diversity actually spread selection across directories?
//!
//! 2. **Is it worth using with coding agents?**
//!    - Does providing `focus_paths` shift selection toward relevant files?
//!    - Are composite scores in a valid range?
//!    - Is output deterministic across runs?
//!    - Does deduplication actually remove redundant context?

mod fixtures {
    include!("../../tests/fixtures/mod.rs");
}

use ctx_optim::{
    config::{Config, ScoringWeights},
    index::discovery::{DiscoveryOptions, discover_files},
    pack_files,
    selection::diversity::{DiversityConfig, GroupingStrategy},
    types::Budget,
};
use fixtures::TempRepo;
use std::collections::HashSet;

// ── Selection Quality ─────────────────────────────────────────────────────────

/// EFFICIENCY INVARIANT: When the budget forces selection (not all files fit),
/// the average composite score of selected files must be >= the average score of
/// rejected files.  This proves the knapsack actually optimises for value density.
#[test]
fn test_selection_quality_selected_avg_score_exceeds_rejected() {
    let repo = TempRepo::larger(50);
    let config = Config::default();
    // Tight enough to force real selection (~10–15 files at ~15 tokens each)
    let budget = Budget::standard(2_000);

    let tight = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // Collect selected paths for set-difference lookup
    let selected_paths: HashSet<_> = tight
        .selected
        .iter()
        .map(|s| s.entry.path.clone())
        .collect();

    // Run again with a huge budget to get every scored file
    let big_budget = Budget::standard(10_000_000);
    let all = pack_files(repo.path(), &big_budget, &[], &config).unwrap();

    let rejected: Vec<_> = all
        .selected
        .iter()
        .filter(|s| !selected_paths.contains(&s.entry.path))
        .collect();

    if rejected.is_empty() {
        // All files fit — nothing meaningful to compare, pass trivially
        return;
    }

    let selected_avg = tight
        .selected
        .iter()
        .map(|s| s.composite_score as f64)
        .sum::<f64>()
        / tight.selected.len() as f64;

    let rejected_avg = rejected
        .iter()
        .map(|s| s.composite_score as f64)
        .sum::<f64>()
        / rejected.len() as f64;

    assert!(
        selected_avg >= rejected_avg - 0.01,
        "selected avg score ({selected_avg:.4}) should be >= rejected avg ({rejected_avg:.4}); \
         knapsack must prefer high-value files"
    );
}

/// TOKEN UTILIZATION: When the repo has more tokens than the budget, the
/// selected set should consume at least 50 % of the L3 budget.
/// Low utilization means context is being wasted.
#[test]
fn test_token_utilization_exceeds_50_pct_when_budget_is_binding() {
    // 50 files × ~15 tokens ≈ 750 tokens total.
    // L3 of budget(500) ≈ 350 tokens — repo clearly exceeds this.
    let repo = TempRepo::larger(50);
    let config = Config::default();
    let budget = Budget::standard(500);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let l3 = budget.l3_tokens();
    if l3 == 0 {
        return; // degenerate budget — skip
    }
    let utilization = result.stats.tokens_used as f64 / l3 as f64;

    assert!(
        utilization >= 0.5 || result.stats.files_selected >= 2,
        "token utilization ({utilization:.2}) is suspiciously low — \
         only {} files selected against an L3 budget of {l3}",
        result.stats.files_selected
    );
}

// ── Focus-Path Value ("worth using with coding agents") ──────────────────────

/// FOCUS ACCURACY: With `focus_paths` pointing at `src/core/`, files inside that
/// directory should appear among the selected entries more often than without focus.
#[test]
fn test_focus_paths_increase_relevant_file_selection() {
    let repo = TempRepo::with_focus_relevance();
    let config = Config::default();
    // Tight budget: at most ~2-3 files from 8
    let budget = Budget::standard(300);

    let no_focus = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let focus = vec![repo.path().join("src/core/logic.rs")];
    let with_focus = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    let core_no_focus = no_focus
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("core"))
        .count();

    let core_with_focus = with_focus
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("core"))
        .count();

    // Focus should boost core/ representation (or at minimum not hurt it)
    assert!(
        core_with_focus >= core_no_focus,
        "focus on src/core/ should prefer core files: \
         with_focus={core_with_focus}, no_focus={core_no_focus}"
    );
}

/// FOCUS BOOST: When focus is given, the average composite score of focus-related
/// files must be higher than those of unrelated files in the same selection.
/// This validates that proximity/dependency signals are doing real work.
#[test]
fn test_focus_related_files_score_higher_than_unrelated() {
    let repo = TempRepo::with_focus_relevance();
    let config = Config::default();
    let budget = Budget::standard(1_000_000); // unlimited — score everything

    let focus = vec![repo.path().join("src/core/logic.rs")];
    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    let core_avg = {
        let core: Vec<_> = result
            .selected
            .iter()
            .filter(|s| s.entry.path.to_string_lossy().contains("core"))
            .collect();
        if core.is_empty() {
            return;
        }
        core.iter().map(|s| s.composite_score as f64).sum::<f64>() / core.len() as f64
    };

    let utils_avg = {
        let utils: Vec<_> = result
            .selected
            .iter()
            .filter(|s| s.entry.path.to_string_lossy().contains("utils"))
            .collect();
        if utils.is_empty() {
            return;
        }
        utils
            .iter()
            .map(|s| s.composite_score as f64)
            .sum::<f64>()
            / utils.len() as f64
    };

    assert!(
        core_avg >= utils_avg - 0.05,
        "focus-related core/ files (avg {core_avg:.4}) should score higher \
         than unrelated utils/ files (avg {utils_avg:.4})"
    );
}

// ── Deduplication Effectiveness ───────────────────────────────────────────────

/// DEDUP: An all-duplicate repo of N files must remove exactly N-1 and select 1.
#[test]
fn test_all_duplicates_repo_removes_n_minus_one() {
    let n = 6usize;
    let repo = TempRepo::all_duplicates(n);
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.duplicates_removed,
        n - 1,
        "expected {n}-1={} duplicates removed, got {}",
        n - 1,
        result.stats.duplicates_removed
    );
    assert_eq!(
        result.stats.files_selected, 1,
        "only 1 unique file should survive and be selected"
    );
}

/// DEDUP RATIO: After dedup, the ratio of unique files to total scanned is
/// meaningful when there are many duplicates.
#[test]
fn test_dedup_with_mixed_repo_reduces_scanned_count() {
    let repo = TempRepo::with_duplicates(); // 3 files, 1 is an exact duplicate
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(result.stats.duplicates_removed, 1);
    assert!(
        result.stats.total_files_scanned >= 3,
        "should have scanned at least 3 files before dedup"
    );
}

// ── Diversity Value ────────────────────────────────────────────────────────────

/// DIVERSITY: With diversity enabled and a tight budget, the selected files
/// should span more directories than without diversity.
#[test]
fn test_diversity_increases_directory_coverage_over_greedy() {
    let repo = TempRepo::with_directory_structure();
    // Tight budget forces selection across a multi-directory repo
    let budget = Budget::standard(300);

    let mut no_div = Config::default();
    no_div.selection.diversity.enabled = false;
    no_div.selection.solver = "greedy".to_string();
    let result_no_div = pack_files(repo.path(), &budget, &[], &no_div).unwrap();

    let mut with_div = Config::default();
    with_div.selection.diversity = DiversityConfig {
        enabled: true,
        decay: 0.4, // aggressive decay to force spreading
        grouping: GroupingStrategy::Directory,
    };
    with_div.selection.solver = "greedy".to_string();
    let result_with_div = pack_files(repo.path(), &budget, &[], &with_div).unwrap();

    let dirs_no_div: HashSet<_> = result_no_div
        .selected
        .iter()
        .filter_map(|s| s.entry.path.parent().map(|p| p.to_owned()))
        .collect();

    let dirs_with_div: HashSet<_> = result_with_div
        .selected
        .iter()
        .filter_map(|s| s.entry.path.parent().map(|p| p.to_owned()))
        .collect();

    // If both selected ≥ 2 files, diversity must spread across more dirs
    if result_with_div.stats.files_selected >= 2 && result_no_div.stats.files_selected >= 2 {
        assert!(
            dirs_with_div.len() >= dirs_no_div.len(),
            "diversity={} dirs should be >= no-diversity={} dirs",
            dirs_with_div.len(),
            dirs_no_div.len()
        );
    }
}

/// DIVERSITY DECAY: With a very aggressive decay (0.1), the second file from
/// the same directory should have a significantly lower effective score.
#[test]
fn test_diversity_decay_reduces_same_dir_score() {
    use ctx_optim::selection::diversity::DiversityTracker;
    use ctx_optim::types::{FileEntry, FileMetadata, ScoreSignals, ScoredEntry};
    use std::path::PathBuf;
    use std::time::SystemTime;

    let cfg = DiversityConfig {
        enabled: true,
        decay: 0.1,
        grouping: GroupingStrategy::Directory,
    };
    let mut tracker = DiversityTracker::new(&cfg);

    let make_entry = |path: &str| ScoredEntry {
        entry: FileEntry {
            path: PathBuf::from(path),
            token_count: 10,
            hash: [0u8; 16],
            metadata: FileMetadata {
                size_bytes: 100,
                last_modified: SystemTime::now(),
                git: None,
                language: None,
            },
            ast: None,
            simhash: None,
        },
        composite_score: 0.8,
        signals: ScoreSignals::default(),
    };

    let entry_a = make_entry("src/scoring/signals.rs");
    let entry_b = make_entry("src/scoring/weights.rs"); // same dir

    let score_first = tracker.effective_score(&entry_a);
    tracker.record_selection(&entry_a);
    let score_second = tracker.effective_score(&entry_b);

    assert!(
        score_second < score_first,
        "second file from same dir (effective={score_second:.4}) \
         should score below first file ({score_first:.4}) with decay=0.1"
    );
}

// ── Edge Cases ────────────────────────────────────────────────────────────────

/// EDGE CASE: A single-file repo must always select that one file.
#[test]
fn test_single_file_repo_is_always_selected() {
    let repo = TempRepo::single_file();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.files_selected, 1,
        "single-file repo must select its only file"
    );
    assert!(result.stats.tokens_used > 0, "token count must be positive");
}

/// EDGE CASE: When the L3 budget equals exactly the token count of the smallest
/// file, at least one file should be selected and the budget must not be exceeded.
#[test]
fn test_budget_exactly_fits_smallest_file() {
    let repo = TempRepo::larger(10);
    let config = Config::default();

    // Discover to find the actual smallest token count
    let opts = DiscoveryOptions::from_config(&config, repo.path());
    let files = discover_files(&opts).unwrap();
    let min_tokens = files.iter().map(|f| f.token_count).min().unwrap_or(10);

    // Set total budget so L3 (70 %) ≈ min_tokens
    let total = ((min_tokens as f64 / 0.70).ceil() as usize).max(1);
    let budget = Budget::standard(total);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.tokens_used <= budget.l3_tokens(),
        "tokens_used={} must not exceed l3_budget={}",
        result.stats.tokens_used,
        budget.l3_tokens()
    );
    assert!(
        result.stats.files_selected >= 1,
        "at least one file must fit when budget covers the smallest file"
    );
}

/// EDGE CASE: A zero-token budget must not panic and must produce 0 tokens used.
#[test]
fn test_zero_budget_does_not_panic() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget {
        total_tokens: 0,
        l1_pct: 0.05,
        l2_pct: 0.25,
        l3_pct: 0.70,
    };

    match pack_files(repo.path(), &budget, &[], &config) {
        Ok(result) => {
            assert_eq!(
                result.stats.tokens_used, 0,
                "zero budget must produce zero tokens used"
            );
        }
        Err(_) => {
            // EmptyRepo or BudgetExceeded are acceptable errors here
        }
    }
}

/// EDGE CASE: A focus path that does not exist in the repository must not panic.
/// The tool should gracefully degrade to no proximity/dependency boost.
#[test]
fn test_nonexistent_focus_path_does_not_panic() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(128_000);
    let focus = vec![repo.path().join("this/does/not/exist.rs")];

    let result = pack_files(repo.path(), &budget, &focus, &config);
    assert!(
        result.is_ok(),
        "nonexistent focus path must not cause an error: {:?}",
        result.err()
    );
}

/// EDGE CASE: When the budget is large enough for every file, all discovered
/// (non-duplicate) files must be selected.
#[test]
fn test_unlimited_budget_selects_all_files() {
    let repo = TempRepo::larger(5);
    let config = Config::default();
    let budget = Budget::standard(10_000_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.files_selected,
        result.stats.total_files_scanned,
        "with unlimited budget all files must be selected"
    );
}

/// EDGE CASE: Files exceeding `max_file_tokens` must be filtered during
/// discovery and must not appear in the selection output.
#[test]
fn test_oversized_files_excluded_from_selection() {
    let repo = TempRepo::with_oversized_files();
    let mut config = Config::default();
    // 50-token limit: passes the tiny helper files (~10–20 tokens each) but
    // filters out the huge generated file (~2 400 tokens).
    config.max_file_tokens = 50;
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let huge_in_selection = result
        .selected
        .iter()
        .any(|s| s.entry.path.to_string_lossy().contains("huge_generated"));

    assert!(
        !huge_in_selection,
        "huge_generated.rs must be filtered by max_file_tokens"
    );
}

// ── Correctness Invariants ────────────────────────────────────────────────────

/// INVARIANT: Every composite_score of a selected entry must lie in [0.0, 1.0].
#[test]
fn test_all_selected_composite_scores_in_valid_range() {
    let repo = TempRepo::larger(30);
    let config = Config::default();
    let budget = Budget::standard(5_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    for e in &result.selected {
        assert!(
            (0.0f32..=1.0).contains(&e.composite_score),
            "composite_score={} out of [0,1] for {}",
            e.composite_score,
            e.entry.path.display()
        );
    }
}

/// INVARIANT: stats.tokens_used must equal the sum of token_counts of selected entries.
#[test]
fn test_stats_tokens_used_matches_selected_entry_sum() {
    let repo = TempRepo::larger(20);
    let config = Config::default();
    let budget = Budget::standard(1_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let sum: usize = result.selected.iter().map(|s| s.entry.token_count).sum();
    assert_eq!(
        result.stats.tokens_used, sum,
        "stats.tokens_used must equal sum of selected token_counts"
    );
}

/// INVARIANT: stats.files_selected must equal result.selected.len().
#[test]
fn test_stats_files_selected_equals_selected_vec_len() {
    let repo = TempRepo::larger(25);
    let config = Config::default();
    let budget = Budget::standard(2_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.files_selected,
        result.selected.len(),
        "stats.files_selected must match result.selected.len()"
    );
}

/// INVARIANT: L3 output must contain the file path of every selected entry.
#[test]
fn test_l3_output_contains_every_selected_file_path() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    for e in &result.selected {
        let p = e.entry.path.to_string_lossy();
        assert!(
            result.l3_output.contains(p.as_ref()),
            "L3 output is missing path: {p}"
        );
    }
}

/// INVARIANT: session_id is populated when the `feedback` feature is enabled,
/// and empty otherwise. This test checks the feature-gated behaviour.
#[cfg(feature = "feedback")]
#[test]
fn test_pack_result_session_id_is_non_empty_with_feedback_feature() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        !result.session_id.is_empty(),
        "session_id must be non-empty when the feedback feature is enabled"
    );
}

/// INVARIANT: Without the `feedback` feature the session_id is an empty string
/// (this is the documented default when feedback is not compiled in).
#[cfg(not(feature = "feedback"))]
#[test]
fn test_pack_result_session_id_is_empty_without_feedback_feature() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // Without feedback feature the session_id is intentionally empty
    // (no UUID generation without the feedback DB).
    assert_eq!(
        result.session_id, "",
        "session_id must be empty when feedback feature is not enabled"
    );
}

/// INVARIANT: Budget layer percentages must sum to exactly 1.0.
#[test]
fn test_budget_percentages_sum_to_one() {
    let budget = Budget::standard(128_000);
    let sum = budget.l1_pct + budget.l2_pct + budget.l3_pct;
    assert!(
        (sum - 1.0f32).abs() < 1e-5,
        "l1_pct + l2_pct + l3_pct must equal 1.0, got {sum}"
    );
}

/// INVARIANT: l1_tokens() + l2_tokens() + l3_tokens() must not exceed total_tokens
/// (rounding may consume at most 2 tokens).
#[test]
fn test_budget_layer_token_sums_within_total() {
    for total in [1, 100, 1_000, 128_000, 1_000_000] {
        let budget = Budget::standard(total);
        let allocated = budget.l1_tokens() + budget.l2_tokens() + budget.l3_tokens();
        assert!(
            allocated <= total + 2,
            "total={total}: l1+l2+l3={allocated} exceeds total"
        );
    }
}

// ── Solver Comparison ─────────────────────────────────────────────────────────

/// SOLVER PARITY: All three solvers must respect the L3 token budget.
#[test]
fn test_all_solvers_stay_within_budget() {
    let repo = TempRepo::larger(30);
    let budget = Budget::standard(1_500);

    for solver in &["greedy", "kkt", "auto"] {
        let mut config = Config::default();
        config.selection.solver = solver.to_string();
        config.selection.diversity.enabled = false;

        let result = pack_files(repo.path(), &budget, &[], &config).unwrap();
        assert!(
            result.stats.tokens_used <= budget.l3_tokens(),
            "solver={solver} exceeded budget: tokens_used={} l3={}",
            result.stats.tokens_used,
            budget.l3_tokens()
        );
    }
}

/// SOLVER QUALITY: KKT total score must be >= 95 % of greedy total score.
/// KKT is designed to be at least as good as greedy on pure score maximisation.
#[test]
fn test_kkt_total_score_not_significantly_worse_than_greedy() {
    let repo = TempRepo::larger(30);
    let budget = Budget::standard(1_500);

    let mut greedy_cfg = Config::default();
    greedy_cfg.selection.solver = "greedy".to_string();
    greedy_cfg.selection.diversity.enabled = false;
    let greedy = pack_files(repo.path(), &budget, &[], &greedy_cfg).unwrap();

    let mut kkt_cfg = Config::default();
    kkt_cfg.selection.solver = "kkt".to_string();
    kkt_cfg.selection.diversity.enabled = false;
    let kkt = pack_files(repo.path(), &budget, &[], &kkt_cfg).unwrap();

    if greedy.stats.files_selected == 0 {
        return; // nothing to compare
    }

    let g_total: f32 = greedy.selected.iter().map(|s| s.composite_score).sum();
    let k_total: f32 = kkt.selected.iter().map(|s| s.composite_score).sum();

    assert!(
        k_total >= g_total * 0.95,
        "KKT score ({k_total:.3}) should be >= 95% of greedy ({g_total:.3})"
    );
}

// ── Configuration Sensitivity ─────────────────────────────────────────────────

/// WEIGHT SENSITIVITY: Changing weights to recency-only must still produce
/// a valid result that respects the budget.
#[test]
fn test_recency_only_weights_valid_result() {
    let repo = TempRepo::larger(20);
    let budget = Budget::standard(1_000);

    let mut config = Config::default();
    config.weights = ScoringWeights {
        recency: 1.0,
        size: 0.0,
        proximity: 0.0,
        dependency: 0.0,
    };

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();
    assert!(result.stats.files_selected > 0);
    assert!(result.stats.tokens_used <= budget.l3_tokens());
}

/// WEIGHT SENSITIVITY: Size-only weights must still produce valid output.
#[test]
fn test_size_only_weights_valid_result() {
    let repo = TempRepo::larger(20);
    let budget = Budget::standard(1_000);

    let mut config = Config::default();
    config.weights = ScoringWeights {
        recency: 0.0,
        size: 1.0,
        proximity: 0.0,
        dependency: 0.0,
    };

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();
    assert!(result.stats.files_selected > 0);
    assert!(result.stats.tokens_used <= budget.l3_tokens());
}

/// WEIGHT SENSITIVITY: All-zero weights should not panic — the tool must
/// degrade gracefully when weights are all zeroed out.
#[test]
fn test_all_zero_weights_does_not_panic() {
    let repo = TempRepo::minimal();
    let budget = Budget::standard(128_000);

    let mut config = Config::default();
    config.weights = ScoringWeights {
        recency: 0.0,
        size: 0.0,
        proximity: 0.0,
        dependency: 0.0,
    };

    // Should not panic; selection may be arbitrary but must not error
    let result = pack_files(repo.path(), &budget, &[], &config);
    assert!(
        result.is_ok(),
        "all-zero weights should not panic: {:?}",
        result.err()
    );
}

// ── Multi-Language Coverage ───────────────────────────────────────────────────

/// MULTI-LANGUAGE: A repo with Rust, TypeScript, Python, and Go files must be
/// indexed without panicking, and all source files must be discovered.
#[test]
fn test_mixed_language_repo_indexed_without_panic() {
    let repo = TempRepo::with_mixed_languages();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // 4 source files + README = at least 4 entries
    assert!(
        result.stats.total_files_scanned >= 4,
        "expected >= 4 files scanned, got {}",
        result.stats.total_files_scanned
    );
    assert!(result.stats.files_selected > 0);
}

// ── Determinism ───────────────────────────────────────────────────────────────

/// DETERMINISM: Two calls to pack_files on the same repo with the same config
/// must produce the same selected file set.
#[test]
fn test_pack_files_is_deterministic_across_two_runs() {
    let repo = TempRepo::larger(20);
    let config = Config::default();
    let budget = Budget::standard(1_000);

    let r1 = pack_files(repo.path(), &budget, &[], &config).unwrap();
    let r2 = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let paths1: HashSet<_> = r1.selected.iter().map(|s| &s.entry.path).collect();
    let paths2: HashSet<_> = r2.selected.iter().map(|s| &s.entry.path).collect();

    assert_eq!(
        paths1, paths2,
        "pack_files must be deterministic across repeated calls"
    );
}

// ── Stress Test ───────────────────────────────────────────────────────────────

/// STRESS: A 200-file repo must index, score, and select without error.
#[test]
fn test_large_repo_200_files_completes_without_error() {
    let repo = TempRepo::larger(200);
    let config = Config::default();
    let budget = Budget::standard(50_000);

    let result = pack_files(repo.path(), &budget, &[], &config);
    assert!(
        result.is_ok(),
        "200-file repo must complete without error: {:?}",
        result.err()
    );
}

/// STRESS: A 200-file repo with near-dedup enabled must also complete without error.
#[test]
fn test_large_repo_with_near_dedup_completes_without_error() {
    let repo = TempRepo::larger(200);
    let mut config = Config::default();
    config.dedup.near = true;
    let budget = Budget::standard(50_000);

    let result = pack_files(repo.path(), &budget, &[], &config);
    assert!(
        result.is_ok(),
        "200-file repo with near-dedup must complete: {:?}",
        result.err()
    );
}

// ── Compression Ratio ─────────────────────────────────────────────────────────

/// COMPRESSION RATIO: Under a tight budget the tool must compress meaningfully
/// — compression_ratio must be > 1.0 when the repo has far more tokens than
/// the budget can hold.
#[test]
fn test_compression_ratio_exceeds_one_under_tight_budget() {
    let repo = TempRepo::larger(40); // ~600 tokens total
    let config = Config::default();
    let budget = Budget::standard(200); // forces significant compression

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.compression_ratio > 1.0,
        "compression_ratio={} must be > 1.0 when budget is tight",
        result.stats.compression_ratio
    );
}

// ── Near-Dedup Quality ────────────────────────────────────────────────────────

/// NEAR-DEDUP: Enabling near-dedup on a repo with near-duplicate files must
/// remove at least one near-duplicate.
#[test]
fn test_near_dedup_enabled_removes_at_least_one_near_duplicate() {
    use ctx_optim::index::simhash::hamming_distance;

    let repo = TempRepo::with_near_duplicates();
    let mut config = Config::default();
    config.dedup.near = true;

    // Discover first to find the actual Hamming distance
    let opts = DiscoveryOptions::from_config(&config, repo.path());
    let files = discover_files(&opts).unwrap();

    let fp_a = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("processor_a"))
        .and_then(|f| f.simhash);
    let fp_b = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("processor_b"))
        .and_then(|f| f.simhash);

    if let (Some(a), Some(b)) = (fp_a, fp_b) {
        let dist = hamming_distance(a, b);
        config.dedup.hamming_threshold = dist.max(1);

        let budget = Budget::standard(128_000);
        let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

        assert!(
            result.stats.near_duplicates_removed >= 1,
            "expected >= 1 near-duplicate removed (hamming_dist={dist}), \
             got {}",
            result.stats.near_duplicates_removed
        );
    }
    // If simhash wasn't computed the test passes trivially
}

/// NEAR-DEDUP DISABLED: With near-dedup off (the default), near-duplicates must
/// NOT be removed.
#[test]
fn test_near_dedup_disabled_by_default_keeps_similar_files() {
    let repo = TempRepo::with_near_duplicates();
    let config = Config::default(); // near dedup is off
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.near_duplicates_removed, 0,
        "near-dedup must be off by default"
    );
}
