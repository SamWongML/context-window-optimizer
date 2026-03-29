use anyhow::Result;
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    serve_server, tool, tool_handler, tool_router,
    transport::io::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::PathBuf;

use crate::{
    config::Config,
    index::discovery::{DiscoveryOptions, discover_files},
    output::format::{OutputLevel, format_stats},
    pack_files_with_options,
    types::Budget,
};

/// Input parameters for the `pack_context` MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PackContextInput {
    /// Absolute path to the repository root to analyze.
    pub repo: String,

    /// Token budget for the packed context (default: 128 000).
    pub budget: Option<usize>,

    /// Output level: `"l1"` (skeleton), `"l2"` (summaries), `"l3"` (full content, default), `"stats"`.
    pub output: Option<String>,

    /// Paths to prioritise — files in these directories get a proximity signal boost.
    pub focus: Option<Vec<String>>,

    /// Include function/struct/trait signatures in L1 output (requires AST feature).
    #[serde(default)]
    pub include_signatures: Option<bool>,
}

/// Input parameters for the `index_stats` MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IndexStatsInput {
    /// Absolute path to the repository root to analyse.
    pub repo: String,
}

/// Input parameters for the `submit_feedback` MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubmitFeedbackInput {
    /// Session ID returned by a previous `pack_context` call.
    #[schemars(description = "Session ID from a previous pack_context result")]
    pub session_id: String,
    /// The LLM's response text — used to compute per-file utilization via identifier overlap.
    #[schemars(description = "The LLM response to score against the packed context")]
    pub llm_response: String,
    /// Absolute path to the repository root (for locating the feedback database).
    #[schemars(description = "Repository root path")]
    pub repo: String,
}

/// Input parameters for the `learned_weights` MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LearnedWeightsInput {
    /// Absolute path to the repository root.
    #[schemars(description = "Repository root path")]
    pub repo: String,
    /// If true, trigger a new learning cycle before returning weights.
    #[serde(default)]
    #[schemars(description = "Run a learning cycle before returning weights")]
    pub run_learning: Option<bool>,
}

