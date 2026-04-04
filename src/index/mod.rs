/// File discovery via the `ignore` crate and git metadata.
pub mod discovery;

/// Exact-match (MD5) and near-duplicate (SimHash) deduplication.
pub mod dedup;

/// SimHash fingerprinting for near-duplicate detection.
pub mod simhash;

/// Token counting via bpe-openai (cl100k_base) and fast byte-class estimation.
pub mod tokenizer;

/// AST analysis via tree-sitter: signature extraction and import detection.
#[cfg(feature = "ast")]
pub mod ast;

/// File-level dependency graph from import statements.
pub mod depgraph;
