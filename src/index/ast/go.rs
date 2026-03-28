//! Go-specific tree-sitter queries for signature and import extraction.

use super::LanguageQueries;
use tree_sitter_language::LanguageFn;

pub(crate) static QUERIES: LanguageQueries = LanguageQueries {
    signatures: r#"
        (function_declaration
            name: (identifier) @name) @signature
        (method_declaration
            name: (field_identifier) @name) @signature
        (type_declaration
            (type_spec
                name: (type_identifier) @name)) @signature
    "#,
    imports: r#"
        (import_spec
            path: (interpreted_string_literal) @import_path)
    "#,
};

pub(crate) fn language() -> LanguageFn {
    tree_sitter_go::LANGUAGE
}

#[cfg(test)]
mod tests {
    use crate::index::ast::analyze_file;
    use crate::types::Language;

    #[test]
    fn test_go_function() {
        let src =
            b"package main\n\nfunc Process(input string) (string, error) {\n\treturn input, nil\n}";
        let data = analyze_file(src, Language::Go, 256_000).unwrap();
        assert!(!data.signatures.is_empty());
        assert!(data.signatures[0].text.contains("Process"));
    }

    #[test]
    fn test_go_method() {
        let src = b"package main\n\nfunc (s *Server) Start(port int) error {\n\treturn nil\n}";
        let data = analyze_file(src, Language::Go, 256_000).unwrap();
        assert!(!data.signatures.is_empty());
        assert!(data.signatures[0].text.contains("Start"));
    }

    #[test]
    fn test_go_type_declaration() {
        let src = b"package main\n\ntype Config struct {\n\tHost string\n\tPort int\n}";
        let data = analyze_file(src, Language::Go, 256_000).unwrap();
        assert!(!data.signatures.is_empty());
        assert!(data.signatures.iter().any(|s| s.text.contains("Config")));
    }

    #[test]
    fn test_go_imports() {
        let src = b"package main\n\nimport (\n\t\"fmt\"\n\t\"os/exec\"\n)";
        let data = analyze_file(src, Language::Go, 256_000).unwrap();
        assert_eq!(data.imports.len(), 2, "imports: {:?}", data.imports);
    }
}
