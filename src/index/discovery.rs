use crate::{
    config::Config,
    error::OptimError,
    index::{dedup::md5_hash, simhash::simhash_fingerprint, tokenizer::Tokenizer},
    types::{FileEntry, FileMetadata, GitMetadata, Language},
};
use ignore::WalkBuilder;
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Options controlling which files to discover.
///
/// # Examples
///
/// ```no_run
/// use ctx_optim::{config::Config, index::discovery::DiscoveryOptions};
///
/// let config = Config::default();
/// let opts = DiscoveryOptions::from_config(&config, ".");
/// assert!(!opts.compute_simhash); // off by default (near-dedup disabled)
/// ```
#[derive(Debug, Clone)]
pub struct DiscoveryOptions {
    /// Root directory to walk.
    pub root: PathBuf,
    /// Paths to explicitly exclude (in addition to .gitignore).
    pub extra_ignore: Vec<PathBuf>,
    /// Maximum file size to index (bytes).
    pub max_file_bytes: u64,
    /// Maximum token count per file.
    pub max_file_tokens: usize,
    /// If non-empty, only files with these extensions are included.
    pub include_extensions: Vec<String>,
    /// Maximum file size for AST parsing (bytes). Files larger skip AST analysis.
    pub max_ast_bytes: usize,
    /// Whether to compute SimHash fingerprints during discovery (for near-dedup).
    pub compute_simhash: bool,
    /// Number of tokens per shingle for SimHash fingerprinting.
    pub shingle_size: usize,
}

impl DiscoveryOptions {
    /// Build `DiscoveryOptions` from a `Config` and a root path.
    pub fn from_config(config: &Config, root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            extra_ignore: config.extra_ignore.clone(),
            max_file_bytes: config.max_file_bytes,
            max_file_tokens: config.max_file_tokens,
            include_extensions: config.include_extensions.clone(),
            max_ast_bytes: config.max_ast_bytes,
            compute_simhash: config.dedup.near,
            shingle_size: config.dedup.shingle_size,
        }
    }
}

/// Attempt to read Git metadata (age and commit count) for a file.
///
/// Counts commits that actually **modified** the file (blob OID changed from parent),
/// not all commits where the file existed. This gives a correct recency signal for
/// scoring — a file last modified 6 months ago will score lower than one touched
/// yesterday, even if both are tracked in HEAD.
///
/// Returns `None` if the file is not in a git repository or git is unavailable.
fn read_git_metadata(repo: &git2::Repository, path: &Path) -> Option<GitMetadata> {
    let relative = path
        .strip_prefix(repo.workdir().unwrap_or_else(|| repo.path()))
        .ok()?;

    let mut revwalk = repo.revwalk().ok()?;
    revwalk.push_head().ok()?;

    let mut commit_count = 0u32;
    let mut latest_time: Option<i64> = None;
    // Cap total commits traversed (not just modifications) to bound latency on large repos.
    let mut commits_visited = 0u32;
    const MAX_COMMITS: u32 = 2_000;
    const MAX_MODIFICATIONS: u32 = 200;

    for oid in revwalk.flatten() {
        commits_visited += 1;
        if commits_visited > MAX_COMMITS {
            break;
        }

        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let tree = match commit.tree() {
            Ok(t) => t,
            Err(_) => continue,
        };

        let current_oid = tree.get_path(relative).ok().map(|e| e.id());

        // Compare to the first parent's blob OID to detect whether this commit
        // actually changed the file (rather than just passing it through).
        let parent_oid = commit
            .parent(0)
            .ok()
            .and_then(|p| p.tree().ok())
            .and_then(|pt| pt.get_path(relative).ok())
            .map(|e| e.id());

        let changed = match (current_oid, parent_oid) {
            (Some(cur), Some(par)) => cur != par, // content changed
            (Some(_), None) => true,              // file introduced in this commit
            _ => false,                           // file absent or only in parent (deletion)
        };

        if changed {
            commit_count += 1;
            if latest_time.is_none() {
                latest_time = Some(commit.time().seconds());
            }
            if commit_count >= MAX_MODIFICATIONS {
                break;
            }
        }
    }

    if commit_count == 0 {
        return None;
    }

    let age_days = latest_time.map(|t| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let diff_secs = (now - t).max(0);
        diff_secs as f64 / 86_400.0
    })?;

    Some(GitMetadata {
        age_days,
        commit_count,
    })
}

