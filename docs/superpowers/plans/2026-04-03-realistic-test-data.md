# Realistic Test Data Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generate realistic git-backed test repositories with edge cases to verify the MCP pipeline's correctness and performance against CLAUDE.md targets.

**Architecture:** A composable `RealisticRepoBuilder` wraps `git2` to create temp repos with real commit history. Content generators produce syntactically valid code per language. Pre-built scenarios compose the builder into named archetypes. Integration tests verify correctness; criterion benchmarks measure performance.

**Tech Stack:** git2 (commit history), rand (content variation + commit patterns), criterion (benchmarks), tempfile (temp dirs), ctx_optim (the library under test)

---

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml` | Add `rand` dev-dependency, add `realistic` bench entry |
| `tests/fixtures/mod.rs` | Refactor to declare submodules; keep existing `TempRepo` |
| `tests/fixtures/content.rs` | Code generators: `generate_{rust,typescript,python,go}(size, index)` |
| `tests/fixtures/builder.rs` | `RealisticRepoBuilder` with git2 operations |
| `tests/fixtures/scenarios.rs` | 5 named scenario constructors |
| `tests/integration/main.rs` | Add `mod realistic;` and crate-level `mod fixtures;` |
| `tests/integration/realistic.rs` | Correctness integration tests (~15 tests) |
| `tests/integration/pack_pipeline.rs` | Remove local `mod fixtures` in favor of crate-level |
| `benches/realistic.rs` | Criterion benchmarks (6 benchmarks) |

---

### Task 1: Foundation — dependencies and module restructure

**Files:**
- Modify: `Cargo.toml`
- Modify: `tests/integration/main.rs`
- Modify: `tests/integration/pack_pipeline.rs`
- Modify: `tests/fixtures/mod.rs`

- [ ] **Step 1: Add `rand` dev-dependency and `realistic` bench entry to Cargo.toml**

Add to `[dev-dependencies]`:
```toml
rand = "0.8"
```

Add to the bottom (before `[profile.release]`):
```toml
[[bench]]
name = "realistic"
harness = false
```

- [ ] **Step 2: Refactor `tests/fixtures/mod.rs` to declare submodules**

Replace the entire file with:

```rust
// Test fixture helpers — create temporary repositories for integration tests.

pub mod builder;
pub mod content;
pub mod scenarios;

use std::path::Path;
use tempfile::TempDir;

/// A temporary repository with a known set of files.
pub struct TempRepo {
    pub dir: TempDir,
}

impl TempRepo {
    /// Create a minimal repo with a few Rust source files.
    pub fn minimal() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        write_file(root, "src/main.rs", MAIN_RS);
        write_file(root, "src/lib.rs", LIB_RS);
        write_file(root, "src/utils.rs", UTILS_RS);
        write_file(root, "README.md", README_MD);

        Self { dir }
    }

    /// Create a repo with duplicate files (same content, different paths).
    pub fn with_duplicates() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        let content = "fn duplicate() { /* same content */ }";
        write_file(root, "src/a.rs", content);
        write_file(root, "src/b.rs", content); // exact duplicate
        write_file(root, "src/c.rs", "fn unique() {}");

        Self { dir }
    }

    /// Create a larger repo for budget/selection tests.
    pub fn larger(n_files: usize) -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        for i in 0..n_files {
            let content = format!(
                "/// File {i}\npub fn function_{i}() -> usize {{\n    {i}\n}}\n"
            );
            write_file(root, &format!("src/module_{i}.rs"), &content);
        }

        Self { dir }
    }

    /// Create a repo with near-duplicate files (very similar content, small differences).
    pub fn with_near_duplicates() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        let base = r#"
use std::collections::HashMap;

pub fn process_data(input: &str) -> Result<String, Box<dyn std::error::Error>> {
    let trimmed = input.trim();
    let parsed: Vec<&str> = trimmed.split(',').collect();
    let mut result = String::new();
    for item in &parsed {
        if !item.is_empty() {
            result.push_str(item.trim());
            result.push('\n');
        }
    }
    Ok(result)
}

pub fn validate(input: &str) -> bool {
    !input.is_empty() && input.len() < 1000
}

pub fn count_items(input: &str) -> usize {
    input.split(',').filter(|s| !s.is_empty()).count()
}
"#;
        let variant = r#"
use std::collections::HashMap;

pub fn process_data(input: &str) -> Result<String, Box<dyn std::error::Error>> {
    let trimmed = input.trim();
    let parsed: Vec<&str> = trimmed.split(',').collect();
    let mut result = String::new();
    for item in &parsed {
        if !item.is_empty() {
            result.push_str(item.trim());
            result.push('\n');
        }
    }
    Ok(result)
}

pub fn validate(value: &str) -> bool {
    !value.is_empty() && value.len() < 1000
}

pub fn count_items(input: &str) -> usize {
    input.split(',').filter(|s| !s.is_empty()).count()
}
"#;

        write_file(root, "src/processor_a.rs", base);
        write_file(root, "src/processor_b.rs", variant);
        write_file(root, "src/unique.rs", "pub fn unique_function() -> u32 { 42 }");

        Self { dir }
    }

    /// Create a repo with files spread across multiple directories.
    pub fn with_directory_structure() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        write_file(root, "src/scoring/signals.rs", "pub fn recency() -> f32 { 0.9 }");
        write_file(root, "src/scoring/weights.rs", "pub fn default_weights() -> f32 { 0.5 }");
        write_file(root, "src/scoring/mod.rs", "pub mod signals;\npub mod weights;");

        write_file(root, "src/index/discovery.rs", "pub fn discover() -> Vec<String> { vec![] }");
        write_file(root, "src/index/tokenizer.rs", "pub fn count_tokens() -> usize { 0 }");
        write_file(root, "src/index/mod.rs", "pub mod discovery;\npub mod tokenizer;");

        write_file(root, "tests/test_scoring.rs", "fn test_score() { assert!(true); }");
        write_file(root, "tests/test_index.rs", "fn test_index() { assert!(true); }");

        Self { dir }
    }

    /// Root path of this repo.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

pub fn write_file(root: &Path, rel_path: &str, content: &str) {
    let full = root.join(rel_path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).expect("create dirs");
    }
    std::fs::write(full, content).expect("write fixture file");
}

// ── Static fixture content ──

const MAIN_RS: &str = r#"
fn main() {
    println!("Context Window Optimizer");
}
"#;

const LIB_RS: &str = r#"
/// Library root.
pub mod utils;

/// Pack context for an LLM.
pub fn pack() -> Vec<String> {
    vec![]
}
"#;

const UTILS_RS: &str = r#"
/// Utility functions.

/// Compute the score of a file based on its age.
pub fn recency_score(age_days: f64) -> f64 {
    (-0.023 * age_days).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recency_score() {
        assert!(recency_score(0.0) > 0.99);
    }
}
"#;

const README_MD: &str = r#"
# Context Window Optimizer

A Rust-based MCP server that intelligently packs code context for LLM agents.
"#;
```

- [ ] **Step 3: Update `tests/integration/main.rs` to use crate-level fixtures**

Replace the entire file with:

```rust
//! Integration tests for the ctx-optim CLI and library.

#[path = "../fixtures/mod.rs"]
mod fixtures;

mod cli;
mod pack_pipeline;
mod realistic;

#[cfg(feature = "feedback")]
mod feedback;

#[cfg(feature = "watch")]
mod watch;
```

- [ ] **Step 4: Update `tests/integration/pack_pipeline.rs` to use crate-level fixtures**

Remove lines 3-5 (the local `mod fixtures { include!(...) }` block) and add a `use` import at the top:

Replace:
```rust
mod fixtures {
    include!("../../tests/fixtures/mod.rs");
}

