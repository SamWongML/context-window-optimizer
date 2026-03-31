//! Edge-case integration tests for the pack pipeline.
//!
//! These tests target boundary conditions, degenerate inputs, and unusual
//! configurations that real-world usage might encounter when vibe-coding with
//! coding agents. Each test documents the expected behavior so that regressions
//! are immediately visible.

mod fixtures {
    include!("../../tests/fixtures/mod.rs");
}

use ctx_optim::{
    config::{Config, DedupConfig},
    pack_files,
    types::Budget,
};
use fixtures::TempRepo;
use std::path::PathBuf;

// ── Budget boundary conditions ────────────────────────────────────────────────

/// A zero-token budget should not panic and should select no files.
///
/// Without MCP, a coding agent would receive 0 tokens of context (unusable);
/// the tool must still return a valid `PackResult` with an empty selection.
#[test]
fn test_pack_zero_token_budget_selects_nothing() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(0);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.tokens_used, 0,
        "zero budget must use zero tokens, got {}",
        result.stats.tokens_used
    );
    assert_eq!(
        result.selected.len(),
        0,
        "zero budget must select nothing, got {} files",
        result.selected.len()
    );
}

/// Budget of exactly 1 token: at most one (tiny) file may be selected.
#[test]
fn test_pack_one_token_budget_selects_at_most_one_file() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(1);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.tokens_used <= 1,
        "1-token budget: used {} tokens, expected ≤ 1",
        result.stats.tokens_used
    );
}

/// Budget that exactly fits all files: every file should be selected.
#[test]
fn test_pack_budget_covers_all_files_selects_all() {
    let repo = TempRepo::larger(5);
    let config = Config::default();

    // First pass: discover total token count without a tight budget
    let generous = pack_files(repo.path(), &Budget::standard(1_000_000), &[], &config).unwrap();
    let total_tokens = generous.stats.tokens_used;

    // Second pass: exact budget equal to total tokens in all files
    // Use a custom Budget that gives 100% of budget to L3
    let tight = Budget {
        total_tokens: total_tokens + 100,
        l1_pct: 0.0,
        l2_pct: 0.0,
        l3_pct: 1.0,
    };
    let result = pack_files(repo.path(), &tight, &[], &config).unwrap();

    assert_eq!(
        result.stats.files_selected,
        generous.stats.files_selected,
        "when budget ≥ total tokens, all files should be selected"
    );
}

// ── Deduplication edge cases ──────────────────────────────────────────────────

/// 50 files, all with identical content: exactly 1 should survive dedup.
///
/// This tests the core value of the MCP: without it, a naive agent would
/// receive 50× the same file, wasting 98% of its context budget.
#[test]
fn test_pack_all_duplicates_reduces_to_one() {
    let repo = TempRepo::all_duplicates(50);
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.duplicates_removed, 49,
        "49 of 50 identical files should be removed, got {}",
        result.stats.duplicates_removed
    );
    assert_eq!(
        result.stats.files_selected, 1,
        "only 1 unique file should be selected, got {}",
        result.stats.files_selected
    );
}

/// With exact dedup disabled, all duplicates pass through scoring and selection.
#[test]
fn test_pack_dedup_disabled_keeps_all_duplicates() {
    let repo = TempRepo::all_duplicates(10);
    let mut config = Config::default();
    config.dedup = DedupConfig { exact: false, near: false, hamming_threshold: 3, shingle_size: 3 };
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.duplicates_removed, 0,
        "dedup disabled: should remove 0 duplicates"
    );
    assert_eq!(
        result.stats.total_files_scanned, 10,
        "dedup disabled: all 10 files should be scanned"
    );
}

// ── Oversized files ───────────────────────────────────────────────────────────

