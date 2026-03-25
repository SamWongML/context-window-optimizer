# CLAUDE.md — Context Window Optimizer

## Project Overview

A Rust-based MCP server + CLI tool that intelligently selects and packs code context
for LLM agents. Solves the context window optimization problem: given a token budget,
select the most valuable code fragments while respecting diversity and deduplication
constraints. Delivered as an MCP server (stdio transport) for Claude Code/Codex, and
as a standalone CLI.

## Tech Stack

- **Language:** Rust 2024 edition, stable >= 1.85
- **Async runtime:** Tokio (full features)
- **MCP framework:** rmcp (official Rust MCP SDK, stdio transport)
- **Token counting:** tiktoken (pure Rust, cl100k_base for Claude estimation)
- **Code analysis:** tree-sitter (Rust/TS/Python/Go grammars)
- **File discovery:** ignore crate (.gitignore-aware) + git2 (recency/blame)
- **Error handling:** thiserror (library types), anyhow (application)
- **Observability:** tracing + tracing-subscriber (ALWAYS to stderr, NEVER stdout)
- **Concurrency:** dashmap (index cache), rayon (parallel scoring)
- **Testing:** cargo-nextest, proptest, insta (snapshots), criterion (benchmarks)

## Architecture

```
Delivery (MCP/CLI) → Selection (Knapsack) → Scoring → Indexing (Files/AST/Dedup)
```

### Core Types

- `FileEntry` — path, token count, hash, metadata
- `ScoredEntry` — FileEntry + composite score + per-signal breakdown
- `PackResult` — selected files, L1/L2/L3 output, stats
- `Budget` — total tokens, L1/L2/L3 allocation percentages

### Key Files

- `src/mcp/tools.rs` — MCP tool definitions (the public API for agents)
- `src/selection/knapsack.rs` — core optimization algorithm
- `src/scoring/mod.rs` — score aggregation and weight management
- `src/index/discovery.rs` — file walking and git metadata extraction
- `src/index/tokenizer.rs` — tiktoken integration, per-file counting

## Build & Quality Commands

```bash
cargo build                                                    # Dev build
cargo build --release --features full                          # Release
cargo nextest run --all-features                               # Tests (preferred)
cargo clippy --all-targets --all-features -- -D warnings       # Lint
cargo fmt --check                                              # Format check
cargo bench                                                    # Benchmarks
cargo audit                                                    # Security
RUST_LOG=debug cargo run -- pack --budget 128000 --repo .      # Run CLI
```

## Coding Conventions

### MUST

- ALL errors use `thiserror` enums in library code, `anyhow` in main/CLI only
- ALL public items have `///` doc comments with `# Examples` section
- ALL scoring signals normalized to [0.0, 1.0] before weighted combination
- ALL file I/O uses the `ignore` crate walker, NEVER raw `std::fs::read_dir`
- ALL async code runs on tokio — use `tokio::task::spawn_blocking` for CPU work
- ALL MCP tool params derive `serde::Deserialize` + `schemars::JsonSchema`
- ALL logging via `tracing::{info,warn,error,debug,trace}!` to STDERR
- Run `cargo clippy` and `cargo fmt` before considering any change complete
- Use `Result<T, E>` everywhere — NEVER panic in library code
- Dedup BEFORE scoring (don't waste compute on duplicate files)
- Normalize scores before combining: `score = w1*s1 + w2*s2 + ... / sum(weights)`

### NEVER

- NEVER use `println!` or `eprintln!` — corrupts MCP stdio JSON-RPC stream
- NEVER use `.unwrap()` outside of tests — use `?` or `.context("msg")?`
- NEVER store secrets in code — use `.env` + `dotenvy`
- NEVER use `std::thread::sleep` — use `tokio::time::sleep`
- NEVER include the `target/` directory in file discovery results
- NEVER use `Rc`/`RefCell` in async code — use `Arc`/`tokio::sync::Mutex`
- NEVER add features to tiktoken beyond what's needed (binary size)

### Error Handling Pattern

```rust
#[derive(Debug, thiserror::Error)]
pub enum OptimError {
    #[error("file discovery failed: {0}")]
    Discovery(#[from] ignore::Error),
    #[error("git error: {0}")]
    Git(#[from] git2::Error),
    #[error("tokenizer error: {0}")]
    Tokenizer(String),
    #[error("budget exceeded: requested {requested}, max {max}")]
    BudgetExceeded { requested: usize, max: usize },
    #[error("no files found in {path}")]
    EmptyRepo { path: String },
}
```

### MCP Server Rules

- Tool functions return `Result<CallToolResult, ErrorData>` — NEVER panic
- Use `isError: true` in CallToolResult for tool-level errors, not exceptions
- Log to stderr: `tracing_subscriber::fmt().with_writer(std::io::stderr).init()`
- Keep tool descriptions clear and action-oriented for LLM agents
- Include `schemars(description = "...")` on ALL tool parameters

### Testing Conventions

- Unit tests: same file, `#[cfg(test)] mod tests { ... }`
- Integration tests: `tests/integration/` — test CLI and MCP protocol
- Fixtures: `tests/fixtures/` — pre-built temp repos via helper functions
- Snapshots: `insta` for output format stability — review with `cargo insta review`
- Benchmarks: `criterion` in `benches/` — scoring and knapsack hot paths
- Property tests: `proptest` for knapsack invariants (total tokens <= budget, etc.)
- Name tests descriptively: `test_knapsack_respects_budget_with_oversized_files`
- Test the scoring normalization: all signals must produce values in [0.0, 1.0]

### Git Conventions

- Branch: `feat/`, `fix/`, `refactor/`, `bench/`
- Commits: conventional commits (`feat: add proximity scoring signal`)
- PRs: must pass clippy + fmt + nextest + audit

## Phase Plan

1. **v0.1** — File inventory + token counting + greedy knapsack + CLI output
2. **v0.2** — MCP server delivery + MD5 dedup + config via TOML
3. **v0.3** — Tree-sitter skeletons + L1/L2/L3 hierarchical output
4. **v0.4** — SimHash near-dedup + submodular diversity + KKT solver
5. **v1.0** — Feedback loop + weight learning + file watcher

## Performance Targets

- Index 10K files in < 500ms
- Score + pack in < 50ms
- MCP tool response in < 100ms total
- Memory: < 100MB for 50K file repo
