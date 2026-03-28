//! File-level dependency graph built from import statements.
//!
//! Used to compute graph distance from focus files for the dependency scoring signal.

use crate::types::{FileEntry, Language};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// A file-level dependency graph built from import statements.
///
/// Nodes are file paths; directed edges are import relationships.
/// [`DependencyGraph::distance`] computes BFS distance treating edges as undirected
/// (both "imports" and "imported by" count as one hop).
///
/// # Examples
/// ```no_run
/// use ctx_optim::index::depgraph::DependencyGraph;
/// use ctx_optim::types::FileEntry;
/// use std::path::PathBuf;
/// // let entries: Vec<FileEntry> = discover_files(...);
/// // let graph = DependencyGraph::build(&entries, std::path::Path::new("/repo"));
/// // let dist = graph.distance(&[PathBuf::from("/repo/src/lib.rs")], &PathBuf::from("/repo/src/scoring/mod.rs"));
/// ```
pub struct DependencyGraph {
    /// Forward edges: file -> files it imports.
    edges: HashMap<PathBuf, Vec<PathBuf>>,
    /// All known file paths (for quick existence checks).
    known_files: HashSet<PathBuf>,
}

impl DependencyGraph {
    /// Build the dependency graph from discovered file entries.
    ///
    /// Resolves raw import paths to actual file paths in the repository.
    /// Unresolvable imports are silently skipped.
    pub fn build(entries: &[FileEntry], repo_root: &Path) -> Self {
        let known_files: HashSet<PathBuf> = entries.iter().map(|e| e.path.clone()).collect();

        let mut edges: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();

        for entry in entries {
            let ast = match &entry.ast {
                Some(a) => a,
                None => continue,
            };

            let language = match entry.metadata.language {
                Some(l) => l,
                None => continue,
            };

            let mut targets = Vec::new();
            for imp in &ast.imports {
                if let Some(resolved) = resolve_import(
                    &imp.raw_path,
                    &entry.path,
                    language,
                    &known_files,
                    repo_root,
                ) {
                    targets.push(resolved);
                } else {
                    tracing::trace!(
                        file = %entry.path.display(),
                        import = %imp.raw_path,
                        "unresolved import"
                    );
                }
            }

            if !targets.is_empty() {
                edges.insert(entry.path.clone(), targets);
            }
        }

        Self { edges, known_files }
    }

    /// Compute the minimum graph distance from any focus file to the target.
    ///
    /// Uses BFS on undirected edges (both import and reverse-import).
    /// Returns `None` if the target is unreachable from all focus files.
    pub fn distance(&self, focus: &[PathBuf], target: &Path) -> Option<usize> {
        if focus.is_empty() {
            return None;
        }

        // Check if target is a focus file
        if focus.iter().any(|f| f == target) {
            return Some(0);
        }

        // Build reverse edges on the fly
        let mut reverse: HashMap<&Path, Vec<&Path>> = HashMap::new();
        for (src, dsts) in &self.edges {
            for dst in dsts {
                reverse
                    .entry(dst.as_path())
                    .or_default()
                    .push(src.as_path());
            }
        }

        // BFS from all focus files simultaneously
        let mut visited: HashSet<&Path> = HashSet::new();
        let mut queue: VecDeque<(&Path, usize)> = VecDeque::new();

        for f in focus {
            if self.known_files.contains(f) {
                visited.insert(f.as_path());
                queue.push_back((f.as_path(), 0));
            }
        }

        while let Some((node, dist)) = queue.pop_front() {
            // Forward edges
            if let Some(neighbors) = self.edges.get(node) {
                for n in neighbors {
                    if n.as_path() == target {
                        return Some(dist + 1);
                    }
                    if visited.insert(n.as_path()) {
                        queue.push_back((n.as_path(), dist + 1));
                    }
                }
            }

            // Reverse edges
            if let Some(neighbors) = reverse.get(node) {
                for &n in neighbors {
                    if n == target {
                        return Some(dist + 1);
                    }
                    if visited.insert(n) {
                        queue.push_back((n, dist + 1));
                    }
                }
            }
        }

        None
    }
}

/// Resolve a raw import string to a file path within the repository.
///
/// Language-specific resolution. Returns `None` for external/unresolvable imports.
fn resolve_import(
    raw_path: &str,
    source_file: &Path,
    language: Language,
    known_files: &HashSet<PathBuf>,
    repo_root: &Path,
) -> Option<PathBuf> {
    match language {
        Language::Rust => resolve_rust_import(raw_path, source_file, known_files, repo_root),
        Language::TypeScript | Language::JavaScript => {
            resolve_ts_import(raw_path, source_file, known_files)
        }
        Language::Python => resolve_python_import(raw_path, source_file, known_files),
        Language::Go => resolve_go_import(raw_path, known_files, repo_root),
        Language::Other => None,
    }
}