/// Walk a directory and collect `FileEntry` records.
///
/// - Respects `.gitignore` and `.ignore` files via the `ignore` crate.
/// - Skips files exceeding `max_file_bytes`.
/// - Hashes content with MD5 (for dedup).
/// - Counts tokens using `tiktoken`.
/// - Extracts git metadata when available.
///
/// # Errors
/// Returns `OptimError::EmptyRepo` if no files pass all filters.
///
/// # Examples
/// ```no_run
/// use ctx_optim::{config::Config, index::discovery::{DiscoveryOptions, discover_files}};
/// let opts = DiscoveryOptions::from_config(&Config::default(), ".");
/// let files = discover_files(&opts).unwrap();
/// println!("found {} files", files.len());
/// ```
pub fn discover_files(opts: &DiscoveryOptions) -> Result<Vec<FileEntry>, OptimError> {
    let tokenizer = Tokenizer::new()?;

    // Try to open git repo for metadata enrichment
    let repo = git2::Repository::discover(&opts.root).ok();
    if repo.is_some() {
        tracing::debug!("git repository found at {}", opts.root.display());
    } else {
        tracing::debug!("no git repository, skipping git metadata");
    }

    let mut builder = WalkBuilder::new(&opts.root);
    builder
        .hidden(false) // include dot-files that aren't ignored
        .ignore(true) // respect .ignore files
        .git_ignore(true) // respect .gitignore
        .git_global(true) // respect global .gitignore
        .git_exclude(true); // respect .git/info/exclude

    // Add extra ignore patterns
    for extra in &opts.extra_ignore {
        builder.add_ignore(extra);
    }

    let walker = builder.build();
    let mut entries: Vec<FileEntry> = Vec::new();
    let mut skipped_size = 0usize;
    let mut skipped_binary = 0usize;

    for result in walker {
        let dir_entry = match result {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!("walk error: {err}");
                continue;
            }
        };

        // Only process regular files
        if !dir_entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }

        let path = dir_entry.path().to_path_buf();

        // Extension filter
        if !opts.include_extensions.is_empty() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !opts.include_extensions.iter().any(|e| e == ext) {
                continue;
            }
        }

        // Size filter
        let size_bytes = match dir_entry.metadata() {
            Ok(m) => m.len(),
            Err(e) => {
                tracing::warn!("metadata error for {}: {e}", path.display());
                continue;
            }
        };

        if size_bytes > opts.max_file_bytes {
            skipped_size += 1;
            tracing::trace!(
                "skipping large file: {} ({size_bytes} bytes)",
                path.display()
            );
            continue;
        }

        // Read content
        let content = match std::fs::read(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("read error for {}: {e}", path.display());
                continue;
            }
        };

        // Skip binary files (contains NUL byte)
        if content.contains(&0u8) {
            skipped_binary += 1;
            tracing::trace!("skipping binary file: {}", path.display());
            continue;
        }

        // Token count
        let token_count = tokenizer.count_bytes(&content);
        if token_count > opts.max_file_tokens {
            tracing::trace!(
                "skipping file over token limit: {} ({token_count} tokens)",
                path.display()
            );
            continue;
        }

        // Hash
        let hash = md5_hash(&content);

        // Language
        let language = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Language::from_extension);

        // Last modified time
        let last_modified = dir_entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // Git metadata (expensive — only fetch for non-large repos)
        let git = repo.as_ref().and_then(|r| read_git_metadata(r, &path));

        // AST analysis via tree-sitter (if feature enabled and language supported)
        #[cfg(feature = "ast")]
        let ast =
            language.and_then(|lang| super::ast::analyze_file(&content, lang, opts.max_ast_bytes));
        #[cfg(not(feature = "ast"))]
        let ast: Option<crate::types::AstData> = None;

        // SimHash fingerprint for near-duplicate detection
        let simhash = if opts.compute_simhash {
            Some(simhash_fingerprint(&content, opts.shingle_size))
        } else {
            None
        };

        entries.push(FileEntry {
            path,
            token_count,
            hash,
            metadata: FileMetadata {
                size_bytes,
                last_modified,
                git,
                language,
            },
            ast,
            simhash,
        });
    }

    tracing::info!(
        "discovery: {} files indexed, {} skipped (size), {} skipped (binary)",
        entries.len(),
        skipped_size,
        skipped_binary
    );

    if entries.is_empty() {
        return Err(OptimError::EmptyRepo {
            path: opts.root.display().to_string(),
        });
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_opts(dir: &Path) -> DiscoveryOptions {
        DiscoveryOptions {
            root: dir.to_path_buf(),
            extra_ignore: vec![],
            max_file_bytes: 1024 * 1024,
            max_file_tokens: 100_000,
            include_extensions: vec![],
            max_ast_bytes: 256 * 1024,
            compute_simhash: false,
            shingle_size: 3,
        }
    }

    #[test]
    fn test_discover_basic() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(tmp.path().join("lib.rs"), "pub fn foo() {}").unwrap();

        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_discover_empty_returns_error() {
        let tmp = TempDir::new().unwrap();
        let result = discover_files(&make_opts(tmp.path()));
        assert!(matches!(result, Err(OptimError::EmptyRepo { .. })));
    }

    #[test]
    fn test_discover_skips_large_files() {
        let tmp = TempDir::new().unwrap();
        let big = vec![b'a'; 2048];
        std::fs::write(tmp.path().join("big.rs"), &big).unwrap();
        std::fs::write(tmp.path().join("small.rs"), "fn small() {}").unwrap();

        let opts = DiscoveryOptions {
            max_file_bytes: 100,
            ..make_opts(tmp.path())
        };
        let entries = discover_files(&opts).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].path.file_name().unwrap() == "small.rs");
    }

    #[test]
    fn test_discover_skips_binary() {
        let tmp = TempDir::new().unwrap();
        let binary = vec![0u8, 1u8, 2u8, 0u8]; // contains NUL
        std::fs::write(tmp.path().join("data.bin"), &binary).unwrap();
        std::fs::write(tmp.path().join("code.rs"), "fn main() {}").unwrap();

        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].path.extension().unwrap() == "rs");
    }

    #[test]
    fn test_discover_extension_filter_only_includes_matching() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(tmp.path().join("script.py"), "def main(): pass").unwrap();
        std::fs::write(tmp.path().join("app.go"), "package main").unwrap();

        let opts = DiscoveryOptions {
            include_extensions: vec!["rs".to_string()],
            ..make_opts(tmp.path())
        };
        let entries = discover_files(&opts).unwrap();

        assert_eq!(entries.len(), 1, "only .rs should be included");
        assert_eq!(
            entries[0].path.extension().and_then(|e| e.to_str()),
            Some("rs")
        );
    }

    #[test]
    fn test_discover_extension_filter_multiple_extensions() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(tmp.path().join("script.py"), "def main(): pass").unwrap();
        std::fs::write(tmp.path().join("README.md"), "# readme").unwrap();

        let opts = DiscoveryOptions {
            include_extensions: vec!["rs".to_string(), "py".to_string()],
            ..make_opts(tmp.path())
        };
        let entries = discover_files(&opts).unwrap();

        assert_eq!(entries.len(), 2, "rs and py should be included");
        let exts: Vec<&str> = entries
            .iter()
            .filter_map(|e| e.path.extension()?.to_str())
            .collect();
        assert!(exts.contains(&"rs"));
        assert!(exts.contains(&"py"));
    }

    #[test]
    fn test_discover_max_token_filter_excludes_large_files() {
        let tmp = TempDir::new().unwrap();
        // Generate a file with many tokens (lots of distinct words)
        let big: String = (0..500).map(|i| format!("word{i} ")).collect();
        std::fs::write(tmp.path().join("big.rs"), &big).unwrap();
        std::fs::write(tmp.path().join("small.rs"), "fn x() {}").unwrap();

        let opts = DiscoveryOptions {
            max_file_tokens: 10,
            ..make_opts(tmp.path())
        };
        let entries = discover_files(&opts).unwrap();

        assert_eq!(
            entries.len(),
            1,
            "only small.rs should pass the token filter"
        );
        assert_eq!(
            entries[0].path.file_name().and_then(|n| n.to_str()),
            Some("small.rs")
        );
    }

    #[test]
    fn test_discover_language_detected_correctly() {
        use crate::types::Language;
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("lib.rs"), "pub fn lib() {}").unwrap();

        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].metadata.language,
            Some(Language::Rust),
            "expected Rust language detection"
        );
    }

    #[test]
    fn test_discover_multiple_languages_detected() {
        use crate::types::Language;
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(tmp.path().join("utils.py"), "def util(): pass").unwrap();
        std::fs::write(tmp.path().join("README.md"), "# readme").unwrap();

        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries.len(), 3);

        let rs = entries.iter().find(|e| {
            e.path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x == "rs")
        });
        let py = entries.iter().find(|e| {
            e.path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x == "py")
        });
        let md = entries.iter().find(|e| {
            e.path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x == "md")
        });

        assert_eq!(rs.unwrap().metadata.language, Some(Language::Rust));
        assert_eq!(py.unwrap().metadata.language, Some(Language::Python));
        assert_eq!(md.unwrap().metadata.language, None); // .md has no Language variant
    }

    #[test]
    fn test_discover_metadata_fields_populated() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("code.rs"), "fn hello() { let x = 1; }").unwrap();

        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];

        assert!(e.token_count > 0, "token_count should be non-zero");
        assert!(e.metadata.size_bytes > 0, "size_bytes should be non-zero");
        assert_ne!(e.hash, [0u8; 16], "hash should not be all-zero");
    }

    #[test]
    fn test_discover_returns_absolute_paths() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("abs.rs"), "fn f() {}").unwrap();

        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].path.is_absolute(),
            "discovered paths should be absolute"
        );
    }

    #[test]
    fn test_discover_populates_simhash_when_enabled() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn main() {}").unwrap();
        let opts = DiscoveryOptions {
            compute_simhash: true,
            shingle_size: 3,
            ..make_opts(tmp.path())
        };
        let entries = discover_files(&opts).unwrap();
        assert!(
            entries[0].simhash.is_some(),
            "simhash should be Some when compute_simhash is true"
        );
    }

    #[test]
    fn test_discover_skips_simhash_when_disabled() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn main() {}").unwrap();
        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        assert!(
            entries[0].simhash.is_none(),
            "simhash should be None when compute_simhash is false"
        );
    }
}