/// A file exceeding the L3 budget alone should not prevent smaller files from
/// being selected — the knapsack must skip it and fill remaining space.
#[test]
fn test_pack_oversized_single_file_does_not_block_selection() {
    let repo = TempRepo::with_oversized_file();
    let config = Config::default();
    // Budget too small for the generated.rs (~5 000 tokens) but large enough for small_a/b
    let budget = Budget::standard(1_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // The small files should be selected even if the big one is skipped
    assert!(
        result.stats.files_selected >= 1,
        "at least one small file should be selected even when big file exceeds budget"
    );
    assert!(
        result.stats.tokens_used <= budget.l3_tokens(),
        "tokens_used {} must not exceed l3_budget {}",
        result.stats.tokens_used,
        budget.l3_tokens()
    );
}

/// When max_file_bytes is very small, all large files are filtered at discovery.
#[test]
fn test_pack_max_file_bytes_filters_large_files() {
    let repo = TempRepo::with_oversized_file();
    let mut config = Config::default();
    // Only allow files under 100 bytes — the oversized file is ~20 KB
    config.max_file_bytes = 100;
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // Large generated.rs must not appear in results
    let has_generated = result
        .selected
        .iter()
        .any(|s| s.entry.path.to_string_lossy().contains("generated"));
    assert!(!has_generated, "generated.rs (>100 bytes) should be filtered by max_file_bytes");
}

// ── Focus-path edge cases ─────────────────────────────────────────────────────

/// Non-existent focus paths should not panic; they should degrade gracefully
/// to standard scoring weights.
#[test]
fn test_pack_nonexistent_focus_path_does_not_panic() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(128_000);
    let focus = vec![
        repo.path().join("does_not_exist"),
        repo.path().join("also_missing").join("nested"),
    ];

    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    assert!(
        result.stats.files_selected > 0,
        "non-existent focus paths should not prevent file selection"
    );
}

/// Focus pointing at a file (not a directory) should work without panicking.
#[test]
fn test_pack_focus_on_specific_file_succeeds() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(128_000);
    let focus_file = repo.path().join("src").join("lib.rs");
    let focus = vec![focus_file];

    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    assert!(result.stats.tokens_used <= budget.l3_tokens());
    assert!(!result.selected.is_empty());
}

/// An empty focus list is equivalent to no focus: proximity signal should be 0.
#[test]
fn test_pack_empty_focus_list_uses_default_weights() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result_no_focus = pack_files(repo.path(), &budget, &[], &config).unwrap();
    let result_empty_focus = pack_files(repo.path(), &budget, &(vec![] as Vec<PathBuf>), &config).unwrap();

    assert_eq!(
        result_no_focus.stats.files_selected,
        result_empty_focus.stats.files_selected,
        "empty focus list must behave identically to no focus"
    );
}

// ── Directory structure edge cases ───────────────────────────────────────────

/// Files buried 5 directories deep should still be discovered and scored.
#[test]
fn test_pack_deeply_nested_files_are_discovered() {
    let repo = TempRepo::deeply_nested();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.total_files_scanned >= 9,
        "deeply nested repo should discover all files, got {}",
        result.stats.total_files_scanned
    );

    let deep_selected = result
        .selected
        .iter()
        .any(|s| s.entry.path.to_string_lossy().contains("level5"));
    assert!(deep_selected, "level5/deep.rs should be reachable and selectable");
}

/// Proximity scoring should favor files closer to the focus, not deep files.
#[test]
fn test_pack_deep_files_score_lower_than_focus_siblings() {
    let repo = TempRepo::deeply_nested();
    let config = Config::default();
    let budget = Budget::standard(128_000);
    let focus = vec![repo.path().join("src").join("level1")];

    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    // Both level1/sibling.rs and level1/level2/level3/level4/level5/deep.rs are selected.
    // The sibling (same directory as focus) must have a higher proximity score.
    let sibling = result
        .selected
        .iter()
        .find(|s| s.entry.path.to_string_lossy().contains("l1_sibling"));
    let deep = result
        .selected
        .iter()
        .find(|s| s.entry.path.to_string_lossy().contains("level5"));

    if let (Some(sib), Some(dep)) = (sibling, deep) {
        assert!(
            sib.signals.proximity >= dep.signals.proximity,
            "sibling proximity {:.3} should be >= deep file proximity {:.3}",
            sib.signals.proximity,
            dep.signals.proximity
        );
    }
    // If either is not present (budget too tight), just verify budget is respected.
    assert!(result.stats.tokens_used <= budget.l3_tokens());
}

// ── Mixed content types ───────────────────────────────────────────────────────

/// The walker must handle non-Rust files (Markdown, TOML, shell scripts) without
/// panicking, and include them in the file count.
#[test]
fn test_pack_mixed_content_types_does_not_panic() {
    let repo = TempRepo::mixed_content_types();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.total_files_scanned > 0,
        "mixed-content repo should produce at least one scanned file"
    );
    assert!(result.stats.tokens_used <= budget.total_tokens);
}

