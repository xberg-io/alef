//! Go e2e method-call assertion helpers.

use crate::e2e::escape::go_string_literal;
use heck::ToUpperCamelCase;

/// Metadata about the return type of a Go method call for `method_result` assertions.
pub(super) struct GoMethodCallInfo {
    /// The call expression string.
    pub(super) call_expr: String,
    /// Whether the return type is a pointer (needs `*` dereference for value comparison).
    pub(super) is_pointer: bool,
    /// Optional Go type cast to apply to numeric literal values in `equals` assertions
    /// (e.g., `"uint"` so that `0` becomes `uint(0)` to match `*uint` deref type).
    pub(super) value_cast: Option<&'static str>,
}

/// Extract the `node_type` string argument shared by `contains_node_type` and
/// `find_nodes_by_type`, defaulting to the empty string when absent.
fn node_type_argument(args: Option<&serde_json::Value>) -> &str {
    args.and_then(|a| a.get("node_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

/// Build the `RunQuery` call info: `RunQuery` returns `(*[]QueryMatch, error)`, so callers use the
/// `is_error` check type rather than `is_pointer`.
fn build_run_query_call(result_var: &str, import_alias: &str, args: Option<&serde_json::Value>) -> GoMethodCallInfo {
    let query_source = args
        .and_then(|a| a.get("query_source"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let language = args
        .and_then(|a| a.get("language"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let query_lit = go_string_literal(query_source);
    let lang_lit = go_string_literal(language);
    GoMethodCallInfo {
        call_expr: format!("{import_alias}.RunQuery({result_var}, {lang_lit}, {query_lit}, []byte(source))"),
        is_pointer: false,
        value_cast: None,
    }
}

/// Build a Go call expression for a `method_result` assertion on a sample_language Tree.
///
/// Maps method names to the appropriate Go function calls, matching the Go binding API
/// in `packages/go/binding.go`. Returns a [`GoMethodCallInfo`] describing the call and
/// its return type characteristics.
///
/// Return types by method:
/// - `has_error_nodes`, `contains_node_type` → `*bool` (pointer)
/// - `error_count` → `*uint` (pointer, value_cast = "uint")
/// - `tree_to_sexp` → `*string` (pointer)
/// - `root_node_type` → `string` via `RootNodeInfo(tree).Kind` (value)
/// - `named_children_count` → `uint` via `RootNodeInfo(tree).NamedChildCount` (value, value_cast = "uint")
/// - `find_nodes_by_type` → `*[]NodeInfo` (pointer to slice)
/// - `run_query` → `(*[]QueryMatch, error)` (pointer + error; use `is_error` check type)
pub(super) fn build_go_method_call(
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    import_alias: &str,
) -> GoMethodCallInfo {
    match method_name {
        "root_node_type" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.RootNodeInfo({result_var}).Kind"),
            is_pointer: false,
            value_cast: None,
        },
        "named_children_count" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.RootNodeInfo({result_var}).NamedChildCount"),
            is_pointer: false,
            value_cast: Some("uint"),
        },
        "has_error_nodes" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.TreeHasErrorNodes({result_var})"),
            is_pointer: true,
            value_cast: None,
        },
        "error_count" | "tree_error_count" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.TreeErrorCount({result_var})"),
            is_pointer: true,
            value_cast: Some("uint"),
        },
        "tree_to_sexp" => GoMethodCallInfo {
            call_expr: format!("{import_alias}.TreeToSexp({result_var})"),
            is_pointer: true,
            value_cast: None,
        },
        "contains_node_type" => {
            let node_type = node_type_argument(args);
            GoMethodCallInfo {
                call_expr: format!("{import_alias}.TreeContainsNodeType({result_var}, \"{node_type}\")"),
                is_pointer: true,
                value_cast: None,
            }
        }
        "find_nodes_by_type" => {
            let node_type = node_type_argument(args);
            GoMethodCallInfo {
                call_expr: format!("{import_alias}.FindNodesByType({result_var}, \"{node_type}\")"),
                is_pointer: true,
                value_cast: None,
            }
        }
        "run_query" => build_run_query_call(result_var, import_alias, args),
        other => {
            let method_pascal = other.to_upper_camel_case();
            GoMethodCallInfo {
                call_expr: format!("{result_var}.{method_pascal}()"),
                is_pointer: false,
                value_cast: None,
            }
        }
    }
}
