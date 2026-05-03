//! Config resolution helpers and bytes classification for Python e2e tests.

use std::collections::{HashMap, HashSet};

use crate::config::E2eConfig;
use crate::fixture::Fixture;

// ---------------------------------------------------------------------------
// Config resolution
// ---------------------------------------------------------------------------

pub(super) fn resolve_function_name(e2e_config: &E2eConfig) -> String {
    resolve_function_name_for_call(&e2e_config.call)
}

pub(super) fn resolve_function_name_for_call(call_config: &crate::config::CallConfig) -> String {
    call_config
        .overrides
        .get("python")
        .and_then(|o| o.function.clone())
        .unwrap_or_else(|| call_config.function.clone())
}

pub(super) fn resolve_module(e2e_config: &E2eConfig) -> String {
    e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.module.clone())
        .unwrap_or_else(|| e2e_config.call.module.replace('-', "_"))
}

pub(super) fn resolve_options_type(e2e_config: &E2eConfig) -> Option<String> {
    e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.options_type.clone())
}

/// Resolve the client factory function name from the Python override config.
///
/// When set, the generated test creates a client instance via `factory("test-key", base_url)`
/// and dispatches API calls as methods on the client rather than top-level functions.
pub(super) fn resolve_client_factory(e2e_config: &E2eConfig) -> Option<String> {
    e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.client_factory.clone())
}

/// Resolve how json_object args are passed: "kwargs" (default), "dict", or "json".
pub(super) fn resolve_options_via(e2e_config: &E2eConfig) -> &str {
    e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.options_via.as_deref())
        .unwrap_or("kwargs")
}

/// Resolve enum field mappings from the Python override config.
pub(super) fn resolve_enum_fields(e2e_config: &E2eConfig) -> &HashMap<String, String> {
    static EMPTY: std::sync::LazyLock<HashMap<String, String>> = std::sync::LazyLock::new(HashMap::new);
    e2e_config
        .call
        .overrides
        .get("python")
        .map(|o| &o.enum_fields)
        .unwrap_or(&EMPTY)
}

/// Resolve handle nested type mappings from the Python override config.
pub(super) fn resolve_handle_nested_types(e2e_config: &E2eConfig) -> &HashMap<String, String> {
    static EMPTY: std::sync::LazyLock<HashMap<String, String>> = std::sync::LazyLock::new(HashMap::new);
    e2e_config
        .call
        .overrides
        .get("python")
        .map(|o| &o.handle_nested_types)
        .unwrap_or(&EMPTY)
}

/// Resolve handle dict type set from the Python override config.
pub(super) fn resolve_handle_dict_types(e2e_config: &E2eConfig) -> &HashSet<String> {
    static EMPTY: std::sync::LazyLock<HashSet<String>> = std::sync::LazyLock::new(HashSet::new);
    e2e_config
        .call
        .overrides
        .get("python")
        .map(|o| &o.handle_dict_types)
        .unwrap_or(&EMPTY)
}

pub(super) fn is_skipped(fixture: &Fixture, language: &str) -> bool {
    fixture.skip.as_ref().is_some_and(|s| s.should_skip(language))
}

// ---------------------------------------------------------------------------
// Bytes classification
// ---------------------------------------------------------------------------

/// How to represent a fixture `type = "bytes"` string value in generated Python.
pub(super) enum BytesKind {
    /// A relative file path like `"pdf/fake_memo.pdf"` — read with `Path(...).read_bytes()`.
    FilePath,
    /// Inline text content like `"<!DOCTYPE html>..."` — encode to `b"..."`.
    InlineText,
    /// A base64-encoded blob like `"/9j/4AAQ"` — decode with `base64.b64decode(...)`.
    Base64,
}

/// Classify a fixture string value that maps to a `bytes` argument.
pub(super) fn classify_bytes_value(s: &str) -> BytesKind {
    if s.starts_with('<') || s.starts_with('{') || s.starts_with('[') || s.contains(' ') {
        return BytesKind::InlineText;
    }

    let first = s.chars().next().unwrap_or('\0');
    if first.is_ascii_alphanumeric() || first == '_' {
        if let Some(slash_pos) = s.find('/') {
            if slash_pos > 0 {
                let after_slash = &s[slash_pos + 1..];
                if after_slash.contains('.') && !after_slash.is_empty() {
                    return BytesKind::FilePath;
                }
            }
        }
    }

    BytesKind::Base64
}

/// Returns the Python import name for a method_result method that uses a
/// module-level helper function (not a method on the result object).
pub(super) fn python_method_helper_import(method_name: &str) -> Option<String> {
    match method_name {
        "has_error_nodes" => Some("tree_has_error_nodes".to_string()),
        "error_count" | "tree_error_count" => Some("tree_error_count".to_string()),
        "tree_to_sexp" => Some("tree_to_sexp".to_string()),
        "contains_node_type" => Some("tree_contains_node_type".to_string()),
        "find_nodes_by_type" => Some("find_nodes_by_type".to_string()),
        "run_query" => Some("run_query".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_bytes_value_html_is_inline() {
        matches!(classify_bytes_value("<!DOCTYPE html>"), BytesKind::InlineText);
    }

    #[test]
    fn classify_bytes_value_pdf_path_is_file_path() {
        matches!(classify_bytes_value("pdf/fake_memo.pdf"), BytesKind::FilePath);
    }

    #[test]
    fn classify_bytes_value_base64_is_base64() {
        matches!(classify_bytes_value("/9j/4AAQSkZJRgABAQEASABIAAD"), BytesKind::Base64);
    }

    #[test]
    fn python_method_helper_import_recognizes_has_error_nodes() {
        assert_eq!(
            python_method_helper_import("has_error_nodes"),
            Some("tree_has_error_nodes".to_string())
        );
    }

    #[test]
    fn python_method_helper_import_returns_none_for_plain_method() {
        assert!(python_method_helper_import("root_child_count").is_none());
    }
}
