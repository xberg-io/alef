//! Assertion rendering for TypeScript e2e tests.

use std::fmt::Write as FmtWrite;

use crate::escape::escape_js;
use crate::field_access::FieldResolver;
use crate::fixture::Assertion;

use super::json::json_to_js;

/// Render a single assertion into the test body.
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
) {
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct property accesses on the result.
    if let Some(f) = &assertion.field {
        if render_synthetic_field_assertion(out, assertion, result_var, f) {
            return;
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "    // skipped: field '{f}' not available on result type");
            return;
        }
    }

    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() => field_resolver.accessor(f, "typescript", result_var),
        _ => result_var.to_string(),
    };

    let field_is_array = assertion
        .field
        .as_deref()
        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));

    render_standard_assertion(out, assertion, result_var, &field_expr, field_resolver, field_is_array);
}

/// Try to render a synthetic/virtual field assertion. Returns `true` when the field was handled.
fn render_synthetic_field_assertion(out: &mut String, assertion: &Assertion, result_var: &str, field: &str) -> bool {
    match field {
        "chunks_have_content" => {
            let pred = format!("({result_var}.chunks ?? []).every((c: {{ content?: string }}) => !!c.content)");
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "chunks_have_embeddings" => {
            let pred = format!(
                "({result_var}.chunks ?? []).every((c: {{ embedding?: number[] }}) => c.embedding != null && c.embedding.length > 0)"
            );
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
                "embeddings_valid" => {
                    format!("{result_var}.every((e: number[]) => e.length > 0)")
                }
                "embeddings_finite" => {
                    format!("{result_var}.every((e: number[]) => e.every((v: number) => isFinite(v)))")
                }
                "embeddings_non_zero" => {
                    format!("{result_var}.every((e: number[]) => e.some((v: number) => v !== 0))")
                }
                "embeddings_normalized" => {
                    format!(
                        "{result_var}.every((e: number[]) => {{ const n = e.reduce((s: number, v: number) => s + v * v, 0); return Math.abs(n - 1.0) < 1e-3; }})"
                    )
                }
                _ => unreachable!(),
            };
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "keywords" | "keywords_count" => {
            let _ = writeln!(
                out,
                "    // skipped: field '{field}' not available on Node JsExtractionResult"
            );
            true
        }
        _ => false,
    }
}

fn emit_bool_assertion(out: &mut String, pred: &str, assertion_type: &str, field: &str) {
    match assertion_type {
        "is_true" => {
            let _ = writeln!(out, "    expect({pred}).toBe(true);");
        }
        "is_false" => {
            let _ = writeln!(out, "    expect({pred}).toBe(false);");
        }
        _ => {
            let _ = writeln!(
                out,
                "    // skipped: unsupported assertion type on synthetic field '{field}'"
            );
        }
    }
}

fn render_embeddings_assertion(out: &mut String, assertion: &Assertion, result_var: &str) {
    match assertion.assertion_type.as_str() {
        "count_equals" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let _ = writeln!(out, "    expect({result_var}.length).toBe({js_val});");
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let _ = writeln!(out, "    expect({result_var}.length).toBeGreaterThanOrEqual({js_val});");
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "    expect({result_var}.length).toBeGreaterThan(0);");
        }
        "is_empty" => {
            let _ = writeln!(out, "    expect({result_var}.length).toBe(0);");
        }
        _ => {
            let _ = writeln!(
                out,
                "    // skipped: unsupported assertion type on synthetic field 'embeddings'"
            );
        }
    }
}

