use crate::{
    config::Config,
    error::OptimError,
    index::{
        cache::{self, CacheManifest},
        dedup::md5_hash,
        simhash::simhash_fingerprint,
        tokenizer::estimate_tokens_bytes,
    },
    types::{FileEntry, FileMetadata, GitMetadata, Language},
};
use dashmap::DashMap;
use ignore::WalkBuilder;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
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
/// Intermediate record produced by the parallel walk phase before git enrichment.
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

    // ── P5: load persistent on-disk discovery cache ────────────────────────
    // Loaded once before the walk and shared (read-only) with all worker
    // threads via Arc.  Lookup is keyed on absolute path so the walker's
    // `dir_entry.path()` matches without per-lookup transformation.
    let cfg_hash = cache::config_hash(opts);
    let cached_lookup: Arc<HashMap<PathBuf, FileEntry>> =
        Arc::new(CacheManifest::load(&opts.root, cfg_hash).into_lookup());
    let initial_cache_size = cached_lookup.len();
    tracing::debug!("discovery cache: {initial_cache_size} entries loaded");

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
    //
    // We unconditionally skip `.ctx-optim` (our own state directory holding
    // the discovery cache and feedback DB) regardless of user config — it's
    // never source code and should never be indexed.
    let extra_ignores = opts.extra_ignore.clone();
    builder.filter_entry(move |e| {
        if let Some(name) = e.path().file_name() {
            if name == std::ffi::OsStr::new(".ctx-optim") {
                return false;
            }
            !extra_ignores.iter().any(|p| p.as_os_str() == name)
        } else {
            true
        }
    });

    // ── Phase 1a: parallel walk ────────────────────────────────────────────
    // Walk the directory tree using crossbeam work-stealing threads (via the
    // `ignore` crate's `WalkParallel`).  Each worker thread traverses,
    // filters, reads content, and pushes RawFile entries into a shared vec.
    // This parallelizes both directory traversal and file I/O.
    let walker = builder.build_parallel();
    let skipped_size = Arc::new(AtomicUsize::new(0));
    let skipped_binary = Arc::new(AtomicUsize::new(0));

    #[derive(Debug)]
    struct RawFile {
        path: PathBuf,
        size_bytes: u64,
        last_modified: SystemTime,
        content: Vec<u8>,
        language: Option<Language>,
    }

    let raw_files: Arc<Mutex<Vec<RawFile>>> = Arc::new(Mutex::new(Vec::new()));
    let cache_hits: Arc<Mutex<Vec<FileEntry>>> = Arc::new(Mutex::new(Vec::new()));

    // Capture options for the closure factory
    let include_extensions = opts.include_extensions.clone();
    let max_file_bytes = opts.max_file_bytes;

    walker.run(|| {
        // Per-thread clones of shared state
        let raw_files = Arc::clone(&raw_files);
        let cache_hits = Arc::clone(&cache_hits);
        let cached_lookup = Arc::clone(&cached_lookup);
        let skipped_size = Arc::clone(&skipped_size);
        let skipped_binary = Arc::clone(&skipped_binary);
        let include_extensions = include_extensions.clone();

        Box::new(move |result| {
            let dir_entry = match result {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!("walk error: {err}");
                    return ignore::WalkState::Continue;
                }
            };

            // Only process regular files
            if !dir_entry.file_type().is_some_and(|t| t.is_file()) {
                return ignore::WalkState::Continue;
            }

            let path = dir_entry.path().to_path_buf();

            // Extension filter
            if !include_extensions.is_empty() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if !include_extensions.iter().any(|e| e == ext) {
                    return ignore::WalkState::Continue;
                }
            }

            // Metadata (single stat() call — reused for size + last_modified
            // + cache key)
            let metadata = match dir_entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("metadata error for {}: {e}", path.display());
                    return ignore::WalkState::Continue;
                }
            };

            let size_bytes = metadata.len();
            let mtime_secs = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            // ── P5 cache lookup ────────────────────────────────────────────
            // Check the on-disk cache BEFORE size filter and content read.
            // If (path, mtime_secs, size_bytes) matches a cached entry, push
            // the cached FileEntry directly and skip read+tokenize+hash+ast+git.
            // Mtime+size match proves the content hasn't changed since the
            // file was originally classified, so the binary-NUL check stays
            // valid.
            if let Some(cached) = cached_lookup.get(&path) {
                let cached_mtime = cached
                    .metadata
                    .last_modified
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if cached_mtime == mtime_secs && cached.metadata.size_bytes == size_bytes {
                    cache_hits.lock().unwrap().push(cached.clone());
                    return ignore::WalkState::Continue;
                }
            }

            // Size filter
            if size_bytes > max_file_bytes {
                skipped_size.fetch_add(1, Ordering::Relaxed);
                tracing::trace!(
                    "skipping large file: {} ({size_bytes} bytes)",
                    path.display()
                );
                return ignore::WalkState::Continue;
            }

            // Read content
            let content = match std::fs::read(&path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("read error for {}: {e}", path.display());
                    return ignore::WalkState::Continue;
                }
            };

            // Skip binary files (contains NUL byte)
            if content.contains(&0u8) {
                skipped_binary.fetch_add(1, Ordering::Relaxed);
                tracing::trace!("skipping binary file: {}", path.display());
                return ignore::WalkState::Continue;
            }

            // Language detection (cheap — just extension lookup)
            let language = path
                .extension()
                .and_then(|e| e.to_str())
                .and_then(Language::from_extension);

            // Last modified time (reuse metadata from above)
            let last_modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);

            raw_files.lock().unwrap().push(RawFile {
                path,
                size_bytes,
                last_modified,
                content,
                language,
            });

            ignore::WalkState::Continue
        })
    });

    let mut raw_files = Arc::try_unwrap(raw_files)
        .expect("all walker threads finished")
        .into_inner()
        .unwrap();
    let mut cache_hits = Arc::try_unwrap(cache_hits)
        .expect("all walker threads finished")
        .into_inner()
        .unwrap();

    // Sort for deterministic output (parallel walk order is non-deterministic)
    raw_files.sort_by(|a, b| a.path.cmp(&b.path));

    let skipped_size = skipped_size.load(Ordering::Relaxed);
    let skipped_binary = skipped_binary.load(Ordering::Relaxed);

    tracing::info!(
        "discovery walk: {} files ({} cache hits, {} fresh), {} skipped (size), {} skipped (binary)",
        raw_files.len() + cache_hits.len(),
        cache_hits.len(),
        raw_files.len(),
        skipped_size,
        skipped_binary
    );

    if raw_files.is_empty() && cache_hits.is_empty() {
        return Err(OptimError::EmptyRepo {
            path: opts.root.display().to_string(),
        });
    }

    // ── P5 fast path: every walked file was cached ──────────────────────────
    // Skip Phase 1b (tokenize/hash/AST/SimHash) and Phase 2 (git revwalk)
    // entirely.  Cached entries already carry their git metadata.  Only
    // rewrite the manifest if files were deleted (cache_hits.len() drops
    // below the original cache size).
    if raw_files.is_empty() {
        cache_hits.sort_by(|a, b| a.path.cmp(&b.path));
        if cache_hits.len() != initial_cache_size {
            CacheManifest::from_entries(&cache_hits, &opts.root, cfg_hash).save(&opts.root);
        }
        return Ok(cache_hits);
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

    let fresh_entries: Vec<FileEntry> = records
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

    // ── P5 slow path: merge cache hits with fresh entries, save manifest ────
    // Cache hits are returned with `content: None` (cached entries never
    // carry content).  Sort by path for deterministic output.  Persist the
    // updated manifest so subsequent runs hit the warm path.
    let mut entries: Vec<FileEntry> = fresh_entries;
    entries.extend(cache_hits);
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    CacheManifest::from_entries(&entries, &opts.root, cfg_hash).save(&opts.root);

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
        std::fs::write(tmp.path().join("bin1.dat"), [0u8, 1, 2]).unwrap();
        std::fs::write(tmp.path().join("bin2.dat"), [0u8, 3, 4]).unwrap();

        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        // Only the 3 .rs files should be discovered (binary skipped)
        assert_eq!(entries.len(), 3, "should discover exactly 3 text files");
    }

    // ── P5: persistent on-disk cache integration tests ──────────────────────

    /// Bump the mtime of `path` by `secs` seconds (works on macOS/Linux).
    /// Returns the new mtime as `SystemTime`.
    fn bump_mtime(path: &Path, secs_offset: i64) -> SystemTime {
        let new_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + secs_offset;
        let new_time = UNIX_EPOCH + std::time::Duration::from_secs(new_secs as u64);

        // Use libc's utimes via filesystem-level syscall — std doesn't expose
        // this directly, so we shell out to `touch -t`.  Format is
        // YYYYMMDDhhmm.SS in local time.
        let dt = chrono_format(new_secs);
        let _ = std::process::Command::new("touch")
            .arg("-t")
            .arg(&dt)
            .arg(path)
            .status();
        new_time
    }

    /// Format a unix timestamp (UTC) as `YYYYMMDDhhmm.SS` for `touch -t`.
    /// Pure-Rust to avoid pulling in chrono just for tests.
    fn chrono_format(secs: i64) -> String {
        let days = secs / 86_400;
        let rem = secs % 86_400;
        let hours = rem / 3600;
        let minutes = (rem % 3600) / 60;
        let seconds = rem % 60;

        // Civil-from-days algorithm (Howard Hinnant, public domain)
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        format!("{y:04}{m:02}{d:02}{hours:02}{minutes:02}.{seconds:02}")
    }

    #[test]
    fn test_discover_writes_cache_on_first_run() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();

        let _ = discover_files(&make_opts(tmp.path())).unwrap();

        let cache_path = crate::index::cache::CacheManifest::cache_path(tmp.path());
        assert!(
            cache_path.exists(),
            "cache file should be written after first discovery"
        );
    }

    #[test]
    fn test_discover_uses_cache_on_second_run() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(tmp.path().join("b.rs"), "fn b() {}").unwrap();

        // First run — populates cache
        let entries1 = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries1.len(), 2);

        // Capture cache file mtime
        let cache_path = crate::index::cache::CacheManifest::cache_path(tmp.path());
        let mtime1 = std::fs::metadata(&cache_path).unwrap().modified().unwrap();

        // Sleep enough to detect a rewrite via mtime delta
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Second run — every file is a cache hit, no fresh files, no deletions
        // → manifest should NOT be rewritten
        let entries2 = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries2.len(), 2);

        let mtime2 = std::fs::metadata(&cache_path).unwrap().modified().unwrap();
        assert_eq!(
            mtime1, mtime2,
            "manifest should not be rewritten when nothing changed"
        );
    }

    #[test]
    fn test_discover_cache_invalidation_on_file_modification() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("a.rs");
        std::fs::write(&file, "fn a() {}").unwrap();

        let entries1 = discover_files(&make_opts(tmp.path())).unwrap();
        let hash1 = entries1[0].hash;

        // Bump mtime backwards by 10 seconds to guarantee a different mtime_secs,
        // then rewrite content with a different hash.
        bump_mtime(&file, -10);
        std::fs::write(&file, "fn b() { let x = 42; }").unwrap();

        let entries2 = discover_files(&make_opts(tmp.path())).unwrap();
        let hash2 = entries2[0].hash;

        assert_ne!(
            hash1, hash2,
            "modified file should produce a different hash on the second run"
        );
    }

    #[test]
    fn test_discover_cache_invalidation_on_file_deletion() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(tmp.path().join("b.rs"), "fn b() {}").unwrap();

        let entries1 = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries1.len(), 2);

        std::fs::remove_file(tmp.path().join("b.rs")).unwrap();

        let entries2 = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries2.len(), 1);
        assert_eq!(entries2[0].path.file_name().unwrap(), "a.rs");

        // Verify the manifest on disk no longer contains b.rs
        let cfg_hash = crate::index::cache::config_hash(&make_opts(tmp.path()));
        let manifest = crate::index::cache::CacheManifest::load(tmp.path(), cfg_hash);
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].path, PathBuf::from("a.rs"));
    }

    #[test]
    fn test_discover_cache_invalidation_on_file_addition() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();

        let entries1 = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries1.len(), 1);

        std::fs::write(tmp.path().join("b.rs"), "fn b() {}").unwrap();

        let entries2 = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries2.len(), 2);

        // Verify both are now in the manifest
        let cfg_hash = crate::index::cache::config_hash(&make_opts(tmp.path()));
        let manifest = crate::index::cache::CacheManifest::load(tmp.path(), cfg_hash);
        assert_eq!(manifest.entries.len(), 2);
    }

    #[test]
    fn test_discover_cache_invalidation_on_config_change() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();

        // First run with default opts
        let _ = discover_files(&make_opts(tmp.path())).unwrap();

        // Second run with a different max_file_bytes — should produce a different
        // config_hash and thus invalidate the cache.
        let opts2 = DiscoveryOptions {
            max_file_bytes: 1, // smaller than the file → should be excluded
            ..make_opts(tmp.path())
        };
        let result = discover_files(&opts2);
        assert!(matches!(result, Err(OptimError::EmptyRepo { .. })));
    }

    #[test]
    fn test_discover_cache_results_match_uncached() {
        let tmp = TempDir::new().unwrap();
        for i in 0..5 {
            std::fs::write(
                tmp.path().join(format!("mod_{i}.rs")),
                format!("pub fn func_{i}() -> usize {{ {i} * 2 }}"),
            )
            .unwrap();
        }

        // First run (cold)
        let cold = discover_files(&make_opts(tmp.path())).unwrap();

        // Verify cache exists
        let cache_path = crate::index::cache::CacheManifest::cache_path(tmp.path());
        assert!(cache_path.exists());

        // Second run (warm)
        let warm = discover_files(&make_opts(tmp.path())).unwrap();

        assert_eq!(cold.len(), warm.len(), "cold and warm should yield same N");

        // Compare on the structurally-stable fields.  `last_modified` and git
        // metadata can drift across calls (now() advances), so compare on
        // path, hash, token_count, size_bytes, language, ast, simhash.
        for (c, w) in cold.iter().zip(warm.iter()) {
            assert_eq!(c.path, w.path, "path mismatch");
            assert_eq!(c.hash, w.hash, "hash mismatch for {}", c.path.display());
            assert_eq!(
                c.token_count,
                w.token_count,
                "token_count mismatch for {}",
                c.path.display()
            );
            assert_eq!(
                c.metadata.size_bytes,
                w.metadata.size_bytes,
                "size_bytes mismatch for {}",
                c.path.display()
            );
            assert_eq!(
                c.metadata.language,
                w.metadata.language,
                "language mismatch for {}",
                c.path.display()
            );
            assert_eq!(
                c.simhash,
                w.simhash,
                "simhash mismatch for {}",
                c.path.display()
            );
        }
    }

    #[test]
    fn test_discover_cache_with_two_phase_mode() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "pub fn a() -> usize { 42 }").unwrap();

        let opts = DiscoveryOptions {
            skip_ast: true,
            retain_content: false,
            ..make_opts(tmp.path())
        };

        // Cold run — populates cache
        let entries1 = discover_files(&opts).unwrap();
        assert_eq!(entries1.len(), 1);
        assert!(entries1[0].ast.is_none());

        // Warm run — cache hit, AST should still be None
        let entries2 = discover_files(&opts).unwrap();
        assert_eq!(entries2.len(), 1);
        assert!(
            entries2[0].ast.is_none(),
            "AST must remain None when skip_ast is set, even on cache hit"
        );
    }

    #[test]
    fn test_discover_handles_corrupted_cache_gracefully() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();

        // Pre-create a corrupted cache file
        let cache_path = crate::index::cache::CacheManifest::cache_path(tmp.path());
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, b"{ this is garbage }").unwrap();

        // Discovery should succeed and overwrite the corrupted cache
        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries.len(), 1);

        // The cache file should now be valid JSON
        let bytes = std::fs::read(&cache_path).unwrap();
        let _: serde_json::Value =
            serde_json::from_slice(&bytes).expect("cache file should be valid JSON after rewrite");
    }

    #[test]
    fn test_discover_cache_partial_walk_drops_missing_entries() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(tmp.path().join("b.rs"), "fn b() {}").unwrap();
        std::fs::write(tmp.path().join("c.rs"), "fn c() {}").unwrap();

        // First run — 3 files cached
        let _ = discover_files(&make_opts(tmp.path())).unwrap();

        // Delete one file
        std::fs::remove_file(tmp.path().join("b.rs")).unwrap();

        // Second run — manifest should drop b.rs
        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries.len(), 2);

        let cfg_hash = crate::index::cache::config_hash(&make_opts(tmp.path()));
        let manifest = crate::index::cache::CacheManifest::load(tmp.path(), cfg_hash);
        assert_eq!(
            manifest.entries.len(),
            2,
            "manifest should be rewritten without b.rs"
        );
        let names: Vec<String> = manifest
            .entries
            .iter()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"a.rs".to_string()));
        assert!(names.contains(&"c.rs".to_string()));
        assert!(!names.contains(&"b.rs".to_string()));
    }
}
