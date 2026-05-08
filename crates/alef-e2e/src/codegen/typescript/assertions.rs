//! Assertion rendering for TypeScript e2e tests.

use crate::field_access::FieldResolver;
use crate::fixture::Assertion;

use super::json::json_to_js;

/// Render a single assertion into the test body.
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
) {
    // For simple-result methods (e.g., `speech` returning bytes/Buffer), every
    // field-based assertion targets the result itself — there is no struct to
    // access. Drop length-only assertions onto the result directly and skip
    // anything that requires a real struct sub-field.
    if result_is_simple {
        if let Some(f) = &assertion.field {
            if !f.is_empty() {
                match assertion.assertion_type.as_str() {
                    "not_empty" => {
                        out.push_str(&format!("    expect({result_var}.length).toBeGreaterThan(0);\n"));
                        return;
                    }
                    "is_empty" => {
                        out.push_str(&format!("    expect({result_var}.length).toBe(0);\n"));
                        return;
                    }
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            out.push_str(&format!(
                                "    expect({result_var}.length).toBe({js_val});\n"
                            ));
                        }
                        return;
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            out.push_str(&format!(
                                "    expect({result_var}.length).toBeGreaterThanOrEqual({js_val});\n"
                            ));
                        }
                        return;
                    }
                    _ => {
                        out.push_str(&format!(
                            "    // skipped: field '{f}' not applicable for simple result type\n"
                        ));
                        return;
                    }
                }
            }
        }
    }

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
            out.push_str(&format!("    // skipped: field '{f}' not available on result type\n"));
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
            out.push_str(&format!(
                "    // skipped: field '{field}' not available on Node JsExtractionResult\n"
            ));
            true
        }
        _ => false,
    }
}

fn emit_bool_assertion(out: &mut String, pred: &str, assertion_type: &str, field: &str) {
    let pred = pred.to_string();
    let assertion_type = assertion_type.to_string();
    let field_name = field.to_string();
    let rendered = crate::template_env::render(
        "typescript/synthetic_assertion.jinja",
        minijinja::context! {
            assertion_type,
            pred,
            field_name,
        },
    );
    out.push_str(&rendered);
}

fn render_embeddings_assertion(out: &mut String, assertion: &Assertion, result_var: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let result_var = result_var.to_string();
    let field_name = "embeddings".to_string();

    match assertion_type {
        "count_equals" | "count_min" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let rendered = crate::template_env::render(
                    "typescript/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_type,
                        result_var,
                        field_name,
                        js_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_empty" | "is_empty" => {
            let rendered = crate::template_env::render(
                "typescript/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_type,
                    result_var,
                    field_name,
                },
            );
            out.push_str(&rendered);
        }
        _ => {
            let rendered = crate::template_env::render(
                "typescript/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_type,
                    field_name,
                },
            );
            out.push_str(&rendered);
        }
    }
}

