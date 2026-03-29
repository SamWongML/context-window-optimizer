/// Individual scoring signals, each normalized to `[0.0, 1.0]`.
pub mod signals;

use crate::{
    config::ScoringWeights,
    index::depgraph::DependencyGraph,
    types::{FileEntry, ScoreSignals, ScoredEntry},
};
use rayon::prelude::*;
use signals::{dependency_signal, entry_recency_signal, proximity_signal, size_signal};
use std::collections::HashMap;
use std::path::PathBuf;

/// Score a single `FileEntry`, producing a `ScoredEntry`.
///
/// The composite score is a weighted average of all signals:
/// `composite = (w_rec * recency + w_size * size + w_prox * proximity) / (w_rec + w_size + w_prox)`.
///
/// All intermediate values are clamped to `[0.0, 1.0]`.
///
/// # Examples
/// ```
/// use ctx_optim::{config::ScoringWeights, scoring::score_entry, types::*};
/// use std::{path::PathBuf, time::SystemTime};
/// let entry = FileEntry {
///     path: PathBuf::from("src/main.rs"),
///     token_count: 200,
///     hash: [0u8; 16],
///     metadata: FileMetadata {
///         size_bytes: 400,
///         last_modified: SystemTime::now(),
///         git: None,
///         language: None,
///     },
///     ast: None,
///     simhash: None,
/// };
/// let scored = score_entry(&entry, &ScoringWeights::default(), &[], None);
/// assert!((0.0..=1.0).contains(&scored.composite_score));
/// ```
pub fn score_entry(
    entry: &FileEntry,
    weights: &ScoringWeights,
    focus_paths: &[PathBuf],
    dep_graph: Option<&DependencyGraph>,
) -> ScoredEntry {
    let recency = entry_recency_signal(entry);
    let size_score = size_signal(entry.token_count);
    let proximity = proximity_signal(&entry.path, focus_paths);
    let dependency = dep_graph
        .map(|g| dependency_signal(g.distance(focus_paths, &entry.path)))
        .unwrap_or(0.0);

    let signals = ScoreSignals {
        recency,
        size_score,
        proximity,
        dependency,
    };

    let weight_sum = weights.recency + weights.size + weights.proximity + weights.dependency;
    let composite = if weight_sum > 0.0 {
        (weights.recency * recency
            + weights.size * size_score
            + weights.proximity * proximity
            + weights.dependency * dependency)
            / weight_sum
    } else {
        0.0
    };

    ScoredEntry {
        entry: entry.clone(),
        composite_score: composite.clamp(0.0, 1.0),
        signals,
    }
}

/// Score a batch of `FileEntry` records in parallel using Rayon.
///
/// The order of results matches the order of inputs.
///
/// # Examples
/// ```no_run
/// use ctx_optim::{config::ScoringWeights, scoring::score_entries, types::*};
/// let entries: Vec<FileEntry> = vec![];
/// let scored = score_entries(&entries, &ScoringWeights::default(), &[], None);
/// assert_eq!(scored.len(), entries.len());
/// ```
pub fn score_entries(
    entries: &[FileEntry],
    weights: &ScoringWeights,
    focus_paths: &[PathBuf],
    dep_graph: Option<&DependencyGraph>,
) -> Vec<ScoredEntry> {
    entries
        .par_iter()
        .map(|e| score_entry(e, weights, focus_paths, dep_graph))
        .collect()
}

/// Score a single `FileEntry` with an optional per-file feedback multiplier.
///
/// If the file's path appears in `feedback_multipliers`, its composite score
/// is multiplied (and clamped to `[0.0, 1.0]`).
///
/// # Examples
///
/// ```no_run
/// use ctx_optim::{config::ScoringWeights, scoring::score_entry_with_feedback};
/// use std::{collections::HashMap, path::PathBuf};
/// let multipliers: HashMap<PathBuf, f32> = HashMap::new();
/// ```
pub fn score_entry_with_feedback(
    entry: &FileEntry,
    weights: &ScoringWeights,
    focus_paths: &[PathBuf],
    dep_graph: Option<&DependencyGraph>,
    feedback_multipliers: &HashMap<PathBuf, f32>,
) -> ScoredEntry {
    let mut scored = score_entry(entry, weights, focus_paths, dep_graph);
    if let Some(&multiplier) = feedback_multipliers.get(&entry.path) {
        scored.composite_score = (scored.composite_score * multiplier).clamp(0.0, 1.0);
    }
    scored
}

