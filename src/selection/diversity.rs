use crate::types::ScoredEntry;
use std::collections::HashMap;
use std::path::Path;

/// Strategy for grouping files in the diversity constraint.
///
/// # Examples
///
/// ```
/// use ctx_optim::selection::diversity::GroupingStrategy;
/// let s = GroupingStrategy::Directory;
/// assert_eq!(s, GroupingStrategy::Directory);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupingStrategy {
    /// Group by immediate parent directory.
    #[default]
    Directory,
    /// Group by grandparent directory (coarser grouping).
    Parent,
}

/// Configuration for submodular diversity during selection.
///
/// Groups files by directory and applies diminishing returns to
/// additional selections from the same group.
///
/// # Examples
///
/// ```
/// use ctx_optim::selection::diversity::DiversityConfig;
/// let config = DiversityConfig::default();
/// assert!(config.enabled);
/// assert!((config.decay - 0.7).abs() < 1e-6);
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiversityConfig {
    /// Whether diversity constraint is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Decay factor per same-group selection.
    ///
    /// `effective_score = composite_score * decay^(group_count)`.
    /// Lower values penalize same-group selections more aggressively.
    /// Must be in `(0.0, 1.0]`.
    #[serde(default = "default_decay")]
    pub decay: f32,
    /// How to group files for diversity purposes.
    #[serde(default)]
    pub grouping: GroupingStrategy,
}

fn default_enabled() -> bool {
    true
}

fn default_decay() -> f32 {
    0.7
}

impl Default for DiversityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            decay: 0.7,
            grouping: GroupingStrategy::Directory,
        }
    }
}

/// Tracks per-group selection counts and computes diversity-adjusted scores.
///
/// Implements a facility-location style submodular function where each
/// additional file from the same directory group gets an exponentially
/// decaying score multiplier.
///
/// # Examples
///
/// ```
/// use ctx_optim::selection::diversity::{DiversityConfig, DiversityTracker};
/// let config = DiversityConfig::default();
/// let tracker = DiversityTracker::new(&config);
/// // First selection from a group gets full score
/// ```
pub struct DiversityTracker {
    group_counts: HashMap<String, usize>,
    decay: f32,
    grouping: GroupingStrategy,
}

impl DiversityTracker {
    /// Create a new tracker from configuration.
    pub fn new(config: &DiversityConfig) -> Self {
        Self {
            group_counts: HashMap::new(),
            decay: config.decay,
            grouping: config.grouping,
        }
    }

    /// Compute the diversity-adjusted effective score for an item.
    ///
    /// `effective = composite_score * decay^(current_group_count)`
    ///
    /// The first file from a group gets full score, the second gets
    /// `decay * score`, the third gets `decay^2 * score`, etc.
    pub fn effective_score(&self, entry: &ScoredEntry) -> f32 {
        let key = group_key(&entry.entry.path, self.grouping);
        let count = self.group_counts.get(&key).copied().unwrap_or(0);
        entry.composite_score * self.decay.powi(count as i32)
    }

    /// Record that an item has been selected (increments its group count).
    pub fn record_selection(&mut self, entry: &ScoredEntry) {
        let key = group_key(&entry.entry.path, self.grouping);
        *self.group_counts.entry(key).or_insert(0) += 1;
    }
}

/// Extract the group key for a file path based on the grouping strategy.
fn group_key(path: &Path, strategy: GroupingStrategy) -> String {
    match strategy {
        GroupingStrategy::Directory => path.parent().unwrap_or(path).to_string_lossy().to_string(),
        GroupingStrategy::Parent => path
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or(path)
            .to_string_lossy()
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileEntry, FileMetadata, ScoreSignals};
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn make_scored(path: &str, tokens: usize, score: f32) -> ScoredEntry {
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
                simhash: None,
            },
            composite_score: score,
            signals: ScoreSignals::default(),
        }
    }

    #[test]
    fn test_diversity_config_defaults() {
        let config = DiversityConfig::default();
        assert!(config.enabled);
        assert!((config.decay - 0.7).abs() < 1e-6);
        assert_eq!(config.grouping, GroupingStrategy::Directory);
    }

    #[test]
    fn test_diversity_tracker_first_selection_full_score() {
        let config = DiversityConfig::default();
        let tracker = DiversityTracker::new(&config);
        let entry = make_scored("src/main.rs", 100, 0.9);
        let eff = tracker.effective_score(&entry);
        assert!(
            (eff - 0.9).abs() < 1e-6,
            "first selection should get full score, got {eff}"
        );
    }

    #[test]
    fn test_diversity_tracker_decays_same_group() {
        let config = DiversityConfig::default();
        let mut tracker = DiversityTracker::new(&config);

        let entry1 = make_scored("src/a.rs", 100, 0.9);
        let entry2 = make_scored("src/b.rs", 100, 0.9);

        tracker.record_selection(&entry1);
        let eff = tracker.effective_score(&entry2);
        // Same group "src", count=1, so 0.9 * 0.7^1 = 0.63
        assert!(
            (eff - 0.63).abs() < 1e-5,
            "second from same group should decay, got {eff}"
        );

        tracker.record_selection(&entry2);
        let entry3 = make_scored("src/c.rs", 100, 0.9);
        let eff3 = tracker.effective_score(&entry3);
        // count=2, so 0.9 * 0.7^2 = 0.441
        assert!(
            (eff3 - 0.441).abs() < 1e-4,
            "third from same group should double-decay, got {eff3}"
        );
    }

    #[test]
    fn test_diversity_tracker_different_groups_no_penalty() {
        let config = DiversityConfig::default();
        let mut tracker = DiversityTracker::new(&config);

        let entry1 = make_scored("src/a.rs", 100, 0.9);
        tracker.record_selection(&entry1);

        let entry2 = make_scored("tests/b.rs", 100, 0.9);
        let eff = tracker.effective_score(&entry2);
        assert!(
            (eff - 0.9).abs() < 1e-6,
            "different group should get full score, got {eff}"
        );
    }

    #[test]
    fn test_group_key_directory_strategy() {
        let key = group_key(
            Path::new("src/scoring/signals.rs"),
            GroupingStrategy::Directory,
        );
        assert_eq!(key, "src/scoring");
    }

    #[test]
    fn test_group_key_parent_strategy() {
        let key = group_key(
            Path::new("src/scoring/signals.rs"),
            GroupingStrategy::Parent,
        );
        assert_eq!(key, "src");
    }

    #[test]
    fn test_group_key_root_file() {
        let key = group_key(Path::new("Cargo.toml"), GroupingStrategy::Directory);
        assert_eq!(key, "");
    }
}
