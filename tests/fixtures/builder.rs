/// git2-backed temporary repository builder for integration tests.
///
/// Provides a fluent builder API for creating realistic test repositories with
/// controlled commit history, file structures, and language mixes.
use std::path::{Path, PathBuf};

use git2::{Repository, Signature, Time};
use rand::prelude::*;
use rand::rngs::StdRng;
use tempfile::TempDir;

use super::write_file;

// Re-export content module types when available.
#[allow(unused_imports)]
use super::content::{self, FileSize};

// ── Public enums ─────────────────────────────────────────────────────────────

/// Source language for generated files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    TypeScript,
    Python,
    Go,
}

impl Lang {
    /// File extension for this language.
    fn extension(self) -> &'static str {
        match self {
            Lang::Rust => "rs",
            Lang::TypeScript => "ts",
            Lang::Python => "py",
            Lang::Go => "go",
        }
    }
}

/// Strategy for distributing commits across files over time.
#[derive(Debug, Clone)]
pub enum CommitPattern {
    /// 80% of commits touch hot_paths; the remaining 20% touch other files.
    Hotspot {
        hot_paths: Vec<String>,
        total_commits: usize,
        span_days: u64,
    },
    /// Commits distributed evenly across all files.
    Uniform {
        total_commits: usize,
        span_days: u64,
    },
    /// An old initial commit followed by a recent burst on specific files.
    Stale {
        initial_age_days: u64,
        burst_paths: Vec<String>,
        burst_age_days: u64,
    },
}

// ── RealisticRepo (output handle) ────────────────────────────────────────────

/// A temporary git repository created by [`RealisticRepoBuilder`].
pub struct RealisticRepo {
    /// The temporary directory backing this repo (dropped on `RealisticRepo` drop).
    dir: TempDir,
}

impl RealisticRepo {
    /// Path to the root of the repository.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

// ── Internal pending-file record ─────────────────────────────────────────────

struct PendingFile {
    rel_path: String,
    content: String,
}

// ── RealisticRepoBuilder ──────────────────────────────────────────────────────

/// Fluent builder for realistic test git repositories.
///
/// # Examples
///
/// ```no_run
/// use tests::fixtures::builder::{RealisticRepoBuilder, Lang, CommitPattern};
///
/// let repo = RealisticRepoBuilder::new()
///     .seed(42)
///     .add_module("src", Lang::Rust, 5)
///     .initial_commit(30)
///     .commit_pattern(CommitPattern::Uniform { total_commits: 10, span_days: 20 })
///     .build();
/// ```
pub struct RealisticRepoBuilder {
    rng: StdRng,
    files: Vec<PendingFile>,
    initial_commit_days_ago: u64,
    patterns: Vec<CommitPattern>,
    recent_edits: Vec<(Vec<String>, u64)>,
}

impl RealisticRepoBuilder {
    /// Create a new builder with a random seed.
    pub fn new() -> Self {
        Self {
            rng: StdRng::from_entropy(),
            files: Vec::new(),
            initial_commit_days_ago: 90,
            patterns: Vec::new(),
            recent_edits: Vec::new(),
        }
    }

    /// Set a deterministic seed for reproducible repositories.
    pub fn seed(mut self, seed: u64) -> Self {
        self.rng = StdRng::seed_from_u64(seed);
        self
    }

    /// Add a single auto-generated file.
    pub fn add_file(mut self, path: impl Into<String>, lang: Lang, size: FileSize) -> Self {
        let path = path.into();
        let content = generate_file_content(&path, lang, size);
        self.files.push(PendingFile {
            rel_path: path,
            content,
        });
        self
    }

    /// Add a file with explicit content.
    pub fn add_file_with_content(
        mut self,
        path: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        self.files.push(PendingFile {
            rel_path: path.into(),
            content: content.into(),
        });
        self
    }