use ctx_optim::selection::diversity::{DiversityConfig, GroupingStrategy};
use ctx_optim::{config::Config, pack_files, types::Budget};
use fixtures::TempRepo;
```

With:
```rust
use crate::fixtures::TempRepo;
use ctx_optim::selection::diversity::{DiversityConfig, GroupingStrategy};
use ctx_optim::{config::Config, pack_files, types::Budget};
```

- [ ] **Step 5: Create placeholder files so compilation succeeds**

Create empty placeholder files:

`tests/fixtures/content.rs`:
```rust
// Content generators — placeholder, implemented in Task 2.
```

`tests/fixtures/builder.rs`:
```rust
// RealisticRepoBuilder — placeholder, implemented in Task 3.
```

`tests/fixtures/scenarios.rs`:
```rust
// Scenario constructors — placeholder, implemented in Task 4.
```

`tests/integration/realistic.rs`:
```rust
// Realistic integration tests — placeholder, implemented in Task 5.
```

- [ ] **Step 6: Run existing tests to verify refactor didn't break anything**

Run: `cargo nextest run --all-features -E 'test(pack_pipeline)'`
Expected: All existing pack_pipeline tests pass.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml tests/fixtures/ tests/integration/main.rs tests/integration/pack_pipeline.rs tests/integration/realistic.rs
git commit -m "refactor: restructure test fixtures as proper modules, add rand + bench entry"
```

---

### Task 2: Content Generators

**Files:**
- Create: `tests/fixtures/content.rs`

- [ ] **Step 1: Implement content generators**

Write the complete `tests/fixtures/content.rs`:

```rust
//! Generates syntactically valid code per language and size tier.
//!
//! The `index` parameter is embedded in identifiers to ensure uniqueness
//! across files, preventing accidental deduplication.

/// Approximate token budget for each size tier.
#[derive(Debug, Clone, Copy)]
pub enum FileSize {
    /// ~20 tokens
    Tiny,
    /// ~100 tokens
    Small,
    /// ~500 tokens
    Medium,
    /// ~2000 tokens
    Large,
    /// ~6000 tokens
    Huge,
}

/// Generate syntactically valid Rust source code.
pub fn generate_rust(size: FileSize, index: usize) -> String {
    match size {
        FileSize::Tiny => format!(
            "/// Auto-generated module {index}.\npub fn func_{index}() -> usize {{ {index} }}\n"
        ),
        FileSize::Small => format!(
            r#"use std::fmt;

/// Data holder for module {index}.
#[derive(Debug, Clone)]
pub struct Data{index} {{
    pub name: String,
    pub value: f64,
    pub active: bool,
}}

impl Data{index} {{
    pub fn new(name: &str) -> Self {{
        Self {{
            name: name.to_string(),
            value: {index} as f64 * 0.1,
            active: true,
        }}
    }}
}}

impl fmt::Display for Data{index} {{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{
        write!(f, "Data{index}({{}})", self.name)
    }}
}}
"#
        ),
        FileSize::Medium => rust_medium_block(index),
        FileSize::Large => {
            let mut out = format!("//! Module {index} — large generated file.\n\n");
            for i in 0..4 {
                out.push_str(&rust_medium_block(index * 10 + i));
                out.push('\n');
            }
            out
        }
        FileSize::Huge => {
            let mut out = format!("//! Module {index} — huge generated file.\n\n");
            for i in 0..12 {
                out.push_str(&rust_medium_block(index * 100 + i));
                out.push('\n');
            }
            out
        }
    }
}

fn rust_medium_block(index: usize) -> String {
    format!(
        r#"use std::collections::HashMap;

/// Error type for processor {index}.
#[derive(Debug)]
pub enum ProcessError{index} {{
    InvalidInput(String),
    NotFound {{ key: String }},
    Internal(String),
}}

impl std::fmt::Display for ProcessError{index} {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
        match self {{
            Self::InvalidInput(msg) => write!(f, "invalid input: {{msg}}"),
            Self::NotFound {{ key }} => write!(f, "not found: {{key}}"),
            Self::Internal(msg) => write!(f, "internal error: {{msg}}"),
        }}
    }}
}}

/// Configuration for processor {index}.
#[derive(Debug, Clone)]
pub struct Config{index} {{
    pub name: String,
    pub threshold: f64,
    pub max_retries: usize,
    pub tags: Vec<String>,
}}

impl Config{index} {{
    pub fn new(name: &str) -> Self {{
        Self {{
            name: name.to_string(),
            threshold: {index} as f64 * 0.01,
            max_retries: 3,
            tags: vec!["default".to_string()],
        }}
    }}

    pub fn validate(&self) -> Result<(), ProcessError{index}> {{
        if self.name.is_empty() {{
            return Err(ProcessError{index}::InvalidInput(
                "name cannot be empty".to_string(),
            ));
        }}
        if self.threshold < 0.0 || self.threshold > 1.0 {{
            return Err(ProcessError{index}::InvalidInput(
                format!("threshold out of range: {{}}", self.threshold),
            ));
        }}
        Ok(())
    }}

    pub fn process(
        &self,
        items: &HashMap<String, f64>,
    ) -> Result<Vec<(String, f64)>, ProcessError{index}> {{
        self.validate()?;
        let mut results: Vec<(String, f64)> = items
            .iter()
            .filter(|(_, &v)| v >= self.threshold)
            .map(|(k, &v)| (k.clone(), v * (1.0 + {index} as f64 * 0.001)))
            .collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        if results.is_empty() {{
            return Err(ProcessError{index}::NotFound {{
                key: format!("no items above threshold {{}}", self.threshold),
            }});
        }}
        Ok(results)
    }}
}}

#[cfg(test)]
mod tests_{index} {{
    use super::*;

    #[test]
    fn test_config_{index}_validate_empty_name() {{
        let mut cfg = Config{index}::new("test");
        cfg.name = String::new();
        assert!(cfg.validate().is_err());
    }}

    #[test]
    fn test_config_{index}_process_filters() {{
        let cfg = Config{index}::new("test");
        let mut items = HashMap::new();
        items.insert("high".to_string(), 0.9);
        items.insert("low".to_string(), 0.001);
        let result = cfg.process(&items);
        assert!(result.is_ok());
    }}
}}
"#
    )
}

/// Generate syntactically valid TypeScript source code.
pub fn generate_typescript(size: FileSize, index: usize) -> String {
    match size {
        FileSize::Tiny => format!(
            "export const VALUE_{index} = {index};\nexport type Id{index} = number;\n"
        ),
        FileSize::Small => format!(
            r#"export interface Item{index} {{
  id: number;
  name: string;
  active: boolean;
  score: number;
}}

export function createItem{index}(name: string): Item{index} {{
  return {{
    id: {index},
    name,
    active: true,
    score: {index} * 0.1,
  }};
}}

export function validateItem{index}(item: Item{index}): boolean {{
  return item.name.length > 0 && item.score >= 0;
}}
"#
        ),
        FileSize::Medium => ts_medium_block(index),
        FileSize::Large => {
            let mut out = format!("// Module {index} — large generated file\n\n");
            for i in 0..4 {
                out.push_str(&ts_medium_block(index * 10 + i));
                out.push('\n');
            }
            out
        }
        FileSize::Huge => {
            let mut out = format!("// Module {index} — huge generated file\n\n");
            for i in 0..12 {
                out.push_str(&ts_medium_block(index * 100 + i));
                out.push('\n');
            }
            out
        }
    }
}

fn ts_medium_block(index: usize) -> String {
    format!(
        r#"import {{ useState, useEffect, useCallback }} from 'react';

interface Props{index} {{
  initialCount: number;
  label: string;
  onUpdate: (value: number) => void;
  disabled?: boolean;
}}

interface State{index} {{
  count: number;
  history: number[];
  loading: boolean;
  error: string | null;
}}

export function Component{index}({{ initialCount, label, onUpdate, disabled }}: Props{index}) {{
  const [state, setState] = useState<State{index}>({{
    count: initialCount,
    history: [],
    loading: false,
    error: null,
  }});

  useEffect(() => {{
    setState(prev => ({{ ...prev, count: initialCount }}));
  }}, [initialCount]);

  const increment = useCallback(() => {{
    if (disabled) return;
    setState(prev => {{
      const next = prev.count + 1;
      onUpdate(next);
      return {{
        ...prev,
        count: next,
        history: [...prev.history, next].slice(-{index_plus_ten}),
      }};
    }});
  }}, [disabled, onUpdate]);

  const reset = useCallback(() => {{
    setState({{ count: 0, history: [], loading: false, error: null }});
    onUpdate(0);
  }}, [onUpdate]);

  const average = state.history.length > 0
    ? state.history.reduce((a, b) => a + b, 0) / state.history.length
    : 0;

  if (state.loading) return <div>Loading {index}...</div>;
  if (state.error) return <div>Error: {{state.error}}</div>;

  return (
    <div className="component-{index}">
      <h2>{{label}} #{index}</h2>
      <p>Count: {{state.count}}</p>
      <p>Average: {{average.toFixed(2)}}</p>
      <button onClick={{increment}} disabled={{disabled}}>+1</button>
      <button onClick={{reset}}>Reset</button>
      <ul>
        {{state.history.map((v, i) => <li key={{i}}>{{v}}</li>)}}
      </ul>
    </div>
  );
}}

export default Component{index};
"#,
        index = index,
        index_plus_ten = index + 10,
    )
}

/// Generate syntactically valid Python source code.
pub fn generate_python(size: FileSize, index: usize) -> String {
    match size {
        FileSize::Tiny => format!(
            "def func_{index}():\n    \"\"\"Auto-generated function {index}.\"\"\"\n    return {index}\n"
        ),
        FileSize::Small => format!(
            r#"from dataclasses import dataclass


@dataclass
class Item{index}:
    """Data item for module {index}."""
    name: str
    value: float = {index}.0
    active: bool = True

    def validate(self) -> bool:
        return len(self.name) > 0 and self.value >= 0

    def to_dict(self) -> dict:
        return {{"name": self.name, "value": self.value, "active": self.active}}
"#
        ),
        FileSize::Medium => python_medium_block(index),
        FileSize::Large => {
            let mut out = format!("\"\"\"Module {index} — large generated file.\"\"\"\n\n");
            for i in 0..4 {
                out.push_str(&python_medium_block(index * 10 + i));
                out.push('\n');
            }
            out
        }
        FileSize::Huge => {
            let mut out = format!("\"\"\"Module {index} — huge generated file.\"\"\"\n\n");
            for i in 0..12 {
                out.push_str(&python_medium_block(index * 100 + i));
                out.push('\n');
            }
            out
        }
    }
}

fn python_medium_block(index: usize) -> String {
    format!(
        r#"from __future__ import annotations
import logging
from dataclasses import dataclass, field
from typing import Optional

logger = logging.getLogger(__name__)


class ProcessError{index}(Exception):
    """Custom error for processor {index}."""
    pass


@dataclass
class Config{index}:
    """Configuration for processor {index}."""
    name: str
    threshold: float = {threshold}
    max_retries: int = 3
    tags: list[str] = field(default_factory=lambda: ["default"])

    def validate(self) -> None:
        if not self.name:
            raise ProcessError{index}("name cannot be empty")
        if not 0.0 <= self.threshold <= 1.0:
            raise ProcessError{index}(f"threshold out of range: {{self.threshold}}")


class Processor{index}:
    """Processes items according to config {index}."""

    def __init__(self, config: Config{index}) -> None:
        self._config = config
        self._results: list[tuple[str, float]] = []

    def process(self, items: dict[str, float]) -> list[tuple[str, float]]:
        self._config.validate()
        self._results = [
            (k, v * (1.0 + {index} * 0.001))
            for k, v in items.items()
            if v >= self._config.threshold
        ]
        self._results.sort(key=lambda x: x[1], reverse=True)
        if not self._results:
            raise ProcessError{index}(
                f"no items above threshold {{self._config.threshold}}"
            )
        logger.info("Processor {index}: processed %d items", len(self._results))
        return self._results

    @property
    def last_results(self) -> list[tuple[str, float]]:
        return list(self._results)

    def reset(self) -> None:
        self._results.clear()


def create_processor_{index}(name: str, threshold: Optional[float] = None) -> Processor{index}:
    """Factory function for Processor{index}."""
    config = Config{index}(name=name)
    if threshold is not None:
        config.threshold = threshold
    return Processor{index}(config)
"#,
        index = index,
        threshold = index as f64 * 0.01,
    )
}

/// Generate syntactically valid Go source code.
pub fn generate_go(size: FileSize, index: usize) -> String {
    match size {
        FileSize::Tiny => format!(
            "package mod{index}\n\n// Func{index} returns {index}.\nfunc Func{index}() int {{ return {index} }}\n"
        ),
        FileSize::Small => format!(
            r#"package mod{index}

import "fmt"

// Item{index} holds data for module {index}.
type Item{index} struct {{
	Name   string
	Value  float64
	Active bool
}}

// NewItem{index} creates an Item{index} with defaults.
func NewItem{index}(name string) *Item{index} {{
	return &Item{index}{{
		Name:   name,
		Value:  float64({index}) * 0.1,
		Active: true,
	}}
}}

// String implements fmt.Stringer.
func (it *Item{index}) String() string {{
	return fmt.Sprintf("Item{index}(%s)", it.Name)
}}
"#
        ),
        FileSize::Medium => go_medium_block(index),
        FileSize::Large => {
            let mut out = format!("package mod{index}\n\n");
            out.push_str(&go_medium_block(index));
            for i in 1..4 {
                out.push_str(&go_medium_inner(index * 10 + i));
                out.push('\n');
            }
            out
        }
        FileSize::Huge => {
            let mut out = format!("package mod{index}\n\n");
            out.push_str(&go_medium_block(index));
            for i in 1..12 {
                out.push_str(&go_medium_inner(index * 100 + i));
                out.push('\n');
            }
            out
        }
    }
}

fn go_medium_block(index: usize) -> String {
    format!(
        r#"package mod{index}

import (
	"errors"
	"fmt"
	"sort"
)

{inner}
"#,
        index = index,
        inner = go_medium_inner(index),
    )
}

fn go_medium_inner(index: usize) -> String {
    format!(
        r#"// ErrInvalid{index} is returned when input validation fails.
var ErrInvalid{index} = errors.New("invalid input for processor {index}")

// Config{index} configures processor {index}.
type Config{index} struct {{
	Name       string
	Threshold  float64
	MaxRetries int
	Tags       []string
}}

// Processor{index} processes items according to Config{index}.
type Processor{index} struct {{
	config  Config{index}
	results []Result{index}
}}

// Result{index} holds a single processing result.
type Result{index} struct {{
	Key   string
	Value float64
}}

// NewProcessor{index} creates a Processor{index} with the given config.
func NewProcessor{index}(cfg Config{index}) (*Processor{index}, error) {{
	if cfg.Name == "" {{
		return nil, fmt.Errorf("%w: name is empty", ErrInvalid{index})
	}}
	if cfg.Threshold < 0 || cfg.Threshold > 1 {{
		return nil, fmt.Errorf("%w: threshold %.2f out of range", ErrInvalid{index}, cfg.Threshold)
	}}
	return &Processor{index}{{config: cfg}}, nil
}}

// Process filters and sorts items above the threshold.
func (p *Processor{index}) Process(items map[string]float64) ([]Result{index}, error) {{
	p.results = nil
	for k, v := range items {{
		if v >= p.config.Threshold {{
			p.results = append(p.results, Result{index}{{
				Key:   k,
				Value: v * (1.0 + float64({index})*0.001),
			}})
		}}
	}}
	sort.Slice(p.results, func(i, j int) bool {{
		return p.results[i].Value > p.results[j].Value
	}})
	if len(p.results) == 0 {{
		return nil, fmt.Errorf("no items above threshold %.4f", p.config.Threshold)
	}}
	return p.results, nil
}}

// Reset clears the last results.
func (p *Processor{index}) Reset() {{
	p.results = nil
}}
"#
    )
}

/// Generate non-code file content (markdown, TOML, JSON, YAML, CSS).
pub fn generate_non_code(filename: &str, index: usize) -> String {
    if filename.ends_with(".md") {
        format!(
            "# Document {index}\n\nThis is documentation for module {index}.\n\n## Overview\n\nModule {index} provides utilities for processing data.\n\n## Usage\n\n```\nimport mod{index}\nmod{index}.process(data)\n```\n"
        )
    } else if filename.ends_with(".toml") {
        format!(
            "[package]\nname = \"module-{index}\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n"
        )
    } else if filename.ends_with(".json") {
        format!(
            "{{\n  \"name\": \"module-{index}\",\n  \"version\": \"1.0.0\",\n  \"main\": \"index.js\",\n  \"scripts\": {{\n    \"build\": \"tsc\",\n    \"test\": \"jest\"\n  }}\n}}\n"
        )
    } else if filename.ends_with(".yaml") || filename.ends_with(".yml") {
        format!(
            "name: service-{index}\nversion: '1.0'\nport: {port}\nlog_level: info\nmax_connections: 100\n",
            port = 3000 + index,
        )
    } else if filename.ends_with(".css") {
        format!(
            ".component-{index} {{\n  display: flex;\n  padding: 16px;\n  margin: 8px;\n  border: 1px solid #ccc;\n  border-radius: 4px;\n}}\n\n.component-{index} h2 {{\n  font-size: 18px;\n  color: #333;\n}}\n"
        )
    } else {
        format!("// Generated file {index}\n")
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --all-features --tests`
Expected: Compiles with no errors.

