/// File discovery via the `ignore` crate and git metadata.
pub mod discovery;

/// Exact-match (MD5) and near-duplicate (SimHash) deduplication.
pub mod dedup;

/// Token counting via tiktoken (cl100k_base).
pub mod tokenizer;

/// AST analysis via tree-sitter: signature extraction and import detection.
#[cfg(feature = "ast")]
pub mod ast;

/// File-level dependency graph from import statements.
pub mod depgraph;
