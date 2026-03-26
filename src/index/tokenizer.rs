use crate::error::OptimError;

/// Counts tokens in a string using the cl100k_base BPE encoding.
///
/// This encoding is used by GPT-4 and is a close approximation for Claude.
/// For exact Claude token counts, use the Anthropic API's token counting endpoint.
///
/// # Examples
/// ```
/// use ctx_optim::index::tokenizer::count_tokens;
/// let n = count_tokens("hello world").unwrap();
/// assert!(n >= 2);
/// ```
pub fn count_tokens(text: &str) -> Result<usize, OptimError> {
    use tiktoken_rs::cl100k_base;
    let bpe = cl100k_base().map_err(|e| OptimError::Tokenizer(e.to_string()))?;
    Ok(bpe.encode_with_special_tokens(text).len())
}

/// Counts tokens in raw bytes, decoding as UTF-8 (lossily).
///
/// Invalid UTF-8 sequences are replaced with the replacement character.
/// This is appropriate for source files that may have encoding quirks.
pub fn count_tokens_bytes(bytes: &[u8]) -> Result<usize, OptimError> {
    let text = String::from_utf8_lossy(bytes);
    count_tokens(&text)
}

/// A cached, reusable tokenizer that avoids re-initialising the BPE table.
///
/// Create one per thread or share via `Arc` across async tasks.
///
/// # Examples
/// ```
/// use ctx_optim::index::tokenizer::Tokenizer;
/// let tok = Tokenizer::new().unwrap();
/// assert!(tok.count("fn main() {}") > 0);
/// ```
pub struct Tokenizer {
    bpe: tiktoken_rs::CoreBPE,
}

impl Tokenizer {
    /// Initialise the cl100k_base tokenizer.
    pub fn new() -> Result<Self, OptimError> {
        use tiktoken_rs::cl100k_base;
        let bpe = cl100k_base().map_err(|e| OptimError::Tokenizer(e.to_string()))?;
        Ok(Self { bpe })
    }

    /// Count tokens in `text`.
    pub fn count(&self, text: &str) -> usize {
        self.bpe.encode_with_special_tokens(text).len()
    }

    /// Count tokens in raw bytes (lossy UTF-8 decode).
    pub fn count_bytes(&self, bytes: &[u8]) -> usize {
        let text = String::from_utf8_lossy(bytes);
        self.count(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_tokens_basic() {
        let n = count_tokens("hello world").unwrap();
        assert!(n >= 2, "expected at least 2 tokens, got {n}");
    }

    #[test]
    fn test_count_tokens_empty() {
        assert_eq!(count_tokens("").unwrap(), 0);
    }

    #[test]
    fn test_tokenizer_reuse() {
        let tok = Tokenizer::new().unwrap();
        let a = tok.count("fn main() {}");
        let b = tok.count("fn main() {}");
        assert_eq!(a, b);
    }

    #[test]
    fn test_count_bytes_utf8() {
        let text = "let x = 42;";
        let bytes = text.as_bytes();
        let tok = Tokenizer::new().unwrap();
        assert_eq!(tok.count(text), tok.count_bytes(bytes));
    }
}
