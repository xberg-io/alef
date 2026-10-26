//! The Ruby call expression for a `method_result` assertion.

/// Build a Ruby call expression for a `method_result` assertion on a sample_language Tree.
/// Maps method names to the appropriate Ruby method or module-function calls.
pub(in crate::e2e::codegen::ruby) fn build_ruby_method_call(
    call_receiver: &str,
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}.root_node.child_count"),
        "root_node_type" => format!("{result_var}.root_node.type"),
        "named_children_count" => format!("{result_var}.named_child_count"),
        "has_error_nodes" => format!("{call_receiver}.tree_has_error_nodes({result_var})"),
        "error_count" | "tree_error_count" => format!("{call_receiver}.tree_error_count({result_var})"),
        "tree_to_sexp" => format!("{call_receiver}.tree_to_sexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{call_receiver}.tree_contains_node_type({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{call_receiver}.find_nodes_by_type({result_var}, \"{node_type}\")")
        }
        "run_query" => {
            let query_source = args
                .and_then(|a| a.get("query_source"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let language = args
                .and_then(|a| a.get("language"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{call_receiver}.run_query({result_var}, \"{language}\", \"{query_source}\", source)")
        }
        _ => format!("{result_var}.{method_name}"),
    }
}
