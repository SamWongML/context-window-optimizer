//! Identifier-overlap utilization scoring.
//!
//! Provides two pure functions:
//! - [`extract_identifiers`] — tokenise source code into meaningful identifiers.
//! - [`utilization_score`] — measure how many of a file's identifiers appear
//!   in an LLM response.

use std::collections::HashSet;

// ── Keyword filter ─────────────────────────────────────────────────────────────

const KEYWORDS: &[&str] = &[
    // Rust
    "fn", "let", "mut", "pub", "use", "mod", "struct", "enum", "impl", "trait",
    "where", "for", "loop", "while", "if", "else", "match", "return", "break",
    "continue", "self", "super", "crate", "true", "false", "const", "static",
    "type", "async", "await", "move", "ref", "dyn", "unsafe",
    // TypeScript/JavaScript
    "function", "var", "const", "class", "interface", "export", "import",
    "from", "extends", "implements", "new", "this", "typeof", "instanceof",
    "null", "undefined", "void", "never",
    // Python
    "def", "class", "import", "from", "None", "True", "False", "lambda",
    "with", "as", "try", "except", "finally", "raise", "yield", "pass",
    // Go
    "func", "package", "import", "type", "struct", "interface", "map",
    "chan", "go", "defer", "select", "case", "default", "range", "nil",
];

// ── Public API ─────────────────────────────────────────────────────────────────

/// Extract meaningful identifiers from a source code string.
///
/// The function splits `source` on any character that is not alphanumeric or an
/// underscore, then filters each candidate token by three rules:
///
/// 1. **Length** — token must be at least 3 characters.
/// 2. **Keywords** — common Rust, TypeScript/JavaScript, Python, and Go
///    keywords are removed.
/// 3. **Pure numerics** — tokens that consist entirely of ASCII digits are
///    removed (e.g. `"42"`, `"0"`).
///
/// # Examples
///
/// ```
/// use ctx_optim::feedback::utilization::extract_identifiers;
///
/// let ids = extract_identifiers("fn calculate_score(entry: &FileEntry) -> f32 {}");
/// assert!(ids.contains("calculate_score"));
/// assert!(ids.contains("FileEntry"));
/// // keywords and short tokens are excluded
/// assert!(!ids.contains("fn"));
/// assert!(!ids.contains("32"));
/// ```
pub fn extract_identifiers(source: &str) -> HashSet<String> {
    source
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|tok| {
            tok.len() >= 3
                && !KEYWORDS.contains(tok)
                && !tok.chars().all(|c| c.is_ascii_digit())
        })
        .map(|tok| tok.to_string())
        .collect()
}

/// Compute the utilization score of `file_content` with respect to `llm_response`.
///
/// The score is defined as:
///
/// ```text
/// |identifiers(file_content) ∩ tokens(llm_response)| / |identifiers(file_content)|
/// ```
///
/// where `tokens(llm_response)` is every whitespace/punctuation-separated token
/// in the response that is at least 3 characters long.
///
/// Returns `0.0` when:
/// - `file_content` yields no identifiers after filtering, **or**
/// - `llm_response` is empty.
///
/// The result is always clamped to `[0.0, 1.0]`.
///
/// # Examples
///
/// ```
/// use ctx_optim::feedback::utilization::utilization_score;
///
/// let score = utilization_score(
///     "pub fn compute_score(entry: &ScoredEntry) -> f32 { entry.composite_score }",
///     "I called compute_score on the ScoredEntry to get composite_score.",
/// );
/// assert!(score > 0.5);
///
/// assert_eq!(utilization_score("", "some response"), 0.0);
/// assert_eq!(utilization_score("fn main() {}", ""), 0.0);
/// ```
pub fn utilization_score(file_content: &str, llm_response: &str) -> f32 {
    let file_ids = extract_identifiers(file_content);
    if file_ids.is_empty() {
        return 0.0;
    }

    // Collect tokens from the LLM response (length >= 3, no keyword filter needed).
    let response_tokens: HashSet<&str> = llm_response
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|tok| tok.len() >= 3)
        .collect();

    if response_tokens.is_empty() {
        return 0.0;
    }

    let overlap = file_ids
        .iter()
        .filter(|id| response_tokens.contains(id.as_str()))
        .count();

    let score = overlap as f32 / file_ids.len() as f32;
    score.clamp(0.0, 1.0)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_identifiers_basic() {
        let code =
            "fn calculate_score(entry: &FileEntry) -> f32 { entry.token_count as f32 }";
        let ids = extract_identifiers(code);
        assert!(ids.contains("calculate_score"));
        assert!(ids.contains("FileEntry"));
        assert!(ids.contains("token_count"));
    }

    #[test]
    fn test_extract_identifiers_filters_short() {
        let code = "fn a(b: u8) -> i32 { 0 }";
        let ids = extract_identifiers(code);
        assert!(!ids.contains("a"));
        assert!(!ids.contains("b"));
        assert!(!ids.contains("u8"));
    }

    #[test]
    fn test_extract_identifiers_filters_keywords() {
        let code = "fn main() { let result = if true { return 42 } else { 0 }; }";
        let ids = extract_identifiers(code);
        assert!(!ids.contains("fn"));
        assert!(!ids.contains("let"));
        assert!(!ids.contains("if"));
        assert!(!ids.contains("true"));
        assert!(!ids.contains("return"));
        assert!(!ids.contains("else"));
        assert!(ids.contains("main"));
        assert!(ids.contains("result"));
    }

    #[test]
    fn test_utilization_score_full_overlap() {
        let file_content =
            "pub fn compute_score(entry: &ScoredEntry) -> f32 { entry.composite_score }";
        let response =
            "I used compute_score to get the ScoredEntry's composite_score from the entry.";
        let score = utilization_score(file_content, response);
        assert!(score > 0.5, "expected high utilization, got {score}");
    }

    #[test]
    fn test_utilization_score_no_overlap() {
        let file_content =
            "pub fn discover_files(opts: &DiscoveryOptions) -> Vec<FileEntry> { vec![] }";
        let response = "The weather today is sunny and warm.";
        let score = utilization_score(file_content, response);
        assert!(score < 0.1, "expected low utilization, got {score}");
    }

    #[test]
    fn test_utilization_score_empty_content() {
        assert_eq!(utilization_score("", "some response"), 0.0);
    }

    #[test]
    fn test_utilization_score_empty_response() {
        assert_eq!(utilization_score("fn main() {}", ""), 0.0);
    }

    #[test]
    fn test_utilization_score_bounded() {
        let content = "pub fn score_entry(entry: &FileEntry, weights: &ScoringWeights) -> ScoredEntry { todo!() }";
        let response = "The score_entry function takes a FileEntry and ScoringWeights to produce a ScoredEntry";
        let score = utilization_score(content, response);
        assert!(
            (0.0..=1.0).contains(&score),
            "score out of range: {score}"
        );
    }
}