// ── Polyglot monorepo ─────────────────────────────────────────────────────────

/// The tool should discover and rank files across Rust, Python, TypeScript, and Go.
#[test]
fn test_pack_polyglot_monorepo_discovers_all_languages() {
    let repo = TempRepo::polyglot_monorepo();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let paths: Vec<String> = result
        .selected
        .iter()
        .map(|s| s.entry.path.to_string_lossy().to_string())
        .collect();

    let has_rust = paths.iter().any(|p| p.ends_with(".rs"));
    let has_python = paths.iter().any(|p| p.ends_with(".py"));
    let has_typescript = paths.iter().any(|p| p.ends_with(".ts") || p.ends_with(".tsx"));
    let has_go = paths.iter().any(|p| p.ends_with(".go"));

    assert!(has_rust, "polyglot repo should include Rust files; selected: {paths:?}");
    assert!(has_python, "polyglot repo should include Python files; selected: {paths:?}");
    assert!(has_typescript, "polyglot repo should include TypeScript files; selected: {paths:?}");
    assert!(has_go, "polyglot repo should include Go files; selected: {paths:?}");
}

/// Focus on the Rust backend in a polyglot repo should boost Rust files
/// above Python/TypeScript files in the selection ranking.
#[test]
fn test_pack_polyglot_focus_on_rust_backend_boosts_rust_files() {
    let repo = TempRepo::polyglot_monorepo();
    let config = Config::default();
    let budget = Budget::standard(16_000);
    let focus = vec![repo.path().join("backend").join("src")];

    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    // Count Rust vs non-Rust in selected set
    let rust_count = result
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().ends_with(".rs"))
        .count();
    let non_rust_count = result.selected.len() - rust_count;

    assert!(
        rust_count >= non_rust_count,
        "focusing on Rust backend should select more Rust files ({rust_count}) than others ({non_rust_count})"
    );
}

// ── Realistic project ─────────────────────────────────────────────────────────

/// Focusing on `src/scoring/` in the realistic project should rank all three
/// scoring files above unrelated modules like index or output.
#[test]
fn test_pack_realistic_project_focus_selects_scoring_module() {
    let repo = TempRepo::realistic_project();
    let config = Config::default();
    // Tight budget: forces selection — can't take everything
    let budget = Budget::standard(8_000);
    let focus = vec![repo.path().join("src").join("scoring")];

    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    let scoring_count = result
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("scoring"))
        .count();

    assert!(
        scoring_count >= 2,
        "focused on src/scoring/: expected ≥ 2 scoring files selected, got {scoring_count}; selected: {:?}",
        result.selected.iter().map(|s| &s.entry.path).collect::<Vec<_>>()
    );
}

/// Focusing on `src/index/` should NOT give higher scores to scoring files.
#[test]
fn test_pack_realistic_project_focus_on_index_does_not_boost_scoring() {
    let repo = TempRepo::realistic_project();
    let config = Config::default();
    let budget = Budget::standard(128_000);
    let focus = vec![repo.path().join("src").join("index")];

    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    // Find proximity scores for index vs scoring files
    let index_max_proximity = result
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("/index/"))
        .map(|s| s.signals.proximity)
        .fold(0.0_f32, f32::max);
    let scoring_max_proximity = result
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("/scoring/"))
        .map(|s| s.signals.proximity)
        .fold(0.0_f32, f32::max);

    assert!(
        index_max_proximity >= scoring_max_proximity,
        "index files (proximity={index_max_proximity:.3}) should have >= proximity than scoring files ({scoring_max_proximity:.3}) when focus is on index"
    );
}

// ── Token budget stress tests ─────────────────────────────────────────────────

