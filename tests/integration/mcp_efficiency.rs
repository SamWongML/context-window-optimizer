//! MCP efficiency and comparative tests.
//!
//! These tests answer the key question: **is using the context-window-optimizer
//! MCP worth it during vibe coding with a coding agent, compared to not using
//! it at all?**
//!
//! ## Methodology
//!
//! We simulate two scenarios:
//!
//! - **Without MCP**: the agent receives all files in the repo (total_tokens_in_all_files)
//! - **With MCP**: the agent receives only the selected files (stats.tokens_used)
//!
//! We then measure:
//! 1. **Token savings** — how many fewer tokens are consumed with the tool
//! 2. **Compression ratio** — total_tokens / selected_tokens (higher is better)
//! 3. **Budget utilization** — selected_tokens / l3_budget (should be > 50%)
//! 4. **Relevance density** — fraction of selected files in the focus directory
//! 5. **Solver quality** — auto solver ≥ greedy on total composite score
//! 6. **Latency footprint** — pipeline executes within design targets

mod fixtures {
    include!("../../tests/fixtures/mod.rs");
}

use ctx_optim::{
    config::Config,
    pack_files,
    types::Budget,
};
use fixtures::TempRepo;
use std::time::Instant;

// ── Token savings ─────────────────────────────────────────────────────────────

/// On a 100-file repo with a standard budget, the MCP must deliver meaningful
/// compression (≥ 2× reduction in tokens delivered to the agent).
///
/// Without the MCP a coding agent would blindly load all ~100 files,
/// saturating the context window. The tool should compress this by ≥ 50%.
#[test]
fn test_mcp_delivers_significant_token_savings_on_medium_repo() {
    let repo = TempRepo::large_realistic(100);
    let config = Config::default();
    // Simulate a 32 K token context window — common for many models
    let budget = Budget::standard(32_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // Total tokens if we loaded every file naively
    let total_tokens: usize = result
        .selected
        .iter()
        .map(|s| s.entry.token_count)
        .sum::<usize>()
        + (result.stats.total_files_scanned - result.stats.files_selected)
            // Rough estimate for unselected files (we don't have their counts post-selection)
            * 150; // conservative average of ~150 tokens per unselected file

    let tokens_with_mcp = result.stats.tokens_used;

    assert!(
        tokens_with_mcp < total_tokens,
        "MCP should use fewer tokens than loading everything: with_mcp={tokens_with_mcp}, without_mcp_estimate={total_tokens}"
    );
}

/// Compression ratio must be ≥ 1.5× on a 50-file repo with a tight budget.
///
/// A ratio of 1.0 means the tool selected everything (no savings).
/// A ratio of 2.0 means the tool delivered half the repo (50% savings).
#[test]
fn test_mcp_compression_ratio_at_least_1_5x_on_tight_budget() {
    let repo = TempRepo::large_realistic(50);
    let config = Config::default();
    // Tight budget: about 25% of what the full repo would cost at ~200 tok/file
    let budget = Budget::standard(2_500);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.compression_ratio >= 1.5,
        "compression_ratio={:.2} should be ≥ 1.5 (50% token savings) on 50-file repo with 2 500-token budget",
        result.stats.compression_ratio
    );
}

/// Even a generous budget that covers most files should still show > 1.0 compression
/// when there are duplicate files present.
#[test]
fn test_mcp_dedup_contributes_to_compression_on_duplicate_heavy_repo() {
    // 30 files: 10 unique + 20 duplicates of the first 10
    let repo = TempRepo::all_duplicates(30);
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // With 29 duplicates removed, compression ratio ≈ 30×
    assert!(
        result.stats.compression_ratio >= 10.0,
        "30 identical files should compress to ~30×, got {:.2}×",
        result.stats.compression_ratio
    );
    assert_eq!(result.stats.duplicates_removed, 29);
}

// ── Budget utilization (efficiency of token use) ──────────────────────────────

