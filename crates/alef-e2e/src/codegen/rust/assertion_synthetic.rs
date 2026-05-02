//! Synthetic field handlers and tree/value helpers for Rust e2e assertion rendering.

use std::fmt::Write as FmtWrite;

use crate::fixture::Assertion;

use super::args::json_to_rust_literal;
use crate::escape::rust_raw_string;

// ---------------------------------------------------------------------------
// Synthetic field handlers
// ---------------------------------------------------------------------------

pub(super) fn render_chunks_have_content(out: &mut String, result_var: &str, assertion_type: &str) {
    match assertion_type {
        "is_true" => {
            let _ = writeln!(
                out,
                "    assert!({result_var}.chunks.as_ref().is_some_and(|chunks| !chunks.is_empty() && chunks.iter().all(|c| !c.content.is_empty())), \"expected all chunks to have content\");"
            );
        }
        "is_false" => {
            let _ = writeln!(
                out,
                "    assert!({result_var}.chunks.as_ref().is_none() || {result_var}.chunks.as_ref().unwrap().iter().any(|c| c.content.is_empty()), \"expected some chunks to be empty\");"
            );
        }
        _ => {
            let _ = writeln!(
                out,
                "    // unsupported assertion type on synthetic field chunks_have_content"
            );
        }
    }
}

pub(super) fn render_chunks_have_embeddings(out: &mut String, result_var: &str, assertion_type: &str) {
    match assertion_type {
        "is_true" => {
            let _ = writeln!(
                out,
                "    assert!({result_var}.chunks.as_ref().is_some_and(|c| c.iter().all(|ch| ch.embedding.as_ref().is_some_and(|e| !e.is_empty()))), \"expected all chunks to have embeddings\");"
            );
        }
        "is_false" => {
            let _ = writeln!(
                out,
                "    assert!({result_var}.chunks.as_ref().is_none_or(|c| c.iter().any(|ch| ch.embedding.as_ref().is_none_or(|e| e.is_empty()))), \"expected some chunks to lack embeddings\");"
            );
        }
        _ => {
            let _ = writeln!(
                out,
                "    // unsupported assertion type on synthetic field chunks_have_embeddings"
            );
        }
    }
}

pub(super) fn render_embeddings_assertion(out: &mut String, result_var: &str, assertion: &Assertion) {
    // "embeddings" as a field in count_equals/count_min means the outer list.
    // embed_texts returns Vec<Vec<f32>> directly; result_var IS the embedding matrix.
    let embed_list = result_var.to_string();
    match assertion.assertion_type.as_str() {
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    assert_eq!({embed_list}.len(), {n}, \"expected exactly {n} elements, got {{}}\", {embed_list}.len());"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if n <= 1 {
                        let _ = writeln!(out, "    assert!(!{embed_list}.is_empty(), \"expected >= {n}\");");
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert!({embed_list}.len() >= {n}, \"expected at least {n} elements, got {{}}\", {embed_list}.len());"
                        );
                    }
                }
            }
        }
        "not_empty" => {
            let _ = writeln!(
                out,
                "    assert!(!{embed_list}.is_empty(), \"expected non-empty embeddings\");"
            );
        }
        "is_empty" => {
            let _ = writeln!(
                out,
                "    assert!({embed_list}.is_empty(), \"expected empty embeddings\");"
            );
        }
        _ => {
            let _ = writeln!(
                out,
                "    // skipped: unsupported assertion type on synthetic field 'embeddings'"
            );
        }
    }
}

pub(super) fn render_embedding_dimensions(out: &mut String, result_var: &str, assertion: &Assertion) {
    let embed_list = result_var;
    let expr = format!("{embed_list}.first().map_or(0, |e| e.len())");
    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                let _ = writeln!(
                    out,
                    "    assert_eq!({expr}, {lit} as usize, \"equals assertion failed\");"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                let _ = writeln!(out, "    assert!({expr} > {lit} as usize, \"expected > {lit}\");");
            }
        }
        _ => {
            let _ = writeln!(
                out,
                "    // skipped: unsupported assertion type on synthetic field 'embedding_dimensions'"
            );
        }
    }
}