- [ ] **Step 3: Commit**

```bash
git add tests/fixtures/content.rs
git commit -m "feat: add content generators for realistic test fixtures (Rust/TS/Python/Go)"
```

---

### Task 3: RealisticRepoBuilder

**Files:**
- Create: `tests/fixtures/builder.rs`

- [ ] **Step 1: Implement the builder**

Write the complete `tests/fixtures/builder.rs`:

```rust
//! Git-backed temporary repository builder for realistic test scenarios.
//!
//! Uses `git2` to create real commit history with backdated timestamps.

use super::write_file;
use super::content::{self, FileSize};
use rand::prelude::*;
use rand::rngs::StdRng;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

/// A built test repository with real git history.
pub struct RealisticRepo {
    pub dir: TempDir,
}

impl RealisticRepo {
    /// Root path of this repo.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// Commit pattern for simulating realistic git history.
pub enum CommitPattern {
    /// 80% of commits touch `hot_paths`, 20% touch the rest.
    Hotspot {
        hot_paths: Vec<String>,
        total_commits: usize,
        span_days: u64,
    },
    /// Commits spread evenly across all files over `span_days`.
    Uniform {
        total_commits: usize,
        span_days: u64,
    },
    /// Most files committed once long ago, a few burst recently.
    Stale {
        initial_age_days: u64,
        burst_paths: Vec<String>,
        burst_age_days: u64,
    },
}

/// Detected language for content generation (mirrors ctx_optim::types::Language
/// without depending on the library).
#[derive(Debug, Clone, Copy)]
pub enum Lang {
    Rust,
    TypeScript,
    Python,
    Go,
}

/// A pending file to be written and committed.
struct PendingFile {
    rel_path: String,
    content: String,
}

/// Builder for creating realistic temporary git repositories.
///
/// # Examples
///
/// ```ignore
/// let repo = RealisticRepoBuilder::new()
///     .add_file("src/main.rs", Lang::Rust, FileSize::Medium)
///     .initial_commit(180)
///     .build();
/// ```
pub struct RealisticRepoBuilder {
    files: Vec<PendingFile>,
    initial_commit_days_ago: Option<u64>,
    commit_patterns: Vec<CommitPattern>,
    recent_edits: Vec<(Vec<String>, u64)>,
    seed: u64,
    file_counter: usize,
}

impl RealisticRepoBuilder {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            initial_commit_days_ago: None,
            commit_patterns: Vec::new(),
            recent_edits: Vec::new(),
            seed: 42,
            file_counter: 0,
        }
    }

    /// Set the RNG seed for deterministic builds.
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Add a single file with auto-generated content.
    pub fn add_file(mut self, path: &str, lang: Lang, size: FileSize) -> Self {
        let idx = self.file_counter;
        self.file_counter += 1;
        let content = match lang {
            Lang::Rust => content::generate_rust(size, idx),
            Lang::TypeScript => content::generate_typescript(size, idx),
            Lang::Python => content::generate_python(size, idx),
            Lang::Go => content::generate_go(size, idx),
        };
        self.files.push(PendingFile {
            rel_path: path.to_string(),
            content,
        });
        self
    }

    /// Add a file with explicit content.
    pub fn add_file_with_content(mut self, path: &str, content: &str) -> Self {
        self.files.push(PendingFile {
            rel_path: path.to_string(),
            content: content.to_string(),
        });
        self
    }

    /// Add a directory of auto-generated files.
    pub fn add_module(mut self, dir: &str, lang: Lang, count: usize) -> Self {
        let ext = match lang {
            Lang::Rust => "rs",
            Lang::TypeScript => "ts",
            Lang::Python => "py",
            Lang::Go => "go",
        };
        let sizes = [FileSize::Small, FileSize::Medium, FileSize::Medium, FileSize::Large];
        for i in 0..count {
            let size = sizes[i % sizes.len()];
            let idx = self.file_counter;
            self.file_counter += 1;
            let content = match lang {
                Lang::Rust => content::generate_rust(size, idx),
                Lang::TypeScript => content::generate_typescript(size, idx),
                Lang::Python => content::generate_python(size, idx),
                Lang::Go => content::generate_go(size, idx),
            };
            self.files.push(PendingFile {
                rel_path: format!("{dir}file_{i}.{ext}"),
                content,
            });
        }
        self
    }

    /// Add exact duplicate files (same content, different paths).
    pub fn add_exact_duplicates(mut self, dir: &str, count: usize) -> Self {
        let content = content::generate_rust(FileSize::Small, 99999);
        for i in 0..count {
            self.files.push(PendingFile {
                rel_path: format!("{dir}dup_{i}.rs"),
                content: content.clone(),
            });
        }
        self
    }

    /// Add near-duplicate files with `similarity` fraction of shared lines.
    pub fn add_near_duplicates(mut self, dir: &str, count: usize, similarity: f64) -> Self {
        let base = content::generate_rust(FileSize::Medium, 88888);
        let lines: Vec<&str> = base.lines().collect();
        let n_lines = lines.len();
        let n_keep = (n_lines as f64 * similarity) as usize;
        let mut rng = StdRng::seed_from_u64(self.seed + 7777);

        for i in 0..count {
            let mut variant_lines = lines.clone();
            // Mutate (n_lines - n_keep) random lines
            for _ in 0..(n_lines.saturating_sub(n_keep)) {
                let idx = rng.gen_range(0..variant_lines.len());
                variant_lines[idx] = "// mutated line for near-duplicate variation";
            }
            let variant = variant_lines.join("\n");
            self.files.push(PendingFile {
                rel_path: format!("{dir}near_dup_{i}.rs"),
                content: variant,
            });
        }
        self
    }

    /// Add empty files (0 bytes, 0 tokens).
    pub fn add_empty_files(mut self, count: usize) -> Self {
        for i in 0..count {
            self.files.push(PendingFile {
                rel_path: format!("empty/empty_{i}.rs"),
                content: String::new(),
            });
        }
        self
    }

    /// Add a file targeting approximately `approx_tokens` tokens.
    pub fn add_large_file(mut self, path: &str, approx_tokens: usize) -> Self {
        // Each medium block is ~500 tokens. Repeat to reach target.
        let blocks = (approx_tokens / 500).max(1);
        let mut content = String::new();
        for i in 0..blocks {
            content.push_str(&content::generate_rust(FileSize::Medium, 70000 + i));
            content.push('\n');
        }
        self.files.push(PendingFile {
            rel_path: path.to_string(),
            content,
        });
        self
    }

    /// Add files nested `depth` levels deep with `files_per_level` files at each level.
    pub fn add_deep_nesting(mut self, base: &str, files_per_level: usize, depth: usize) -> Self {
        let mut prefix = base.to_string();
        for d in 0..depth {
            for f in 0..files_per_level {
                let idx = self.file_counter;
                self.file_counter += 1;
                self.files.push(PendingFile {
                    rel_path: format!("{prefix}file_{f}.rs"),
                    content: content::generate_rust(FileSize::Tiny, idx),
                });
            }
            prefix = format!("{prefix}level_{d}/");
        }
        self
    }

    /// Add common non-code files (README, config, etc).
    pub fn add_non_code_files(mut self) -> Self {
        for (path, idx) in [
            ("README.md", 0),
            ("docs/architecture.md", 1),
            ("docs/setup.md", 2),
            ("config.toml", 3),
            ("package.json", 4),
            ("config/app.yaml", 5),
            ("styles/main.css", 6),
        ] {
            self.files.push(PendingFile {
                rel_path: path.to_string(),
                content: content::generate_non_code(path, idx),
            });
        }
        self
    }

    /// Set the age (in days ago) of the initial commit containing all files.
    pub fn initial_commit(mut self, days_ago: u64) -> Self {
        self.initial_commit_days_ago = Some(days_ago);
        self
    }

    /// Add a commit pattern for simulating realistic git history.
    pub fn commit_pattern(mut self, pattern: CommitPattern) -> Self {
        self.commit_patterns.push(pattern);
        self
    }

    /// Touch specific files with a recent commit.
    pub fn recent_edits(mut self, paths: &[&str], days_ago: u64) -> Self {
        self.recent_edits
            .push((paths.iter().map(|s| s.to_string()).collect(), days_ago));
        self
    }

    /// Build the repository, creating all files and git history.
    pub fn build(self) -> RealisticRepo {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path();

        // Write all files to disk
        for f in &self.files {
            write_file(root, &f.rel_path, &f.content);
        }

        // Initialize git repository and create history
        let repo = git2::Repository::init(root).expect("git init");
        let initial_days = self.initial_commit_days_ago.unwrap_or(0);

        // Initial commit with all files
        git_add_all_and_commit(&repo, "initial commit", days_ago_epoch(initial_days));

        // Apply commit patterns
        let all_paths: Vec<String> = self.files.iter().map(|f| f.rel_path.clone()).collect();
        let mut rng = StdRng::seed_from_u64(self.seed);

        for pattern in &self.commit_patterns {
            match pattern {
                CommitPattern::Hotspot {
                    hot_paths,
                    total_commits,
                    span_days,
                } => {
                    let hot: Vec<&str> = all_paths
                        .iter()
                        .filter(|p| hot_paths.iter().any(|hp| p.starts_with(hp.as_str())))
                        .map(|s| s.as_str())
                        .collect();
                    let cold: Vec<&str> = all_paths
                        .iter()
                        .filter(|p| !hot_paths.iter().any(|hp| p.starts_with(hp.as_str())))
                        .map(|s| s.as_str())
                        .collect();

                    for i in 0..*total_commits {
                        let age_days =
                            initial_days.saturating_sub(*span_days * i as u64 / *total_commits as u64);
                        let pick_hot = rng.gen_bool(0.8) && !hot.is_empty();
                        let file = if pick_hot {
                            hot[rng.gen_range(0..hot.len())]
                        } else if !cold.is_empty() {
                            cold[rng.gen_range(0..cold.len())]
                        } else {
                            continue;
                        };

                        let full_path = root.join(file);
                        if full_path.exists() {
                            append_comment(&full_path, i);
                            git_add_and_commit(
                                &repo,
                                file,
                                &format!("update {file} (commit {i})"),
                                days_ago_epoch(age_days),
                            );
                        }
                    }
                }
                CommitPattern::Uniform {
                    total_commits,
                    span_days,
                } => {
                    for i in 0..*total_commits {
                        if all_paths.is_empty() {
                            break;
                        }
                        let file = &all_paths[rng.gen_range(0..all_paths.len())];
                        let age_days =
                            initial_days.saturating_sub(*span_days * i as u64 / *total_commits as u64);

                        let full_path = root.join(file);
                        if full_path.exists() {
                            append_comment(&full_path, i);
                            git_add_and_commit(
                                &repo,
                                file,
                                &format!("update {file} (commit {i})"),
                                days_ago_epoch(age_days),
                            );
                        }
                    }
                }
                CommitPattern::Stale {
                    initial_age_days: _,
                    burst_paths,
                    burst_age_days,
                } => {
                    // The initial commit already covers initial_age_days.
                    // Now add burst commits on specified paths.
                    let burst_files: Vec<&str> = all_paths
                        .iter()
                        .filter(|p| burst_paths.iter().any(|bp| p.starts_with(bp.as_str())))
                        .map(|s| s.as_str())
                        .collect();

                    for (i, file) in burst_files.iter().enumerate() {
                        let full_path = root.join(file);
                        if full_path.exists() {
                            append_comment(&full_path, i + 10000);
                            git_add_and_commit(
                                &repo,
                                file,
                                &format!("burst update {file}"),
                                days_ago_epoch(*burst_age_days),
                            );
                        }
                    }
                }
            }
        }

        // Apply recent edits
        for (paths, days) in &self.recent_edits {
            for (i, file) in paths.iter().enumerate() {
                let full_path = root.join(file);
                if full_path.exists() {
                    append_comment(&full_path, 50000 + i);
                    git_add_and_commit(
                        &repo,
                        file,
                        &format!("recent edit to {file}"),
                        days_ago_epoch(*days),
                    );
                }
            }
        }

        RealisticRepo { dir }
    }
}