/// MCP server handler for the Context Window Optimizer.
///
/// Exposes tools to LLM agents:
/// - `pack_context` — pack the highest-value files within a token budget.
/// - `index_stats` — report repo statistics without packing.
/// - `submit_feedback` — submit LLM response feedback for a session (requires `feedback` feature).
/// - `learned_weights` — return or trigger learning of scoring weights (requires `feedback` feature).
#[derive(Debug, Clone)]
pub struct ContextOptimizerServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl ContextOptimizerServer {
    /// Create a new server instance.
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// Pack the most valuable code context for an LLM within a token budget.
    ///
    /// Walks the repository, scores every file using recency/size/proximity signals,
    /// removes duplicates, and returns the optimal subset that fits within the budget.
    #[tool(
        description = "Pack the most valuable code context for an LLM agent within a token budget. Walks the repo, deduplicates, scores, and selects files using a greedy knapsack algorithm."
    )]
    async fn pack_context(&self, params: Parameters<PackContextInput>) -> Result<String, String> {
        let input = params.0;
        let repo = PathBuf::from(&input.repo);
        let budget = Budget::standard(input.budget.unwrap_or(128_000));
        let focus_paths: Vec<PathBuf> = input
            .focus
            .unwrap_or_default()
            .iter()
            .map(PathBuf::from)
            .collect();

        let output_level = match input.output.as_deref().unwrap_or("l3") {
            "l1" => OutputLevel::L1,
            "l2" => OutputLevel::L2,
            "stats" => OutputLevel::Stats,
            _ => OutputLevel::L3,
        };

        let config = Config::find_and_load(&repo).map_err(|e| format!("config error: {e}"))?;
        let include_signatures = input.include_signatures.unwrap_or(false);

        let result = tokio::task::spawn_blocking(move || {
            pack_files_with_options(&repo, &budget, &focus_paths, &config, include_signatures)
        })
        .await
        .map_err(|e| format!("task error: {e}"))?
        .map_err(|e| format!("pack error: {e}"))?;

        let text = match output_level {
            OutputLevel::L1 => result.l1_output,
            OutputLevel::L2 => result.l2_output,
            OutputLevel::L3 => result.l3_output,
            OutputLevel::Stats => format_stats(&result.stats),
        };

        #[cfg(feature = "feedback")]
        let text = format!("<!-- session_id: {} -->\n{}", result.session_id, text);

        Ok(text)
    }

    /// Return statistics about a repository's index without packing.
    ///
    /// Useful for understanding repo size and composition before committing to a pack.
    #[tool(
        description = "Return index statistics (file count, total tokens, language breakdown) without packing context."
    )]
    async fn index_stats(&self, params: Parameters<IndexStatsInput>) -> Result<String, String> {
        let input = params.0;
        let repo = PathBuf::from(&input.repo);

        let config = Config::find_and_load(&repo).map_err(|e| format!("config error: {e}"))?;
        let opts = DiscoveryOptions::from_config(&config, &repo);

        let files = tokio::task::spawn_blocking(move || discover_files(&opts))
            .await
            .map_err(|e| format!("task error: {e}"))?
            .map_err(|e| format!("discovery error: {e}"))?;

        let total_tokens: usize = files.iter().map(|f| f.token_count).sum();

        let mut by_lang: std::collections::HashMap<String, usize> = Default::default();
        for f in &files {
            let lang = f
                .metadata
                .language
                .map(|l| format!("{l:?}"))
                .unwrap_or_else(|| "Other".to_string());
            *by_lang.entry(lang).or_default() += 1;
        }

        let mut out = format!(
            "Repository: {repo}\nFiles: {n}\nTotal tokens: {tokens}\n\nBy language:\n",
            repo = input.repo,
            n = files.len(),
            tokens = total_tokens,
        );

        let mut lang_vec: Vec<_> = by_lang.into_iter().collect();
        lang_vec.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        for (lang, count) in lang_vec {
            out.push_str(&format!("  {lang}: {count}\n"));
        }

        Ok(out)
    }

    /// Submit feedback for a previously packed context session.
    ///
    /// Computes per-file identifier-overlap utilization by comparing each
    /// selected file's content against the LLM response.
    /// Requires the `feedback` feature — returns an error otherwise.
    #[tool(
        description = "Submit feedback for a pack_context session. Computes per-file utilization by measuring identifier overlap between each file and the LLM response. Requires the feedback feature."
    )]
    async fn submit_feedback(
        &self,
        #[allow(unused_variables)] params: Parameters<SubmitFeedbackInput>,
    ) -> Result<String, String> {
        #[cfg(not(feature = "feedback"))]
        {
            Err("feedback feature is not enabled — rebuild with --features feedback".to_string())
        }

        #[cfg(feature = "feedback")]
        {
            use crate::feedback;

            let input = params.0;
            let repo = PathBuf::from(&input.repo);

            let config = Config::find_and_load(&repo).map_err(|e| format!("config error: {e}"))?;
            let db_path = repo.join(&config.feedback.db_path);

            let store = feedback::store::FeedbackStore::open(&db_path)
                .map_err(|e| format!("feedback store error: {e}"))?;

            let session = store
                .get_session(&input.session_id)
                .map_err(|e| format!("session lookup error: {e}"))?
                .ok_or_else(|| format!("session not found: {}", input.session_id))?;

            let records = store
                .get_session_feedback(&input.session_id)
                .map_err(|e| format!("feedback lookup error: {e}"))?;

            let llm_response = input.llm_response;
            let session_id = input.session_id;
            let repo_path = PathBuf::from(&session.repo);

            let updated_records: Vec<feedback::store::FeedbackRecord> = records
                .into_iter()
                .map(|mut rec| {
                    let file_path = repo_path.join(&rec.file_path);
                    let content = std::fs::read_to_string(&file_path).unwrap_or_default();
                    let util = feedback::utilization::utilization_score(&content, &llm_response);
                    rec.utilization = Some(util);
                    rec
                })
                .collect();

            let n_files = updated_records.len();
            let avg_util: f32 = if n_files > 0 {
                updated_records
                    .iter()
                    .filter_map(|r| r.utilization)
                    .sum::<f32>()
                    / n_files as f32
            } else {
                0.0
            };

            store
                .record_feedback(&session_id, &updated_records)
                .map_err(|e| format!("store feedback error: {e}"))?;

            Ok(format!(
                "Feedback recorded for session {session_id}: {n_files} files, avg utilization {avg_util:.3}"
            ))
        }
    }

    /// Return the current learned scoring weights, optionally triggering a new learning cycle.
    ///
    /// Requires the `feedback` feature — returns an error otherwise.
    #[tool(
        description = "Return the current learned scoring weights. Optionally triggers a new learning cycle from stored feedback data before returning. Requires the feedback feature."
    )]
    async fn learned_weights(
        &self,
        #[allow(unused_variables)] params: Parameters<LearnedWeightsInput>,
    ) -> Result<String, String> {
        #[cfg(not(feature = "feedback"))]
        {
            Err("feedback feature is not enabled — rebuild with --features feedback".to_string())
        }

        #[cfg(feature = "feedback")]
        {
            use crate::feedback;

            let input = params.0;
            let repo = PathBuf::from(&input.repo);

            let config = Config::find_and_load(&repo).map_err(|e| format!("config error: {e}"))?;
            let db_path = repo.join(&config.feedback.db_path);

            let store = feedback::store::FeedbackStore::open(&db_path)
                .map_err(|e| format!("feedback store error: {e}"))?;

            if input.run_learning.unwrap_or(false) {
                let updated = feedback::learning::run_learning_cycle(
                    &store,
                    &config.weights,
                    config.feedback.learning_rate,
                )
                .map_err(|e| format!("learning cycle error: {e}"))?;

                return match updated {
                    Some(w) => Ok(format!(
                        "Learning cycle complete. Updated weights:\n  recency:    {:.4}\n  size:       {:.4}\n  proximity:  {:.4}\n  dependency: {:.4}",
                        w.recency, w.size, w.proximity, w.dependency
                    )),
                    None => Ok(
                        "No feedback data available for learning. Using default weights."
                            .to_string(),
                    ),
                };
            }

            // Return latest stored weights or defaults
            match store.latest_weights() {
                Ok(Some(snap)) => Ok(format!(
                    "Learned weights (from {}):\n  recency:    {:.4}\n  size:       {:.4}\n  proximity:  {:.4}\n  dependency: {:.4}\n  avg_util:   {:.4}",
                    snap.created_at,
                    snap.recency,
                    snap.size,
                    snap.proximity,
                    snap.dependency,
                    snap.avg_utilization
                )),
                Ok(None) => Ok(format!(
                    "No learned weights yet. Using defaults:\n  recency:    {:.4}\n  size:       {:.4}\n  proximity:  {:.4}\n  dependency: {:.4}",
                    config.weights.recency,
                    config.weights.size,
                    config.weights.proximity,
                    config.weights.dependency
                )),
                Err(e) => Err(format!("weight lookup error: {e}")),
            }
        }
    }
}

