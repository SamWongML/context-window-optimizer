/// All library-level errors for the context window optimizer.
///
/// Application code (main, CLI) should wrap these with `anyhow`.
/// Never use `.unwrap()` — propagate via `?`.
#[derive(Debug, thiserror::Error)]
pub enum OptimError {
    /// File discovery / walk failed.
    #[error("file discovery failed: {0}")]
    Discovery(#[from] ignore::Error),

    /// Git operation failed.
    #[error("git error: {0}")]
    Git(#[from] git2::Error),

    /// Tokenizer could not encode the given text.
    #[error("tokenizer error: {0}")]
    Tokenizer(String),

    /// Requested budget exceeds the configured maximum.
    #[error("budget exceeded: requested {requested}, max {max}")]
    BudgetExceeded { requested: usize, max: usize },

    /// No files were found under the given path.
    #[error("no files found in {path}")]
    EmptyRepo { path: String },

    /// Config file could not be read or parsed.
    #[error("config error: {0}")]
    Config(String),

    /// I/O error wrapping std::io.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}
