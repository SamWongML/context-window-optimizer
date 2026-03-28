use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt};

use ctx_optim::{
    config::Config,
    output::format::{OutputLevel, format_stats},
    pack_files_with_options,
    types::Budget,
};

#[cfg(feature = "feedback")]
use ctx_optim::feedback;

/// Context Window Optimizer — intelligent context selection for LLM agents.
#[derive(Debug, Parser)]
#[command(
    name = "ctx-optim",
    version = env!("CARGO_PKG_VERSION"),
    author,
    about = "Pack the most valuable code context into your LLM's context window",
    long_about = None,
)]
struct Cli {
    /// Enable verbose logging (set RUST_LOG for fine-grained control).
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Pack the most valuable files from a repository into a token budget.
    Pack(PackArgs),

    /// Start the MCP stdio server.
    #[cfg(feature = "mcp")]
    Serve,

    /// Index a repository and report statistics without packing.
    Index(IndexArgs),

    /// Submit feedback or manage learned weights.
    #[cfg(feature = "feedback")]
    Feedback(FeedbackArgs),

    /// Start the file watcher for incremental re-indexing.
    #[cfg(feature = "watch")]
    Watch(WatchArgs),
}

/// Arguments for the `pack` command.
#[derive(Debug, clap::Args)]
struct PackArgs {
    /// Token budget for the packed context.
    #[arg(short, long, default_value = "128000")]
    budget: usize,

    /// Repository root to index.
    #[arg(short, long, default_value = ".")]
    repo: PathBuf,

    /// Output level: l1 (skeleton), l2 (summaries), l3 (full content), stats.
    #[arg(short, long, default_value = "l3")]
    output: OutputLevel,

    /// Files/directories to prioritise (proximity signal boost).
    #[arg(short, long)]
    focus: Vec<PathBuf>,

    /// Include function/struct/trait signatures in L1 output (requires AST feature).
    #[arg(short, long)]
    signatures: bool,

    /// Path to a custom config file (default: search upward for ctx-optim.toml).
    #[arg(short = 'C', long)]
    config: Option<PathBuf>,
}

/// Arguments for the `index` command.
#[derive(Debug, clap::Args)]
struct IndexArgs {
    /// Repository root to index.
    #[arg(short, long, default_value = ".")]
    repo: PathBuf,
}

/// Arguments for the `watch` command.
#[cfg(feature = "watch")]
#[derive(Debug, clap::Args)]
struct WatchArgs {
    /// Repository root to watch.
    #[arg(short, long, default_value = ".")]
    repo: PathBuf,
}

/// Arguments for the `feedback` command group.
#[cfg(feature = "feedback")]
#[derive(Debug, clap::Args)]
struct FeedbackArgs {
    #[command(subcommand)]
    action: FeedbackAction,
}

/// Feedback subcommands.
#[cfg(feature = "feedback")]
#[derive(Debug, Subcommand)]
enum FeedbackAction {
    /// Submit feedback for a pack session.
    Submit {
        /// Session ID from a previous pack_context call.
        #[arg(short, long)]
        session: String,
        /// Path to the file containing the LLM response text.
        #[arg(short = 'f', long)]
        response_file: PathBuf,
        /// Repository root path.
        #[arg(short, long, default_value = ".")]
        repo: PathBuf,
    },
    /// Show current learned weights.
    Weights {
        /// Repository root path.
        #[arg(short, long, default_value = ".")]
        repo: PathBuf,
        /// Run a learning cycle before showing weights.
        #[arg(short, long)]
        learn: bool,
    },
    /// Show feedback statistics.
    Stats {
        /// Repository root path.
        #[arg(short, long, default_value = ".")]
        repo: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialise tracing to STDERR only — never stdout (MCP stdio uses stdout)
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };
    fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .init();

    match cli.command {
        Commands::Pack(args) => cmd_pack(args).await,

        #[cfg(feature = "mcp")]
        Commands::Serve => ctx_optim::mcp::server::run_stdio_server().await,

        Commands::Index(args) => cmd_index(args).await,

        #[cfg(feature = "feedback")]
        Commands::Feedback(args) => cmd_feedback(args).await,

        #[cfg(feature = "watch")]
        Commands::Watch(args) => cmd_watch(args).await,
    }
}

async fn cmd_pack(args: PackArgs) -> Result<()> {
    let repo = args.repo.canonicalize().context("invalid --repo path")?;

    let config = if let Some(cfg_path) = args.config {
        Config::load(&cfg_path).context("loading config")?
    } else {
        Config::find_and_load(&repo).context("loading config")?
    };

    let budget = Budget::standard(args.budget);

    let include_signatures = args.signatures;
    let result = tokio::task::spawn_blocking({
        let repo = repo.clone();
        let focus = args.focus.clone();
        move || pack_files_with_options(&repo, &budget, &focus, &config, include_signatures)
    })
    .await
    .context("pack task panicked")?
    .context("pack failed")?;

    let output = match args.output {
        OutputLevel::L1 => result.l1_output.as_str(),
        OutputLevel::L2 => result.l2_output.as_str(),
        OutputLevel::L3 => result.l3_output.as_str(),
        OutputLevel::Stats => {
            // Stats needs a temporary owned string — handle separately
            let stats_text = format_stats(&result.stats);
            print!("{stats_text}");
            return Ok(());
        }
    };

    print!("{output}");
    Ok(())
}

