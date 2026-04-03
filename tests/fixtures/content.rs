//! Code content generators for test fixtures.
//!
//! Produces syntactically-valid source files in Rust, TypeScript, Python, Go,
//! and several non-code formats.  The `index` parameter is embedded in all
//! identifiers so files are unique and will never be accidentally deduplicated.

// ── FileSize ──────────────────────────────────────────────────────────────────

/// Approximate token-budget classes for generated files.
///
/// The sizes are intentionally approximate; exact counts will vary with the
/// tokeniser.  Use these to control fixture diversity rather than to hit a
/// precise budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSize {
    /// ~20 tokens — a one-liner or tiny stub.
    Tiny,
    /// ~100 tokens — a short function with a doc-comment.
    Small,
    /// ~500 tokens — one medium block (struct + impl + tests).
    Medium,
    /// ~2 000 tokens — four medium blocks composed together.
    Large,
    /// ~6 000 tokens — twelve medium blocks composed together.
    Huge,
}

// ── Rust ──────────────────────────────────────────────────────────────────────

/// Generate a syntactically-valid Rust source file.
///
/// # Examples
/// ```
/// use crate::fixtures::content::{generate_rust, FileSize};
/// let src = generate_rust(FileSize::Small, 0);
/// assert!(src.contains("fn rust_fn_0"));
/// ```
pub fn generate_rust(size: FileSize, index: usize) -> String {
    match size {
        FileSize::Tiny => {
            format!("/// Tiny stub {index}.\npub fn rust_tiny_{index}() -> usize {{ {index} }}\n")
        }
        FileSize::Small => format!(
            r#"/// Small Rust module {index}.
use std::fmt;

/// A simple value wrapper.
pub struct RustSmall{index}(usize);

impl fmt::Display for RustSmall{index} {{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{
        write!(f, "RustSmall{index}({{}})", self.0)
    }}
}}

/// Construct a new wrapper.
pub fn rust_fn_{index}(v: usize) -> RustSmall{index} {{
    RustSmall{index}(v)
}}
"#
        ),
        FileSize::Medium => rust_medium_block(index),
        FileSize::Large => {
            let mut out = String::new();
            for i in 0..4 {
                out.push_str(&rust_medium_block(index * 10 + i));
            }
            out
        }
        FileSize::Huge => {
            let mut out = String::new();
            for i in 0..12 {
                out.push_str(&rust_medium_block(index * 20 + i));
            }
            out
        }
    }
}

/// One ~500-token Rust block: error enum + config struct + impl + 2 tests.
fn rust_medium_block(index: usize) -> String {
    format!(
        r#"use std::collections::HashMap;

/// Errors for block {index}.
#[derive(Debug, thiserror::Error)]
pub enum RustError{index} {{
    #[error("invalid config: {{0}}")]
    InvalidConfig(String),
    #[error("processing failed: {{0}}")]
    ProcessingFailed(String),
    #[error("key not found: {{0}}")]
    KeyNotFound(String),
}}

/// Configuration for block {index}.
#[derive(Debug, Clone)]
pub struct RustConfig{index} {{
    pub name: String,
    pub max_items: usize,
    pub timeout_ms: u64,
    pub tags: Vec<String>,
}}

impl RustConfig{index} {{
    /// Create a new config with defaults.
    pub fn new(name: impl Into<String>) -> Self {{
        Self {{
            name: name.into(),
            max_items: 100,
            timeout_ms: 5000,
            tags: Vec::new(),
        }}
    }}

    /// Validate this configuration.
    pub fn validate(&self) -> Result<(), RustError{index}> {{
        if self.name.is_empty() {{
            return Err(RustError{index}::InvalidConfig("name is empty".into()));
        }}
        if self.max_items == 0 {{
            return Err(RustError{index}::InvalidConfig("max_items must be > 0".into()));
        }}
        Ok(())
    }}

    /// Process a payload using this config.
    pub fn process(
        &self,
        payload: &str,
    ) -> Result<HashMap<String, String>, RustError{index}> {{
        self.validate()?;
        let mut map = HashMap::new();
        for (i, part) in payload.split(',').enumerate() {{
            if i >= self.max_items {{
                break;
            }}
            let key = format!("{{}}_{{i}}", self.name);
            map.insert(key, part.trim().to_owned());
        }}
        Ok(map)
    }}
}}

#[cfg(test)]
mod tests_{index} {{
    use super::*;

    #[test]
    fn test_config_{index}_validates_empty_name() {{
        let cfg = RustConfig{index} {{
            name: String::new(),
            max_items: 10,
            timeout_ms: 1000,
            tags: vec![],
        }};
        assert!(cfg.validate().is_err());
    }}

    #[test]
    fn test_config_{index}_process_splits_payload() {{
        let cfg = RustConfig{index}::new("blk{index}");
        let result = cfg.process("a, b, c").unwrap();
        assert_eq!(result.len(), 3);
    }}
}}
"#
    )
}

