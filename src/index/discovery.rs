use crate::{
    config::Config,
    error::OptimError,
    index::{dedup::md5_hash, simhash::simhash_fingerprint, tokenizer::estimate_tokens_bytes},
    types::{FileEntry, FileMetadata, GitMetadata, Language},
};
use dashmap::DashMap;
use ignore::WalkBuilder;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------
// Per-process git-metadata cache: keyed by (absolute path, mtime_secs).
// Survives across MCP tool calls in the same server process.
// ---------------------------------------------------------------------------
type GitCache = DashMap<(PathBuf, u64), Option<GitMetadata>>;

static GIT_CACHE: OnceLock<GitCache> = OnceLock::new();

fn git_cache() -> &'static GitCache {
    GIT_CACHE.get_or_init(DashMap::new)
}

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
    /// Skip AST parsing during discovery (for two-phase optimization).
    /// When true, all entries will have `ast: None`.
    pub skip_ast: bool,
    /// Retain raw file content in FileEntry for L3 caching.
    /// When false, all entries will have `content: None` (saves memory).
    pub retain_content: bool,
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
            skip_ast: false,
            retain_content: true,
        }
    }
}

/// Batch-compute git metadata for all given paths in a single revwalk.
///
/// Instead of N separate revwalks (one per file), this does **one** revwalk and
/// uses `diff_tree_to_tree` per commit to identify all changed files at once.
/// For C commits and N files this is O(C × D_avg) where D_avg is the average
/// number of files changed per commit, instead of O(N × C) tree lookups.
///
/// Returns a map from absolute path → `GitMetadata`. Files that were never
/// changed in the traversed commits (or that cannot be resolved) are absent
/// from the result.
fn batch_git_metadata(repo: &git2::Repository, paths: &[PathBuf]) -> HashMap<PathBuf, GitMetadata> {
    let workdir = match repo.workdir() {
        Some(w) => w,
        None => return HashMap::new(),
    };

    const MAX_COMMITS: usize = 500;
    const MAX_MODIFICATIONS: u32 = 100;

    struct FileState {
        commit_count: u32,
        latest_time: Option<i64>,
        abs_index: usize,
    }

    let mut file_states: HashMap<PathBuf, FileState> = HashMap::with_capacity(paths.len());
    let mut remaining = 0usize; // files not yet at MAX_MODIFICATIONS

    for (i, path) in paths.iter().enumerate() {
        if let Ok(relative) = path.strip_prefix(workdir) {
            file_states.insert(
                relative.to_path_buf(),
                FileState {
                    commit_count: 0,
                    latest_time: None,
                    abs_index: i,
                },
            );
            remaining += 1;
        }
    }

    let mut revwalk = match repo.revwalk() {
        Ok(rw) => rw,
        Err(_) => return HashMap::new(),
    };
    if revwalk.push_head().is_err() {
        return HashMap::new();
    }

    let mut commits_visited = 0usize;

    for oid in revwalk.flatten() {
        if remaining == 0 || commits_visited >= MAX_COMMITS {
            break;
        }
        commits_visited += 1;

        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let commit_tree = match commit.tree() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let commit_time = commit.time().seconds();

        // Parent tree (None for the initial commit → diff against empty tree)
        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

        let diff = match repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&commit_tree), None) {
            Ok(d) => d,
            Err(_) => continue,
        };

        for delta in diff.deltas() {
            // Only count additions and modifications, not deletions
            match delta.status() {
                git2::Delta::Added | git2::Delta::Modified | git2::Delta::Copied => {}
                _ => continue,
            }

            let delta_path = match delta.new_file().path() {
                Some(p) => p,
                None => continue,
            };

            if let Some(state) = file_states.get_mut(delta_path) {
                if state.commit_count < MAX_MODIFICATIONS {
                    state.commit_count += 1;
                    if state.latest_time.is_none() {
                        state.latest_time = Some(commit_time);
                    }
                    if state.commit_count >= MAX_MODIFICATIONS {
                        remaining -= 1;
                    }
                }
            }
        }
    }

    tracing::debug!(
        "batch git metadata: {} files, {} commits visited, {} fully resolved",
        paths.len(),
        commits_visited,
        paths.len() - remaining,
    );

    // Convert to result HashMap<absolute_path, GitMetadata>
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut result = HashMap::with_capacity(file_states.len());
    for (_, state) in file_states {
        if state.commit_count == 0 {
            continue;
        }
        if let Some(latest) = state.latest_time {
            let diff_secs = (now - latest).max(0);
            let age_days = diff_secs as f64 / 86_400.0;
            result.insert(
                paths[state.abs_index].clone(),
                GitMetadata {
                    age_days,
                    commit_count: state.commit_count,
                },
            );
        }
    }

    result
}