// ── Git helpers ──────────────────────────────────────────────────────────────

fn days_ago_epoch(days: u64) -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    now - (days as i64 * 86400)
}

fn make_sig(epoch_secs: i64) -> git2::Signature<'static> {
    git2::Signature::new(
        "Test Author",
        "test@example.com",
        &git2::Time::new(epoch_secs, 0),
    )
    .expect("create git signature")
}

fn git_add_all_and_commit(repo: &git2::Repository, message: &str, epoch_secs: i64) {
    let mut index = repo.index().expect("get index");
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("add all");
    index.write().expect("write index");
    let tree_oid = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_oid).expect("find tree");
    let sig = make_sig(epoch_secs);

    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    match parent {
        Some(ref p) => {
            repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[p])
                .expect("commit with parent");
        }
        None => {
            repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
                .expect("initial commit");
        }
    }
}

fn git_add_and_commit(
    repo: &git2::Repository,
    rel_path: &str,
    message: &str,
    epoch_secs: i64,
) {
    let mut index = repo.index().expect("get index");
    index
        .add_path(Path::new(rel_path))
        .expect("add path to index");
    index.write().expect("write index");
    let tree_oid = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_oid).expect("find tree");
    let sig = make_sig(epoch_secs);

    let parent = repo
        .head()
        .expect("head exists after initial commit")
        .peel_to_commit()
        .expect("peel to commit");
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
        .expect("commit");
}

