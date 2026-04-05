# P3: Two-Phase Scoring with Lazy AST

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce pipeline latency by deferring AST parsing and content retention to post-selection. Only the ~K selected files get the expensive treatment, saving ~80% of AST+content work for medium-file repos.

**Architecture:** Add `skip_ast` and `retain_content` flags to `DiscoveryOptions`. When `focus_paths` is empty and `include_signatures` is false, `pack_files_with_options()` uses lite discovery (no AST, no content), runs cheap scoring + knapsack selection, then enriches only the selected files with content and AST. Falls back to full pipeline when focus_paths or signatures are needed.

**Tech Stack:** Rust, no new dependencies.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `src/index/discovery.rs` | Modify | Add `skip_ast`/`retain_content` flags, conditional AST/content in Phase 1b |
| `src/lib.rs` | Modify | Two-phase pipeline orchestration, `enrich_selected()` |
| `tests/fixtures/scenarios.rs` | Modify | Add `scale_test_medium()` scenario |
| `benches/realistic.rs` | Modify | Add medium-file pipeline benchmark |

---

### Task 1: Add `skip_ast` and `retain_content` flags to DiscoveryOptions

**Files:**
- Modify: `src/index/discovery.rs:39-73` (DiscoveryOptions struct + from_config)
- Modify: `src/index/discovery.rs:230-243` (WalkRecord struct)
- Modify: `src/index/discovery.rs:399-434` (Phase 1b parallel processing)
- Modify: `src/index/discovery.rs:493-510` (Phase 3 assembly)

- [ ] **Step 1: Add fields to `DiscoveryOptions`**

In `src/index/discovery.rs`, add two new fields to `DiscoveryOptions` after `shingle_size`:

```rust
    /// Skip AST parsing during discovery (for two-phase optimization).
    /// When true, all entries will have `ast: None`.
    pub skip_ast: bool,
    /// Retain raw file content in FileEntry for L3 caching.
    /// When false, all entries will have `content: None` (saves memory).
    pub retain_content: bool,
```

Update `from_config` to set defaults:

```rust
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
```

- [ ] **Step 2: Change `WalkRecord.content` from `Vec<u8>` to `Option<Vec<u8>>`**

In the `WalkRecord` struct (around line 230), change:

```rust
    content: Vec<u8>,
```

to:

```rust
    content: Option<Vec<u8>>,
```

- [ ] **Step 3: Update Phase 1b to conditionally skip AST and content**

In the `raw_files.into_par_iter().map(|raw| { ... })` closure (around line 399-434):

Capture the new flags at the top of Phase 1b (near where `compute_simhash` etc. are captured):

```rust
    let skip_ast = opts.skip_ast;
    let retain_content = opts.retain_content;
```

Replace the AST block:

Old:
```rust
            #[cfg(feature = "ast")]
            let ast = raw
                .language
                .and_then(|lang| super::ast::analyze_file(&raw.content, lang, max_ast_bytes));
            #[cfg(not(feature = "ast"))]
            let ast: Option<crate::types::AstData> = None;
```

New:
```rust
            #[cfg(feature = "ast")]
            let ast = if !skip_ast {
                raw.language
                    .and_then(|lang| super::ast::analyze_file(&raw.content, lang, max_ast_bytes))
            } else {
                None
            };
            #[cfg(not(feature = "ast"))]
            let ast: Option<crate::types::AstData> = None;
```

In the `Some(WalkRecord { ... })` construction, change:

```rust
                content: raw.content,
```

to:

```rust
                content: if retain_content { Some(raw.content) } else { None },
```

- [ ] **Step 4: Update Phase 3 assembly for `Option<Vec<u8>>`**

In the FileEntry assembly (around line 493-510), change:

```rust
            content: Some(rec.content),
```

to:

```rust
            content: rec.content,
```

(Since `rec.content` is now `Option<Vec<u8>>`, this passes through directly.)

- [ ] **Step 5: Update `make_opts` test helper**

In the test module's `make_opts` function, add the new fields:

```rust
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
```

- [ ] **Step 6: Add tests for the new flags**

Add to the test module in `src/index/discovery.rs`:

```rust
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
        assert!(entries[0].token_count > 0, "tokens should still be computed");
        assert_ne!(entries[0].hash, [0u8; 16], "hash should still be computed");
        assert!(entries[0].ast.is_none());
        assert!(entries[0].content.is_none());
    }
```

- [ ] **Step 7: Run `cargo check --all-features` and fix any errors**

Run: `cargo check --all-features 2>&1`
Expected: No errors.

- [ ] **Step 8: Run tests**

Run: `cargo nextest run --all-features 2>&1 | tail -30`
Expected: All tests pass.

