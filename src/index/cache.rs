//! Persistent on-disk discovery cache (P5).
//!
//! Stores a JSON manifest at `<repo>/.ctx-optim/cache/discovery.json`
//! containing the [`FileEntry`] vec from the last successful
//! [`discover_files`] call, plus a config hash for invalidation.
//!
//! On a warm second run, files whose `(rel_path, mtime_secs, size_bytes)`
//! match a cached entry skip the file read AND all of Phase 1b
//! (tokenize/hash/AST/SimHash) AND Phase 2 (git revwalk).  The performance
//! win is large because the entire downstream pipeline is bypassed for
//! unchanged files.
//!
//! ## Invalidation rules
//! - Schema version mismatch  → empty manifest, full re-walk
//! - Config hash mismatch     → empty manifest, full re-walk
//! - Repo root mismatch       → empty manifest, full re-walk
//! - File mtime+size mismatch → that file re-processed
//! - File missing from walk   → cached entry dropped (deletion)
//! - File missing from cache  → new file processed normally
//!
//! ## Bumping the version
//! Increment [`CACHE_VERSION`] any time `FileEntry`, `FileMetadata`,
//! `GitMetadata`, `AstData`, `Signature`, `ImportRef`, `Language`, or
//! `SymbolKind` changes shape on the wire.  Existing caches will be
//! transparently invalidated and rebuilt.
//!
//! ## Thread safety
//! Atomic writes via `tmp + rename`.  Concurrent readers either see the
//! old or the new version of the file.  Last writer wins on concurrent
//! writes.  No flock needed.
//!
//! ## Git metadata staleness
//! `GitMetadata.age_days` is computed at write time as `now - latest_commit`,
//! so cache hits return slightly stale `age_days` values.  This matches the
//! existing in-memory `GIT_CACHE` behavior.  The recency signal has smooth
//! decay so a few days drift is tolerable.  Users can `rm` the cache file
//! to force a fresh git walk.
//!
//! [`discover_files`]: super::discovery::discover_files
//! [`FileEntry`]: crate::types::FileEntry

use crate::index::discovery::DiscoveryOptions;
use crate::types::FileEntry;
use siphasher::sip::SipHasher13;
use std::collections::HashMap;
use std::hash::Hasher;
use std::path::{Path, PathBuf};

/// On-disk schema version.  Bump on any breaking change to `FileEntry`
/// or its nested types.
pub const CACHE_VERSION: u32 = 1;

/// Subdirectory under the repo root that holds the cache file.
const CACHE_SUBDIR: &str = ".ctx-optim/cache";

/// Cache file name within [`CACHE_SUBDIR`].
const CACHE_FILE_NAME: &str = "discovery.json";

/// On-disk manifest containing the cached discovery results.
///
/// Paths in `entries` are stored **relative** to `repo_root` for
/// portability across moved repos and `git worktree` setups.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct CacheManifest {
    /// Schema version (see [`CACHE_VERSION`]).
    pub version: u32,
    /// Hash of discovery-relevant config fields (see [`config_hash`]).
    pub config_hash: u64,
    /// Repo workdir at the time the cache was written.
    pub repo_root: PathBuf,
    /// Cached file entries with paths relative to `repo_root`.
    pub entries: Vec<FileEntry>,
}

impl CacheManifest {
    /// Compute the on-disk path for the cache file under `repo_root`.
    ///
    /// # Examples
    /// ```
    /// use ctx_optim::index::cache::CacheManifest;
    /// use std::path::Path;
    /// let p = CacheManifest::cache_path(Path::new("/repo"));
    /// assert!(p.ends_with(".ctx-optim/cache/discovery.json"));
    /// ```
    pub fn cache_path(repo_root: &Path) -> PathBuf {
        repo_root.join(CACHE_SUBDIR).join(CACHE_FILE_NAME)
    }

    /// Build an empty manifest tagged with the current version and the
    /// given `config_hash`.  Used as the fallback when load fails.
    pub fn empty(repo_root: &Path, config_hash: u64) -> Self {
        Self {
            version: CACHE_VERSION,
            config_hash,
            repo_root: repo_root.to_path_buf(),
            entries: Vec::new(),
        }
    }

    /// Load the manifest from disk.
    ///
    /// Returns an empty manifest on **any** error (missing file, parse
    /// failure, version mismatch, config hash mismatch, repo moved).
    /// Never panics, never propagates errors — caching is best-effort.
    pub fn load(repo_root: &Path, expected_config_hash: u64) -> Self {
        match Self::try_load(repo_root, expected_config_hash) {
            Ok(m) => m,
            Err(reason) => {
                tracing::debug!(
                    "discovery cache miss at {}: {reason}",
                    Self::cache_path(repo_root).display()
                );
                Self::empty(repo_root, expected_config_hash)
            }
        }
    }

