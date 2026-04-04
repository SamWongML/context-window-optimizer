//! Shared helpers for running tree-sitter queries and converting captures.
//!
//! Query compilation is cached per thread to avoid re-parsing S-expression
//! patterns on every file.  This gives a measurable speedup when processing
//! thousands of files with the same language.

use crate::types::{ImportRef, Signature, SymbolKind};
use std::cell::RefCell;
use std::collections::HashMap;
use tree_sitter::{Language, Query, QueryCursor, StreamingIterator, Tree};

// ---------------------------------------------------------------------------
// Thread-local query cache: keyed by (query_source pointer address).
//
// Query::new() compiles an S-expression into an automaton — expensive when
// called 10K times with the same static string.  Since our query strings are
// `&'static str`, we can use the pointer address as a cheap identity key.
// ---------------------------------------------------------------------------

/// Cache key: raw pointer to the static query string.
/// Two queries with the same source text at different addresses are treated as
/// distinct, but that never happens in practice (all our queries are `static`).
type QueryCacheKey = usize;

/// Cached compiled query plus pre-resolved capture indices.
struct CachedQuery {
    query: Query,
    /// Pre-resolved capture index for "signature" (None if absent).
    sig_idx: Option<usize>,
    /// Pre-resolved capture index for "name" (None if absent).
    name_idx: Option<usize>,
    /// Pre-resolved capture index for "import_path" (None if absent).
    import_path_idx: Option<usize>,
}

thread_local! {
    static QUERY_CACHE: RefCell<HashMap<QueryCacheKey, CachedQuery>> = RefCell::new(HashMap::new());
}

/// Get or compile a query, caching the result per-thread.
fn with_cached_query<F, R>(query_source: &str, language: &Language, f: F) -> R
where
    F: FnOnce(&CachedQuery) -> R,
{
    QUERY_CACHE.with(|cache| {
        let mut map = cache.borrow_mut();
        let key = query_source.as_ptr() as usize;
        let cached = map.entry(key).or_insert_with(|| {
            let query = match Query::new(language, query_source) {
                Ok(q) => q,
                Err(e) => {
                    tracing::warn!("failed to compile query: {e}");
                    // Return a dummy — callers will get empty results
                    return CachedQuery {
                        query: Query::new(language, "(_)").expect("trivial query"),
                        sig_idx: None,
                        name_idx: None,
                        import_path_idx: None,
                    };
                }
            };
            let sig_idx = query.capture_names().iter().position(|n| *n == "signature");
            let name_idx = query.capture_names().iter().position(|n| *n == "name");
            let import_path_idx = query
                .capture_names()
                .iter()
                .position(|n| *n == "import_path");
            CachedQuery {
                query,
                sig_idx,
                name_idx,
                import_path_idx,
            }
        });
        f(cached)
    })
}

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
    with_cached_query(query_source, language, |cached| {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&cached.query, tree.root_node(), source);
        let mut signatures = Vec::new();

        while let Some(m) = matches.next() {
            let sig_node = cached.sig_idx.and_then(|idx| {
                m.captures
                    .iter()
                    .find(|c| c.index as usize == idx)
                    .map(|c| c.node)
            });

            let name_node = cached.name_idx.and_then(|idx| {
                m.captures
                    .iter()
                    .find(|c| c.index as usize == idx)
                    .map(|c| c.node)
            });

            let node = match sig_node.or(name_node) {
                Some(n) => n,
                None => continue,
            };

            let text = extract_signature_text(source, sig_node.unwrap_or(node));
            if text.is_empty() {
                continue;
            }

            let kind = node_kind_to_symbol(node.kind());
            let line = node.start_position().row + 1;

            signatures.push(Signature { kind, text, line });
        }

        signatures
    })
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
    with_cached_query(query_source, language, |cached| {
        let path_idx = cached.import_path_idx.unwrap_or(0);

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&cached.query, tree.root_node(), source);
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
    })
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