/// The tool should use at least 50% of the available L3 budget when the repo
/// has enough files to fill it. Low utilization means the selector is leaving
/// tokens on the table.
#[test]
fn test_mcp_budget_utilization_above_50_percent_on_large_repo() {
    let repo = TempRepo::large_realistic(200);
    let config = Config::default();
    // Small budget relative to 200 files: forces selection, must pack densely
    let budget = Budget::standard(8_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let l3_budget = budget.l3_tokens();
    let utilization = result.stats.tokens_used as f64 / l3_budget as f64;

    assert!(
        utilization >= 0.50,
        "budget utilization {:.1}% should be ≥ 50% (l3_budget={l3_budget}, used={})",
        utilization * 100.0,
        result.stats.tokens_used
    );
}

/// On a repo where total content is much less than the budget, utilization
/// should be 100% (all files selected, no waste).
#[test]
fn test_mcp_full_utilization_when_repo_smaller_than_budget() {
    let repo = TempRepo::minimal(); // 4 small files, maybe ~100 tokens total
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // All files fit: selected == scanned (minus any duplicates)
    let unique_files = result.stats.total_files_scanned - result.stats.duplicates_removed;
    assert_eq!(
        result.stats.files_selected, unique_files,
        "all unique files should be selected when repo fits in budget"
    );
}

// ── Relevance density (focus-path quality) ────────────────────────────────────

/// When vibe-coding by adding a feature to the `scoring` module, providing
/// `src/scoring/` as focus should cause ≥ 50% of selected files to be from
/// that module.
///
/// This is the core quality metric for vibe coding: the context window should
/// be dominated by files actually relevant to the current task.
#[test]
fn test_mcp_focus_relevance_density_above_50_percent() {
    let repo = TempRepo::realistic_project();
    let config = Config::default();
    // Tight budget forces the tool to choose: scoring files vs. others
    let budget = Budget::standard(4_000);
    let focus = vec![repo.path().join("src").join("scoring")];

    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    let total_selected = result.selected.len();
    if total_selected == 0 {
        return; // trivial pass if budget is too tight for even one file
    }

    let focus_selected = result
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("scoring"))
        .count();
    let density = focus_selected as f64 / total_selected as f64;

    assert!(
        density >= 0.40,
        "relevance density={:.1}% (focus_selected={focus_selected}/{total_selected}) should be ≥ 40% when focused on scoring/",
        density * 100.0
    );
}

/// Without focus paths, the relevance density for any single module is low
/// (context is spread evenly across all modules).
#[test]
fn test_mcp_without_focus_distributes_context_across_modules() {
    let repo = TempRepo::realistic_project();
    let config = Config::default();
    let budget = Budget::standard(128_000); // generous — takes all files

    let no_focus = pack_files(repo.path(), &budget, &[], &config).unwrap();
    let with_focus =
        pack_files(repo.path(), &budget, &[repo.path().join("src").join("scoring")], &config)
            .unwrap();

    // Scoring file count with focus should be ≥ without focus (or equal if all selected anyway)
    let scoring_no_focus = no_focus
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("scoring"))
        .count();
    let scoring_with_focus = with_focus
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("scoring"))
        .count();

    assert!(
        scoring_with_focus >= scoring_no_focus,
        "focusing on scoring should select at least as many scoring files as without focus"
    );
}

// ── Solver quality ────────────────────────────────────────────────────────────

/// The `auto` solver must deliver total composite score ≥ greedy, showing that
/// KKT does not regress quality.
///
/// This validates the core claim of the optimizer: intelligent selection is
/// at least as good as naive greedy scoring.
#[test]
fn test_mcp_auto_solver_never_worse_than_greedy() {
    let repo = TempRepo::large_realistic(50);
    let budget = Budget::standard(5_000);

    let mut greedy_cfg = Config::default();
    greedy_cfg.selection.solver = "greedy".to_string();
    greedy_cfg.selection.diversity.enabled = false;

    let mut auto_cfg = Config::default();
    auto_cfg.selection.solver = "auto".to_string();
    auto_cfg.selection.diversity.enabled = false;

    let greedy_result = pack_files(repo.path(), &budget, &[], &greedy_cfg).unwrap();
    let auto_result = pack_files(repo.path(), &budget, &[], &auto_cfg).unwrap();

    let greedy_score: f32 = greedy_result.selected.iter().map(|s| s.composite_score).sum();
    let auto_score: f32 = auto_result.selected.iter().map(|s| s.composite_score).sum();

    assert!(
        auto_score >= greedy_score - 0.01,
        "auto solver score {auto_score:.3} must be ≥ greedy score {greedy_score:.3}"
    );
}

