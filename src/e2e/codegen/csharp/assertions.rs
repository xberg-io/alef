//! C# assertion rendering for generated e2e tests.

use crate::e2e::escape::escape_csharp;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};

use super::{
    build_csharp_method_call, json_to_csharp, parse_discriminated_union_access, render_discriminated_union_assertion,
};

fn render_synthetic_bool_assertion(out: &mut String, field: &str, assertion_type: &str, pred: String) -> bool {
    let pred_type = match assertion_type {
        "is_true" => "is_true",
        "is_false" => "is_false",
        _ => {
            out.push_str(&format!(
                "        // skipped: unsupported assertion type on synthetic field '{field}'\n"
            ));
            return false;
        }
    };
    out.push_str(&crate::e2e::template_env::render(
        "csharp/assertion.jinja",
        minijinja::context! { assertion_type => "synthetic_assertion", synthetic_pred => pred, synthetic_pred_type => pred_type },
    ));
    true
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    class_name: &str,
    exception_class: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_vec: bool,
    result_is_array: bool,
    result_is_bytes: bool,
    fields_enum: &std::collections::HashSet<String>,
    assert_enum_fields: &std::collections::HashMap<String, String>,
) {
    // Byte-buffer returns: emit length-based assertions instead of struct-field
    // accessors. The result is a `byte[]` and has no named fields like
    // `result.Audio` or `result.Content`.
    if result_is_bytes {
        match assertion.assertion_type.as_str() {
            "not_empty" => {
                let _ = writeln!(out, "        Assert.NotEmpty({result_var});");
                return;
            }
            "is_empty" => {
                let _ = writeln!(out, "        Assert.Empty({result_var});");
                return;
            }
            "count_equals" | "length_equals" => {
                if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                    let _ = writeln!(out, "        Assert.Equal({n}, {result_var}.Length);");
                }
                return;
            }
            "count_min" | "length_min" => {
                if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                    let _ = writeln!(out, "        Assert.True({result_var}.Length >= {n});");
                }
                return;
            }
            "not_error" => {
                let _ = writeln!(out, "        Assert.NotNull({result_var});");
                return;
            }
            _ => {
                // Other assertion types are not meaningful on raw byte buffers;
                // emit a comment so the test still compiles but flags unsupported
                // assertion types for fixture authors.
                let _ = writeln!(
                    out,
                    "        // skipped: assertion type '{}' not supported on byte[] result",
                    assertion.assertion_type
                );
                return;
            }
        }
    }
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct property accesses on the result.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                render_synthetic_bool_assertion(
                    out,
                    f,
                    &assertion.assertion_type,
                    format!("({result_var}.Chunks ?? new()).All(c => !string.IsNullOrEmpty(c.Content))"),
                );
                return;
            }
            "chunks_have_embeddings" => {
                render_synthetic_bool_assertion(
                    out,
                    f,
                    &assertion.assertion_type,
                    format!("({result_var}.Chunks ?? new()).All(c => c.Embedding != null && c.Embedding.Count > 0)"),
                );
                return;
            }
            "chunks_have_heading_context" => {
                render_synthetic_bool_assertion(
                    out,
                    f,
                    &assertion.assertion_type,
                    format!("({result_var}.Chunks ?? new()).All(c => c.Metadata?.HeadingContext != null)"),
                );
                return;
            }
            "first_chunk_starts_with_heading" => {
                render_synthetic_bool_assertion(
                    out,
                    f,
                    &assertion.assertion_type,
                    format!("({result_var}.Chunks ?? new()).FirstOrDefault()?.Metadata?.HeadingContext != null"),
                );
                return;
            }
            // ---- EmbedResponse virtual fields ----
            // embed_texts returns List<List<float>> in C# — no wrapper object.
            // result_var is the embedding matrix; use it directly.
            "embeddings" => {
                match assertion.assertion_type.as_str() {
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            if let Some(n) = val.as_u64() {
                                let rendered = crate::e2e::template_env::render(
                                    "csharp/assertion.jinja",
                                    minijinja::context! {
                                        assertion_type => "synthetic_embeddings_count_equals",
                                        synthetic_pred => format!("{result_var}.Count"),
                                        n => n,
                                    },
                                );
                                out.push_str(&rendered);
                            }
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            if let Some(n) = val.as_u64() {
                                let rendered = crate::e2e::template_env::render(
                                    "csharp/assertion.jinja",
                                    minijinja::context! {
                                        assertion_type => "synthetic_embeddings_count_min",
                                        synthetic_pred => format!("{result_var}.Count"),
                                        n => n,
                                    },
                                );
                                out.push_str(&rendered);
                            }
                        }
                    }
                    "not_empty" => {
                        let rendered = crate::e2e::template_env::render(
                            "csharp/assertion.jinja",
                            minijinja::context! {
                                assertion_type => "synthetic_embeddings_not_empty",
                                synthetic_pred => result_var.to_string(),
                            },
                        );
                        out.push_str(&rendered);
                    }
                    "is_empty" => {
                        let rendered = crate::e2e::template_env::render(
                            "csharp/assertion.jinja",
                            minijinja::context! {
                                assertion_type => "synthetic_embeddings_is_empty",
                                synthetic_pred => result_var.to_string(),
                            },
                        );
                        out.push_str(&rendered);
                    }
                    _ => {
                        out.push_str(
                            "        // skipped: unsupported assertion type on synthetic field 'embeddings'\n",
                        );
                    }
                }
                return;
            }
            "embedding_dimensions" => {
                let expr = format!("({result_var}.Count > 0 ? {result_var}[0].Count : 0)");
                match assertion.assertion_type.as_str() {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            if let Some(n) = val.as_u64() {
                                let rendered = crate::e2e::template_env::render(
                                    "csharp/assertion.jinja",
                                    minijinja::context! {
                                        assertion_type => "synthetic_embedding_dimensions_equals",
                                        synthetic_pred => expr,
                                        n => n,
                                    },
                                );
                                out.push_str(&rendered);
                            }
                        }
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value {
                            if let Some(n) = val.as_u64() {
                                let rendered = crate::e2e::template_env::render(
                                    "csharp/assertion.jinja",
                                    minijinja::context! {
                                        assertion_type => "synthetic_embedding_dimensions_greater_than",
                                        synthetic_pred => expr,
                                        n => n,
                                    },
                                );
                                out.push_str(&rendered);
                            }
                        }
                    }
                    _ => {
                        out.push_str("        // skipped: unsupported assertion type on synthetic field 'embedding_dimensions'\n");
                    }
                }
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                let synthetic_pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!("{result_var}.All(e => e.Count > 0)")
                    }
                    "embeddings_finite" => {
                        format!("{result_var}.All(e => e.All(v => !float.IsInfinity(v) && !float.IsNaN(v)))")
                    }
                    "embeddings_non_zero" => {
                        format!("{result_var}.All(e => e.Any(v => v != 0.0f))")
                    }
                    "embeddings_normalized" => {
                        format!(
                            "{result_var}.All(e => {{ var n = e.Sum(v => (double)v * v); return Math.Abs(n - 1.0) < 1e-3; }})"
                        )
                    }
                    _ => unreachable!(),
                };
                let synthetic_pred_type = match assertion.assertion_type.as_str() {
                    "is_true" => "is_true",
                    "is_false" => "is_false",
                    _ => {
                        out.push_str(&format!(
                            "        // skipped: unsupported assertion type on synthetic field '{f}'\n"
                        ));
                        return;
                    }
                };
                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "synthetic_assertion",
                        synthetic_pred => synthetic_pred,
                        synthetic_pred_type => synthetic_pred_type,
                    },
                );
                out.push_str(&rendered);
                return;
            }
            // ---- keywords / keywords_count ----
            // The generated C# result type does not expose this fixture alias; skip.
            "keywords" | "keywords_count" => {
                let skipped_reason = format!("field '{f}' not available on the generated C# result type");
                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        skipped_reason => skipped_reason,
                    },
                );
                out.push_str(&rendered);
                return;
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let skipped_reason = format!("field '{f}' not available on result type");
            let rendered = crate::e2e::template_env::render(
                "csharp/assertion.jinja",
                minijinja::context! {
                    skipped_reason => skipped_reason,
                },
            );
            out.push_str(&rendered);
            return;
        }
    }

    // For count assertions on list results with no field specified, use the list directly.
    // Otherwise, when the result is a List<T>, index into the first element for field access.
    let is_count_assertion = matches!(
        assertion.assertion_type.as_str(),
        "count_equals" | "count_min" | "count_max"
    );
    let is_no_field = assertion.field.is_none() || assertion.field.as_ref().is_some_and(|f| f.is_empty());
    let use_list_directly = result_is_vec && is_count_assertion && is_no_field;

    let effective_result_var: String = if result_is_vec && !use_list_directly {
        format!("{result_var}[0]")
    } else {
        result_var.to_string()
    };

    // Check if this is a discriminated union access (e.g., metadata.format.excel.sheet_count)
    let is_discriminated_union = assertion
        .field
        .as_ref()
        .is_some_and(|f| parse_discriminated_union_access(f).is_some());

    // For discriminated union assertions, generate pattern-matching wrapper
    if is_discriminated_union {
        if let Some((_, variant_name, inner_field)) = assertion
            .field
            .as_ref()
            .and_then(|f| parse_discriminated_union_access(f))
        {
            // Use a unique variable name based on the field hash to avoid shadowing
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            inner_field.hash(&mut hasher);
            let var_hash = format!("{:x}", hasher.finish());
            let variant_var = format!("variant_{}", &var_hash[..8]);
            // Resolve the discriminated-union container (`…Metadata.Format`) through the
            // field resolver so list-result field paths (`results[0].metadata.format.…`)
            // index into `.Results[0]` exactly like the flat-field assertions do, instead
            // of hardcoding `{effective_result_var}.Metadata.Format` (which assumes the
            // metadata lives on the top-level `ExtractionResult`, breaking batch results).
            let container = assertion
                .field
                .as_ref()
                .map(|f| {
                    let format_path = match f.find(".format") {
                        Some(idx) => &f[..idx + ".format".len()],
                        None => f.as_str(),
                    };
                    field_resolver.accessor(format_path, "csharp", &effective_result_var)
                })
                .unwrap_or_else(|| format!("{effective_result_var}.Metadata.Format"));
            let _ = writeln!(
                out,
                "        if ({container} is FormatMetadata.{} {})",
                variant_name, &variant_var
            );
            let _ = writeln!(out, "        {{");
            render_discriminated_union_assertion(
                out,
                assertion,
                &variant_var,
                &inner_field,
                result_is_vec,
                assert_enum_fields,
            );
            let _ = writeln!(out, "        }}");
            let _ = writeln!(out, "        else");
            let _ = writeln!(out, "        {{");
            let _ = writeln!(
                out,
                "            Assert.Fail(\"Expected {} format metadata\");",
                variant_name.to_lowercase()
            );
            let _ = writeln!(out, "        }}");
            return;
        }
    }

    let field_expr = if result_is_simple {
        effective_result_var.clone()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "csharp", &effective_result_var),
            _ => effective_result_var.clone(),
        }
    };

    // Fields declared in `assert_enum_fields` map to sealed/internally-tagged enum
    // types. Wrap the accessor with a display helper (e.g., `FormatMetadataDisplay.ToDisplayString`)
    // so the assertion sees a display string rather than the raw sealed-union object.
    let field_expr = match &assertion.field {
        Some(f) if assert_enum_fields.contains_key(f.as_str()) => {
            let type_name = assert_enum_fields.get(f.as_str()).unwrap();
            format!("{type_name}Display.ToDisplayString({field_expr})")
        }
        _ => field_expr,
    };

    // Determine if field_expr is a list or complex object that requires JSON serialization
    // for string-based assertions (contains, not_contains, etc.). List<T>.ToString() in C#
    // returns the type name, not the contents.
    let field_needs_json_serialize = if result_is_simple {
        // Simple results are scalars, but when they're also arrays (e.g., List<string>),
        // JSON-serialize so substring checks see actual content, not the type name.
        result_is_array
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.is_array(f),
            // No field specified — the whole result object; needs serialization when complex.
            _ => !result_is_simple,
        }
    };
    // Build the string representation of field_expr for substring-based assertions.
    // For display_as_text fields, use the `.Text()` text accessor instead of
    // `.ToString()` to get the textual content rather than the type-name representation.
    let field_as_str = if field_needs_json_serialize {
        format!("JsonSerializer.Serialize({field_expr})")
    } else if assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| field_resolver.is_display_as_text(f))
    {
        format!("({field_expr}?.Text() ?? \"\")")
    } else {
        format!("{field_expr}.ToString()")
    };

    // Detect enum-typed fields. C# emits typed enums (e.g. `FinishReason?`) for
    // these so the codegen must avoid `.Trim()` (string-only) and instead
    // compare via `?.ToString()?.ToLower()` to match snake_case JSON.
    let field_is_enum = assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        fields_enum.contains(f) || fields_enum.contains(resolved)
    });

    // Detect display-as-text fields (e.g. `RichTextContent?`). Their C# binding
    // exposes a `.Text()` method returning `string?`. Wrap the accessor so that
    // string equality and substring assertions compare the textual representation
    // rather than the C# object reference, which `.ToString()` / `!.Trim()` would
    // mishandle for non-String types.
    let field_is_display_as_text = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| field_resolver.is_display_as_text(f));
    // For display_as_text fields, produce a string expression via `.Text()` so
    // `equals` and `contains` assertions see a plain string. The `?.` null-safe
    // chain means a `null` field yields `null`, which both `Assert.Equal` (with
    // explicit null expected) and string methods handle gracefully.
    let field_expr_for_text = if field_is_display_as_text {
        format!("{field_expr}?.Text()")
    } else {
        field_expr.clone()
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                // Enum field equality bypasses the template (which would emit `.Trim()`,
                // a string-only API). Compare the snake-cased ToString() against the
                // expected value to match the wire JSON form (`InProgress` → `in_progress`,
                // `ContentFilter` → `content_filter`, etc.). `JsonNamingPolicy.SnakeCaseLower`
                // is the same policy used by the global JsonStringEnumConverter, so the
                // assertion compares against exactly what serde would emit.
                if field_is_enum && expected.is_string() {
                    let s_lower = expected.as_str().map(|s| s.to_lowercase()).unwrap_or_default();
                    let _ = writeln!(
                        out,
                        "        Assert.Equal(\"{}\", {field_expr} == null ? null : JsonNamingPolicy.SnakeCaseLower.ConvertName({field_expr}.ToString()!));",
                        escape_csharp(&s_lower)
                    );
                    return;
                }
                let cs_val = json_to_csharp(expected);
                let is_string_val = expected.is_string();
                let is_bool_true = expected.as_bool() == Some(true);
                let is_bool_false = expected.as_bool() == Some(false);
                let is_integer_val = expected.is_number() && !expected.as_f64().is_some_and(|f| f.fract() != 0.0);

                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "equals",
                        // Use the text-accessor expression for display_as_text fields so that
                        // `Assert.Equal(expected, field_expr!.Trim())` calls `.Text()` instead
                        // of `.Trim()` on the raw content-union type.
                        field_expr => field_expr_for_text.clone(),
                        cs_val => cs_val,
                        is_string_val => is_string_val,
                        is_bool_true => is_bool_true,
                        is_bool_false => is_bool_false,
                        is_integer_val => is_integer_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                // Lowercase both expected and actual so that enum fields (where .ToString()
                // returns the PascalCase C# member name like "Anchor") correctly match
                // fixture snake_case values like "anchor".  String fields are unaffected
                // because lowercasing both sides preserves substring matches.
                // List/complex fields use JsonSerializer.Serialize() since List<T>.ToString()
                // returns the type name, not the contents.
                let lower_expected = expected.as_str().map(|s| s.to_lowercase());
                let cs_val = lower_expected
                    .as_deref()
                    .map(|s| format!("\"{}\"", escape_csharp(s)))
                    .unwrap_or_else(|| json_to_csharp(expected));

                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains",
                        field_as_str => field_as_str.clone(),
                        cs_val => cs_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                let values_cs_lower: Vec<String> = values
                    .iter()
                    .map(|val| {
                        let lower_val = val.as_str().map(|s| s.to_lowercase());
                        lower_val
                            .as_deref()
                            .map(|s| format!("\"{}\"", escape_csharp(s)))
                            .unwrap_or_else(|| json_to_csharp(val))
                    })
                    .collect();

                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains_all",
                        field_as_str => field_as_str.clone(),
                        values_cs_lower => values_cs_lower,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let cs_val = json_to_csharp(expected);

                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "not_contains",
                        field_as_str => field_as_str.clone(),
                        cs_val => cs_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_empty" => {
            // Detect non-nullable: if expression has ! operator or is a method call
            let field_is_nullable = !field_expr.contains('!') && !field_expr.contains(")");
            let rendered = crate::e2e::template_env::render(
                "csharp/assertion.jinja",
                minijinja::context! {
                    assertion_type => "not_empty",
                    field_expr => field_expr.clone(),
                    field_needs_json_serialize => field_needs_json_serialize,
                    field_is_nullable => field_is_nullable,
                },
            );
            out.push_str(&rendered);
        }
        "is_empty" => {
            // Detect non-nullable: if expression has ! operator or is a method call
            let field_is_nullable = !field_expr.contains('!') && !field_expr.contains(")");
            let rendered = crate::e2e::template_env::render(
                "csharp/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_empty",
                    field_expr => field_expr.clone(),
                    field_needs_json_serialize => field_needs_json_serialize,
                    field_is_nullable => field_is_nullable,
                },
            );
            out.push_str(&rendered);
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let cs_val = json_to_csharp(v);
                        format!("{field_as_str}.Contains({cs_val})")
                    })
                    .collect();
                let contains_any_expr = checks.join(" || ");

                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains_any",
                        contains_any_expr => contains_any_expr,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let cs_val = json_to_csharp(val);

                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "greater_than",
                        field_expr => field_expr.clone(),
                        cs_val => cs_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let cs_val = json_to_csharp(val);

                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "less_than",
                        field_expr => field_expr.clone(),
                        cs_val => cs_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let cs_val = json_to_csharp(val);

                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "greater_than_or_equal",
                        field_expr => field_expr.clone(),
                        cs_val => cs_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let cs_val = json_to_csharp(val);

                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "less_than_or_equal",
                        field_expr => field_expr.clone(),
                        cs_val => cs_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let cs_val = json_to_csharp(expected);

                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "starts_with",
                        field_expr => field_expr.clone(),
                        cs_val => cs_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let cs_val = json_to_csharp(expected);

                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "ends_with",
                        field_expr => field_expr.clone(),
                        cs_val => cs_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let rendered = crate::e2e::template_env::render(
                        "csharp/assertion.jinja",
                        minijinja::context! {
                            assertion_type => "min_length",
                            field_expr => field_expr.clone(),
                            n => n,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let rendered = crate::e2e::template_env::render(
                        "csharp/assertion.jinja",
                        minijinja::context! {
                            assertion_type => "max_length",
                            field_expr => field_expr.clone(),
                            n => n,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let rendered = crate::e2e::template_env::render(
                        "csharp/assertion.jinja",
                        minijinja::context! {
                            assertion_type => "count_min",
                            field_expr => field_expr.clone(),
                            n => n,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let rendered = crate::e2e::template_env::render(
                        "csharp/assertion.jinja",
                        minijinja::context! {
                            assertion_type => "count_equals",
                            field_expr => field_expr.clone(),
                            n => n,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
        }
        "is_true" => {
            // When the field expression is not a simple bool (e.g., a complex object or
            // result type), use Assert.NotNull instead of Assert.True to avoid cast issues.
            // If it's clearly not a boolean type (contains null-checking operators or is a
            // complex object), treat it as a not-null check.
            let is_complex_or_object = field_expr.contains("(object)")
                || (field_expr.contains(".")
                    && !result_is_simple
                    && !field_expr.contains("?")
                    && !field_expr.contains("=="));

            let rendered = if is_complex_or_object {
                crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "not_empty",
                        field_expr => field_expr.clone(),
                        field_needs_json_serialize => false,
                    },
                )
            } else {
                crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "is_true",
                        field_expr => field_expr.clone(),
                    },
                )
            };
            out.push_str(&rendered);
        }
        "is_false" => {
            let is_complex_or_object = field_expr.contains("(object)")
                || (field_expr.contains(".")
                    && !result_is_simple
                    && !field_expr.contains("?")
                    && !field_expr.contains("=="));

            let rendered = if is_complex_or_object {
                // For complex types, is_false means "is empty/null"
                crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "is_empty",
                        field_expr => field_expr.clone(),
                        field_needs_json_serialize => false,
                    },
                )
            } else {
                crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "is_false",
                        field_expr => field_expr.clone(),
                    },
                )
            };
            out.push_str(&rendered);
        }
        "not_error" => {
            // Already handled by the call succeeding without exception.
            let rendered = crate::e2e::template_env::render(
                "csharp/assertion.jinja",
                minijinja::context! {
                    assertion_type => "not_error",
                },
            );
            out.push_str(&rendered);
        }
        "error" => {
            // Handled at the test method level.
            let rendered = crate::e2e::template_env::render(
                "csharp/assertion.jinja",
                minijinja::context! {
                    assertion_type => "error",
                },
            );
            out.push_str(&rendered);
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                let call_expr = build_csharp_method_call(result_var, method_name, assertion.args.as_ref(), class_name);
                let check = assertion.check.as_deref().unwrap_or("is_true");

                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let is_check_bool_true = val.as_bool() == Some(true);
                            let is_check_bool_false = val.as_bool() == Some(false);
                            let cs_check_val = json_to_csharp(val);

                            let rendered = crate::e2e::template_env::render(
                                "csharp/assertion.jinja",
                                minijinja::context! {
                                    assertion_type => "method_result",
                                    check => "equals",
                                    call_expr => call_expr.clone(),
                                    is_check_bool_true => is_check_bool_true,
                                    is_check_bool_false => is_check_bool_false,
                                    cs_check_val => cs_check_val,
                                },
                            );
                            out.push_str(&rendered);
                        }
                    }
                    "is_true" => {
                        let rendered = crate::e2e::template_env::render(
                            "csharp/assertion.jinja",
                            minijinja::context! {
                                assertion_type => "method_result",
                                check => "is_true",
                                call_expr => call_expr.clone(),
                            },
                        );
                        out.push_str(&rendered);
                    }
                    "is_false" => {
                        let rendered = crate::e2e::template_env::render(
                            "csharp/assertion.jinja",
                            minijinja::context! {
                                assertion_type => "method_result",
                                check => "is_false",
                                call_expr => call_expr.clone(),
                            },
                        );
                        out.push_str(&rendered);
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let check_n = val.as_u64().unwrap_or(0);

                            let rendered = crate::e2e::template_env::render(
                                "csharp/assertion.jinja",
                                minijinja::context! {
                                    assertion_type => "method_result",
                                    check => "greater_than_or_equal",
                                    call_expr => call_expr.clone(),
                                    check_n => check_n,
                                },
                            );
                            out.push_str(&rendered);
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let check_n = val.as_u64().unwrap_or(0);

                            let rendered = crate::e2e::template_env::render(
                                "csharp/assertion.jinja",
                                minijinja::context! {
                                    assertion_type => "method_result",
                                    check => "count_min",
                                    call_expr => call_expr.clone(),
                                    check_n => check_n,
                                },
                            );
                            out.push_str(&rendered);
                        }
                    }
                    "is_error" => {
                        let rendered = crate::e2e::template_env::render(
                            "csharp/assertion.jinja",
                            minijinja::context! {
                                assertion_type => "method_result",
                                check => "is_error",
                                call_expr => call_expr.clone(),
                                exception_class => exception_class,
                            },
                        );
                        out.push_str(&rendered);
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let cs_check_val = json_to_csharp(val);

                            let rendered = crate::e2e::template_env::render(
                                "csharp/assertion.jinja",
                                minijinja::context! {
                                    assertion_type => "method_result",
                                    check => "contains",
                                    call_expr => call_expr.clone(),
                                    cs_check_val => cs_check_val,
                                },
                            );
                            out.push_str(&rendered);
                        }
                    }
                    other_check => {
                        panic!("C# e2e generator: unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("C# e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let cs_val = json_to_csharp(expected);

                let rendered = crate::e2e::template_env::render(
                    "csharp/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "matches_regex",
                        field_expr => field_expr.clone(),
                        cs_val => cs_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        other => {
            panic!("C# e2e generator: unsupported assertion type: {other}");
        }
    }
}