fn append_comment(path: &Path, counter: usize) {
    let mut content = std::fs::read_to_string(path).unwrap_or_default();
    // Detect language from extension to use correct comment syntax
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let comment = match ext {
        "py" => format!("\n# update {counter}\n"),
        "md" => format!("\n<!-- update {counter} -->\n"),
        "yaml" | "yml" | "toml" => format!("\n# update {counter}\n"),
        "json" => return, // JSON doesn't support comments; skip
        "css" => format!("\n/* update {counter} */\n"),
        _ => format!("\n// update {counter}\n"),
    };
    content.push_str(&comment);
    std::fs::write(path, content).expect("append comment");
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --all-features --tests`
Expected: Compiles with no errors.

- [ ] **Step 3: Commit**

```bash
git add tests/fixtures/builder.rs
git commit -m "feat: add RealisticRepoBuilder with git2-backed commit history"
```

---

### Task 4: Scenarios

**Files:**
- Create: `tests/fixtures/scenarios.rs`

- [ ] **Step 1: Implement all five scenarios**

Write the complete `tests/fixtures/scenarios.rs`:

```rust
//! Pre-built repo scenarios for integration tests and benchmarks.

use super::builder::{CommitPattern, Lang, RealisticRepo, RealisticRepoBuilder};
use super::content::FileSize;

/// Web fullstack project (~200 files): React + Express, vendor duplication, hotspot git.
pub fn web_fullstack() -> RealisticRepo {
    RealisticRepoBuilder::new()
        .seed(1)
        // Frontend components
        .add_module("src/components/", Lang::TypeScript, 40)
        .add_module("src/hooks/", Lang::TypeScript, 10)
        .add_module("src/styles/", Lang::TypeScript, 5) // small utility files
        // Backend
        .add_module("src/api/routes/", Lang::TypeScript, 25)
        .add_module("src/api/middleware/", Lang::TypeScript, 8)
        .add_module("src/models/", Lang::TypeScript, 15)
        .add_module("src/utils/", Lang::TypeScript, 10)
        // Tests
        .add_module("tests/", Lang::TypeScript, 30)
        // Non-code
        .add_non_code_files()
        // Vendor duplication: 15 files, only 3 unique (5 copies each)
        .add_exact_duplicates("vendor/legacy/group_a/", 5)
        .add_exact_duplicates("vendor/legacy/group_b/", 5)
        .add_exact_duplicates("vendor/legacy/group_c/", 5)
        // Git history
        .initial_commit(180)
        .commit_pattern(CommitPattern::Hotspot {
            hot_paths: vec![
                "src/components/".to_string(),
                "src/api/routes/".to_string(),
            ],
            total_commits: 150,
            span_days: 180,
        })
        .build()
}

/// Rust workspace (~150 files): multiple crates, deep module trees, steady development.
pub fn rust_workspace() -> RealisticRepo {
    RealisticRepoBuilder::new()
        .seed(2)
        // Core crate
        .add_module("core/src/", Lang::Rust, 25)
        .add_module("core/tests/", Lang::Rust, 10)
        .add_file_with_content("core/Cargo.toml", "[package]\nname = \"core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")
        // Server crate
        .add_module("server/src/", Lang::Rust, 20)
        .add_module("server/src/handlers/", Lang::Rust, 10)
        .add_module("server/tests/", Lang::Rust, 8)
        .add_file_with_content("server/Cargo.toml", "[package]\nname = \"server\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ncore = {{ path = \"../core\" }}\n")
        // CLI crate
        .add_module("cli/src/", Lang::Rust, 10)
        .add_file_with_content("cli/Cargo.toml", "[package]\nname = \"cli\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")
        // Macros crate
        .add_module("macros/src/", Lang::Rust, 5)
        .add_file_with_content("macros/Cargo.toml", "[package]\nname = \"macros\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nproc-macro = true\n")
        // Workspace root
        .add_file_with_content("Cargo.toml", "[workspace]\nmembers = [\"core\", \"server\", \"cli\", \"macros\"]\n")
        // Benchmarks and docs
        .add_module("benches/", Lang::Rust, 5)
        .add_non_code_files()
        // Git history
        .initial_commit(90)
        .commit_pattern(CommitPattern::Uniform {
            total_commits: 100,
            span_days: 90,
        })
        .recent_edits(
            &[
                "server/src/file_0.rs",
                "server/src/file_1.rs",
                "server/src/handlers/file_0.rs",
            ],
            1,
        )
        .build()
}

/// Polyglot monorepo (~400 files): Go + Python + TypeScript services, per-service duplication.
pub fn polyglot_monorepo() -> RealisticRepo {
    RealisticRepoBuilder::new()
        .seed(3)
        // Go API service (old)
        .add_module("services/api/", Lang::Go, 50)
        .add_file_with_content("services/api/Dockerfile", "FROM golang:1.21\nWORKDIR /app\nCOPY . .\nRUN go build -o server .\nCMD [\"./server\"]\n")
        .add_file_with_content("services/api/Makefile", "build:\n\tgo build -o server .\ntest:\n\tgo test ./...\n")
        // Python ML service (medium age)
        .add_module("services/ml/", Lang::Python, 60)
        .add_file_with_content("services/ml/Dockerfile", "FROM python:3.11\nWORKDIR /app\nCOPY requirements.txt .\nRUN pip install -r requirements.txt\nCOPY . .\nCMD [\"python\", \"main.py\"]\n")
        .add_file_with_content("services/ml/Makefile", "install:\n\tpip install -r requirements.txt\ntest:\n\tpytest\n")
        // TypeScript web service (active)
        .add_module("services/web/src/", Lang::TypeScript, 80)
        .add_file_with_content("services/web/Dockerfile", "FROM node:20\nWORKDIR /app\nCOPY package*.json .\nRUN npm install\nCOPY . .\nRUN npm run build\nCMD [\"npm\", \"start\"]\n")
        .add_file_with_content("services/web/Makefile", "install:\n\tnpm install\nbuild:\n\tnpm run build\ntest:\n\tnpm test\n")
        // Shared Go library
        .add_module("libs/shared/", Lang::Go, 30)
        // Infrastructure
        .add_file_with_content("infra/main.yaml", "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: api\nspec:\n  replicas: 3\n")
        .add_non_code_files()
        // Near-duplicate Dockerfiles/Makefiles across services
        .add_near_duplicates("services/config/", 10, 0.90)
        // Docs
        .add_module("docs/", Lang::Python, 5) // Python used for doc scripts
        // Scripts
        .add_module("scripts/", Lang::Python, 15)
        // Git history: web service active, others stale
        .initial_commit(180)
        .commit_pattern(CommitPattern::Stale {
            initial_age_days: 180,
            burst_paths: vec!["services/web/".to_string()],
            burst_age_days: 3,
        })
        .build()
}

/// Legacy codebase (~200 files): heavy duplication, mostly stale, giant files.
pub fn legacy_with_duplication() -> RealisticRepo {
    RealisticRepoBuilder::new()
        .seed(4)
        // Near-duplicate handlers (25 of 30 are 95% similar)
        .add_near_duplicates("src/handlers/", 25, 0.95)
        .add_module("src/handlers/", Lang::Rust, 5) // 5 unique
        // Old models
        .add_module("src/models/", Lang::Rust, 20)
        // Small utilities
        .add_module("src/utils/", Lang::Rust, 15)
        // Vendor: 4 groups of 5 exact duplicates
        .add_exact_duplicates("vendor/lib_a/", 5)
        .add_exact_duplicates("vendor/lib_b/", 5)
        .add_exact_duplicates("vendor/lib_c/", 5)
        .add_exact_duplicates("vendor/lib_d/", 5)
        // Core: recently touched, some large
        .add_module("src/core/", Lang::Rust, 7)
        .add_large_file("src/core/engine.rs", 7000)
        .add_large_file("src/core/parser.rs", 5000)
        .add_large_file("src/core/codegen.rs", 6000)
        // Tests (old)
        .add_module("tests/", Lang::Rust, 20)
        // Config
        .add_non_code_files()
        // Git: stale with recent core burst
        .initial_commit(365)
        .commit_pattern(CommitPattern::Stale {
            initial_age_days: 365,
            burst_paths: vec!["src/core/".to_string()],
            burst_age_days: 3,
        })
        .build()
}

/// Scale test repo (n files): for benchmarking performance targets.
///
/// Minimal git history (single commit) to keep setup fast.
/// Files are small Rust functions spread across 10 subdirectories.
pub fn scale_test(n: usize) -> RealisticRepo {
    let mut builder = RealisticRepoBuilder::new().seed(5);

    let dirs_count = 10;
    let per_dir = n / dirs_count;
    let remainder = n % dirs_count;

    for d in 0..dirs_count {
        let count = per_dir + if d < remainder { 1 } else { 0 };
        for i in 0..count {
            let idx = d * per_dir + i;
            builder = builder.add_file(
                &format!("src/mod_{d}/file_{i}.rs"),
                Lang::Rust,
                if i % 5 == 0 {
                    FileSize::Small
                } else {
                    FileSize::Tiny
                },
            );
        }
    }

    builder.initial_commit(30).build()
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --all-features --tests`
Expected: Compiles with no errors.

- [ ] **Step 3: Commit**

```bash
git add tests/fixtures/scenarios.rs
git commit -m "feat: add 5 named repo scenarios (web_fullstack, rust_workspace, polyglot, legacy, scale)"
```

---

### Task 5: Integration Tests

**Files:**
- Create: `tests/integration/realistic.rs`

- [ ] **Step 1: Implement all integration tests**

Write the complete `tests/integration/realistic.rs`:

```rust
//! Realistic scenario integration tests.
//!
//! Tests correctness of selection quality, dedup effectiveness, budget compliance,
//! diversity, output consistency, and git signal ordering across real-world-like repos.

use crate::fixtures::builder::{CommitPattern, Lang, RealisticRepoBuilder};
use crate::fixtures::content::FileSize;
use crate::fixtures::scenarios;
use ctx_optim::selection::diversity::{DiversityConfig, GroupingStrategy};
use ctx_optim::{config::Config, pack_files, types::Budget};

// ── Selection quality ────────────────────────────────────────────────────────

#[test]
fn test_focus_boosts_adjacent_files_web_fullstack() {
    let repo = scenarios::web_fullstack();
    let config = Config::default();
    let budget = Budget::standard(50_000);

    // Focus on API routes — nearby files should rank highest
    let focus = vec![repo.path().join("src/api/routes/file_0.ts")];
    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    // Count how many of the top-5 selected files are from src/api/
    let top5_api_count = result
        .selected
        .iter()
        .take(5)
        .filter(|s| {
            s.entry
                .path
                .to_string_lossy()
                .contains("src/api/")
        })
        .count();

    assert!(
        top5_api_count >= 2,
        "expected at least 2 of top-5 from src/api/ with focus, got {top5_api_count}"
    );
}

#[test]
fn test_recent_beats_old_rust_workspace() {
    let repo = scenarios::rust_workspace();
    let config = Config::default();
    let budget = Budget::standard(20_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // Recently-edited server files should have higher recency than old cli files
    let server_scores: Vec<f32> = result
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("server/src/"))
        .map(|s| s.signals.recency)
        .collect();
    let cli_scores: Vec<f32> = result
        .selected
        .iter()
        .filter(|s| s.entry.path.to_string_lossy().contains("cli/src/"))
        .map(|s| s.signals.recency)
        .collect();

    if !server_scores.is_empty() && !cli_scores.is_empty() {
        let avg_server = server_scores.iter().sum::<f32>() / server_scores.len() as f32;
        let avg_cli = cli_scores.iter().sum::<f32>() / cli_scores.len() as f32;
        assert!(
            avg_server > avg_cli,
            "recently-edited server (avg recency={avg_server:.3}) should score higher \
             than untouched cli (avg recency={avg_cli:.3})"
        );
    }
}

#[test]
fn test_recency_signal_reflects_git_history() {
    // Create a minimal repo with two files committed at different times
    let repo = RealisticRepoBuilder::new()
        .seed(100)
        .add_file("src/fresh.rs", Lang::Rust, FileSize::Small)
        .add_file("src/ancient.rs", Lang::Rust, FileSize::Small)
        .initial_commit(365) // both committed 1 year ago
        .recent_edits(&["src/fresh.rs"], 1) // fresh.rs edited yesterday
        .build();

    let config = Config::default();
    let budget = Budget::standard(128_000);
    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let fresh = result
        .selected
        .iter()
        .find(|s| s.entry.path.to_string_lossy().contains("fresh"))
        .expect("fresh.rs should be selected");
    let ancient = result
        .selected
        .iter()
        .find(|s| s.entry.path.to_string_lossy().contains("ancient"))
        .expect("ancient.rs should be selected");

    assert!(
        fresh.signals.recency > ancient.signals.recency,
        "fresh (recency={:.3}) should beat ancient (recency={:.3})",
        fresh.signals.recency,
        ancient.signals.recency,
    );
}

// ── Dedup effectiveness ──────────────────────────────────────────────────────

#[test]
fn test_exact_dedup_web_fullstack() {
    let repo = scenarios::web_fullstack();
    let config = Config::default(); // exact dedup enabled by default
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    // We added 15 vendor files (3 groups of 5 identical), so 12 should be removed
    assert!(
        result.stats.duplicates_removed >= 10,
        "expected at least 10 exact duplicates removed, got {}",
        result.stats.duplicates_removed
    );
}

#[test]
fn test_near_dedup_legacy() {
    let repo = scenarios::legacy_with_duplication();
    let mut config = Config::default();
    config.dedup.near = true;
    config.dedup.hamming_threshold = 5; // generous threshold for near-dupes
    let budget = Budget::standard(128_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.near_duplicates_removed >= 1,
        "expected at least 1 near-duplicate removed with threshold 5, got {}",
        result.stats.near_duplicates_removed
    );
}

#[test]
fn test_dedup_preserves_all_unique() {
    // Create a repo with only unique files
    let repo = RealisticRepoBuilder::new()
        .seed(200)
        .add_module("src/", Lang::Rust, 10)
        .initial_commit(30)
        .build();

    let config = Config::default();
    let budget = Budget::standard(128_000);
    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.duplicates_removed, 0,
        "no duplicates should be removed from unique-only repo"
    );
}

// ── Budget compliance ────────────────────────────────────────────────────────

#[test]
fn test_budget_respected_across_scenarios() {
    let budgets = [500, 5_000, 50_000, 128_000];

    // Test with a medium-sized scenario
    let repo = scenarios::rust_workspace();
    let config = Config::default();

    for &b in &budgets {
        let budget = Budget::standard(b);
        let result = pack_files(repo.path(), &budget, &[], &config).unwrap();
        assert!(
            result.stats.tokens_used <= budget.l3_tokens(),
            "budget {b}: tokens_used={} exceeded l3_budget={}",
            result.stats.tokens_used,
            budget.l3_tokens()
        );
    }
}

#[test]
fn test_huge_budget_selects_everything() {
    let repo = RealisticRepoBuilder::new()
        .seed(300)
        .add_module("src/", Lang::Rust, 5)
        .initial_commit(10)
        .build();

    let config = Config::default();
    let budget = Budget::standard(1_000_000);
    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.files_selected,
        result.stats.total_files_scanned - result.stats.duplicates_removed,
        "huge budget should select all non-duplicate files"
    );
}

