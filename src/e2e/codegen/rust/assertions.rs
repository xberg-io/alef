//! Assertion rendering for Rust e2e tests.

use std::fmt::Write as FmtWrite;

use crate::e2e::escape::escape_rust;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::assertion_helpers::{
    render_count_equals_assertion, render_count_min_assertion, render_equals_assertion, render_gte_assertion,
    render_is_empty_assertion, render_method_result_assertion, render_not_empty_assertion,
};
use super::assertion_synthetic::{
    numeric_literal, render_chunks_have_content, render_chunks_have_embeddings, render_chunks_have_heading_context,
    render_embedding_dimensions, render_embedding_quality, render_embeddings_assertion,
    render_first_chunk_starts_with_heading, render_keywords_assertion, render_keywords_count_assertion,
    tree_field_access_expr, value_to_rust_string,
};

/// Returns `true` when the assertion's leaf field resolves to an `Option<T>` where
/// `T` is a scalar (i.e. not a collection). Used to decide whether numeric comparison
/// operators (`>`, `<`, `>=`, `<=`) need to unwrap the field before comparing — directly
/// comparing `Option<usize>` against a numeric literal is a type error.
fn is_optional_scalar_field(assertion: &Assertion, is_unwrapped: bool, field_resolver: &FieldResolver) -> bool {
    assertion.field.as_ref().is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        let is_opt = !is_unwrapped && field_resolver.is_optional(resolved);
        let is_arr = field_resolver.is_array(resolved);
        is_opt && !is_arr
    })
}

/// Render a single assertion into the test function body.
#[allow(clippy::too_many_arguments)]
pub fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    module: &str,
    dep_name: &str,
    is_error_context: bool,
    unwrapped_fields: &[(String, String)], // (fixture_field, local_var)
    field_resolver: &FieldResolver,
    result_is_tree: bool,
    result_is_simple: bool,
    result_is_vec: bool,
    result_is_option: bool,
    returns_result: bool,
    streaming_item_type: Option<&str>,
) {
    render_assertion_with_streaming(
        out,
        assertion,
        result_var,
        module,
        dep_name,
        is_error_context,
        unwrapped_fields,
        field_resolver,
        result_is_tree,
        result_is_simple,
        result_is_vec,
        result_is_option,
        returns_result,
        streaming_item_type,
        false,
    )
}

