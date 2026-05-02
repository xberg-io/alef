//! Assertion rendering for Python e2e tests.

use std::collections::HashSet;
use std::fmt::Write as FmtWrite;

use crate::field_access::FieldResolver;
use crate::fixture::Assertion;

use super::json::{python_string_literal, value_to_python_string};

/// Render a single assertion into the test function body.
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    fields_enum: &HashSet<String>,
    result_is_simple: bool,
) {
    // When result_is_simple, skip fields that reference struct sub-fields.
    if result_is_simple {
        if let Some(f) = &assertion.field {
            let f_lower = f.to_lowercase();
            if !f.is_empty()
                && f_lower != "content"
                && f_lower != "result"
                && (f_lower.starts_with("metadata")
                    || f_lower.starts_with("document")
                    || f_lower.starts_with("structure")
                    || f_lower.starts_with("pages")
                    || f_lower.starts_with("chunks")
                    || f_lower.starts_with("tables")
                    || f_lower.starts_with("images")
                    || f_lower.starts_with("mime_type")
                    || f_lower.starts_with("is_")
                    || f_lower == "byte_length"
                    || f_lower == "page_count"
                    || f_lower == "output_format"
                    || f_lower == "extraction_method")
            {
                let _ = writeln!(out, "    # skipped: field '{f}' not applicable for simple result type");
                return;
            }
        }
    }

    // Handle synthetic / derived fields.
    if let Some(f) = &assertion.field {
        if render_synthetic_field(out, assertion, result_var, f) {
            return;
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if !result_is_simple {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
                let _ = writeln!(out, "    # skipped: field '{f}' not available on result type");
                return;
            }
        }
    }

    let field_access = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "python", result_var),
            _ => result_var.to_string(),
        }
    };

    let field_is_enum = assertion.field.as_deref().is_some_and(|f| {
        if fields_enum.contains(f) {
            return true;
        }
        let resolved = field_resolver.resolve(f);
        if fields_enum.contains(resolved) {
            return true;
        }
        field_resolver.accessor(f, "python", result_var).contains("[0]")
    });

    let field_is_optional = match &assertion.field {
        Some(f) if !f.is_empty() => {
            let resolved = field_resolver.resolve(f);
            field_resolver.is_optional(resolved)
        }
        _ => false,
    };
    let field_is_array = assertion
        .field
        .as_deref()
        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));

    render_standard_assertion(
        out,
        assertion,
        result_var,
        &field_access,
        field_is_enum,
        field_is_optional,
        field_is_array,
    );
}

fn render_synthetic_field(out: &mut String, assertion: &Assertion, result_var: &str, field: &str) -> bool {
    match field {
        "chunks_have_content" => {
            let pred = format!("all(c.content for c in ({result_var}.chunks or []))");
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "chunks_have_embeddings" => {
            let pred =
                format!("all(c.embedding is not None and len(c.embedding) > 0 for c in ({result_var}.chunks or []))");
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "embeddings" => {
            render_embeddings_assertion(out, assertion, result_var);
            true
        }
        "embedding_dimensions" => {
            render_embedding_dimensions(out, assertion, result_var);
            true
        }
        "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
            let pred = match field {
                "embeddings_valid" => format!("all(bool(e) for e in {result_var})"),
                "embeddings_finite" => {
                    format!("all(v == v and abs(v) != float('inf') for e in {result_var} for v in e)")
                }
                "embeddings_non_zero" => {
                    format!("all(any(v != 0.0 for v in e) for e in {result_var})")
                }
                "embeddings_normalized" => {
                    format!("all(abs(sum(v * v for v in e) - 1.0) < 1e-3 for e in {result_var})")
                }
                _ => unreachable!(),
            };
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "keywords" | "keywords_count" => {
            let _ = writeln!(
                out,
                "    # skipped: field '{field}' not available on Python ExtractionResult"
            );
            true
        }
        _ => false,
    }
}

fn emit_bool_assertion(out: &mut String, pred: &str, assertion_type: &str, field: &str) {
    match assertion_type {
        "is_true" => {
            let _ = writeln!(out, "    assert {pred}  # noqa: S101");
        }
        "is_false" => {
            let _ = writeln!(out, "    assert not ({pred})  # noqa: S101");
        }
        _ => {
            let _ = writeln!(
                out,
                "    # skipped: unsupported assertion type on synthetic field '{field}'"
            );
        }
    }
}

fn render_embeddings_assertion(out: &mut String, assertion: &Assertion, result_var: &str) {
    match assertion.assertion_type.as_str() {
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    assert len({result_var}) == {n}  # noqa: S101");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    assert len({result_var}) >= {n}  # noqa: S101");
                }
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "    assert len({result_var}) > 0  # noqa: S101");
        }
        "is_empty" => {
            let _ = writeln!(out, "    assert len({result_var}) == 0  # noqa: S101");
        }
        _ => {
            let _ = writeln!(
                out,
                "    # skipped: unsupported assertion type on synthetic field 'embeddings'"
            );
        }
    }
}

