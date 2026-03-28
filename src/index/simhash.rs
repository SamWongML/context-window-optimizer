use siphasher::sip::SipHasher13;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Compute a 64-bit SimHash fingerprint of content.
///
/// Splits content on whitespace into tokens, builds overlapping shingles
/// of `shingle_size` tokens, hashes each via SipHash-1-3, and votes on
/// each of 64 bit positions to produce the final fingerprint.
///
/// # Examples
///
/// ```
/// use ctx_optim::index::simhash::{simhash_fingerprint, hamming_distance};
///
/// let fp1 = simhash_fingerprint(b"fn main() { println!(\"hello\"); }", 3);
/// let fp2 = simhash_fingerprint(b"fn main() { println!(\"hello\"); }", 3);
/// assert_eq!(fp1, fp2); // deterministic
///
/// let fp3 = simhash_fingerprint(b"fn main() { println!(\"hi\"); }", 3);
/// assert!(hamming_distance(fp1, fp3) < 32); // similar content → low distance
/// ```
pub fn simhash_fingerprint(content: &[u8], shingle_size: usize) -> u64 {
    let shingle_size = shingle_size.max(1);

    // Tokenize on whitespace
    let text = String::from_utf8_lossy(content);
    let tokens: Vec<&str> = text.split_whitespace().collect();

    if tokens.is_empty() {
        return 0;
    }

    // If fewer tokens than shingle_size, use all tokens as one shingle
    let shingle_count = if tokens.len() >= shingle_size {
        tokens.len() - shingle_size + 1
    } else {
        1
    };

    let mut weights = [0i64; 64];

    for i in 0..shingle_count {
        let end = (i + shingle_size).min(tokens.len());
        let shingle = &tokens[i..end];

        let mut hasher = SipHasher13::new();
        for token in shingle {
            token.hash(&mut hasher);
        }
        let hash = hasher.finish();

        for (bit, w) in weights.iter_mut().enumerate() {
            if hash & (1u64 << bit) != 0 {
                *w += 1;
            } else {
                *w -= 1;
            }
        }
    }

    let mut fingerprint = 0u64;
    for (bit, w) in weights.iter().enumerate() {
        if *w > 0 {
            fingerprint |= 1u64 << bit;
        }
    }
    fingerprint
}

/// Compute the Hamming distance between two 64-bit fingerprints.
///
/// # Examples
///
/// ```
/// use ctx_optim::index::simhash::hamming_distance;
/// assert_eq!(hamming_distance(0b1010, 0b1001), 2);
/// assert_eq!(hamming_distance(0, 0), 0);
/// assert_eq!(hamming_distance(0, u64::MAX), 64);
/// ```
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Compute LSH band keys from a 64-bit fingerprint.
///
/// Divides the fingerprint into `num_bands` equal-width bands. Two
/// fingerprints sharing any band value are candidate near-duplicates.
///
/// # Examples
///
/// ```
/// use ctx_optim::index::simhash::lsh_bands;
/// let bands = lsh_bands(0xAABBCCDDEEFF0011, 4);
/// assert_eq!(bands.len(), 4);
/// ```
pub fn lsh_bands(fingerprint: u64, num_bands: usize) -> Vec<u16> {
    if num_bands == 0 {
        return vec![];
    }
    let bits_per_band = 64 / num_bands;
    (0..num_bands)
        .map(|i| {
            let shift = i * bits_per_band;
            let mask = if bits_per_band >= 64 {
                u64::MAX
            } else {
                (1u64 << bits_per_band) - 1
            };
            ((fingerprint >> shift) & mask) as u16
        })
        .collect()
}

