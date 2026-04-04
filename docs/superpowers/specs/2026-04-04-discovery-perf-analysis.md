# Discovery Performance Analysis & Improvement Plan (v4)

**Date**: 2026-04-04 (updated with P2b results)
**Status**: P0+P1+P2+P2b implemented. 10K discovery target met. tiktoken-rs eliminated. Full pipeline optimization ongoing.

## Current Architecture (post-P0+P1+P2+P2b)

```
Phase 1a (serial):   Walk + read + filter           ~fast (ignore crate)
Phase 1b (parallel): estimate_tokens + hash + AST    ~dominant cost (rayon)
                     (thread-local parser/query cache, byte-class token estimator)
                     Exact counts: bpe-openai cl100k (zero-alloc, static tables)
Phase 2  (serial):   Batch git metadata              ~fast (single revwalk)
Phase 3  (serial):   Assemble FileEntry vec           ~trivial
```

## Benchmark Results (cumulative)

| Benchmark | Baseline | P0 | P1 | P2 | P2b | Target |
|---|---|---|---|---|---|---|
| discover/10k | 28.9s | 4.85s | 381ms | 316ms | **341ms** | < 500ms |
| full_pipeline/400 | — | 126ms | 54ms | 14ms | **14.5ms** | < 100ms |
| full_pipeline/5k | — | — | 806ms | 812ms | **826ms** | — |
| score_pack/200 | 214µs | 214µs | 219µs | 221µs | **222µs** | < 50ms |

### Why P2 helps 400-file pipeline (74%) but not 5K (0%)

The 400→14ms improvement comes from **two** sources:
1. **Eliminating `Tokenizer::new()`** — `tiktoken_rs::cl100k_base()` decodes BPE vocabulary from base64 and builds hash tables on every `discover_files()` call (~10-40ms one-time cost). P2 removes this entirely.
2. **Zero-allocation token estimation** — `estimate_tokens_bytes()` does a single byte scan (~0.1µs/file) vs tiktoken's `encode_with_special_tokens` (~300µs/file with Vec allocation).

At 400 files, these savings are a large fraction of total time. At 5K files, post-discovery costs dominate:

#### 5K Full Pipeline Cost Breakdown (estimated)

| Phase | Cost | % of 812ms |
|---|---|---|
| Discovery (walk+read+hash+AST+git) | ~160ms | 20% |
| Scoring (par_iter + FileEntry clone) | ~50ms | 6% |
| Scored clone + sort | ~15ms | 2% |
| Knapsack selection | ~1ms | <1% |
| format_l1 (all 5K entries) | ~5ms | <1% |
| **format_l3 (re-read ~5K files from disk)** | ~150ms | 18% |
| **String assembly (L1+L2+L3 output)** | ~50ms | 6% |
| **Other overhead (alloc, git cache, etc.)** | ~381ms | 47% |

Key insight: **tokenization was <2% of the 5K pipeline cost** even before P2. The fast estimator saves ~15ms out of 812ms — invisible against benchmark noise.

## P2 Implementation Details

### Approach: Byte-class word-boundary estimator

```rust
pub fn estimate_tokens_bytes(bytes: &[u8]) -> usize {
    // Single pass: count word boundaries + punctuation + newlines
    // Per-word subword factor: 1.0 + max(0, word_len - 4) * 0.04
    // Punctuation: 0.65 tokens/char (operator pairs merge in BPE)
    // Newlines: 0.5 tokens/newline (merge with indentation)
}
```

**Accuracy** (calibrated against cl100k_base):

| Sample type | Error range | Typical |
|---|---|---|
| Rust functions | +3% to +11% | +7% |
| Import blocks | +6% | +6% |
| JSON | -12% | -12% |
| Comments/prose | +9% | +9% |
| Struct definitions | 0% to +4% | +2% |

Max observed error: 14.6% across all test samples. Acceptable for scoring/ranking where ±15% is absorbed by scoring weights.

### Alternatives Evaluated

#### `bpe-openai` (GitHub's rust-gems) — Exact count, no allocation

