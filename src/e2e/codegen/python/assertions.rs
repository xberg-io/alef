//! Assertion rendering for Python e2e tests.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::field_skip::{FieldSkip, nested_wildcard_skip_line};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::helpers::strip_redundant_call_arg_parens;
use super::json::{python_string_literal, value_to_python_string};

mod chunks_synthetic;

/// Render a single assertion into the test function body.
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    fields_enum: &HashSet<String>,
    assert_enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
) {
    // When result_is_simple, skip fields that reference struct sub-fields.
    if result_is_simple && let Some(f) = &assertion.field {
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
            let _ = writeln!(
                out,
                "    # skipped: {}",
                FieldSkip::NotApplicableForSimpleResultType.message(f)
            );
            return;
        }
    }

    // Handle synthetic / derived fields.
    if let Some(f) = &assertion.field {
        if let Some(reason) = crate::e2e::codegen::assertion_recipes::chunks_synthetic_skip_reason(f, field_resolver) {
            let _ = writeln!(out, "    # skipped: {reason}");
            return;
        }
        if render_synthetic_field(out, assertion, result_var, f, field_resolver) {
            return;
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        let _ = writeln!(out, "    # skipped: {}", FieldSkip::NotAvailableOnResultType.message(f));
        return;
    }

    // A `foo[].bar` fixture path means EVERY element of `foo`. The shared accessor lowers
    // `[]` to `[0]`, so the wildcard must be expanded into an `any(..)` comprehension
    // before the accessor is built. ~keep
    if !result_is_simple
        && let Some(f) = assertion.field.as_deref()
        && !f.is_empty()
        && let Some((array_part, elem_part)) = field_resolver.wildcard_split(f)
    {
        render_python_wildcard_assertion(out, assertion, f, &array_part, &elem_part, result_var, field_resolver);
        return;
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
        // Per-call assertion override wins over the global set.
        if assert_enum_fields.contains_key(f) {
            return true;
        }
        if fields_enum.contains(f) {
            return true;
        }
        let resolved = field_resolver.resolve(f);
        if fields_enum.contains(resolved) {
            return true;
        }
        // Neither the explicit config nor the per-call override named this field. Fall back
        // to the IR-derived classification (`with_ir_enum_map`, anchored at the call's
        // declared Rust return type via `resolve_declared_result_type`) so a consumer that
        // never configured `fields_enum` still gets a correct classification instead of the
        // dynamically-typed default of "compare as a plain string" — which asserts the wire
        // value against the Python enum's `repr`/member name instead of coercing it first.
        // This is purely additive: it only turns a `false` into a `true`. ~keep
        // An indexed accessor says nothing about its element type. Unknown helper return
        // types must retain exact comparisons unless config explicitly declares an enum.
        field_resolver.is_enum(f)
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

fn render_python_wildcard_assertion(
    out: &mut String,
    assertion: &Assertion,
    field: &str,
    array_part: &str,
    elem_part: &str,
    result_var: &str,
    field_resolver: &FieldResolver,
) {
    // `wildcard_split` consumes the first `[].` only, so a doubly-nested path leaves a second
    // wildcard in `elem_part` that the element accessor below would lower to index 0. ~keep
    if let Some(line) = nested_wildcard_skip_line("    ", "#", field, elem_part) {
        let _ = writeln!(out, "{line}");
        return;
    }
    let array_accessor = if array_part.is_empty() {
        result_var.to_string()
    } else {
        field_resolver.accessor(array_part, "python", result_var)
    };
    // `python_element_accessor`, not `accessor`: the path is already element-relative, so the
    // result-anchoring `accessor` applies would re-prefix it with the container. It also isn't
    // the generic cross-language `element_accessor`: that anchors Python's TypedDict-vs-attribute
    // owner cursor at the call's RESULT type, which answers a different question than "is the
    // ELEMENT type a TypedDict" whenever a result envelope and its collection elements classify
    // differently (subscript at the container level, attribute at the element level, or vice
    // versa) -- `python_element_accessor` takes `array_part` so it can resolve the actual element
    // owner type instead. ~keep
    let elem_accessor = if elem_part.is_empty() {
        "_e".to_string()
    } else {
        field_resolver.python_element_accessor(elem_part, array_part, "_e")
    };
    let iterable = format!("({array_accessor} or [])");
    // `str({elem_accessor})` below is call-argument position: a narrowing crossing in
    // `elem_accessor` (`(x if y else None)`) carries its own load-bearing parens elsewhere, but
    // is redundant once already wrapped in `str(...)`'s own delimiters. `elem_accessor` itself
    // stays unstripped for the non-call-argument uses this function doesn't have today but a
    // future arm might add. ~keep
    let elem_accessor_arg = strip_redundant_call_arg_parens(&elem_accessor);

    match assertion.assertion_type.as_str() {
        "contains" | "contains_all" | "not_contains" => {
            let negate = assertion.assertion_type == "not_contains";
            let values: Vec<&serde_json::Value> = if assertion.assertion_type == "contains" {
                assertion.value.iter().collect()
            } else {
                assertion.expected_values()
            };
            for val in values {
                let expected = value_to_python_string(val);
                // `in str(..)` needs a str left operand; non-string fixture values
                // (numbers, bools) have to be stringified first. ~keep
                let needle = if val.is_string() {
                    expected
                } else {
                    format!("str({expected})")
                };
                let pred = format!("any({needle} in str({elem_accessor_arg}) for _e in {iterable})");
                if negate {
                    let _ = writeln!(out, "    assert not {pred}");
                } else {
                    let _ = writeln!(out, "    assert {pred}");
                }
            }
        }
        "not_empty" => {
            let pred = format!("any(str({elem_accessor_arg}) != \"\" for _e in {iterable})");
            let _ = writeln!(out, "    assert {pred}");
        }
        other => {
            let _ = writeln!(
                out,
                "    # skipped: unsupported traversal assertion '{other}' on '{field}'"
            );
        }
    }
}

fn render_synthetic_field(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field: &str,
    field_resolver: &FieldResolver,
) -> bool {
    match field {
        _ if chunks_synthetic::try_render(out, assertion, result_var, field, field_resolver) => true,
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
                "    # skipped: {}",
                FieldSkip::NotAvailableOnPythonProcessingResult.message(field)
            );
            true
        }
        _ => false,
    }
}

