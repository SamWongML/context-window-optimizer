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
    }
}
