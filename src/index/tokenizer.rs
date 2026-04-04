/// Fast token count estimation using byte-class heuristics (zero allocation).
///
/// Achieves ~95% accuracy compared to full BPE encoding at roughly 100x less
/// cost.  Suitable for the discovery / scoring phase where exact counts are not
/// required — the scoring weights absorb the small estimation error.
///
/// Ratios are calibrated against `cl100k_base` on mixed code corpora.
///
/// # Examples
/// ```
/// use ctx_optim::index::tokenizer::estimate_tokens;
/// let est = estimate_tokens("fn main() { let x = 42; }");
/// assert!(est > 0);
/// ```
pub fn estimate_tokens(text: &str) -> usize {
    estimate_tokens_bytes(text.as_bytes())
}

/// Fast token count estimation on raw bytes (see [`estimate_tokens`]).
///
/// Invalid UTF-8 is handled gracefully — the byte-class classifier works on
/// raw bytes without requiring valid UTF-8.
///
/// # Examples
/// ```
/// use ctx_optim::index::tokenizer::estimate_tokens_bytes;
/// let est = estimate_tokens_bytes(b"fn main() {}");
/// assert!(est > 0);
/// ```
pub fn estimate_tokens_bytes(bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        return 0;
    }

    // Count structural features that BPE uses to form tokens:
    // 1. "Words" — maximal runs of alphanumeric/underscore/non-ASCII bytes.
    //    Each word becomes 1-3 BPE tokens depending on length.
    // 2. Punctuation characters — often individual tokens, though common pairs
    //    like `::`, `->`, `!=`, `//` merge into one.
    // 3. Newlines — usually merge with following indentation whitespace.
    let mut word_tokens = 0.0f64;
    let mut cur_word_len = 0u32;
    let mut punct = 0u32;
    let mut newlines = 0u32;

    for &b in bytes {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | 0x80..=0xFF => {
                cur_word_len += 1;
            }
            _ => {
                if cur_word_len > 0 {
                    // Per-word subword estimate, calibrated against cl100k_base:
                    //   len <= 4: ~1.0 token  (keywords: fn, let, self, true)
                    //   len  5-7: ~1.0-1.1    (short identifiers, common words)
                    //   len  8+:  ~1.1-1.5    (compound identifiers that split)
                    word_tokens += 1.0 + (cur_word_len.saturating_sub(4) as f64) * 0.04;
                    cur_word_len = 0;
                }
                match b {
                    b'\n' => newlines += 1,
                    b' ' | b'\t' | b'\r' => {}
                    _ => punct += 1,
                }
            }
        }
    }
    // Flush final word if input doesn't end with non-word byte.
    if cur_word_len > 0 {
        word_tokens += 1.0 + (cur_word_len.saturating_sub(4) as f64) * 0.04;
    }

    // Punctuation: many common operator/bracket pairs merge in BPE
    // (`::`, `->`, `!=`, `==`, `//`, `/*`, `*/`, `<=`, `>=`, `()`).
    // Empirically ~0.65 tokens per punctuation byte.
    let punct_tokens = punct as f64 * 0.65;

    // Newlines: usually merge with following indentation → low token yield.
    let newline_tokens = newlines as f64 * 0.5;

    let estimate = word_tokens + punct_tokens + newline_tokens;
    (estimate.ceil() as usize).max(1)
}

/// Returns exact BPE token count using cl100k_base encoding.
///
/// Uses `bpe-openai` (GitHub's rust-gems) — zero-allocation counting with
/// linear worst-case complexity. The cl100k_base encoding is a close
/// approximation for Claude. For exact Claude counts, use the Anthropic API.
///
/// # Examples
/// ```
/// use ctx_optim::index::tokenizer::count_tokens;
/// let n = count_tokens("hello world");
/// assert!(n >= 2);
/// ```
pub fn count_tokens(text: &str) -> usize {
    bpe_openai::cl100k_base().count(text)
}

/// Counts tokens in raw bytes, decoding as UTF-8 (lossily).
///
/// Invalid UTF-8 sequences are replaced with the replacement character.
/// This is appropriate for source files that may have encoding quirks.
///
/// # Examples
/// ```
/// use ctx_optim::index::tokenizer::count_tokens_bytes;
/// let n = count_tokens_bytes(b"let x = 42;");
/// assert!(n >= 1);
/// ```
pub fn count_tokens_bytes(bytes: &[u8]) -> usize {
    let text = String::from_utf8_lossy(bytes);
    count_tokens(&text)
}

/// A reusable tokenizer handle backed by the static cl100k_base BPE tables.
///
/// Since `bpe-openai` uses pre-serialized static tables, construction is
/// zero-cost (no BPE vocabulary decoding). The struct exists for API
/// compatibility with code that holds a tokenizer instance.
///
/// # Examples
/// ```
/// use ctx_optim::index::tokenizer::Tokenizer;
/// let tok = Tokenizer::new();
/// assert!(tok.count("fn main() {}") > 0);
/// ```
pub struct Tokenizer {
    bpe: &'static bpe_openai::Tokenizer,
}

impl Default for Tokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Tokenizer {
    /// Create a tokenizer handle (zero-cost — references static BPE tables).
    pub fn new() -> Self {
        Self {
            bpe: bpe_openai::cl100k_base(),
        }
    }

    /// Count tokens in `text` (exact BPE count, zero allocation).
    pub fn count(&self, text: &str) -> usize {
        self.bpe.count(text)
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
        let n = count_tokens("hello world");
        assert!(n >= 2, "expected at least 2 tokens, got {n}");
    }

    #[test]
    fn test_count_tokens_empty() {
        assert_eq!(count_tokens(""), 0);
    }