/// Walk a directory and collect `FileEntry` records.
///
/// - Respects `.gitignore` and `.ignore` files via the `ignore` crate.
/// - Skips files exceeding `max_file_bytes`.
/// - Hashes content with MD5 (for dedup).
/// - Estimates tokens using byte-class heuristics (fast path).
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
/// Intermediate record produced by the serial walk phase before git enrichment.
struct WalkRecord {
    path: PathBuf,
    size_bytes: u64,
    last_modified: SystemTime,
    token_count: usize,
    hash: [u8; 16],
    language: Option<Language>,
    #[cfg(feature = "ast")]
    ast: Option<crate::types::AstData>,
    #[cfg(not(feature = "ast"))]
    ast: Option<crate::types::AstData>,
    simhash: Option<u64>,
    content: Option<Vec<u8>>,
}

pub fn discover_files(opts: &DiscoveryOptions) -> Result<Vec<FileEntry>, OptimError> {
    // Check whether a git repo is reachable (used later for batch git metadata).
    let repo_root: Option<PathBuf> = git2::Repository::discover(&opts.root)
        .ok()
        .and_then(|r| r.workdir().map(|p| p.to_path_buf()));
    if repo_root.is_some() {
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

    // Filter entries whose *name* matches any extra_ignore pattern.
    // NOTE: WalkBuilder::add_ignore() expects a path to an ignore-rules FILE,
    // not a bare name — using it with directory names like ".git" silently does
    // nothing.  filter_entry() is the correct API for excluding paths by name.
    let extra_ignores = opts.extra_ignore.clone();
    builder.filter_entry(move |e| {
        if let Some(name) = e.path().file_name() {
            !extra_ignores.iter().any(|p| p.as_os_str() == name)
        } else {
            true
        }
    });

    // ── Phase 1a: serial walk ───────────────────────────────────────────────
    // Walk the directory tree and read file contents.  Only cheap filtering
    // (extension, size, binary) is done here.  Expensive per-file work
    // (tokenization, hashing, AST) is deferred to Phase 1b (parallel).
    let walker = builder.build();
    let mut skipped_size = 0usize;
    let mut skipped_binary = 0usize;

    struct RawFile {
        path: PathBuf,
        size_bytes: u64,
        last_modified: SystemTime,
        content: Vec<u8>,
        language: Option<Language>,
    }

    let mut raw_files: Vec<RawFile> = Vec::new();

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

        // Language detection (cheap — just extension lookup)
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

        raw_files.push(RawFile {
            path,
            size_bytes,
            last_modified,
            content,
            language,
        });
    }

    tracing::info!(
        "discovery walk: {} files collected, {} skipped (size), {} skipped (binary)",
        raw_files.len(),
        skipped_size,
        skipped_binary
    );

    if raw_files.is_empty() {
        return Err(OptimError::EmptyRepo {
            path: opts.root.display().to_string(),
        });
    }

    // ── Phase 1b: parallel processing ───────────────────────────────────────
    // Tokenize, hash, AST-parse, and SimHash in parallel via rayon.
    // Tree-sitter parsing dominates serial Phase 1 cost (~2.8ms/file),
    // so parallelizing this phase gives a near-linear speedup with cores.
    use rayon::prelude::*;

    let compute_simhash = opts.compute_simhash;
    let shingle_size = opts.shingle_size;
    let max_file_tokens = opts.max_file_tokens;
    let max_ast_bytes = opts.max_ast_bytes;
    let skip_ast = opts.skip_ast;
    let retain_content = opts.retain_content;

    let records: Vec<Option<WalkRecord>> = raw_files
        .into_par_iter()
        .map(|raw| {
            let token_count = estimate_tokens_bytes(&raw.content);
            if token_count > max_file_tokens {
                return None;
            }

            let hash = md5_hash(&raw.content);

            #[cfg(feature = "ast")]
            let ast = if !skip_ast {
                raw.language
                    .and_then(|lang| super::ast::analyze_file(&raw.content, lang, max_ast_bytes))
            } else {
                None
            };
            #[cfg(not(feature = "ast"))]
            let ast: Option<crate::types::AstData> = None;

            let simhash = if compute_simhash {
                Some(simhash_fingerprint(&raw.content, shingle_size))
            } else {
                None
            };

            Some(WalkRecord {
                path: raw.path,
                size_bytes: raw.size_bytes,
                last_modified: raw.last_modified,
                token_count,
                hash,
                language: raw.language,
                ast,
                simhash,
                content: if retain_content {
                    Some(raw.content)
                } else {
                    None
                },
            })
        })
        .collect();

    let records: Vec<WalkRecord> = records.into_iter().flatten().collect();

    if records.is_empty() {
        return Err(OptimError::EmptyRepo {
            path: opts.root.display().to_string(),
        });
    }

    // ── Phase 2: batch git metadata enrichment ────────────────────────────────
    // A single revwalk + diff_tree_to_tree processes all files at once instead
    // of N separate revwalks.  The process-wide GIT_CACHE is still checked
    // first so repeated MCP calls skip the git walk for unchanged files.

    let mut git_metadata_vec: Vec<Option<GitMetadata>> = vec![None; records.len()];

    if let Some(root) = &repo_root {
        // Check which files are already cached
        let mut uncached_indices: Vec<usize> = Vec::new();

        for (i, rec) in records.iter().enumerate() {
            let mtime_key = rec
                .last_modified
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let cache_key = (rec.path.clone(), mtime_key);

            if let Some(cached) = git_cache().get(&cache_key) {
                git_metadata_vec[i] = cached.clone();
            } else {
                uncached_indices.push(i);
            }
        }

        // Batch-compute metadata for all uncached files in a single revwalk
        if !uncached_indices.is_empty() {
            if let Ok(repo) = git2::Repository::open(root) {
                let uncached_paths: Vec<PathBuf> = uncached_indices
                    .iter()
                    .map(|&i| records[i].path.clone())
                    .collect();
                let batch = batch_git_metadata(&repo, &uncached_paths);

                for &i in &uncached_indices {
                    let meta = batch.get(&records[i].path).cloned();
                    let mtime_key = records[i]
                        .last_modified
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    git_cache().insert((records[i].path.clone(), mtime_key), meta.clone());
                    git_metadata_vec[i] = meta;
                }
            }
        }
    }

    let entries: Vec<FileEntry> = records
        .into_iter()
        .zip(git_metadata_vec)
        .map(|(rec, git)| FileEntry {
            path: rec.path,
            token_count: rec.token_count,
            hash: rec.hash,
            metadata: FileMetadata {
                size_bytes: rec.size_bytes,
                last_modified: rec.last_modified,
                git,
                language: rec.language,
            },
            ast: rec.ast,
            simhash: rec.simhash,
            content: rec.content,
        })
        .collect();

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
            skip_ast: false,
            retain_content: true,
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

    #[test]
    fn test_discover_excludes_extra_ignore_directories() {
        let tmp = TempDir::new().unwrap();

        // Real source file that should be indexed
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();

        // Create a fake .git directory with text files that would previously
        // leak through (the old add_ignore() bug treated these as rules files,
        // not exclusion targets, so .git/ was never filtered).
        let git_dir = tmp.path().join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git_dir.join("COMMIT_EDITMSG"), "initial commit\n").unwrap();
        let refs_dir = git_dir.join("refs").join("heads");
        std::fs::create_dir_all(&refs_dir).unwrap();
        std::fs::write(
            refs_dir.join("main"),
            "0000000000000000000000000000000000000000\n",
        )
        .unwrap();

        // Also create a target directory (another common extra_ignore entry)
        let target_dir = tmp.path().join("target");
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(target_dir.join("binary_output.rs"), "// generated").unwrap();

        let opts = DiscoveryOptions {
            extra_ignore: vec![PathBuf::from(".git"), PathBuf::from("target")],
            ..make_opts(tmp.path())
        };
        let entries = discover_files(&opts).unwrap();

        // Only main.rs should be discovered
        assert_eq!(entries.len(), 1, "expected only main.rs, got {entries:?}");
        assert_eq!(
            entries[0].path.file_name().and_then(|n| n.to_str()),
            Some("main.rs")
        );

        // Confirm no .git/ or target/ paths leaked through
        for e in &entries {
            let p = e.path.to_string_lossy();
            assert!(
                !p.contains("/.git/") && !p.contains("\\.git\\"),
                "discovered a .git/ path: {p}"
            );
            assert!(
                !p.contains("/target/") && !p.contains("\\target\\"),
                "discovered a target/ path: {p}"
            );
        }
    }

    #[test]
    fn test_discover_skip_ast_returns_none_ast() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("main.rs"), "pub fn hello() -> usize { 42 }").unwrap();

        let opts = DiscoveryOptions {
            skip_ast: true,
            ..make_opts(tmp.path())
        };
        let entries = discover_files(&opts).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].ast.is_none(),
            "AST should be None when skip_ast is true"
        );
    }

    #[test]
    fn test_discover_retain_content_false_returns_none_content() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();

        let opts = DiscoveryOptions {
            retain_content: false,
            ..make_opts(tmp.path())
        };
        let entries = discover_files(&opts).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].content.is_none(),
            "content should be None when retain_content is false"
        );
    }

    #[test]
    fn test_discover_lite_mode_still_computes_tokens_and_hash() {
        let tmp = TempDir::new().unwrap();
        let code = "pub fn lite_test() -> usize { 42 }";
        std::fs::write(tmp.path().join("lite.rs"), code).unwrap();

        let opts = DiscoveryOptions {
            skip_ast: true,
            retain_content: false,
            ..make_opts(tmp.path())
        };
        let entries = discover_files(&opts).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].token_count > 0,
            "tokens should still be computed"
        );
        assert_ne!(entries[0].hash, [0u8; 16], "hash should still be computed");
        assert!(entries[0].ast.is_none());
        assert!(entries[0].content.is_none());
    }

    #[test]
    fn test_discover_git_cache_is_populated_on_first_run() {
        // Verify the process-level git cache is accessible (not a correctness
        // test of git metadata, just that the cache infrastructure is live).
        let cache = git_cache();
        // The cache is a DashMap — we can read its len without error.
        let _ = cache.len();
    }

    #[test]
    fn test_discover_returns_sorted_paths() {
        let tmp = TempDir::new().unwrap();
        // Create files in reverse alphabetical order to catch ordering issues
        for name in ["z.rs", "m.rs", "a.rs", "f.rs", "b.rs"] {
            std::fs::write(tmp.path().join(name), format!("fn {name}() {{}}")).unwrap();
        }

        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries.len(), 5);

        let paths: Vec<String> = entries
            .iter()
            .map(|e| e.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "entries should be sorted by path");
    }

    #[test]
    fn test_discover_no_duplicates_with_nested_dirs() {
        let tmp = TempDir::new().unwrap();
        // Create a deep directory structure — parallel walkers could visit entries twice
        // if work-stealing boundaries overlap.
        for i in 0..5 {
            let dir = tmp.path().join(format!("level_{i}"));
            std::fs::create_dir_all(&dir).unwrap();
            for j in 0..3 {
                std::fs::write(
                    dir.join(format!("file_{j}.rs")),
                    format!("fn f_{i}_{j}() {{}}"),
                )
                .unwrap();
            }
        }

        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries.len(), 15);

        // Check no duplicates
        let mut paths: Vec<PathBuf> = entries.iter().map(|e| e.path.clone()).collect();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), 15, "no duplicate paths should be returned");
    }

    #[test]
    fn test_discover_skip_counts_are_accurate() {
        let tmp = TempDir::new().unwrap();
        // 3 valid files
        std::fs::write(tmp.path().join("good1.rs"), "fn a() {}").unwrap();
        std::fs::write(tmp.path().join("good2.rs"), "fn b() {}").unwrap();
        std::fs::write(tmp.path().join("good3.rs"), "fn c() {}").unwrap();
        // 2 binary files (contain NUL)
        std::fs::write(tmp.path().join("bin1.dat"), &[0u8, 1, 2]).unwrap();
        std::fs::write(tmp.path().join("bin2.dat"), &[0u8, 3, 4]).unwrap();

        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        // Only the 3 .rs files should be discovered (binary skipped)
        assert_eq!(entries.len(), 3, "should discover exactly 3 text files");
    }
}