async fn cmd_index(args: IndexArgs) -> Result<()> {
    let repo = args.repo.canonicalize().context("invalid --repo path")?;
    let config = Config::find_and_load(&repo).context("loading config")?;

    use ctx_optim::index::discovery::{DiscoveryOptions, discover_files};
    let opts = DiscoveryOptions::from_config(&config, &repo);

    let files = tokio::task::spawn_blocking(move || discover_files(&opts))
        .await
        .context("index task panicked")?
        .context("discovery failed")?;

    let total_tokens: usize = files.iter().map(|f| f.token_count).sum();

    let stats_text = format!(
        "Repository: {}\nFiles: {}\nTotal tokens: {}\n",
        repo.display(),
        files.len(),
        total_tokens
    );
    print!("{stats_text}");
    Ok(())
}

#[cfg(feature = "feedback")]
async fn cmd_feedback(args: FeedbackArgs) -> Result<()> {
    match args.action {
        FeedbackAction::Submit {
            session,
            response_file,
            repo,
        } => {
            let repo = repo.canonicalize().context("invalid --repo path")?;
            let config = Config::find_and_load(&repo).context("loading config")?;
            let db_path = repo.join(&config.feedback.db_path);

            let store =
                feedback::store::FeedbackStore::open(&db_path).context("open feedback store")?;

            let _session_data = store
                .get_session(&session)
                .context("session lookup")?
                .ok_or_else(|| anyhow::anyhow!("session not found: {session}"))?;

            let records = store
                .get_session_feedback(&session)
                .context("feedback lookup")?;

            let llm_response =
                std::fs::read_to_string(&response_file).context("reading response file")?;

            let updated_records: Vec<feedback::store::FeedbackRecord> = records
                .into_iter()
                .map(|mut rec| {
                    let file_path = repo.join(&rec.file_path);
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
                .record_feedback(&session, &updated_records)
                .context("store feedback")?;

            let msg = format!(
                "Feedback recorded for session {session}: {n_files} files, avg utilization {avg_util:.3}\n"
            );
            print!("{msg}");
        }

        FeedbackAction::Weights { repo, learn } => {
            let repo = repo.canonicalize().context("invalid --repo path")?;
            let config = Config::find_and_load(&repo).context("loading config")?;
            let db_path = repo.join(&config.feedback.db_path);

            let store =
                feedback::store::FeedbackStore::open(&db_path).context("open feedback store")?;

            if learn {
                let updated = feedback::learning::run_learning_cycle(
                    &store,
                    &config.weights,
                    config.feedback.learning_rate,
                )
                .context("learning cycle")?;

                match updated {
                    Some(w) => {
                        let msg = format!(
                            "Learning cycle complete. Updated weights:\n  recency:    {:.4}\n  size:       {:.4}\n  proximity:  {:.4}\n  dependency: {:.4}\n",
                            w.recency, w.size, w.proximity, w.dependency
                        );
                        print!("{msg}");
                    }
                    None => {
                        let msg =
                            "No feedback data available for learning. Using default weights.\n";
                        print!("{msg}");
                    }
                }
            } else {
                match store.latest_weights().context("weight lookup")? {
                    Some(snap) => {
                        let msg = format!(
                            "Learned weights (snapshot at {}):\n  recency:    {:.4}\n  size:       {:.4}\n  proximity:  {:.4}\n  dependency: {:.4}\n  avg_util:   {:.4}\n",
                            snap.created_at,
                            snap.recency,
                            snap.size,
                            snap.proximity,
                            snap.dependency,
                            snap.avg_utilization
                        );
                        print!("{msg}");
                    }
                    None => {
                        let msg = format!(
                            "No learned weights yet. Using defaults:\n  recency:    {:.4}\n  size:       {:.4}\n  proximity:  {:.4}\n  dependency: {:.4}\n",
                            config.weights.recency,
                            config.weights.size,
                            config.weights.proximity,
                            config.weights.dependency
                        );
                        print!("{msg}");
                    }
                }
            }
        }

        FeedbackAction::Stats { repo } => {
            let repo = repo.canonicalize().context("invalid --repo path")?;
            let config = Config::find_and_load(&repo).context("loading config")?;
            let db_path = repo.join(&config.feedback.db_path);

            let store =
                feedback::store::FeedbackStore::open(&db_path).context("open feedback store")?;

            let session_count = store.session_count().context("session count")?;
            let record_count = store.feedback_record_count().context("record count")?;
            let avg_util = store.avg_utilization().context("avg utilization")?;

            let util_line = match avg_util {
                Some(u) => format!("  Avg utilization:  {u:.4}\n"),
                None => "  Avg utilization:  (no data)\n".to_string(),
            };
            let msg = format!(
                "Feedback Statistics:\n  Sessions:         {session_count}\n  Feedback records: {record_count}\n{util_line}"
            );
            print!("{msg}");
        }
    }

    Ok(())
}

#[cfg(feature = "watch")]
async fn cmd_watch(args: WatchArgs) -> Result<()> {
    let repo = args.repo.canonicalize().context("invalid --repo path")?;
    tokio::task::spawn_blocking(move || ctx_optim::watch::run_watch_loop(&repo))
        .await
        .context("watch task panicked")?
        .context("watch failed")
}
