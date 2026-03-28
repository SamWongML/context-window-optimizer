use crate::types::{PackResult, PackStats, ScoredEntry};
use std::fmt::Write as FmtWrite;

/// Format the L1 skeleton output: one line per selected file showing path and token count.
///
/// This is the lightest output layer — 5% of the total token budget. It gives the
/// LLM agent a complete map of what's available without the content.
///
/// # Examples
/// ```
/// use ctx_optim::output::format::format_l1;
/// use ctx_optim::types::{ScoredEntry, FileEntry, FileMetadata, ScoreSignals};
/// use std::{path::PathBuf, time::SystemTime};
/// let entry = ScoredEntry {
///     entry: FileEntry {
///         path: PathBuf::from("src/main.rs"),
///         token_count: 150,
///         hash: [0u8; 16],
///         metadata: FileMetadata {
///             size_bytes: 600,
///             last_modified: SystemTime::now(),
///             git: None,
///             language: None,
///         },
///         ast: None,
///         simhash: None,
///     },
///     composite_score: 0.85,
///     signals: Default::default(),
/// };
/// let out = format_l1(&[entry], false);
/// assert!(out.contains("src/main.rs"));
/// assert!(out.contains("150"));
/// ```
pub fn format_l1(entries: &[ScoredEntry], include_signatures: bool) -> String {
    let mut out = String::new();
    writeln!(
        out,
        "<!-- L1: File Skeleton Map ({} files) -->",
        entries.len()
    )
    .unwrap();
    for e in entries {
        writeln!(
            out,
            "{path}  [{tokens} tokens, score={score:.3}]",
            path = e.entry.path.display(),
            tokens = e.entry.token_count,
            score = e.composite_score,
        )
        .unwrap();
        if include_signatures {
            if let Some(ast) = &e.entry.ast {
                for sig in &ast.signatures {
                    writeln!(out, "  {kind:?}: {text}", kind = sig.kind, text = sig.text).unwrap();
                }
            }
        }
    }
    out
}

/// Format the L2 summary output: path + metadata block for each selected file.
///
/// This is the medium output layer — 25% of the token budget. It includes file
/// metadata and score signals to help the agent understand why each file was selected.
pub fn format_l2(entries: &[ScoredEntry]) -> String {
    let mut out = String::new();
    writeln!(
        out,
        "<!-- L2: Dependency Cluster Expansion ({} files) -->",
        entries.len()
    )
    .unwrap();

    for e in entries {
        writeln!(out, "\n### {path}", path = e.entry.path.display()).unwrap();
        writeln!(out, "- **Tokens**: {}", e.entry.token_count).unwrap();
        writeln!(out, "- **Score**: {:.3}", e.composite_score).unwrap();
        writeln!(
            out,
            "- **Signals**: recency={:.3}, size={:.3}, proximity={:.3}",
            e.signals.recency, e.signals.size_score, e.signals.proximity,
        )
        .unwrap();
        if let Some(git) = &e.entry.metadata.git {
            writeln!(
                out,
                "- **Git**: {:.1} days ago, {} commits",
                git.age_days, git.commit_count
            )
            .unwrap();
        }
        if let Some(lang) = e.entry.metadata.language {
            writeln!(out, "- **Language**: {lang:?}").unwrap();
        }
    }
    out
}

/// Format the L3 full-content output: XML-wrapped file contents.
///
/// This is the full output layer — 70% of the token budget. Each file is
/// wrapped in an `<file>` tag with path, token count, and score attributes.
///
/// # Examples
/// ```no_run
/// use ctx_optim::output::format::format_l3;
/// // Each selected file is wrapped in <file path="..." tokens="...">...</file>
/// ```
pub fn format_l3(entries: &[ScoredEntry]) -> String {
    let mut out = String::new();
    writeln!(out, "<context>").unwrap();
    writeln!(
        out,
        "<!-- L3: Full Content Fragments ({} files) -->",
        entries.len()
    )
    .unwrap();

    for e in entries {
        let content = match std::fs::read_to_string(&e.entry.path) {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!("L3: could not read {}: {err}", e.entry.path.display());
                String::new()
            }
        };

        writeln!(
            out,
            r#"<file path="{path}" tokens="{tokens}" score="{score:.3}">"#,
            path = e.entry.path.display(),
            tokens = e.entry.token_count,
            score = e.composite_score,
        )
        .unwrap();
        out.push_str(&content);
        if !content.ends_with('\n') {
            out.push('\n');
        }
        writeln!(out, "</file>").unwrap();
    }
    writeln!(out, "</context>").unwrap();
    out
}

