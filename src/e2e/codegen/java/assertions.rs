use crate::e2e::escape::escape_java;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use heck::ToLowerCamelCase;

use super::values::json_to_java;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    class_name: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_bytes: bool,
    result_is_option: bool,
    is_streaming: bool,
    streaming_item_type: Option<&str>,
    enum_fields: &std::collections::HashSet<String>,
    assert_enum_types: &std::collections::HashMap<String, String>,
) {
    // Bare-result is_empty / not_empty on Option<T> returns: the Java facade exposes
    // these as `@Nullable T` (via `.orElse(null)`) rather than `Optional<T>`, so the
    // template's `.isEmpty()` call would not compile for record types. Emit a
    // null-check instead — mirrors the kotlin / zig codegen behaviour.
    let bare_field = assertion.field.as_deref().is_none_or(str::is_empty);
    if result_is_option && bare_field {
        match assertion.assertion_type.as_str() {
            "is_empty" => {
                out.push_str(&format!(
                    "        assertNull({result_var}, \"expected empty value\");\n"
                ));
                return;
            }
            "not_empty" => {
                out.push_str(&format!(
                    "        assertNotNull({result_var}, \"expected non-empty value\");\n"
                ));
                return;
            }
            _ => {}
        }
    }

    // Byte-buffer returns: emit length-based assertions instead of struct-field
    // accessors. The result is `byte[]`, which has no `isEmpty()`/struct-field methods.
    // Field paths on byte-buffer results (e.g. `audio`, `content`) are pseudo-fields
    // referencing the buffer itself — treat them the same as no-field assertions.
    if result_is_bytes {
        match assertion.assertion_type.as_str() {
            "not_empty" => {
                out.push_str(&format!(
                    "        assertTrue({result_var}.length > 0, \"expected non-empty value\");\n"
                ));
                return;
            }
            "is_empty" => {
                out.push_str(&format!(
                    "        assertEquals(0, {result_var}.length, \"expected empty value\");\n"
                ));
                return;
            }
            "count_equals" | "length_equals" => {
                if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                    out.push_str(&format!("        assertEquals({n}, {result_var}.length);\n"));
                }
                return;
            }
            "count_min" | "length_min" => {
                if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                    out.push_str(&format!(
                        "        assertTrue({result_var}.length >= {n}, \"expected length >= {n}\");\n"
                    ));
                }
                return;
            }
            "not_error" => {
                // Use the statically-imported assertion (org.junit.jupiter.api.Assertions.*)
                // so we don't need a separate FQN import of the `Assertions` class.
                out.push_str(&format!(
                    "        assertNotNull({result_var}, \"expected non-null byte[] response\");\n"
                ));
                return;
            }
            _ => {
                out.push_str(&format!(
                    "        // skipped: assertion type '{}' not supported on byte[] result\n",
                    assertion.assertion_type
                ));
                return;
            }
        }
    }

    // Handle synthetic/virtual fields that are computed rather than direct record accessors.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            // ---- ProcessingResult chunk-level computed predicates ----
            "chunks_have_content" => {
                let pred = format!(
                    "java.util.Optional.ofNullable({result_var}.chunks()).orElse(java.util.List.of()).stream().allMatch(c -> c.content() != null && !c.content().isBlank())"
                );
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_content",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            "chunks_have_heading_context" => {
                let pred = format!(
                    "java.util.Optional.ofNullable({result_var}.chunks()).orElse(java.util.List.of()).stream().allMatch(c -> c.metadata().headingContext() != null)"
                );
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_heading_context",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            "chunks_have_embeddings" => {
                let pred = format!(
                    "java.util.Optional.ofNullable({result_var}.chunks()).orElse(java.util.List.of()).stream().allMatch(c -> c.embedding() != null && !c.embedding().isEmpty())"
                );
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_embeddings",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            "first_chunk_starts_with_heading" => {
                let pred = format!(
                    "java.util.Optional.ofNullable({result_var}.chunks()).orElse(java.util.List.of()).stream().findFirst().map(c -> c.metadata().headingContext() != null).orElse(false)"
                );
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "first_chunk_heading",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            // ---- EmbedResponse virtual fields ----
            // When result_is_simple=true the result IS List<List<Float>> (the raw embeddings list).
            // When result_is_simple=false the result has an .embeddings() accessor.
            "embedding_dimensions" => {
                // Dimension = size of the first embedding vector in the list.
                let embed_list = if result_is_simple {
                    result_var.to_string()
                } else {
                    format!("{result_var}.embeddings()")
                };
                let expr = format!("({embed_list}.isEmpty() ? 0 : {embed_list}.get(0).size())");
                let java_val = assertion.value.as_ref().map(json_to_java).unwrap_or_default();
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "embedding_dimensions",
                        assertion_type => assertion.assertion_type.as_str(),
                        expr => expr,
                        java_val => java_val,
                        field_name => f,
                    },
                ));
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                // These are validation predicates that require iterating the embedding matrix.
                let embed_list = if result_is_simple {
                    result_var.to_string()
                } else {
                    format!("{result_var}.embeddings()")
                };
                let pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!("{embed_list}.stream().allMatch(e -> e != null && !e.isEmpty())")
                    }
                    "embeddings_finite" => {
                        format!("{embed_list}.stream().flatMap(java.util.Collection::stream).allMatch(Float::isFinite)")
                    }
                    "embeddings_non_zero" => {
                        format!("{embed_list}.stream().allMatch(e -> e.stream().anyMatch(v -> v != 0.0f))")
                    }
                    "embeddings_normalized" => format!(
                        "{embed_list}.stream().allMatch(e -> {{ double n = e.stream().mapToDouble(v -> v * v).sum(); return Math.abs(n - 1.0) < 1e-3; }})"
                    ),
                    _ => unreachable!(),
                };
                let assertion_kind = format!("embeddings_{}", f.strip_prefix("embeddings_").unwrap_or(f));
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => assertion_kind,
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return;
            }
            // ---- Fields not present on the Java ProcessingResult ----
            "keywords" | "keywords_count" => {
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "keywords",
                        field_name => f,
                    },
                ));
                return;
            }
            // ---- metadata not_empty / is_empty: Metadata is a required record, not Optional ----
            // Metadata has no .isEmpty() method; check that at least one optional field is present.
            "metadata" => {
                match assertion.assertion_type.as_str() {
                    "not_empty" | "is_empty" => {
                        out.push_str(&crate::e2e::template_env::render(
                            "java/synthetic_assertion.jinja",
                            minijinja::context! {
                                assertion_kind => "metadata",
                                assertion_type => assertion.assertion_type.as_str(),
                                result_var => result_var,
                            },
                        ));
                        return;
                    }
                    _ => {} // fall through to normal handling
                }
            }
            _ => {}
        }
    }

    // Streaming virtual fields: intercept before is_valid_for_result so they are
    // never skipped.  These fields resolve against the `chunks` collected-list variable.
    // Gate on `is_streaming` so non-streaming fixtures (e.g. consumers whose real
    // result struct has a literal `chunks` field) don't divert into the virtual
    // accessor path — they should fall through to the normal field resolver.
    if let Some(f) = &assertion.field {
        if is_streaming && !f.is_empty() && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
            if let Some(expr) =
                crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor_with_streaming_context(
                    f,
                    "java",
                    "chunks",
                    None,
                    streaming_item_type,
                )
            {
                let line = match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertTrue({expr}.size() >= {n}, \"expected >= {n} chunks\");\n")
                        } else {
                            String::new()
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertEquals({n}, {expr}.size());\n")
                        } else {
                            String::new()
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = crate::e2e::escape::escape_java(s);
                            format!("        assertEquals(\"{escaped}\", {expr});\n")
                        } else if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertEquals({n}, {expr});\n")
                        } else {
                            String::new()
                        }
                    }
                    "not_empty" => format!("        assertFalse({expr}.isEmpty(), \"expected non-empty\");\n"),
                    "is_empty" => format!("        assertTrue({expr}.isEmpty(), \"expected empty\");\n"),
                    "is_true" => format!("        assertTrue({expr}, \"expected true\");\n"),
                    "is_false" => format!("        assertFalse({expr}, \"expected false\");\n"),
                    "greater_than" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertTrue({expr} > {n}, \"expected > {n}\");\n")
                        } else {
                            String::new()
                        }
                    }
                    "greater_than_or_equal" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertTrue({expr} >= {n}, \"expected >= {n}\");\n")
                        } else {
                            String::new()
                        }
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = crate::e2e::escape::escape_java(s);
                            format!(
                                "        assertTrue({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\");\n"
                            )
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
                "java/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_kind => "skipped",
                    field_name => f,
                },
            ));
            return;
        }
    }

    // Determine if this field maps to a sealed-interface type declared in
    // `assert_enum_types`.  When `Some`, the value is the type name (e.g.
    // "FormatMetadata") and the corresponding `{TypeName}Display` helper will
    // be used to produce the display string for assertions.
    let sealed_display_type: Option<String> = assertion.field.as_deref().and_then(|f| {
        let resolved = field_resolver.resolve(f);
        assert_enum_types
            .get(f)
            .or_else(|| assert_enum_types.get(resolved))
            .cloned()
    });
    let is_sealed_display_field = sealed_display_type.is_some();

    // Determine if this field is an enum type (no `.contains()` on enums in Java).
    // Check both the raw fixture field path and the resolved (aliased) path so that
    // `fields_enum` entries can use either form (e.g., `"assets[].category"` or the
    // resolved `"assets[].asset_category"`).
    // NOTE: Sealed-interface types (those in assert_enum_types) are not Java enums
    // and do not have a .getValue() method — exclude them from enum field treatment.
    let field_is_enum = assertion.field.as_deref().is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        let in_enum_fields = enum_fields.get(f).is_some() || enum_fields.get(resolved).is_some();
        in_enum_fields && !is_sealed_display_field
    });

    // Determine if this field is an array (List<T>) — needed to choose .toString() for
    // contains assertions, since List.contains(Object) uses equals() which won't match
    // strings against complex record types like StructureItem.
    let field_is_array = assertion
        .field
        .as_deref()
        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));

    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => {
                let accessor = field_resolver.accessor(f, "java", result_var);
                let resolved = field_resolver.resolve(f);
                // Unwrap Optional fields with a type-appropriate fallback.
                // Map.get() returns nullable, not Optional, so skip .orElse() for map access.
                // NOTE: is_optional() means the field is in optional_fields, but that doesn't
                // guarantee it returns Optional<T> in Java — nested fields like metadata.twitterCard
                // return @Nullable String, not Optional<String>. We detect this by checking
                // if the field path contains a dot (nested access).
                if field_resolver.is_optional(resolved) && !field_resolver.has_map_access(f) {
                    // All nullable fields in the Java binding return @Nullable types, not Optional<T>.
                    // Wrap them in Optional.ofNullable() so e2e tests can use .orElse() fallbacks.
                    let optional_expr = format!("java.util.Optional.ofNullable({accessor})");
                    // Enum-typed optional fields need .map(v -> v.getValue()) to coerce to String
                    // before the orElse("") fallback can type-check (Optional<Enum>.orElse("") would
                    // be a type mismatch — Optional<String>.orElse("") is the only safe form).
                    if field_is_enum {
                        match assertion.assertion_type.as_str() {
                            "not_empty" | "is_empty" => optional_expr,
                            _ => {
                                // `field_is_enum` already excludes sealed-interface types
                                // (is_sealed_display_field), so any remaining enum type
                                // has .getValue() available.
                                format!("{optional_expr}.map(v -> v.getValue()).orElse(\"\")")
                            }
                        }
                    } else {
                        match assertion.assertion_type.as_str() {
                            // For not_empty / is_empty on Optional fields, return the raw Optional
                            // so the assertion arms can call isPresent()/isEmpty().
                            "not_empty" | "is_empty" => optional_expr,
                            // For size/count assertions on Optional<List<T>> fields, use List.of() fallback.
                            "count_min" | "count_equals" => {
                                format!("{optional_expr}.orElse(java.util.List.of())")
                            }
                            // For numeric comparisons on Optional<Long/Integer> fields, coerce
                            // the boxed numeric type to `long` via Number::longValue so the same
                            // code path compiles for both `Optional<Integer>` (e.g. mapped from
                            // Rust `Option<u32>`) and `Optional<Long>` fields.  Using a bare
                            // `.orElse(0L)` would fail for `Optional<Integer>` because the
                            // fallback type would not match the element type.
                            "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" => {
                                if field_resolver.is_array(resolved) {
                                    format!("{optional_expr}.orElse(java.util.List.of())")
                                } else {
                                    format!("{optional_expr}.map(Number::longValue).orElse(0L)")
                                }
                            }
                            // For equals on Optional fields, determine fallback based on whether value is numeric.
                            // If the fixture value is a number, coerce via Number::longValue so the
                            // comparison compiles for both Optional<Integer> and Optional<Long>.
                            // Sealed-display fields are handled via the {TypeName}Display helper in
                            // string_expr — keep as Optional here so the helper receives the unwrapped value.
                            "equals" => {
                                if is_sealed_display_field {
                                    // Sealed-interface Optional: keep, will be handled by string_expr path
                                    optional_expr
                                } else if let Some(expected) = &assertion.value {
                                    if expected.is_number() {
                                        format!("{optional_expr}.map(Number::longValue).orElse(0L)")
                                    } else {
                                        // `.map(Objects::toString)` collapses Optional<T> to
                                        // Optional<String> before `.orElse("")`, so the result
                                        // is unambiguously a String even when T is `Object`
                                        // (which is the Java mapping for free-form JSON values
                                        // like `Option<serde_json::Value>` — javac otherwise
                                        // infers LUB(Object, String) = Object and breaks
                                        // String-only method calls like .contains()).
                                        format!("{optional_expr}.map(java.util.Objects::toString).orElse(\"\")")
                                    }
                                } else {
                                    format!("{optional_expr}.map(java.util.Objects::toString).orElse(\"\")")
                                }
                            }
                            _ if field_resolver.is_array(resolved) => {
                                format!("{optional_expr}.orElse(java.util.List.of())")
                            }
                            _ => format!("{optional_expr}.map(java.util.Objects::toString).orElse(\"\")"),
                        }
                    }
                } else {
                    accessor
                }
            }
            _ => result_var.to_string(),
        }
    };

    // For enum fields, string-based assertions need .getValue() to convert the enum to
    // its serde-serialized lowercase string value (e.g., AssetCategory.Image -> "image").
    // All alef-generated Java enums expose a getValue() method annotated with @JsonValue.
    // Optional enum fields are already coerced to String via `.map(v -> v.getValue()).orElse("")`
    // upstream in field_expr; in that case the value is already a String and we must not
    // call .getValue() again. Detect by looking for `.map(v -> v.getValue())` in the expr.
    // Sealed-interface types (is_sealed_display_field) use a pattern-match helper instead.
    let string_expr = if field_is_enum && !field_expr.contains(".map(v -> v.getValue())") {
        format!("{field_expr}.getValue()")
    } else if let Some(ref stype) = sealed_display_type {
        // Sealed-interface type: convert via a generated `{TypeName}Display.toDisplayString`
        // helper that pattern-matches over all variants from the IR.
        // For Optional<T>, unwrap with orElse(null) so the helper can handle null safely.
        let inner_expr = if field_expr.contains("Optional.ofNullable") {
            format!("{field_expr}.orElse(null)")
        } else {
            field_expr.clone()
        };
        format!("{stype}Display.toDisplayString({inner_expr})")
    } else {
        field_expr.clone()
    };

    // Pre-compute context for template
    let assertion_type = assertion.assertion_type.as_str();
    let java_val = assertion.value.as_ref().map(json_to_java).unwrap_or_default();
    let is_string_val = assertion.value.as_ref().is_some_and(|v| v.is_string());
    let is_numeric_val = assertion.value.as_ref().is_some_and(|v| v.is_number());

    // values_java is consumed by `contains`, `contains_all`, `contains_any`, and
    // `not_contains` loops. Fall back to wrapping the singular `value` so single-entry
    // fixtures still emit one assertion call per value instead of an empty loop.
    let values_java: Vec<String> = assertion
        .values
        .as_ref()
        .map(|values| values.iter().map(json_to_java).collect::<Vec<_>>())
        .or_else(|| assertion.value.as_ref().map(|v| vec![json_to_java(v)]))
        .unwrap_or_default();

    let contains_any_expr = if !values_java.is_empty() {
        values_java
            .iter()
            .map(|v| format!("{string_expr}.contains({v})"))
            .collect::<Vec<_>>()
            .join(" || ")
    } else {
        String::new()
    };

    let length_expr = if result_is_bytes {
        format!("{field_expr}.length")
    } else {
        format!("{field_expr}.length()")
    };

    let n = assertion.value.as_ref().and_then(|v| v.as_u64()).unwrap_or(0);

    let call_expr = if let Some(method_name) = &assertion.method {
        build_java_method_call(result_var, method_name, assertion.args.as_ref(), class_name)
    } else {
        String::new()
    };

    let check = assertion.check.as_deref().unwrap_or("is_true");

    let java_check_val = assertion.value.as_ref().map(json_to_java).unwrap_or_default();

    let check_n = assertion.value.as_ref().and_then(|v| v.as_u64()).unwrap_or(0);

    let is_bool_val = assertion.value.as_ref().is_some_and(|v| v.is_boolean());
    let bool_is_true = assertion.value.as_ref().is_some_and(|v| v.as_bool() == Some(true));

    let method_returns_collection = assertion
        .method
        .as_ref()
        .is_some_and(|m| matches!(m.as_str(), "find_nodes_by_type" | "findNodesByType"));

    let rendered = crate::e2e::template_env::render(
        "java/assertion.jinja",
        minijinja::context! {
            assertion_type,
            java_val,
            string_expr,
            field_expr,
            field_is_enum,
            field_is_array,
            is_string_val,
            is_numeric_val,
            values_java => values_java,
            contains_any_expr,
            length_expr,
            n,
            call_expr,
            check,
            java_check_val,
            check_n,
            is_bool_val,
            bool_is_true,
            method_returns_collection,
        },
    );
    out.push_str(&rendered);
}

/// Build a Java call expression for a `method_result` assertion on a sample_language Tree.
///
/// Maps method names to the appropriate Java static/instance method calls.
pub(super) fn build_java_method_call(
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    class_name: &str,
) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}.rootNode().childCount()"),
        "root_node_type" => format!("{result_var}.rootNode().kind()"),
        "named_children_count" => format!("{result_var}.rootNode().namedChildCount()"),
        "has_error_nodes" => format!("{class_name}.treeHasErrorNodes({result_var})"),
        "error_count" | "tree_error_count" => format!("{class_name}.treeErrorCount({result_var})"),
        "tree_to_sexp" => format!("{class_name}.treeToSexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{class_name}.treeContainsNodeType({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{class_name}.findNodesByType({result_var}, \"{node_type}\")")
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
            let escaped_query = escape_java(query_source);
            format!("{class_name}.runQuery({result_var}, \"{language}\", \"{escaped_query}\", source)")
        }
        _ => {
            format!("{result_var}.{}()", method_name.to_lower_camel_case())
        }
    }
}
