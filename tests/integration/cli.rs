//! CLI integration tests using `assert_cmd`.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Create a small temp repo with a couple of Rust source files.
fn make_temp_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("main.rs"),
        "fn main() { println!(\"hello\"); }",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("lib.rs"),
        "/// Add two numbers.\npub fn add(a: i32, b: i32) -> i32 { a + b }",
    )
    .unwrap();
    dir
}

fn ctx_optim() -> Command {
    Command::cargo_bin("ctx-optim").unwrap()
}

// ── pack ──────────────────────────────────────────────────────────────────────

#[test]
fn test_pack_exits_zero() {
    let repo = make_temp_repo();
    ctx_optim()
        .args(["pack", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn test_pack_l1_output_contains_header() {
    let repo = make_temp_repo();
    ctx_optim()
        .args([
            "pack",
            "--repo",
            repo.path().to_str().unwrap(),
            "--output",
            "l1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("L1: File Skeleton Map"));
}

#[test]
fn test_pack_l2_output_contains_header() {
    let repo = make_temp_repo();
    ctx_optim()
        .args([
            "pack",
            "--repo",
            repo.path().to_str().unwrap(),
            "--output",
            "l2",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("L2: Dependency Cluster Expansion"));
}

#[test]
fn test_pack_l3_output_has_context_tags() {
    let repo = make_temp_repo();
    ctx_optim()
        .args([
            "pack",
            "--repo",
            repo.path().to_str().unwrap(),
            "--output",
            "l3",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("<context>"))
        .stdout(predicate::str::contains("</context>"));
}

#[test]
fn test_pack_stats_output_contains_expected_fields() {
    let repo = make_temp_repo();
    ctx_optim()
        .args([
            "pack",
            "--repo",
            repo.path().to_str().unwrap(),
            "--output",
            "stats",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Files scanned"))
        .stdout(predicate::str::contains("Tokens used"));
}

#[test]
fn test_pack_default_output_is_l3() {
    let repo = make_temp_repo();
    // Without --output flag, should default to l3
    ctx_optim()
        .args(["pack", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("<context>"));
}

#[test]
fn test_pack_nonexistent_repo_exits_nonzero() {
    ctx_optim()
        .args(["pack", "--repo", "/nonexistent/path/to/nowhere_xyz"])
        .assert()
        .failure();
}

#[test]
fn test_pack_with_focus_flag() {
    let repo = make_temp_repo();
    let lib_path = repo.path().join("lib.rs");
    ctx_optim()
        .args([
            "pack",
            "--repo",
            repo.path().to_str().unwrap(),
            "--focus",
            lib_path.to_str().unwrap(),
            "--output",
            "l1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("L1:"));
}

#[test]
fn test_pack_stdout_contains_no_tracing_output() {
    // Tracing must go to stderr only, never stdout (MCP invariant).
    let repo = make_temp_repo();
    let output = ctx_optim()
        .args([
            "pack",
            "--repo",
            repo.path().to_str().unwrap(),
            "--output",
            "l3",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Tracing lines start with timestamps like "2024-" or log levels
    assert!(
        !stdout.contains("INFO ") && !stdout.contains("DEBUG ") && !stdout.contains("WARN "),
        "tracing output leaked to stdout: {stdout}"
    );
}

// ── index ─────────────────────────────────────────────────────────────────────

#[test]
fn test_index_exits_zero() {
    let repo = make_temp_repo();
    ctx_optim()
        .args(["index", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn test_index_output_contains_file_count_and_tokens() {
    let repo = make_temp_repo();
    ctx_optim()
        .args(["index", "--repo", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Files:"))
        .stdout(predicate::str::contains("Total tokens:"));
}

#[test]
fn test_index_nonexistent_repo_exits_nonzero() {
    ctx_optim()
        .args(["index", "--repo", "/nonexistent/path/xyz_abc"])
        .assert()
        .failure();
}

#[test]
fn test_index_file_count_matches_repo_size() {
    let repo = make_temp_repo(); // 2 files: main.rs and lib.rs
    let output = ctx_optim()
        .args(["index", "--repo", repo.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The output should report 2 files
    assert!(
        stdout.contains("Files: 2"),
        "expected 'Files: 2' in: {stdout}"
    );
}

// ── help ──────────────────────────────────────────────────────────────────────

#[test]
fn test_help_exits_zero() {
    ctx_optim().arg("--help").assert().success();
}

#[test]
fn test_pack_help_exits_zero() {
    ctx_optim().args(["pack", "--help"]).assert().success();
}

#[test]
fn test_index_help_exits_zero() {
    ctx_optim().args(["index", "--help"]).assert().success();
}
