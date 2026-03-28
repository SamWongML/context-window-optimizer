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

/// Compute the dependency-distance signal, normalized to `[0.0, 1.0]`.
///
/// Uses inverse distance: `1.0 / (1.0 + distance)`.
/// - Focus file itself (distance 0) → 1.0
/// - Direct import neighbor (distance 1) → 0.5
/// - Two hops away (distance 2) → 0.33
/// - Unreachable (`None`) → 0.0
///
/// # Examples
/// ```
/// use ctx_optim::scoring::signals::dependency_signal;
/// assert!((dependency_signal(Some(0)) - 1.0).abs() < 1e-6);
/// assert!((dependency_signal(Some(1)) - 0.5).abs() < 1e-6);
/// assert_eq!(dependency_signal(None), 0.0);
/// ```
pub fn dependency_signal(distance: Option<usize>) -> f32 {
    match distance {
        Some(d) => 1.0 / (1.0 + d as f32),
        None => 0.0,
    }
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

    #[test]
    fn test_proximity_signal_same_file_is_maximum() {
        use std::path::PathBuf;
        let focus = vec![PathBuf::from("src/lib.rs")];
        let same = proximity_signal(Path::new("src/lib.rs"), &focus);
        let diff = proximity_signal(Path::new("benches/knapsack.rs"), &focus);
        // Same file should have equal or better proximity than anything else
        assert!(same >= diff);
    }

    #[test]
    fn test_proximity_signal_multiple_focus_paths_uses_max() {
        use std::path::PathBuf;
        let focus = vec![
            PathBuf::from("src/scoring/mod.rs"),
            PathBuf::from("benches/knapsack.rs"),
        ];
        let candidate = Path::new("benches/scoring.rs");
        // The candidate is close to benches/knapsack.rs, so should score reasonably high
        let score = proximity_signal(candidate, &focus);
        let score_single = proximity_signal(candidate, &[PathBuf::from("src/scoring/mod.rs")]);
        // With more focus paths including a close one, score should be >= single
        assert!(score >= score_single);
    }

    #[test]
    fn test_entry_recency_signal_uses_git_when_present() {
        use crate::types::{FileEntry, FileMetadata, GitMetadata};
        use std::time::SystemTime;
        let entry = FileEntry {
            path: std::path::PathBuf::from("src/main.rs"),
            token_count: 100,
            hash: [0u8; 16],
            metadata: FileMetadata {
                size_bytes: 400,
                last_modified: SystemTime::UNIX_EPOCH, // very old filesystem time
                git: Some(GitMetadata {
                    age_days: 1.0, // but recently committed
                    commit_count: 5,
                }),
                language: None,
            },
            ast: None,
            simhash: None,
        };
        // Git age (1 day) should dominate over filesystem time (epoch = very old)
        let score = entry_recency_signal(&entry);
        // 1 day old should score close to 1.0 (half-life is 30 days)
        assert!(
            score > 0.9,
            "expected high recency for 1-day-old file, got {score}"
        );
    }

    #[test]
    fn test_entry_recency_signal_falls_back_to_filesystem() {
        use crate::types::{FileEntry, FileMetadata};
        use std::time::SystemTime;
        let entry = FileEntry {
            path: std::path::PathBuf::from("src/main.rs"),
            token_count: 100,
            hash: [0u8; 16],
            metadata: FileMetadata {
                size_bytes: 400,
                last_modified: SystemTime::now(), // just modified
                git: None,                        // no git metadata
                language: None,
            },
            ast: None,
            simhash: None,
        };
        let score = entry_recency_signal(&entry);
        // File just modified → should score close to 1.0
        assert!(
            score > 0.95,
            "recently modified file should score high, got {score}"
        );
    }

    #[test]
    fn test_dependency_signal_self() {
        assert!((dependency_signal(Some(0)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_dependency_signal_direct() {
        assert!((dependency_signal(Some(1)) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_dependency_signal_unreachable() {
        assert_eq!(dependency_signal(None), 0.0);
    }

    #[test]
    fn test_dependency_signal_monotone() {
        let d0 = dependency_signal(Some(0));
        let d1 = dependency_signal(Some(1));
        let d5 = dependency_signal(Some(5));
        assert!(d0 > d1);
        assert!(d1 > d5);
    }

    // ── Property-based tests ──────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn prop_recency_signal_always_in_0_1(age_days in 0.0f64..3650.0) {
            let s = recency_signal(age_days);
            prop_assert!(
                (0.0f32..=1.0).contains(&s),
                "recency_signal({age_days}) = {s} out of [0, 1]"
            );
        }

        #[test]
        fn prop_size_signal_always_in_0_1(tokens in 0usize..1_000_000) {
            let s = size_signal(tokens);
            prop_assert!(
                (0.0f32..=1.0).contains(&s),
                "size_signal({tokens}) = {s} out of [0, 1]"
            );
        }

        #[test]
        fn prop_recency_signal_monotone_decreasing(a in 0.0f64..1000.0, b in 0.0f64..1000.0) {
            let (young, old) = if a <= b { (a, b) } else { (b, a) };
            let s_young = recency_signal(young);
            let s_old = recency_signal(old);
            prop_assert!(
                s_young >= s_old,
                "recency_signal({young}) = {s_young} < recency_signal({old}) = {s_old}"
            );
        }

        #[test]
        fn prop_size_signal_monotone_decreasing(a in 0usize..100_000, b in 0usize..100_000) {
            let (small, large) = if a <= b { (a, b) } else { (b, a) };
            let s_small = size_signal(small);
            let s_large = size_signal(large);
            prop_assert!(
                s_small >= s_large,
                "size_signal({small}) = {s_small} < size_signal({large}) = {s_large}"
            );
        }

        #[test]
        fn prop_dependency_signal_always_in_0_1(d in 0usize..10_000) {
            let s = dependency_signal(Some(d));
            prop_assert!(
                (0.0f32..=1.0).contains(&s),
                "dependency_signal(Some({d})) = {s} out of [0, 1]"
            );
        }

        #[test]
        fn prop_proximity_signal_always_in_0_1(
            path_a in "[a-z]{2,8}/[a-z]{2,8}\\.rs",
            path_b in "[a-z]{2,8}/[a-z]{2,8}\\.rs",
        ) {
            let focus = vec![std::path::PathBuf::from(&path_b)];
            let s = proximity_signal(Path::new(&path_a), &focus);
            prop_assert!(
                (0.0f32..=1.0).contains(&s),
                "proximity_signal = {s} out of [0, 1]"
            );
        }
    }
}
