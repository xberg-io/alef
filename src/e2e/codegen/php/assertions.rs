//! PHP fixture assertion rendering helpers.

use crate::e2e::codegen::assertion_recipes::chunks_result_var;
use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::{FieldSkip, nested_wildcard_skip_line};
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
    variant_access: &super::enum_variant_access::PhpVariantAccess<'_>,
) {
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct property accesses on the result.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                let result_var = &chunks_result_var(field_resolver, "php", result_var);
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
                let result_var = &chunks_result_var(field_resolver, "php", result_var);
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

    // What the PHP binding offers for an enum-variant segment is asked of the binding backend's
    // own enum lowering, not matched on the field's spelling — see `enum_variant_access`. ~keep
    if let Some(f) = &assertion.field {
        let assertion_kind = match variant_access.classify(f) {
            super::enum_variant_access::VariantAccess::Available => None,
            super::enum_variant_access::VariantAccess::NoAccessor => Some("enum_variant_accessor_unavailable"),
            super::enum_variant_access::VariantAccess::UnspellableFlatProperty => {
                Some("enum_variant_accessor_unspellable")
            }
        };
        if let Some(assertion_kind) = assertion_kind {
            out.push_str(&crate::e2e::template_env::render(
                "php/synthetic_assertion.jinja",
                minijinja::context! { assertion_kind => assertion_kind, field_name => f },
            ));
            return;
        }
    }

    // Streaming virtual fields: intercept before is_valid_for_result so they are
    // never skipped.  These fields resolve against the `$chunks` collected-list variable.
    // Only treat a field as streaming if the call is actually streaming.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && is_streaming
        && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f)
    {
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
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "count_equals" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        format!("        $this->assertCount({n}, {expr});\n")
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "equals" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
                        format!("        $this->assertEquals('{escaped}', {expr});\n")
                    } else if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        format!("        $this->assertEquals({n}, {expr});\n")
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
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
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "greater_than_or_equal" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        format!("        $this->assertGreaterThanOrEqual({n}, {expr});\n")
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "contains" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
                        format!("        $this->assertStringContainsString('{escaped}', {expr});\n")
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                _ => format!(
                    "{}\n",
                    streaming_assertion_type_skip_line("        ", "//", f, &assertion.assertion_type)
                ),
            };
            out.push_str(&line);
        } else {
            // ~keep `accessor` returns `None` for every `stream.has_*_event` predicate in PHP by
            // design (the crawl-stream is delivered as eager JSON — see `accessors.rs`), and this
            // branch used to be absent: the assertion vanished with no line for
            // `fail_on_unavailable_field_markers` to see. alef's streaming adapter owns the gap,
            // so it is counted, never fatal.
            let _ = writeln!(
                out,
                "        // skipped: {}",
                FieldSkip::StreamingAssertionOnUnsupportedField.message(f)
            );
        }
        return;
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        out.push_str(&crate::e2e::template_env::render(
            "php/synthetic_assertion.jinja",
            minijinja::context! {
                assertion_kind => "skipped",
                field_name => f,
            },
        ));
        return;
    }

    // When result_is_simple, skip assertions that reference non-content fields
    // (e.g., metadata, document, structure) since the binding returns a plain value.
    if result_is_simple && let Some(f) = &assertion.field {
        let f_lower = f.to_lowercase();
        if !f.is_empty()
            && f_lower != "content"
            && (f_lower.starts_with("metadata") || f_lower.starts_with("document") || f_lower.starts_with("structure"))
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

    // Bracket-wildcard traversal (`links[].link_type`) means "any element", so it must
    // render an array_filter quantifier. Falling through to `accessor` would lower the
    // wildcard to index 0 and silently assert against only the first element. Keyed off
    // the fixture path alone, never off the `[]`-spelled config sets. ~keep
    if !result_is_simple
        && let Some(f) = assertion.field.as_deref().filter(|f| !f.is_empty())
        && let Some((array_part, elem_part)) = field_resolver.wildcard_split(f)
    {
        // `wildcard_split` consumes the first `[].` only, so a doubly-nested path leaves a
        // second wildcard in `elem_part` that the element accessor below lowers to index 0. ~keep
        if let Some(line) = nested_wildcard_skip_line("        ", "//", f, &elem_part) {
            let _ = writeln!(out, "{line}");
            return;
        }
        let raw_array_accessor = if array_part.is_empty() {
            format!("${result_var}")
        } else {
            field_resolver.accessor(&array_part, "php", &format!("${result_var}"))
        };
        // array_filter() on null is a TypeError; `?? []` makes the quantifier false. ~keep
        let array_accessor = if !array_part.is_empty() && field_resolver.is_optional(&array_part) {
            format!("({raw_array_accessor} ?? [])")
        } else {
            raw_array_accessor
        };
        // `php_element_accessor`, not `accessor`: the path is already element-relative, so the
        // result-anchoring `accessor` applies would re-prefix it with the container. It also
        // isn't the generic cross-language `element_accessor`: that anchors PHP's
        // getter-vs-property owner cursor at the call's RESULT type, which answers a different
        // question than "is the ELEMENT type's field a getter" whenever a result envelope and its
        // collection elements classify differently -- `php_element_accessor` takes `array_part` so
        // it can resolve the actual element owner type instead. ~keep
        let elem_accessor = field_resolver.php_element_accessor(&elem_part, &array_part, "$e");
        match assertion.assertion_type.as_str() {
            "contains" | "contains_all" | "not_contains" => {
                let assert_fn = if assertion.assertion_type == "not_contains" {
                    "assertFalse"
                } else {
                    "assertTrue"
                };
                for expected in assertion.expected_values() {
                    let php_val = json_to_php(expected);
                    let _ = writeln!(
                        out,
                        "        $this->{assert_fn}((bool)array_filter({array_accessor}, fn($e) => str_contains((string){elem_accessor}, {php_val})));"
                    );
                }
            }
            "not_empty" => {
                let _ = writeln!(
                    out,
                    "        $this->assertTrue((bool)array_filter({array_accessor}, fn($e) => (string){elem_accessor} !== ''));"
                );
            }
            other => {
                let _ = writeln!(
                    out,
                    "        // skipped: unsupported traversal assertion '{other}' on '{f}'"
                );
            }
        }
        return;
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

    let field_is_optional = assertion
        .field
        .as_ref()
        .is_some_and(|f| !f.is_empty() && field_resolver.is_optional(f));

    let rendered = crate::e2e::template_env::render(
        "php/assertion.jinja",
        minijinja::context! {
            assertion_type => assertion_type,
            field_expr => field_expr,
            field_is_optional => field_is_optional,
            php_val => php_val,
            has_php_val => has_php_val,
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};

    use super::*;
    use crate::e2e::field_access::PhpGetterMap;

    fn empty_resolver() -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    /// `data`'s Rust type is `Option<DataNode>` -- a non-scalar struct, which ext-php-rs
    /// exposes only through a `#[php(getter)]` method, not a plain property. Build the same
    /// shape of `PhpGetterMap` alef's own codegen would derive from the IR, using the
    /// "owner type unknown" bare-name-union fallback (`needs_getter`'s legacy path) so the
    /// test doesn't have to also wire a full type graph.
    fn optional_getter_resolver(field: &str) -> FieldResolver {
        let optional: HashSet<String> = [field.to_string()].into_iter().collect();
        let getter_map = PhpGetterMap {
            getters: HashMap::from([("ProcessResult".to_string(), HashSet::from([field.to_string()]))]),
            ..PhpGetterMap::default()
        };
        FieldResolver::new_with_php_getters(
            &HashMap::new(),
            &optional,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            getter_map,
        )
    }

    fn is_true_assertion(field: &str) -> Assertion {
        Assertion {
            assertion_type: "is_true".to_string(),
            field: Some(field.to_string()),
            ..Assertion::default()
        }
    }

    fn render(resolver: &FieldResolver, assertion: &Assertion) -> String {
        let mut out = String::new();
        let getter_map = PhpGetterMap::default();
        let lowering = super::super::enum_variant_access::PhpEnumLowering::from_enums(&[]);
        render_assertion(
            &mut out,
            assertion,
            "result",
            resolver,
            false,
            false,
            &BTreeMap::new(),
            false,
            &super::super::enum_variant_access::PhpVariantAccess::new(&getter_map, &lowering),
        );
        out
    }

    /// `Option<DataNode>` presence: before the fix `field_expr` was unconditionally wrapped
    /// `($result->getData() ?? null)` for every assertion type on an optional field, and
    /// `is_true` passed that straight to `assertTrue`, which PHPUnit's declared `bool`
    /// parameter type rejects at runtime for a present non-bool value.
    #[test]
    fn is_true_on_optional_struct_field_checks_presence() {
        let out = render(&optional_getter_resolver("data"), &is_true_assertion("data"));
        assert_eq!(
            out,
            "            $this->assertTrue(($result->getData() ?? null) !== null);\n"
        );
    }

    #[test]
    fn is_false_on_optional_struct_field_checks_absence() {
        let out = render(
            &optional_getter_resolver("data"),
            &Assertion {
                assertion_type: "is_false".to_string(),
                field: Some("data".to_string()),
                ..Assertion::default()
            },
        );
        assert_eq!(
            out,
            "            $this->assertTrue(($result->getData() ?? null) === null);\n"
        );
    }

    /// A follow-on member access through the same optional field must null-safe navigate:
    /// before the fix `render_php_with_getters` never consulted `optional_fields`, so a
    /// nested path emitted `$result->getData()->kind` with no `?->`.
    #[test]
    fn equals_on_nested_field_through_optional_parent_null_safe_navigates() {
        let out = render(
            &optional_getter_resolver("data"),
            &Assertion {
                assertion_type: "equals".to_string(),
                field: Some("data.kind".to_string()),
                value: Some(serde_json::json!("KeyValue")),
                ..Assertion::default()
            },
        );
        assert!(out.contains("$result->getData()?->kind"), "got: {out}");
    }

    #[test]
    fn is_true_on_non_optional_field_is_unchanged() {
        let out = render(&empty_resolver(), &is_true_assertion("active"));
        assert_eq!(out, "            $this->assertTrue($result->active);\n");
    }

    /// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
    /// (present, non-`binding_excluded`, on some IR type) but missing from the
    /// hand-maintained `result_fields` config must still render a real assertion,
    /// not a "skipped: field not available" comment — `php/test_method.rs` (shared
    /// by `php` and `php_ext`) now threads `FieldResolver::ir_field_sets(type_defs)`
    /// into `with_ir_fields`. ~keep
    #[test]
    fn php_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
        let reachable: HashSet<String> = ["data".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(reachable, HashSet::new(), HashSet::new());
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("data".to_string()),
            value: Some(serde_json::Value::String("hello".to_string())),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            false,
            &BTreeMap::new(),
            false,
            &super::super::enum_variant_access::PhpVariantAccess::none(),
        );
        assert!(!out.contains("skipped"), "got: {out}");
    }

    /// The negative-control half of the same regression: `internal_diagnostics`
    /// represents a field carrying `#[doc(hidden)]` or `#[cfg_attr(alef,
    /// alef(skip))]` in the real struct (a genuine `binding_excluded` field) —
    /// NOT `#[serde(skip)]`, which alone does not exclude a field from the
    /// binding surface. Even though it is listed in `result_fields` (a stale/
    /// wrong config entry), the IR must still win and reject it. ~keep
    #[test]
    fn php_ir_excluded_field_present_in_result_fields_is_still_skipped() {
        let result_fields: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
        let excluded: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(HashSet::new(), excluded, HashSet::new());
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("internal_diagnostics".to_string()),
            value: Some(serde_json::Value::String("hello".to_string())),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            false,
            &BTreeMap::new(),
            false,
            &super::super::enum_variant_access::PhpVariantAccess::none(),
        );
        assert!(out.contains("skipped"), "got: {out}");
    }

    /// Regression test for a one-sided-trim bug: `trim()` wrapped the actual value while the
    /// fixture `expected` literal was emitted verbatim. Fixture expectations may legitimately
    /// end in `\n`, so trimming only one side made those assertions impossible to satisfy —
    /// and trimming both would silently mask real trailing-whitespace regressions. Equals is
    /// exact: neither side is normalized.
    /// Control for the trim fix: the tightened contract must still DISCRIMINATE values that
    /// differ only in trailing whitespace. If either side were normalized, the emitted
    /// assertion for "hello\n" and for "hello" would be identical and a real trailing-newline
    /// regression would pass unnoticed.
    #[test]
    fn render_assertion_equals_still_discriminates_trailing_whitespace() {
        let render_for = |value: &str| {
            let resolver = empty_resolver();
            let assertion = Assertion {
                assertion_type: "equals".to_string(),
                field: None,
                value: Some(serde_json::Value::String(value.into())),
                ..Default::default()
            };
            let mut out = String::new();
            render_assertion(
                &mut out,
                &assertion,
                "result",
                &resolver,
                true,
                false,
                &BTreeMap::new(),
                false,
                &super::super::enum_variant_access::PhpVariantAccess::none(),
            );
            out
        };
        let emitted = render_for("hello\n");
        // The actual side must be the bare expression: any normalizing call (trim/strip/
        // case-folding) wrapped around it would silently accept a mismatched value.
        assert_eq!(
            emitted, "            $this->assertEquals(\"hello\\n\", $result);\n",
            "emitted assertion drifted: {emitted}"
        );
        // And a value differing only by the trailing newline must still produce a
        // different expectation, proving trailing whitespace is discriminated.
        assert_ne!(
            emitted,
            render_for("hello"),
            "trailing newline must still change the emitted assertion"
        );
    }

    #[test]
    fn render_assertion_equals_string_compares_exactly_without_trim() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: None,
            value: Some(serde_json::Value::String("hello\n".into())),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            true,
            false,
            &BTreeMap::new(),
            false,
            &super::super::enum_variant_access::PhpVariantAccess::none(),
        );
        assert!(!out.contains("trim("), "equals must not trim either side; got: {out}");
        assert!(out.contains("assertEquals("), "got: {out}");
    }

    fn wildcard_resolver() -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::from(["links".to_string(), "pages".to_string()]),
            &HashSet::from(["links".to_string(), "pages".to_string()]),
            &HashSet::new(),
        )
    }

    fn render_wildcard(assertion_type: &str, field: &str) -> String {
        let assertion = Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value: if assertion_type == "not_empty" {
                None
            } else {
                Some(serde_json::Value::String("internal".to_string()))
            },
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &wildcard_resolver(),
            false,
            false,
            &BTreeMap::new(),
            false,
            &super::super::enum_variant_access::PhpVariantAccess::none(),
        );
        out
    }

    /// A bracket-wildcard fixture path means "every element", so the emitted PHP
    /// must quantify with array_filter over the whole array.
    #[test]
    fn php_wildcard_contains_emits_quantifier_over_all_elements() {
        let out = render_wildcard("contains", "links[].link_type");
        assert!(
            out.contains(
                "$this->assertTrue((bool)array_filter($result->links, fn($e) => str_contains((string)$e->linkType, \"internal\")));"
            ),
            "expected an any-element quantifier, got:\n{out}"
        );
    }

    /// THE CANARY. A fixture whose match lives only in element 1 is satisfied by an
    /// any-element quantifier and missed by an index-0 lookup. This unit test observes
    /// the emitted source rather than executing it, so it pins the property that makes
    /// the runtime difference: the wildcard must NOT lower to a single-element access.
    /// Pre-fix the wildcard rendered `$result->links[0]->linkType`, which reads element 0
    /// only and reports a false green; this assertion is red then.
    #[test]
    fn php_wildcard_does_not_collapse_to_element_zero() {
        let out = render_wildcard("contains", "links[].link_type");
        assert!(
            !out.contains("[0]"),
            "wildcard must not lower to a single-element access, got:\n{out}"
        );
    }

    /// Regression lock: an explicit numeric index is not a wildcard and must keep
    /// resolving to that exact element.
    #[test]
    fn php_explicit_index_still_resolves_to_element_zero() {
        let out = render_wildcard("contains", "links[0].link_type");
        assert!(
            out.contains("$result->links[0]->linkType"),
            "explicit index 0 must keep its index-preserving accessor, got:\n{out}"
        );
        assert!(
            !out.contains("array_filter"),
            "explicit index must not become a quantifier, got:\n{out}"
        );
    }

    /// `wildcard_split` consumes the first `[].` only, so before the guard the `array_filter`
    /// ranged over `pages` while its closure read `$e->links[0]->url` — a whole-array claim
    /// that only ever inspected element zero of the inner array. Pre-guard this test fails:
    /// the skip line is absent and a quantifier over `[0]` is present. ~keep
    #[test]
    fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
        let out = render_wildcard("contains", "pages[].links[].url");
        assert_eq!(
            out, "        // skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
            "got:\n{out}"
        );
    }
}
