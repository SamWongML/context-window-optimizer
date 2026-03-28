//! Shared helpers for running tree-sitter queries and converting captures.

use crate::types::{ImportRef, Signature, SymbolKind};
use tree_sitter::{Language, Query, QueryCursor, StreamingIterator, Tree};

/// Execute a signature query and return extracted signatures.
///
/// The query must use `@signature` to capture the full node and `@name` for the
/// symbol name. The `kind` field maps to a `SymbolKind` based on the node type.
pub(crate) fn extract_signatures(
    query_source: &str,
    language: &Language,
    tree: &Tree,
    source: &[u8],
) -> Vec<Signature> {
    let query = match Query::new(language, query_source) {
        Ok(q) => q,
        Err(e) => {
            tracing::warn!("failed to compile signature query: {e}");
            return Vec::new();
        }
    };

    let sig_idx = query.capture_names().iter().position(|n| *n == "signature");
    let name_idx = query.capture_names().iter().position(|n| *n == "name");

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source);
    let mut signatures = Vec::new();

    while let Some(m) = matches.next() {
        // Get the signature node (outer node for text extraction)
        let sig_node = sig_idx.and_then(|idx| {
            m.captures
                .iter()
                .find(|c| c.index as usize == idx)
                .map(|c| c.node)
        });

        let name_node = name_idx.and_then(|idx| {
            m.captures
                .iter()
                .find(|c| c.index as usize == idx)
                .map(|c| c.node)
        });

        let node = match sig_node.or(name_node) {
            Some(n) => n,
            None => continue,
        };

        // Extract signature text: from node start to first '{' or end of line
        let text = extract_signature_text(source, sig_node.unwrap_or(node));
        if text.is_empty() {
            continue;
        }

        let kind = node_kind_to_symbol(node.kind());
        let line = node.start_position().row + 1; // 1-indexed

        signatures.push(Signature { kind, text, line });
    }

    signatures
}

/// Execute an import query and return extracted import references.
///
/// The query must use `@import_path` to capture the import path node.
pub(crate) fn extract_imports(
    query_source: &str,
    language: &Language,
    tree: &Tree,
    source: &[u8],
) -> Vec<ImportRef> {
    let query = match Query::new(language, query_source) {
        Ok(q) => q,
        Err(e) => {
            tracing::warn!("failed to compile import query: {e}");
            return Vec::new();
        }
    };

    let path_idx = query
        .capture_names()
        .iter()
        .position(|n| *n == "import_path")
        .unwrap_or(0);

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source);
    let mut imports = Vec::new();

    while let Some(m) = matches.next() {
        if let Some(capture) = m.captures.iter().find(|c| c.index as usize == path_idx) {
            let node = capture.node;
            let raw_path = node_text(source, node)
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            if !raw_path.is_empty() {
                imports.push(ImportRef {
                    raw_path,
                    line: node.start_position().row + 1,
                });
            }
        }
    }

    imports
}

/// Extract signature text from a node: everything up to the first `{` or end of node.
fn extract_signature_text(source: &[u8], node: tree_sitter::Node) -> String {
    let start = node.start_byte();
    let end = node.end_byte();
    let slice = &source[start..end];

    // Find the first `{` to cut off the body
    let text = String::from_utf8_lossy(slice);
    let sig = if let Some(brace) = text.find('{') {
        text[..brace].trim_end()
    } else {
        text.trim()
    };

    // Collapse multiple whitespace/newlines into single spaces
    sig.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Get the text of a tree-sitter node.
fn node_text(source: &[u8], node: tree_sitter::Node) -> String {
    String::from_utf8_lossy(&source[node.start_byte()..node.end_byte()]).to_string()
}

/// Map tree-sitter node kind strings to `SymbolKind`.
fn node_kind_to_symbol(kind: &str) -> SymbolKind {
    match kind {
        "function_item" | "function_declaration" | "function_definition" => SymbolKind::Function,
        "method_declaration" | "method_definition" => SymbolKind::Method,
        "struct_item" => SymbolKind::Struct,
        "enum_item" => SymbolKind::Enum,
        "trait_item" => SymbolKind::Trait,
        "interface_declaration" => SymbolKind::Interface,
        "class_declaration" | "class_definition" => SymbolKind::Class,
        "type_item" | "type_alias_declaration" | "type_declaration" => SymbolKind::TypeAlias,
        "impl_item" => SymbolKind::Impl,
        other if other.contains("method") => SymbolKind::Method,
        _ => SymbolKind::Function,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_kind_to_symbol_known() {
        assert_eq!(node_kind_to_symbol("function_item"), SymbolKind::Function);
        assert_eq!(node_kind_to_symbol("struct_item"), SymbolKind::Struct);
        assert_eq!(node_kind_to_symbol("trait_item"), SymbolKind::Trait);
        assert_eq!(node_kind_to_symbol("impl_item"), SymbolKind::Impl);
        assert_eq!(node_kind_to_symbol("class_declaration"), SymbolKind::Class);
    }

    #[test]
    fn test_extract_signature_text_trims_body() {
        let src = b"pub fn hello(name: &str) { body }";
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(src, None).unwrap();
        let root = tree.root_node();
        let fn_node = root.child(0).unwrap();
        let text = extract_signature_text(src, fn_node);
        assert!(text.contains("hello"));
        assert!(!text.contains("body"));
    }
}
