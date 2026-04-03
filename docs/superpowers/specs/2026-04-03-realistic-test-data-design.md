# Realistic Test Data for MCP Efficiency Verification

**Date**: 2026-04-03
**Goal**: Generate real-world test repositories with git history, edge cases, and realistic code to verify the context-window-optimizer MCP pipeline's correctness and performance.

## Deliverables

| File | Purpose |
|---|---|
| `tests/fixtures/builder.rs` | `RealisticRepoBuilder` — git-backed temp repo construction |
| `tests/fixtures/content.rs` | Syntactically valid code generators per language |
| `tests/fixtures/scenarios.rs` | Named scenario constructors composing the builder |
| `tests/fixtures/mod.rs` | Updated to re-export new modules alongside existing `TempRepo` |
| `tests/integration/realistic.rs` | Correctness integration tests across all scenarios |
| `benches/realistic.rs` | Criterion benchmarks validating performance targets |

## 1. RealisticRepoBuilder

### Location
`tests/fixtures/builder.rs`

### Dependencies
- `git2` (already in `Cargo.toml` as a dependency)
- `tempfile` (already used by existing fixtures)
- `rand` (for content uniqueness — add as dev-dependency if not present)

### API

```rust
pub struct RealisticRepoBuilder { ... }

pub enum FileSize { Tiny, Small, Medium, Large, Huge }
// Approximate token counts: 20, 100, 500, 2000, 6000

pub enum CommitPattern {
    /// 80% of commits touch `hot_paths`, 20% touch the rest
    Hotspot { hot_paths: Vec<&'static str>, total_commits: usize, span_days: u64 },
    /// Commits spread evenly across all files over `span_days`
    Uniform { total_commits: usize, span_days: u64 },
    /// Most files committed once long ago, a few burst recently
    Stale { initial_age_days: u64, burst_paths: Vec<&'static str>, burst_age_days: u64 },
}

impl RealisticRepoBuilder {
    pub fn new() -> Self;

    // ---- File creation ----
    pub fn add_file(self, path: &str, lang: Language, size: FileSize) -> Self;
    pub fn add_file_with_content(self, path: &str, content: &str) -> Self;
    pub fn add_module(self, dir: &str, lang: Language, count: usize) -> Self;
    pub fn add_exact_duplicates(self, dir: &str, count: usize) -> Self;
    pub fn add_near_duplicates(self, dir: &str, count: usize, similarity: f64) -> Self;
    pub fn add_empty_files(self, count: usize) -> Self;
    pub fn add_large_file(self, path: &str, approx_tokens: usize) -> Self;
    pub fn add_deep_nesting(self, base: &str, files_per_level: usize, depth: usize) -> Self;
    pub fn add_non_code_files(self) -> Self;  // README.md, config.toml, etc.

    // ---- Git history ----
    pub fn initial_commit(self, days_ago: u64) -> Self;
    pub fn commit_pattern(self, pattern: CommitPattern) -> Self;
    pub fn recent_edits(self, paths: &[&str], days_ago: u64) -> Self;

    // ---- Build ----
    pub fn build(self) -> TempRepo;
}
```

### Git implementation details

- `git2::Repository::init()` to create the repo
- `git2::Signature::new("Test", "test@test.com", &git2::Time::new(epoch_secs, 0))` for backdated commits
- Each `commit_pattern` call iterates over files, makes trivial modifications (append a comment line with a unique counter), stages, and commits with the appropriate timestamp
- `recent_edits` appends a comment and commits at the specified date
- All git operations use `git2` directly — no shell `git` commands

### TempRepo extension

The existing `TempRepo` struct gains a `repo()` method returning `Option<&git2::Repository>` for scenarios that init git. Existing `TempRepo::minimal()` etc. remain unchanged.

## 2. Content Generators

### Location
`tests/fixtures/content.rs`

### Design

Each language has a `generate(size: FileSize, index: usize) -> String` function. The `index` parameter ensures uniqueness (embedded in identifiers) to prevent accidental dedup.

**Rust** (`generate_rust`):
- Tiny: single function `pub fn func_{index}() -> usize { {index} }`
- Small: function + struct + impl block (~100 tokens)
- Medium: module with imports, 3 functions, error enum, tests block (~500 tokens)
- Large: full module with traits, generics, multiple impls (~2000 tokens)
- Huge: large module simulating a real-world file with extensive logic (~6000 tokens)

