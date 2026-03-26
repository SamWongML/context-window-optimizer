//! Integration tests for the full pack pipeline.

mod fixtures {
    include!("../../tests/fixtures/mod.rs");
}

use ctx_optim::{config::Config, pack_files, types::Budget};
use fixtures::TempRepo;

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