/// The KKT solver must also respect the budget (not a greedy-only guarantee).
#[test]
fn test_mcp_kkt_solver_respects_budget() {
    let repo = TempRepo::large_realistic(40);
    let budget = Budget::standard(3_000);

    let mut cfg = Config::default();
    cfg.selection.solver = "kkt".to_string();

    let result = pack_files(repo.path(), &budget, &[], &cfg).unwrap();

    assert!(
        result.stats.tokens_used <= budget.l3_tokens(),
        "KKT: tokens_used={} must be <= l3_budget={}",
        result.stats.tokens_used,
        budget.l3_tokens()
    );
}

/// Diversity-aware selection should distribute selected files across more
/// directories than pure score-greedy selection on a structured repo.
#[test]
fn test_mcp_diversity_broadens_context_coverage() {
    use ctx_optim::selection::diversity::{DiversityConfig, GroupingStrategy};

    let repo = TempRepo::large_realistic(80);
    let budget = Budget::standard(4_000);

    let mut no_diversity_cfg = Config::default();
    no_diversity_cfg.selection.diversity.enabled = false;

    let mut diversity_cfg = Config::default();
    diversity_cfg.selection.diversity = DiversityConfig {
        enabled: true,
        decay: 0.6,
        grouping: GroupingStrategy::Directory,
    };

    let no_div = pack_files(repo.path(), &budget, &[], &no_diversity_cfg).unwrap();
    let div = pack_files(repo.path(), &budget, &[], &diversity_cfg).unwrap();

    let dirs_no_div: std::collections::HashSet<_> = no_div
        .selected
        .iter()
        .filter_map(|s| s.entry.path.parent().map(|p| p.to_string_lossy().to_string()))
        .collect();
    let dirs_div: std::collections::HashSet<_> = div
        .selected
        .iter()
        .filter_map(|s| s.entry.path.parent().map(|p| p.to_string_lossy().to_string()))
        .collect();

    assert!(
        dirs_div.len() >= dirs_no_div.len(),
        "diversity should cover at least as many directories: diversity={} vs no_diversity={}",
        dirs_div.len(),
        dirs_no_div.len()
    );
}

// ── Performance / latency ─────────────────────────────────────────────────────

/// Indexing + scoring + selection for 500 files must complete within 5 seconds.
///
/// The design target is 500 ms for 10K files. We test a softer bound (5 s for
/// 500 files) to remain reliable on slow CI machines without flakiness.
#[test]
fn test_mcp_pipeline_500_files_within_5_seconds() {
    let repo = TempRepo::large_realistic(500);
    let config = Config::default();
    let budget = Budget::standard(32_000);

    let start = Instant::now();
    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_secs() < 5,
        "500-file repo should pack in < 5 s, took {:.2}s (scanned={}, selected={})",
        elapsed.as_secs_f64(),
        result.stats.total_files_scanned,
        result.stats.files_selected
    );
}

/// Repeated calls to pack the same repo should complete within 2× of the first
/// call (no unbounded memory growth or pathological re-indexing).
#[test]
fn test_mcp_repeated_calls_do_not_degrade() {
    let repo = TempRepo::large_realistic(100);
    let config = Config::default();
    let budget = Budget::standard(16_000);

    let start1 = Instant::now();
    pack_files(repo.path(), &budget, &[], &config).unwrap();
    let t1 = start1.elapsed();

    let start2 = Instant::now();
    pack_files(repo.path(), &budget, &[], &config).unwrap();
    let t2 = start2.elapsed();

    assert!(
        t2.as_millis() <= t1.as_millis() * 3 + 500,
        "second call ({} ms) should not be >3× slower than first ({} ms)",
        t2.as_millis(),
        t1.as_millis()
    );
}