fn emit_bool_assertion(out: &mut String, pred: &str, assertion_type: &str, field: &str) {
    match assertion_type {
        "is_true" => {
            let _ = writeln!(out, "    assert {pred}");
        }
        "is_false" => {
            let _ = writeln!(out, "    assert not ({pred})");
        }
        other => {
            panic!("Python e2e generator: unsupported assertion type '{other}' on synthetic field '{field}'");
        }
    }
}

fn render_embeddings_assertion(out: &mut String, assertion: &Assertion, result_var: &str) {
    match assertion.assertion_type.as_str() {
        "count_equals" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    assert len({result_var}) == {n}");
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    assert len({result_var}) >= {n}");
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "    assert len({result_var}) > 0");
        }
        "is_empty" => {
            let _ = writeln!(out, "    assert len({result_var}) == 0");
        }
        other => {
            panic!("Python e2e generator: unsupported assertion type '{other}' on synthetic field 'embeddings'");
        }
    }
}

fn render_embedding_dimensions(out: &mut String, assertion: &Assertion, result_var: &str) {
    let expr = format!("(len({result_var}[0]) if {result_var} else 0)");
    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(val) = &assertion.value {
                let py_val = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expr} == {py_val}");
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let py_val = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expr} > {py_val}");
            }
        }
        other => {
            panic!(
                "Python e2e generator: unsupported assertion type '{other}' on synthetic field 'embedding_dimensions'"
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
        "contains_all" => {
            if let Some(values) = &assertion.values {
                let values_list: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let expected = value_to_python_string(v);
                        python_contains_expr(field_access, &expected, field_is_enum, field_is_array, v.is_string())
                    })
                    .collect();
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains_all",
                        field_access => field_access,
                        field_is_optional => field_is_optional,
                        values_list => values_list,
                    },
                );
                out.push_str(&rendered);
            }
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
                    // `str({field_access})` is call-argument position (the sole content between
                    // `str(...)`'s own parens): a narrowing crossing's own enclosing parens are
                    // redundant there, exactly as `strip_redundant_call_arg_parens` already
                    // handles for `len(...)`/`hasattr(...)`. Every OTHER use of `field_access`
                    // in this match (the `for item in {field_access}` / `v in {field_access}`
                    // arms above and below) keeps it unstripped: those parens are load-bearing,
                    // not call-argument position. ~keep
                    let field_access_arg = strip_redundant_call_arg_parens(field_access);
                    format!("any(v.lower() in str({field_access_arg}).lower() for v in [{list_str}])")
                } else {
                    format!("any(v in {field_access} for v in [{list_str}])")
                };
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains_any",
                        field_access => field_access,
                        field_is_optional => field_is_optional,
                        cmp_expr_any => cmp_expr,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "method_result" => {
            render_method_result(out, assertion, result_var);
        }
        "equals" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let op = if val.is_boolean() || val.is_null() { "is" } else { "==" };
                // `field_access_arg`: the enum branch below wraps `field_access` as the sole
                // argument of `_alef_e2e_text(...)` -- call-argument position, same shape as
                // `len(...)`/`hasattr(...)` above. Harmless to compute even when `is_enum` is
                // false: the template only reads it on the branch that needs it.
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "equals",
                        field_access => field_access,
                        field_access_arg => strip_redundant_call_arg_parens(field_access),
                        field_is_optional => field_is_optional,
                        is_enum => field_is_enum,
                        expected_val => expected,
                        op => op,
                        is_string_val => val.is_string(),
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let cmp_expr =
                    python_contains_expr(field_access, &expected, field_is_enum, field_is_array, val.is_string());
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains",
                        field_access => field_access,
                        field_is_optional => field_is_optional,
                        cmp_expr => cmp_expr,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_contains" => {
            for val in assertion.expected_values() {
                let expected = value_to_python_string(val);
                let cmp_expr =
                    python_contains_expr(field_access, &expected, field_is_enum, field_is_array, val.is_string());
                let negated_cmp_expr = negate_contains_expr(&cmp_expr, field_is_array, field_is_enum);
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "not_contains",
                        field_access => field_access,
                        field_is_optional => field_is_optional,
                        negated_cmp_expr => negated_cmp_expr,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_empty" => {
            let rendered = crate::e2e::template_env::render(
                "python/assertion.jinja",
                minijinja::context! {
                    assertion_type => "not_empty",
                    field_access => field_access,
                    field_access_arg => strip_redundant_call_arg_parens(field_access),
                },
            );
            out.push_str(&rendered);
        }
        "is_empty" => {
            let rendered = crate::e2e::template_env::render(
                "python/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_empty",
                    field_access => field_access,
                },
            );
            out.push_str(&rendered);
        }
        "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" | "min" | "max" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion.assertion_type.as_str(),
                        field_access => field_access,
                        expected_val => expected,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "starts_with" | "ends_with" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion.assertion_type.as_str(),
                        field_access => field_access,
                        expected_val => expected,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "min_length" | "max_length" | "count_min" | "count_equals" => {
            if let Some(val) = &assertion.value {
                let n = val.as_u64().unwrap_or(0);
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion.assertion_type.as_str(),
                        field_access => field_access,
                        field_access_arg => strip_redundant_call_arg_parens(field_access),
                        n => n,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "is_true" => {
            let rendered = crate::e2e::template_env::render(
                "python/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_true",
                    field_access => field_access,
                    field_is_optional => field_is_optional,
                },
            );
            out.push_str(&rendered);
        }
        "is_false" => {
            let rendered = crate::e2e::template_env::render(
                "python/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_false",
                    field_access => field_access,
                    field_is_optional => field_is_optional,
                },
            );
            out.push_str(&rendered);
        }
        "matches_regex" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "matches_regex",
                        field_access => field_access,
                        expected_val => expected,
                    },
                );
                out.push_str(&rendered);
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
        // `str({field_access})` is call-argument position — see the identical fix and rationale
        // for the `"contains_any"` arm in `render_standard_assertion`, which this mirrors for
        // `contains_all`/`contains`/`not_contains`. ~keep
        let field_access_arg = strip_redundant_call_arg_parens(field_access);
        return format!("{expected}.lower() in str({field_access_arg}).lower()");
    }
    format!("{expected} in {field_access}")
}

