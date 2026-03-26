use crate::types::FileEntry;
use std::path::Path;
use std::time::SystemTime;

/// Compute the recency signal for a file, normalized to `[0.0, 1.0]`.
///
/// Uses an exponential decay with a 30-day half-life:
/// `recency = exp(-ln(2) * age_days / half_life)`.
///
/// Files modified today → 1.0. Files not touched in 6 months → ~0.03.
///
/// # Examples
/// ```
/// use ctx_optim::scoring::signals::recency_signal;
/// // A brand-new file should score close to 1.0
/// assert!(recency_signal(0.0) > 0.99);
/// // A file from 30 days ago should score around 0.5
/// let score = recency_signal(30.0);
/// assert!(score > 0.4 && score < 0.6, "got {score}");
/// ```
pub fn recency_signal(age_days: f64) -> f32 {
    const HALF_LIFE_DAYS: f64 = 30.0;
    let decay = (-std::f64::consts::LN_2 * age_days / HALF_LIFE_DAYS).exp();
    decay.clamp(0.0, 1.0) as f32
}

/// Compute the recency signal for a `FileEntry`.
///
/// Prefers git `age_days` if available; falls back to filesystem `last_modified`.
pub fn entry_recency_signal(entry: &FileEntry) -> f32 {
    if let Some(git) = &entry.metadata.git {
        return recency_signal(git.age_days);
    }

    // Fallback: use filesystem mtime
    let age_days = SystemTime::now()
        .duration_since(entry.metadata.last_modified)
        .unwrap_or_default()
        .as_secs_f64()
        / 86_400.0;
    recency_signal(age_days)
}

/// Compute the inverse-size signal for a file, normalized to `[0.0, 1.0]`.
///
/// Smaller files get higher scores — we prefer focused, dense files over
/// large boilerplate. Curve: `1 / (1 + tokens / inflection)`.
///
/// - 0 tokens → 1.0
/// - `inflection` tokens → 0.5 (default: 500)
/// - Very large files → 0.0
///
/// # Examples
/// ```
/// use ctx_optim::scoring::signals::size_signal;
/// assert!((size_signal(0) - 1.0).abs() < 1e-6);
/// let mid = size_signal(500);
/// assert!((mid - 0.5).abs() < 0.05);
/// assert!(size_signal(10_000) < 0.1);
/// ```
pub fn size_signal(token_count: usize) -> f32 {
    const INFLECTION: f32 = 500.0;
    1.0 / (1.0 + token_count as f32 / INFLECTION)
}

/// Compute path-based proximity between a candidate file and a set of focus paths.
///
/// Returns the maximum proximity across all focus paths, where proximity is
/// `1.0 / (1 + path_component_distance)`.
///
/// - Same file → 1.0
/// - Same directory → 0.5
/// - Parent/sibling directory → 0.33
/// - No focus paths → 0.0
///
/// # Examples
/// ```
/// use std::path::Path;
/// use ctx_optim::scoring::signals::proximity_signal;
/// let focus = vec![Path::new("src/index/mod.rs").to_path_buf()];
/// let close = proximity_signal(Path::new("src/index/discovery.rs"), &focus);
/// let far = proximity_signal(Path::new("benches/knapsack.rs"), &focus);
/// assert!(close > far);
/// ```
pub fn proximity_signal(candidate: &Path, focus_paths: &[std::path::PathBuf]) -> f32 {
    if focus_paths.is_empty() {
        return 0.0;
    }

    focus_paths
        .iter()
        .map(|focus| path_proximity(candidate, focus))
        .fold(0.0f32, f32::max)
}

fn path_proximity(a: &Path, b: &Path) -> f32 {
    let a_components: Vec<_> = a.components().collect();
    let b_components: Vec<_> = b.components().collect();

    // Count common prefix length
    let common = a_components
        .iter()
        .zip(b_components.iter())
        .take_while(|(x, y)| x == y)
        .count();

    let distance = (a_components.len() - common) + (b_components.len() - common);
    1.0 / (1.0 + distance as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recency_signal_today() {
        assert!(recency_signal(0.0) > 0.99);
    }

    #[test]
    fn test_recency_signal_half_life() {
        let s = recency_signal(30.0);
        assert!((s - 0.5).abs() < 0.01, "expected ~0.5, got {s}");
    }

    #[test]
    fn test_recency_signal_monotone() {
        let r0 = recency_signal(0.0);
        let r10 = recency_signal(10.0);
        let r100 = recency_signal(100.0);
        assert!(r0 > r10 && r10 > r100);
    }

    #[test]
    fn test_recency_signal_bounds() {
        for days in [0.0, 1.0, 30.0, 180.0, 365.0, 3650.0] {
            let s = recency_signal(days);
            assert!((0.0..=1.0).contains(&s), "out of range at {days} days: {s}");
        }
    }

    #[test]
    fn test_size_signal_zero() {
        assert!((size_signal(0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_size_signal_inflection() {
        let s = size_signal(500);
        assert!((s - 0.5).abs() < 0.01, "expected ~0.5, got {s}");
    }

    #[test]
    fn test_size_signal_monotone_decreasing() {
        assert!(size_signal(0) > size_signal(100));
        assert!(size_signal(100) > size_signal(1000));
        assert!(size_signal(1000) > size_signal(10_000));
    }

    #[test]
    fn test_size_signal_bounds() {
        for tokens in [0, 100, 500, 2000, 10_000, 100_000] {
            let s = size_signal(tokens);
            assert!((0.0..=1.0).contains(&s), "out of range at {tokens}: {s}");
        }
    }

    #[test]
    fn test_proximity_signal_same_dir() {
        use std::path::PathBuf;
        let focus = vec![PathBuf::from("src/index/mod.rs")];
        let same = proximity_signal(Path::new("src/index/discovery.rs"), &focus);
        let diff = proximity_signal(Path::new("benches/knapsack.rs"), &focus);
        assert!(
            same > diff,
            "same dir should score higher: {same} vs {diff}"
        );
    }

    #[test]
    fn test_proximity_signal_empty_focus() {
        let score = proximity_signal(Path::new("src/main.rs"), &[]);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_proximity_signal_bounds() {
        use std::path::PathBuf;
        let focus = vec![PathBuf::from("src/lib.rs")];
        let s = proximity_signal(Path::new("tests/integration/mod.rs"), &focus);
        assert!((0.0..=1.0).contains(&s));
    }
}