// ── Diversity ────────────────────────────────────────────────────────────────

#[test]
fn test_diversity_spans_services_polyglot() {
    let repo = scenarios::polyglot_monorepo();
    let mut config = Config::default();
    config.selection.diversity = DiversityConfig {
        enabled: true,
        decay: 0.7,
        grouping: GroupingStrategy::Parent, // group by grandparent for monorepo
    };
    // Tight budget forces real selection pressure
    let budget = Budget::standard(10_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    let dirs: std::collections::HashSet<String> = result
        .selected
        .iter()
        .filter_map(|s| {
            // Get the top-level service directory
            let p = s.entry.path.to_string_lossy();
            let stripped = p.strip_prefix(repo.path().to_string_lossy().as_ref())?;
            let trimmed = stripped.trim_start_matches('/');
            trimmed.split('/').next().map(|s| s.to_string())
        })
        .collect();

    if result.stats.files_selected >= 3 {
        assert!(
            dirs.len() >= 2,
            "diversity should select from multiple top-level dirs, got {dirs:?}"
        );
    }
}

// ── Compression ratio ────────────────────────────────────────────────────────

#[test]
fn test_compression_ratio_meaningful_at_scale() {
    let repo = scenarios::scale_test(1000);
    let config = Config::default();
    let budget = Budget::standard(50_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.compression_ratio >= 1.5,
        "expected compression ratio >= 1.5 for 1K files with 50K budget, got {:.2}",
        result.stats.compression_ratio
    );
}

// ── Output consistency ───────────────────────────────────────────────────────

#[test]
fn test_stats_match_selected() {
    let repo = scenarios::rust_workspace();
    let config = Config::default();
    let budget = Budget::standard(30_000);

    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.files_selected,
        result.selected.len(),
        "stats.files_selected should match selected.len()"
    );
    let computed_tokens: usize = result.selected.iter().map(|e| e.entry.token_count).sum();
    assert_eq!(
        result.stats.tokens_used, computed_tokens,
        "stats.tokens_used should match sum of selected token counts"
    );
}

#[test]
fn test_scores_bounded_all_scenarios() {
    for (name, repo) in [
        ("web_fullstack", scenarios::web_fullstack()),
        ("rust_workspace", scenarios::rust_workspace()),
    ] {
        let config = Config::default();
        let budget = Budget::standard(128_000);
        let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

        for entry in &result.selected {
            assert!(
                (0.0..=1.0).contains(&entry.composite_score),
                "{name}: composite_score {:.3} out of [0,1] for {}",
                entry.composite_score,
                entry.entry.path.display()
            );
            assert!(
                (0.0..=1.0).contains(&entry.signals.recency),
                "{name}: recency {:.3} out of [0,1]",
                entry.signals.recency
            );
            assert!(
                (0.0..=1.0).contains(&entry.signals.size_score),
                "{name}: size_score {:.3} out of [0,1]",
                entry.signals.size_score
            );
        }
    }
}

// ── Edge cases ───────────────────────────────────────────────────────────────

