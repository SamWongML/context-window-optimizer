//! TypeScript/TSX-specific tree-sitter queries for signature and import extraction.
//!
//! JavaScript files also use this grammar (TypeScript is a superset).

use super::LanguageQueries;
use tree_sitter_language::LanguageFn;

pub(crate) static QUERIES: LanguageQueries = LanguageQueries {
    signatures: r#"
        (function_declaration
            name: (identifier) @name) @signature
        (class_declaration
            name: (type_identifier) @name) @signature
        (interface_declaration
            name: (type_identifier) @name) @signature
        (type_alias_declaration
            name: (type_identifier) @name) @signature
    "#,
    imports: r#"
        (import_statement
            source: (string) @import_path)
    "#,
};

pub(crate) fn language_typescript() -> LanguageFn {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT
}

#[cfg(test)]
mod tests {
    use crate::index::ast::analyze_file;
    use crate::types::Language;

    #[test]
    fn test_ts_function_declaration() {
        let src = b"function greet(name: string): string { return `Hello ${name}`; }";
        let data = analyze_file(src, Language::TypeScript, 256_000).unwrap();
        assert!(!data.signatures.is_empty(), "sigs: {:?}", data.signatures);
        assert!(data.signatures[0].text.contains("greet"));
    }

    #[test]
    fn test_ts_class_and_interface() {
        let src = b"interface Logger { log(msg: string): void; }\nclass ConsoleLogger implements Logger { log(msg: string) { console.log(msg); } }";
        let data = analyze_file(src, Language::TypeScript, 256_000).unwrap();
        assert!(data.signatures.len() >= 2, "sigs: {:?}", data.signatures);
    }

    #[test]
    fn test_ts_type_alias() {
        let src = b"type UserId = string;";
        let data = analyze_file(src, Language::TypeScript, 256_000).unwrap();
        assert!(!data.signatures.is_empty(), "sigs: {:?}", data.signatures);
        assert!(data.signatures[0].text.contains("UserId"));
    }

    #[test]
    fn test_ts_imports() {
        let src = b"import { foo } from './utils';\nimport * as path from 'path';";
        let data = analyze_file(src, Language::TypeScript, 256_000).unwrap();
        assert_eq!(data.imports.len(), 2, "imports: {:?}", data.imports);
    }
}