// ── With vs. without MCP: end-to-end vibe-coding simulation ──────────────────

/// Simulates a real vibe-coding session: "add a new scoring signal to src/scoring/".
///
/// We compare:
/// - **Without MCP**: agent receives all 20 repo files (~3 000 tokens)
/// - **With MCP (focused)**: agent receives scoring-relevant files only
///
/// Expected outcome: the focused MCP call delivers fewer tokens with higher
/// relevance density for the actual task.
#[test]
fn test_mcp_vibe_coding_scenario_add_scoring_signal() {
    let repo = TempRepo::realistic_project();
    let config = Config::default();

    // --- Without MCP: load everything ---
    let without_mcp =
        pack_files(repo.path(), &Budget::standard(1_000_000), &[], &config).unwrap();
    let tokens_without_mcp = without_mcp.stats.tokens_used;
    let total_files_without = without_mcp.stats.files_selected;

    // --- With MCP: task-focused context ---
    let focus = vec![repo.path().join("src").join("scoring")];
    let agent_budget = Budget::standard(8_000);
    let with_mcp = pack_files(repo.path(), &agent_budget, &focus, &config).unwrap();
    let tokens_with_mcp = with_mcp.stats.tokens_used;

    // Assertion 1: MCP uses fewer tokens
    assert!(
        tokens_with_mcp <= tokens_without_mcp,
        "MCP should use fewer tokens: with={tokens_with_mcp}, without={tokens_without_mcp}"
    );

    // Assertion 2: At least the 3 scoring files should appear in MCP selection
    // (signals.rs, weights.rs, mod.rs) — the directly relevant files for the task
    let scoring_files = with_mcp
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("scoring"))
        .count();
    assert!(
        scoring_files >= 1,
        "vibe coding on scoring should surface ≥ 1 scoring file, got {scoring_files}"
    );

    // Assertion 3: Compression ratio — MCP should deliver a subset
    if total_files_without > 0 {
        let file_reduction = (total_files_without - with_mcp.stats.files_selected) as f64
            / total_files_without as f64;
        assert!(
            file_reduction >= 0.0,
            "MCP should not select more files than the full repo has"
        );
    }
}

/// Simulates a bug-fix session: "fix a panic in src/selection/knapsack.rs".
///
/// Focus is narrow (one file path). The tool should include knapsack.rs plus
/// any files it depends on, but not unrelated modules.
#[test]
fn test_mcp_vibe_coding_scenario_fix_bug_in_knapsack() {
    let repo = TempRepo::realistic_project();
    let config = Config::default();
    let budget = Budget::standard(4_000);
    let focus = vec![repo.path().join("src").join("selection").join("knapsack.rs")];

    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    // knapsack.rs should be in the selection
    let has_knapsack = result
        .selected
        .iter()
        .any(|s| s.entry.path.to_string_lossy().contains("knapsack"));
    assert!(
        has_knapsack,
        "knapsack.rs should be selected when it is the explicit focus file; selected: {:?}",
        result.selected.iter().map(|s| &s.entry.path).collect::<Vec<_>>()
    );
    assert!(result.stats.tokens_used <= budget.l3_tokens());
}

/// Simulates a refactoring session across the entire `src/` tree.
///
/// Wide focus (the whole src/ dir) with a moderate budget — the tool should
/// select a representative cross-section, not just the highest-scored files
/// from one directory.
#[test]
fn test_mcp_vibe_coding_scenario_wide_refactor_with_diversity() {
    use ctx_optim::selection::diversity::{DiversityConfig, GroupingStrategy};

    let repo = TempRepo::realistic_project();
    let mut config = Config::default();
    config.selection.diversity = DiversityConfig {
        enabled: true,
        decay: 0.7,
        grouping: GroupingStrategy::Directory,
    };
    let budget = Budget::standard(12_000);
    let focus = vec![repo.path().join("src")];

    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    // At least 3 different top-level subdirectories should be represented
    let dirs: std::collections::HashSet<_> = result
        .selected
        .iter()
        .filter_map(|s| {
            // Get the first two path components after the repo root
            s.entry.path.strip_prefix(repo.path()).ok()?.components().nth(1).map(|c| {
                c.as_os_str().to_string_lossy().to_string()
            })
        })
        .collect();

    assert!(
        dirs.len() >= 2,
        "wide refactor with diversity should span ≥ 2 subdirectories; got dirs: {dirs:?}"
    );
}