fn render_embedding_dimensions(out: &mut String, assertion: &Assertion, result_var: &str) {
    let expr = format!("({result_var}.length > 0 ? {result_var}[0].length : 0)");
    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let _ = writeln!(out, "    expect({expr}).toBe({js_val});");
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let _ = writeln!(out, "    expect({expr}).toBeGreaterThan({js_val});");
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
    field_expr: &str,
    field_resolver: &FieldResolver,
    field_is_array: bool,
) {
    let _ = escape_js; // imported for potential future use; used by visitors module
    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                if expected.is_string() {
                    let resolved = assertion.field.as_deref().unwrap_or("");
                    if !resolved.is_empty() && field_resolver.is_optional(field_resolver.resolve(resolved)) {
                        let _ = writeln!(out, "    expect(({field_expr} ?? \"\").trim()).toBe({js_val});");
                    } else {
                        let _ = writeln!(out, "    expect({field_expr}.trim()).toBe({js_val});");
                    }
                } else {
                    let _ = writeln!(out, "    expect({field_expr}).toBe({js_val});");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let resolved = assertion.field.as_deref().unwrap_or("");
                if field_is_array && expected.is_string() {
                    let _ = writeln!(
                        out,
                        "    expect({field_expr}.some((item) => _alefE2eItemTexts(item).some((text) => text.includes({js_val})))).toBe(true);"
                    );
                } else if !resolved.is_empty()
                    && expected.is_string()
                    && field_resolver.is_optional(field_resolver.resolve(resolved))
                {
                    let _ = writeln!(out, "    expect({field_expr} ?? \"\").toContain({js_val});");
                } else {
                    let _ = writeln!(out, "    expect({field_expr}).toContain({js_val});");
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let js_val = json_to_js(val);
                    if field_is_array && val.is_string() {
                        let _ = writeln!(
                            out,
                            "    expect({field_expr}.some((item) => _alefE2eItemTexts(item).some((text) => text.includes({js_val})))).toBe(true);"
                        );
                    } else {
                        let _ = writeln!(out, "    expect({field_expr}).toContain({js_val});");
                    }
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                if field_is_array && expected.is_string() {
                    let _ = writeln!(
                        out,
                        "    expect({field_expr}.some((item) => _alefE2eItemTexts(item).some((text) => text.includes({js_val})))).toBe(false);"
                    );
                } else {
                    let _ = writeln!(out, "    expect({field_expr}).not.toContain({js_val});");
                }
            }
        }
        "not_empty" => {
            let resolved = assertion.field.as_deref().unwrap_or("");
            if !resolved.is_empty() && field_resolver.is_optional(field_resolver.resolve(resolved)) {
                let _ = writeln!(out, "    expect(({field_expr} ?? \"\").length).toBeGreaterThan(0);");
            } else {
                let _ = writeln!(out, "    expect({field_expr}.length).toBeGreaterThan(0);");
            }
        }
        "is_empty" => {
            let resolved = assertion.field.as_deref().unwrap_or("");
            if !resolved.is_empty() && field_resolver.is_optional(field_resolver.resolve(resolved)) {
                let _ = writeln!(out, "    expect(({field_expr} ?? \"\").length).toBe(0);");
            } else {
                let _ = writeln!(out, "    expect(({field_expr} ?? \"\").length).toBe(0);");
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(json_to_js).collect();
                let arr_str = items.join(", ");
                if field_is_array && values.iter().all(serde_json::Value::is_string) {
                    let _ = writeln!(
                        out,
                        "    expect([{arr_str}].some((v) => {field_expr}.some((item) => _alefE2eItemTexts(item).some((text) => text.includes(v))))).toBe(true);"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    expect([{arr_str}].some((v) => {field_expr}.includes(v))).toBe(true);"
                    );
                }
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let _ = writeln!(out, "    expect({field_expr}).toBeGreaterThan({js_val});");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let _ = writeln!(out, "    expect({field_expr}).toBeLessThan({js_val});");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let _ = writeln!(out, "    expect({field_expr}).toBeGreaterThanOrEqual({js_val});");
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let _ = writeln!(out, "    expect({field_expr}).toBeLessThanOrEqual({js_val});");
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let resolved = assertion.field.as_deref().unwrap_or("");
                if !resolved.is_empty() && field_resolver.is_optional(field_resolver.resolve(resolved)) {
                    let _ = writeln!(
                        out,
                        "    expect(({field_expr} ?? \"\").startsWith({js_val})).toBe(true);"
                    );
                } else {
                    let _ = writeln!(out, "    expect({field_expr}.startsWith({js_val})).toBe(true);");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).toBeGreaterThanOrEqual({n});");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).toBe({n});");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    expect({field_expr}).toBe(true);");
        }
        "is_false" => {
            let _ = writeln!(out, "    expect({field_expr}).toBe(false);");
        }
        "method_result" => {
            render_method_result_assertion(out, assertion, result_var);
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).toBeGreaterThanOrEqual({n});");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    expect({field_expr}.length).toBeLessThanOrEqual({n});");
                }
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let _ = writeln!(out, "    expect({field_expr}.endsWith({js_val})).toBe(true);");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                if let Some(pattern) = expected.as_str() {
                    let _ = writeln!(out, "    expect({field_expr}).toMatch(/{pattern}/);");
                }
            }
        }
        "not_error" => {
            // No-op — if we got here, the call succeeded (it would have thrown).
        }
        "error" => {
            // Handled at the test level (early return above).
        }
        other => {
            panic!("TypeScript e2e generator: unsupported assertion type: {other}");
        }
    }
}

