//! File watcher for incremental re-indexing.
//!
//! Uses the `notify` crate to detect filesystem changes and trigger
//! callbacks when source files are created, modified, or deleted.

use crate::error::OptimError;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// Source file extensions that trigger re-indexing.
const WATCHED_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "pyi", "go", "toml", "yaml", "yml", "json",
];

/// Directory names to ignore.
const IGNORED_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "dist",
    "build",
    "__pycache__",
];

/// Check if a file path has a watched extension.
///
/// # Examples
///
/// ```
/// use ctx_optim::watch::is_watched_extension;
/// assert!(is_watched_extension(std::path::Path::new("src/main.rs")));
/// assert!(!is_watched_extension(std::path::Path::new("image.png")));
/// ```
pub fn is_watched_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| WATCHED_EXTENSIONS.contains(&ext))
}

/// Check if a path falls under an ignored directory.
///
/// # Examples
///
/// ```
/// use ctx_optim::watch::should_ignore_path;
/// assert!(should_ignore_path(std::path::Path::new("target/debug/main")));
/// assert!(!should_ignore_path(std::path::Path::new("src/main.rs")));
/// ```
pub fn should_ignore_path(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| IGNORED_DIRS.contains(&s))
    })
}

/// Start watching a directory for source file changes.
///
/// Returns a `RecommendedWatcher` (must be kept alive) and sends batches
/// of changed file paths through the channel.
///
/// Only source files (by extension) are reported. Ignored directories
/// (target/, node_modules/, .git/) are filtered out.
///
/// # Examples
///
/// ```no_run
/// use ctx_optim::watch::start_watching;
/// use std::sync::mpsc;
/// let (tx, rx) = mpsc::channel();
/// let _watcher = start_watching(std::path::Path::new("."), tx).unwrap();
/// // rx will receive Vec<PathBuf> batches of changed source files
/// ```
pub fn start_watching(
    root: &Path,
    sender: mpsc::Sender<Vec<PathBuf>>,
) -> Result<RecommendedWatcher, OptimError> {
    let mut watcher = RecommendedWatcher::new(
        move |result: Result<Event, notify::Error>| {
            if let Ok(event) = result {
                match event.kind {
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                        let relevant: Vec<PathBuf> = event
                            .paths
                            .into_iter()
                            .filter(|p| is_watched_extension(p) && !should_ignore_path(p))
                            .collect();
                        if !relevant.is_empty() {
                            let _ = sender.send(relevant);
                        }
                    }
                    _ => {}
                }
            }
        },
        notify::Config::default().with_poll_interval(Duration::from_secs(1)),
    )
    .map_err(|e| OptimError::Watch(format!("create watcher: {e}")))?;

    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|e| OptimError::Watch(format!("watch {}: {e}", root.display())))?;

    tracing::info!("file watcher started on {}", root.display());
    Ok(watcher)
}

/// Run the file watcher loop, logging changed files.
///
/// This blocks the calling thread until the channel is closed.
/// Useful for the CLI `watch` command.
///
/// # Examples
///
/// ```no_run
/// use ctx_optim::watch::run_watch_loop;
/// run_watch_loop(std::path::Path::new(".")).unwrap();
/// ```
pub fn run_watch_loop(root: &Path) -> Result<(), OptimError> {
    let (tx, rx) = mpsc::channel();
    let _watcher = start_watching(root, tx)?;

    tracing::info!(
        "watching {} for source file changes (Ctrl+C to stop)",
        root.display()
    );

    loop {
        match rx.recv() {
            Ok(paths) => {
                for path in &paths {
                    tracing::info!("changed: {}", path.display());
                }
            }
            Err(_) => {
                tracing::info!("watcher channel closed");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_is_watched_extension() {
        assert!(is_watched_extension(Path::new("src/main.rs")));
        assert!(is_watched_extension(Path::new("app.ts")));
        assert!(is_watched_extension(Path::new("lib.py")));
        assert!(is_watched_extension(Path::new("main.go")));
        assert!(!is_watched_extension(Path::new("image.png")));
        assert!(!is_watched_extension(Path::new("binary.exe")));
        assert!(!is_watched_extension(Path::new("Cargo.lock")));
    }

    #[test]
    fn test_should_ignore_path() {
        assert!(should_ignore_path(Path::new("target/debug/main")));
        assert!(should_ignore_path(Path::new(
            "node_modules/lodash/index.js"
        )));
        assert!(should_ignore_path(Path::new(".git/objects/abc")));
        assert!(!should_ignore_path(Path::new("src/main.rs")));
    }

    #[test]
    fn test_watch_detects_file_creation() {
        let dir = TempDir::new().unwrap();
        let (tx, rx) = mpsc::channel();

        let _watcher = start_watching(dir.path(), tx).unwrap();

        // Create a file
        std::fs::write(dir.path().join("new_file.rs"), "fn main() {}").unwrap();

        // Wait for the event (with timeout)
        let mut found = false;
        for _ in 0..20 {
            if let Ok(paths) = rx.recv_timeout(Duration::from_millis(100)) {
                if paths.iter().any(|p| p.ends_with("new_file.rs")) {
                    found = true;
                    break;
                }
            }
        }
        assert!(found, "should detect new_file.rs creation");
    }

    #[test]
    fn test_watch_detects_file_modification() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("existing.rs");
        std::fs::write(&file, "fn old() {}").unwrap();

        let (tx, rx) = mpsc::channel();
        let _watcher = start_watching(dir.path(), tx).unwrap();

        // Modify the file
        std::fs::write(&file, "fn new() {}").unwrap();

        let mut found = false;
        for _ in 0..20 {
            if let Ok(paths) = rx.recv_timeout(Duration::from_millis(100)) {
                if paths.iter().any(|p| p.ends_with("existing.rs")) {
                    found = true;
                    break;
                }
            }
        }
        assert!(found, "should detect existing.rs modification");
    }

    #[test]
    fn test_watch_ignores_non_source_files() {
        let dir = TempDir::new().unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = start_watching(dir.path(), tx).unwrap();

        // Create a non-source file
        std::fs::write(dir.path().join("image.png"), b"PNG").unwrap();

        // Should NOT receive an event for this file
        let found = rx.recv_timeout(Duration::from_millis(500)).is_ok();
        assert!(!found, "should not report non-source file changes");
    }
}
