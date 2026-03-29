use crate::types::{PackResult, PackStats, ScoredEntry};
use std::fmt::Write as FmtWrite;

/// Rough token estimate for a single L1 skeleton line (path + metadata).
/// Used to budget how many entries we can fit in the L1 output.
const TOKENS_PER_L1_LINE: usize = 12;

/// Format the L1 skeleton output: one line per file showing path and token count.
///
/// Receives ALL scored repo files (not just the knapsack-selected subset) so it
/// provides a complete map of the repository for the LLM agent.  Entries must be
/// pre-sorted by descending composite score; the function truncates at
/// `max_tokens` so the output stays within the L1 budget (typically 5% of total).
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
/// let out = format_l1(&[entry], false, 10_000);
/// assert!(out.contains("src/main.rs"));
/// assert!(out.contains("150"));
/// ```
pub fn format_l1(entries: &[ScoredEntry], include_signatures: bool, max_tokens: usize) -> String {
    let mut out = String::new();
    let mut tokens_estimated = 0usize;

    // We don't know the final count upfront (truncation), so write the header
    // last and prepend it.
    let mut body = String::new();
    let mut included = 0usize;

    for e in entries {
        // Each signature line adds roughly the same cost as a file line.
        let sig_count = if include_signatures {
            e.entry
                .ast
                .as_ref()
                .map(|a| a.signatures.len())
                .unwrap_or(0)
        } else {
            0
        };
        let line_tokens = TOKENS_PER_L1_LINE * (1 + sig_count);

        if tokens_estimated + line_tokens > max_tokens && included > 0 {
            break;
        }

        writeln!(
            body,
            "{path}  [{tokens} tokens, score={score:.3}]",
            path = e.entry.path.display(),
            tokens = e.entry.token_count,
            score = e.composite_score,
        )
        .unwrap();
        if include_signatures {
            if let Some(ast) = &e.entry.ast {
                for sig in &ast.signatures {
                    writeln!(body, "  {kind:?}: {text}", kind = sig.kind, text = sig.text).unwrap();
                }
            }
        }

        tokens_estimated += line_tokens;
        included += 1;
    }

    let total = entries.len();
    let truncated = total > included;
    if truncated {
        writeln!(
            out,
            "<!-- L1: File Skeleton Map ({included}/{total} files, truncated to {max_tokens} tokens) -->"
        )
        .unwrap();
    } else {
        writeln!(out, "<!-- L1: File Skeleton Map ({total} files) -->").unwrap();
    }
    out.push_str(&body);
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
        let out = format_l1(&entries, false, 10_000);
        assert!(out.contains("src/main.rs"), "missing path: {out}");
        assert!(out.contains("100"), "missing token count: {out}");
        assert!(out.contains("0.900"), "missing score: {out}");
    }

    #[test]
    fn test_format_l1_empty() {
        let out = format_l1(&[], false, 10_000);
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

    // ── format_l1 truncation ──────────────────────────────────────────────────

    #[test]
    fn test_format_l1_truncates_at_max_tokens() {
        // Create 20 entries.  With TOKENS_PER_L1_LINE=12, a budget of 60 tokens
        // should fit exactly 5 entries (5 × 12 = 60).
        let entries: Vec<ScoredEntry> = (0..20)
            .map(|i| make_scored(&format!("src/file{i}.rs"), 100, 0.9 - i as f32 * 0.01))
            .collect();
        let max_tokens = 5 * TOKENS_PER_L1_LINE;
        let out = format_l1(&entries, false, max_tokens);

        // Count file lines (lines that contain ".rs")
        let file_lines = out.lines().filter(|l| l.contains(".rs")).count();
        assert_eq!(
            file_lines, 5,
            "expected 5 entries at budget={max_tokens}, got {file_lines}\n{out}"
        );
        assert!(
            out.contains("truncated"),
            "truncated header expected when not all entries fit: {out}"
        );
    }

    #[test]
    fn test_format_l1_no_truncation_when_budget_large() {
        let entries: Vec<ScoredEntry> = (0..5)
            .map(|i| make_scored(&format!("src/f{i}.rs"), 50, 0.5))
            .collect();
        let out = format_l1(&entries, false, 100_000);

        let file_lines = out.lines().filter(|l| l.contains(".rs")).count();
        assert_eq!(file_lines, 5, "all 5 entries should appear: {out}");
        assert!(
            !out.contains("truncated"),
            "no truncation expected at large budget: {out}"
        );
    }

    #[test]
    fn test_format_l1_at_least_one_entry_always_included() {
        // Even with max_tokens=0 the first entry must always be emitted so the
        // caller receives something useful.
        let entries = vec![make_scored("src/only.rs", 200, 1.0)];
        let out = format_l1(&entries, false, 0);
        assert!(
            out.contains("src/only.rs"),
            "first entry must always appear: {out}"
        );
    }
}
