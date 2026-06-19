//! PHP fixture assertion rendering helpers.

use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;

use super::values::json_to_php;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_array: bool,
    fields_array_bindings: &std::collections::BTreeMap<String, (String, String)>,
    is_streaming: bool,
) {
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct property accesses on the result.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                let pred = format!(
                    "array_reduce(${result_var}->chunks ?? [], fn($carry, $c) => $carry && !empty($c->content), true)"
                );
                out.push_str(&crate::e2e::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_content",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            "chunks_have_embeddings" => {
                let pred = format!(
                    "array_reduce(${result_var}->chunks ?? [], fn($carry, $c) => $carry && !empty($c->embedding), true)"
                );
                out.push_str(&crate::e2e::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_embeddings",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            // ---- EmbedResponse virtual fields ----
            // embed_texts returns array<array<float>> in PHP — no wrapper object.
            // $result_var is the embedding matrix; use it directly.
            "embeddings" => {
                let php_val = assertion.value.as_ref().map(json_to_php).unwrap_or_default();
                out.push_str(&crate::e2e::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "embeddings",
                        assertion_type => assertion.assertion_type.as_str(),
                        php_val => php_val,
                        result_var => result_var,
                    },
                ));
                return;
            }
            "embedding_dimensions" => {
                let expr = format!("(empty(${result_var}) ? 0 : count(${result_var}[0]))");
                let php_val = assertion.value.as_ref().map(json_to_php).unwrap_or_default();
                out.push_str(&crate::e2e::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "embedding_dimensions",
                        assertion_type => assertion.assertion_type.as_str(),
                        expr => expr,
                        php_val => php_val,
                    },
                ));
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                let pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!("array_reduce(${result_var}, fn($carry, $e) => $carry && count($e) > 0, true)")
                    }
                    "embeddings_finite" => {
                        format!(
                            "array_reduce(${result_var}, fn($carry, $e) => $carry && array_reduce($e, fn($c, $v) => $c && is_finite($v), true), true)"
                        )
                    }
                    "embeddings_non_zero" => {
                        format!(
                            "array_reduce(${result_var}, fn($carry, $e) => $carry && count(array_filter($e, fn($v) => $v !== 0.0)) > 0, true)"
                        )
                    }
                    "embeddings_normalized" => {
                        format!(
                            "array_reduce(${result_var}, fn($carry, $e) => $carry && abs(array_sum(array_map(fn($v) => $v * $v, $e)) - 1.0) < 1e-3, true)"
                        )
                    }
                    _ => unreachable!(),
                };
                let assertion_kind = format!("embeddings_{}", f.strip_prefix("embeddings_").unwrap_or(f));
                out.push_str(&crate::e2e::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => assertion_kind,
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            // ---- keywords / keywords_count ----
            // PHP ProcessingResult does not expose result_keywords; skip.
            "keywords" | "keywords_count" => {
                out.push_str(&crate::e2e::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "keywords",
                        field_name => f,
                    },
                ));
                return;
            }
            _ => {}
        }
    }

    // Skip enum variant accessors (metadata.format.excel etc.) — PHP bindings
    // serialize FormatMetadata to JSON, so variants are unavailable in PHP.
    if let Some(f) = &assertion.field {
        if f.contains("metadata.format.") && f.matches('.').count() >= 2 {
            out.push_str(&crate::e2e::template_env::render(
                "php/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_kind => "skipped",
                    field_name => f,
                },
            ));
            return;
        }
    }

    // Streaming virtual fields: intercept before is_valid_for_result so they are
    // never skipped.  These fields resolve against the `$chunks` collected-list variable.
    // Only treat a field as streaming if the call is actually streaming.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && is_streaming && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
            if let Some(expr) =
                crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, "php", "chunks")
            {
                let line = match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!(
                                "        $this->assertGreaterThanOrEqual({n}, count({expr}), 'expected >= {n} chunks');\n"
                            )
                        } else {
                            String::new()
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        $this->assertCount({n}, {expr});\n")
                        } else {
                            String::new()
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
                            format!("        $this->assertEquals('{escaped}', {expr});\n")
                        } else if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        $this->assertEquals({n}, {expr});\n")
                        } else {
                            String::new()
                        }
                    }
                    "not_empty" => format!("        $this->assertNotEmpty({expr});\n"),
                    "is_empty" => format!("        $this->assertEmpty({expr});\n"),
                    "is_true" => format!("        $this->assertTrue({expr});\n"),
                    "is_false" => format!("        $this->assertFalse({expr});\n"),
                    "greater_than" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        $this->assertGreaterThan({n}, {expr});\n")
                        } else {
                            String::new()
                        }
                    }
                    "greater_than_or_equal" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        $this->assertGreaterThanOrEqual({n}, {expr});\n")
                        } else {
                            String::new()
                        }
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
                            format!("        $this->assertStringContainsString('{escaped}', {expr});\n")
                        } else {
                            String::new()
                        }
                    }
                    _ => format!(
                        "        // streaming field '{f}': assertion type '{}' not rendered\n",
                        assertion.assertion_type
                    ),
                };
                if !line.is_empty() {
                    out.push_str(&line);
                }
            }
            return;
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            out.push_str(&crate::e2e::template_env::render(
                "php/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_kind => "skipped",
                    field_name => f,
                },
            ));
            return;
        }
    }

    // When result_is_simple, skip assertions that reference non-content fields
    // (e.g., metadata, document, structure) since the binding returns a plain value.
    if result_is_simple {
        if let Some(f) = &assertion.field {
            let f_lower = f.to_lowercase();
            if !f.is_empty()
                && f_lower != "content"
                && (f_lower.starts_with("metadata")
                    || f_lower.starts_with("document")
                    || f_lower.starts_with("structure"))
            {
                out.push_str(&crate::e2e::template_env::render(
                    "php/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "result_is_simple",
                        field_name => f,
                    },
                ));
                return;
            }
        }
    }

    let field_expr = match &assertion.field {
        // When result_is_simple, the result is a scalar (bytes/string/etc.) — any
        // field access on it would fail. Treat all assertions as referring to the
        // result itself.
        _ if result_is_simple => format!("${result_var}"),
        Some(f) if !f.is_empty() => {
            // Check if this field_array field has been bound to a variable
            if let Some((var_name, _)) = fields_array_bindings.get(f) {
                format!("${}", var_name)
            } else {
                // For display_as_text fields (content unions like AssistantContent),
                // call the text() accessor to get the textual representation.
                // For example, for "choices[0].message.content", we call text() on
                // the parent "choices[0].message" object (AssistantMessage).
                if field_resolver.is_display_as_text(f) {
                    // Parse the field path to get the parent accessor (without the leaf field).
                    // For "choices[0].message.content", we want "choices[0].message"
                    let parent_field = if let Some(last_dot) = f.rfind('.') {
                        &f[..last_dot]
                    } else {
                        f
                    };
                    let parent_accessor = field_resolver.accessor(parent_field, "php", &format!("${result_var}"));
                    // Check if the parent accessor might be optional and needs safe-call syntax
                    if field_resolver.is_optional(parent_field) {
                        format!("({parent_accessor}?->text() ?? '')")
                    } else {
                        format!("{parent_accessor}->text()")
                    }
                } else {
                    let accessor = field_resolver.accessor(f, "php", &format!("${result_var}"));
                    // For optional fields, wrap with ?? null to handle null-safe access
                    if field_resolver.is_optional(f) {
                        format!("({accessor} ?? null)")
                    } else {
                        accessor
                    }
                }
            }
        }
        _ => format!("${result_var}"),
    };

    // Detect if this field is an array type
    // When there's no field, default to result_is_array (the result itself is the array)
    // When result_is_simple, the assertion's `field` is a logical alias for the
    // result itself (`field_expr` above already routes to `$result_var`), so
    // `field_is_array` must mirror `result_is_array` rather than trying to
    // resolve a sub-field that doesn't exist on a scalar return type.
    let field_is_array = if result_is_simple {
        result_is_array
    } else {
        assertion.field.as_ref().map_or(result_is_array, |f| {
            if f.is_empty() {
                result_is_array
            } else {
                field_resolver.is_array(f)
            }
        })
    };

    // For string equality, trim trailing whitespace to handle trailing newlines.
    // Only apply trim() when the expected value is a string — calling trim() on int/bool
    // throws TypeError in PHP 8.4+.
    let trimmed_field_expr_for = |expected: &serde_json::Value| -> String {
        if expected.is_string() {
            format!("trim({})", field_expr)
        } else {
            field_expr.clone()
        }
    };

    // Prepare template context.
    let assertion_type = assertion.assertion_type.as_str();
    let has_php_val = assertion.value.is_some();
    // serde collapses `"value": null` to `None`, but `equals` against null is a real
    // assertion (e.g. `result.message.content == null`). Default to PHP `null` in that
    // case so the rendered code compiles instead of producing `assertEquals(, ...)`.
    let php_val = match assertion.value.as_ref() {
        Some(v) => json_to_php(v),
        None if assertion_type == "equals" => "null".to_string(),
        None => String::new(),
    };
    let trimmed_field_expr = trimmed_field_expr_for(assertion.value.as_ref().unwrap_or(&serde_json::Value::Null));
    let is_string_val = assertion.value.as_ref().is_some_and(|v| v.is_string());
    // values_php is consumed by `contains`, `contains_all`, and `not_contains` loops.
    // Fall back to wrapping the singular `value` so single-entry fixtures still emit one
    // assertion call per value instead of an empty loop.
    let values_php: Vec<String> = assertion
        .values
        .as_ref()
        .map(|vals| vals.iter().map(json_to_php).collect::<Vec<_>>())
        .or_else(|| assertion.value.as_ref().map(|v| vec![json_to_php(v)]))
        .unwrap_or_default();
    let contains_any_checks: Vec<String> = assertion
        .values
        .as_ref()
        .map_or(Vec::new(), |vals| vals.iter().map(json_to_php).collect());
    let n = assertion.value.as_ref().and_then(|v| v.as_u64()).unwrap_or(0);

    // For method_result assertions.
    let call_expr = if let Some(method_name) = &assertion.method {
        build_php_method_call(result_var, method_name, assertion.args.as_ref())
    } else {
        String::new()
    };
    let check = assertion.check.as_deref().unwrap_or("is_true");
    let has_php_check_val = matches!(assertion.assertion_type.as_str(), "method_result") && assertion.value.is_some();
    let php_check_val = if matches!(assertion.assertion_type.as_str(), "method_result") {
        assertion.value.as_ref().map(json_to_php).unwrap_or_default()
    } else {
        String::new()
    };
    let check_n = assertion.value.as_ref().and_then(|v| v.as_u64()).unwrap_or(0);
    let is_bool_val = assertion.value.as_ref().is_some_and(|v| v.is_boolean());
    let bool_is_true = assertion.value.as_ref().and_then(|v| v.as_bool()).unwrap_or(false);

    // Early returns for non-template-renderable assertions.
    if matches!(assertion_type, "not_error" | "error") {
        if assertion_type == "not_error" {
            // Already handled by the call succeeding without exception.
        }
        // "error" is handled at the test method level.
        return;
    }

    let rendered = crate::e2e::template_env::render(
        "php/assertion.jinja",
        minijinja::context! {
            assertion_type => assertion_type,
            field_expr => field_expr,
            php_val => php_val,
            has_php_val => has_php_val,
            trimmed_field_expr => trimmed_field_expr,
            is_string_val => is_string_val,
            field_is_array => field_is_array,
            values_php => values_php,
            contains_any_checks => contains_any_checks,
            n => n,
            call_expr => call_expr,
            check => check,
            php_check_val => php_check_val,
            has_php_check_val => has_php_check_val,
            check_n => check_n,
            is_bool_val => is_bool_val,
            bool_is_true => bool_is_true,
        },
    );
    let _ = write!(out, "        {}", rendered);
}

/// Build a PHP call expression for a `method_result` assertion.
///
/// Uses generic instance method dispatch: `$result_var->method_name(args...)`.
/// Args from the fixture JSON object are emitted as positional PHP arguments in
/// insertion order, using best-effort type conversion (strings → PHP string literals,
/// numbers and booleans → verbatim literals).
pub(super) fn build_php_method_call(result_var: &str, method_name: &str, args: Option<&serde_json::Value>) -> String {
    let extra_args = if let Some(args_val) = args {
        args_val
            .as_object()
            .map(|obj| {
                obj.values()
                    .map(|v| match v {
                        serde_json::Value::String(s) => {
                            format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
                        }
                        serde_json::Value::Bool(true) => "true".to_string(),
                        serde_json::Value::Bool(false) => "false".to_string(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Null => "null".to_string(),
                        other => format!("\"{}\"", other.to_string().replace('\\', "\\\\").replace('"', "\\\"")),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    if extra_args.is_empty() {
        format!("${result_var}->{method_name}()")
    } else {
        format!("${result_var}->{method_name}({extra_args})")
    }
}
