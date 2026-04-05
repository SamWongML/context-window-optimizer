use crate::index::simhash::find_near_duplicates;
use crate::types::FileEntry;
use md5::{Digest, Md5};
use std::collections::HashSet;

/// Compute the MD5 hash of a byte slice.
///
/// # Examples
/// ```
/// use ctx_optim::index::dedup::md5_hash;
/// let h1 = md5_hash(b"hello");
/// let h2 = md5_hash(b"hello");
/// let h3 = md5_hash(b"world");
/// assert_eq!(h1, h2);
/// assert_ne!(h1, h3);
/// ```
pub fn md5_hash(content: &[u8]) -> [u8; 16] {
    let result = Md5::digest(content);
    result.into()
}

/// Deduplicates a list of `(hash, T)` pairs, keeping the first occurrence of each hash.
///
/// Returns a tuple of `(deduped, n_removed)`.
///
/// # Examples
/// ```
/// use ctx_optim::index::dedup::dedup_by_hash;
/// let items = vec![([1u8; 16], "a"), ([1u8; 16], "b"), ([2u8; 16], "c")];
/// let (kept, removed) = dedup_by_hash(items);
/// assert_eq!(kept.len(), 2);
/// assert_eq!(removed, 1);
/// ```
pub fn dedup_by_hash<T>(items: Vec<([u8; 16], T)>) -> (Vec<T>, usize) {
    let mut seen = HashSet::new();
    let mut kept = Vec::with_capacity(items.len());
    let mut removed = 0usize;

    for (hash, item) in items {
        if seen.insert(hash) {
            kept.push(item);
        } else {
            removed += 1;
        }
    }

    (kept, removed)
}

/// Deduplicate near-duplicate files using pre-computed SimHash fingerprints.
///
/// Uses the `simhash` field on each [`FileEntry`] (computed during discovery)
/// to find groups of near-duplicates via LSH bucketing and Hamming distance
/// verification. Keeps the first occurrence from each group (by input order).
///
/// Files without a pre-computed fingerprint (`simhash == None`) are never
/// grouped as near-duplicates.
///
/// # Arguments
///
/// * `items` — File entries to deduplicate (must have `simhash` populated).
/// * `threshold` — Maximum Hamming distance to consider near-duplicate (default: 3).
///
/// # Returns
///
/// `(kept_items, removed_count)`
///
/// # Examples
///
/// ```
/// use ctx_optim::index::dedup::dedup_near_duplicates;
/// use ctx_optim::types::{FileEntry, FileMetadata};
/// use std::path::PathBuf;
/// use std::time::SystemTime;
///
/// let entry = FileEntry {
///     path: PathBuf::from("a.rs"),
///     token_count: 10,
///     hash: [0u8; 16],
///     metadata: FileMetadata {
///         size_bytes: 40,
///         last_modified: SystemTime::now(),
///         git: None,
///         language: None,
///     },
///     ast: None,
///     simhash: Some(0xAAAA_BBBB_CCCC_DDDD),
///     content: None,
/// };
/// let (kept, removed) = dedup_near_duplicates(vec![entry], 3);
/// assert_eq!(kept.len(), 1);
/// assert_eq!(removed, 0);
/// ```
pub fn dedup_near_duplicates(items: Vec<FileEntry>, threshold: u32) -> (Vec<FileEntry>, usize) {
    if items.len() < 2 {
        return (items, 0);
    }

    // Use pre-computed SimHash fingerprints from discovery.
    // Files without a fingerprint get a unique sentinel so they never match.
    // We use a hash of the index to spread sentinel bits across all 64 positions,
    // ensuring any two sentinels differ by many bits (high Hamming distance).
    let fingerprints: Vec<u64> = items
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            entry.simhash.unwrap_or_else(|| {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                i.hash(&mut hasher);
                entry.path.hash(&mut hasher);
                hasher.finish()
            })
        })
        .collect();

    let groups = find_near_duplicates(&fingerprints, threshold, 4);

    // Build set of indices to remove (keep first from each group)
    let mut remove_set = HashSet::new();
    for group in &groups {
        for &idx in &group[1..] {
            remove_set.insert(idx);
        }
    }

    let removed = remove_set.len();
    let kept: Vec<FileEntry> = items
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !remove_set.contains(i))
        .map(|(_, f)| f)
        .collect();

    (kept, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_md5_deterministic() {
        assert_eq!(md5_hash(b"test content"), md5_hash(b"test content"));
    }

    #[test]
    fn test_md5_distinct() {
        assert_ne!(md5_hash(b"foo"), md5_hash(b"bar"));
    }

    #[test]
    fn test_dedup_removes_duplicates() {
        let items = vec![
            ([1u8; 16], "first"),
            ([1u8; 16], "duplicate"),
            ([2u8; 16], "unique"),
        ];
        let (kept, removed) = dedup_by_hash(items);
        assert_eq!(kept, vec!["first", "unique"]);
        assert_eq!(removed, 1);
    }

    #[test]
    fn test_dedup_all_unique() {
        let items: Vec<([u8; 16], &str)> =
            vec![([1u8; 16], "a"), ([2u8; 16], "b"), ([3u8; 16], "c")];
        let (kept, removed) = dedup_by_hash(items);
        assert_eq!(kept.len(), 3);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_dedup_empty() {
        let items: Vec<([u8; 16], &str)> = vec![];
        let (kept, removed) = dedup_by_hash(items);
        assert!(kept.is_empty());
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_near_dedup_removes_similar_fingerprints() {
        use crate::types::FileMetadata;
        use std::time::SystemTime;

        let make_entry = |path: &str, simhash: u64| FileEntry {
            path: std::path::PathBuf::from(path),
            token_count: 100,
            hash: [0u8; 16],
            metadata: FileMetadata {
                size_bytes: 400,
                last_modified: SystemTime::now(),
                git: None,
                language: None,
            },
            ast: None,
            simhash: Some(simhash),
            content: None,
        };

        // Two entries with Hamming distance 1 (threshold 3 → grouped)
        let items = vec![
            make_entry("a.rs", 0xAAAA_BBBB_CCCC_DDDD),
            make_entry("b.rs", 0xAAAA_BBBB_CCCC_DDDE), // 1 bit different
            make_entry("c.rs", 0x1111_2222_3333_4444), // completely different
        ];
        let (kept, removed) = dedup_near_duplicates(items, 3);
        assert_eq!(removed, 1, "expected 1 near-duplicate removed");
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn test_near_dedup_no_simhash_never_grouped() {
        use crate::types::FileMetadata;
        use std::time::SystemTime;

        let make_entry = |path: &str, simhash: Option<u64>| FileEntry {
            path: std::path::PathBuf::from(path),
            token_count: 100,
            hash: [0u8; 16],
            metadata: FileMetadata {
                size_bytes: 400,
                last_modified: SystemTime::now(),
                git: None,
                language: None,
            },
            ast: None,
            simhash,
            content: None,
        };

        // Two entries without simhash should never be grouped
        let items = vec![make_entry("a.rs", None), make_entry("b.rs", None)];
        let (kept, removed) = dedup_near_duplicates(items, 3);
        assert_eq!(removed, 0, "entries without simhash should not be grouped");
        assert_eq!(kept.len(), 2);
    }
}