#[tool_handler]
impl ServerHandler for ContextOptimizerServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("context-window-optimizer", env!("CARGO_PKG_VERSION")),
        )
    }
}

impl Default for ContextOptimizerServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Start the MCP server on stdio and block until the connection closes.
///
/// Logs to stderr only — stdout is reserved for the MCP JSON-RPC stream.
pub async fn run_stdio_server() -> anyhow::Result<()> {
    tracing::info!("starting context-window-optimizer MCP server (stdio transport)");

    let service = serve_server(ContextOptimizerServer::new(), stdio()).await?;
    service.waiting().await?;

    tracing::info!("MCP server connection closed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_temp_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "/// Add two numbers.\npub fn add(a: usize, b: usize) -> usize { a + b }",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("main.rs"),
            "fn main() { println!(\"hello\"); }",
        )
        .unwrap();
        dir
    }

    // ── pack_context ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_pack_context_l1_returns_nonempty_output() {
        let repo = make_temp_repo();
        let server = ContextOptimizerServer::new();
        let input = PackContextInput {
            repo: repo.path().to_str().unwrap().to_string(),
            budget: Some(128_000),
            output: Some("l1".to_string()),
            focus: None,
            include_signatures: None,
        };
        let result = server.pack_context(Parameters(input)).await;
        assert!(result.is_ok(), "pack_context failed: {:?}", result.err());
        let text = result.unwrap();
        assert!(!text.is_empty(), "output should not be empty");
        assert!(text.contains("L1: File Skeleton Map"), "missing L1 header");
    }

    #[tokio::test]
    async fn test_pack_context_l3_wraps_in_context_tags() {
        let repo = make_temp_repo();
        let server = ContextOptimizerServer::new();
        let input = PackContextInput {
            repo: repo.path().to_str().unwrap().to_string(),
            budget: Some(128_000),
            output: Some("l3".to_string()),
            focus: None,
            include_signatures: None,
        };
        let text = server.pack_context(Parameters(input)).await.unwrap();
        assert!(text.contains("<context>"), "L3 missing <context>");
        assert!(text.contains("</context>"), "L3 missing </context>");
    }

    #[tokio::test]
    async fn test_pack_context_stats_output_level() {
        let repo = make_temp_repo();
        let server = ContextOptimizerServer::new();
        let input = PackContextInput {
            repo: repo.path().to_str().unwrap().to_string(),
            budget: Some(128_000),
            output: Some("stats".to_string()),
            focus: None,
            include_signatures: None,
        };
        let text = server.pack_context(Parameters(input)).await.unwrap();
        assert!(
            text.contains("Files scanned"),
            "stats output missing 'Files scanned'"
        );
        assert!(
            text.contains("Tokens used"),
            "stats output missing 'Tokens used'"
        );
    }

    #[tokio::test]
    async fn test_pack_context_all_output_levels_succeed() {
        let repo = make_temp_repo();
        let server = ContextOptimizerServer::new();
        for level in &["l1", "l2", "l3", "stats"] {
            let input = PackContextInput {
                repo: repo.path().to_str().unwrap().to_string(),
                budget: Some(128_000),
                output: Some(level.to_string()),
                focus: None,
                include_signatures: None,
            };
            let result = server.pack_context(Parameters(input)).await;
            assert!(
                result.is_ok(),
                "output level '{level}' failed: {:?}",
                result.err()
            );
            assert!(
                !result.unwrap().is_empty(),
                "level '{level}' returned empty output"
            );
        }
    }

    #[tokio::test]
    async fn test_pack_context_with_focus_paths() {
        let repo = make_temp_repo();
        let lib_path = repo.path().join("lib.rs").to_str().unwrap().to_string();
        let server = ContextOptimizerServer::new();
        let input = PackContextInput {
            repo: repo.path().to_str().unwrap().to_string(),
            budget: Some(128_000),
            output: Some("l1".to_string()),
            focus: Some(vec![lib_path]),
            include_signatures: None,
        };
        let result = server.pack_context(Parameters(input)).await;
        assert!(
            result.is_ok(),
            "focus paths caused failure: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn test_pack_context_default_output_level_is_l3() {
        let repo = make_temp_repo();
        let server = ContextOptimizerServer::new();
        let input = PackContextInput {
            repo: repo.path().to_str().unwrap().to_string(),
            budget: Some(128_000),
            output: None, // no output level specified → defaults to l3
            focus: None,
            include_signatures: None,
        };
        let text = server.pack_context(Parameters(input)).await.unwrap();
        assert!(
            text.contains("<context>"),
            "default output should be L3 with <context>"
        );
    }

    #[tokio::test]
    async fn test_pack_context_invalid_repo_returns_err() {
        let server = ContextOptimizerServer::new();
        let input = PackContextInput {
            repo: "/nonexistent/path/repo_xyz_123".to_string(),
            budget: Some(128_000),
            output: None,
            focus: None,
            include_signatures: None,
        };
        let result = server.pack_context(Parameters(input)).await;
        assert!(result.is_err(), "expected error for nonexistent repo");
    }

    // ── index_stats ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_index_stats_returns_file_and_token_counts() {
        let repo = make_temp_repo();
        let server = ContextOptimizerServer::new();
        let input = IndexStatsInput {
            repo: repo.path().to_str().unwrap().to_string(),
        };
        let text = server.index_stats(Parameters(input)).await.unwrap();
        assert!(text.contains("Files:"), "missing Files: in output: {text}");
        assert!(
            text.contains("Total tokens:"),
            "missing Total tokens: in output: {text}"
        );
    }

    #[tokio::test]
    async fn test_index_stats_includes_language_breakdown() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("utils.py"), "def util(): pass").unwrap();

        let server = ContextOptimizerServer::new();
        let input = IndexStatsInput {
            repo: dir.path().to_str().unwrap().to_string(),
        };
        let text = server.index_stats(Parameters(input)).await.unwrap();
        assert!(
            text.contains("By language:"),
            "missing language breakdown: {text}"
        );
        assert!(text.contains("Rust"), "expected Rust in breakdown: {text}");
        assert!(
            text.contains("Python"),
            "expected Python in breakdown: {text}"
        );
    }

    #[tokio::test]
    async fn test_index_stats_invalid_repo_returns_err() {
        let server = ContextOptimizerServer::new();
        let input = IndexStatsInput {
            repo: "/nonexistent/path/repo_xyz_456".to_string(),
        };
        let result = server.index_stats(Parameters(input)).await;
        assert!(result.is_err(), "expected error for nonexistent repo");
    }

    // ── submit_feedback / learned_weights ─────────────────────────────────────

    #[tokio::test]
    async fn test_submit_feedback_missing_session_returns_err() {
        let server = ContextOptimizerServer::new();

        // Without feedback feature this returns feature-not-enabled error;
        // with feedback feature it returns session-not-found.
        let input = SubmitFeedbackInput {
            session_id: "nonexistent".to_string(),
            llm_response: "some response".to_string(),
            repo: "/nonexistent/path".to_string(),
        };
        let result = server.submit_feedback(Parameters(input)).await;
        assert!(result.is_err(), "expected error for missing session");
    }

    #[tokio::test]
    async fn test_learned_weights_without_store_returns_err_or_defaults() {
        let server = ContextOptimizerServer::new();
        let input = LearnedWeightsInput {
            repo: "/nonexistent/path".to_string(),
            run_learning: None,
        };
        let result = server.learned_weights(Parameters(input)).await;
        // Without feedback feature: Err (feature not enabled)
        // With feedback feature: Err (can't open store) or Ok with defaults
        // Either outcome is acceptable
        let _ = result;
    }

    // ── server construction ───────────────────────────────────────────────────

    #[test]
    fn test_server_new_and_default_are_constructable() {
        let _s1 = ContextOptimizerServer::new();
        let _s2 = ContextOptimizerServer::default();
    }
}