// ── TypeScript ────────────────────────────────────────────────────────────────

/// Generate a syntactically-valid TypeScript source file.
///
/// # Examples
/// ```
/// use crate::fixtures::content::{generate_typescript, FileSize};
/// let src = generate_typescript(FileSize::Small, 0);
/// assert!(src.contains("tsSmall0"));
/// ```
pub fn generate_typescript(size: FileSize, index: usize) -> String {
    match size {
        FileSize::Tiny => format!(
            "export const tsValue{index} = {index};\n\
             export function tsTiny{index}(): number {{ return {index}; }}\n"
        ),
        FileSize::Small => format!(
            r#"export interface TsSmallProps{index} {{
  id: number;
  label: string;
}}

export function tsSmall{index}(props: TsSmallProps{index}): string {{
  return `[${{props.id}}] ${{props.label}}`;
}}

export const TS_SMALL_DEFAULT_{index}: TsSmallProps{index} = {{ id: {index}, label: "item{index}" }};
"#
        ),
        FileSize::Medium => typescript_medium_block(index),
        FileSize::Large => {
            let mut out = String::new();
            for i in 0..4 {
                out.push_str(&typescript_medium_block(index * 10 + i));
            }
            out
        }
        FileSize::Huge => {
            let mut out = String::new();
            for i in 0..12 {
                out.push_str(&typescript_medium_block(index * 20 + i));
            }
            out
        }
    }
}

/// One ~500-token TypeScript block: interfaces + hooks + component + handlers.
fn typescript_medium_block(index: usize) -> String {
    format!(
        r#"import React, {{ useState, useEffect, useCallback }} from 'react';

export interface TsProps{index} {{
  initialCount: number;
  label: string;
  onSubmit: (value: number) => void;
}}

export interface TsState{index} {{
  count: number;
  loading: boolean;
  error: string | null;
}}

export const TsComponent{index}: React.FC<TsProps{index}> = ({{
  initialCount,
  label,
  onSubmit,
}}) => {{
  const [state, setState] = useState<TsState{index}>({{
    count: initialCount,
    loading: false,
    error: null,
  }});

  useEffect(() => {{
    setState((prev) => ({{ ...prev, count: initialCount }}));
  }}, [initialCount]);

  const handleIncrement = useCallback(() => {{
    setState((prev) => ({{ ...prev, count: prev.count + 1 }}));
  }}, []);

  const handleReset = useCallback(() => {{
    setState({{ count: initialCount, loading: false, error: null }});
  }}, [initialCount]);

  const handleSubmit = useCallback(() => {{
    setState((prev) => ({{ ...prev, loading: true }}));
    try {{
      onSubmit(state.count);
      setState((prev) => ({{ ...prev, loading: false }}));
    }} catch (err) {{
      setState((prev) => ({{
        ...prev,
        loading: false,
        error: err instanceof Error ? err.message : 'Unknown error',
      }}));
    }}
  }}, [onSubmit, state.count]);

  if (state.loading) return <div>Loading...</div>;
  if (state.error) return <div>Error: {{state.error}}</div>;

  return (
    <div className={{`ts-component-{index}`}}>
      <h2>{{label}}</h2>
      <p>Count: {{state.count}}</p>
      <button onClick={{handleIncrement}}>Increment</button>
      <button onClick={{handleReset}}>Reset</button>
      <button onClick={{handleSubmit}}>Submit</button>
    </div>
  );
}};

export default TsComponent{index};
"#
    )
}

// ── Python ────────────────────────────────────────────────────────────────────