/// Format a human-readable stats summary.
///
/// # Examples
/// ```
/// use ctx_optim::types::PackStats;
/// use ctx_optim::output::format::format_stats;
/// let stats = PackStats {
///     total_files_scanned: 500,
///     duplicates_removed: 10,
///     near_duplicates_removed: 0,
///     files_selected: 25,
///     tokens_used: 64_000,
///     tokens_budget: 128_000,
///     compression_ratio: 0.3,
///     solver_used: String::new(),
/// };
/// let s = format_stats(&stats);
/// assert!(s.contains("64000"));
/// ```
pub fn format_stats(stats: &PackStats) -> String {
    let mut out = format!(
        "\
Context Window Optimizer — Pack Stats
──────────────────────────────────────
  Files scanned:      {total}
  Duplicates removed: {dups}",
        total = stats.total_files_scanned,
        dups = stats.duplicates_removed,
    );

    if stats.near_duplicates_removed > 0 {
        out.push_str(&format!(
            "\n  Near-dupes removed: {}",
            stats.near_duplicates_removed
        ));
    }

    out.push_str(&format!(
        "\n  Files selected:     {selected}\
         \n  Tokens used:        {used} / {budget} ({pct:.1}%)\
         \n  Compression ratio:  {ratio:.2}x",
        selected = stats.files_selected,
        used = stats.tokens_used,
        budget = stats.tokens_budget,
        pct = stats.tokens_used as f32 / stats.tokens_budget.max(1) as f32 * 100.0,
        ratio = stats.compression_ratio,
    ));

    if !stats.solver_used.is_empty() {
        out.push_str(&format!("\n  Solver:             {}", stats.solver_used));
    }

    out.push('\n');
    out
}

/// Format a `PackResult` for a given output level.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize,
)]
pub enum OutputLevel {
    /// L1: file skeleton map (paths + token counts)
    #[value(name = "l1")]
    L1,
    /// L2: file metadata summaries
    #[value(name = "l2")]
    L2,
    /// L3: full XML-wrapped file contents (default)
    #[value(name = "l3")]
    L3,
    /// Stats: human-readable pack statistics
    #[value(name = "stats")]
    Stats,
}