/// Resolve Rust `use` paths: `crate::foo::bar` -> `src/foo/bar.rs` or `src/foo/bar/mod.rs`.
fn resolve_rust_import(
    raw_path: &str,
    source_file: &Path,
    known_files: &HashSet<PathBuf>,
    repo_root: &Path,
) -> Option<PathBuf> {
    let path = raw_path.trim();

    // Handle crate::, super::, self:: prefixes
    if let Some(rest) = path.strip_prefix("crate::") {
        let parts: Vec<&str> = rest.split("::").collect();
        let file_path = parts.join("/");
        let src = repo_root.join("src");
        let candidate = src.join(format!("{file_path}.rs"));
        if known_files.contains(&candidate) {
            return Some(candidate);
        }
        let candidate = src.join(&file_path).join("mod.rs");
        if known_files.contains(&candidate) {
            return Some(candidate);
        }
    } else if let Some(rest) = path.strip_prefix("super::") {
        let parent = source_file.parent()?.parent()?;
        let parts: Vec<&str> = rest.split("::").collect();
        let file_path = parts.join("/");
        let candidate = parent.join(format!("{file_path}.rs"));
        if known_files.contains(&candidate) {
            return Some(candidate);
        }
        let candidate = parent.join(&file_path).join("mod.rs");
        if known_files.contains(&candidate) {
            return Some(candidate);
        }
    } else if let Some(rest) = path.strip_prefix("self::") {
        let parent = source_file.parent()?;
        let parts: Vec<&str> = rest.split("::").collect();
        let file_path = parts.join("/");
        let candidate = parent.join(format!("{file_path}.rs"));
        if known_files.contains(&candidate) {
            return Some(candidate);
        }
        let candidate = parent.join(&file_path).join("mod.rs");
        if known_files.contains(&candidate) {
            return Some(candidate);
        }
    }
    // External crate or std — skip
    None
}

/// Resolve TypeScript/JS imports: `"./utils"` -> try `.ts`, `.tsx`, `.js`, `/index.ts`.
fn resolve_ts_import(
    raw_path: &str,
    source_file: &Path,
    known_files: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    let path = raw_path.trim();

    // Only resolve relative imports
    if !path.starts_with('.') {
        return None;
    }

    let base = source_file.parent()?;
    let resolved = base.join(path);

    // Try with extensions
    for ext in &["ts", "tsx", "js", "jsx"] {
        let candidate = resolved.with_extension(ext);
        if known_files.contains(&candidate) {
            return Some(candidate);
        }
    }

    // Try as directory with index file
    for ext in &["ts", "tsx", "js"] {
        let candidate = resolved.join(format!("index.{ext}"));
        if known_files.contains(&candidate) {
            return Some(candidate);
        }
    }

    None
}

