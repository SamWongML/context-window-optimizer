//! EMA-based scoring weight updates from feedback data.

use crate::config::ScoringWeights;
use crate::error::OptimError;
use crate::feedback::store::{FeedbackStore, WeightSnapshot};
use crate::feedback::unix_now;

// ── Public API ────────────────────────────────────────────────────────────────

/// Compute updated [`ScoringWeights`] from stored feedback using EMA blending.
///
/// The algorithm:
/// 1. Loads all feedback records that have a utilization value from `store`.
/// 2. Returns `current` unchanged if there are no records or no high-utilization
///    records (>= 0.5).
/// 3. Computes signal correlations between high-util and low-util groups to
///    derive a target weight vector:
///    - High-util files smaller than low-util → boost `size` weight (× 1.2).
///    - High-util files larger than low-util → reduce `size` weight (× 0.8).
///    - Score discrimination poor (mean low-util score > mean high-util score)
///      → boost `recency` weight (× 1.3).
/// 4. Blends current and target via EMA:
///    `w_new = (1 - alpha) * w_current + alpha * w_target`
/// 5. Normalises so all weights sum to 1.0.
///
/// # Errors
///
/// Returns [`OptimError::Feedback`] if the store cannot be queried.
///
/// # Examples
///
/// ```no_run
/// use ctx_optim::config::ScoringWeights;
/// use ctx_optim::feedback::store::FeedbackStore;
/// use ctx_optim::feedback::learning::compute_updated_weights;
///
/// let store = FeedbackStore::open_in_memory().unwrap();
/// let current = ScoringWeights::default();
/// let updated = compute_updated_weights(&store, &current, 0.1).unwrap();
/// // No data → weights unchanged
/// assert!((updated.recency - current.recency).abs() < 1e-6);
/// ```
pub fn compute_updated_weights(
    store: &FeedbackStore,
    current: &ScoringWeights,
    learning_rate: f32,
) -> Result<ScoringWeights, OptimError> {
    let records = store.all_feedback_with_utilization()?;

    if records.is_empty() {
        return Ok(current.clone());
    }

    // Partition into high-util (>= 0.5) and low-util (< 0.5).
    let high: Vec<_> = records
        .iter()
        .filter(|r| r.utilization.unwrap_or(0.0) >= 0.5)
        .collect();
    let low: Vec<_> = records
        .iter()
        .filter(|r| r.utilization.unwrap_or(0.0) < 0.5)
        .collect();

    if high.is_empty() {
        return Ok(current.clone());
    }

    // Build a target weight vector starting from current weights.
    let mut target = current.clone();

    // Signal 1: size correlation.
    let high_avg_tokens: f32 =
        high.iter().map(|r| r.token_count as f32).sum::<f32>() / high.len() as f32;

    if !low.is_empty() {
        let low_avg_tokens: f32 =
            low.iter().map(|r| r.token_count as f32).sum::<f32>() / low.len() as f32;

        if high_avg_tokens < low_avg_tokens {
            // High-util files are smaller → smaller files are more valuable → boost size.
            target.size *= 1.2;
        } else {
            // High-util files are larger → penalise inverse-size signal.
            target.size *= 0.8;
        }
    }

    // Signal 2: score discrimination.
    let high_avg_score: f32 =
        high.iter().map(|r| r.composite_score).sum::<f32>() / high.len() as f32;

    if !low.is_empty() {
        let low_avg_score: f32 =
            low.iter().map(|r| r.composite_score).sum::<f32>() / low.len() as f32;

        if low_avg_score > high_avg_score {
            // Current scores fail to separate high/low utilization → boost recency.
            target.recency *= 1.3;
        }
    }

    // Apply EMA blend: w_new = (1 - alpha) * w_current + alpha * w_target.
    let alpha = learning_rate;
    let blended = ScoringWeights {
        recency: (1.0 - alpha) * current.recency + alpha * target.recency,
        size: (1.0 - alpha) * current.size + alpha * target.size,
        proximity: (1.0 - alpha) * current.proximity + alpha * target.proximity,
        dependency: (1.0 - alpha) * current.dependency + alpha * target.dependency,
    };

    // Normalise so weights sum to 1.0.
    let sum = blended.recency + blended.size + blended.proximity + blended.dependency;
    if sum <= 0.0 {
        return Err(OptimError::Feedback(
            "weight sum is zero after EMA blend".to_string(),
        ));
    }

    Ok(ScoringWeights {
        recency: blended.recency / sum,
        size: blended.size / sum,
        proximity: blended.proximity / sum,
        dependency: blended.dependency / sum,
    })
}

