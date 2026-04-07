# CLAUDE.md — Context Window Optimizer

Rust MCP server + CLI for intelligent code context selection. Packs the most valuable
files into a token budget using scoring, dedup, and knapsack optimization.
See @Cargo.toml for dependencies and features.

## CRITICAL — Read These First

- Use `tracing::{info,warn,error,debug,trace}!` for ALL logging — `println!`/`eprintln!` corrupt the MCP stdio JSON-RPC stream and break the server
- Use `?` or `.context("msg")?` for error propagation — `.unwrap()` is only allowed in `#[cfg(test)]` code
- Use `thiserror` enums in library code (`src/`), `anyhow` only in `src/main.rs`

## Sub-agents

When spawning sub-agents, ensure they have Edit, Bash, and Read permissions. If agents fail due to permission errors, fall back to doing the work directly without retrying agents.

## Verify Before Done

After any code change, run all three and fix issues before finishing:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --all-features
```

## Commands

```bash
cargo build                                                    # Dev build
cargo build --release --features full                          # Release (mcp+ast+watch+feedback)
cargo nextest run --all-features                               # Tests (use nextest, not cargo test)
cargo bench                                                    # Criterion benchmarks in benches/
RUST_LOG=debug cargo run -- pack --budget 128000 --repo .      # Run CLI
cargo run --features mcp -- serve                              # Start MCP stdio server
cargo run --features feedback -- feedback weights              # feedback/watch subcommands need their feature flag
```

## Architecture

`Delivery (MCP/CLI) → Selection (Knapsack) → Scoring → Indexing (Files/AST/Dedup)`

Pipeline: discover files → dedup (exact MD5, then SimHash near-dedup) → score → knapsack select → format L1/L2/L3 output. Entry point: `pack_files()` in `src/lib.rs`.

## Code Style

- Public items get `///` doc comments with `# Examples` section
- Scoring signals normalized to [0.0, 1.0] before weighted combination: `score = w1*s1 + w2*s2 + ... / sum(weights)`
- File I/O uses the `ignore` crate walker (respects .gitignore), not `std::fs::read_dir`
- Async code runs on tokio — use `tokio::task::spawn_blocking` for CPU-heavy work
- MCP tool params derive `serde::Deserialize` + `schemars::JsonSchema` with `schemars(description = "...")` on each field
- MCP tools return `Result<CallToolResult, ErrorData>` — use `isError: true` for tool-level errors
- Shared state in async code uses `Arc`/`tokio::sync::Mutex`, not `Rc`/`RefCell`
- Dedup runs BEFORE scoring (don't waste compute on duplicates)

## Testing

- Unit tests: `#[cfg(test)] mod tests` in the same file
- Integration tests: `tests/integration/` — CLI and MCP protocol
- Fixtures: `tests/fixtures/` via helper functions
- Snapshots: `insta` — review with `cargo insta review`
- Property tests: `proptest` for knapsack invariants (total tokens <= budget)
- Benchmarks: `criterion` in `benches/` — scoring and knapsack hot paths
- Name tests descriptively: `test_knapsack_respects_budget_with_oversized_files`

## Git

- Branch: `feat/`, `fix/`, `refactor/`, `bench/`
- Commits: conventional commits (`feat: add proximity scoring signal`)
- PRs: must pass fmt + clippy + nextest

## Compaction

When compacting, preserve: the verify commands above, the list of modified files, and any active error context.
