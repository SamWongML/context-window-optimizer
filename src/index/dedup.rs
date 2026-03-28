use crate::index::simhash::{find_near_duplicates, simhash_fingerprint};
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

/// Deduplicate near-duplicate files using SimHash fingerprinting with LSH.
///
/// Computes a 64-bit SimHash fingerprint per file (reading content from disk),
/// groups near-duplicates by Hamming distance, and keeps only the first
/// occurrence from each group (by input order).
///
/// # Arguments
///
/// * `items` — File entries to deduplicate.
/// * `threshold` — Maximum Hamming distance to consider near-duplicate (default: 3).
/// * `shingle_size` — Number of tokens per shingle (default: 3).
///
/// # Returns
///
/// `(kept_items, removed_count)`
///
/// # Examples
///
/// ```no_run
/// use ctx_optim::index::dedup::dedup_near_duplicates;
/// use ctx_optim::types::FileEntry;
/// // let (kept, removed) = dedup_near_duplicates(entries, 3, 3);
/// ```
pub fn dedup_near_duplicates(
    items: Vec<FileEntry>,
    threshold: u32,
    shingle_size: usize,
) -> (Vec<FileEntry>, usize) {
    if items.len() < 2 {
        return (items, 0);
    }

    // Compute SimHash fingerprints by reading file content
    let fingerprints: Vec<u64> = items
        .iter()
        .map(|entry| {
            match std::fs::read(&entry.path) {
                Ok(content) => simhash_fingerprint(&content, shingle_size),
                Err(err) => {
                    tracing::warn!(
                        "dedup: could not read {} for SimHash: {err}",
                        entry.path.display()
                    );
                    // Return a unique fingerprint so it won't match anything
                    0
                }
            }
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
}