    /// Generate `count` source files inside `dir`.
    pub fn add_module(mut self, dir: impl Into<String>, lang: Lang, count: usize) -> Self {
        let dir = dir.into();
        for i in 0..count {
            let rel_path = format!("{}/{}.{}", dir, module_name(lang, i), lang.extension());
            let content = generate_file_content(&rel_path, lang, FileSize::Medium);
            self.files.push(PendingFile { rel_path, content });
        }
        self
    }

    /// Add `count` files with identical content (exact duplicates) inside `dir`.
    pub fn add_exact_duplicates(mut self, dir: impl Into<String>, count: usize) -> Self {
        let dir = dir.into();
        let shared_content = "fn duplicate_function() {\n    // Identical content across all duplicate files.\n    let x = 42;\n    let _ = x;\n}\n".to_string();
        for i in 0..count {
            let rel_path = format!("{}/dup_{i}.rs", dir);
            self.files.push(PendingFile {
                rel_path,
                content: shared_content.clone(),
            });
        }
        self
    }

    /// Add `count` files where a `similarity` fraction of lines are shared.
    ///
    /// `similarity` is in [0.0, 1.0]. A value of 0.9 means 90% of lines are
    /// identical; the rest are varied per file.
    pub fn add_near_duplicates(
        mut self,
        dir: impl Into<String>,
        count: usize,
        similarity: f64,
    ) -> Self {
        let dir = dir.into();
        let similarity = similarity.clamp(0.0, 1.0);

        // Build shared lines and per-file unique lines.
        let total_lines = 30usize;
        let shared_count = (total_lines as f64 * similarity).round() as usize;
        let unique_count = total_lines.saturating_sub(shared_count);

        let shared_lines: Vec<String> = (0..shared_count)
            .map(|i| format!("    let shared_{i}: u32 = {i};"))
            .collect();

        for file_idx in 0..count {
            let unique_lines: Vec<String> = (0..unique_count)
                .map(|j| {
                    format!(
                        "    let unique_f{file_idx}_l{j}: u32 = {};",
                        file_idx * 100 + j
                    )
                })
                .collect();

            let mut all_lines = Vec::with_capacity(total_lines + 2);
            all_lines.push(format!("fn near_dup_{file_idx}() {{"));
            all_lines.extend(shared_lines.iter().cloned());
            all_lines.extend(unique_lines);
            all_lines.push("}".to_string());

            let rel_path = format!("{}/near_dup_{file_idx}.rs", dir);
            self.files.push(PendingFile {
                rel_path,
                content: all_lines.join("\n") + "\n",
            });
        }
        self
    }

    /// Add `count` empty files (zero bytes).
    pub fn add_empty_files(mut self, count: usize) -> Self {
        for i in 0..count {
            self.files.push(PendingFile {
                rel_path: format!("empty/empty_{i}.rs"),
                content: String::new(),
            });
        }
        self
    }

    /// Add a large file that reaches approximately `approx_tokens` by repeating
    /// medium-sized blocks.
    pub fn add_large_file(mut self, path: impl Into<String>, approx_tokens: usize) -> Self {
        let path = path.into();
        // Rough heuristic: ~4 chars per token, medium block ~200 tokens.
        let block_tokens = 200usize;
        let blocks_needed = (approx_tokens / block_tokens).max(1);
        let block = generate_large_block();
        let mut content = String::with_capacity(block.len() * blocks_needed);
        for i in 0..blocks_needed {
            content.push_str(&format!("// Block {i}\n"));
            content.push_str(&block);
        }
        self.files.push(PendingFile {
            rel_path: path,
            content,
        });
        self
    }

    /// Create files in a deeply nested directory structure.
    ///
    /// Creates `files_per_level` files at each depth level from 1 to `depth`.
    pub fn add_deep_nesting(
        mut self,
        base: impl Into<String>,
        files_per_level: usize,
        depth: usize,
    ) -> Self {
        let base = base.into();
        let mut current_path = base;
        for level in 0..depth {
            for file_idx in 0..files_per_level {
                let rel_path = format!("{}/level{}_{}.rs", current_path, level, file_idx);
                let content = format!(
                    "// Level {} file {}\npub fn fn_l{level}_{file_idx}() {{}}\n",
                    level, file_idx
                );
                self.files.push(PendingFile { rel_path, content });
            }
            current_path = format!("{}/nested_{}", current_path, level);
        }
        self
    }