pub(super) fn render_embedding_quality(out: &mut String, result_var: &str, field: &str, assertion_type: &str) {
    let embed_list = result_var;
    let pred = match field {
        "embeddings_valid" => {
            format!("{embed_list}.iter().all(|e| !e.is_empty())")
        }
        "embeddings_finite" => {
            format!("{embed_list}.iter().all(|e| e.iter().all(|v| v.is_finite()))")
        }
        "embeddings_non_zero" => {
            format!("{embed_list}.iter().all(|e| e.iter().any(|v| *v != 0.0_f32))")
        }
        "embeddings_normalized" => {
            format!(
                "{embed_list}.iter().all(|e| {{ let n: f64 = e.iter().map(|v| f64::from(*v) * f64::from(*v)).sum(); (n - 1.0_f64).abs() < 1e-3 }})"
            )
        }
        _ => unreachable!(),
    };
    match assertion_type {
        "is_true" => {
            let _ = writeln!(out, "    assert!({pred}, \"expected true\");");
        }
        "is_false" => {
            let _ = writeln!(out, "    assert!(!({pred}), \"expected false\");");
        }
        _ => {
            let _ = writeln!(
                out,
                "    // skipped: unsupported assertion type on synthetic field '{field}'"
            );
        }
    }
}

pub(super) fn render_keywords_assertion(out: &mut String, result_var: &str, assertion: &Assertion) {
    let accessor = format!("{result_var}.extracted_keywords");
    match assertion.assertion_type.as_str() {
        "not_empty" => {
            let _ = writeln!(
                out,
                "    assert!({accessor}.as_ref().is_some_and(|v| !v.is_empty()), \"expected keywords to be present and non-empty\");"
            );
        }
        "is_empty" => {
            let _ = writeln!(
                out,
                "    assert!({accessor}.as_ref().is_none_or(|v| v.is_empty()), \"expected keywords to be empty or absent\");"
            );
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if n <= 1 {
                        let _ = writeln!(
                            out,
                            "    assert!({accessor}.as_ref().is_some_and(|v| !v.is_empty()), \"expected >= {n}\");"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert!({accessor}.as_ref().is_some_and(|v| v.len() >= {n}), \"expected at least {n} keywords\");"
                        );
                    }
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    assert!({accessor}.as_ref().is_some_and(|v| v.len() == {n}), \"expected exactly {n} keywords\");"
                    );
                }
            }
        }
        _ => {
            let _ = writeln!(
                out,
                "    // skipped: unsupported assertion type on synthetic field 'keywords'"
            );
        }
    }
}