/// Generate a syntactically-valid Python source file.
///
/// # Examples
/// ```
/// use crate::fixtures::content::{generate_python, FileSize};
/// let src = generate_python(FileSize::Small, 0);
/// assert!(src.contains("py_small_0"));
/// ```
pub fn generate_python(size: FileSize, index: usize) -> String {
    match size {
        FileSize::Tiny => format!(
            "PY_TINY_{index} = {index}\n\n\
             def py_tiny_{index}() -> int:\n    return {index}\n"
        ),
        FileSize::Small => format!(
            r#"from typing import Optional


PY_SMALL_CONST_{index}: int = {index}


def py_small_{index}(value: Optional[int] = None) -> str:
    """Return a small string for block {index}."""
    v = value if value is not None else {index}
    return f"py_small_{index}({{v}})"
"#
        ),
        FileSize::Medium => python_medium_block(index),
        FileSize::Large => {
            let mut out = String::new();
            for i in 0..4 {
                out.push_str(&python_medium_block(index * 10 + i));
            }
            out
        }
        FileSize::Huge => {
            let mut out = String::new();
            for i in 0..12 {
                out.push_str(&python_medium_block(index * 20 + i));
            }
            out
        }
    }
}

/// One ~500-token Python block: exception + @dataclass + Processor + factory.
fn python_medium_block(index: usize) -> String {
    format!(
        r#"from __future__ import annotations

import logging
from dataclasses import dataclass, field
from typing import Dict, List, Optional

logger = logging.getLogger(__name__)


class PyError{index}(Exception):
    """Base error for block {index}."""

    def __init__(self, message: str, code: int = 0) -> None:
        super().__init__(message)
        self.code = code

    def __repr__(self) -> str:
        return f"PyError{index}({{str(self)!r}}, code={{self.code}})"


@dataclass
class PyConfig{index}:
    """Configuration for block {index}."""

    name: str
    max_items: int = 100
    timeout_ms: int = 5000
    tags: List[str] = field(default_factory=list)
    metadata: Dict[str, str] = field(default_factory=dict)

    def validate(self) -> None:
        """Raise PyError{index} if this config is invalid."""
        if not self.name:
            raise PyError{index}("name must not be empty", code=1)
        if self.max_items <= 0:
            raise PyError{index}("max_items must be positive", code=2)
        if self.timeout_ms <= 0:
            raise PyError{index}("timeout_ms must be positive", code=3)


class PyProcessor{index}:
    """Processor for block {index}."""

    def __init__(self, config: PyConfig{index}) -> None:
        config.validate()
        self._config = config
        self._results: List[str] = []
        self._processed: int = 0

    def process(self, payload: str) -> List[str]:
        """Split payload and store up to max_items parts."""
        parts = [p.strip() for p in payload.split(",") if p.strip()]
        for part in parts[: self._config.max_items]:
            self._results.append(f"{{self._config.name}}_{{part}}")
            self._processed += 1
        logger.debug("block{index}: processed %d items", self._processed)
        return list(self._results)

    def reset(self) -> None:
        """Clear accumulated results."""
        self._results.clear()
        self._processed = 0


def make_processor_{index}(
    name: str,
    max_items: int = 50,
    tags: Optional[List[str]] = None,
) -> PyProcessor{index}:
    """Factory that creates a PyProcessor{index} with sensible defaults."""
    cfg = PyConfig{index}(
        name=name,
        max_items=max_items,
        tags=tags or [],
    )
    return PyProcessor{index}(cfg)
"#
    )
}

// ── Go ────────────────────────────────────────────────────────────────────────

/// Generate a syntactically-valid Go source file.
///
/// # Examples
/// ```
/// use crate::fixtures::content::{generate_go, FileSize};
/// let src = generate_go(FileSize::Small, 0);
/// assert!(src.contains("GoSmall0"));
/// ```
pub fn generate_go(size: FileSize, index: usize) -> String {
    match size {
        FileSize::Tiny => format!(
            "package block{index}\n\n\
             // GoTiny{index} is a tiny stub.\n\
             const GoTiny{index} = {index}\n"
        ),
        FileSize::Small => format!(
            r#"package block{index}

import "fmt"

// GoSmall{index} is a small helper type.
type GoSmall{index} struct {{
	ID    int
	Label string
}}

// String returns a human-readable representation.
func (g GoSmall{index}) String() string {{
	return fmt.Sprintf("[%d] %s", g.ID, g.Label)
}}

// NewGoSmall{index} constructs a GoSmall{index}.
func NewGoSmall{index}(id int, label string) GoSmall{index} {{
	return GoSmall{index}{{ID: id, Label: label}}
}}
"#
        ),
        FileSize::Medium => go_medium_block(index),
        FileSize::Large => {
            let mut out = String::new();
            for i in 0..4 {
                out.push_str(&go_medium_block(index * 10 + i));
            }
            out
        }
        FileSize::Huge => {
            let mut out = String::new();
            for i in 0..12 {
                out.push_str(&go_medium_block(index * 20 + i));
            }
            out
        }
    }
}