    /// Add a standard set of non-code files: README, config, docs.
    pub fn add_non_code_files(mut self) -> Self {
        self.files.push(PendingFile {
            rel_path: "README.md".to_string(),
            content: "# Test Repository\n\nA test repository generated by RealisticRepoBuilder.\n"
                .to_string(),
        });
        self.files.push(PendingFile {
            rel_path: "Cargo.toml".to_string(),
            content: "[package]\nname = \"test-repo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
                .to_string(),
        });
        self.files.push(PendingFile {
            rel_path: "docs/guide.md".to_string(),
            content: "# Guide\n\nUsage documentation.\n".to_string(),
        });
        self.files.push(PendingFile {
            rel_path: ".github/workflows/ci.yml".to_string(),
            content: "name: CI\non: [push]\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n".to_string(),
        });
        self
    }

    /// Set how many days ago the initial commit should be dated.
    pub fn initial_commit(mut self, days_ago: u64) -> Self {
        self.initial_commit_days_ago = days_ago;
        self
    }

    /// Append a commit pattern that will be replayed during `build()`.
    pub fn commit_pattern(mut self, pattern: CommitPattern) -> Self {
        self.patterns.push(pattern);
        self
    }

    /// Mark specific files as recently edited at `days_ago` days before now.
    pub fn recent_edits(mut self, paths: Vec<String>, days_ago: u64) -> Self {
        self.recent_edits.push((paths, days_ago));
        self
    }