pub(super) fn render_keywords_count_assertion(out: &mut String, result_var: &str, assertion: &Assertion) {
    let expr = format!("{result_var}.extracted_keywords.as_ref().map_or(0, |v| v.len())");
    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                let _ = writeln!(
                    out,
                    "    assert_eq!({expr}, {lit} as usize, \"equals assertion failed\");"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                let _ = writeln!(out, "    assert!({expr} <= {lit} as usize, \"expected <= {lit}\");");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                let _ = writeln!(out, "    assert!({expr} >= {lit} as usize, \"expected >= {lit}\");");
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                let _ = writeln!(out, "    assert!({expr} > {lit} as usize, \"expected > {lit}\");");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                let _ = writeln!(out, "    assert!({expr} < {lit} as usize, \"expected < {lit}\");");
            }
        }
        _ => {
            let _ = writeln!(
                out,
                "    // skipped: unsupported assertion type on synthetic field 'keywords_count'"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tree helpers
// ---------------------------------------------------------------------------

/// Translate a fixture pseudo-field name on a `tree_sitter::Tree` into the
/// correct Rust accessor expression.
pub(super) fn tree_field_access_expr(field: &str, result_var: &str, module: &str) -> String {
    match field {
        "root_child_count" => format!("{result_var}.root_node().child_count()"),
        "root_node_type" => format!("{result_var}.root_node().kind()"),
        "named_children_count" => format!("{result_var}.root_node().named_child_count()"),
        "has_error_nodes" => format!("{module}::tree_has_error_nodes(&{result_var})"),
        "error_count" | "tree_error_count" => format!("{module}::tree_error_count(&{result_var})"),
        "tree_to_sexp" => format!("{module}::tree_to_sexp(&{result_var})"),
        // Unknown pseudo-field: fall back to direct field access (will likely fail to compile,
        // but gives the developer a useful error pointing to the fixture).
        other => format!("{result_var}.{other}"),
    }
}

/// Build a Rust call expression for a logical "method" on a `tree_sitter::Tree`.
pub(super) fn build_tree_call_expr(
    field_access: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    module: &str,
) -> String {
    match method_name {
        "root_child_count" => format!("{field_access}.root_node().child_count()"),
        "root_node_type" => format!("{field_access}.root_node().kind()"),
        "named_children_count" => format!("{field_access}.root_node().named_child_count()"),
        "has_error_nodes" => format!("{module}::tree_has_error_nodes(&{field_access})"),
        "error_count" | "tree_error_count" => format!("{module}::tree_error_count(&{field_access})"),
        "tree_to_sexp" => format!("{module}::tree_to_sexp(&{field_access})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{module}::tree_contains_node_type(&{field_access}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{module}::find_nodes_by_type(&{field_access}, \"{node_type}\")")
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
            // Use a raw string for the query to avoid escaping issues.
            // run_query returns Result — unwrap it for assertion access.
            format!(
                "{module}::run_query(&{field_access}, \"{language}\", r#\"{query_source}\"#, source.as_bytes()).unwrap()"
            )
        }
        // Fallback: try as a plain method call.
        _ => {
            if let Some(args) = args {
                let arg_lit = json_to_rust_literal(args, "");
                format!("{field_access}.{method_name}({arg_lit})")
            } else {
                format!("{field_access}.{method_name}()")
            }
        }
    }
}

/// Returns `true` when the tree method name produces a numeric result (usize/u64),
/// meaning `>= N` comparisons should use direct numeric comparison rather than
/// `.is_empty()` (which only works for collections).
pub(super) fn is_tree_numeric_method(method_name: &str) -> bool {
    matches!(
        method_name,
        "root_child_count" | "named_children_count" | "error_count" | "tree_error_count"
    )
}

// ---------------------------------------------------------------------------
// Value helpers
// ---------------------------------------------------------------------------

/// Convert a JSON numeric value to a Rust literal suitable for comparisons.
///
/// Whole numbers (no fractional part) are emitted as bare integer literals so
/// they are compatible with `usize`, `u64`, etc. (e.g., `.len()` results).
/// Numbers with a fractional component get the `_f64` suffix.
pub fn numeric_literal(value: &serde_json::Value) -> String {
    if let Some(n) = value.as_f64() {
        if n.fract() == 0.0 {
            // Whole number — emit without a type suffix so Rust can infer the
            // correct integer type from context (usize, u64, i64, …).
            return format!("{}", n as i64);
        }
        return format!("{n}_f64");
    }
    // Fallback: use the raw JSON representation.
    value.to_string()
}

pub fn value_to_rust_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => rust_raw_string(s),
        serde_json::Value::Bool(b) => format!("{b}"),
        serde_json::Value::Number(n) => n.to_string(),
        other => {
            let s = other.to_string();
            format!("\"{s}\"")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_literal_whole_number_has_no_suffix() {
        let v = serde_json::Value::Number(serde_json::Number::from(42u64));
        assert_eq!(numeric_literal(&v), "42");
    }

    #[test]
    fn numeric_literal_fractional_has_f64_suffix() {
        let v = serde_json::json!(1.5f64);
        let out = numeric_literal(&v);
        assert!(out.ends_with("_f64"), "got: {out}");
    }

    #[test]
    fn value_to_rust_string_string_produces_raw_string() {
        let v = serde_json::Value::String("hello".to_string());
        let out = value_to_rust_string(&v);
        assert!(out.starts_with("r#") || out.starts_with('"'), "got: {out}");
    }

    #[test]
    fn tree_field_access_expr_root_child_count() {
        let expr = tree_field_access_expr("root_child_count", "tree", "my_mod");
        assert_eq!(expr, "tree.root_node().child_count()");
    }

    #[test]
    fn is_tree_numeric_method_recognizes_child_count() {
        assert!(is_tree_numeric_method("root_child_count"));
        assert!(!is_tree_numeric_method("has_error_nodes"));
    }
}
