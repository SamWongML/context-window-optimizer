//! Test fixture helpers — create temporary repositories for integration tests.

use std::path::Path;
use tempfile::TempDir;

/// A temporary repository with a known set of files.
pub struct TempRepo {
    pub dir: TempDir,
}

impl TempRepo {
    /// Create a minimal repo with a few Rust source files.
    pub fn minimal() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        write_file(root, "src/main.rs", MAIN_RS);
        write_file(root, "src/lib.rs", LIB_RS);
        write_file(root, "src/utils.rs", UTILS_RS);
        write_file(root, "README.md", README_MD);

        Self { dir }
    }

    /// Create a repo with duplicate files (same content, different paths).
    pub fn with_duplicates() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        let content = "fn duplicate() { /* same content */ }";
        write_file(root, "src/a.rs", content);
        write_file(root, "src/b.rs", content); // exact duplicate
        write_file(root, "src/c.rs", "fn unique() {}");

        Self { dir }
    }

    /// Create a larger repo for budget/selection tests.
    pub fn larger(n_files: usize) -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        for i in 0..n_files {
            let content = format!(
                "/// File {i}\npub fn function_{i}() -> usize {{\n    {i}\n}}\n"
            );
            write_file(root, &format!("src/module_{i}.rs"), &content);
        }

        Self { dir }
    }

    /// Root path of this repo.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

fn write_file(root: &Path, rel_path: &str, content: &str) {
    let full = root.join(rel_path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).expect("create dirs");
    }
    std::fs::write(full, content).expect("write fixture file");
}

// ── Static fixture content ──

const MAIN_RS: &str = r#"
fn main() {
    println!("Context Window Optimizer");
}
"#;

const LIB_RS: &str = r#"
/// Library root.
pub mod utils;

/// Pack context for an LLM.
pub fn pack() -> Vec<String> {
    vec![]
}
"#;

const UTILS_RS: &str = r#"
/// Utility functions.

/// Compute the score of a file based on its age.
pub fn recency_score(age_days: f64) -> f64 {
    (-0.023 * age_days).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recency_score() {
        assert!(recency_score(0.0) > 0.99);
    }
}
"#;

const README_MD: &str = r#"
# Context Window Optimizer

A Rust-based MCP server that intelligently packs code context for LLM agents.
"#;
