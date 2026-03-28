/// Individual scoring signals, each normalized to `[0.0, 1.0]`.
pub mod signals;

use crate::{
    config::ScoringWeights,
    index::depgraph::DependencyGraph,
    types::{FileEntry, ScoreSignals, ScoredEntry},
};
use rayon::prelude::*;
use signals::{dependency_signal, entry_recency_signal, proximity_signal, size_signal};
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
}