fn render_method_result_assertion(out: &mut String, assertion: &Assertion, result_var: &str) {
    if let Some(method_name) = &assertion.method {
        let call_expr = build_ts_method_call(result_var, method_name, assertion.args.as_ref());
        let check = assertion.check.as_deref().unwrap_or("is_true");
        match check {
            "equals" => {
                if let Some(val) = &assertion.value {
                    let js_val = json_to_js(val);
                    let _ = writeln!(out, "    expect({call_expr}).toBe({js_val});");
                }
            }
            "is_true" => {
                let _ = writeln!(out, "    expect({call_expr}).toBe(true);");
            }
            "is_false" => {
                let _ = writeln!(out, "    expect({call_expr}).toBe(false);");
            }
            "greater_than_or_equal" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    let _ = writeln!(out, "    expect({call_expr}).toBeGreaterThanOrEqual({n});");
                }
            }
            "count_min" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    let _ = writeln!(out, "    expect({call_expr}.length).toBeGreaterThanOrEqual({n});");
                }
            }
            "contains" => {
                if let Some(val) = &assertion.value {
                    let js_val = json_to_js(val);
                    let _ = writeln!(out, "    expect({call_expr}).toContain({js_val});");
                }
            }
            "is_error" => {
                let _ = writeln!(out, "    expect(() => {{ {call_expr}; }}).toThrow();");
            }
            other_check => {
                panic!("TypeScript e2e generator: unsupported method_result check type: {other_check}");
            }
        }
    } else {
        panic!("TypeScript e2e generator: method_result assertion missing 'method' field");
    }
}

/// Build a TypeScript call expression for a method_result assertion on a tree-sitter Tree.
pub(super) fn build_ts_method_call(result_var: &str, method_name: &str, args: Option<&serde_json::Value>) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}.rootNode.childCount"),
        "root_node_type" => format!("{result_var}.rootNode.type"),
        "named_children_count" => format!("{result_var}.rootNode.namedChildCount"),
        "has_error_nodes" => format!("treeHasErrorNodes({result_var})"),
        "error_count" | "tree_error_count" => format!("treeErrorCount({result_var})"),
        "tree_to_sexp" => format!("treeToSexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("treeContainsNodeType({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("findNodesByType({result_var}, \"{node_type}\")")
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
            format!("runQuery({result_var}, \"{language}\", \"{query_source}\", source)")
        }
        _ => {
            if let Some(args_val) = args {
                let arg_str = args_val
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .map(|(k, v)| format!("{}: {}", k, json_to_js(v)))
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

    fn array_resolver(field: &str) -> FieldResolver {
        let result_fields = HashSet::from([field.to_string()]);
        let array_fields = HashSet::from([field.to_string()]);
        FieldResolver::new(&HashMap::new(), &HashSet::new(), &result_fields, &array_fields)
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
    fn render_assertion_not_empty_emits_length_check() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_empty", None, None);
        let mut out = String::new();
        render_assertion(&mut out, &assertion, "result", &resolver);
        assert!(out.contains(".length"), "got: {out}");
    }

    #[test]
    fn render_assertion_equals_string_trims() {
        let resolver = empty_resolver();
        let assertion = make_assertion("equals", None, Some(serde_json::Value::String("hello".into())));
        let mut out = String::new();
        render_assertion(&mut out, &assertion, "result", &resolver);
        assert!(out.contains(".trim()"), "got: {out}");
    }

    #[test]
    fn render_assertion_is_empty_allows_nullish_simple_results() {
        let resolver = empty_resolver();
        let assertion = make_assertion("is_empty", None, None);
        let mut out = String::new();
        render_assertion(&mut out, &assertion, "result", &resolver);
        assert!(out.contains("(result ?? \"\").length"), "got: {out}");
    }

    #[test]
    fn render_assertion_contains_string_array_uses_item_texts() {
        let resolver = array_resolver("structure");
        let assertion = make_assertion(
            "contains",
            Some("structure"),
            Some(serde_json::Value::String("Function".into())),
        );
        let mut out = String::new();
        render_assertion(&mut out, &assertion, "result", &resolver);
        assert!(out.contains("_alefE2eItemTexts(item)"), "got: {out}");
    }

    #[test]
    fn build_ts_method_call_root_child_count() {
        let expr = build_ts_method_call("tree", "root_child_count", None);
        assert_eq!(expr, "tree.rootNode.childCount");
    }
}