/// Render a `PackResult` at the requested output level.
pub fn render(result: &PackResult, level: OutputLevel) -> &str {
    match level {
        OutputLevel::L1 => &result.l1_output,
        OutputLevel::L2 => &result.l2_output,
        OutputLevel::L3 => &result.l3_output,
        OutputLevel::Stats => {
            // Stats is formatted on demand; here we return L3 as fallback
            // since stats rendering needs the full PackStats separately.
            &result.l3_output
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileEntry, FileMetadata, ScoreSignals};
    use std::{path::PathBuf, time::SystemTime};

    fn make_scored(path: &str, tokens: usize, score: f32) -> ScoredEntry {
        ScoredEntry {
            entry: FileEntry {
                path: PathBuf::from(path),
                token_count: tokens,
                hash: [0u8; 16],
                metadata: FileMetadata {
                    size_bytes: 200,
                    last_modified: SystemTime::now(),
                    git: None,
                    language: None,
                },
                ast: None,
                simhash: None,
            },
            composite_score: score,
            signals: ScoreSignals::default(),
        }
    }

    #[test]
    fn test_format_l1_contains_path_and_tokens() {
        let entries = vec![make_scored("src/main.rs", 100, 0.9)];
        let out = format_l1(&entries, false);
        assert!(out.contains("src/main.rs"), "missing path: {out}");
        assert!(out.contains("100"), "missing token count: {out}");
        assert!(out.contains("0.900"), "missing score: {out}");
    }

    #[test]
    fn test_format_l1_empty() {
        let out = format_l1(&[], false);
        assert!(out.contains("0 files"));
    }

    #[test]
    fn test_format_l2_contains_metadata() {
        let entries = vec![make_scored("src/lib.rs", 250, 0.75)];
        let out = format_l2(&entries);
        assert!(out.contains("src/lib.rs"));
        assert!(out.contains("250"));
        assert!(out.contains("0.750"));
    }

    #[test]
    fn test_format_stats_contains_numbers() {
        let stats = PackStats {
            total_files_scanned: 100,
            duplicates_removed: 5,
            near_duplicates_removed: 0,
            files_selected: 10,
            tokens_used: 32_000,
            tokens_budget: 128_000,
            compression_ratio: 0.5,
            solver_used: String::new(),
        };
        let s = format_stats(&stats);
        assert!(s.contains("100"));
        assert!(s.contains("32000"));
        assert!(s.contains("128000"));
    }

    // ── format_l3 ─────────────────────────────────────────────────────────────

    #[test]
    fn test_format_l3_empty_wraps_in_context_tags() {
        let out = format_l3(&[]);
        assert!(out.contains("<context>"), "missing opening tag: {out}");
        assert!(out.contains("</context>"), "missing closing tag: {out}");
        assert!(out.contains("0 files"));
    }

    #[test]
    fn test_format_l3_reads_existing_file_content() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("hello.rs");
        std::fs::write(&path, "fn hello() { /* hi */ }").unwrap();

        let entry = make_scored(&path.to_string_lossy(), 5, 0.9);
        let out = format_l3(&[entry]);

        assert!(out.contains("<file"), "missing <file> open tag: {out}");
        assert!(out.contains("</file>"), "missing </file> close tag: {out}");
        assert!(
            out.contains("fn hello()"),
            "missing file content in L3: {out}"
        );
    }

    #[test]
    fn test_format_l3_file_tag_attributes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("mod.rs");
        std::fs::write(&path, "// content").unwrap();

        let entry = make_scored(&path.to_string_lossy(), 42, 0.75);
        let out = format_l3(&[entry]);

        assert!(out.contains("tokens=\"42\""), "missing tokens attr: {out}");
        assert!(out.contains("score=\"0.750\""), "missing score attr: {out}");
    }

    #[test]
    fn test_format_l3_handles_missing_file_gracefully() {
        // A file that doesn't exist should not panic; content is just empty.
        let entry = make_scored("/nonexistent/path/ghost.rs", 10, 0.5);
        let out = format_l3(&[entry]);
        assert!(out.contains("<context>"));
        assert!(out.contains("</context>"));
        assert!(out.contains("<file"));
        assert!(out.contains("</file>"));
    }

    #[test]
    fn test_format_l3_multiple_files_all_wrapped() {
        let tmp = tempfile::TempDir::new().unwrap();
        for name in &["a.rs", "b.rs", "c.rs"] {
            std::fs::write(tmp.path().join(name), format!("// {name}")).unwrap();
        }
        let entries: Vec<ScoredEntry> = ["a.rs", "b.rs", "c.rs"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                make_scored(
                    tmp.path().join(name).to_string_lossy().as_ref(),
                    10 + i,
                    0.5,
                )
            })
            .collect();

        let out = format_l3(&entries);
        assert_eq!(out.matches("<file").count(), 3, "expected 3 <file> tags");
        assert_eq!(
            out.matches("</file>").count(),
            3,
            "expected 3 </file> close tags"
        );
    }

    // ── render ────────────────────────────────────────────────────────────────

    #[test]
    fn test_render_returns_correct_output_per_level() {
        use crate::types::{PackResult, PackStats};
        let result = PackResult {
            session_id: String::new(),
            selected: vec![],
            l1_output: "L1_DATA".to_string(),
            l2_output: "L2_DATA".to_string(),
            l3_output: "L3_DATA".to_string(),
            stats: PackStats::default(),
        };
        assert_eq!(render(&result, OutputLevel::L1), "L1_DATA");
        assert_eq!(render(&result, OutputLevel::L2), "L2_DATA");
        assert_eq!(render(&result, OutputLevel::L3), "L3_DATA");
        // Stats falls back to L3 (documented behaviour)
        assert_eq!(render(&result, OutputLevel::Stats), "L3_DATA");
    }

    // ── format_stats ──────────────────────────────────────────────────────────

    #[test]
    fn test_format_stats_percentage_calculation() {
        let stats = PackStats {
            total_files_scanned: 10,
            duplicates_removed: 0,
            near_duplicates_removed: 0,
            files_selected: 5,
            tokens_used: 64_000,
            tokens_budget: 128_000,
            compression_ratio: 2.0,
            solver_used: String::new(),
        };
        let s = format_stats(&stats);
        // 64000/128000 = 50.0%
        assert!(s.contains("50.0%"), "expected 50.0% in: {s}");
    }
}