    fn try_load(repo_root: &Path, expected_config_hash: u64) -> Result<Self, String> {
        let path = Self::cache_path(repo_root);
        let bytes = std::fs::read(&path).map_err(|e| format!("read: {e}"))?;
        let manifest: Self = serde_json::from_slice(&bytes).map_err(|e| format!("parse: {e}"))?;
        if manifest.version != CACHE_VERSION {
            return Err(format!(
                "version mismatch: file has {} but current is {}",
                manifest.version, CACHE_VERSION
            ));
        }
        if manifest.config_hash != expected_config_hash {
            return Err("config hash mismatch".into());
        }
        if manifest.repo_root != repo_root {
            return Err(format!(
                "repo moved from {} to {}",
                manifest.repo_root.display(),
                repo_root.display()
            ));
        }
        Ok(manifest)
    }

    /// Build a manifest from a fresh discover result.
    ///
    /// Paths are stored relative to `repo_root`.  Entries whose path is
    /// not within `repo_root` are silently dropped (they cannot be
    /// rehydrated against the same root anyway).  The `content` field is
    /// defensively cleared (it should already be `None` due to
    /// `#[serde(skip_serializing)]`, but this is belt-and-suspenders).
    pub fn from_entries(entries: &[FileEntry], repo_root: &Path, config_hash: u64) -> Self {
        let rel_entries = entries
            .iter()
            .filter_map(|entry| {
                let rel = entry.path.strip_prefix(repo_root).ok()?.to_path_buf();
                let mut clone = entry.clone();
                clone.path = rel;
                clone.content = None;
                Some(clone)
            })
            .collect();
        Self {
            version: CACHE_VERSION,
            config_hash,
            repo_root: repo_root.to_path_buf(),
            entries: rel_entries,
        }
    }

    /// Convert the manifest into a lookup table keyed by **absolute** path.
    ///
    /// The walker compares against `dir_entry.path()` which is absolute,
    /// so we rebuild the abs path here once instead of doing it per
    /// lookup.
    pub fn into_lookup(self) -> HashMap<PathBuf, FileEntry> {
        let repo_root = self.repo_root;
        self.entries
            .into_iter()
            .map(|mut entry| {
                let abs = repo_root.join(&entry.path);
                entry.path = abs.clone();
                (abs, entry)
            })
            .collect()
    }

    /// Persist the manifest atomically.
    ///
    /// Writes to `discovery.json.tmp` then renames over the target.  On
    /// POSIX `rename(2)` is atomic, so concurrent readers either see the
    /// old or the new version.  Errors are logged at warn level but
    /// never propagated — caching is best-effort.
    pub fn save(&self, repo_root: &Path) {
        if let Err(e) = self.try_save(repo_root) {
            tracing::warn!("failed to save discovery cache: {e}");
        }
    }

