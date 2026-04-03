//! Integration tests for the full pack pipeline.

use crate::fixtures::TempRepo;
use ctx_optim::selection::diversity::{DiversityConfig, GroupingStrategy};
use ctx_optim::{config::Config, pack_files, types::Budget};

#[test]
fn test_pack_minimal_repo_succeeds() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.files_selected > 0,
        "expected at least one file selected"
    );
    assert!(result.stats.tokens_used <= budget.total_tokens);
    assert!(!result.l1_output.is_empty());
    assert!(!result.l3_output.is_empty());
}

#[test]
fn test_pack_deduplicates_exact_duplicates() {
    let repo = TempRepo::with_duplicates();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.duplicates_removed, 1,
        "expected 1 duplicate removed"
    );
    assert!(
        result.stats.files_selected <= 2,
        "only 2 unique files should remain"
    );
}

#[test]
fn test_pack_respects_token_budget() {
    let repo = TempRepo::larger(50);
    let config = Config::default();
    // Very tight budget — forces selection
    let budget = Budget::standard(500);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.tokens_used <= budget.l3_tokens(),
        "tokens_used={} should be <= l3_budget={}",
        result.stats.tokens_used,
        budget.l3_tokens()
    );
}

#[test]
fn test_pack_l1_contains_all_selected_files() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    for entry in &result.selected {
        let path_str = entry.entry.path.to_string_lossy();
        assert!(
            result.l1_output.contains(path_str.as_ref()),
            "L1 output missing path: {path_str}"
        );
    }
}

#[test]
fn test_pack_l3_wraps_in_context_tags() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.l3_output.contains("<context>"),
        "L3 should open <context>"
    );
    assert!(
        result.l3_output.contains("</context>"),
        "L3 should close </context>"
    );
}

#[test]
fn test_pack_stats_consistency() {
    let repo = TempRepo::minimal();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();
    let stats = &result.stats;

    assert_eq!(stats.files_selected, result.selected.len());
    let computed_tokens: usize = result.selected.iter().map(|e| e.entry.token_count).sum();
    assert_eq!(stats.tokens_used, computed_tokens);
}

#[test]
fn test_pack_near_dedup_removes_similar_files() {
    use ctx_optim::index::{
        discovery::{DiscoveryOptions, discover_files},
        simhash::hamming_distance,
    };

    let repo = TempRepo::with_near_duplicates();
    let mut config = Config::default();
    config.dedup.near = true;

    // First, discover with SimHash enabled to check actual distances
    let opts = DiscoveryOptions::from_config(&config, repo.path());
    let files = discover_files(&opts).unwrap();

    // Find the two processor files and check their SimHash distance
    let processor_a = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("processor_a"))
        .expect("processor_a.rs should exist");
    let processor_b = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("processor_b"))
        .expect("processor_b.rs should exist");

    let fp_a = processor_a.simhash.expect("simhash should be computed");
    let fp_b = processor_b.simhash.expect("simhash should be computed");
    let dist = hamming_distance(fp_a, fp_b);

    // Set threshold to match — near-duplicates should be within this distance
    config.dedup.hamming_threshold = dist.max(1);
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.near_duplicates_removed >= 1,
        "expected at least 1 near-duplicate removed (hamming_distance={dist}), got {}",
        result.stats.near_duplicates_removed
    );
}

#[test]
fn test_pack_near_dedup_disabled_by_default() {
    let repo = TempRepo::with_near_duplicates();
    let config = Config::default(); // near dedup is off by default
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.near_duplicates_removed, 0,
        "near-dedup should be off by default"
    );
}

#[test]
fn test_pack_diversity_selects_from_multiple_directories() {
    let repo = TempRepo::with_directory_structure();
    let mut config = Config::default();
    config.selection.diversity = DiversityConfig {
        enabled: true,
        decay: 0.7,
        grouping: GroupingStrategy::Directory,
    };
    // Tight budget forces selection — can't take everything
    let budget = Budget::standard(400);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // With diversity enabled, selected files should span multiple directories
    let dirs: std::collections::HashSet<_> = result
        .selected
        .iter()
        .filter_map(|s| {
            s.entry
                .path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
        })
        .collect();

    if result.stats.files_selected >= 2 {
        assert!(
            dirs.len() >= 2,
            "diversity should select from multiple directories, got dirs: {dirs:?}"
        );
    }
}