/// One ~500-token Go block: error var + Config struct + Processor + Process + Reset.
fn go_medium_block(index: usize) -> String {
    format!(
        r#"package block{index}

import (
	"errors"
	"fmt"
	"sort"
	"strings"
)

// ErrInvalid{index} is returned when a config is invalid.
var ErrInvalid{index} = errors.New("block{index}: invalid configuration")

// GoConfig{index} holds the configuration for this block.
type GoConfig{index} struct {{
	Name     string
	MaxItems int
	Timeout  int64
	Tags     []string
}}

// GoResult{index} holds a single processing result.
type GoResult{index} struct {{
	Key   string
	Value string
}}

// GoProcessor{index} processes payloads according to a GoConfig{index}.
type GoProcessor{index} struct {{
	cfg     GoConfig{index}
	results []GoResult{index}
	count   int
}}

// NewGoProcessor{index} constructs a processor and validates the config.
func NewGoProcessor{index}(cfg GoConfig{index}) (*GoProcessor{index}, error) {{
	if cfg.Name == "" {{
		return nil, fmt.Errorf("%w: name is empty", ErrInvalid{index})
	}}
	if cfg.MaxItems <= 0 {{
		return nil, fmt.Errorf("%w: max_items must be positive", ErrInvalid{index})
	}}
	return &GoProcessor{index}{{cfg: cfg}}, nil
}}

// Process splits the payload on commas and stores up to MaxItems entries.
func (p *GoProcessor{index}) Process(payload string) ([]GoResult{index}, error) {{
	parts := strings.Split(payload, ",")
	sort.Strings(parts)
	for i, part := range parts {{
		if i >= p.cfg.MaxItems {{
			break
		}}
		trimmed := strings.TrimSpace(part)
		if trimmed == "" {{
			continue
		}}
		p.results = append(p.results, GoResult{index}{{
			Key:   fmt.Sprintf("%s_%d", p.cfg.Name, p.count),
			Value: trimmed,
		}})
		p.count++
	}}
	return append([]GoResult{index}(nil), p.results...), nil
}}

// Reset clears accumulated state.
func (p *GoProcessor{index}) Reset() {{
	p.results = nil
	p.count = 0
}}
"#
    )
}

// ── Non-code ──────────────────────────────────────────────────────────────────

/// Generate non-code file content based on the file's extension.
///
/// Supported extensions: `.md`, `.toml`, `.json`, `.yaml` / `.yml`, `.css`.
/// Unknown extensions fall back to a plain-text stub.
///
/// # Examples
/// ```
/// use crate::fixtures::content::generate_non_code;
/// let md = generate_non_code("README.md", 0);
/// assert!(md.starts_with('#'));
/// ```
pub fn generate_non_code(filename: &str, index: usize) -> String {
    // Determine extension (everything after the last '.')
    let ext = filename
        .rsplit('.')
        .next()
        .map(str::to_lowercase)
        .unwrap_or_default();

    match ext.as_str() {
        "md" => generate_markdown(filename, index),
        "toml" => generate_toml(filename, index),
        "json" => generate_json(filename, index),
        "yaml" | "yml" => generate_yaml(filename, index),
        "css" => generate_css(filename, index),
        _ => format!("# {filename} (index {index})\n\nPlain-text fixture file.\n"),
    }
}

fn generate_markdown(filename: &str, index: usize) -> String {
    format!(
        r#"# Document {index}: {filename}

## Overview

This document describes module `fixture_{index}`, a generated test fixture.

## Installation

```bash
cargo add fixture_{index}
```

## Usage

```rust
use fixture_{index}::run;

fn main() {{
    run({index});
}}
```

## Configuration

| Field       | Type   | Default | Description                       |
|-------------|--------|---------|-----------------------------------|
| `max_items` | `u32`  | `100`   | Maximum items to process          |
| `timeout`   | `u64`  | `5000`  | Timeout in milliseconds           |
| `name`      | `&str` | `""`    | Human-readable identifier         |

## Changelog

### v{index}.0.0

- Initial release of fixture_{index}.
- Added `run`, `validate`, and `reset` functions.
- Full test coverage.

## License

MIT OR Apache-2.0
"#
    )
}