    fn try_save(&self, repo_root: &Path) -> std::io::Result<()> {
        let path = Self::cache_path(repo_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// Compute a stable hash of the discovery options that affect cache validity.
///
/// Stable across process restarts (fixed SipHasher key).  Includes
/// `CACHE_VERSION` so a version bump implicitly invalidates all caches
/// without relying on the version-mismatch path alone.
///
/// Excluded fields:
/// - `root` — caches are stored per repo root, not keyed by it
/// - `retain_content` — controls in-memory behavior only; the cached
///   manifest never persists `content`
///
/// # Examples
///
/// ```
/// use ctx_optim::config::Config;
/// use ctx_optim::index::cache::config_hash;
/// use ctx_optim::index::discovery::DiscoveryOptions;
///
/// let opts = DiscoveryOptions::from_config(&Config::default(), ".");
/// assert_eq!(config_hash(&opts), config_hash(&opts));
/// ```
pub fn config_hash(opts: &DiscoveryOptions) -> u64 {
    let mut h = SipHasher13::new_with_keys(0xC7F1_A35E, 0x9D24_B08C);
    h.write_u32(CACHE_VERSION);
    h.write_u64(opts.max_file_bytes);
    h.write_usize(opts.max_file_tokens);
    h.write_usize(opts.max_ast_bytes);
    h.write_u8(opts.compute_simhash as u8);
    h.write_usize(opts.shingle_size);
    h.write_u8(opts.skip_ast as u8);
    // Length-prefixed + zero-delimited so ["a","bc"] != ["ab","c"]
    h.write_usize(opts.include_extensions.len());
    for ext in &opts.include_extensions {
        h.write(ext.as_bytes());
        h.write_u8(0);
    }
    h.write_usize(opts.extra_ignore.len());
    for p in &opts.extra_ignore {
        h.write(p.as_os_str().as_encoded_bytes());
        h.write_u8(0);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::types::{
        AstData, FileMetadata, GitMetadata, ImportRef, Language, Signature, SymbolKind,
    };
    use std::time::{Duration, UNIX_EPOCH};
    use tempfile::TempDir;

    fn make_entry_full(rel_path: &str) -> FileEntry {
        FileEntry {
            path: PathBuf::from(rel_path),
            token_count: 42,
            hash: [0xAB; 16],
            metadata: FileMetadata {
                size_bytes: 256,
                last_modified: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
                git: Some(GitMetadata {
                    age_days: 3.5,
                    commit_count: 7,
                }),
                language: Some(Language::Rust),
            },
            ast: Some(AstData {
                signatures: vec![Signature {
                    kind: SymbolKind::Function,
                    text: "pub fn foo()".into(),
                    line: 1,
                }],
                imports: vec![ImportRef {
                    raw_path: "std::io".into(),
                    line: 2,
                }],
            }),
            simhash: Some(0xDEAD_BEEF_CAFE_BABE),
            content: None,
        }
    }

    fn make_entry_minimal(rel_path: &str) -> FileEntry {
        FileEntry {
            path: PathBuf::from(rel_path),
            token_count: 1,
            hash: [0u8; 16],
            metadata: FileMetadata {
                size_bytes: 1,
                last_modified: UNIX_EPOCH,
                git: None,
                language: None,
            },
            ast: None,
            simhash: None,
            content: None,
        }
    }

    fn make_opts(root: &Path) -> DiscoveryOptions {
        DiscoveryOptions::from_config(&Config::default(), root)
    }

    // ── CacheManifest tests ──────────────────────────────────────────────

    #[test]
    fn test_cache_path_under_repo_root() {
        let p = CacheManifest::cache_path(Path::new("/foo/bar"));
        assert!(p.ends_with(".ctx-optim/cache/discovery.json"));
        assert!(p.starts_with("/foo/bar"));
    }

    #[test]
    fn test_cache_roundtrip_preserves_all_fields() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Use abs paths so from_entries strips the prefix
        let e1 = {
            let mut e = make_entry_full("src/lib.rs");
            e.path = root.join("src/lib.rs");
            e
        };
        let e2 = {
            let mut e = make_entry_minimal("README.md");
            e.path = root.join("README.md");
            e
        };
        let manifest = CacheManifest::from_entries(&[e1.clone(), e2.clone()], root, 0xCAFE);
        manifest.save(root);

        let loaded = CacheManifest::load(root, 0xCAFE);
        assert_eq!(loaded.version, CACHE_VERSION);
        assert_eq!(loaded.config_hash, 0xCAFE);
        assert_eq!(loaded.entries.len(), 2);

        let lookup = loaded.into_lookup();
        let e1_loaded = lookup.get(&root.join("src/lib.rs")).expect("e1 loaded");
        assert_eq!(e1_loaded.token_count, e1.token_count);
        assert_eq!(e1_loaded.hash, e1.hash);
        assert_eq!(e1_loaded.metadata.size_bytes, e1.metadata.size_bytes);
        assert_eq!(e1_loaded.metadata.last_modified, e1.metadata.last_modified);
        assert!(e1_loaded.metadata.git.is_some());
        assert_eq!(e1_loaded.metadata.language, Some(Language::Rust));
        assert!(e1_loaded.ast.is_some());
        assert_eq!(e1_loaded.simhash, Some(0xDEAD_BEEF_CAFE_BABE));

        let e2_loaded = lookup.get(&root.join("README.md")).expect("e2 loaded");
        assert!(e2_loaded.metadata.git.is_none());
        assert!(e2_loaded.ast.is_none());
        assert!(e2_loaded.simhash.is_none());
    }

    #[test]
    fn test_cache_load_missing_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let m = CacheManifest::load(tmp.path(), 0);
        assert_eq!(m.version, CACHE_VERSION);
        assert!(m.entries.is_empty());
    }

    #[test]
    fn test_cache_load_corrupted_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let path = CacheManifest::cache_path(tmp.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ this is not valid json").unwrap();

        let m = CacheManifest::load(tmp.path(), 0);
        assert!(m.entries.is_empty());
    }

    #[test]
    fn test_cache_load_version_mismatch_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let mut manifest = CacheManifest::from_entries(&[make_entry_minimal("a.rs")], root, 0xAA);
        manifest.version = 999; // simulate old/new schema
        manifest.save(root);

        let loaded = CacheManifest::load(root, 0xAA);
        assert!(loaded.entries.is_empty());
    }

    #[test]
    fn test_cache_load_config_hash_mismatch_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let entry = {
            let mut e = make_entry_minimal("a.rs");
            e.path = root.join("a.rs");
            e
        };
        CacheManifest::from_entries(&[entry], root, 0x1111).save(root);

        // Load with different hash
        let loaded = CacheManifest::load(root, 0x2222);
        assert!(loaded.entries.is_empty());
    }