fn render_embedding_dimensions(out: &mut String, assertion: &Assertion, result_var: &str) {
    let expr = format!("(len({result_var}[0]) if {result_var} else 0)");
    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(val) = &assertion.value {
                let py_val = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expr} == {py_val}  # noqa: S101");
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let py_val = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expr} > {py_val}  # noqa: S101");
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

fn render_standard_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_access: &str,
    field_is_enum: bool,
    field_is_optional: bool,
    field_is_array: bool,
) {
    let _ = (result_var, python_string_literal); // available for potential future use
    match assertion.assertion_type.as_str() {
        "error" | "not_error" => {
            // Handled at call site.
        }
        "equals" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let op = if val.is_boolean() || val.is_null() { "is" } else { "==" };
                if val.is_string() {
                    let _ = writeln!(out, "    assert {field_access}.strip() {op} {expected}  # noqa: S101");
                } else {
                    let _ = writeln!(out, "    assert {field_access} {op} {expected}  # noqa: S101");
                }
            }
        }
        "contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let cmp_expr =
                    python_contains_expr(field_access, &expected, field_is_enum, field_is_array, val.is_string());
                if field_is_optional {
                    let _ = writeln!(out, "    assert {field_access} is not None  # noqa: S101");
                    let _ = writeln!(out, "    assert {cmp_expr}  # noqa: S101");
                } else {
                    let _ = writeln!(out, "    assert {cmp_expr}  # noqa: S101");
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let expected = value_to_python_string(val);
                    let cmp_expr =
                        python_contains_expr(field_access, &expected, field_is_enum, field_is_array, val.is_string());
                    if field_is_optional {
                        let _ = writeln!(out, "    assert {field_access} is not None  # noqa: S101");
                        let _ = writeln!(out, "    assert {cmp_expr}  # noqa: S101");
                    } else {
                        let _ = writeln!(out, "    assert {cmp_expr}  # noqa: S101");
                    }
                }
            }
        }
        "not_contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let cmp_expr =
                    python_contains_expr(field_access, &expected, field_is_enum, field_is_array, val.is_string());
                if field_is_optional {
                    let _ = writeln!(
                        out,
                        "    assert {field_access} is None or not ({cmp_expr})  # noqa: S101"
                    );
                } else {
                    let _ = writeln!(out, "    assert not ({cmp_expr})  # noqa: S101");
                }
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "    assert {field_access}  # noqa: S101");
        }
        "is_empty" => {
            let _ = writeln!(out, "    assert not {field_access}  # noqa: S101");
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(value_to_python_string).collect();
                let list_str = items.join(", ");
                let cmp_expr = if field_is_array {
                    format!(
                        "any(any(v in text for text in _alef_e2e_item_texts(item)) for item in {field_access} for v in [{list_str}])"
                    )
                } else if field_is_enum {
                    format!("any(v in str({field_access}).lower() for v in [{list_str}])")
                } else {
                    format!("any(v in {field_access} for v in [{list_str}])")
                };
                if field_is_optional {
                    let _ = writeln!(out, "    assert {field_access} is not None  # noqa: S101");
                    let _ = writeln!(out, "    assert {cmp_expr}  # noqa: S101");
                } else {
                    let _ = writeln!(out, "    assert {cmp_expr}  # noqa: S101");
                }
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {field_access} > {expected}  # noqa: S101");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {field_access} < {expected}  # noqa: S101");
            }
        }
        "greater_than_or_equal" | "min" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {field_access} >= {expected}  # noqa: S101");
            }
        }
        "less_than_or_equal" | "max" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {field_access} <= {expected}  # noqa: S101");
            }
        }
        "starts_with" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {field_access}.startswith({expected})  # noqa: S101");
            }
        }
        "ends_with" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {field_access}.endswith({expected})  # noqa: S101");
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    assert len({field_access}) >= {n}  # noqa: S101");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    assert len({field_access}) <= {n}  # noqa: S101");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    assert len({field_access}) >= {n}  # noqa: S101");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    assert len({field_access}) == {n}  # noqa: S101");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    assert {field_access} is True  # noqa: S101");
        }
        "is_false" => {
            let _ = writeln!(out, "    assert not {field_access}  # noqa: S101");
        }
        "method_result" => {
            render_method_result(out, assertion, result_var);
        }
        "matches_regex" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    import re  # noqa: PLC0415");
                let _ = writeln!(
                    out,
                    "    assert re.search({expected}, {field_access}) is not None  # noqa: S101"
                );
            }
        }
        other => {
            panic!("unsupported assertion type: {other}");
        }
    }
}

fn python_contains_expr(
    field_access: &str,
    expected: &str,
    field_is_enum: bool,
    field_is_array: bool,
    expected_is_string: bool,
) -> String {
    if field_is_array && expected_is_string {
        return format!("any({expected} in text for item in {field_access} for text in _alef_e2e_item_texts(item))");
    }
    if field_is_enum && expected_is_string {
        return format!("{expected} in str({field_access}).lower()");
    }
    format!("{expected} in {field_access}")
}

