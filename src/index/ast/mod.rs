//! AST analysis via tree-sitter: signature extraction and import detection.
//!
//! Supports Rust, TypeScript, JavaScript (via TS grammar), Python, and Go.
//! Unsupported languages gracefully return `None`.

mod go;
mod python;
mod queries;
mod rust;
mod typescript;

use crate::types::{AstData, Language};
use tree_sitter::Parser;
use tree_sitter_language::LanguageFn;

/// Compiled query strings for a language.
pub(crate) struct LanguageQueries {
    /// S-expression query for extracting signatures.
    pub signatures: &'static str,
    /// S-expression query for extracting imports.
    pub imports: &'static str,
}

/// Parse a source file and extract signatures + imports.
///
/// Returns `None` if the language is unsupported, the file exceeds
/// `max_ast_bytes`, or parsing fails.
///
/// # Examples
/// ```
/// use ctx_optim::index::ast::analyze_file;
/// use ctx_optim::types::Language;
/// let source = b"pub fn hello(name: &str) -> String { format!(\"hi {name}\") }";
/// let result = analyze_file(source, Language::Rust, 256_000);
/// assert!(result.is_some());
/// let data = result.unwrap();
/// assert!(!data.signatures.is_empty());
/// ```
pub fn analyze_file(source: &[u8], language: Language, max_ast_bytes: usize) -> Option<AstData> {
    if source.len() > max_ast_bytes {
        tracing::debug!(
            lang = ?language,
            size = source.len(),
            "skipping AST parse: file exceeds max_ast_bytes ({max_ast_bytes})"
        );
        return None;
    }

    let (lang_fn, lq) = grammar_for(language)?;
    let ts_lang = tree_sitter::Language::from(lang_fn);

    let mut parser = Parser::new();
    parser
        .set_language(&ts_lang)
        .map_err(|e| {
            tracing::warn!("failed to set tree-sitter language: {e}");
        })
        .ok()?;

    let tree = parser.parse(source, None)?;

    let signatures = queries::extract_signatures(lq.signatures, &ts_lang, &tree, source);
    let imports = queries::extract_imports(lq.imports, &ts_lang, &tree, source);

    Some(AstData {
        signatures,
        imports,
    })
}

/// Returns the tree-sitter language function and query strings for a `Language`.
///
/// Returns `None` for `Language::Other`.
fn grammar_for(lang: Language) -> Option<(LanguageFn, &'static LanguageQueries)> {
    match lang {
        Language::Rust => Some((rust::language(), &rust::QUERIES)),
        Language::TypeScript => Some((typescript::language_typescript(), &typescript::QUERIES)),
        Language::JavaScript => Some((typescript::language_typescript(), &typescript::QUERIES)),
        Language::Python => Some((python::language(), &python::QUERIES)),
        Language::Go => Some((go::language(), &go::QUERIES)),
        Language::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_rust_function() {
        let src = b"pub fn greet(name: &str) -> String { format!(\"Hello {name}\") }";
        let data = analyze_file(src, Language::Rust, 256_000).unwrap();
        assert!(!data.signatures.is_empty());
        assert!(data.signatures[0].text.contains("greet"));
    }

    #[test]
    fn test_analyze_unsupported_language_returns_none() {
        let src = b"some random content";
        assert!(analyze_file(src, Language::Other, 256_000).is_none());
    }

    #[test]
    fn test_analyze_oversized_file_returns_none() {
        let src = b"fn tiny() {}";
        assert!(analyze_file(src, Language::Rust, 5).is_none());
    }

    #[test]
    fn test_analyze_empty_file() {
        let src = b"";
        let data = analyze_file(src, Language::Rust, 256_000).unwrap();
        assert!(data.signatures.is_empty());
        assert!(data.imports.is_empty());
    }

    #[test]
    fn test_javascript_uses_typescript_grammar() {
        let src = b"function add(a, b) { return a + b; }";
        let data = analyze_file(src, Language::JavaScript, 256_000).unwrap();
        assert!(!data.signatures.is_empty());
    }
}
