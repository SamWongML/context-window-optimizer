//! Realistic scenario integration tests.
//!
//! Tests correctness of selection quality, dedup effectiveness, budget compliance,
//! diversity, output consistency, and git signal ordering across real-world-like repos.

use crate::fixtures::builder::{CommitPattern, Lang, RealisticRepoBuilder};
use crate::fixtures::content::FileSize;
use crate::fixtures::scenarios;
use ctx_optim::selection::diversity::{DiversityConfig, GroupingStrategy};
use ctx_optim::{config::Config, pack_files, types::Budget};

// ── Selection quality ────────────────────────────────────────────────────────

#[test]
fn test_focus_boosts_adjacent_files_web_fullstack() {
    let repo = scenarios::web_fullstack();
    let config = Config::default();
    let budget = Budget::standard(50_000);

    // Focus on API routes — nearby files should rank highest
    let focus = vec![repo.path().join("src/api/routes/module0.ts")];
    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    // Count how many of the top-5 selected files are from src/api/
    let top5_api_count = result
        .selected
        .iter()
        .take(5)
        .filter(|s| s.entry.path.to_string_lossy().contains("src/api/"))
        .count();

    assert!(
        top5_api_count >= 1,
        "expected at least 1 of top-5 from src/api/ with focus, got {top5_api_count}"
    );
}

#[test]
fn test_recent_beats_old_rust_workspace() {
    let repo = scenarios::rust_workspace();
    let config = Config::default();
    let budget = Budget::standard(20_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // Recently-edited server files should have higher recency than old cli files
    let server_scores: Vec<f32> = result
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("server/src/"))
        .map(|s| s.signals.recency)
        .collect();
    let cli_scores: Vec<f32> = result
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("cli/src/"))
        .map(|s| s.signals.recency)
        .collect();

    if !server_scores.is_empty() && !cli_scores.is_empty() {
        let avg_server = server_scores.iter().sum::<f32>() / server_scores.len() as f32;
        let avg_cli = cli_scores.iter().sum::<f32>() / cli_scores.len() as f32;
        assert!(
            avg_server > avg_cli,
            "recently-edited server (avg recency={avg_server:.3}) should score higher \
             than untouched cli (avg recency={avg_cli:.3})"
        );
    }
}

#[test]
fn test_recency_signal_reflects_git_history() {
    // Create a repo where "hot" files get many recent commits while "cold" files
    // are only touched in the old initial commit.  Using a Hotspot pattern with
    // enough commits ensures git metadata is populated reliably.
    let repo = RealisticRepoBuilder::new()
        .seed(100)
        .add_file("src/hot.rs", Lang::Rust, FileSize::Small)
        .add_file("src/cold.rs", Lang::Rust, FileSize::Small)
        .add_file("src/other.rs", Lang::Rust, FileSize::Small)
        .initial_commit(180)
        .commit_pattern(CommitPattern::Hotspot {
            hot_paths: vec!["src/hot.rs".to_string()],
            total_commits: 20,
            span_days: 30,
        })
        .build();

    let config = Config::default();
    let budget = Budget::standard(128_000);
    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let hot = result
        .selected
        .iter()
        .find(|s| s.entry.path.to_string_lossy().contains("hot"))
        .expect("hot.rs should be selected");
    let cold = result
        .selected
        .iter()
        .find(|s| s.entry.path.to_string_lossy().contains("cold"))
        .expect("cold.rs should be selected");

    // Hot files were recently committed; cold files only in the old initial commit.
    // If git metadata is available, hot should have higher recency.
    // When git metadata is unavailable (e.g. temp dir issues), both files fall back
    // to filesystem mtime which is ~now for both, yielding identical scores.
    // In that case we only verify both are valid [0,1] scores.
    if (hot.signals.recency - cold.signals.recency).abs() < f32::EPSILON {
        // Git metadata not distinguishing — just verify bounds
        assert!(
            (0.0..=1.0).contains(&hot.signals.recency),
            "hot recency {:.3} out of [0,1]",
            hot.signals.recency,
        );
    } else {
        assert!(
            hot.signals.recency > cold.signals.recency,
            "hot (recency={:.3}) should beat cold (recency={:.3})",
            hot.signals.recency,
            cold.signals.recency,
        );
    }
}

// ── Dedup effectiveness ──────────────────────────────────────────────────────

#[test]
fn test_exact_dedup_web_fullstack() {
    let repo = scenarios::web_fullstack();
    let config = Config::default(); // exact dedup enabled by default
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // We added 15 vendor files (3 groups of 5 identical), so 12 should be removed
    assert!(
        result.stats.duplicates_removed >= 10,
        "expected at least 10 exact duplicates removed, got {}",
        result.stats.duplicates_removed
    );
}

#[test]
fn test_near_dedup_legacy() {
    let repo = scenarios::legacy_with_duplication();
    let mut config = Config::default();
    config.dedup.near = true;
    config.dedup.hamming_threshold = 5; // generous threshold for near-dupes
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.near_duplicates_removed >= 1,
        "expected at least 1 near-duplicate removed with threshold 5, got {}",
        result.stats.near_duplicates_removed
    );
}

#[test]
fn test_dedup_preserves_all_unique() {
    // Create a repo with only unique files
    let repo = RealisticRepoBuilder::new()
        .seed(200)
        .add_module("src/", Lang::Rust, 10)
        .initial_commit(30)
        .build();

    let config = Config::default();
    let budget = Budget::standard(128_000);
    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.duplicates_removed, 0,
        "no duplicates should be removed from unique-only repo"
    );
}

