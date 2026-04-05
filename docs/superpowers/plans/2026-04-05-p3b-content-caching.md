# P3b: Content Caching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate redundant disk re-reads in `format_l3()` by caching file content in `FileEntry` during discovery.

**Architecture:** Add `content: Option<Vec<u8>>` to `FileEntry`. Discovery populates it from the already-read `RawFile.content`. `format_l3()` uses cached content when present, falls back to `fs::read_to_string` for entries without cached content. All other `FileEntry` construction sites pass `content: None`.

**Tech Stack:** Rust, no new dependencies.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `src/types.rs` | Modify | Add `content` field to `FileEntry` |
| `src/index/discovery.rs` | Modify | Populate `content` from `RawFile` during assembly |
| `src/output/format.rs` | Modify | Use cached content in `format_l3` |
| `src/scoring/mod.rs` | Modify | Add `content: None` to test helpers |
| `src/scoring/signals.rs` | Modify | Add `content: None` to test helpers |
| `src/selection/knapsack.rs` | Modify | Add `content: None` to test helpers |
| `src/selection/diversity.rs` | Modify | Add `content: None` to test helpers |
| `src/index/dedup.rs` | Modify | Add `content: None` to test/doc helpers |
| `src/index/depgraph.rs` | Modify | Add `content: None` to test helpers |
| `benches/scoring.rs` | Modify | Add `content: None` to bench helpers |
| `benches/knapsack.rs` | Modify | Add `content: None` to bench helpers |

---

### Task 1: Add `content` field to `FileEntry`

**Files:**
- Modify: `src/types.rs:107-123`

- [ ] **Step 1: Add the `content` field to `FileEntry`**

In `src/types.rs`, add after the `simhash` field:

```rust
    /// Cached raw file content from discovery, used by L3 output formatting.
    /// Populated during `discover_files()` to avoid redundant disk reads.
    /// Skipped during serialization (ephemeral pipeline data).
    #[serde(skip)]
    pub content: Option<Vec<u8>>,
```

- [ ] **Step 2: Update doc example for `score_entry` in `scoring/mod.rs`**

The doc example at `src/scoring/mod.rs:25-37` constructs a `FileEntry`. Add `content: None,` after `simhash: None,`.

- [ ] **Step 3: Update doc example for `dedup_by_hash` in `dedup.rs`**

The doc example at `src/index/dedup.rs:76` constructs a `FileEntry`. Add `content: None,` after `simhash: None,`.

- [ ] **Step 4: Update doc example for `ScoredEntry::efficiency` in `types.rs`**

The doc example at `src/types.rs:159` constructs a `FileEntry`. Add `content: None,` after `simhash: None,`.

- [ ] **Step 5: Run `cargo check --all-features` to find remaining construction sites**

Run: `cargo check --all-features 2>&1 | head -80`
Expected: Compiler errors listing every `FileEntry { ... }` missing the `content` field.

- [ ] **Step 6: Fix ALL remaining `FileEntry` construction sites**

Every `FileEntry { ... }` literal across the codebase needs `content: None,`. The known sites (from grep) are:

- `src/types.rs:310` — `make_scored` test helper
- `src/scoring/mod.rs:166` — `make_entry` test helper
- `src/scoring/signals.rs:245,274` — test `FileEntry` literals
- `src/selection/knapsack.rs:42,426,522` — doc example + test helpers
- `src/selection/diversity.rs:143` — test helper
- `src/index/dedup.rs:185,215` — test closures
- `src/index/depgraph.rs:346,371` — test helpers
- `src/output/format.rs:21,284` — doc example + test helper
- `benches/scoring.rs:10` — bench helper
- `benches/knapsack.rs:13` — bench helper

Add `content: None,` after the `simhash` field in each.

- [ ] **Step 7: Verify compilation**

Run: `cargo check --all-features 2>&1`
Expected: No errors.

- [ ] **Step 8: Run tests**

Run: `cargo nextest run --all-features 2>&1 | tail -20`
Expected: All tests pass (content is `None` everywhere, no behavior change).

- [ ] **Step 9: Commit**

```bash
git add src/types.rs src/scoring/mod.rs src/scoring/signals.rs src/selection/knapsack.rs \
  src/selection/diversity.rs src/index/dedup.rs src/index/depgraph.rs src/output/format.rs \
  benches/scoring.rs benches/knapsack.rs
git commit -m "feat: add content field to FileEntry for L3 caching (P3b)"
```

---

### Task 2: Populate content during discovery

**Files:**
- Modify: `src/index/discovery.rs:421,494`

- [ ] **Step 1: Add `content` to `WalkRecord`**

In `src/index/discovery.rs`, the `WalkRecord` struct (around line 230) needs a new field. Add after `simhash`:

```rust
    content: Vec<u8>,
```

- [ ] **Step 2: Pass content through Phase 1b parallel processing**

In the `raw_files.into_par_iter().map(|raw| { ... })` closure (around line 400-432), the `Some(WalkRecord { ... })` construction needs to include the content. Change the closure to move `raw.content` into the record. The key change is that `raw.content` is currently borrowed for hashing/tokenizing/AST, then discarded. Instead, retain it:

The current code reads `&raw.content` for `estimate_tokens_bytes`, `md5_hash`, and `analyze_file`. After those, add `content: raw.content,` to the `WalkRecord`:

```rust
            Some(WalkRecord {
                path: raw.path,
                size_bytes: raw.size_bytes,
                last_modified: raw.last_modified,
                token_count,
                hash,
                language: raw.language,
                ast,
                simhash,
                content: raw.content,
            })
```

- [ ] **Step 3: Pass content into `FileEntry` during Phase 3 assembly**

In the final assembly (around line 491-507), the `FileEntry` construction currently does not include content. Add it:

```rust
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
            content: Some(rec.content),
        })
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check --all-features 2>&1`
Expected: No errors.

- [ ] **Step 5: Write test — discovered files have content populated**

Add to `src/index/discovery.rs` tests module:

```rust
    #[test]
    fn test_discover_populates_content() {
        let tmp = TempDir::new().unwrap();
        let expected = "fn cached() { let x = 42; }";
        std::fs::write(tmp.path().join("cached.rs"), expected).unwrap();

        let entries = discover_files(&make_opts(tmp.path())).unwrap();
        assert_eq!(entries.len(), 1);
        let content = entries[0].content.as_ref().expect("content should be populated");
        assert_eq!(
            std::str::from_utf8(content).unwrap(),
            expected,
            "cached content should match file on disk"
        );
    }
```

- [ ] **Step 6: Run tests**

Run: `cargo nextest run --all-features 2>&1 | tail -20`
Expected: All tests pass including the new `test_discover_populates_content`.

- [ ] **Step 7: Commit**

```bash
git add src/index/discovery.rs
git commit -m "feat: retain file content in FileEntry during discovery (P3b)"
```

---

### Task 3: Use cached content in `format_l3`

**Files:**
- Modify: `src/output/format.rs:150-185`

- [ ] **Step 1: Write test — format_l3 uses cached content without disk read**

Add to `src/output/format.rs` tests module. This test creates a `ScoredEntry` with cached content pointing to a non-existent path — proving `format_l3` uses the cache, not disk:

```rust
    #[test]
    fn test_format_l3_uses_cached_content() {
        // Path does NOT exist on disk — format_l3 must use cached content
        let mut entry = make_scored("/nonexistent/cached_test.rs", 10, 0.8);
        entry.entry.content = Some(b"fn from_cache() {}".to_vec());

        let out = format_l3(&[entry]);
        assert!(
            out.contains("fn from_cache()"),
            "L3 should use cached content, not disk: {out}"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --all-features -E 'test(test_format_l3_uses_cached_content)' 2>&1`
Expected: FAIL — current `format_l3` reads from disk, gets empty string for non-existent path.

- [ ] **Step 3: Update `format_l3` to use cached content**

Replace the file-reading block in `format_l3` (around line 161-167):

Old:
```rust
        let content = match std::fs::read_to_string(&e.entry.path) {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!("L3: could not read {}: {err}", e.entry.path.display());
                String::new()
            }
        };
```

New:
```rust
        let content = if let Some(bytes) = &e.entry.content {
            String::from_utf8_lossy(bytes).into_owned()
        } else {
            match std::fs::read_to_string(&e.entry.path) {
                Ok(c) => c,
                Err(err) => {
                    tracing::warn!("L3: could not read {}: {err}", e.entry.path.display());
                    String::new()
                }
            }
        };
```

- [ ] **Step 4: Run all format tests**

Run: `cargo nextest run --all-features -E 'test(format_l3)' 2>&1`
Expected: All pass — cached content test uses cache, existing tests still work (they create real files, and their `make_scored` has `content: None` so they fall back to disk read).

- [ ] **Step 5: Write test — format_l3 falls back to disk when content is None**

Add to tests module:

```rust
    #[test]
    fn test_format_l3_falls_back_to_disk_when_no_cached_content() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("disk_read.rs");
        std::fs::write(&path, "fn from_disk() {}").unwrap();

        // content is None — should fall back to reading from disk
        let entry = make_scored(&path.to_string_lossy(), 5, 0.9);
        assert!(entry.entry.content.is_none());

        let out = format_l3(&[entry]);
        assert!(
            out.contains("fn from_disk()"),
            "L3 should fall back to disk read: {out}"
        );
    }
```

- [ ] **Step 6: Run all tests**

Run: `cargo nextest run --all-features 2>&1 | tail -20`
Expected: All pass.

- [ ] **Step 7: Commit**

```bash
git add src/output/format.rs
git commit -m "perf: use cached content in format_l3, eliminate re-reads (P3b)"
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
Expected: `pack_pipeline/files/1000` should show improvement. The 1000-file bench is the best proxy for the 5K pipeline improvement (same pattern: discovery → score → format_l3).

- [ ] **Step 4: Run realistic benchmarks (if available)**

Run: `cargo bench --bench realistic 2>&1`
Expected: Should show improvement on full pipeline.

- [ ] **Step 5: Update perf analysis doc**

Update `docs/superpowers/specs/2026-04-04-discovery-perf-analysis.md`:
- Change P3b status from planned to DONE
- Add actual benchmark numbers
- Update the projected performance stack table

- [ ] **Step 6: Commit doc update**

```bash
git add docs/superpowers/specs/2026-04-04-discovery-perf-analysis.md
git commit -m "docs: update perf analysis with P3b results"
```