fn render_embedding_dimensions(out: &mut String, assertion: &Assertion, result_var: &str) {
    let expr = format!("({result_var}.length > 0 ? {result_var}[0].length : 0)");
    let assertion_type = assertion.assertion_type.as_str();
    let expr = expr.clone();
    let field_name = "embedding_dimensions".to_string();

    match assertion_type {
        "equals" | "greater_than" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let rendered = crate::template_env::render(
                    "typescript/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_type,
                        expr,
                        field_name,
                        js_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        _ => {
            let rendered = crate::template_env::render(
                "typescript/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_type,
                    field_name,
                },
            );
            out.push_str(&rendered);
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
    let assertion_type = assertion.assertion_type.as_str();
    let resolved_field = assertion.field.as_deref().unwrap_or("");
    let field_is_optional =
        !resolved_field.is_empty() && field_resolver.is_optional(field_resolver.resolve(resolved_field));

    // Handle different assertion types
    match assertion_type {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let rendered = crate::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        field_is_optional => field_is_optional,
                        is_string_val => expected.is_string(),
                        js_val => js_val,
                        has_js_val => true,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let rendered = crate::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        field_is_optional => field_is_optional,
                        field_is_array => field_is_array,
                        is_string_val => expected.is_string(),
                        js_val => js_val,
                        has_js_val => true,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(json_to_js).collect();
                let rendered = crate::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        field_is_optional => field_is_optional,
                        field_is_array => field_is_array,
                        is_string_val => values.iter().all(|v| v.is_string()),
                        values_js => items,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let rendered = crate::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        field_is_array => field_is_array,
                        is_string_val => expected.is_string(),
                        js_val => js_val,
                        has_js_val => true,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_empty" => {
            let rendered = crate::template_env::render(
                "typescript/assertion.jinja",
                minijinja::context! {
                    assertion_type => assertion_type,
                    field_expr => field_expr,
                    field_is_optional => field_is_optional,
                },
            );
            out.push_str(&rendered);
        }
        "is_empty" => {
            let rendered = crate::template_env::render(
                "typescript/assertion.jinja",
                minijinja::context! {
                    assertion_type => assertion_type,
                    field_expr => field_expr,
                },
            );
            out.push_str(&rendered);
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(json_to_js).collect();
                let rendered = crate::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        field_is_array => field_is_array,
                        is_string_val => values.iter().all(|v| v.is_string()),
                        values_js => items,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let rendered = crate::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        js_val => js_val,
                        has_js_val => true,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let rendered = crate::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        field_is_optional => field_is_optional,
                        js_val => js_val,
                        has_js_val => true,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "count_min" | "count_equals" | "min_length" | "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let rendered = crate::template_env::render(
                        "typescript/assertion.jinja",
                        minijinja::context! {
                            assertion_type => assertion_type,
                            field_expr => field_expr,
                            n => n,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
        }
        "is_true" => {
            let rendered = crate::template_env::render(
                "typescript/assertion.jinja",
                minijinja::context! {
                    assertion_type => assertion_type,
                    field_expr => field_expr,
                },
            );
            out.push_str(&rendered);
        }
        "is_false" => {
            let rendered = crate::template_env::render(
                "typescript/assertion.jinja",
                minijinja::context! {
                    assertion_type => assertion_type,
                    field_expr => field_expr,
                },
            );
            out.push_str(&rendered);
        }
        "method_result" => {
            render_method_result_assertion(out, assertion, result_var);
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let rendered = crate::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        js_val => js_val,
                        has_js_val => true,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                if let Some(pattern) = expected.as_str() {
                    let rendered = crate::template_env::render(
                        "typescript/assertion.jinja",
                        minijinja::context! {
                            assertion_type => assertion_type,
                            field_expr => field_expr,
                            expected_pattern => pattern,
                        },
                    );
                    out.push_str(&rendered);
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
                    let rendered = crate::template_env::render(
                        "typescript/assertion.jinja",
                        minijinja::context! {
                            assertion_type => "method_result",
                            check => check,
                            call_expr => call_expr,
                            check_js_val => js_val,
                            has_check_js_val => true,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
            "is_true" => {
                let rendered = crate::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "method_result",
                        check => check,
                        call_expr => call_expr,
                    },
                );
                out.push_str(&rendered);
            }
            "is_false" => {
                let rendered = crate::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "method_result",
                        check => check,
                        call_expr => call_expr,
                    },
                );
                out.push_str(&rendered);
            }
            "greater_than_or_equal" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    let rendered = crate::template_env::render(
                        "typescript/assertion.jinja",
                        minijinja::context! {
                            assertion_type => "method_result",
                            check => check,
                            call_expr => call_expr,
                            check_n => n,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
            "count_min" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    let rendered = crate::template_env::render(
                        "typescript/assertion.jinja",
                        minijinja::context! {
                            assertion_type => "method_result",
                            check => check,
                            call_expr => call_expr,
                            check_n => n,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
            "contains" => {
                if let Some(val) = &assertion.value {
                    let js_val = json_to_js(val);
                    let rendered = crate::template_env::render(
                        "typescript/assertion.jinja",
                        minijinja::context! {
                            assertion_type => "method_result",
                            check => check,
                            call_expr => call_expr,
                            check_js_val => js_val,
                            has_check_js_val => true,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
            "is_error" => {
                let rendered = crate::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "method_result",
                        check => check,
                        call_expr => call_expr,
                    },
                );
                out.push_str(&rendered);
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
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    fn array_resolver(field: &str) -> FieldResolver {
        let result_fields = HashSet::from([field.to_string()]);
        let array_fields = HashSet::from([field.to_string()]);
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &array_fields,
            &HashSet::new(),
        )
    }

    fn make_assertion(assertion_type: &str, field: Option<&str>, value: Option<serde_json::Value>) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: field.map(|s| s.to_string()),
            value,
            ..Default::default()
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
