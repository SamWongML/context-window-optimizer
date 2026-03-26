/// File discovery via the `ignore` crate and git metadata.
pub mod discovery;

/// Exact-match (MD5) and near-duplicate (SimHash) deduplication.
pub mod dedup;

/// Token counting via tiktoken (cl100k_base).
pub mod tokenizer;
