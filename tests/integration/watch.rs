//! Integration tests for the file watcher.

use ctx_optim::watch::{is_watched_extension, should_ignore_path, start_watching};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn test_watcher_integration_create_and_modify() {
    let dir = TempDir::new().unwrap();
    let (tx, rx) = mpsc::channel();
    let _watcher = start_watching(dir.path(), tx).unwrap();

    // Create a Rust file
    std::fs::write(dir.path().join("test.rs"), "fn test() {}").unwrap();

    let mut events = Vec::new();
    for _ in 0..20 {
        if let Ok(paths) = rx.recv_timeout(Duration::from_millis(100)) {
            events.extend(paths);
        }
    }

    assert!(
        events
            .iter()
            .any(|p: &std::path::PathBuf| p.ends_with("test.rs")),
        "should detect test.rs creation: {events:?}"
    );
}

#[test]
fn test_extension_filtering_comprehensive() {
    let watched = ["main.rs", "app.ts", "lib.py", "server.go", "config.toml"];
    let ignored = ["logo.png", "Cargo.lock", "binary.exe", "data.csv"];

    for f in &watched {
        assert!(is_watched_extension(Path::new(f)), "{f} should be watched");
    }
    for f in &ignored {
        assert!(
            !is_watched_extension(Path::new(f)),
            "{f} should NOT be watched"
        );
    }
}

#[test]
fn test_ignore_path_comprehensive() {
    let ignored = [
        "target/debug/main",
        "node_modules/lodash/index.js",
        ".git/objects/abc123",
        "dist/bundle.js",
        "build/output.js",
    ];
    let watched = ["src/main.rs", "tests/test.rs", "benches/bench.rs"];

    for p in &ignored {
        assert!(should_ignore_path(Path::new(p)), "{p} should be ignored");
    }
    for p in &watched {
        assert!(
            !should_ignore_path(Path::new(p)),
            "{p} should NOT be ignored"
        );
    }
}
