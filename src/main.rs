use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt};

use ctx_optim::{
    config::Config,
    output::format::{OutputLevel, format_stats},
    pack_files,
    types::Budget,
};

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

    /// Path to a custom config file (default: search upward for ctx-optim.toml).
    #[arg(short, long)]
    config: Option<PathBuf>,
}

/// Arguments for the `index` command.
#[derive(Debug, clap::Args)]
struct IndexArgs {
    /// Repository root to index.
    #[arg(short, long, default_value = ".")]
    repo: PathBuf,
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

    let result = tokio::task::spawn_blocking({
        let repo = repo.clone();
        let focus = args.focus.clone();
        move || pack_files(&repo, &budget, &focus, &config)
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
