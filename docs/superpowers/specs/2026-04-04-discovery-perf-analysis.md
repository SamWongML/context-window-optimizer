# Discovery Performance Analysis & Improvement Plan (v2)

**Date**: 2026-04-04 (updated with research-backed optimizations)
**Problem**: `discover_files` takes ~4.85s for 10K files after P0 (batch git + parallel rayon). Target is < 500ms.
**Gap**: Still ~10x over target. Root cause has shifted from git to per-file processing.

## Current Architecture (post-P0)

```
Phase 1a (serial):   Walk + read + filter           ~fast (ignore crate)
Phase 1b (parallel): Tokenize + hash + AST + simhash ~dominant cost
Phase 2  (serial):   Batch git metadata              ~fast (single revwalk)
Phase 3  (serial):   Assemble FileEntry vec           ~trivial
```

## Benchmark Results (post-P0)

| Benchmark | Before P0 | After P0 | Target | Gap |
|---|---|---|---|---|
| discover/10k_files | 28.9s | 4.85s | < 500ms | ~10x |
| full_pipeline/400_files | 520ms | 126ms | < 100ms | ~1.3x |
| score_pack/200_files | 214us | 214us | < 50ms | Under |

## Remaining Cost Breakdown (10K files, ~8 cores)

Phase 1b dominates. Profiling of the serial version showed:

| Component | Per-file cost | 10K serial | 10K parallel (8c) | % of total |
|-----------|--------------|------------|-------------------|-----------|
| **tree-sitter AST** | ~2.8ms | ~28s | ~3.5s | **~72%** |
| **tiktoken BPE** | ~0.3ms | ~3s | ~375ms | **~8%** |
| MD5 hash | ~0.02ms | ~200ms | ~25ms | ~0.5% |
| SimHash | ~0.05ms | ~500ms | ~62ms | ~1.3% |
| File walk + read | — | ~500ms | ~500ms (serial) | ~10% |
| Git batch | — | ~5ms | ~5ms | ~0.1% |
| Assembly | — | ~50ms | ~50ms | ~1% |
| **TOTAL** | | | **~4.5-5s** | |

**Primary bottleneck: tree-sitter Parser::new() + set_language() + parse() called 10K times across rayon threads, creating a fresh parser per file.**

Secondary bottleneck: tiktoken BPE encoding does full tokenization when we only need the count.

## Research Findings

### 1. Tree-sitter Parser Reuse (from Helix/Zed editor patterns)