    #[test]
    fn test_cache_repo_moved_invalidates() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        let root1 = tmp1.path();
        let root2 = tmp2.path();

        // Write cache for tmp1
        let entry = {
            let mut e = make_entry_minimal("a.rs");
            e.path = root1.join("a.rs");
            e
        };
        CacheManifest::from_entries(&[entry], root1, 0x55).save(root1);

        // Copy the cache file from tmp1 to tmp2 (simulating a moved repo)
        let src_path = CacheManifest::cache_path(root1);
        let dst_path = CacheManifest::cache_path(root2);
        std::fs::create_dir_all(dst_path.parent().unwrap()).unwrap();
        std::fs::copy(&src_path, &dst_path).unwrap();

        // Loading from tmp2 should detect the mismatch
        let loaded = CacheManifest::load(root2, 0x55);
        assert!(
            loaded.entries.is_empty(),
            "moved repo should invalidate cache"
        );
    }

    #[test]
    fn test_cache_save_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // .ctx-optim/cache/ does not exist yet
        assert!(!root.join(".ctx-optim").exists());

        CacheManifest::empty(root, 0).save(root);

        assert!(CacheManifest::cache_path(root).exists());
    }

    #[test]
    fn test_cache_save_atomic_no_temp_left_behind() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        CacheManifest::empty(root, 0).save(root);

        let cache_dir = root.join(".ctx-optim/cache");
        let entries: Vec<_> = std::fs::read_dir(&cache_dir).unwrap().collect();
        let names: Vec<String> = entries
            .iter()
            .map(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(names.contains(&"discovery.json".to_string()));
        assert!(
            !names.iter().any(|n| n.ends_with(".tmp")),
            "tmp file leaked: {names:?}"
        );
    }

    #[test]
    fn test_cache_paths_stored_relatively() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let abs_path = root.join("src/lib.rs");
        let entry = FileEntry {
            path: abs_path,
            ..make_entry_minimal("placeholder")
        };
        CacheManifest::from_entries(&[entry], root, 0).save(root);

        // Read the raw JSON and verify the path is relative
        let bytes = std::fs::read(CacheManifest::cache_path(root)).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let stored_path = json["entries"][0]["path"].as_str().unwrap();
        assert_eq!(
            stored_path, "src/lib.rs",
            "expected relative path, got {stored_path}"
        );
        assert!(!stored_path.starts_with('/'));
    }

    #[test]
    fn test_cache_does_not_persist_content() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let entry = FileEntry {
            path: root.join("a.rs"),
            content: Some(b"this should never be persisted".to_vec()),
            ..make_entry_minimal("a.rs")
        };
        CacheManifest::from_entries(&[entry], root, 0).save(root);

        let bytes = std::fs::read(CacheManifest::cache_path(root)).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("this should never be persisted"),
            "raw content leaked into cache: {text}"
        );

        let loaded = CacheManifest::load(root, 0);
        assert!(loaded.entries[0].content.is_none());
    }

    #[test]
    fn test_into_lookup_uses_absolute_paths() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let entry = {
            let mut e = make_entry_minimal("nested/file.rs");
            e.path = root.join("nested/file.rs");
            e
        };
        let manifest = CacheManifest::from_entries(&[entry], root, 0);
        let lookup = manifest.into_lookup();

        assert_eq!(lookup.len(), 1);
        let key = root.join("nested/file.rs");
        assert!(lookup.contains_key(&key));
        assert!(lookup.keys().all(|k| k.is_absolute()));
    }

    #[test]
    fn test_from_entries_drops_paths_outside_repo_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let inside = {
            let mut e = make_entry_minimal("inside.rs");
            e.path = root.join("inside.rs");
            e
        };
        let outside = {
            let mut e = make_entry_minimal("outside.rs");
            e.path = PathBuf::from("/totally/elsewhere/outside.rs");
            e
        };
        let manifest = CacheManifest::from_entries(&[inside, outside], root, 0);
        assert_eq!(
            manifest.entries.len(),
            1,
            "entries outside repo_root should be dropped"
        );
    }

    // ── config_hash tests ────────────────────────────────────────────────

    #[test]
    fn test_config_hash_stable_across_calls() {
        let tmp = TempDir::new().unwrap();
        let opts = make_opts(tmp.path());
        let h1 = config_hash(&opts);
        let h2 = config_hash(&opts);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_config_hash_changes_on_max_file_bytes() {
        let tmp = TempDir::new().unwrap();
        let mut a = make_opts(tmp.path());
        let mut b = make_opts(tmp.path());
        a.max_file_bytes = 1024;
        b.max_file_bytes = 2048;
        assert_ne!(config_hash(&a), config_hash(&b));
    }

    #[test]
    fn test_config_hash_changes_on_max_file_tokens() {
        let tmp = TempDir::new().unwrap();
        let mut a = make_opts(tmp.path());
        let mut b = make_opts(tmp.path());
        a.max_file_tokens = 1000;
        b.max_file_tokens = 2000;
        assert_ne!(config_hash(&a), config_hash(&b));
    }

    #[test]
    fn test_config_hash_changes_on_max_ast_bytes() {
        let tmp = TempDir::new().unwrap();
        let mut a = make_opts(tmp.path());
        let mut b = make_opts(tmp.path());
        a.max_ast_bytes = 1024;
        b.max_ast_bytes = 2048;
        assert_ne!(config_hash(&a), config_hash(&b));
    }

    #[test]
    fn test_config_hash_changes_on_compute_simhash() {
        let tmp = TempDir::new().unwrap();
        let mut a = make_opts(tmp.path());
        let mut b = make_opts(tmp.path());
        a.compute_simhash = false;
        b.compute_simhash = true;
        assert_ne!(config_hash(&a), config_hash(&b));
    }

    #[test]
    fn test_config_hash_changes_on_shingle_size() {
        let tmp = TempDir::new().unwrap();
        let mut a = make_opts(tmp.path());
        let mut b = make_opts(tmp.path());
        a.shingle_size = 3;
        b.shingle_size = 5;
        assert_ne!(config_hash(&a), config_hash(&b));
    }

    #[test]
    fn test_config_hash_changes_on_skip_ast() {
        let tmp = TempDir::new().unwrap();
        let mut a = make_opts(tmp.path());
        let mut b = make_opts(tmp.path());
        a.skip_ast = false;
        b.skip_ast = true;
        assert_ne!(config_hash(&a), config_hash(&b));
    }

    #[test]
    fn test_config_hash_changes_on_include_extensions() {
        let tmp = TempDir::new().unwrap();
        let mut a = make_opts(tmp.path());
        let mut b = make_opts(tmp.path());
        a.include_extensions = vec![];
        b.include_extensions = vec!["rs".to_string()];
        assert_ne!(config_hash(&a), config_hash(&b));
    }

    #[test]
    fn test_config_hash_changes_on_extra_ignore() {
        let tmp = TempDir::new().unwrap();
        let mut a = make_opts(tmp.path());
        let mut b = make_opts(tmp.path());
        a.extra_ignore = vec![];
        b.extra_ignore = vec![PathBuf::from(".git")];
        assert_ne!(config_hash(&a), config_hash(&b));
    }

    #[test]
    fn test_config_hash_independent_of_retain_content() {
        let tmp = TempDir::new().unwrap();
        let mut a = make_opts(tmp.path());
        let mut b = make_opts(tmp.path());
        a.retain_content = false;
        b.retain_content = true;
        assert_eq!(
            config_hash(&a),
            config_hash(&b),
            "retain_content should not affect cache validity"
        );
    }

    #[test]
    fn test_config_hash_extension_collision_avoided() {
        // ["a","bc"] should hash differently from ["ab","c"]
        let tmp = TempDir::new().unwrap();
        let mut a = make_opts(tmp.path());
        let mut b = make_opts(tmp.path());
        a.include_extensions = vec!["a".to_string(), "bc".to_string()];
        b.include_extensions = vec!["ab".to_string(), "c".to_string()];
        assert_ne!(
            config_hash(&a),
            config_hash(&b),
            "delimited hashing must prevent collision"
        );
    }
}
