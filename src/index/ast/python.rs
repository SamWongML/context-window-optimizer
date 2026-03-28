//! Python-specific tree-sitter queries for signature and import extraction.

use super::LanguageQueries;
use tree_sitter_language::LanguageFn;

pub(crate) static QUERIES: LanguageQueries = LanguageQueries {
    signatures: r#"
        (function_definition
            name: (identifier) @name) @signature
        (class_definition
            name: (identifier) @name) @signature
    "#,
    imports: r#"
        (import_statement
            name: (dotted_name) @import_path)
        (import_from_statement
            module_name: (_) @import_path)
    "#,
};

pub(crate) fn language() -> LanguageFn {
    tree_sitter_python::LANGUAGE
}

#[cfg(test)]
mod tests {
    use crate::index::ast::analyze_file;
    use crate::types::Language;

    #[test]
    fn test_python_function() {
        let src = b"def process(data: list[str], limit: int = 10) -> dict:\n    return {}";
        let data = analyze_file(src, Language::Python, 256_000).unwrap();
        assert!(!data.signatures.is_empty());
        assert!(data.signatures[0].text.contains("process"));
    }

    #[test]
    fn test_python_class() {
        let src = b"class UserService:\n    def __init__(self, db):\n        self.db = db";
        let data = analyze_file(src, Language::Python, 256_000).unwrap();
        assert!(!data.signatures.is_empty());
        assert!(
            data.signatures
                .iter()
                .any(|s| s.text.contains("UserService")),
            "sigs: {:?}",
            data.signatures
        );
    }

    #[test]
    fn test_python_imports() {
        let src = b"import os\nfrom pathlib import Path\nimport sys";
        let data = analyze_file(src, Language::Python, 256_000).unwrap();
        assert!(data.imports.len() >= 2, "imports: {:?}", data.imports);
    }

    #[test]
    fn test_python_relative_import() {
        let src = b"from .utils import helper\nfrom ..config import Settings";
        let data = analyze_file(src, Language::Python, 256_000).unwrap();
        assert!(data.imports.len() >= 2, "imports: {:?}", data.imports);
    }
}