- [ ] **Step 9: Commit**

```bash
git add src/index/discovery.rs
git commit -m "feat: add skip_ast and retain_content flags to DiscoveryOptions (P3)"
```

---

### Task 2: Two-phase pipeline orchestration + enrich_selected

**Files:**
- Modify: `src/lib.rs:104-293`

- [ ] **Step 1: Add `enrich_selected` function**

Add this function in `src/lib.rs`, before `pack_files_with_options`:

```rust
/// Enrich selected entries with file content and AST data.
///
/// Re-reads file bytes from disk (expected to be in OS page cache) and
/// optionally parses AST for signature extraction.  Called in two-phase
/// mode after knapsack selection to avoid processing all N files.
fn enrich_selected(selected: &mut [types::ScoredEntry], max_ast_bytes: usize) {
    for se in selected.iter_mut() {
        match std::fs::read(&se.entry.path) {
            Ok(bytes) => {
                #[cfg(feature = "ast")]
                {
                    if let Some(lang) = se.entry.metadata.language {
                        se.entry.ast =
                            index::ast::analyze_file(&bytes, lang, max_ast_bytes);
                    }
                }
                se.entry.content = Some(bytes);
            }
            Err(err) => {
                tracing::warn!(
                    "enrich: could not read {}: {err}",
                    se.entry.path.display()
                );
            }
        }
    }
}
```

Note: The `#[cfg(feature = "ast")]` block requires `index::ast` to be accessible. Since `ast` is declared as `pub mod ast` in `src/index/mod.rs` with `#[cfg(feature = "ast")]`, the import works.

- [ ] **Step 2: Modify `pack_files_with_options` for two-phase mode**

In `pack_files_with_options`, after `let repo = repo.as_ref();` and before the DiscoveryOptions construction, add:

```rust
    // Two-phase optimization: skip AST and content during discovery when
    // neither focus_paths (dependency signal) nor signatures (AST in L1)
    // are needed. Enrichment happens post-selection for the small set of
    // chosen files.
    let use_two_phase = focus_paths.is_empty() && !include_signatures;
```

Then modify the opts construction:

```rust
    let mut opts = DiscoveryOptions::from_config(config, repo);
    if use_two_phase {
        opts.skip_ast = true;
        opts.retain_content = false;
    }
```

After the knapsack selection and before the format outputs section, add enrichment:

```rust
    let mut selected = result.selected;

    // Two-phase: enrich only the selected files with content + AST
    if use_two_phase {
        enrich_selected(&mut selected, config.max_ast_bytes);
    }
```

Note: `selected` needs to be `mut` now. It was previously `let selected = result.selected;` — change to `let mut selected = result.selected;`.

- [ ] **Step 3: Verify compilation**

Run: `cargo check --all-features 2>&1`
Expected: No errors.

- [ ] **Step 4: Write test — two-phase produces correct L3 output**

