<p align="center">
  <h1 align="center">ctx-optim</h1>
  <p align="center">
    <strong>Intelligent context selection for LLM agents</strong>
  </p>
  <p align="center">
    <a href="https://github.com/SamWongML/context_window_optimizer/actions"><img src="https://img.shields.io/github/actions/workflow/status/SamWongML/context_window_optimizer/ci.yml?style=flat-square&logo=github&label=CI" alt="CI"></a>
    <a href="https://crates.io/crates/ctx-optim"><img src="https://img.shields.io/crates/v/ctx-optim?style=flat-square&logo=rust" alt="crates.io"></a>
    <a href="https://github.com/SamWongML/context_window_optimizer/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="License: MIT"></a>
    <img src="https://img.shields.io/badge/rust-1.85%2B-orange?style=flat-square&logo=rust" alt="Rust 1.85+">
  </p>
</p>

---

Your repo has 10,000 files. Your LLM's context window fits 500.
**ctx-optim picks the 500 that matter most.**

It scores every file by git recency, size, proximity to your focus area, and import distance — then solves a knapsack optimization to maximize value within your token budget. Ships as an **MCP server** for Claude Code / Codex or as a standalone **CLI**.

```
10K files → Index → Dedup → Score → Select → 128K tokens of pure signal
```

## Highlights

- ⚡ **Fast** — indexes 10K files in <500ms, packs in <50ms
- 🎯 **Smart scoring** — git recency, file size, path proximity, dependency graph
- 🧹 **Deduplication** — exact (MD5) and near-duplicate (SimHash) removal
- 🌳 **AST-aware** — tree-sitter signatures for Rust, TypeScript, Python, Go
- 📐 **Diversity-aware** — prevents any single directory from dominating results
- 🔄 **Learns from feedback** — tracks which files the LLM actually used, tunes weights
- 📡 **MCP native** — stdio server, drop-in for any MCP client
- 👁️ **File watcher** — incremental re-indexing on source changes

## Quick start

### Install

**Prebuilt binaries** (no Rust toolchain required) — download the archive for your platform from the [latest release](https://github.com/SamWongML/context_window_optimizer/releases/latest), extract it, and put `ctx-optim` on your `$PATH`.

Or use the one-liner installers:

```bash
# macOS
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/SamWongML/context_window_optimizer/releases/latest/download/ctx-optim-installer.sh | sh

# Windows (PowerShell)
irm https://github.com/SamWongML/context_window_optimizer/releases/latest/download/ctx-optim-installer.ps1 | iex
```

> **macOS — first-run Gatekeeper warning.** The binary is not yet notarized by Apple, so on first launch macOS may say _"ctx-optim cannot be opened because the developer cannot be verified."_ Clear the quarantine flag once:
> ```bash
> xattr -dr com.apple.quarantine "$(which ctx-optim)"
> ```
> Or right-click the binary in Finder → **Open** → **Open**. Notarized builds are planned for v1.0.

> **Windows — first-run SmartScreen warning.** The binary is not yet signed with a Windows code-signing certificate. On first launch you may see _"Windows protected your PC."_ Click **More info** → **Run anyway**. Signed builds are planned for v1.0.

**Linux / other platforms, or if you want to build from source:**

```bash
cargo install --path . --features full
```

<details>
<summary>Minimal install (no AST / feedback / watcher)</summary>

```bash
cargo install --path . --no-default-features --features mcp
```

</details>

### Pack context (CLI)

```bash
# Default: 128K budget, full file contents
ctx-optim pack --repo .

# Focus on specific areas with a smaller budget
ctx-optim pack --budget 32000 --repo . --focus src/api src/models

# Just the skeleton map
ctx-optim pack --repo . --output l1 --signatures
```

### As an MCP server

```bash
ctx-optim serve
```

Add to your MCP client config (Claude Code, Codex, etc.):

```json
{
  "mcpServers": {
    "ctx-optim": {
      "command": "ctx-optim",
      "args": ["serve"]
    }
  }
}
```

The server exposes four tools:

| Tool | What it does |
|------|-------------|
| **`pack_context`** | Score, select, and pack files within a token budget |
| **`index_stats`** | File count, total tokens, language breakdown |
| **`submit_feedback`** | Record which files the LLM actually used |
| **`learned_weights`** | View or trigger weight learning |

## How it works

**Score → Select → Pack**, in three steps:

1. **Index** the repo — walk files (.gitignore-aware), count tokens, extract git metadata, deduplicate
2. **Score** each file on four signals (all normalized 0–1, then weighted):

   | Signal | Intuition |
   |--------|-----------|
   | Recency | Recently changed files are more relevant |
   | Size | Smaller, focused files carry denser signal |
   | Proximity | Files near your `--focus` paths matter more |
   | Dependency | Direct imports of focus files score higher |

3. **Select** via greedy knapsack with diversity decay — maximizes total value while ensuring broad coverage across the codebase

Output is layered: **L1** (skeleton map, 5%) → **L2** (metadata, 25%) → **L3** (full contents, 70%).

## Configuration

Drop a `ctx-optim.toml` in your repo root to customize:

```toml
[weights]
recency    = 0.4
size       = 0.15
proximity  = 0.2
dependency = 0.25

[selection.diversity]
enabled = true
decay   = 0.7        # penalty per extra file from same directory

[dedup]
exact = true
near  = false         # enable SimHash near-dedup
```

<details>
<summary>All configuration options</summary>

```toml
extra_ignore = ["target", "node_modules", "dist"]
max_file_bytes = 524288     # 512 KB
max_file_tokens = 8000

[weights]
recency    = 0.4
size       = 0.15
proximity  = 0.2
dependency = 0.25

[dedup]
exact = true
near  = false
hamming_threshold = 3
shingle_size = 3

[selection]
solver = "auto"
[selection.diversity]
enabled  = true
decay    = 0.7
grouping = "directory"

[feedback]
db_path       = ".ctx-optim/feedback.db"
learning_rate = 0.1
min_sessions  = 5
```

</details>

## Feature flags

| Flag | Default | What you get |
|------|---------|-------------|
| `mcp` | ✅ | MCP stdio server |
| `ast` | ✅ | Tree-sitter AST extraction |
| `feedback` | — | SQLite feedback loop + weight learning |
| `watch` | — | File watcher for incremental re-indexing |
| `full` | — | Everything above |

## Development

```bash
cargo build                                              # dev build
cargo nextest run --all-features                         # tests
cargo clippy --all-targets --all-features -- -D warnings # lint
cargo bench                                              # benchmarks
```

See [`CLAUDE.md`](CLAUDE.md) for coding conventions, architecture details, and contribution guidelines.

## License

[MIT](LICENSE)