/// Negate a comparison expression for use in `not_contains` assertions.
/// For simple membership tests (e.g., `x in y`), emits `x not in y` directly
/// instead of wrapping with `not (...)` to avoid ruff E713 flip-flop issues.
fn negate_contains_expr(cmp_expr: &str, field_is_array: bool, field_is_enum: bool) -> String {
    // Simple membership test: `expected in field_access` → `expected not in field_access`
    if !field_is_array && !field_is_enum && cmp_expr.contains(" in ") && !cmp_expr.contains("not in") {
        return cmp_expr.replace(" in ", " not in ");
    }

    // For complex expressions (any(...), lower() calls, etc.), wrap with `not (...)`
    format!("not ({cmp_expr})")
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
                            let _ = writeln!(out, "    assert {call_expr} is True");
                        } else {
                            let _ = writeln!(out, "    assert {call_expr} is False");
                        }
                    } else {
                        let expected = value_to_python_string(val);
                        let _ = writeln!(out, "    assert {call_expr} == {expected}");
                    }
                }
            }
            "is_true" => {
                let _ = writeln!(out, "    assert {call_expr}");
            }
            "is_false" => {
                let _ = writeln!(out, "    assert not {call_expr}");
            }
            "greater_than_or_equal" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    let _ = writeln!(out, "    assert {call_expr} >= {n}");
                }
            }
            "count_min" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    let _ = writeln!(out, "    assert len({call_expr}) >= {n}");
                }
            }
            "contains" => {
                if let Some(val) = &assertion.value {
                    let expected = value_to_python_string(val);
                    let _ = writeln!(out, "    assert {expected} in {call_expr}");
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
#[path = "assertions/tests.rs"]
mod tests;