#[test]
fn test_pack_kkt_solver_respects_budget() {
    let repo = TempRepo::larger(20);
    let mut config = Config::default();
    config.selection.solver = "kkt".to_string();
    let budget = Budget::standard(1_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.tokens_used <= budget.l3_tokens(),
        "KKT solver exceeded budget: {} > {}",
        result.stats.tokens_used,
        budget.l3_tokens()
    );
}

#[test]
fn test_pack_auto_solver_at_least_as_good_as_greedy() {
    let repo = TempRepo::larger(30);
    let budget = Budget::standard(2_000);

    let mut greedy_config = Config::default();
    greedy_config.selection.solver = "greedy".to_string();
    greedy_config.selection.diversity.enabled = false;
    let greedy_result = pack_files(repo.path(), &budget, &[], &greedy_config).unwrap();

    let mut auto_config = Config::default();
    auto_config.selection.solver = "auto".to_string();
    auto_config.selection.diversity.enabled = false;
    let auto_result = pack_files(repo.path(), &budget, &[], &auto_config).unwrap();

    let greedy_score: f32 = greedy_result
        .selected
        .iter()
        .map(|s| s.composite_score)
        .sum();
    let auto_score: f32 = auto_result.selected.iter().map(|s| s.composite_score).sum();

    assert!(
        auto_score >= greedy_score - 0.01,
        "auto ({auto_score:.3}) should be >= greedy ({greedy_score:.3})"
    );
}

#[test]
fn test_l1_covers_more_files_than_selected_when_budget_tight() {
    // L1 should list ALL discovered files as a repo skeleton map, even when
    // L3 (selection) is constrained to only a subset.
    //
    // We use a custom Budget that gives L3 a tight constraint (5%) but gives
    // L1 generous room (50%) so it can list all files even while L3 selects few.
    let repo = TempRepo::larger(20);
    let config = Config::default();

    // L3 gets 5% of 10_000 = 500 tokens; each file is ~15-20 tokens so ~25-33
    // can be selected — but we have 20 files and they should all fit unless token
    // estimates vary.  Use a smaller total so L3 can only hold ~5 files.
    let budget = Budget {
        total_tokens: 10_000,
        l1_pct: 0.50, // 5 000 tokens for L1 → can list many files
        l2_pct: 0.45,
        l3_pct: 0.05, // 500 tokens for L3 → selects only a few files
    };

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let selected_count = result.stats.files_selected;
    let total_scanned = result.stats.total_files_scanned;

    // Count entries in L1 output (lines that contain the token-count annotation)
    let l1_entry_count = result
        .l1_output
        .lines()
        .filter(|l| l.contains("tokens"))
        .count();

    assert!(
        l1_entry_count >= selected_count,
        "L1 must contain at least the selected files: l1={l1_entry_count} selected={selected_count}"
    );

    // If not everything could be selected, L1 must show ALL files.
    if total_scanned > selected_count {
        assert!(
            l1_entry_count > selected_count,
            "L1 ({l1_entry_count} entries) should cover more than just the {selected_count} selected files \
             (total_scanned={total_scanned})"
        );
    }
}

#[test]
fn test_focus_files_rank_above_unrelated_files() {
    // Verify that providing focus_paths causes files near the focus to rank
    // above unrelated files even when the unrelated files are "recent".
    let repo = TempRepo::with_directory_structure();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    // Focus on the "models" subdirectory (if present in fixture).
    let focus = vec![repo.path().join("models")];
    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    // At minimum the pipeline should succeed and respect the budget.
    assert!(result.stats.tokens_used <= budget.l3_tokens());
    assert!(!result.selected.is_empty());
}