**TypeScript** (`generate_typescript`):
- Tiny: single export `export const value_{index} = {index};`
- Small: interface + function (~100 tokens)
- Medium: React-style component with props, hooks, JSX (~500 tokens)
- Large: full API route handler with validation, error handling (~2000 tokens)
- Huge: complex component with state management, multiple handlers (~6000 tokens)

**Python** (`generate_python`):
- Tiny: single function
- Small: class with `__init__` and one method
- Medium: class with decorators, type hints, dataclass (~500 tokens)
- Large: full module with multiple classes, error handling (~2000 tokens)
- Huge: complex module with async functions, context managers (~6000 tokens)

**Go** (`generate_go`):
- Tiny: single function with package declaration
- Small: struct + method + constructor
- Medium: interface + implementation + error types (~500 tokens)
- Large: full package with HTTP handler, middleware (~2000 tokens)
- Huge: complex service implementation (~6000 tokens)

**Non-code** (markdown, TOML, JSON, YAML, CSS):
- Fixed templates with parameterized names/values

## 3. Scenarios

### Location
`tests/fixtures/scenarios.rs`

### Scenario definitions

#### `web_fullstack()` (~200 files)
```
src/components/     — 40 TSX files (Small-Medium), high churn
src/api/routes/     — 25 TS files (Medium), high churn
src/api/middleware/  — 8 TS files (Small), moderate churn
src/models/         — 15 TS files (Small-Medium), low churn
src/utils/          — 10 TS files (Tiny-Small), low churn
src/hooks/          — 10 TSX files (Small), moderate churn
src/styles/         — 15 CSS files (Small)
tests/              — 30 TS test files (Medium)
config/             — 5 JSON/YAML files
public/             — 5 HTML files
docs/               — 10 MD files
vendor/legacy/      — 15 JS files (exact duplicates of each other, 5 unique)
```
Git: `Hotspot { hot_paths: ["src/components/", "src/api/"], total_commits: 150, span_days: 180 }`

#### `rust_workspace()` (~150 files)
```
core/src/           — 25 Rust files (Medium-Large), moderate churn
core/tests/         — 10 Rust files (Medium)
server/src/         — 20 Rust files (Medium-Large), high churn
server/tests/       — 8 Rust files (Medium)
cli/src/            — 10 Rust files (Small-Medium), low churn
macros/src/         — 5 Rust files (Medium), low churn
*/Cargo.toml        — 4 TOML config files
benches/            — 5 Rust files (Small)
docs/               — 10 MD files
```
Git: `Uniform { total_commits: 100, span_days: 90 }`
Plus `recent_edits` on `server/src/` files.

#### `polyglot_monorepo()` (~400 files)
```
services/api/       — 50 Go files (Medium-Large), 6 months old
services/ml/        — 60 Python files (Medium-Large), 3 months old
services/web/       — 80 TS files (Small-Medium), actively developed
libs/shared/        — 30 Go files (Medium), old
infra/              — 20 YAML/TOML files (Small)
docs/               — 30 MD files
scripts/            — 15 Python files (Small)
```
Near-duplicates: each service has similar Dockerfile/Makefile/CI config (15 near-dupes total).
Git: `Stale { initial_age_days: 180, burst_paths: ["services/web/"], burst_age_days: 7 }`

#### `legacy_with_duplication()` (~200 files)
```
src/handlers/       — 30 files: 25 are near-duplicates (95% similar), 5 unique
src/models/         — 20 files (Medium-Large), all old
src/utils/          — 15 files (Small), all old
vendor/             — 20 files: 4 groups of 5 exact duplicates
src/core/           — 10 files (Large-Huge), recently touched
tests/              — 20 files (Medium), old
config/             — 5 files (Tiny)
```
Git: `Stale { initial_age_days: 365, burst_paths: ["src/core/"], burst_age_days: 3 }`

#### `scale_test(n: usize)` (n files, default 10_000)
```
src/mod_{0..n/10}/file_{0..10}.rs  — uniformly distributed Rust files (Tiny-Small)
```
Git: single initial commit (minimize setup time for perf benchmarks).
Content: minimal unique functions to avoid dedup.