The [`bpe-openai`](https://github.com/github/rust-gems/tree/main/crates/bpe-openai) crate provides a **`count()` method that returns exact BPE token counts without allocating a Vec<u32>**:

```rust
use bpe_openai::cl100k;
let bpe = cl100k();  // Pre-serialized, loaded as static — no init cost
let count = bpe.count(text.as_bytes());  // Exact count, zero allocation
```

Performance (vs tiktoken-rs, MacBook Air M4):

| Text size | tiktoken-rs | bpe count() | Speedup |
|---|---|---|---|
| Small (~100 bytes) | 605µs | 40µs | **15x** |
| Medium (~1KB) | 287µs | 77µs | **3.7x** |
| Large (~10KB) | 3.6ms | 1.65ms | **2.2x** |
| Batch (1000 texts) | 1.97M tok/s | 6.0M tok/s | **3x** |

Key features:
- Novel dynamic programming algorithm: tracks all prefix encodings via last-token backtracking
- Aho-Corasick for efficient overlapping token matching (~1.3 candidates/position)
- **Interval encoding**: after O(n) preprocessing, count any substring in O(1)
- Linear worst-case (vs tiktoken's quadratic on adversarial inputs)
- Pre-serialized BPE tables → zero initialization cost

**Assessment**: `bpe-openai` count() is the optimal drop-in replacement for tiktoken when exact counts are needed (output formatting, budget enforcement). At ~40µs/file for small code files, it's 7.5x faster than tiktoken but ~40x slower than our heuristic estimator (~1µs). **Recommended for L3 output token counting where exactness matters, while keeping the heuristic for discovery/scoring.**

#### `bpe-openai` count_till_limit() — Early-exit budget check

```rust
pub fn count_till_limit(&self, text: &[u8], token_limit: usize) -> Option<usize>
```

Returns `None` as soon as the limit is exceeded — ideal for the `max_file_tokens` check in discovery. Avoids counting the entire file when it's obviously over budget.

#### `bpe` IntervalEncoding — O(1) substring token counts

After O(n) preprocessing of a file, count tokens for any byte range in **typically O(1)**:

```rust
use bpe::interval_encoding::IntervalEncoding;
let ie = IntervalEncoding::new(bpe, file_bytes);  // O(n) preprocessing
let count = ie.count(start..end);                  // O(1) per query
```

This enables a powerful AST + tokenization co-optimization: parse with tree-sitter, then get token counts for each function/struct/import node's byte range without re-tokenizing. BPE counting is **non-monotonic** (appending text can decrease count due to new merges), so naive substring counting is incorrect — IntervalEncoding solves this correctly via a token tree with topological ordering.

#### `tiktoken` crate (distinct from tiktoken-rs) — Newer alternative

The [`tiktoken`](https://docs.rs/tiktoken/latest/tiktoken/) crate (not tiktoken-rs) is a separate, newer implementation with:
- **Zero-alloc `count()` method**: `enc.count("hello world") -> usize`
- Arena-based vocabulary with better cache locality
- Heap-accelerated BPE merge (O(n log n) vs linear scan)
- DFA regex instead of `fancy_regex` backtracking
- Compile-time embedded vocabulary via ruzstd compression
- Supports cl100k, o200k, Llama 3, DeepSeek, Qwen, Mistral

A possible alternative to `bpe-openai`, though `bpe-openai` has the unique interval encoding advantage.

#### tokenx-rs — Statistical estimator

[tokenx-rs](https://github.com/qbit-ai/tokenx-rs) uses segment-based scoring: whitespace→0, CJK→1/char, digits→1/sequence, short words→1, punctuation→ceil(len/2), alphanum→ceil(len/chars_per_token). Accuracy 95-98% for Latin/code, degrades to ~89% for Japanese. Performance: ~107ns for short text, ~11µs for medium, ~333µs for long text (Rust).

Our word-boundary estimator achieves comparable accuracy with simpler logic. The main gap: **CJK handling** — tokenx-rs correctly counts CJK characters as ~1 token each, while our estimator lumps `0x80..=0xFF` bytes into the word category. Worth adding if CJK repos are a target.

#### tiktoken-rs — Why it's slow (root cause analysis)

tiktoken-rs has **three** compounding bottlenecks ([issue #33](https://github.com/openai/tiktoken/issues/33), [issue #391](https://github.com/openai/tiktoken/issues/391)):

1. **`fancy_regex` pre-tokenization (~75% of runtime)** — the pattern includes `\s+(?!\S)` negative lookahead causing backtracking. Hand-coded Rust splitting achieves 104 MB/s vs 6 MB/s for full encoding.
2. **`Vec<usize>` allocation** — `encode_with_special_tokens()` always builds the token ID vector. No count-only API exists. A 10K-token file allocates ~80KB of heap thrown away immediately.
3. **O(n²) BPE merge** on adversarial inputs — linear scan for lowest-rank pair, repeat. GitHub's `bpe` crate uses backtracking + bitfield for guaranteed O(n).

#### How other AI coding tools estimate tokens

| Tool | Method | Accuracy |
|---|---|---|
| Gemini CLI | ASCII: `chars/4`, non-ASCII: `chars*1.3` | ~85-90% |
| Cursor | Custom pure-Rust tiktoken port (experimental) | 100% exact |
| Qwen Code | Removed tiktoken; uses API-reported counts + char heuristic | ~85-90% |
| Claude Code | Anthropic API token counting endpoint | 100% exact |
| Our estimator | Per-word subword + punct + newline heuristic | ~90-95% |

## Research Findings: Industry Approaches

### The consensus pattern across AI tools

All production tools use a **tiered approach**:
1. **Hot path (gate checks, scoring)**: fast heuristic or cached estimate
2. **Cold path (output, billing, budget enforcement)**: exact BPE counting
3. **Session management**: API-reported counts for precise tracking

Our architecture already matches this: P2 heuristic for discovery/scoring, exact counts for output. The gap is using tiktoken-rs (slow, allocating) for the cold path instead of `bpe-openai` (fast, zero-alloc).

### Rayon overhead for small files

From [optimization research](https://gendignoux.com/blog/2024/11/18/rust-rayon-optimized.html): rayon worker threads poll continuously via futex/sched_yield syscalls even when idle. For thousands of small work items (our tiny files), scheduling overhead can be significant. **Mitigation**: batch multiple files per rayon work item, or consider [`paralight`](https://github.com/gendx/paralight) for lighter-weight parallelism.

## Updated Optimization Roadmap

### P0 (DONE): Batch git + parallel rayon
- Result: 28.9s → 4.85s (5.9x)

### P1 (DONE): Thread-local parser reuse + query caching
- Result: 4.85s → 381ms (12.7x cumulative)

### P2 (DONE): Fast token estimation in discovery
- Result: 381ms → 316ms discovery, 54ms → 14ms full_pipeline/400
- Note: No impact on 5K pipeline (tokenization is <2% of cost at that scale)

### P2b (DONE): Replace tiktoken-rs with bpe-openai for exact counts

**Result: tiktoken-rs eliminated. Zero init cost. Benchmarks stable (performance-neutral — expected since discovery uses heuristic estimator).**

What changed:
- `tiktoken-rs = "0.9"` → `bpe-openai = "0.3"` in Cargo.toml
- `Tokenizer` struct now holds `&'static bpe_openai::Tokenizer` (zero-cost construction)
- `count_tokens()` / `count_tokens_bytes()` are now infallible (`-> usize`, not `Result`)
- `Tokenizer::new()` is infallible (no `Result` needed — static BPE tables)
- Removed `fancy_regex` transitive dependency (75% of tiktoken-rs runtime)
- Linear worst-case BPE encoding (eliminates quadratic adversarial input risk)
- Unlocks `IntervalEncoding` for future O(1) substring token queries (P3)

### P3: Two-phase scoring with lazy AST (est. 5-8x for tight budgets)
**Expected: 812ms → ~200-400ms for 5K full pipeline**

The highest-impact optimization for the 5K pipeline. Split processing into cheap pre-score and selective enrichment:

```
Phase A: Walk + read + estimate_tokens + hash + git_metadata → cheap_score
          (no AST, no full tokenization)
Phase B: Sort by cheap_score, take top K candidates
Phase C: Full tokenize + AST only for K candidates → final FileEntry
```

Where K = `min(2 * budget_capacity, total_files)`. For 128K budget at ~20 tok/file avg (Tiny files), K could still be large. Effectiveness depends on budget tightness:

| Budget | Avg tokens/file | Files needed | K (2x) | Total files | Savings |
|---|---|---|---|---|---|
| 128K | 20 (tiny) | ~6,400 | 12,800 | 5,000 | 0% (all needed) |
| 128K | 200 (mixed) | ~640 | 1,280 | 5,000 | 74% |
| 128K | 500 (medium) | ~256 | 512 | 5,000 | 90% |

**Key insight**: P3 helps most when budget is tight relative to repo size. For the 5K scale_test with tiny files, most files fit in the budget anyway, limiting P3's impact. Real-world repos with medium-sized files benefit enormously.

### P3b (NEW): Content caching — eliminate format_l3 re-reads

**Expected: ~150ms savings on 5K pipeline**

Currently, `format_l3()` re-reads every selected file from disk. At 5K files all selected, that's ~5K `read_to_string()` calls.

Fix: retain file content from Phase 1a in a `HashMap<PathBuf, Vec<u8>>` or store it in `FileEntry` directly. Trade memory for I/O:

```rust
struct FileEntry {
    // ... existing fields ...
    /// Raw file content, retained from discovery for output formatting.
    /// Only populated for files that pass initial filters.
    content: Option<Vec<u8>>,
}
```

Memory impact: 5K × 200 bytes avg = 1MB — negligible vs the 100MB target.

### P4: Parallel directory walking (est. 30% on Phase 1a)
**Expected: 500ms → ~300ms for Phase 1a**

Switch from `WalkBuilder::build()` to `WalkBuilder::build_parallel()`. Most impactful when combined with P3 (filter during walk).

### P5: Persistent on-disk cache (instant repeat calls)
**Expected: ~50ms for unchanged repos**

## Revised Projected Performance Stack

| Optimization | 10K discover | 5K full pipeline | Notes |
|---|---|---|---|
| Baseline | 28.9s | — | |
| P0 (batch git + rayon) | 4.85s | — | |
| P1 (parser/query reuse) | 381ms | 806ms | |
| P2 (fast token estimation) | 316ms | 812ms | 5K bottleneck is post-discovery |
| **P2b (bpe-openai exact)** | **341ms** | **826ms** | Perf-neutral; eliminates tiktoken-rs |
| + P3b (content caching) | ~310ms | ~620ms | Eliminates L3 re-reads |
| + P3 (two-phase scoring) | ~200ms | ~300-500ms | Impact depends on budget tightness |
| + P4 (parallel walk) | ~150ms | ~250-400ms | |
| + P5 (disk cache, repeat) | ~50ms | ~50ms | |

## Implementation Priority (revised)

1. ~~**P2b** (bpe-openai)~~ — DONE
2. **P3b** (content caching) — simple refactor, significant 5K pipeline improvement
3. **P3** (two-phase scoring) — highest impact for real-world repos, moderate complexity
4. **P4** (parallel walk) — moderate impact, API change
5. **P5** (disk cache) — transformative for repeat calls, new subsystem

## Sources

### Tokenizer Libraries
- [bpe crate (GitHub rust-gems)](https://github.com/github/rust-gems/tree/main/crates/bpe) — Novel DP algorithm, 3.5x faster than tiktoken, zero-alloc count(), interval encoding
- [bpe-openai crate](https://github.com/github/rust-gems/tree/main/crates/bpe-openai) — Pre-built cl100k/o200k tokenizers, `count()` + `count_till_limit()` API
- [GitHub Blog: "So many tokens, so little time"](https://github.blog/ai-and-ml/llms/so-many-tokens-so-little-time-introducing-a-faster-more-flexible-byte-pair-tokenizer/) — bpe crate design rationale and benchmarks
- [rs-bpe benchmarks (dev.to)](https://dev.to/gweidart/rs-bpe-outperforms-tiktoken-tokenizers-2h3j) — 3-15x faster, 6M tokens/sec batch throughput
- [tiktoken crate (distinct from tiktoken-rs)](https://docs.rs/tiktoken/latest/tiktoken/) — Zero-alloc count(), DFA regex, arena vocab
- [tokenx-rs](https://github.com/qbit-ai/tokenx-rs) — Heuristic estimator, 95-98% accuracy, CJK support
- [tryAGI/Tiktoken (.NET)](https://github.com/tryAGI/Tiktoken) — Zero-alloc counting architecture, 141-618 MiB/s

### tiktoken-rs Performance Issues
- [tiktoken issue #33](https://github.com/openai/tiktoken/issues/33) — Performance optimization discussion, regex dominates runtime
- [tiktoken issue #195](https://github.com/openai/tiktoken/issues/195) — Superlinear complexity on repeated characters
- [tiktoken issue #391](https://github.com/openai/tiktoken/issues/391) — Quadratic adversarial input risk

### AI Tool Token Estimation
- [Gemini CLI token estimation PR #13824](https://github.com/google-gemini/gemini-cli/pull/13824) — ASCII `chars/4`, non-ASCII `chars*1.3` heuristic
- [Cursor (anysphere/tiktoken-rs)](https://github.com/anysphere/tiktoken-rs) — Pure-Rust tiktoken port (experimental)
- [Qwen Code tiktoken removal](https://github.com/QwenLM/qwen-code/issues/1289) — Switched to API-reported counts

### Parser and Pipeline Optimization
- [Tree-sitter Parser docs](https://docs.rs/tree-sitter/latest/tree_sitter/struct.Parser.html) — Parser reuse pattern (P1)
- [Zed syntax-aware editing](https://zed.dev/blog/syntax-aware-editing) — Thread-local parser pool pattern (P1)
- [50x context reduction with tree-sitter code index](https://dev.to/uwe_c_39d9ab7d16ff8dfe67e/how-i-cut-ai-context-usage-by-50x-with-a-tree-sitter-code-index-plm) — AST-guided selective processing
- [V8 lazy parsing](https://v8.dev/blog/preparser) — Two-phase parse inspiration (P3)
- [Rayon optimization (Endignoux)](https://gendignoux.com/blog/2024/11/18/rust-rayon-optimized.html) — Polling overhead for small work items
- [paralight crate](https://github.com/gendx/paralight) — Lighter-weight parallelism alternative
- [ripgrep parallel walk](https://github.com/BurntSushi/ripgrep/discussions/2472) — Diminishing returns beyond 4-8 threads (P4)
- [ignore crate WalkParallel](https://docs.rs/ignore/latest/ignore/struct.WalkParallel.html) — Parallel traversal API (P4)