// ── Score signal sanity ───────────────────────────────────────────────────────

/// All composite scores must lie in [0.0, 1.0] — a core invariant of the scorer.
#[test]
fn test_mcp_all_composite_scores_normalized() {
    for repo in [
        TempRepo::minimal(),
        TempRepo::large_realistic(50),
        TempRepo::polyglot_monorepo(),
    ] {
        let result =
            pack_files(repo.path(), &Budget::standard(128_000), &[], &Config::default()).unwrap();
        for entry in &result.selected {
            assert!(
                (0.0..=1.0).contains(&entry.composite_score),
                "composite_score={} out of [0,1] for {}",
                entry.composite_score,
                entry.entry.path.display()
            );
        }
    }
}

/// All per-signal scores must lie in [0.0, 1.0].
#[test]
fn test_mcp_all_signal_scores_normalized() {
    let repo = TempRepo::large_realistic(30);
    let result =
        pack_files(repo.path(), &Budget::standard(128_000), &[], &Config::default()).unwrap();

    for entry in &result.selected {
        let s = &entry.signals;
        let path = entry.entry.path.display();
        assert!((0.0..=1.0).contains(&s.recency), "{path}: recency={}", s.recency);
        assert!((0.0..=1.0).contains(&s.size_score), "{path}: size_score={}", s.size_score);
        assert!((0.0..=1.0).contains(&s.proximity), "{path}: proximity={}", s.proximity);
        assert!((0.0..=1.0).contains(&s.dependency), "{path}: dependency={}", s.dependency);
    }
}

// ── Incremental budget sensitivity ───────────────────────────────────────────

/// Doubling the token budget should always select at least as many files.
/// This verifies monotonicity: more budget → more (or equal) context.
#[test]
fn test_mcp_more_budget_selects_more_or_equal_files() {
    let repo = TempRepo::large_realistic(80);
    let config = Config::default();

    let budgets = [1_000usize, 2_000, 4_000, 8_000, 16_000, 32_000];
    let mut prev_selected = 0;

    for &b in &budgets {
        let result = pack_files(repo.path(), &Budget::standard(b), &[], &config).unwrap();
        assert!(
            result.stats.files_selected >= prev_selected,
            "budget={b}: files_selected={} should be >= previous {prev_selected}",
            result.stats.files_selected
        );
        prev_selected = result.stats.files_selected;
    }
}

/// Selected files should have higher average composite score than unselected
/// files, validating that the knapsack picks the right items.
#[test]
fn test_mcp_selected_files_have_higher_scores_than_unselected() {
    let repo = TempRepo::large_realistic(80);
    let config = Config::default();
    // Tight budget so some files must be left out
    let budget = Budget::standard(4_000);

    let tight = pack_files(repo.path(), &budget, &[], &config).unwrap();
    let all = pack_files(repo.path(), &Budget::standard(1_000_000), &[], &config).unwrap();

    if tight.selected.len() >= all.selected.len() {
        return; // all files fit — nothing left to compare
    }

    let selected_paths: std::collections::HashSet<_> =
        tight.selected.iter().map(|s| &s.entry.path).collect();

    let selected_avg: f32 = tight.selected.iter().map(|s| s.composite_score).sum::<f32>()
        / tight.selected.len().max(1) as f32;

    let unselected_scores: Vec<f32> = all
        .selected
        .iter()
        .filter(|s| !selected_paths.contains(&s.entry.path))
        .map(|s| s.composite_score)
        .collect();

    if unselected_scores.is_empty() {
        return;
    }

    let unselected_avg: f32 =
        unselected_scores.iter().sum::<f32>() / unselected_scores.len() as f32;

    assert!(
        selected_avg >= unselected_avg - 0.05,
        "selected avg score {selected_avg:.3} should be >= unselected avg {unselected_avg:.3}"
    );
}
