# Discovery Performance Analysis & Improvement Plan

**Date**: 2026-04-04
**Problem**: `discover_files` takes ~29s for 10K files. Target is < 500ms. Score+pack is only 214us, so discovery is 99.99% of time.

## Root Cause: Per-File Git Revwalk

The bottleneck is `read_git_metadata()` at `src/index/discovery.rs:88-165`. For each file it:

1. Creates a new `revwalk` from HEAD
2. Iterates up to 500 commits (MAX_COMMITS)
3. Per commit: `find_commit()` -> `commit.tree()` -> `tree.get_path(relative)` — each a libgit2 FFI call
4. Checks parent tree to determine if file actually changed

Even for `scale_test(10_000)` with only 1 commit, the revwalk does ~4 libgit2 object lookups x 10K files = 40K FFI calls minimum. No git-level batching means the same commits and trees are deserialized thousands of times.

## Benchmark Results (2026-04-04)

| Benchmark | Result | Target | Status |
|---|---|---|---|
| discover/10k_files | 28.9s | < 500ms | Way over |
| score_pack/200_files | 214us | < 50ms | Well under |
| full_pipeline/400_files | 520ms | < 100ms | Over |
| full_pipeline/5k_files | 15.0s | — | Baseline |
| dedup/exact_only | 384ms | — | Baseline |
| dedup/exact+near | 389ms | — | ~1.3% overhead |
| focus/no_focus | 307ms | — | Baseline |
| focus/with_focus | 305ms | — | Negligible overhead |

## Estimated Time Breakdown (10K files)

| Phase | Component | Time | % |
|-------|-----------|------|---|
| 1 | File walk + read + tokenize + hash | ~960ms | 3.3% |
| 2 | **git2 revwalk + metadata per file** | **~28,000ms** | **96.7%** |
| | TOTAL | ~29,000ms | 100% |

## Improvement Approaches (ordered by impact)

### P0: Batch git metadata via single revwalk (50-100x speedup)

One revwalk processing all files simultaneously instead of N separate revwalks. For C commits and N files: O(C) tree loads instead of O(N x C).

### P1: Two-phase scoring with early termination (10-100x for tight budgets)

Quick pass using size+proximity only (no git), then selective git enrichment for top candidates only.

### P2: Persistent on-disk cache (instant on repeat calls)

Store git metadata in `.ctx-optim/` keyed by HEAD commit + file mtime. Invalidate when HEAD or mtime changes.

### P3: Lazy git metadata via OnceCell

Don't compute during discovery; resolve only when scoring needs it.

### P4: diff-tree instead of get_path (2-5x within batch)

Use libgit2 diff machinery which compares trees at directory level, avoiding per-file `tree.get_path()`.

## Recommended Order

P0 + P1 together should bring 10K discovery from ~29s to well under 500ms.