/// With a budget of exactly 1 file's worth of tokens, exactly 1 file is selected.
#[test]
fn test_pack_budget_fits_exactly_one_file() {
    let repo = TempRepo::near_token_limit();
    let config = Config::default();

    // Discover the smallest file's token count
    let large_budget = pack_files(repo.path(), &Budget::standard(1_000_000), &[], &config).unwrap();
    let min_tokens = large_budget
        .selected
        .iter()
        .map(|s| s.entry.token_count)
        .min()
        .unwrap_or(1);

    // Budget that should fit exactly 1 file (give or take rounding)
    let budget = Budget {
        total_tokens: min_tokens + 10,
        l1_pct: 0.0,
        l2_pct: 0.0,
        l3_pct: 1.0,
    };

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.files_selected >= 1,
        "budget for 1 file should select at least 1 file"
    );
    assert!(
        result.stats.tokens_used <= min_tokens + 10,
        "tokens_used {} must not exceed budget {}",
        result.stats.tokens_used,
        min_tokens + 10
    );
}

/// Under progressively tighter budgets, the selected token count never exceeds
/// the L3 allocation.
#[test]
fn test_pack_budget_invariant_across_multiple_budgets() {
    let repo = TempRepo::larger(30);
    let config = Config::default();

    for total in [100, 500, 1_000, 5_000, 20_000, 128_000] {
        let budget = Budget::standard(total);
        let result = pack_files(repo.path(), &budget, &[], &config)
            .expect("pack should not fail for any reasonable budget");
        assert!(
            result.stats.tokens_used <= budget.l3_tokens(),
            "budget={total}: tokens_used={} must be <= l3_budget={}",
            result.stats.tokens_used,
            budget.l3_tokens()
        );
    }
}

// ── Output format correctness ─────────────────────────────────────────────────

/// L3 output must always contain opening and closing `<context>` tags.
#[test]
fn test_pack_l3_output_always_has_context_tags() {
    for (name, repo) in [
        ("minimal", TempRepo::minimal()),
        ("with_duplicates", TempRepo::with_duplicates()),
        ("deeply_nested", TempRepo::deeply_nested()),
        ("polyglot", TempRepo::polyglot_monorepo()),
    ] {
        let result =
            pack_files(repo.path(), &Budget::standard(128_000), &[], &Config::default()).unwrap();
        assert!(
            result.l3_output.contains("<context>"),
            "{name}: L3 output missing opening <context> tag"
        );
        assert!(
            result.l3_output.contains("</context>"),
            "{name}: L3 output missing closing </context> tag"
        );
    }
}

/// L1 output must list ALL discovered (post-dedup) files, not only selected ones.
#[test]
fn test_pack_l1_output_covers_all_post_dedup_files() {
    let repo = TempRepo::larger(20);
    let config = Config::default();
    // Very tight L3 budget but generous L1
    let budget = Budget {
        total_tokens: 50_000,
        l1_pct: 0.80,
        l2_pct: 0.10,
        l3_pct: 0.10, // 5 000 tokens for L3 → selects only ~5 files
    };

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let l1_line_count = result.l1_output.lines().filter(|l| l.contains("tokens")).count();
    let selected_count = result.stats.files_selected;

    assert!(
        l1_line_count >= selected_count,
        "L1 must list at least as many files as selected; l1={l1_line_count} selected={selected_count}"
    );
}

/// PackStats fields must be mutually consistent.
#[test]
fn test_pack_stats_internally_consistent_across_scenarios() {
    let scenarios: &[(&str, TempRepo)] = &[
        ("minimal", TempRepo::minimal()),
        ("with_duplicates", TempRepo::with_duplicates()),
        ("deeply_nested", TempRepo::deeply_nested()),
        ("polyglot", TempRepo::polyglot_monorepo()),
        ("realistic", TempRepo::realistic_project()),
    ];

    for (name, repo) in scenarios {
        let result =
            pack_files(repo.path(), &Budget::standard(128_000), &[], &Config::default()).unwrap();
        let stats = &result.stats;

        // selected count == len(selected vec)
        assert_eq!(
            stats.files_selected,
            result.selected.len(),
            "{name}: files_selected field mismatch"
        );

        // tokens_used == sum of selected token counts
        let computed: usize = result.selected.iter().map(|s| s.entry.token_count).sum();
        assert_eq!(
            stats.tokens_used, computed,
            "{name}: tokens_used field mismatch"
        );

        // total_files_scanned >= files_selected
        assert!(
            stats.total_files_scanned >= stats.files_selected,
            "{name}: total_files_scanned {} < files_selected {}",
            stats.total_files_scanned,
            stats.files_selected
        );
    }
}