/// Resolve Python imports: relative imports resolved from source, absolute checked against known.
fn resolve_python_import(
    raw_path: &str,
    source_file: &Path,
    known_files: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    let path = raw_path.trim();

    if path.starts_with('.') {
        // Relative import: count dots, resolve from parent
        let dots = path.chars().take_while(|c| *c == '.').count();
        let rest = &path[dots..];

        let mut base = source_file.parent()?;
        for _ in 1..dots {
            base = base.parent()?;
        }

        let parts: Vec<&str> = if rest.is_empty() {
            vec![]
        } else {
            rest.split('.').collect()
        };
        let file_path = parts.join("/");

        if !file_path.is_empty() {
            let candidate = base.join(format!("{file_path}.py"));
            if known_files.contains(&candidate) {
                return Some(candidate);
            }
            let candidate = base.join(&file_path).join("__init__.py");
            if known_files.contains(&candidate) {
                return Some(candidate);
            }
        }
    } else {
        // Absolute import — check if it maps to a known file
        let parts: Vec<&str> = path.split('.').collect();
        let file_path = parts.join("/");

        // Check from the same root
        if let Some(root) = find_python_root(source_file) {
            let candidate = root.join(format!("{file_path}.py"));
            if known_files.contains(&candidate) {
                return Some(candidate);
            }
            let candidate = root.join(&file_path).join("__init__.py");
            if known_files.contains(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

/// Find the Python package root by walking up to find a directory without `__init__.py`.
fn find_python_root(file: &Path) -> Option<&Path> {
    let mut dir = file.parent()?;
    while dir.join("__init__.py").exists() {
        dir = dir.parent()?;
    }
    Some(dir)
}

/// Resolve Go imports: only local (non-url) imports.
fn resolve_go_import(
    raw_path: &str,
    known_files: &HashSet<PathBuf>,
    repo_root: &Path,
) -> Option<PathBuf> {
    let path = raw_path.trim();

    // Skip external imports (containing dots like "github.com/...")
    if path.contains('.') {
        return None;
    }

    // Try as a local package directory
    let pkg_dir = repo_root.join(path);
    // Find any .go file in that directory
    known_files
        .iter()
        .find(|f| f.parent() == Some(pkg_dir.as_path()) && f.extension().is_some_and(|e| e == "go"))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use std::time::SystemTime;

    fn make_entry(path: &str, language: Option<Language>, imports: Vec<&str>) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            token_count: 100,
            hash: [0u8; 16],
            metadata: FileMetadata {
                size_bytes: 400,
                last_modified: SystemTime::now(),
                git: None,
                language,
            },
            ast: Some(AstData {
                signatures: vec![],
                imports: imports
                    .into_iter()
                    .map(|p| ImportRef {
                        raw_path: p.to_string(),
                        line: 1,
                    })
                    .collect(),
            }),
            simhash: None,
        }
    }

    fn make_entry_no_ast(path: &str) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            token_count: 100,
            hash: [0u8; 16],
            metadata: FileMetadata {
                size_bytes: 400,
                last_modified: SystemTime::now(),
                git: None,
                language: None,
            },
            ast: None,
            simhash: None,
        }
    }

    #[test]
    fn test_build_empty() {
        let graph = DependencyGraph::build(&[], Path::new("/repo"));
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn test_distance_focus_to_self() {
        let entries = vec![make_entry_no_ast("/repo/src/lib.rs")];
        let graph = DependencyGraph::build(&entries, Path::new("/repo"));
        let dist = graph.distance(
            &[PathBuf::from("/repo/src/lib.rs")],
            Path::new("/repo/src/lib.rs"),
        );
        assert_eq!(dist, Some(0));
    }

    #[test]
    fn test_distance_unreachable() {
        let a = make_entry_no_ast("/repo/src/a.rs");
        let b = make_entry_no_ast("/repo/src/b.rs");
        let graph = DependencyGraph::build(&[a, b], Path::new("/repo"));
        let dist = graph.distance(
            &[PathBuf::from("/repo/src/a.rs")],
            Path::new("/repo/src/b.rs"),
        );
        assert_eq!(dist, None);
    }

    #[test]
    fn test_distance_direct_rust_import() {
        let a = make_entry(
            "/repo/src/lib.rs",
            Some(Language::Rust),
            vec!["crate::scoring::signals"],
        );
        let b = make_entry_no_ast("/repo/src/scoring/signals.rs");
        let graph = DependencyGraph::build(&[a, b], Path::new("/repo"));
        let dist = graph.distance(
            &[PathBuf::from("/repo/src/lib.rs")],
            Path::new("/repo/src/scoring/signals.rs"),
        );
        assert_eq!(dist, Some(1));
    }

    #[test]
    fn test_distance_reverse_edge() {
        // b imports a, so distance from a to b should be 1 (reverse edge)
        let a = make_entry_no_ast("/repo/src/a.rs");
        let b = make_entry("/repo/src/lib.rs", Some(Language::Rust), vec!["crate::a"]);
        let graph = DependencyGraph::build(&[a, b], Path::new("/repo"));
        let dist = graph.distance(
            &[PathBuf::from("/repo/src/a.rs")],
            Path::new("/repo/src/lib.rs"),
        );
        assert_eq!(dist, Some(1));
    }

    #[test]
    fn test_distance_transitive() {
        // a -> b -> c (two hops)
        let a = make_entry("/repo/src/a.rs", Some(Language::Rust), vec!["crate::b"]);
        let b = make_entry("/repo/src/b.rs", Some(Language::Rust), vec!["crate::c"]);
        let c = make_entry_no_ast("/repo/src/c.rs");
        let graph = DependencyGraph::build(&[a, b, c], Path::new("/repo"));
        let dist = graph.distance(
            &[PathBuf::from("/repo/src/a.rs")],
            Path::new("/repo/src/c.rs"),
        );
        assert_eq!(dist, Some(2));
    }

    #[test]
    fn test_distance_empty_focus() {
        let a = make_entry_no_ast("/repo/src/a.rs");
        let graph = DependencyGraph::build(&[a], Path::new("/repo"));
        let dist = graph.distance(&[], Path::new("/repo/src/a.rs"));
        assert_eq!(dist, None);
    }

    #[test]
    fn test_ts_relative_import_resolution() {
        let a = make_entry(
            "/repo/src/app.ts",
            Some(Language::TypeScript),
            vec!["./utils"],
        );
        let b = make_entry_no_ast("/repo/src/utils.ts");
        let graph = DependencyGraph::build(&[a, b], Path::new("/repo"));
        let dist = graph.distance(
            &[PathBuf::from("/repo/src/app.ts")],
            Path::new("/repo/src/utils.ts"),
        );
        assert_eq!(dist, Some(1));
    }
}