    /// Write all files, initialise a git repo, and replay commit history.
    ///
    /// Returns a [`RealisticRepo`] handle wrapping the temporary directory.
    pub fn build(mut self) -> RealisticRepo {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // 1. Write files to disk.
        for file in &self.files {
            write_file(root, &file.rel_path, &file.content);
        }

        // 2. Init git repo.
        let repo = Repository::init(root).expect("git init");
        configure_git_repo(&repo);

        // 3. Initial commit (backdated).
        let initial_epoch = days_ago_to_epoch(self.initial_commit_days_ago);
        git_add_all_and_commit(&repo, root, "Initial commit", initial_epoch);

        // Collect all file paths for use in patterns.
        let all_paths: Vec<String> = self.files.iter().map(|f| f.rel_path.clone()).collect();

        // 4. Execute commit patterns.
        for pattern in &self.patterns {
            match pattern {
                CommitPattern::Hotspot {
                    hot_paths,
                    total_commits,
                    span_days,
                } => {
                    for i in 0..*total_commits {
                        // Space commits evenly, most recent first (i=0 is most recent).
                        let age_days = if *total_commits <= 1 {
                            0
                        } else {
                            (*span_days * i as u64) / (*total_commits as u64 - 1)
                        };
                        let epoch = days_ago_to_epoch(age_days);

                        // 80% chance: pick from hot_paths; 20%: pick from rest.
                        let use_hot: f64 = self.rng.r#gen();
                        let target = if use_hot < 0.8 && !hot_paths.is_empty() {
                            let idx = self.rng.gen_range(0..hot_paths.len());
                            hot_paths[idx].clone()
                        } else {
                            let cold: Vec<&String> = all_paths
                                .iter()
                                .filter(|p| !hot_paths.contains(p))
                                .collect();
                            if cold.is_empty() {
                                // Fall back to hot path if no cold files exist.
                                let idx = self.rng.gen_range(0..hot_paths.len().max(1));
                                hot_paths.get(idx).cloned().unwrap_or_else(|| {
                                    all_paths.first().cloned().unwrap_or_default()
                                })
                            } else {
                                let idx = self.rng.gen_range(0..cold.len());
                                cold[idx].clone()
                            }
                        };

                        if !target.is_empty() {
                            append_comment(root, &target, i);
                            git_add_and_commit(
                                &repo,
                                root,
                                &target,
                                &format!("hotspot: update {}", &target),
                                epoch,
                            );
                        }
                    }
                }

                CommitPattern::Uniform {
                    total_commits,
                    span_days,
                } => {
                    if all_paths.is_empty() {
                        continue;
                    }
                    for i in 0..*total_commits {
                        let age_days = if *total_commits <= 1 {
                            0
                        } else {
                            (*span_days * i as u64) / (*total_commits as u64 - 1)
                        };
                        let epoch = days_ago_to_epoch(age_days);

                        let idx = self.rng.gen_range(0..all_paths.len());
                        let target = all_paths[idx].clone();
                        append_comment(root, &target, i);
                        git_add_and_commit(
                            &repo,
                            root,
                            &target,
                            &format!("uniform: update {}", &target),
                            epoch,
                        );
                    }
                }

                CommitPattern::Stale {
                    initial_age_days: _,
                    burst_paths,
                    burst_age_days,
                } => {
                    // Initial commit is already placed at initial_age_days.
                    // Now commit each burst_path at burst_age_days.
                    let epoch = days_ago_to_epoch(*burst_age_days);
                    for (i, path) in burst_paths.iter().enumerate() {
                        if root.join(path).exists() {
                            append_comment(root, path, i);
                            git_add_and_commit(
                                &repo,
                                root,
                                path,
                                &format!("burst: update {}", path),
                                epoch,
                            );
                        }
                    }
                }
            }
        }

        // 5. Apply recent_edits.
        for (paths, days_ago) in &self.recent_edits {
            let epoch = days_ago_to_epoch(*days_ago);
            for (i, path) in paths.iter().enumerate() {
                if root.join(path).exists() {
                    append_comment(root, path, i + 1000);
                    git_add_and_commit(&repo, root, path, &format!("edit: update {}", path), epoch);
                }
            }
        }

        RealisticRepo { dir }
    }
}

impl Default for RealisticRepoBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ── Content generation helpers ────────────────────────────────────────────────

/// Generate plausible source content for a file.
fn generate_file_content(path: &str, lang: Lang, _size: FileSize) -> String {
    let stem = PathBuf::from(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module")
        .to_string();

    // Sanitise stem so it's a valid identifier.
    let ident: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    match lang {
        Lang::Rust => format!(
            "/// Module `{ident}`.\npub fn {ident}_main() -> u32 {{\n    let value: u32 = 42;\n    value\n}}\n\n#[cfg(test)]\nmod tests {{\n    use super::*;\n    #[test]\n    fn test_{ident}() {{\n        assert_eq!({ident}_main(), 42);\n    }}\n}}\n"
        ),
        Lang::TypeScript => format!(
            "/**\n * Module {ident}.\n */\nexport function {ident}Main(): number {{\n    const value: number = 42;\n    return value;\n}}\n\nexport default {ident}Main;\n"
        ),
        Lang::Python => format!(
            "\"\"\"Module {ident}.\"\"\"\n\n\ndef {ident}_main() -> int:\n    \"\"\"Return the answer.\"\"\"\n    value: int = 42\n    return value\n\n\nif __name__ == \"__main__\":\n    print({ident}_main())\n"
        ),
        Lang::Go => format!(
            "// Package {ident} provides utilities.\npackage {ident}\n\n// {ident}Main returns the answer.\nfunc {ident}Main() uint32 {{\n\treturn 42\n}}\n"
        ),
    }
}

/// Generate a large repeatable block of ~200 tokens.
fn generate_large_block() -> String {
    r#"pub fn large_block_function(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    for &byte in input {
        let transformed = match byte {
            0..=31 => byte.wrapping_add(32),
            127..=159 => byte.wrapping_sub(32),
            _ => byte,
        };
        output.push(transformed);
    }
    output
}

pub struct LargeBlockStruct {
    data: Vec<u8>,
    capacity: usize,
    offset: usize,
}

impl LargeBlockStruct {
    pub fn new(capacity: usize) -> Self {
        Self { data: Vec::with_capacity(capacity), capacity, offset: 0 }
    }