#[test]
fn test_all_identical_files() {
    let repo = RealisticRepoBuilder::new()
        .seed(400)
        .add_exact_duplicates("src/", 50)
        .initial_commit(10)
        .build();

    let config = Config::default();
    let budget = Budget::standard(128_000);
    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert_eq!(
        result.stats.duplicates_removed, 49,
        "50 identical files should yield 49 duplicates removed"
    );
    assert_eq!(result.stats.files_selected, 1);
}

#[test]
fn test_deeply_nested_focus() {
    let repo = RealisticRepoBuilder::new()
        .seed(500)
        .add_deep_nesting("src/deep/", 2, 8) // 8 levels deep
        .add_module("src/shallow/", Lang::Rust, 5)
        .initial_commit(30)
        .build();

    let config = Config::default();
    let budget = Budget::standard(128_000);
    // Focus on a deeply nested file
    let focus = vec![repo.path().join("src/deep/level_6/file_0.rs")];
    let result = pack_files(repo.path(), &budget, &focus, &config).unwrap();

    // Should succeed without panic
    assert!(!result.selected.is_empty());
    assert!(result.stats.tokens_used <= budget.l3_tokens());
}

#[test]
fn test_non_code_files_only() {
    let repo = RealisticRepoBuilder::new()
        .seed(600)
        .add_non_code_files()
        .initial_commit(10)
        .build();

    let config = Config::default();
    let budget = Budget::standard(128_000);
    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

    assert!(
        result.stats.files_selected > 0,
        "non-code files should still be packed"
    );
}
```

- [ ] **Step 2: Verify all integration tests pass**

Run: `cargo nextest run --all-features -E 'test(realistic::)'`
Expected: All tests pass. If any fail, debug by checking:
- Are generated files actually written to disk?
- Does git history produce expected age_days?
- Are dedup counts matching expectations?

- [ ] **Step 3: Commit**

```bash
git add tests/integration/realistic.rs
git commit -m "feat: add realistic scenario integration tests (selection, dedup, budget, diversity, edge cases)"
```

---

### Task 6: Criterion Benchmarks

**Files:**
- Create: `benches/realistic.rs`

- [ ] **Step 1: Implement benchmarks**

Write the complete `benches/realistic.rs`:

```rust
//! Realistic scenario benchmarks verifying CLAUDE.md performance targets:
//! - Index 10K files in < 500ms
//! - Score + pack in < 50ms
//! - MCP tool response in < 100ms total

// The fixtures module is not available in bench crates (they compile separately).
// We inline the scenario builders here using a path include.
#[path = "../tests/fixtures/mod.rs"]
mod fixtures;

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use ctx_optim::{
    config::Config,
    index::discovery::{DiscoveryOptions, discover_files},
    pack_files,
    scoring::score_entries,
    selection::knapsack::select_items,
    types::Budget,
};

/// Benchmark: full pipeline on the scale_test(10_000) scenario.
/// Target: < 500ms for discovery; < 100ms total for score+pack on pre-discovered files.
fn bench_discover_10k(c: &mut Criterion) {
    let repo = fixtures::scenarios::scale_test(10_000);
    let config = Config::default();
    let opts = DiscoveryOptions::from_config(&config, repo.path());

    let mut group = c.benchmark_group("discover");
    group.sample_size(10); // expensive setup
    group.bench_function("10k_files", |b| {
        b.iter(|| {
            discover_files(black_box(&opts)).expect("discover should not fail")
        });
    });
    group.finish();
}

/// Benchmark: scoring + knapsack on ~200 pre-discovered files.
/// Target: < 50ms.
fn bench_score_pack_200(c: &mut Criterion) {
    let repo = fixtures::scenarios::web_fullstack();
    let config = Config::default();
    let opts = DiscoveryOptions::from_config(&config, repo.path());
    let files = discover_files(&opts).expect("discover");
    let budget = Budget::standard(50_000);

    let mut group = c.benchmark_group("score_pack");
    group.bench_function("200_files", |b| {
        b.iter(|| {
            let scored = score_entries(
                black_box(&files),
                black_box(&config.weights),
                black_box(&[]),
                None,
            );
            select_items(
                scored,
                black_box(budget.l3_tokens()),
                black_box("auto"),
                None,
            )
        });
    });
    group.finish();
}

/// Benchmark: full pipeline on polyglot_monorepo (~400 files).
/// Target: < 100ms.
fn bench_full_pipeline_medium(c: &mut Criterion) {
    let repo = fixtures::scenarios::polyglot_monorepo();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let mut group = c.benchmark_group("full_pipeline");
    group.sample_size(10);
    group.bench_function("400_files_polyglot", |b| {
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

/// Benchmark: full pipeline on scale_test(5000).
fn bench_full_pipeline_large(c: &mut Criterion) {
    let repo = fixtures::scenarios::scale_test(5_000);
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let mut group = c.benchmark_group("full_pipeline");
    group.sample_size(10);
    group.bench_function("5k_files_scale", |b| {
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

/// Benchmark: dedup overhead on legacy_with_duplication.
fn bench_dedup_heavy(c: &mut Criterion) {
    let repo = fixtures::scenarios::legacy_with_duplication();

    let mut group = c.benchmark_group("dedup");
    group.sample_size(10);

    // Exact dedup only
    let config_exact = Config::default();
    let budget = Budget::standard(128_000);
    group.bench_function("exact_only", |b| {
        b.iter(|| {
            pack_files(
                black_box(repo.path()),
                black_box(&budget),
                black_box(&[]),
                black_box(&config_exact),
            )
            .expect("pack should not fail")
        });
    });

    // Exact + near dedup
    let mut config_near = Config::default();
    config_near.dedup.near = true;
    config_near.dedup.hamming_threshold = 5;
    group.bench_function("exact_plus_near", |b| {
        b.iter(|| {
            pack_files(
                black_box(repo.path()),
                black_box(&budget),
                black_box(&[]),
                black_box(&config_near),
            )
            .expect("pack should not fail")
        });
    });

    group.finish();
}

/// Benchmark: focus-path scoring vs no focus.
fn bench_focus_vs_no_focus(c: &mut Criterion) {
    let repo = fixtures::scenarios::rust_workspace();
    let config = Config::default();
    let budget = Budget::standard(50_000);

    let mut group = c.benchmark_group("focus");
    group.sample_size(10);

    group.bench_function("no_focus", |b| {
        b.iter(|| {
            pack_files(
                black_box(repo.path()),
                black_box(&budget),
                black_box(&[]),
                black_box(&config),
            )
            .expect("pack")
        });
    });

    let focus = vec![repo.path().join("server/src/file_0.rs")];
    group.bench_function("with_focus", |b| {
        b.iter(|| {
            pack_files(
                black_box(repo.path()),
                black_box(&budget),
                black_box(&focus),
                black_box(&config),
            )
            .expect("pack")
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_discover_10k,
    bench_score_pack_200,
    bench_full_pipeline_medium,
    bench_full_pipeline_large,
    bench_dedup_heavy,
    bench_focus_vs_no_focus,
);
criterion_main!(benches);
```

- [ ] **Step 2: Verify benchmark compiles**

Run: `cargo bench --bench realistic -- --test`
Expected: Compiles and runs a single quick iteration per benchmark. (The `--test` flag runs each benchmark once to verify correctness without full measurement.)

- [ ] **Step 3: Commit**

```bash
git add benches/realistic.rs
git commit -m "feat: add criterion benchmarks for realistic scenarios (10K discover, score+pack, dedup)"
```

---

### Task 7: Full Verification

- [ ] **Step 1: Run all existing tests (ensure no regressions)**

Run: `cargo nextest run --all-features`
Expected: All tests pass, including existing pack_pipeline tests and new realistic tests.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No warnings.

- [ ] **Step 3: Run fmt check**

Run: `cargo fmt --check`
Expected: No formatting issues. If any, run `cargo fmt` and recommit.

- [ ] **Step 4: Run benchmarks (quick sanity check)**

Run: `cargo bench --bench realistic -- --test`
Expected: All 6 benchmarks complete without error.

- [ ] **Step 5: Final commit (if any fixes were needed)**

Only if Steps 1-4 required changes:
```bash
git add -A
git commit -m "fix: address clippy/fmt issues in realistic test data"
```