/// Find groups of near-duplicate indices using LSH bucketing and Hamming verification.
///
/// Returns groups where each group contains indices of mutually near-duplicate items.
/// Only groups with 2+ members are returned (singletons are omitted).
///
/// Uses O(n * num_bands) time instead of O(n^2) pairwise comparison.
///
/// # Algorithm
///
/// 1. Compute LSH bands for each fingerprint
/// 2. Bucket items by (band_index, band_value) → candidate pairs
/// 3. Verify Hamming distance <= threshold for candidate pairs
/// 4. Build connected components via union-find
///
/// # Examples
///
/// ```
/// use ctx_optim::index::simhash::find_near_duplicates;
/// let fps = vec![0xAAAA_BBBB_CCCC_DDDDu64, 0xAAAA_BBBB_CCCC_DDDEu64, 0x1111_2222_3333_4444u64];
/// let groups = find_near_duplicates(&fps, 3, 4);
/// // First two are near-duplicates, third is distinct
/// assert_eq!(groups.len(), 1);
/// assert!(groups[0].contains(&0) && groups[0].contains(&1));
/// ```
pub fn find_near_duplicates(
    fingerprints: &[u64],
    threshold: u32,
    num_bands: usize,
) -> Vec<Vec<usize>> {
    let n = fingerprints.len();
    if n < 2 {
        return vec![];
    }

    // Union-find
    let mut parent: Vec<usize> = (0..n).collect();

    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]]; // path compression
            x = parent[x];
        }
        x
    }

    fn union(parent: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[rb] = ra;
        }
    }

    // LSH bucketing: (band_index, band_value) → list of item indices
    let mut buckets: HashMap<(usize, u16), Vec<usize>> = HashMap::new();

    for (idx, &fp) in fingerprints.iter().enumerate() {
        let bands = lsh_bands(fp, num_bands);
        for (band_idx, &band_val) in bands.iter().enumerate() {
            buckets.entry((band_idx, band_val)).or_default().push(idx);
        }
    }

    // Verify candidate pairs
    for items in buckets.values() {
        if items.len() < 2 {
            continue;
        }
        for i in 0..items.len() {
            for j in (i + 1)..items.len() {
                let a = items[i];
                let b = items[j];
                if find(&mut parent, a) != find(&mut parent, b)
                    && hamming_distance(fingerprints[a], fingerprints[b]) <= threshold
                {
                    union(&mut parent, a, b);
                }
            }
        }
    }

    // Build groups from union-find
    let mut groups_map: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        groups_map.entry(root).or_default().push(i);
    }

    groups_map.into_values().filter(|g| g.len() > 1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_simhash_identical_content_same_fingerprint() {
        let content = b"fn main() { println!(\"hello world\"); }";
        assert_eq!(
            simhash_fingerprint(content, 3),
            simhash_fingerprint(content, 3)
        );
    }

    #[test]
    fn test_simhash_different_content_different_fingerprint() {
        let fp1 = simhash_fingerprint(b"fn main() { let x = 1; let y = 2; }", 3);
        let fp2 = simhash_fingerprint(b"struct Config { host: String, port: u16 }", 3);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn test_simhash_near_identical_low_hamming() {
        let base = b"fn process(input: &str) -> Result<String, Error> { let data = parse(input)?; transform(data) }";
        let similar = b"fn process(input: &str) -> Result<String, Error> { let data = parse(input)?; transform_v2(data) }";
        let fp1 = simhash_fingerprint(base, 3);
        let fp2 = simhash_fingerprint(similar, 3);
        let dist = hamming_distance(fp1, fp2);
        assert!(
            dist <= 15,
            "near-identical content should have low Hamming distance, got {dist}"
        );
    }

    #[test]
    fn test_simhash_completely_different_high_hamming() {
        // Two very different pieces of content
        let fp1 = simhash_fingerprint(
            b"use std::collections::HashMap; fn build_index(files: &[File]) -> HashMap<String, Vec<usize>> { let mut idx = HashMap::new(); for (i, f) in files.iter().enumerate() { idx.entry(f.name.clone()).or_default().push(i); } idx }",
            3,
        );
        let fp2 = simhash_fingerprint(
            b"import React from 'react'; const App = () => { const [count, setCount] = useState(0); return <div><h1>Counter: {count}</h1><button onClick={() => setCount(c => c+1)}>Increment</button></div>; };",
            3,
        );
        let dist = hamming_distance(fp1, fp2);
        assert!(
            dist > 10,
            "completely different content should have high Hamming distance, got {dist}"
        );
    }

    #[test]
    fn test_simhash_empty_content() {
        assert_eq!(simhash_fingerprint(b"", 3), 0);
        assert_eq!(simhash_fingerprint(b"   ", 3), 0);
    }

    #[test]
    fn test_hamming_distance_zero_for_equal() {
        assert_eq!(hamming_distance(42, 42), 0);
        assert_eq!(hamming_distance(0, 0), 0);
        assert_eq!(hamming_distance(u64::MAX, u64::MAX), 0);
    }

    #[test]
    fn test_hamming_distance_max_for_inverted() {
        assert_eq!(hamming_distance(0, u64::MAX), 64);
    }

    #[test]
    fn test_hamming_distance_single_bit() {
        assert_eq!(hamming_distance(0b1000, 0b0000), 1);
    }

    #[test]
    fn test_lsh_bands_correct_count() {
        assert_eq!(lsh_bands(0xDEADBEEF, 4).len(), 4);
        assert_eq!(lsh_bands(0xDEADBEEF, 2).len(), 2);
        assert_eq!(lsh_bands(0xDEADBEEF, 1).len(), 1);
    }

    #[test]
    fn test_lsh_bands_zero_returns_empty() {
        assert!(lsh_bands(0xDEADBEEF, 0).is_empty());
    }

    #[test]
    fn test_find_near_duplicates_groups_similar() {
        // Two fingerprints differing by 1 bit
        let fps = vec![0xAAAA_BBBB_CCCC_DDDDu64, 0xAAAA_BBBB_CCCC_DDDEu64];
        let groups = find_near_duplicates(&fps, 3, 4);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].contains(&0));
        assert!(groups[0].contains(&1));
    }

    #[test]
    fn test_find_near_duplicates_leaves_distinct_alone() {
        let fps = vec![0x0000_0000_0000_0000u64, 0xFFFF_FFFF_FFFF_FFFFu64];
        let groups = find_near_duplicates(&fps, 3, 4);
        assert!(groups.is_empty(), "distinct fingerprints should not group");
    }

    #[test]
    fn test_find_near_duplicates_empty_input() {
        let groups = find_near_duplicates(&[], 3, 4);
        assert!(groups.is_empty());
    }

    #[test]
    fn test_find_near_duplicates_single_input() {
        let groups = find_near_duplicates(&[42], 3, 4);
        assert!(groups.is_empty());
    }

    #[test]
    fn test_find_near_duplicates_three_items_two_similar() {
        let fps = vec![
            0xAAAA_BBBB_CCCC_DDDDu64,
            0xAAAA_BBBB_CCCC_DDDEu64, // near-dup of [0]
            0x1111_2222_3333_4444u64, // distinct
        ];
        let groups = find_near_duplicates(&fps, 3, 4);
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert!(group.contains(&0) && group.contains(&1));
        assert!(!group.contains(&2));
    }

    #[test]
    fn test_find_near_duplicates_transitive_grouping() {
        // A ≈ B ≈ C but A and C might not match directly.
        // If A-B match and B-C match, all should be in one group.
        let a =
            0b0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000u64;
        let b =
            0b0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0011u64; // 2 bits from a
        let c =
            0b0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_1111u64; // 2 bits from b, 4 from a
        let fps = vec![a, b, c];
        let groups = find_near_duplicates(&fps, 3, 4);
        // a-b: dist=2, b-c: dist=2, a-c: dist=4 (> threshold)
        // But via transitivity through union-find, all grouped
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 3);
    }

    proptest! {
        #[test]
        fn prop_hamming_distance_symmetric(a: u64, b: u64) {
            prop_assert_eq!(hamming_distance(a, b), hamming_distance(b, a));
        }

        #[test]
        fn prop_hamming_distance_bounds(a: u64, b: u64) {
            let d = hamming_distance(a, b);
            prop_assert!(d <= 64);
        }

        #[test]
        fn prop_simhash_deterministic(data in proptest::collection::vec(proptest::num::u8::ANY, 0..500)) {
            let fp1 = simhash_fingerprint(&data, 3);
            let fp2 = simhash_fingerprint(&data, 3);
            prop_assert_eq!(fp1, fp2);
        }
    }
}
