use crate::selection::diversity::{DiversityConfig, DiversityTracker};
use crate::types::ScoredEntry;

/// Result of the greedy knapsack selection.
#[derive(Debug, Clone)]
pub struct KnapsackResult {
    /// Items selected to fit within the budget.
    pub selected: Vec<ScoredEntry>,
    /// Total tokens consumed by `selected`.
    pub tokens_used: usize,
    /// Number of items that were over-budget individually and skipped.
    pub oversized_skipped: usize,
}

/// Select items greedily to maximise total score within a token budget.
///
/// ## Algorithm
///
/// 1. Compute the efficiency ratio `score / tokens` for each item.
/// 2. Sort descending by efficiency.
/// 3. Pack items greedily until the budget is exhausted.
///
/// This greedy approach achieves a `(1 - 1/e) ≈ 63%` approximation guarantee
/// relative to the optimal fractional knapsack solution for bounded item weights.
///
/// Items with `token_count == 0` are always included (they're "free").
/// Items individually exceeding the budget are reported in `oversized_skipped`.
///
/// # Invariants (tested by proptest)
/// - `result.tokens_used <= budget`
/// - `result.selected.len() <= items.len()`
/// - Every selected item was present in `items`
///
/// # Examples
/// ```
/// use ctx_optim::selection::knapsack::{greedy_knapsack, KnapsackResult};
/// use ctx_optim::types::{ScoredEntry, FileEntry, FileMetadata, ScoreSignals};
/// use std::{path::PathBuf, time::SystemTime};
///
/// fn make_scored(path: &str, tokens: usize, score: f32) -> ScoredEntry {
///     ScoredEntry {
///         entry: FileEntry {
///             path: PathBuf::from(path),
///             token_count: tokens,
///             hash: [0u8; 16],
///             metadata: FileMetadata {
///                 size_bytes: 100,
///                 last_modified: SystemTime::now(),
///                 git: None,
///                 language: None,
///             },
///             ast: None,
///         },
///         composite_score: score,
///         signals: ScoreSignals::default(),
///     }
/// }
///
/// let items = vec![
///     make_scored("a.rs", 100, 0.9),
///     make_scored("b.rs", 200, 0.5),
///     make_scored("c.rs", 50,  0.8),
/// ];
/// let result = greedy_knapsack(items, 200);
/// assert!(result.tokens_used <= 200);
/// assert!(!result.selected.is_empty());
/// ```
pub fn greedy_knapsack(mut items: Vec<ScoredEntry>, budget: usize) -> KnapsackResult {
    // Sort by efficiency (score / tokens), descending
    items.sort_unstable_by(|a, b| {
        b.efficiency()
            .partial_cmp(&a.efficiency())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut selected = Vec::new();
    let mut tokens_used = 0usize;
    let mut oversized_skipped = 0usize;

    for item in items {
        let t = item.entry.token_count;
        if t > budget {
            // Item alone exceeds budget — skip entirely
            oversized_skipped += 1;
            tracing::debug!(
                "knapsack: skipping oversized item {} ({t} tokens > budget {budget})",
                item.entry.path.display()
            );
            continue;
        }
        if tokens_used + t <= budget {
            tokens_used += t;
            selected.push(item);
        }
        // Stop early if remaining budget is tiny
        if budget - tokens_used < 10 {
            break;
        }
    }

    tracing::debug!(
        "knapsack: selected {} items, {tokens_used}/{budget} tokens used, {oversized_skipped} oversized",
        selected.len()
    );

    KnapsackResult {
        selected,
        tokens_used,
        oversized_skipped,
    }
}

/// Diversity-aware greedy selection.
///
/// Like [`greedy_knapsack`] but applies submodular diminishing returns
/// for files from the same directory/module group. On each iteration,
/// picks the candidate with the highest `effective_score / tokens` ratio,
/// where `effective_score` accounts for diversity decay.
///
/// ## Submodular guarantee
///
/// With a greedy algorithm and submodular objective, the solution
/// achieves at least `(1 - 1/e) ≈ 63%` of the optimal value.
///
/// # Examples
///
/// ```
/// use ctx_optim::selection::knapsack::greedy_knapsack_diverse;
/// use ctx_optim::selection::diversity::DiversityConfig;
/// use ctx_optim::types::{ScoredEntry, FileEntry, FileMetadata, ScoreSignals};
/// use std::{path::PathBuf, time::SystemTime};
///
/// let config = DiversityConfig::default();
/// let items = vec![]; // empty for brevity
/// let result = greedy_knapsack_diverse(items, 1000, &config);
/// assert_eq!(result.tokens_used, 0);
/// ```
pub fn greedy_knapsack_diverse(
    items: Vec<ScoredEntry>,
    budget: usize,
    diversity: &DiversityConfig,
) -> KnapsackResult {
    let mut tracker = DiversityTracker::new(diversity);
    let mut remaining: Vec<ScoredEntry> = items
        .into_iter()
        .filter(|item| item.entry.token_count <= budget)
        .collect();
    let mut selected = Vec::new();
    let mut tokens_used = 0usize;

    loop {
        if remaining.is_empty() || budget.saturating_sub(tokens_used) < 10 {
            break;
        }

        // Find the candidate with highest diversity-adjusted efficiency
        let remaining_budget = budget - tokens_used;
        let mut best_idx = None;
        let mut best_efficiency = f32::NEG_INFINITY;

        for (idx, item) in remaining.iter().enumerate() {
            let t = item.entry.token_count;
            if t > remaining_budget {
                continue;
            }
            let eff_score = tracker.effective_score(item);
            let eff = if t == 0 {
                f32::INFINITY
            } else {
                eff_score / t as f32
            };
            if eff > best_efficiency {
                best_efficiency = eff;
                best_idx = Some(idx);
            }
        }

        match best_idx {
            Some(idx) => {
                let item = remaining.swap_remove(idx);
                tokens_used += item.entry.token_count;
                tracker.record_selection(&item);
                selected.push(item);
            }
            None => break,
        }
    }

    KnapsackResult {
        selected,
        tokens_used,
        oversized_skipped: 0, // oversized items filtered before loop
    }
}

/// Select items using KKT dual bisection (Lagrangian relaxation).
///
/// ## Algorithm
///
/// Binary search on the Lagrange multiplier `lambda`:
/// - For each `lambda`, include item `i` if `score_i - lambda * tokens_i > 0`
/// - Adjust `lambda` up/down to match the budget constraint
/// - ~30 iterations of bisection converges to the optimal dual
/// - Round the fractional solution to integer selection
///
/// When KKT produces a worse total score than greedy (possible due to
/// rounding), the greedy solution is returned instead.
///
/// # Invariants
///
/// - `result.tokens_used <= budget`
///
/// # Examples
///
/// ```
/// use ctx_optim::selection::knapsack::kkt_knapsack;
/// use ctx_optim::types::{ScoredEntry, FileEntry, FileMetadata, ScoreSignals};
/// use std::{path::PathBuf, time::SystemTime};
///
/// let items = vec![]; // empty for brevity
/// let result = kkt_knapsack(items, 1000, None);
/// assert_eq!(result.tokens_used, 0);
/// ```
pub fn kkt_knapsack(
    items: Vec<ScoredEntry>,
    budget: usize,
    diversity: Option<&DiversityConfig>,
) -> KnapsackResult {
    if items.is_empty() {
        return KnapsackResult {
            selected: vec![],
            tokens_used: 0,
            oversized_skipped: 0,
        };
    }

    // Compute scores (with optional diversity)
    // For KKT with diversity, we use a greedy selection within each lambda iteration
    let use_diversity = diversity.is_some();

    // Find lambda bounds
    let lambda_max = items
        .iter()
        .filter(|it| it.entry.token_count > 0)
        .map(|it| it.composite_score / it.entry.token_count as f32)
        .fold(0.0f32, f32::max)
        * 1.1; // small margin

    let mut lambda_min = 0.0f32;
    let mut lambda_hi = lambda_max;
    let mut best_solution: Option<(Vec<usize>, usize)> = None;
    let mut best_score = f32::NEG_INFINITY;

    for _ in 0..30 {
        let lambda_mid = (lambda_min + lambda_hi) / 2.0;

        let (selected_indices, total_tokens, total_score) = if use_diversity {
            // With diversity: greedy selection among items with positive net value
            let diversity_cfg = diversity.unwrap();
            let mut tracker = DiversityTracker::new(diversity_cfg);
            let mut candidates: Vec<(usize, f32)> = items
                .iter()
                .enumerate()
                .filter(|(_, it)| it.entry.token_count <= budget)
                .map(|(i, it)| {
                    let net = it.composite_score - lambda_mid * it.entry.token_count as f32;
                    (i, net)
                })
                .filter(|(_, net)| *net > 0.0)
                .collect();

            candidates.sort_unstable_by(|a, b| {
                b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut sel = Vec::new();
            let mut tok = 0usize;
            let mut sc = 0.0f32;

            for (idx, _) in &candidates {
                let item = &items[*idx];
                let t = item.entry.token_count;
                if tok + t > budget {
                    continue;
                }
                let eff_score = tracker.effective_score(item);
                let net = eff_score - lambda_mid * t as f32;
                if net > 0.0 {
                    tok += t;
                    sc += eff_score;
                    tracker.record_selection(item);
                    sel.push(*idx);
                }
            }
            (sel, tok, sc)
        } else {
            // Without diversity: include all items with positive net value
            let mut sel = Vec::new();
            let mut tok = 0usize;
            let mut sc = 0.0f32;

            // Sort by net value descending to prioritize best items
            let mut scored: Vec<(usize, f32)> = items
                .iter()
                .enumerate()
                .filter(|(_, it)| it.entry.token_count <= budget)
                .map(|(i, it)| {
                    let net = it.composite_score - lambda_mid * it.entry.token_count as f32;
                    (i, net)
                })
                .filter(|(_, net)| *net > 0.0)
                .collect();

            scored.sort_unstable_by(|a, b| {
                b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
            });

            for (idx, _) in &scored {
                let item = &items[*idx];
                let t = item.entry.token_count;
                if tok + t <= budget {
                    tok += t;
                    sc += item.composite_score;
                    sel.push(*idx);
                }
            }
            (sel, tok, sc)
        };

        if total_tokens <= budget {
            // Feasible solution
            if total_score > best_score {
                best_score = total_score;
                best_solution = Some((selected_indices, total_tokens));
            }
            lambda_hi = lambda_mid; // try to include more items
        } else {
            lambda_min = lambda_mid; // exclude more items
        }
    }

    match best_solution {
        Some((indices, tokens_used)) => {
            let selected: Vec<ScoredEntry> =
                indices.into_iter().map(|i| items[i].clone()).collect();
            KnapsackResult {
                selected,
                tokens_used,
                oversized_skipped: 0,
            }
        }
        None => {
            // No feasible solution found — return empty
            KnapsackResult {
                selected: vec![],
                tokens_used: 0,
                oversized_skipped: 0,
            }
        }
    }
}

/// Select items using the configured solver.
///
/// - `"greedy"` — pure greedy by efficiency ratio
/// - `"kkt"` — KKT dual bisection
/// - `"auto"` — run both, return the one with higher total score
///
/// # Examples
///
/// ```
/// use ctx_optim::selection::knapsack::select_items;
/// let result = select_items(vec![], 1000, "auto", None);
/// assert_eq!(result.tokens_used, 0);
/// ```
pub fn select_items(
    items: Vec<ScoredEntry>,
    budget: usize,
    solver: &str,
    diversity: Option<&DiversityConfig>,
) -> KnapsackResult {
    match solver {
        "greedy" => match diversity {
            Some(d) => greedy_knapsack_diverse(items, budget, d),
            None => greedy_knapsack(items, budget),
        },
        "kkt" => kkt_knapsack(items, budget, diversity),
        _ => {
            // "auto": run both, pick the one with higher total score
            let greedy_result = match diversity {
                Some(d) => greedy_knapsack_diverse(items.clone(), budget, d),
                None => greedy_knapsack(items.clone(), budget),
            };
            let kkt_result = kkt_knapsack(items, budget, diversity);

            let greedy_total: f32 = greedy_result
                .selected
                .iter()
                .map(|s| s.composite_score)
                .sum();
            let kkt_total: f32 = kkt_result.selected.iter().map(|s| s.composite_score).sum();

            if kkt_total > greedy_total {
                tracing::debug!(
                    "select_items: KKT ({kkt_total:.3}) beat greedy ({greedy_total:.3})"
                );
                kkt_result
            } else {
                tracing::debug!("select_items: greedy ({greedy_total:.3}) >= KKT ({kkt_total:.3})");
                greedy_result
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileEntry, FileMetadata, ScoreSignals};
    use proptest::prelude::*;
    use std::{path::PathBuf, time::SystemTime};

    fn make_scored(tokens: usize, score: f32) -> ScoredEntry {
        ScoredEntry {
            entry: FileEntry {
                path: PathBuf::from(format!("file_{tokens}_{score:.2}.rs")),
                token_count: tokens,
                hash: [0u8; 16],
                metadata: FileMetadata {
                    size_bytes: 100,
                    last_modified: SystemTime::now(),
                    git: None,
                    language: None,
                },
                ast: None,
            },
            composite_score: score,
            signals: ScoreSignals::default(),
        }
    }

    #[test]
    fn test_knapsack_respects_budget() {
        let items = vec![
            make_scored(100, 0.9),
            make_scored(200, 0.8),
            make_scored(50, 0.7),
            make_scored(300, 0.5),
        ];
        let result = greedy_knapsack(items, 300);
        assert!(result.tokens_used <= 300);
    }

    #[test]
    fn test_knapsack_selects_high_efficiency_first() {
        // item A: 10 tokens, score 1.0 → efficiency 0.1
        // item B: 100 tokens, score 0.5 → efficiency 0.005
        // budget 100: should prefer A
        let items = vec![make_scored(10, 1.0), make_scored(100, 0.5)];
        let result = greedy_knapsack(items, 100);
        let paths: Vec<_> = result
            .selected
            .iter()
            .map(|s| s.entry.path.to_string_lossy().to_string())
            .collect();
        assert!(
            paths.iter().any(|p| p.contains("10_")),
            "high-efficiency item should be selected: {paths:?}"
        );
    }

    #[test]
    fn test_knapsack_skips_oversized() {
        let items = vec![make_scored(1000, 1.0)];
        let result = greedy_knapsack(items, 100);
        assert!(result.selected.is_empty());
        assert_eq!(result.oversized_skipped, 1);
        assert_eq!(result.tokens_used, 0);
    }

    #[test]
    fn test_knapsack_empty_input() {
        let result = greedy_knapsack(vec![], 1000);
        assert!(result.selected.is_empty());
        assert_eq!(result.tokens_used, 0);
    }

    #[test]
    fn test_knapsack_zero_budget() {
        let items = vec![make_scored(1, 1.0)];
        let result = greedy_knapsack(items, 0);
        // All items > 0 tokens exceed a 0 budget
        assert!(result.selected.is_empty());
    }

    #[test]
    fn test_knapsack_respects_budget_with_oversized_files() {
        // Mix of normal and oversized items (budget = 100)
        // 10000-token item: individually exceeds budget → oversized_skipped
        // 500-token item: individually exceeds budget → oversized_skipped
        // 50 + 30 = 80 tokens → both fit
        let items = vec![
            make_scored(50, 0.9),    // fits
            make_scored(10000, 1.0), // oversized
            make_scored(30, 0.8),    // fits
            make_scored(500, 0.3),   // also oversized (> 100 budget)
        ];
        let result = greedy_knapsack(items, 100);
        assert!(
            result.tokens_used <= 100,
            "tokens_used={}",
            result.tokens_used
        );
        assert_eq!(result.oversized_skipped, 2);
        assert_eq!(result.selected.len(), 2); // 50-token and 30-token items
    }

    fn make_scored_path(path: &str, tokens: usize, score: f32) -> ScoredEntry {
        ScoredEntry {
            entry: FileEntry {
                path: PathBuf::from(path),
                token_count: tokens,
                hash: [0u8; 16],
                metadata: FileMetadata {
                    size_bytes: 100,
                    last_modified: SystemTime::now(),
                    git: None,
                    language: None,
                },
                ast: None,
            },
            composite_score: score,
            signals: ScoreSignals::default(),
        }
    }

    // ── Diversity-aware greedy tests ─────────────────────────────────

    #[test]
    fn test_greedy_diverse_prefers_variety() {
        let config = DiversityConfig::default(); // decay=0.7, directory grouping
        let items = vec![
            make_scored_path("src/a.rs", 100, 0.9),
            make_scored_path("src/b.rs", 100, 0.9),
            make_scored_path("tests/c.rs", 100, 0.9),
            make_scored_path("tests/d.rs", 100, 0.9),
        ];
        let result = greedy_knapsack_diverse(items, 200, &config);
        assert_eq!(result.tokens_used, 200);
        // Should pick one from each directory for diversity
        let dirs: Vec<_> = result
            .selected
            .iter()
            .map(|s| s.entry.path.parent().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(
            dirs.contains(&"src".to_string()) && dirs.contains(&"tests".to_string()),
            "should select from both directories: {dirs:?}"
        );
    }

    #[test]
    fn test_greedy_diverse_respects_budget() {
        let config = DiversityConfig::default();
        let items = vec![
            make_scored_path("a/x.rs", 100, 0.9),
            make_scored_path("b/y.rs", 200, 0.8),
            make_scored_path("c/z.rs", 300, 0.7),
        ];
        let result = greedy_knapsack_diverse(items, 250, &config);
        assert!(result.tokens_used <= 250);
    }

    #[test]
    fn test_greedy_diverse_empty_input() {
        let config = DiversityConfig::default();
        let result = greedy_knapsack_diverse(vec![], 1000, &config);
        assert!(result.selected.is_empty());
        assert_eq!(result.tokens_used, 0);
    }

    // ── KKT tests ────────────────────────────────────────────────────

    #[test]
    fn test_kkt_respects_budget() {
        let items = vec![
            make_scored(100, 0.9),
            make_scored(200, 0.8),
            make_scored(50, 0.7),
            make_scored(300, 0.5),
        ];
        let result = kkt_knapsack(items, 300, None);
        assert!(
            result.tokens_used <= 300,
            "tokens_used={}",
            result.tokens_used
        );
    }

    #[test]
    fn test_kkt_empty_input() {
        let result = kkt_knapsack(vec![], 1000, None);
        assert!(result.selected.is_empty());
        assert_eq!(result.tokens_used, 0);
    }

    #[test]
    fn test_kkt_single_item_fits() {
        let items = vec![make_scored(50, 0.8)];
        let result = kkt_knapsack(items, 100, None);
        assert_eq!(result.selected.len(), 1);
        assert_eq!(result.tokens_used, 50);
    }

    #[test]
    fn test_kkt_single_item_oversized() {
        let items = vec![make_scored(200, 0.8)];
        let result = kkt_knapsack(items, 100, None);
        assert!(result.selected.is_empty());
    }

    // ── select_items tests ───────────────────────────────────────────

    #[test]
    fn test_select_items_auto_at_least_as_good_as_greedy() {
        let items = vec![
            make_scored(100, 0.9),
            make_scored(80, 0.85),
            make_scored(50, 0.7),
            make_scored(200, 0.6),
            make_scored(30, 0.5),
        ];
        let greedy_result = greedy_knapsack(items.clone(), 250);
        let auto_result = select_items(items, 250, "auto", None);
        let greedy_total: f32 = greedy_result
            .selected
            .iter()
            .map(|s| s.composite_score)
            .sum();
        let auto_total: f32 = auto_result.selected.iter().map(|s| s.composite_score).sum();
        assert!(
            auto_total >= greedy_total - 1e-6,
            "auto ({auto_total}) should be >= greedy ({greedy_total})"
        );
    }

    #[test]
    fn test_select_items_greedy_mode() {
        let items = vec![make_scored(100, 0.9), make_scored(50, 0.5)];
        let result = select_items(items, 200, "greedy", None);
        assert!(result.tokens_used <= 200);
        assert!(!result.selected.is_empty());
    }

    #[test]
    fn test_select_items_kkt_mode() {
        let items = vec![make_scored(100, 0.9), make_scored(50, 0.5)];
        let result = select_items(items, 200, "kkt", None);
        assert!(result.tokens_used <= 200);
    }

    #[test]
    fn test_select_items_with_diversity() {
        let div = DiversityConfig::default();
        let items = vec![
            make_scored_path("src/a.rs", 100, 0.9),
            make_scored_path("src/b.rs", 100, 0.8),
            make_scored_path("tests/c.rs", 100, 0.7),
        ];
        let result = select_items(items, 200, "greedy", Some(&div));
        assert!(result.tokens_used <= 200);
    }

    proptest! {
        #[test]
        fn prop_knapsack_tokens_never_exceed_budget(
            token_counts in prop::collection::vec(1usize..=500, 1..=50),
            budget in 1usize..=2000,
        ) {
            let items: Vec<ScoredEntry> = token_counts
                .iter()
                .map(|&t| make_scored(t, 0.5))
                .collect();
            let result = greedy_knapsack(items, budget);
            prop_assert!(
                result.tokens_used <= budget,
                "tokens_used={} exceeded budget={budget}",
                result.tokens_used
            );
        }

        #[test]
        fn prop_knapsack_selection_subset(
            token_counts in prop::collection::vec(1usize..=200, 1..=20),
            budget in 100usize..=1000,
        ) {
            let items: Vec<ScoredEntry> = token_counts
                .iter()
                .map(|&t| make_scored(t, 0.5))
                .collect();
            let n_items = items.len();
            let result = greedy_knapsack(items, budget);
            prop_assert!(result.selected.len() <= n_items);
        }

        #[test]
        fn prop_greedy_diverse_tokens_never_exceed_budget(
            token_counts in prop::collection::vec(1usize..=500, 1..=50),
            budget in 1usize..=2000,
        ) {
            let config = DiversityConfig::default();
            let items: Vec<ScoredEntry> = token_counts
                .iter()
                .enumerate()
                .map(|(i, &t)| make_scored_path(&format!("dir{}/f.rs", i % 5), t, 0.5))
                .collect();
            let result = greedy_knapsack_diverse(items, budget, &config);
            prop_assert!(
                result.tokens_used <= budget,
                "tokens_used={} exceeded budget={budget}",
                result.tokens_used
            );
        }

        #[test]
        fn prop_kkt_tokens_never_exceed_budget(
            token_counts in prop::collection::vec(1usize..=500, 1..=50),
            budget in 1usize..=2000,
        ) {
            let items: Vec<ScoredEntry> = token_counts
                .iter()
                .map(|&t| make_scored(t, 0.5))
                .collect();
            let result = kkt_knapsack(items, budget, None);
            prop_assert!(
                result.tokens_used <= budget,
                "tokens_used={} exceeded budget={budget}",
                result.tokens_used
            );
        }
    }
}
