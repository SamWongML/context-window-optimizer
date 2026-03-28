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

    /// AST parsing failed for a file.
    #[error("ast parse error for {path}: {detail}")]
    AstParse { path: String, detail: String },

    /// I/O error wrapping std::io.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// SimHash computation failed.
    #[error("simhash computation failed for {path}: {detail}")]
    SimHash {
        /// The file path.
        path: String,
        /// Details of the failure.
        detail: String,
    },

    /// Selection solver error.
    #[error("selection solver error: {0}")]
    Selection(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_repo_error_message_contains_path() {
        let err = OptimError::EmptyRepo {
            path: "/some/repo/path".to_string(),
        };
        assert!(
            err.to_string().contains("/some/repo/path"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn test_budget_exceeded_message_contains_values() {
        let err = OptimError::BudgetExceeded {
            requested: 200,
            max: 100,
        };
        let msg = err.to_string();
        assert!(msg.contains("200"), "missing requested: {msg}");
        assert!(msg.contains("100"), "missing max: {msg}");
    }

    #[test]
    fn test_tokenizer_error_message_contains_detail() {
        let err = OptimError::Tokenizer("encoding failed".to_string());
        assert!(
            err.to_string().contains("encoding failed"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn test_config_error_message_contains_detail() {
        let err = OptimError::Config("bad toml syntax on line 3".to_string());
        assert!(
            err.to_string().contains("bad toml syntax on line 3"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn test_io_error_from_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err = OptimError::from(io_err);
        assert!(matches!(err, OptimError::Io(_)));
        assert!(err.to_string().contains("i/o error"));
    }

    #[test]
    fn test_all_variants_implement_display() {
        let variants: &[&dyn std::fmt::Display] = &[
            &OptimError::Tokenizer("t".to_string()),
            &OptimError::BudgetExceeded {
                requested: 1,
                max: 0,
            },
            &OptimError::EmptyRepo {
                path: "p".to_string(),
            },
            &OptimError::Config("c".to_string()),
        ];
        for v in variants {
            assert!(!v.to_string().is_empty());
        }
    }
}
