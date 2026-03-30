// Test fixture helpers — create temporary repositories for integration tests.

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

    /// Create a repo with near-duplicate files (very similar content, small differences).
    pub fn with_near_duplicates() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // These files share ~95% identical content — only one variable name differs.
        // The more shared tokens, the lower the SimHash Hamming distance.
        let base = r#"
use std::collections::HashMap;

pub fn process_data(input: &str) -> Result<String, Box<dyn std::error::Error>> {
    let trimmed = input.trim();
    let parsed: Vec<&str> = trimmed.split(',').collect();
    let mut result = String::new();
    for item in &parsed {
        if !item.is_empty() {
            result.push_str(item.trim());
            result.push('\n');
        }
    }
    Ok(result)
}

pub fn validate(input: &str) -> bool {
    !input.is_empty() && input.len() < 1000
}

pub fn count_items(input: &str) -> usize {
    input.split(',').filter(|s| !s.is_empty()).count()
}
"#;
        let variant = r#"
use std::collections::HashMap;

pub fn process_data(input: &str) -> Result<String, Box<dyn std::error::Error>> {
    let trimmed = input.trim();
    let parsed: Vec<&str> = trimmed.split(',').collect();
    let mut result = String::new();
    for item in &parsed {
        if !item.is_empty() {
            result.push_str(item.trim());
            result.push('\n');
        }
    }
    Ok(result)
}

pub fn validate(value: &str) -> bool {
    !value.is_empty() && value.len() < 1000
}

pub fn count_items(input: &str) -> usize {
    input.split(',').filter(|s| !s.is_empty()).count()
}
"#;

        write_file(root, "src/processor_a.rs", base);
        write_file(root, "src/processor_b.rs", variant); // near-duplicate
        write_file(root, "src/unique.rs", "pub fn unique_function() -> u32 { 42 }");

        Self { dir }
    }

    /// Create a repo with files spread across multiple directories.
    pub fn with_directory_structure() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // 3 files in src/scoring/
        write_file(root, "src/scoring/signals.rs", "pub fn recency() -> f32 { 0.9 }");
        write_file(root, "src/scoring/weights.rs", "pub fn default_weights() -> f32 { 0.5 }");
        write_file(root, "src/scoring/mod.rs", "pub mod signals;\npub mod weights;");

        // 3 files in src/index/
        write_file(root, "src/index/discovery.rs", "pub fn discover() -> Vec<String> { vec![] }");
        write_file(root, "src/index/tokenizer.rs", "pub fn count_tokens() -> usize { 0 }");
        write_file(root, "src/index/mod.rs", "pub mod discovery;\npub mod tokenizer;");

        // 2 files in tests/
        write_file(root, "tests/test_scoring.rs", "fn test_score() { assert!(true); }");
        write_file(root, "tests/test_index.rs", "fn test_index() { assert!(true); }");

        Self { dir }
    }

    /// Create a repo with exactly one source file.
    pub fn single_file() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        write_file(dir.path(), "src/lib.rs", LIB_RS);
        Self { dir }
    }

    /// Create a repo where every file has identical content (all are exact duplicates).
    pub fn all_duplicates(n: usize) -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let content = "fn duplicated_content() { /* identical */ }\n";
        for i in 0..n {
            write_file(dir.path(), &format!("src/dup_{i}.rs"), content);
        }
        Self { dir }
    }

    /// Create a repo with a mix of small source files and one very large file.
    /// The large file is named `huge_generated.rs` and is designed to exceed
    /// typical `max_file_tokens` limits.
    pub fn with_oversized_files() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // Normal-sized files
        write_file(root, "src/small_a.rs", "pub fn a() -> u32 { 1 }");
        write_file(root, "src/small_b.rs", "pub fn b() -> u32 { 2 }");

        // A huge file: repeat a function body ~10 000 times to blow past token limits
        let repeated = "pub fn generated_fn() -> u64 { 42 }\n".repeat(300);
        let huge = format!("// auto-generated — do not edit\n{repeated}");
        write_file(root, "src/huge_generated.rs", &huge);

        Self { dir }
    }

    /// Create a repo where files are clearly split into a "core" module and
    /// "utils" module.  Used to verify that focus-path ranking drives selection
    /// toward the focused module.
    pub fn with_focus_relevance() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // Core module — files we want to focus on
        write_file(root, "src/core/logic.rs",
            "pub fn core_logic(x: u64) -> u64 { x * 2 }\npub fn helper(x: u64) -> u64 { x + 1 }");
        write_file(root, "src/core/types.rs",
            "pub struct CoreData { pub value: u64 }");
        write_file(root, "src/core/mod.rs",
            "pub mod logic;\npub mod types;");

        // Utils module — unrelated to core focus
        write_file(root, "src/utils/string.rs",
            "pub fn trim_ws(s: &str) -> &str { s.trim() }");
        write_file(root, "src/utils/math.rs",
            "pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 { x.max(lo).min(hi) }");
        write_file(root, "src/utils/mod.rs",
            "pub mod string;\npub mod math;");

        // Top-level files
        write_file(root, "src/lib.rs",
            "pub mod core;\npub mod utils;");
        write_file(root, "README.md",
            "# Focus Relevance Test Fixture\n");

        Self { dir }
    }

    /// Create a repo with source files in multiple programming languages.
    pub fn with_mixed_languages() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        write_file(root, "src/main.rs",
            "fn main() { println!(\"hello\"); }");
        write_file(root, "src/utils.ts",
            "export function add(a: number, b: number): number { return a + b; }");
        write_file(root, "src/helper.py",
            "def greet(name: str) -> str:\n    return f\"Hello, {name}\"");
        write_file(root, "src/server.go",
            "package main\nimport \"fmt\"\nfunc main() { fmt.Println(\"hello\") }");
        write_file(root, "README.md",
            "# Multi-language fixture\n");

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
