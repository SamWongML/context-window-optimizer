```
context-window-optimizer/
├── Cargo.toml
├── CLAUDE.md                    # ← Golden guidelines (see Section 6)
├── rustfmt.toml
├── clippy.toml
├── deny.toml                    # cargo-deny config
├── .cargo/
│   └── config.toml              # Linker config (lld)
├── config/
│   └── default.toml             # Default scoring weights, budget, etc.
├── src/
│   ├── main.rs                  # CLI + MCP server entrypoint
│   ├── lib.rs                   # Public API surface
│   ├── cli.rs                   # clap CLI definition
│   ├── config.rs                # Config loading (TOML + env)
│   ├── error.rs                 # thiserror error types
│   ├── index/
│   │   ├── mod.rs               # File inventory orchestrator
│   │   ├── discovery.rs         # ignore + git2 file walking
│   │   ├── tokenizer.rs         # bpe-openai exact counts + fast heuristic estimator
│   │   ├── dedup.rs             # MD5 + SimHash deduplication
│   │   └── skeleton.rs          # Phase 2: tree-sitter AST extraction
│   ├── scoring/
│   │   ├── mod.rs               # Score aggregation
│   │   ├── recency.rs           # Git log recency (Ebbinghaus decay)
│   │   ├── proximity.rs         # Dependency/import distance
│   │   ├── density.rs           # Information density (Shannon entropy)
│   │   └── weights.rs           # Configurable weight vector
│   ├── selection/
│   │   ├── mod.rs               # Selection orchestrator
│   │   ├── knapsack.rs          # Greedy + KKT dual bisection
│   │   ├── diversity.rs         # Submodular diversity constraint
│   │   └── budget.rs            # L1/L2/L3 budget allocation
│   ├── output/
│   │   ├── mod.rs               # Output formatting
│   │   ├── hierarchical.rs      # L1 skeleton / L2 cluster / L3 full
│   │   └── stats.rs             # Token savings reporting
│   └── mcp/
│       ├── mod.rs               # MCP server setup
│       └── tools.rs             # MCP tool definitions (optimize_context, etc.)
├── tests/
│   ├── integration/
│   │   ├── cli_test.rs          # End-to-end CLI tests
│   │   └── mcp_test.rs          # MCP protocol tests
│   └── fixtures/
│       ├── small_repo/          # 10-file test repo
│       └── medium_repo/         # 100-file test repo with git history
├── benches/
│   ├── scoring.rs               # Criterion benchmarks for scoring
│   └── knapsack.rs              # Criterion benchmarks for selection
└── examples/
    └── basic_pack.rs            # Minimal example: pack a directory
```