/// Same as [`render_assertion`], but with an `is_streaming` flag so the streaming-virtual
/// field arm can fire when `result_var` is the raw call result rather than the collected
/// `chunks` variable.  Callers that already drained the stream into a `chunks: Vec<_>`
/// local should pass `is_streaming = true`.
#[allow(clippy::too_many_arguments)]
pub fn render_assertion_with_streaming(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    module: &str,
    dep_name: &str,
    is_error_context: bool,
    unwrapped_fields: &[(String, String)], // (fixture_field, local_var)
    field_resolver: &FieldResolver,
    result_is_tree: bool,
    result_is_simple: bool,
    result_is_vec: bool,
    result_is_option: bool,
    returns_result: bool,
    streaming_item_type: Option<&str>,
    _is_streaming: bool,
) {
    // Vec<T> result: iterate per-element so each assertion checks every element.
    // Field-path assertions become `for r in &{result} { <assert using r> }`.
    // Length-style assertions on the Vec itself (no field path) operate on the
    // Vec directly.
    let has_field = assertion.field.as_ref().is_some_and(|f| !f.is_empty());
    if result_is_vec && has_field && !is_error_context {
        let _ = writeln!(out, "    for r in &{result_var} {{");
        render_assertion(
            out,
            assertion,
            "r",
            module,
            dep_name,
            is_error_context,
            unwrapped_fields,
            field_resolver,
            result_is_tree,
            result_is_simple,
            false, // already inside loop
            result_is_option,
            returns_result,
            streaming_item_type,
        );
        let _ = writeln!(out, "    }}");
        return;
    }
    // Option<T> result: map `is_empty`/`not_empty` to `is_none()`/`is_some()`,
    // and unwrap the inner value before any other assertion runs.
    if result_is_option && !is_error_context {
        let assertion_type = assertion.assertion_type.as_str();
        if !has_field && (assertion_type == "is_empty" || assertion_type == "not_empty") {
            let check = if assertion_type == "is_empty" {
                "is_none"
            } else {
                "is_some"
            };
            let _ = writeln!(
                out,
                "    assert!({result_var}.{check}(), \"expected Option to be {check}\");"
            );
            return;
        }
        // For any other assertion shape, unwrap the Option and recurse with a
        // bare reference variable so the rest of the renderer treats the inner
        // value as the result.
        let _ = writeln!(
            out,
            "    let r = {result_var}.as_ref().expect(\"Option<T> should be Some\");"
        );
        render_assertion(
            out,
            assertion,
            "r",
            module,
            dep_name,
            is_error_context,
            unwrapped_fields,
            field_resolver,
            result_is_tree,
            result_is_simple,
            result_is_vec,
            false, // already unwrapped
            returns_result,
            streaming_item_type,
        );
        return;
    }
    // Handle synthetic fields like chunks_have_content (derived assertions).
    // These are computed expressions, not real struct fields — intercept before
    // the is_valid_for_result check so they are never treated as field accesses.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                render_chunks_have_content(out, result_var, assertion.assertion_type.as_str());
                return;
            }
            "chunks_have_embeddings" => {
                render_chunks_have_embeddings(out, result_var, assertion.assertion_type.as_str());
                return;
            }
            "chunks_have_heading_context" => {
                render_chunks_have_heading_context(out, result_var, assertion.assertion_type.as_str());
                return;
            }
            "first_chunk_starts_with_heading" => {
                render_first_chunk_starts_with_heading(out, result_var, assertion.assertion_type.as_str());
                return;
            }
            "embeddings" => {
                render_embeddings_assertion(out, result_var, assertion);
                return;
            }
            "embedding_dimensions" => {
                render_embedding_dimensions(out, result_var, assertion);
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                render_embedding_quality(out, result_var, f, assertion.assertion_type.as_str());
                return;
            }
            "keywords" => {
                render_keywords_assertion(out, result_var, assertion);
                return;
            }
            "keywords_count" => {
                render_keywords_count_assertion(out, result_var, assertion);
                return;
            }
            _ => {}
        }
    }

    // Streaming virtual fields: intercept before is_valid_for_result so they are
    // never skipped.  These fields resolve against the `chunks` collected-list variable.
    //
    // For streaming fixtures, `chunks` is bound by the collect snippet emitted in
    // `render_test_function`.  For non-streaming fixtures whose result struct has a
    // literal field whose name collides with a streaming-virtual name (e.g. `chunks`,
    // `imports`, `structure`), `render_test_function` emits `let {f} = &result.{f};`
    // before assertions, so the hardcoded `chunks` identifier used below still resolves.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
            if let Some(expr) =
                crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor_with_streaming_context(
                    f,
                    "rust",
                    "chunks",
                    Some(dep_name),
                    streaming_item_type,
                )
            {
                match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            if let Some(n) = val.as_u64() {
                                let expr_for_len = if field_resolver.is_optional(f) {
                                    format!("{expr}.as_ref().map_or(0, |v| v.len())")
                                } else {
                                    format!("{expr}.len()")
                                };
                                let _ = writeln!(
                                    out,
                                    "    assert!({expr_for_len} >= {n} as usize, \"expected >= {n} chunks\");"
                                );
                            }
                        }
                    }
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            if let Some(n) = val.as_u64() {
                                let expr_for_len = if field_resolver.is_optional(f) {
                                    format!("{expr}.as_ref().map_or(0, |v| v.len())")
                                } else {
                                    format!("{expr}.len()")
                                };
                                let _ = writeln!(
                                    out,
                                    "    assert_eq!({expr_for_len}, {n} as usize, \"expected exactly {n} chunks\");"
                                );
                            }
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = crate::e2e::escape::escape_rust(s);
                            let _ = writeln!(out, "    assert_eq!({expr}, \"{escaped}\");");
                        } else if let Some(val) = &assertion.value {
                            let lit = super::assertion_synthetic::numeric_literal(val);
                            let _ = writeln!(out, "    assert_eq!({expr}, {lit});");
                        }
                    }
                    "not_empty" => {
                        let check_expr = if field_resolver.is_optional(f) {
                            format!("{expr}.as_ref().is_some_and(|v| !v.is_empty())")
                        } else {
                            format!("!{expr}.is_empty()")
                        };
                        let _ = writeln!(out, "    assert!({check_expr}, \"expected non-empty\");");
                    }
                    "is_empty" => {
                        let check_expr = if field_resolver.is_optional(f) {
                            format!("{expr}.as_ref().is_none_or(|v| v.is_empty())")
                        } else {
                            format!("{expr}.is_empty()")
                        };
                        let _ = writeln!(out, "    assert!({check_expr}, \"expected empty\");");
                    }
                    "is_true" => {
                        let _ = writeln!(out, "    assert!({expr}, \"expected true\");");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    assert!(!{expr}, \"expected false\");");
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value {
                            let lit = super::assertion_synthetic::numeric_literal(val);
                            let _ = writeln!(out, "    assert!({expr} > {lit}, \"expected > {lit}\");");
                        }
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let lit = super::assertion_synthetic::numeric_literal(val);
                            let _ = writeln!(out, "    assert!({expr} >= {lit}, \"expected >= {lit}\");");
                        }
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = crate::e2e::escape::escape_rust(s);
                            let _ = writeln!(
                                out,
                                "    assert!({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\");"
                            );
                        }
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "    // streaming field '{f}': assertion type '{}' not rendered",
                            assertion.assertion_type
                        );
                    }
                }
            }
            return;
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    // Exception: fields prefixed with "error." target the error value in error-context
    // assertions — they are resolved against the error type via accessor_for_error,
    // not against the success result type, so they must not be skipped here.
    // However, when NOT in error context (i.e. the call site uses .expect() and binds
    // the Ok value), there is no Err to inspect — skip error.* assertions with a comment.
    if let Some(f) = &assertion.field {
        if !f.is_empty() {
            if f.starts_with("error.") && !is_error_context {
                let _ = writeln!(out, "    // skipped: field '{f}' not available on result type");
                return;
            }
            // When result_is_simple the function returns a plain scalar/string type —
            // `field_access` uses `effective_result_var` directly regardless of the
            // field name, so the skip guard must not fire for these calls.
            if !f.starts_with("error.") && !result_is_simple && !field_resolver.is_valid_for_result(f) {
                let _ = writeln!(out, "    // skipped: field '{f}' not available on result type");
                return;
            }
        }
    }

    // Check if this field was unwrapped (i.e., it is optional and was bound to a local).
    let is_unwrapped = assertion
        .field
        .as_ref()
        .is_some_and(|f| unwrapped_fields.iter().any(|(ff, _)| ff == f));

    // When in error context with returns_result=true and accessing a field (not an error check),
    // we need to unwrap the Result first. The test generator creates a binding like
    // `let result_ok = result.as_ref().ok();` which we can dereference here.
    // Exception: fields prefixed with "error." access the Err value, not the Ok value.
    let has_field = assertion.field.as_ref().is_some_and(|f| !f.is_empty());
    let is_field_assertion = !matches!(assertion.assertion_type.as_str(), "error" | "not_error");
    let is_error_field = assertion.field.as_ref().is_some_and(|f| f.starts_with("error."));
    let effective_result_var =
        if has_field && is_error_context && returns_result && is_field_assertion && !is_error_field {
            // Dereference the Option<&T> bound as {result_var}_ok
            format!("{result_var}_ok.as_ref().unwrap()")
        } else {
            result_var.to_string()
        };

    // Determine field access expression:
    // 1. If the field was unwrapped to a local var, use that local var name.
    // 2. When result_is_simple, the function returns a plain type (String etc.) — use result_var.
    // 3. When the field path is exactly the result var name (sentinel: `field: "result"`),
    //    refer to the result variable directly to avoid emitting `result.result`.
    // 4. When the result is a Tree, map pseudo-field names to correct Rust expressions.
    // 5. When the field starts with "error.", resolve against the error type.
    // 6. Otherwise, use the field resolver to generate the accessor.
    let field_access = match &assertion.field {
        Some(f) if !f.is_empty() => {
            if let Some((_, local_var)) = unwrapped_fields.iter().find(|(ff, _)| ff == f) {
                local_var.clone()
            } else if result_is_simple && !f.starts_with("error.") {
                // Plain return type (String, Vec<T>, etc.) has no struct fields.
                // Use the result variable directly so assertions operate on the value itself.
                // Exception: error.* fields must resolve against the Err value, not the
                // plain result variable, even when the success type is simple (e.g. Bytes).
                effective_result_var.clone()
            } else if f == result_var {
                // Sentinel: fixture uses `field: "result"` (or matches the result variable name)
                // to refer to the whole return value, not a struct field named "result".
                effective_result_var.clone()
            } else if result_is_tree {
                // Tree is an opaque type — its "fields" are accessed via root_node() or
                // free functions. Map known pseudo-field names to correct Rust expressions.
                tree_field_access_expr(f, &effective_result_var, module)
            } else if let Some(sub) = f.strip_prefix("error.") {
                // Error-path field: access a field on the Err value rather than the Ok value.
                // Inline-bind the error so the expression is self-contained.
                let err_accessor = field_resolver.accessor_for_error(sub, "rust", "__err");
                format!("{{ let __err = {result_var}.as_ref().err().unwrap(); {err_accessor} }}")
            } else {
                field_resolver.accessor(f, "rust", &effective_result_var)
            }
        }
        _ => effective_result_var,
    };

    match assertion.assertion_type.as_str() {
        "error" => {
            let _ = writeln!(out, "    assert!({result_var}.is_err(), \"expected call to fail\");");
            if let Some(serde_json::Value::String(msg)) = &assertion.value {
                let escaped = escape_rust(msg);
                // Match against the Debug format (variant-name-style) and the Display format
                // (human-readable text). Fixtures often name the error variant ("BadRequest"),
                // but Display impls typically lowercase with a colon ("bad request: ..."), so
                // checking both lets either kind of fixture value match.
                let _ = writeln!(
                    out,
                    "    {{ let __e = {result_var}.as_ref().err().unwrap(); assert!(format!(\"{{:?}}\", __e).contains(\"{escaped}\") || __e.to_string().contains(\"{escaped}\"), \"error message mismatch\"); }}"
                );
            }
        }
        "not_error" => {
            // Handled at call site; nothing extra needed here.
        }
        "equals" => {
            render_equals_assertion(out, assertion, &field_access, is_unwrapped, field_resolver);
        }
        "contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_rust_string(val);
                let line = format!(
                    "    assert!(format!(\"{{:?}}\", {field_access}).contains({expected}), \"expected to contain: {{}}\", {expected});"
                );
                let _ = writeln!(out, "{line}");
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let expected = value_to_rust_string(val);
                    let line = format!(
                        "    assert!(format!(\"{{:?}}\", {field_access}).contains({expected}), \"expected to contain: {{}}\", {expected});"
                    );
                    let _ = writeln!(out, "{line}");
                }
            }
        }
        "not_contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_rust_string(val);
                let line = format!(
                    "    assert!(!format!(\"{{:?}}\", {field_access}).contains({expected}), \"expected NOT to contain: {{}}\", {expected});"
                );
                let _ = writeln!(out, "{line}");
            }
        }
        "not_empty" => {
            render_not_empty_assertion(
                out,
                assertion,
                &field_access,
                result_var,
                result_is_option,
                is_unwrapped,
                field_resolver,
            );
        }
        "is_empty" => {
            render_is_empty_assertion(out, assertion, &field_access, is_unwrapped, field_resolver);
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let expected = value_to_rust_string(v);
                        format!("{field_access}.contains({expected})")
                    })
                    .collect();
                let joined = checks.join(" || ");
                let _ = writeln!(
                    out,
                    "    assert!({joined}, \"expected to contain at least one of the specified values\");"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                // Skip comparisons with negative values against unsigned types (.len() etc.)
                if val.as_f64().is_some_and(|n| n < 0.0) {
                    let _ = writeln!(
                        out,
                        "    // skipped: greater_than with negative value is always true for unsigned types"
                    );
                } else if val.as_u64() == Some(0) {
                    if field_access.ends_with(".len()") {
                        // Clippy prefers !is_empty() over len() > 0 for collections.
                        let base = field_access.strip_suffix(".len()").unwrap();
                        let _ = writeln!(out, "    assert!(!{base}.is_empty(), \"expected > 0\");");
                    } else if is_optional_scalar_field(assertion, is_unwrapped, field_resolver) {
                        // Use 0 for integer comparisons (the common case for > 0).
                        let _ = writeln!(out, "    assert!({field_access}.unwrap_or(0) > 0, \"expected > 0\");");
                    } else {
                        // Scalar types (usize, u64, etc.) — use direct comparison.
                        let _ = writeln!(out, "    assert!({field_access} > 0, \"expected > 0\");");
                    }
                } else {
                    let lit = numeric_literal(val);
                    if is_optional_scalar_field(assertion, is_unwrapped, field_resolver) {
                        // Option<usize>/Option<u64>/Option<f64>: unwrap with appropriate zero literal
                        // before comparing so the assertion fails (rather than fails to compile) on a missing field.
                        let default_literal = if lit.contains("_f64") || lit.contains('.') {
                            "0.0"
                        } else {
                            "0"
                        };
                        let _ = writeln!(
                            out,
                            "    assert!({field_access}.unwrap_or({default_literal}) > {lit}, \"expected > {lit}\");"
                        );
                    } else {
                        let _ = writeln!(out, "    assert!({field_access} > {lit}, \"expected > {lit}\");");
                    }
                }
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                if is_optional_scalar_field(assertion, is_unwrapped, field_resolver) {
                    // Option<usize>/Option<u64>/Option<f64>: unwrap with appropriate zero literal
                    // before comparing. Note this means a missing field will satisfy `< N` for any positive N,
                    // matching the convention used by render_gte_assertion.
                    let default_literal = if lit.contains("_f64") || lit.contains('.') {
                        "0.0"
                    } else {
                        "0"
                    };
                    let _ = writeln!(
                        out,
                        "    assert!({field_access}.unwrap_or({default_literal}) < {lit}, \"expected < {lit}\");"
                    );
                } else {
                    let _ = writeln!(out, "    assert!({field_access} < {lit}, \"expected < {lit}\");");
                }
            }
        }
        "greater_than_or_equal" => {
            render_gte_assertion(out, assertion, &field_access, is_unwrapped, field_resolver);
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                if is_optional_scalar_field(assertion, is_unwrapped, field_resolver) {
                    // Option<usize>/Option<u64>/Option<f64>: unwrap with appropriate zero literal.
                    let default_literal = if lit.contains("_f64") || lit.contains('.') {
                        "0.0"
                    } else {
                        "0"
                    };
                    let _ = writeln!(
                        out,
                        "    assert!({field_access}.unwrap_or({default_literal}) <= {lit}, \"expected <= {lit}\");"
                    );
                } else {
                    let _ = writeln!(out, "    assert!({field_access} <= {lit}, \"expected <= {lit}\");");
                }
            }
        }
        "starts_with" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_rust_string(val);
                let _ = writeln!(
                    out,
                    "    assert!({field_access}.starts_with({expected}), \"expected to start with: {{}}\", {expected});"
                );
            }
        }
        "ends_with" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_rust_string(val);
                let _ = writeln!(
                    out,
                    "    assert!({field_access}.ends_with({expected}), \"expected to end with: {{}}\", {expected});"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if n == 1 {
                        // Clippy prefers !is_empty() over len() >= 1 for collections.
                        let _ = writeln!(
                            out,
                            "    assert!(!{field_access}.is_empty(), \"expected length >= 1, got {{}}\", {field_access}.len());"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert!({field_access}.len() >= {n}, \"expected length >= {n}, got {{}}\", {field_access}.len());"
                        );
                    }
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    assert!({field_access}.len() <= {n}, \"expected length <= {n}, got {{}}\", {field_access}.len());"
                    );
                }
            }
        }
        "count_min" => {
            render_count_min_assertion(out, assertion, &field_access, is_unwrapped, field_resolver);
        }
        "count_equals" => {
            render_count_equals_assertion(out, assertion, &field_access, is_unwrapped, field_resolver);
        }
        "is_true" => {
            if is_optional_scalar_field(assertion, is_unwrapped, field_resolver) {
                // Option<T>: "is_true" semantically means "present and truthy".
                // For `Option<bool>` that's `Some(true)`; for `Option<serde_json::Value>`
                // (e.g. interact action_results[0].data) it's "Some and not null/false".
                // `is_some()` is the broadest correct interpretation that compiles for any T.
                let _ = writeln!(out, "    assert!({field_access}.is_some(), \"expected true (Some)\");");
            } else {
                let _ = writeln!(out, "    assert!({field_access}, \"expected true\");");
            }
        }
        "is_false" => {
            if is_optional_scalar_field(assertion, is_unwrapped, field_resolver) {
                // Option<T>: "is_false" semantically means "absent or falsy" — `.is_none()`
                // is the safe interpretation that compiles uniformly.
                let _ = writeln!(out, "    assert!({field_access}.is_none(), \"expected false (None)\");");
            } else {
                let _ = writeln!(out, "    assert!(!{field_access}, \"expected false\");");
            }
        }
        "method_result" => {
            render_method_result_assertion(out, assertion, &field_access, result_is_tree, module);
        }
        other => {
            panic!("Rust e2e generator: unsupported assertion type: {other}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;

    fn empty_resolver() -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
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
    fn render_assertion_error_type_emits_is_err_check() {
        let resolver = empty_resolver();
        let assertion = make_assertion("error", None, None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            true,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(out.contains("is_err()"), "got: {out}");
    }

    #[test]
    fn render_assertion_vec_result_wraps_in_for_loop() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_empty", Some("content"), None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            true,
            false,
            false,
            None,
        );
        assert!(out.contains("for r in"), "got: {out}");
    }

    #[test]
    fn render_assertion_not_empty_bare_result_uses_is_empty() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_empty", None, None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(out.contains("is_empty()"), "got: {out}");
    }

    #[test]
    fn render_assertion_min_length_one_uses_is_empty_not_len_ge_one() {
        let resolver = empty_resolver();
        let assertion = make_assertion("min_length", Some("content"), Some(serde_json::Value::from(1u64)));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(
            out.contains("is_empty()"),
            "min_length 1 should use !is_empty(); got: {out}"
        );
        assert!(
            !out.contains("len() >= 1"),
            "min_length 1 must not emit len() >= 1 (clippy::len_zero); got: {out}"
        );
    }

    #[test]
    fn render_assertion_min_length_two_still_uses_len_ge() {
        let resolver = empty_resolver();
        let assertion = make_assertion("min_length", Some("content"), Some(serde_json::Value::from(2u64)));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(
            out.contains("len() >= 2"),
            "min_length 2 should emit len() >= 2; got: {out}"
        );
    }
}