/// Score a batch of entries in parallel, with optional feedback multipliers.
///
/// If `feedback_multipliers` is empty, delegates to [`score_entries`] with no overhead.
///
/// # Examples
///
/// ```no_run
/// use ctx_optim::{config::ScoringWeights, scoring::score_entries_with_feedback, types::FileEntry};
/// use std::collections::HashMap;
/// let entries: Vec<FileEntry> = vec![];
/// let scored = score_entries_with_feedback(&entries, &ScoringWeights::default(), &[], None, &HashMap::new());
/// assert_eq!(scored.len(), entries.len());
/// ```
pub fn score_entries_with_feedback(
    entries: &[FileEntry],
    weights: &ScoringWeights,
    focus_paths: &[PathBuf],
    dep_graph: Option<&DependencyGraph>,
    feedback_multipliers: &HashMap<PathBuf, f32>,
) -> Vec<ScoredEntry> {
    if feedback_multipliers.is_empty() {
        return score_entries(entries, weights, focus_paths, dep_graph);
    }
    entries
        .par_iter()
        .map(|e| {
            score_entry_with_feedback(e, weights, focus_paths, dep_graph, feedback_multipliers)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileMetadata, GitMetadata};
    use std::time::SystemTime;

    fn make_entry(path: &str, token_count: usize, age_days: f64) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            token_count,
            hash: [0u8; 16],
            metadata: FileMetadata {
                size_bytes: token_count as u64 * 4,
                last_modified: SystemTime::now(),
                git: Some(GitMetadata {
                    age_days,
                    commit_count: 1,
                }),
                language: None,
            },
            ast: None,
            simhash: None,
        }
    }

    #[test]
    fn test_score_entry_bounded() {
        let entry = make_entry("src/main.rs", 300, 5.0);
        let scored = score_entry(&entry, &ScoringWeights::default(), &[], None);
        assert!((0.0..=1.0).contains(&scored.composite_score));
        assert!((0.0..=1.0).contains(&scored.signals.recency));
        assert!((0.0..=1.0).contains(&scored.signals.size_score));
        assert!((0.0..=1.0).contains(&scored.signals.proximity));
    }

    #[test]
    fn test_recent_file_scores_higher() {
        let fresh = make_entry("src/fresh.rs", 300, 0.0);
        let old = make_entry("src/old.rs", 300, 365.0);
        let w = ScoringWeights {
            recency: 1.0,
            size: 0.0,
            proximity: 0.0,
            dependency: 0.0,
        };
        let s_fresh = score_entry(&fresh, &w, &[], None).composite_score;
        let s_old = score_entry(&old, &w, &[], None).composite_score;
        assert!(s_fresh > s_old, "fresh={s_fresh} should beat old={s_old}");
    }

    #[test]
    fn test_small_file_scores_higher_on_size_weight() {
        let small = make_entry("src/small.rs", 50, 10.0);
        let large = make_entry("src/large.rs", 5000, 10.0);
        let w = ScoringWeights {
            recency: 0.0,
            size: 1.0,
            proximity: 0.0,
            dependency: 0.0,
        };
        let s_small = score_entry(&small, &w, &[], None).composite_score;
        let s_large = score_entry(&large, &w, &[], None).composite_score;
        assert!(
            s_small > s_large,
            "small={s_small} should beat large={s_large}"
        );
    }

    #[test]
    fn test_score_entries_parallel() {
        let entries: Vec<FileEntry> = (0..100)
            .map(|i| make_entry(&format!("src/file{i}.rs"), 100 + i, i as f64))
            .collect();
        let scored = score_entries(&entries, &ScoringWeights::default(), &[], None);
        assert_eq!(scored.len(), 100);
        for s in &scored {
            assert!((0.0..=1.0).contains(&s.composite_score));
        }
    }

    #[test]
    fn test_zero_weight_sum_gives_zero_score() {
        let entry = make_entry("src/x.rs", 100, 1.0);
        let w = ScoringWeights {
            recency: 0.0,
            size: 0.0,
            proximity: 0.0,
            dependency: 0.0,
        };
        let scored = score_entry(&entry, &w, &[], None);
        assert_eq!(scored.composite_score, 0.0);
    }

    #[test]
    fn test_score_entry_with_feedback_multiplier() {
        let entry = make_entry("src/hot_file.rs", 200, 5.0);
        let w = ScoringWeights::default();
        let scored_normal = score_entry(&entry, &w, &[], None);

        let mut multipliers = std::collections::HashMap::new();
        multipliers.insert(std::path::PathBuf::from("src/hot_file.rs"), 1.5f32);
        let scored_boosted = score_entry_with_feedback(&entry, &w, &[], None, &multipliers);

        assert!(
            scored_boosted.composite_score > scored_normal.composite_score,
            "feedback multiplier should boost score: {:.3} vs {:.3}",
            scored_boosted.composite_score,
            scored_normal.composite_score
        );
    }

    #[test]
    fn test_score_entry_with_empty_feedback() {
        let entry = make_entry("src/main.rs", 200, 5.0);
        let w = ScoringWeights::default();
        let scored_normal = score_entry(&entry, &w, &[], None);

        let multipliers = std::collections::HashMap::new();
        let scored_with_empty = score_entry_with_feedback(&entry, &w, &[], None, &multipliers);

        assert!(
            (scored_with_empty.composite_score - scored_normal.composite_score).abs() < 1e-6,
            "empty feedback should not change score"
        );
    }

    #[test]
    fn test_focus_weights_boost_adjacent_file_over_unrelated() {
        // Both files have the same age so recency is equal.  With focus-boosted
        // weights the file co-located with the focus should win purely via proximity.
        let readme = make_entry("README.md", 400, 5.0);
        let signals = make_entry("src/scoring/signals.rs", 600, 5.0);

        // Focus-boosted weights (as applied in lib.rs when focus_paths is set)
        let focus_w = ScoringWeights {
            recency: 0.15,
            size: 0.05,
            proximity: 0.45,
            dependency: 0.35,
        };
        let focus_paths = vec![PathBuf::from("src/scoring/mod.rs")];

        let score_readme = score_entry(&readme, &focus_w, &focus_paths, None).composite_score;
        let score_signals = score_entry(&signals, &focus_w, &focus_paths, None).composite_score;

        assert!(
            score_signals > score_readme,
            "adjacent scoring file (score={score_signals:.3}) should beat unrelated README (score={score_readme:.3}) with focus weights"
        );
    }

    #[test]
    fn test_focus_weights_exact_focus_file_ranks_highest() {
        // The focus file itself should always rank highest among candidates at the
        // same age, because it gets proximity = 1.0.
        let focus_file = make_entry("src/scoring/mod.rs", 300, 5.0);
        let unrelated = make_entry("README.md", 100, 5.0); // same age, very small (good size score)

        let focus_w = ScoringWeights {
            recency: 0.15,
            size: 0.05,
            proximity: 0.45,
            dependency: 0.35,
        };
        let focus_paths = vec![PathBuf::from("src/scoring/mod.rs")];

        let s_focus = score_entry(&focus_file, &focus_w, &focus_paths, None).composite_score;
        let s_other = score_entry(&unrelated, &focus_w, &focus_paths, None).composite_score;

        assert!(
            s_focus > s_other,
            "focus file itself (score={s_focus:.3}) should outrank unrelated file (score={s_other:.3})"
        );
    }
}