fn generate_toml(filename: &str, index: usize) -> String {
    format!(
        r#"# {filename} — generated fixture {index}

[package]
name = "fixture-{index}"
version = "{index}.0.0"
edition = "2024"
description = "Generated fixture package {index}"
license = "MIT OR Apache-2.0"
repository = "https://github.com/example/fixture-{index}"

[dependencies]
thiserror = "2"
anyhow = "1"
serde = {{ version = "1", features = ["derive"] }}
tokio = {{ version = "1", features = ["full"] }}
tracing = "0.1"

[dev-dependencies]
tokio-test = "0.4"
proptest = "1"

[features]
default = []
full = ["serde"]

[[bin]]
name = "fixture-{index}"
path = "src/main.rs"
"#
    )
}

fn generate_json(filename: &str, index: usize) -> String {
    format!(
        r#"{{
  "name": "fixture-{index}",
  "version": "{index}.0.0",
  "description": "Generated fixture {filename} for index {index}",
  "main": "dist/index.js",
  "types": "dist/index.d.ts",
  "scripts": {{
    "build": "tsc",
    "test": "jest --coverage",
    "lint": "eslint src --ext .ts,.tsx",
    "format": "prettier --write src"
  }},
  "dependencies": {{
    "react": "^18.0.0",
    "react-dom": "^18.0.0"
  }},
  "devDependencies": {{
    "@types/react": "^18.0.0",
    "@types/react-dom": "^18.0.0",
    "typescript": "^5.0.0",
    "jest": "^29.0.0",
    "eslint": "^8.0.0",
    "prettier": "^3.0.0"
  }},
  "keywords": ["fixture", "test", "generated", "index-{index}"],
  "license": "MIT",
  "private": false
}}
"#
    )
}

fn generate_yaml(filename: &str, index: usize) -> String {
    format!(
        r#"# {filename} — generated fixture {index}

service:
  name: fixture-service-{index}
  version: "{index}.0.0"
  replicas: 2
  image: "ghcr.io/example/fixture-{index}:latest"

environment:
  LOG_LEVEL: info
  MAX_ITEMS: "100"
  TIMEOUT_MS: "5000"
  SERVICE_NAME: fixture-{index}

ports:
  - containerPort: 8080
    protocol: TCP
  - containerPort: 9090
    name: metrics

resources:
  requests:
    cpu: "100m"
    memory: "128Mi"
  limits:
    cpu: "500m"
    memory: "512Mi"

healthCheck:
  path: /healthz
  initialDelaySeconds: 5
  periodSeconds: 10
  failureThreshold: 3

labels:
  app: fixture-{index}
  tier: backend
  managed-by: fixtures
"#
    )
}

fn generate_css(filename: &str, index: usize) -> String {
    format!(
        r#"/* {filename} — generated fixture {index} */

.fixture-{index} {{
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  padding: 1rem;
  margin: 0.5rem;
  border: 1px solid #e2e8f0;
  border-radius: 0.375rem;
  background-color: #ffffff;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
}}

.fixture-{index}__header {{
  font-size: 1.25rem;
  font-weight: 600;
  color: #1a202c;
  margin-bottom: 0.5rem;
}}

.fixture-{index}__body {{
  font-size: 0.875rem;
  color: #4a5568;
  line-height: 1.6;
}}

.fixture-{index}__footer {{
  display: flex;
  gap: 0.5rem;
  margin-top: 1rem;
}}

.fixture-{index}__button {{
  padding: 0.375rem 0.75rem;
  font-size: 0.875rem;
  font-weight: 500;
  border: none;
  border-radius: 0.25rem;
  cursor: pointer;
  transition: background-color 0.15s ease;
}}

.fixture-{index}__button--primary {{
  background-color: #3b82f6;
  color: #ffffff;
}}

.fixture-{index}__button--primary:hover {{
  background-color: #2563eb;
}}

.fixture-{index}__button--secondary {{
  background-color: #e5e7eb;
  color: #374151;
}}

.fixture-{index}__button--secondary:hover {{
  background-color: #d1d5db;
}}

@media (max-width: 640px) {{
  .fixture-{index} {{
    padding: 0.75rem;
  }}

  .fixture-{index}__header {{
    font-size: 1rem;
  }}
}}
"#
    )
}