    pub fn push(&mut self, value: u8) -> bool {
        if self.data.len() < self.capacity {
            self.data.push(value);
            self.offset += 1;
            true
        } else {
            false
        }
    }
}

"#
    .to_string()
}

/// Generate a language-specific module file name for index `i`.
fn module_name(lang: Lang, i: usize) -> String {
    match lang {
        Lang::Rust => format!("module_{i}"),
        Lang::TypeScript => format!("module{i}"),
        Lang::Python => format!("module_{i}"),
        Lang::Go => format!("module{i}"),
    }
}

// ── Git helpers ───────────────────────────────────────────────────────────────

/// Configure the git repo with test author identity.
fn configure_git_repo(repo: &Repository) {
    let mut config = repo.config().expect("open git config");
    config
        .set_str("user.name", "Test Author")
        .expect("set user.name");
    config
        .set_str("user.email", "test@example.com")
        .expect("set user.email");
}

/// Convert a "days ago" offset to a Unix epoch timestamp.
fn days_ago_to_epoch(days_ago: u64) -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX epoch")
        .as_secs() as i64;
    now - (days_ago as i64 * 86_400)
}

/// Create a signature backdated to `epoch_secs`.
fn make_sig(epoch_secs: i64) -> Signature<'static> {
    let time = Time::new(epoch_secs, 0);
    Signature::new("Test Author", "test@example.com", &time).expect("create signature")
}

/// Stage all files and create the initial commit (no parent).
fn git_add_all_and_commit(repo: &Repository, _root: &Path, message: &str, epoch_secs: i64) {
    let mut index = repo.index().expect("get index");
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("git add -A");
    index.write().expect("write index");

    let tree_oid = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_oid).expect("find tree");
    let sig = make_sig(epoch_secs);

    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
        .expect("initial commit");
}

/// Stage a single file and create a commit with a parent.
fn git_add_and_commit(
    repo: &Repository,
    root: &Path,
    rel_path: &str,
    message: &str,
    epoch_secs: i64,
) {
    let _ = root; // root used via repo workdir
    let mut index = repo.index().expect("get index");
    index.add_path(Path::new(rel_path)).unwrap_or_else(|_| {
        // If add_path fails (e.g. file doesn't exist in workdir), try add_all.
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .expect("git add -A fallback");
    });
    index.write().expect("write index");

    let tree_oid = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_oid).expect("find tree");
    let sig = make_sig(epoch_secs);

    let parent_commit = {
        let head = match repo.head() {
            Ok(h) => h,
            Err(_) => {
                // No HEAD yet; fall back to initial-style commit.
                repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
                    .expect("fallback initial commit");
                return;
            }
        };
        head.peel_to_commit().expect("peel HEAD to commit")
    };

    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent_commit])
        .expect("commit");
}

/// Append a language-appropriate comment to a file so it becomes a new commit.
fn append_comment(root: &Path, rel_path: &str, seq: usize) {
    let full = root.join(rel_path);
    if !full.exists() {
        return;
    }
    let ext = full.extension().and_then(|e| e.to_str()).unwrap_or("");

    let comment = match ext {
        "rs" | "ts" | "go" | "js" => format!("// edit {seq}\n"),
        "py" | "yaml" | "yml" | "toml" => format!("# edit {seq}\n"),
        "md" => format!("<!-- edit {seq} -->\n"),
        "css" => format!("/* edit {seq} */\n"),
        "json" => return, // JSON doesn't support comments
        _ => format!("// edit {seq}\n"),
    };

    let mut existing = std::fs::read_to_string(&full).unwrap_or_default();
    existing.push_str(&comment);
    std::fs::write(&full, existing).expect("append comment");
}