The tree-sitter `Parser` struct is `Send + Sync` and designed for reuse across multiple `parse()` calls ([docs.rs/tree-sitter](https://docs.rs/tree-sitter/latest/tree_sitter/struct.Parser.html)). Creating a Parser + calling `set_language()` involves allocating internal parsing tables. Editors like Helix and Zed reuse parser instances via thread-local storage rather than creating per-file ([Zed blog](https://zed.dev/blog/syntax-aware-editing)).

**Key insight**: `Parser::new()` + `set_language()` is the expensive part, not `parse()` itself. Tree-sitter can parse a 10K-line C file in <100ms — the per-file cost we see (~2.8ms) is dominated by parser creation, not parsing.

**Technique**: Use `thread_local!` with a `RefCell<HashMap<Language, Parser>>` so each rayon thread maintains one Parser per language, reusing it across all files of that language. Expected: **5-10x reduction in AST cost**.

### 2. Tree-sitter Query Compilation Caching

In `queries.rs`, `Query::new(language, query_source)` is called per-file for both signature and import queries. Query compilation involves parsing S-expressions and building automata. Since the queries are static strings, they should be compiled once per language and cached.

**Technique**: `OnceLock<HashMap<Language, (Query, Query)>>` or lazy_static for compiled queries. Expected: **additional 2-3x improvement** on top of parser reuse.

### 3. Fast Token Estimation vs Full BPE (from tokenx-rs research)

The `tiktoken-rs` library does full BPE encoding (`encode_with_special_tokens`) to count tokens — allocating a `Vec<usize>` of token IDs that we immediately discard. The [tokenx-rs](https://github.com/qbit-ai/tokenx-rs) library achieves 96% accuracy token estimation via simple character classification at **~10µs per file** (vs ~300µs for full BPE) — a **30x speedup** with zero allocations.

For our use case (scoring/ranking files, not exact billing), 96% accuracy is more than sufficient. The scoring weights will absorb the ~4% estimation error.

Alternative: [rs-bpe](https://dev.to/gweidart/rs-bpe-outperforms-tiktoken-tokenizers-2h3j) offers exact BPE at 3-15x faster than tiktoken with O(n) complexity and zero-alloc counting mode.

**Technique**: Replace `tiktoken-rs` with either:
- (a) `tokenx-rs` for ~30x speedup with 96% accuracy (recommended for scoring phase)
- (b) Keep `tiktoken-rs` only for final output formatting where exact counts matter

### 4. Two-Phase Scoring with Early Termination (V8 preparser inspiration)

V8's [lazy parsing](https://v8.dev/blog/preparser) skips full AST parsing for functions until they're actually called. Same principle applies here: most files in a 10K-file repo won't make it into the final context window.

**Technique**: 
- **Phase A (cheap score)**: Score using only size, path proximity, extension, git metadata (~0.05ms/file). No AST, no tokenization.
- **Phase B (full score)**: Only for top-K candidates (K = 2-3x budget capacity), do full tokenization + AST analysis.

For a 128K token budget with avg 200 tokens/file, we need ~640 files. Scoring 2x that = ~1,280 files with full AST instead of 10,000. Expected: **~8x reduction in AST + tokenization work**.

### 5. Parallel Directory Walking (from ripgrep/ignore architecture)

Ripgrep's `ignore` crate supports [`WalkParallel`](https://docs.rs/ignore/latest/ignore/struct.WalkParallel.html) which uses a work-stealing queue pattern via `crossbeam-deque` for multi-threaded directory traversal. Our current Phase 1a uses sequential `Walk`.

For 10K files, the serial walk + read is ~500ms. `WalkParallel` with file reading integrated into the walk callbacks could overlap I/O with processing, reducing wall-clock time.

**Research caveat**: [BurntSushi's own analysis](https://github.com/BurntSushi/ripgrep/discussions/2472) shows diminishing returns: 4 threads gives ~36% speedup on cached FS, degrading beyond 8 threads. Most useful when combined with per-file processing in the walk callback.

**Technique**: Use `WalkParallel` with `DirEntry` callback that does file read + cheap scoring inline, feeding results into a crossbeam channel. Expected: **~30-50% reduction in Phase 1a time** (500ms → ~300ms).

### 6. Switch from libgit2 to gitoxide (gix) for git operations

[Gitoxide](https://github.com/GitoxideLabs/gitoxide) is a pure-Rust git implementation achieving [11x faster tree-tree diffs](https://github.com/Byron/gitoxide/discussions/74) on single-core and 1.6-2x faster in real-world scenarios vs libgit2. Its blame cache achieves 75x speedup (~300ms → ~4ms).

**Assessment**: Our batch git is already fast (~5ms for 10K files). Switching to gitoxide would add complexity for minimal gain. **Defer unless git becomes a bottleneck again** (e.g., when walking 500+ commits).

### 7. libgit2 Object Caching Control

The `git2` crate exposes [`git2::opts::enable_caching(false)`](https://docs.rs/git2/latest/git2/opts/fn.enable_caching.html) — disabling libgit2's internal object cache when loading many objects that won't be referenced again. This can reduce memory pressure for large revwalks.

**Assessment**: Marginal impact given our batch approach. Worth testing if memory becomes an issue.

## Recommended Implementation Order

### P0 (DONE): Batch git + parallel rayon
- Implemented: single revwalk, rayon Phase 1b
- Result: 28.9s → 4.85s (5.9x)

### P1: Thread-local parser reuse + query caching (est. 3-5x speedup)
**Expected: 4.85s → ~1.0-1.6s**

In `src/index/ast/mod.rs`, replace per-file `Parser::new()` + `set_language()` with thread-local parser pool:

```rust
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static PARSERS: RefCell<HashMap<Language, Parser>> = RefCell::new(HashMap::new());
}

pub fn analyze_file(source: &[u8], language: Language, max_ast_bytes: usize) -> Option<AstData> {
    // ...size check...
    let (lang_fn, lq) = grammar_for(language)?;
    let ts_lang = tree_sitter::Language::from(lang_fn);

    PARSERS.with(|parsers| {
        let mut map = parsers.borrow_mut();
        let parser = map.entry(language).or_insert_with(|| {
            let mut p = Parser::new();
            p.set_language(&ts_lang).expect("language load");
            p
        });
        let tree = parser.parse(source, None)?;
        // ...query execution with cached queries...
    })
}
```

Similarly, cache compiled `Query` objects in a `OnceLock` or `thread_local!`.

### P2: Fast token estimation (est. 2-3x speedup on tokenization)
**Expected: further ~200ms savings**

Replace `tiktoken-rs` `encode_with_special_tokens` with a fast byte-ratio estimator for the discovery/scoring phase:

```rust
/// Fast token count estimation (~96% accuracy, zero allocation).
/// Uses character-class heuristics: ~10µs per file vs ~300µs for full BPE.
fn estimate_tokens(bytes: &[u8]) -> usize {
    // Average ~3.5 bytes per token for code (empirically calibrated)
    // Adjust for language: CJK ~1 token/char, code ~3.5 bytes/token
    let text_len = bytes.len();
    let whitespace = bytecount::count(bytes, b' ') + bytecount::count(bytes, b'\n');
    let code_bytes = text_len - whitespace;
    // Whitespace is ~0 tokens, code is ~1 token per 3.5 bytes
    (code_bytes as f64 / 3.5).ceil() as usize + (whitespace as f64 / 8.0).ceil() as usize
}
```

Or integrate `tokenx-rs` for more accurate multi-language estimation. Keep `tiktoken-rs` for final output where exact counts are needed (L1/L2/L3 budget allocation).

### P3: Two-phase scoring with lazy AST (est. 5-8x for tight budgets)
**Expected: 4.85s → ~0.5-1.0s (combined with P1+P2)**

Split discovery into cheap pre-scoring and selective enrichment:

```
Phase A: Walk + read + estimate_tokens + hash + git_metadata → cheap_score
          (no AST, no full tokenization)
Phase B: Sort by cheap_score, take top K candidates
Phase C: Full tokenize + AST only for K candidates → final FileEntry
```

Where K = `min(2 * budget_capacity, total_files)`. For 128K budget at ~200 tok/file avg, K ≈ 1,280 instead of 10,000.

This is the **highest-impact single change** for tight budgets but requires restructuring the scoring pipeline to work in two passes.

### P4: Parallel directory walking (est. 30% on Phase 1a)
**Expected: 500ms → ~300ms**

Switch from `WalkBuilder::build()` to `WalkBuilder::build_parallel()` with integrated file reading:

```rust
let (tx, rx) = crossbeam_channel::bounded(1024);
builder.build_parallel().run(|| {
    let tx = tx.clone();
    Box::new(move |entry| {
        if let Ok(entry) = entry {
            // read + filter inline
            tx.send(raw_file).ok();
        }
        ignore::WalkState::Continue
    })
});
```

**Note**: Diminishing returns beyond 4-8 threads per ripgrep benchmarks. Most valuable when combined with P3 (filtering during walk reduces downstream work).

### P5: Persistent on-disk cache (instant repeat calls)

Store discovery results in `.ctx-optim/cache.bin` keyed by `(HEAD_oid, file_path, mtime)`. On cache hit, skip all of Phase 1b + Phase 2. Only new/modified files need processing.

**Expected**: Second call on unchanged repo → ~50ms (walk + cache lookup only).

## Projected Performance Stack

| Optimization | Cumulative 10K time | Speedup from baseline |
|---|---|---|
| Baseline (pre-P0) | 28.9s | 1x |
| P0: Batch git + rayon (done) | 4.85s | 6x |
| + P1: Parser/query reuse | ~1.5s | 19x |
| + P2: Fast token estimation | ~1.2s | 24x |
| + P3: Two-phase scoring | ~0.4-0.6s | **48-72x** |
| + P4: Parallel walk | ~0.3-0.5s | **58-96x** |
| + P5: On-disk cache (repeat) | ~50ms | **578x** |

**P1 + P2 + P3 together should bring 10K discovery under 500ms target.**

## Implementation Priority

1. **P1** (parser reuse) — highest certainty, lowest risk, touches only `ast/mod.rs` + `ast/queries.rs`
2. **P3** (two-phase) — highest impact, moderate complexity, restructures discovery pipeline
3. **P2** (fast tokens) — moderate impact, simple, drop-in replacement in tokenizer.rs
4. **P4** (parallel walk) — moderate impact, API change in discovery.rs Phase 1a
5. **P5** (disk cache) — transformative for repeat calls, new subsystem

## Sources

- [Tree-sitter Parser docs (Rust)](https://docs.rs/tree-sitter/latest/tree_sitter/struct.Parser.html) — Parser is Send+Sync, designed for reuse
- [Zed: Syntax-aware editing with tree-sitter](https://zed.dev/blog/syntax-aware-editing) — Editor parser reuse patterns
- [Incremental parsing with tree-sitter](https://dasroot.net/posts/2026/02/incremental-parsing-tree-sitter-code-analysis/) — 10K-line file in <100ms, 70% reduction with incremental
- [V8 lazy parsing (preparser)](https://v8.dev/blog/preparser) — Two-phase parse: cheap pre-scan, full parse on demand
- [tokenx-rs: 96% accurate token estimation](https://github.com/qbit-ai/tokenx-rs) — ~10µs/file, zero deps, zero allocs
- [rs-bpe: 3-15x faster than tiktoken](https://dev.to/gweidart/rs-bpe-outperforms-tiktoken-tokenizers-2h3j) — O(n) BPE with zero-alloc counting
- [gitoxide vs libgit2 performance](https://github.com/Byron/gitoxide/discussions/74) — 11x faster tree diffs, 75x faster blame with cache
- [ripgrep parallel walk analysis](https://github.com/BurntSushi/ripgrep/discussions/2472) — Diminishing returns beyond 4-8 threads
- [ripgrep architecture (DeepWiki)](https://deepwiki.com/BurntSushi/ripgrep) — Work-stealing queue, multi-level filtering
- [ignore crate WalkParallel](https://docs.rs/ignore/latest/ignore/struct.WalkParallel.html) — Parallel directory traversal API
- [git2 enable_caching](https://docs.rs/git2/latest/git2/opts/fn.enable_caching.html) — Disable object cache for large traversals
- [Helix editor tree-sitter integration](https://helix-editor.com/news/release-25-07-highlights/) — Parser reuse via thread-local RefCell
