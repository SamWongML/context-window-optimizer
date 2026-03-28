//! Rust-specific tree-sitter queries for signature and import extraction.

use super::LanguageQueries;
use tree_sitter_language::LanguageFn;

pub(crate) static QUERIES: LanguageQueries = LanguageQueries {
    signatures: r#"
        (function_item
            name: (identifier) @name) @signature
        (struct_item
            name: (type_identifier) @name) @signature
        (enum_item
            name: (type_identifier) @name) @signature
        (trait_item
            name: (type_identifier) @name) @signature
        (type_item
            name: (type_identifier) @name) @signature
        (impl_item
            type: (type_identifier) @name) @signature
    "#,
    imports: r#"
        (use_declaration
            argument: (_) @import_path)
    "#,
};

pub(crate) fn language() -> LanguageFn {
    tree_sitter_rust::LANGUAGE
}

#[cfg(test)]
mod tests {
    use crate::index::ast::analyze_file;
    use crate::types::Language;

    #[test]
    fn test_rust_function_signature() {
        let src = b"pub fn process(input: &str, count: usize) -> Result<String, Error> { todo!() }";
        let data = analyze_file(src, Language::Rust, 256_000).unwrap();
        assert_eq!(data.signatures.len(), 1);
        assert!(data.signatures[0].text.contains("process"));
        assert!(data.signatures[0].text.contains("Result"));
    }

    #[test]
    fn test_rust_struct_and_enum() {
        let src = b"pub struct Config { pub name: String }\npub enum Mode { Fast, Slow }";
        let data = analyze_file(src, Language::Rust, 256_000).unwrap();
        assert!(
            data.signatures.len() >= 2,
            "expected struct + enum, got {:?}",
            data.signatures
        );
    }

    #[test]
    fn test_rust_trait() {
        let src = b"pub trait Handler { fn handle(&self) -> bool; }";
        let data = analyze_file(src, Language::Rust, 256_000).unwrap();
        assert!(!data.signatures.is_empty());
        assert!(data.signatures.iter().any(|s| s.text.contains("Handler")));
    }

    #[test]
    fn test_rust_use_declarations() {
        let src = b"use crate::scoring::signals;\nuse std::path::Path;\nuse super::config;";
        let data = analyze_file(src, Language::Rust, 256_000).unwrap();
        assert_eq!(data.imports.len(), 3, "imports: {:?}", data.imports);
    }

    #[test]
    fn test_rust_impl_block() {
        let src = b"impl Config { pub fn new() -> Self { Self {} } }";
        let data = analyze_file(src, Language::Rust, 256_000).unwrap();
        assert!(!data.signatures.is_empty());
    }
}
