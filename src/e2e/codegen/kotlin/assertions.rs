//! Kotlin assertion rendering helpers.

use heck::ToLowerCamelCase;
use std::fmt::Write as FmtWrite;

use crate::e2e::escape::escape_kotlin;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    _class_name: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_option: bool,
    enum_fields: &std::collections::HashSet<String>,
    fields_c_types: &std::collections::HashMap<String, String>,
    is_streaming: bool,
    kotlin_android_style: bool,
) {
    // In streaming context, `usage` and `usage.*` fields must be read from the
    // last collected chunk, not from the stream iterator (which has no `usage()` method).
    // Route them through `StreamingFieldResolver::accessor("usage", ...)` + deep-tail
    // rendering, using `chunks.last().usage()` as the base expression.
    if is_streaming {
        if let Some(f) = &assertion.field {
            if f == "usage" || f.starts_with("usage.") {
                let stream_lang = if kotlin_android_style {
                    "kotlin_android"
                } else {
                    "kotlin"
                };
                let base_expr = crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(
                    "usage",
                    stream_lang,
                    "chunks",
                )
                .unwrap_or_else(|| {
                    if kotlin_android_style {
                        "(if (chunks.isEmpty()) null else chunks.last().usage)".to_string()
                    } else {
                        "(if (chunks.isEmpty()) null else chunks.last().usage())".to_string()
                    }
                });

                // For a deep path like `usage.total_tokens`, render the tail `.total_tokens`
                // in a language-appropriate accessor style.
                let expr = if let Some(tail) = f.strip_prefix("usage.") {
                    if kotlin_android_style {
                        // kotlin-android: data classes use Kotlin property access (no parens).
                        tail.split('.')
                            .fold(base_expr, |acc, seg| format!("{acc}?.{}", seg.to_lower_camel_case()))
                    } else {
                        // Kotlin/Java: accessor methods have parens.
                        tail.split('.')
                            .fold(base_expr, |acc, seg| format!("{acc}?.{}()", seg.to_lower_camel_case()))
                    }
                } else {
                    base_expr
                };

                // Determine if the field maps to a 64-bit C type requiring `L` suffix.
                let field_is_long = fields_c_types
                    .get(f.as_str())
                    .is_some_and(|t| matches!(t.as_str(), "uint64_t" | "int64_t"));

                let line = match assertion.assertion_type.as_str() {
                    "equals" => {
                        if let Some(expected) = &assertion.value {
                            let kotlin_val = if field_is_long && expected.is_number() && !expected.is_f64() {
                                format!("{}L", expected)
                            } else {
                                super::values::json_to_kotlin(expected)
                            };
                            format!("        assertEquals({kotlin_val}, {expr}!!)\n")
                        } else {
                            String::new()
                        }
                    }
                    _ => String::new(),
                };
                if !line.is_empty() {
                    out.push_str(&line);
                }
                return;
            }
        }
    }

    // Streaming virtual fields resolve against the `chunks` collected-list variable.
    // Intercept before is_valid_for_result so they are never skipped.
    // Gate on `is_streaming` so non-streaming fixtures (e.g. consumers whose real
    // result struct has a literal `chunks` field) don't divert into the virtual
    // accessor path — they should fall through to the normal field resolver.
    if let Some(f) = &assertion.field {
        if is_streaming && !f.is_empty() && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
            let stream_lang = if kotlin_android_style {
                "kotlin_android"
            } else {
                "kotlin"
            };
            if let Some(expr) =
                crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, stream_lang, "chunks")
            {
                let line = match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertTrue({expr}.size >= {n}, \"expected >= {n} chunks\")\n")
                        } else {
                            String::new()
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!(
                                "        assertEquals({n}.toLong(), {expr}.size.toLong(), \"expected exactly {n} elements\")\n"
                            )
                        } else {
                            String::new()
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_kotlin(s);
                            format!("        assertEquals(\"{escaped}\", {expr})\n")
                        } else if let Some(b) = assertion.value.as_ref().and_then(|v| v.as_bool()) {
                            format!("        assertEquals({b}, {expr})\n")
                        } else {
                            String::new()
                        }
                    }
                    "not_empty" => {
                        format!("        assertFalse({expr}.isEmpty(), \"expected non-empty\")\n")
                    }
                    "is_empty" => {
                        format!("        assertTrue({expr}.isEmpty(), \"expected empty\")\n")
                    }
                    "is_true" => {
                        format!("        assertTrue({expr}, \"expected true\")\n")
                    }
                    "is_false" => {
                        format!("        assertFalse({expr}, \"expected false\")\n")
                    }
                    "greater_than" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        assertTrue({expr} > {n}, \"expected > {n}\")\n")
                        } else {
                            String::new()
                        }
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_kotlin(s);
                            // Use `.toString().lowercase().contains(...)` to mirror the Java
                            // emitter — `(list as List<String>)` is an unchecked cast that
                            // succeeds at runtime via erasure but `.contains("Module")` then
                            // compares `StructureItem`s against a `String` and always returns
                            // `false`. Stringifying the collection lets the assertion match
                            // both `List<String>` and `List<ComplexType>` cases uniformly.
                            format!(
                                "        assertTrue({expr}.toString().lowercase().contains(\"{escaped}\".lowercase()), \"expected to contain: {escaped}\")\n"
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
            let _ = writeln!(out, "        // skipped: field '{f}' not available on result type");
            return;
        }
    }

    // Discriminated-union navigation (sealed `FormatMetadata` in Kotlin).
    // Field paths like `metadata.format.excel.sheet_count` cannot be expressed as
    // a flat property chain because `FormatMetadata` is a sealed class with
    // variant subclasses (`FormatMetadata.Excel`, `FormatMetadata.Pdf`, …); each
    // variant exposes its payload through a `.metadata` property of the variant
    // type.  Emit an `is`-pattern `when` block that binds the variant, then
    // delegate the leaf assertion to `render_discriminated_union_assertion`.
    if kotlin_android_style {
        if let Some(f) = assertion.field.as_deref().filter(|f| !f.is_empty()) {
            if let Some((variant_pascal, inner_field)) = super::discriminated::parse_discriminated_union_access(f) {
                let variant_var = format!("format{variant_pascal}");
                let _ = writeln!(
                    out,
                    "        when (val {variant_var} = {result_var}.metadata.format) {{"
                );
                let _ = writeln!(out, "            is FormatMetadata.{variant_pascal} -> {{");
                super::discriminated::render_discriminated_union_assertion(out, assertion, &variant_var, &inner_field);
                let _ = writeln!(out, "            }}");
                let _ = writeln!(out, "            else -> {{}}");
                let _ = writeln!(out, "        }}");
                return;
            }
        }
    }

    // Determine if this field is an enum type.
    let field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));

    // Raw field accessor — may end with nullable type if field is optional.
    // kotlin_android data classes expose properties (no parens), so use the
    // dedicated "kotlin_android" language key for the accessor renderer.
    let accessor_lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, accessor_lang, result_var),
            _ => result_var.to_string(),
        }
    };

    // Whether the accessor may return a nullable type in Kotlin. This is true
    // when the leaf field OR any intermediate segment in the path is optional
    // (the `?.` safe-call propagates null through the whole chain).
    //
    // Additionally, if the generated accessor expression itself contains `?.`
    // then the return type is `T?` regardless of what the path-resolver says —
    // sticky nullability means any `?.` in the chain makes the whole expression
    // nullable. This handles cases like `toolCalls()?.first()?.function()?.name()`
    // where the `is_optional` prefix lookup misses due to index notation mismatch.
    let field_is_optional = !result_is_simple
        && (field_expr.contains("?.")
            || assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
                let resolved = field_resolver.resolve(f);
                if field_resolver.has_map_access(f) {
                    // Kotlin's `Map<K, V>.get(key)` always returns `V?`. In the
                    // kotlin_android target, DTOs are pure Kotlin data classes so
                    // the nullable propagates through and string operations on
                    // the result must coalesce or safe-call. In the kotlin/JVM
                    // target the same map field flows through Java records and
                    // appears as a platform type, so adding `.orEmpty()` is
                    // unnecessary but harmless — keep the legacy behaviour for
                    // JVM to avoid churning unrelated snapshots.
                    return kotlin_android_style;
                }
                // Check the leaf field itself.
                if field_resolver.is_optional(resolved) {
                    return true;
                }
                // Also check every prefix segment: if any intermediate field is
                // optional the ?.  chain propagates null to the final result.
                let mut prefix = String::new();
                for part in resolved.split('.') {
                    // Strip array notation for the lookup key.
                    let key = part.split('[').next().unwrap_or(part);
                    if !prefix.is_empty() {
                        prefix.push('.');
                    }
                    prefix.push_str(key);
                    if field_resolver.is_optional(&prefix) {
                        return true;
                    }
                }
                false
            }));

    // String-context expression: append .orEmpty() for nullable string fields so
    // string operations (contains, trim) don't require a safe-call chain.
    // Note: this is only sound when the leaf type is `String?`. For enum-typed
    // optional fields (`T?` where `T` is an enum class), `.orEmpty()` is undefined;
    // the enum branch below handles those by going through `?.getValue()` first.
    // Also handle the case where the bare result (no field specified) is nullable
    // due to `result_is_option` being true.
    let bare_result_is_nullable = result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
    let string_field_expr = if field_is_optional || bare_result_is_nullable {
        format!("{field_expr}.orEmpty()")
    } else {
        field_expr.clone()
    };

    // Non-null expression: use !! to assert presence for numeric comparisons where
    // the fixture guarantees the value is non-null.
    let nonnull_field_expr = if field_is_optional {
        format!("{field_expr}!!")
    } else {
        field_expr.clone()
    };

    // For enum fields, convert to string for comparison.
    //
    // - JVM (kotlin) mode: The Java facade wraps enums in a Java enum type that
    //   exposes a `.getValue()` accessor. Use `.getValue()` (with optional-safe
    //   variant when the field is nullable), mirroring the Java codegen pattern
    //   `Optional.ofNullable(...).map(v -> v.getValue()).orElse("")`.
    //
    // - kotlin_android mode: Enums are plain Kotlin `enum class` values with no
    //   `.getValue()` method. Serialize to the lowercase wire string via
    //   `.name.lowercase()`, which maps `FinishReason.STOP` → `"stop"` and
    //   `FinishReason.TOOL_CALLS` → `"tool_calls"`, matching the JSON wire values.
    let string_expr = if kotlin_android_style {
        match (field_is_enum, field_is_optional) {
            (true, true) => format!("{field_expr}?.name?.lowercase().orEmpty()"),
            (true, false) => format!("{field_expr}.name.lowercase()"),
            (false, _) => string_field_expr.clone(),
        }
    } else {
        match (field_is_enum, field_is_optional) {
            (true, true) => format!("{field_expr}?.getValue().orEmpty()"),
            (true, false) => format!("{field_expr}.getValue()"),
            (false, _) => string_field_expr.clone(),
        }
    };

    // Determine if this assertion field maps to a 64-bit C type (uint64_t / int64_t),
    // which corresponds to Kotlin `Long`. When true, integer literals must be suffixed
    // with `L` to avoid a type mismatch between Kotlin `Int` and `Long`.
    let field_is_long = assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        matches!(
            fields_c_types.get(resolved).map(String::as_str),
            Some("uint64_t") | Some("int64_t")
        )
    });

    // Determine whether the field's underlying type is a list/collection. For
    // `contains` / `contains_all` / `not_contains` assertions on `List<String>`
    // fields Kotlin requires a cast to `List<String>` so the `@OnlyInputTypes`
    // annotation on `Collection.contains()` can infer `T`. For plain `String`
    // fields (e.g. `result.text` on TranscribeTest) the assertion is a
    // substring check on a `String` — emitting `(s as List<String>).contains`
    // throws ClassCastException at runtime, so the cast must be gated on the
    // field actually being a collection. `field_resolver.is_array` is true for
    // paths in `fields_array`; `is_collection_root` is true when the field is
    // a top-level collection accessor (e.g. `tags` whose entries are tracked
    // as `tags[0]` in `fields_array`).
    let field_is_collection = assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        field_resolver.is_array(f)
            || field_resolver.is_array(resolved)
            || field_resolver.is_collection_root(f)
            || field_resolver.is_collection_root(resolved)
    });

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                // Suffix integer literals with `L` when the target field is a Java `long`
                // (uint64_t / int64_t in C FFI terms). Without the suffix, Kotlin infers
                // the literal as `Int`, causing a type mismatch with `Long` at runtime.
                let kotlin_val = if field_is_long && expected.is_number() && !expected.is_f64() {
                    format!("{}L", expected)
                } else {
                    super::values::json_to_kotlin(expected)
                };
                if expected.is_string() {
                    let _ = writeln!(out, "        assertEquals({kotlin_val}, {string_expr}.trim())");
                } else {
                    let _ = writeln!(out, "        assertEquals({kotlin_val}, {nonnull_field_expr})");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = super::values::json_to_kotlin(expected);
                if field_is_collection {
                    // `(list as List<String>)` is an unchecked erasure cast that
                    // succeeds at runtime even for `List<StructureItem>` etc.
                    // `.contains("Module")` then compares records against a
                    // String and always fails. Stringifying the collection
                    // mirrors the Java emitter (`toString().toLowerCase().contains(...)`)
                    // and matches both `List<String>` and `List<ComplexType>`.
                    let _ = writeln!(
                        out,
                        "        assertTrue({string_expr}.toString().lowercase().contains({kotlin_val}.toString().lowercase()), \"expected to contain: \" + {kotlin_val})"
                    );
                } else {
                    // String substring check. Use the field expression directly so
                    // `String.contains(CharSequence)` resolves without a cast.
                    let _ = writeln!(
                        out,
                        "        assertTrue({string_expr}.contains({kotlin_val}), \"expected to contain: \" + {kotlin_val})"
                    );
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let kotlin_val = super::values::json_to_kotlin(val);
                    if field_is_collection {
                        let _ = writeln!(
                            out,
                            "        assertTrue({string_expr}.toString().lowercase().contains({kotlin_val}.toString().lowercase()), \"expected to contain: \" + {kotlin_val})"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "        assertTrue({string_expr}.contains({kotlin_val}), \"expected to contain: \" + {kotlin_val})"
                        );
                    }
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = super::values::json_to_kotlin(expected);
                if field_is_collection {
                    let _ = writeln!(
                        out,
                        "        assertFalse({string_expr}.toString().lowercase().contains({kotlin_val}.toString().lowercase()), \"expected NOT to contain: \" + {kotlin_val})"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "        assertFalse({string_expr}.contains({kotlin_val}), \"expected NOT to contain: \" + {kotlin_val})"
                    );
                }
            }
        }
        "not_empty" => {
            // For optional fields, the field type may be a non-String object
            // (e.g. DocumentStructure) for which `.orEmpty()` is undefined. A
            // null-check is the safe primitive: it works for any reference type
            // and matches the Java codegen's `Optional.ofNullable(...).isEmpty()`.
            // When the bare result is `T?` (result_is_option) the same null-check
            // applies, because `.isEmpty()` is undefined on arbitrary nullable types.
            // The JVM Kotlin e2e tests call the Java facade class which returns
            // `java.util.Optional<T>` for option results — use `.isPresent` rather
            // than `!= null` so the assertion semantics match the JVM return type.
            // The kotlin-android wrapper unwraps `Optional<T>` to Kotlin's `T?`
            // at the boundary, so its bare-option result is a nullable reference
            // and must use `!= null` instead.
            let bare_result_is_option =
                result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
            if bare_result_is_option && !kotlin_android_style {
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr}.isPresent, \"expected non-empty value\")"
                );
            } else if bare_result_is_option || field_is_optional {
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr} != null, \"expected non-empty value\")"
                );
            } else {
                let _ = writeln!(
                    out,
                    "        assertFalse({string_field_expr}.isEmpty(), \"expected non-empty value\")"
                );
            }
        }
        "is_empty" => {
            let bare_result_is_option =
                result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
            if bare_result_is_option && !kotlin_android_style {
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr}.isEmpty, \"expected empty value\")"
                );
            } else if bare_result_is_option || field_is_optional {
                let _ = writeln!(
                    out,
                    "        assertTrue({field_expr} == null, \"expected empty value\")"
                );
            } else {
                let _ = writeln!(
                    out,
                    "        assertTrue({string_field_expr}.isEmpty(), \"expected empty value\")"
                );
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let kotlin_val = super::values::json_to_kotlin(v);
                        format!("{string_expr}.contains({kotlin_val})")
                    })
                    .collect();
                let joined = checks.join(" || ");
                let _ = writeln!(
                    out,
                    "        assertTrue({joined}, \"expected to contain at least one of the specified values\")"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let kotlin_val = super::values::json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({nonnull_field_expr} > {kotlin_val}, \"expected > {kotlin_val}\")"
                );
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let kotlin_val = super::values::json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({nonnull_field_expr} < {kotlin_val}, \"expected < {kotlin_val}\")"
                );
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let kotlin_val = super::values::json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({nonnull_field_expr} >= {kotlin_val}, \"expected >= {kotlin_val}\")"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let kotlin_val = super::values::json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "        assertTrue({nonnull_field_expr} <= {kotlin_val}, \"expected <= {kotlin_val}\")"
                );
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = super::values::json_to_kotlin(expected);
                let _ = writeln!(
                    out,
                    "        assertTrue({string_expr}.startsWith({kotlin_val}), \"expected to start with: \" + {kotlin_val})"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = super::values::json_to_kotlin(expected);
                let _ = writeln!(
                    out,
                    "        assertTrue({string_expr}.endsWith({kotlin_val}), \"expected to end with: \" + {kotlin_val})"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    // For simple result types (ByteArray), use .size; for String use .length
                    let length_accessor = if result_is_simple && field_expr == result_var {
                        "size"
                    } else {
                        "length"
                    };
                    let _ = writeln!(
                        out,
                        "        assertTrue({string_field_expr}.{length_accessor} >= {n}, \"expected {length_accessor} >= {n}\")"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    // For simple result types (ByteArray), use .size; for String use .length
                    let length_accessor = if result_is_simple && field_expr == result_var {
                        "size"
                    } else {
                        "length"
                    };
                    let _ = writeln!(
                        out,
                        "        assertTrue({string_field_expr}.{length_accessor} <= {n}, \"expected {length_accessor} <= {n}\")"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        assertTrue({nonnull_field_expr}.size >= {n}, \"expected at least {n} elements\")"
                    );
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "        assertEquals({n}, {nonnull_field_expr}.size, \"expected exactly {n} elements\")"
                    );
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "        assertTrue({field_expr}, \"expected true\")");
        }
        "is_false" => {
            let _ = writeln!(out, "        assertFalse({field_expr}, \"expected false\")");
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = super::values::json_to_kotlin(expected);
                let _ = writeln!(
                    out,
                    "        assertTrue(Regex({kotlin_val}).containsMatchIn({string_expr}), \"expected value to match regex: \" + {kotlin_val})"
                );
            }
        }
        "not_error" => {
            // Already handled by the call succeeding without exception.
        }
        "error" => {
            // Handled at the test method level.
        }
        "method_result" => {
            // Placeholder: Kotlin support for method_result would need sample_language integration.
            let _ = writeln!(
                out,
                "        // method_result assertions not yet implemented for Kotlin"
            );
        }
        other => {
            panic!("Kotlin e2e generator: unsupported assertion type: {other}");
        }
    }
}