/// Run a full learning cycle: compute new weights, persist a snapshot, and
/// return the updated weights when feedback data is available.
///
/// Returns `Some(weights)` if there was feedback data to learn from, or `None`
/// if the store was empty (current weights are preserved).
///
/// # Errors
///
/// Returns [`OptimError::Feedback`] if the store cannot be read or written.
///
/// # Examples
///
/// ```no_run
/// use ctx_optim::config::ScoringWeights;
/// use ctx_optim::feedback::store::FeedbackStore;
/// use ctx_optim::feedback::learning::run_learning_cycle;
///
/// let store = FeedbackStore::open_in_memory().unwrap();
/// let current = ScoringWeights::default();
/// // Empty store → no snapshot saved, None returned.
/// let result = run_learning_cycle(&store, &current, 0.1).unwrap();
/// assert!(result.is_none());
/// ```
pub fn run_learning_cycle(
    store: &FeedbackStore,
    current: &ScoringWeights,
    learning_rate: f32,
) -> Result<Option<ScoringWeights>, OptimError> {
    let records = store.all_feedback_with_utilization()?;

    if records.is_empty() {
        return Ok(None);
    }

    // Compute average utilization across all records.
    let avg_utilization: f32 = records
        .iter()
        .map(|r| r.utilization.unwrap_or(0.0))
        .sum::<f32>()
        / records.len() as f32;

    let updated = compute_updated_weights(store, current, learning_rate)?;

    let snapshot = WeightSnapshot {
        recency: updated.recency,
        size: updated.size,
        proximity: updated.proximity,
        dependency: updated.dependency,
        avg_utilization,
        created_at: unix_now(),
    };
    store.save_weights(&snapshot)?;

    Ok(Some(updated))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feedback::store::{FeedbackRecord, FeedbackStore, Session};

    fn seed_feedback(store: &FeedbackStore, n_sessions: usize) {
        for i in 0..n_sessions {
            let session = Session {
                id: format!("s{i}"),
                repo: "/repo".to_string(),
                budget: 128_000,
                created_at: 1711612800 + i as i64,
            };
            store.create_session(&session).unwrap();
            store
                .record_feedback(
                    &format!("s{i}"),
                    &[
                        FeedbackRecord {
                            file_path: "src/recent.rs".to_string(),
                            token_count: 100,
                            composite_score: 0.9,
                            was_selected: true,
                            utilization: Some(0.8),
                        },
                        FeedbackRecord {
                            file_path: "src/old_large.rs".to_string(),
                            token_count: 2000,
                            composite_score: 0.3,
                            was_selected: true,
                            utilization: Some(0.1),
                        },
                    ],
                )
                .unwrap();
        }
    }

    #[test]
    fn test_compute_updated_weights_returns_valid_weights() {
        let store = FeedbackStore::open_in_memory().unwrap();
        seed_feedback(&store, 5);
        let current = ScoringWeights::default();
        let updated = compute_updated_weights(&store, &current, 0.1).unwrap();

        assert!(updated.recency > 0.0, "recency should be positive");
        assert!(updated.size > 0.0, "size should be positive");
        assert!(updated.proximity > 0.0, "proximity should be positive");
        assert!(updated.dependency > 0.0, "dependency should be positive");
    }

    #[test]
    fn test_compute_updated_weights_sums_to_one() {
        let store = FeedbackStore::open_in_memory().unwrap();
        seed_feedback(&store, 10);
        let current = ScoringWeights::default();
        let updated = compute_updated_weights(&store, &current, 0.1).unwrap();

        let sum = updated.recency + updated.size + updated.proximity + updated.dependency;
        assert!(
            (sum - 1.0).abs() < 1e-5,
            "weights should sum to 1.0, got {sum}"
        );
    }

    #[test]
    fn test_compute_updated_weights_no_feedback_returns_current() {
        let store = FeedbackStore::open_in_memory().unwrap();
        let current = ScoringWeights::default();
        let updated = compute_updated_weights(&store, &current, 0.1).unwrap();

        assert!(
            (updated.recency - current.recency).abs() < 1e-6,
            "recency should be unchanged"
        );
        assert!(
            (updated.size - current.size).abs() < 1e-6,
            "size should be unchanged"
        );
        assert!(
            (updated.proximity - current.proximity).abs() < 1e-6,
            "proximity should be unchanged"
        );
        assert!(
            (updated.dependency - current.dependency).abs() < 1e-6,
            "dependency should be unchanged"
        );
    }

    #[test]
    fn test_learning_rate_zero_preserves_current() {
        let store = FeedbackStore::open_in_memory().unwrap();
        seed_feedback(&store, 5);
        let current = ScoringWeights::default();
        // With alpha = 0.0, EMA blend is entirely the current weights.
        // After normalisation the proportions remain the same, so the
        // normalised result equals the normalised current weights.
        let updated = compute_updated_weights(&store, &current, 0.0).unwrap();

        let cur_sum = current.recency + current.size + current.proximity + current.dependency;
        assert!(
            (updated.recency - current.recency / cur_sum).abs() < 1e-5,
            "recency should match normalised current"
        );
        assert!(
            (updated.size - current.size / cur_sum).abs() < 1e-5,
            "size should match normalised current"
        );
        assert!(
            (updated.proximity - current.proximity / cur_sum).abs() < 1e-5,
            "proximity should match normalised current"
        );
        assert!(
            (updated.dependency - current.dependency / cur_sum).abs() < 1e-5,
            "dependency should match normalised current"
        );
    }

    #[test]
    fn test_run_learning_cycle_stores_weights() {
        let store = FeedbackStore::open_in_memory().unwrap();
        seed_feedback(&store, 10);
        let current = ScoringWeights::default();

        let result = run_learning_cycle(&store, &current, 0.1).unwrap();
        assert!(
            result.is_some(),
            "expected Some(weights) with feedback data"
        );

        let latest = store.latest_weights().unwrap();
        assert!(
            latest.is_some(),
            "store should have a weight snapshot after learning cycle"
        );
    }

    #[test]
    fn test_run_learning_cycle_insufficient_data() {
        let store = FeedbackStore::open_in_memory().unwrap();
        // 2 sessions is still enough to compute — task spec says assert Some returned.
        seed_feedback(&store, 2);
        let current = ScoringWeights::default();

        let result = run_learning_cycle(&store, &current, 0.1).unwrap();
        assert!(
            result.is_some(),
            "expected Some even with only 2 sessions of data"
        );
    }
}