## 4. Integration Tests

### Location
`tests/integration/realistic.rs`

### Test cases

**Selection quality:**
- `test_focus_boosts_adjacent_files_web_fullstack` — focus on `src/api/routes/users.ts`, assert top-5 selected files are from `src/api/` or `src/models/`
- `test_recent_beats_old_rust_workspace` — recently-edited `server/src/` files rank above old `cli/src/` files
- `test_hotspot_files_rank_higher` — files with many commits score higher on recency

**Dedup effectiveness:**
- `test_exact_dedup_web_fullstack` — `vendor/legacy/` exact dupes removed, count matches expected
- `test_near_dedup_legacy` — with `dedup.near=true`, near-duplicate handlers reduced
- `test_dedup_preserves_all_unique` — unique files never removed by dedup

**Budget compliance:**
- `test_budget_respected_all_scenarios` — parametric test: for each scenario x budget in `[500, 5_000, 50_000, 128_000]`, assert `tokens_used <= budget.l3_tokens()`
- `test_zero_budget_selects_nothing` — budget=0 returns empty selection
- `test_huge_budget_selects_everything` — budget=1_000_000 on small repo selects all files

**Diversity:**
- `test_diversity_spans_services_polyglot` — with diversity enabled on polyglot monorepo, selected files come from >=3 service directories

**Compression:**
- `test_compression_ratio_meaningful_at_scale` — on `scale_test(1000)` with budget=50K, compression ratio >= 2.0

**Output consistency:**
- `test_stats_match_selected_all_scenarios` — `stats.files_selected == selected.len()` and `stats.tokens_used == sum(selected.token_count)`
- `test_scores_bounded_all_scenarios` — all composite scores in [0.0, 1.0], all signals in [0.0, 1.0]

**Git signal ordering:**
- `test_recency_signal_reflects_git_history` — file committed 1 day ago has higher recency signal than file committed 180 days ago

**Edge cases:**
- `test_all_identical_files` — repo with 50 identical files, dedup removes 49
- `test_file_at_token_limit` — file with exactly `max_file_tokens` tokens is included
- `test_deeply_nested_focus` — focus on `src/a/b/c/d/e/f/leaf.rs` boosts proximity for neighboring files
- `test_no_code_files_only` — repo with only .md/.toml/.json files still packs successfully
- `test_empty_files_handled` — empty files get 0 tokens, 0 score, included as free items

## 5. Criterion Benchmarks

### Location
`benches/realistic.rs`

### Benchmark groups

```rust
// Each benchmark uses criterion's iter_batched to exclude repo setup cost

fn bench_discover_10k(c: &mut Criterion) {
    // Target: < 500ms
    // Uses scale_test(10_000) built once in setup
}

fn bench_score_pack_500(c: &mut Criterion) {
    // Target: < 50ms  
    // Pre-discovers files, benchmarks only score_entries + select_items
}

fn bench_full_pipeline_medium(c: &mut Criterion) {
    // Target: < 100ms
    // Full pack_files on polyglot_monorepo (~400 files)
}

fn bench_full_pipeline_large(c: &mut Criterion) {
    // Full pack_files on scale_test(5_000)
}

fn bench_dedup_heavy(c: &mut Criterion) {
    // Measures dedup phase on legacy_with_duplication
    // Separate groups for exact-only vs exact+near
}

fn bench_focus_vs_no_focus(c: &mut Criterion) {
    // Compare scoring speed with/without focus paths
    // and with/without dependency graph construction
}
```

### Benchmark setup

The `scale_test` repo is expensive to build (10K files + git init). Build it once per benchmark group using `lazy_static` or criterion's setup closure. Smaller scenarios are cheap enough to rebuild per iteration if needed, but should also be cached.

## 6. Dev-dependencies to add

```toml
[dev-dependencies]
# Already present: tempfile, proptest, criterion
rand = "0.8"  # For content uniqueness in generators (if not already present)
```

## Non-goals

- No MCP protocol-level testing (JSON-RPC transport) — that's a separate concern
- No actual LLM evaluation (comparing with/without optimizer on real tasks) — out of scope for automated tests
- No benchmark CI integration — local-only for now