Add to `src/lib.rs` at the bottom (or in a `#[cfg(test)] mod tests` block if one exists):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_pack_files_two_phase_produces_correct_l3() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        // Create a few files
        for i in 0..5 {
            std::fs::write(
                src.join(format!("mod_{i}.rs")),
                format!("pub fn func_{i}() -> usize {{ {i} }}"),
            )
            .unwrap();
        }

        let config = config::Config::default();
        let budget = types::Budget::standard(128_000);

        // No focus paths, no signatures → two-phase path
        let result = pack_files(tmp.path(), &budget, &[], &config).unwrap();

        assert!(result.stats.files_selected > 0);
        // L3 should contain file content (via enrichment)
        assert!(
            result.l3_output.contains("func_"),
            "L3 should contain enriched file content: {}",
            &result.l3_output[..result.l3_output.len().min(500)]
        );
    }

    #[test]
    fn test_pack_files_focus_paths_uses_full_pipeline() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        std::fs::write(src.join("main.rs"), "use crate::lib;\nfn main() {}").unwrap();
        std::fs::write(src.join("lib.rs"), "pub fn lib_fn() {}").unwrap();

        let config = config::Config::default();
        let budget = types::Budget::standard(128_000);
        let focus = vec![src.join("main.rs")];

        // With focus paths → full pipeline (not two-phase)
        let result = pack_files(tmp.path(), &budget, &focus, &config).unwrap();

        assert!(result.stats.files_selected > 0);
        assert!(result.l3_output.contains("main"));
    }

    #[test]
    fn test_pack_files_two_phase_l3_content_matches_disk() {
        let tmp = TempDir::new().unwrap();
        let expected = "pub fn exact_match() -> &'static str { \"hello\" }";
        std::fs::write(tmp.path().join("exact.rs"), expected).unwrap();

        let config = config::Config::default();
        let budget = types::Budget::standard(128_000);

        let result = pack_files(tmp.path(), &budget, &[], &config).unwrap();

        assert!(
            result.l3_output.contains("exact_match"),
            "L3 must contain the file content after enrichment"
        );
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo nextest run --all-features 2>&1 | tail -30`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs
git commit -m "perf: two-phase pipeline — defer AST and content to post-selection (P3)"
```

---

### Task 3: Add medium-file benchmark

**Files:**
- Modify: `tests/fixtures/scenarios.rs`
- Modify: `benches/realistic.rs`

- [ ] **Step 1: Add `scale_test_medium` scenario**

Add to `tests/fixtures/scenarios.rs` after the `scale_test` function:

```rust
// ── 6. Scale test with medium files (n files) ───────────────────────────────

/// Build a scale-test repository with medium-sized files (~233 tokens avg).
///
/// One third of files use [`FileSize::Medium`] (~500 tokens), the rest use
/// [`FileSize::Small`] (~100 tokens).  This distribution creates a tight
/// budget scenario where P3 two-phase scoring provides significant savings
/// (K ≪ N).
///
/// # Examples
///
/// ```no_run
/// use tests::fixtures::scenarios::scale_test_medium;
/// let repo = scale_test_medium(1000);
/// assert!(repo.path().join("src/mod_0").exists());
/// ```
pub fn scale_test_medium(n: usize) -> RealisticRepo {
    let mut builder = RealisticRepoBuilder::new().seed(6).initial_commit(30);

    for i in 0..n {
        let subdir = i % 10;
        let size = if i % 3 == 0 {
            FileSize::Medium
        } else {
            FileSize::Small
        };
        let path = format!("src/mod_{subdir}/file_{i}.rs");
        builder = builder.add_file(path, Lang::Rust, size);
    }

    builder.build()
}
```

- [ ] **Step 2: Add medium-file pipeline benchmark**

Add to `benches/realistic.rs`, before the `criterion_group!` macro:

```rust
/// Benchmark: full pipeline on scale_test_medium(5000).
/// P3 target: significant improvement over scale_test(5000) due to tight budget.
fn bench_full_pipeline_medium_files(c: &mut Criterion) {
    let repo = fixtures::scenarios::scale_test_medium(5_000);
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let mut group = c.benchmark_group("full_pipeline");
    group.sample_size(10);
    group.bench_function("5k_medium_files", |b| {
        b.iter(|| {
            pack_files(
                black_box(repo.path()),
                black_box(&budget),
                black_box(&[]),
                black_box(&config),
            )
            .expect("pack should not fail")
        });
    });
    group.finish();
}
```

Add `bench_full_pipeline_medium_files` to the `criterion_group!` invocation:

```rust
criterion_group!(
    benches,
    bench_discover_10k,
    bench_score_pack_200,
    bench_full_pipeline_medium,
    bench_full_pipeline_large,
    bench_full_pipeline_medium_files,
    bench_dedup_heavy,
    bench_focus_vs_no_focus,
);
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check --all-features --benches 2>&1`
Expected: No errors.

- [ ] **Step 4: Commit**

```bash
git add tests/fixtures/scenarios.rs benches/realistic.rs
git commit -m "bench: add medium-file scale test for P3 two-phase validation"
```

---

### Task 4: Verification and benchmarks

**Files:**
- No new files

- [ ] **Step 1: Run fmt + clippy**

Run: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings 2>&1`
Expected: Clean.

- [ ] **Step 2: Run full test suite**

Run: `cargo nextest run --all-features 2>&1 | tail -30`
Expected: All tests pass.

- [ ] **Step 3: Run pipeline benchmarks**

Run: `cargo bench --bench pipeline 2>&1`
Expected: `pack_pipeline/files/1000` should show some improvement (less content cloning).

- [ ] **Step 4: Run realistic benchmarks**

Run: `cargo bench --bench realistic 2>&1`
Expected:
- `full_pipeline/5k_files_scale` — minimal change (tiny files, K ≈ N)
- `full_pipeline/5k_medium_files` — significant improvement (medium files, K ≪ N)

- [ ] **Step 5: Update perf analysis doc**

Update `docs/superpowers/specs/2026-04-04-discovery-perf-analysis.md`:
- Change P3 status from planned to DONE
- Add actual benchmark numbers
- Add the 5k_medium_files benchmark row
- Update the projected performance stack table

- [ ] **Step 6: Commit doc update**

```bash
git add docs/superpowers/specs/2026-04-04-discovery-perf-analysis.md
git commit -m "docs: update perf analysis with P3 results"
```