    #[test]
    fn test_tokenizer_reuse() {
        let tok = Tokenizer::new();
        let a = tok.count("fn main() {}");
        let b = tok.count("fn main() {}");
        assert_eq!(a, b);
    }

    #[test]
    fn test_count_bytes_utf8() {
        let text = "let x = 42;";
        let bytes = text.as_bytes();
        let tok = Tokenizer::new();
        assert_eq!(tok.count(text), tok.count_bytes(bytes));
    }

    #[test]
    fn test_estimate_empty() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens_bytes(b""), 0);
    }

    #[test]
    fn test_estimate_nonempty_returns_at_least_one() {
        assert!(estimate_tokens("a") >= 1);
        assert!(estimate_tokens("{") >= 1);
        assert!(estimate_tokens(" ") >= 1);
    }

    /// Calibration helper — prints byte-class breakdowns vs bpe-openai.
    /// Not a correctness test, just diagnostic output.
    #[test]
    fn test_estimate_calibration_diagnostics() {
        let bpe = bpe_openai::cl100k_base();
        let samples: &[(&str, &str)] = &[
            (
                "fn main() {\n    let x = 42;\n    println!(\"hello {}\", x);\n}\n",
                "tiny_rust",
            ),
            (
                "pub fn discover(opts: &Opts) -> Result<Vec<Entry>, Error> {\n    let tok = Tokenizer::new()?;\n    let mut b = WalkBuilder::new(&opts.root);\n    b.hidden(false).ignore(true);\n}\n",
                "rust_func",
            ),
            (
                "use std::collections::HashMap;\nuse std::path::{Path, PathBuf};\nuse crate::error::OptimError;\nuse rayon::prelude::*;\n",
                "imports",
            ),
            (
                "{\"key\": \"value\", \"number\": 42, \"array\": [1, 2, 3]}",
                "json",
            ),
            (
                "// Comment line about implementation details\n// Another comment\n",
                "comments",
            ),
            (
                "    let r = func(a, b, c);\n    if r.is_ok() {\n        process(r.unwrap());\n    }\n",
                "indented",
            ),
            (
                "#[derive(Debug, Clone)]\npub struct Foo {\n    pub x: usize,\n    pub y: String,\n}\n",
                "struct",
            ),
        ];
        for (text, name) in samples {
            let exact = bpe.count(text);
            let bytes = text.len();
            let alpha: u32 = text
                .bytes()
                .filter(|&b| b.is_ascii_alphanumeric() || b == b'_')
                .count() as u32;
            let nl: u32 = text.bytes().filter(|&b| b == b'\n').count() as u32;
            let ws: u32 = text
                .bytes()
                .filter(|&b| matches!(b, b' ' | b'\t' | b'\r'))
                .count() as u32;
            let punct = bytes as u32 - alpha - nl - ws;
            let ratio = bytes as f64 / exact as f64;
            let est = estimate_tokens(text);
            let err = (est as f64 - exact as f64) / exact as f64 * 100.0;
            eprintln!(
                "{name:12}: {bytes:4}b {exact:3}tok ratio={ratio:.2} a={alpha:3} nl={nl:2} ws={ws:3} p={punct:3} | est={est:3} err={err:+.0}%"
            );
        }
    }

    /// Verify that the fast estimator is within 15% of bpe-openai on realistic
    /// code samples.  The 15% threshold is generous — we typically see < 10%.
    #[test]
    fn test_estimate_accuracy_vs_bpe() {
        let tok = Tokenizer::new();

        let samples: &[(&str, &str)] = &[
            (
                "rust_func",
                concat!(
                    "pub fn discover_files(opts: &DiscoveryOptions) -> Result<Vec<FileEntry>, OptimError> {\n",
                    "    let tokenizer = Tokenizer::new()?;\n",
                    "    let mut builder = WalkBuilder::new(&opts.root);\n",
                    "    builder.hidden(false).ignore(true).git_ignore(true);\n",
                    "}\n",
                ),
            ),
            (
                "imports",
                concat!(
                    "use std::collections::HashMap;\n",
                    "use std::path::{Path, PathBuf};\n",
                    "use crate::error::OptimError;\n",
                    "use rayon::prelude::*;\n",
                ),
            ),
            (
                "json",
                r#"{"key": "value", "number": 42, "array": [1, 2, 3], "nested": {"a": true}}"#,
            ),
            (
                "comments",
                concat!(
                    "// This is a comment explaining what the code does in detail\n",
                    "// Another line of comments here about the implementation\n",
                    "// Third comment line with some technical notes about performance\n",
                ),
            ),
            (
                "mixed_indent",
                concat!(
                    "    let result = some_function(arg1, arg2, arg3);\n",
                    "    if result.is_ok() {\n",
                    "        let val = result.unwrap();\n",
                    "        process(val);\n",
                    "    } else {\n",
                    "        eprintln!(\"error\");\n",
                    "    }\n",
                ),
            ),
            (
                "struct_def",
                concat!(
                    "#[derive(Debug, Clone, serde::Serialize)]\n",
                    "pub struct FileEntry {\n",
                    "    pub path: PathBuf,\n",
                    "    pub token_count: usize,\n",
                    "    pub hash: [u8; 16],\n",
                    "    pub metadata: FileMetadata,\n",
                    "}\n",
                ),
            ),
        ];

        for (name, text) in samples {
            let exact = tok.count(text);
            let est = estimate_tokens(text);
            let err_pct = (est as f64 - exact as f64).abs() / exact as f64 * 100.0;
            eprintln!("  {name}: exact={exact}, est={est}, err={err_pct:.1}%");
            assert!(
                err_pct < 15.0,
                "{name}: estimation error {err_pct:.1}% exceeds 15% (exact={exact}, est={est})"
            );
        }
    }
}