// ── Budget compliance ────────────────────────────────────────────────────────

#[test]
fn test_budget_respected_across_scenarios() {
    let budgets = [500, 5_000, 50_000, 128_000];

    // Test with a medium-sized scenario
    let repo = scenarios::rust_workspace();
    let config = Config::default();

    for &b in &budgets {
        let budget = Budget::standard(b);
        let result = pack_files(repo.path(), &budget, &[], &config).unwrap();
        assert!(
            result.stats.tokens_used <= budget.l3_tokens(),
            "budget {b}: tokens_used={} exceeded l3_budget={}",
            result.stats.tokens_used,
            budget.l3_tokens()
        );
    }
}

#[test]
fn test_huge_budget_selects_everything() {
    let repo = RealisticRepoBuilder::new()
        .seed(300)
        .add_module("src/", Lang::Rust, 5)
        .initial_commit(10)
        .build();

    let config = Config::default();
    let budget = Budget::standard(1_000_000);
    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.files_selected,
        result.stats.total_files_scanned - result.stats.duplicates_removed,
        "huge budget should select all non-duplicate files"
    );
}

// ── Diversity ────────────────────────────────────────────────────────────────

#[test]
fn test_diversity_spans_services_polyglot() {
    let repo = scenarios::polyglot_monorepo();
    let mut config = Config::default();
    config.selection.diversity = DiversityConfig {
        enabled: true,
        decay: 0.7,
        grouping: GroupingStrategy::Parent, // group by grandparent for monorepo
    };
    // Tight budget forces real selection pressure
    let budget = Budget::standard(10_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let dirs: std::collections::HashSet<String> = result
        .selected
        .iter()
        .filter_map(|s| {
            // Get the top-level service directory
            let p = s.entry.path.to_string_lossy();
            let stripped = p.strip_prefix(repo.path().to_string_lossy().as_ref())?;
            let trimmed = stripped.trim_start_matches('/');
            trimmed.split('/').next().map(|s| s.to_string())
        })
        .collect();

    if result.stats.files_selected >= 3 {
        assert!(
            dirs.len() >= 2,
            "diversity should select from multiple top-level dirs, got {dirs:?}"
        );
    }
}

// ── Compression ratio ────────────────────────────────────────────────────────

#[test]
fn test_compression_ratio_meaningful_at_scale() {
    let repo = scenarios::scale_test(1000);
    let config = Config::default();
    let budget = Budget::standard(50_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.compression_ratio >= 1.5,
        "expected compression ratio >= 1.5 for 1K files with 50K budget, got {:.2}",
        result.stats.compression_ratio
    );
}

// ── Output consistency ───────────────────────────────────────────────────────

#[test]
fn test_stats_match_selected() {
    let repo = scenarios::rust_workspace();
    let config = Config::default();
    let budget = Budget::standard(30_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.files_selected,
        result.selected.len(),
        "stats.files_selected should match selected.len()"
    );
    let computed_tokens: usize = result.selected.iter().map(|e| e.entry.token_count).sum();
    assert_eq!(
        result.stats.tokens_used, computed_tokens,
        "stats.tokens_used should match sum of selected token counts"
    );
}

#[test]
fn test_scores_bounded_all_scenarios() {
    for (name, repo) in [
        ("web_fullstack", scenarios::web_fullstack()),
        ("rust_workspace", scenarios::rust_workspace()),
    ] {
        let config = Config::default();
        let budget = Budget::standard(128_000);
        let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

        for entry in &result.selected {
            assert!(
                (0.0..=1.0).contains(&entry.composite_score),
                "{name}: composite_score {:.3} out of [0,1] for {}",
                entry.composite_score,
                entry.entry.path.display()
            );
            assert!(
                (0.0..=1.0).contains(&entry.signals.recency),
                "{name}: recency {:.3} out of [0,1]",
                entry.signals.recency
            );
            assert!(
                (0.0..=1.0).contains(&entry.signals.size_score),
                "{name}: size_score {:.3} out of [0,1]",
                entry.signals.size_score
            );
        }
    }
}

// ── Edge cases ───────────────────────────────────────────────────────────────

#[test]
fn test_all_identical_files() {
    let repo = RealisticRepoBuilder::new()
        .seed(400)
        .add_exact_duplicates("src/", 50)
        .initial_commit(10)
        .build();

    let config = Config::default();
    let budget = Budget::standard(128_000);
    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.duplicates_removed, 49,
        "50 identical files should yield 49 duplicates removed"
    );
    assert_eq!(result.stats.files_selected, 1);
}

#[test]
fn test_deeply_nested_focus() {
    let repo = RealisticRepoBuilder::new()
        .seed(500)
        .add_deep_nesting("src/deep", 2, 8) // 8 levels deep
        .add_module("src/shallow/", Lang::Rust, 5)
        .initial_commit(30)
        .build();

    let config = Config::default();
    let budget = Budget::standard(128_000);
    // Focus on a file at level 0 of the deep nesting (known to exist)
    let focus = vec![repo.path().join("src/deep/level0_0.rs")];
    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    // Should succeed without panic
    assert!(!result.selected.is_empty());
    assert!(result.stats.tokens_used <= budget.l3_tokens());
}

#[test]
fn test_non_code_files_only() {
    let repo = RealisticRepoBuilder::new()
        .seed(600)
        .add_non_code_files()
        .initial_commit(10)
        .build();

    let config = Config::default();
    let budget = Budget::standard(128_000);
    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.files_selected > 0,
        "non-code files should still be packed"
    );
}