fn render_method_result(out: &mut String, assertion: &Assertion, result_var: &str) {
    if let Some(method_name) = &assertion.method {
        let call_expr = build_python_method_call(result_var, method_name, assertion.args.as_ref());
        let check = assertion.check.as_deref().unwrap_or("is_true");
        match check {
            "equals" => {
                if let Some(val) = &assertion.value {
                    if val.is_boolean() {
                        if val.as_bool() == Some(true) {
                            let _ = writeln!(out, "    assert {call_expr} is True  # noqa: S101");
                        } else {
                            let _ = writeln!(out, "    assert {call_expr} is False  # noqa: S101");
                        }
                    } else {
                        let expected = value_to_python_string(val);
                        let _ = writeln!(out, "    assert {call_expr} == {expected}  # noqa: S101");
                    }
                }
            }
            "is_true" => {
                let _ = writeln!(out, "    assert {call_expr}  # noqa: S101");
            }
            "is_false" => {
                let _ = writeln!(out, "    assert not {call_expr}  # noqa: S101");
            }
            "greater_than_or_equal" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    let _ = writeln!(out, "    assert {call_expr} >= {n}  # noqa: S101");
                }
            }
            "count_min" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    let _ = writeln!(out, "    assert len({call_expr}) >= {n}  # noqa: S101");
                }
            }
            "contains" => {
                if let Some(val) = &assertion.value {
                    let expected = value_to_python_string(val);
                    let _ = writeln!(out, "    assert {expected} in {call_expr}  # noqa: S101");
                }
            }
            "is_error" => {
                let _ = writeln!(out, "    with pytest.raises(Exception):  # noqa: B017");
                let _ = writeln!(out, "        {call_expr}");
            }
            other_check => {
                panic!("unsupported method_result check type: {other_check}");
            }
        }
    } else {
        panic!("method_result assertion missing 'method' field");
    }
}

pub(super) fn build_python_method_call(
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}.root_node().child_count()"),
        "root_node_type" => format!("{result_var}.root_node().kind()"),
        "named_children_count" => format!("{result_var}.root_node().named_child_count()"),
        "has_error_nodes" => format!("tree_has_error_nodes({result_var})"),
        "error_count" | "tree_error_count" => format!("tree_error_count({result_var})"),
        "tree_to_sexp" => format!("tree_to_sexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("tree_contains_node_type({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("find_nodes_by_type({result_var}, \"{node_type}\")")
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
            format!("run_query({result_var}, \"{language}\", \"{query_source}\", source)")
        }
        _ => {
            if let Some(args_val) = args {
                let arg_str = args_val
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .map(|(k, v)| format!("{}={}", k, value_to_python_string(v)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                format!("{result_var}.{method_name}({arg_str})")
            } else {
                format!("{result_var}.{method_name}()")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::field_access::FieldResolver;
    use crate::fixture::Assertion;

    fn empty_resolver() -> FieldResolver {
        FieldResolver::new(&HashMap::new(), &HashSet::new(), &HashSet::new(), &HashSet::new())
    }

    fn resolver_with_array_field(field: &str) -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::from([field.to_string()]),
        )
    }

    fn make_assertion(assertion_type: &str, field: Option<&str>, value: Option<serde_json::Value>) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: field.map(|s| s.to_string()),
            value,
            values: None,
            method: None,
            args: None,
            check: None,
        }
    }

    #[test]
    fn render_assertion_not_empty_emits_assert() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_empty", None, None);
        let mut out = String::new();
        render_assertion(&mut out, &assertion, "result", &resolver, &HashSet::new(), false);
        assert!(out.contains("assert result"), "got: {out}");
    }

    #[test]
    fn render_assertion_equals_string_uses_strip() {
        let resolver = empty_resolver();
        let assertion = make_assertion("equals", None, Some(serde_json::Value::String("hello".into())));
        let mut out = String::new();
        render_assertion(&mut out, &assertion, "result", &resolver, &HashSet::new(), false);
        assert!(out.contains(".strip()"), "got: {out}");
    }

    #[test]
    fn render_assertion_contains_string_array_uses_item_texts() {
        let resolver = resolver_with_array_field("structure");
        let assertion = make_assertion(
            "contains",
            Some("structure"),
            Some(serde_json::Value::String("Function".into())),
        );
        let mut out = String::new();
        render_assertion(&mut out, &assertion, "result", &resolver, &HashSet::new(), false);

        assert!(out.contains("_alef_e2e_item_texts(item)"), "got: {out}");
        assert!(out.contains("for item in result.structure"), "got: {out}");
    }

    #[test]
    fn build_python_method_call_root_child_count() {
        let expr = build_python_method_call("tree", "root_child_count", None);
        assert_eq!(expr, "tree.root_node().child_count()");
    }
}
