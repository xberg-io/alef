//! C# assertion rendering for generated e2e tests.

use crate::e2e::codegen::assertion_recipes::chunks_result_var;
use crate::e2e::codegen::field_skip::{FieldSkip, nested_wildcard_skip_line};
use crate::e2e::escape::escape_csharp;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};

use super::{build_csharp_method_call, json_to_csharp};

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
    assert_enum_fields: &std::collections::HashMap<String, String>,
    not_error_may_assert_presence: bool,
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
            _ if crate::e2e::codegen::assertion_recipes::chunks_synthetic_skip_reason(f, field_resolver)
                .is_some_and(|skipped_reason| {
                    out.push_str(&crate::e2e::template_env::render(
                        "csharp/assertion.jinja",
                        minijinja::context! { skipped_reason => skipped_reason },
                    ));
                    true
                }) =>
            {
                return;
            }
            "chunks_have_content" => {
                let result_var = &chunks_result_var(field_resolver, "csharp", result_var);
                render_synthetic_bool_assertion(
                    out,
                    f,
                    &assertion.assertion_type,
                    format!("({result_var}.Chunks ?? new()).All(c => !string.IsNullOrEmpty(c.Content))"),
                );
                return;
            }
            "chunks_have_embeddings" => {
                let result_var = &chunks_result_var(field_resolver, "csharp", result_var);
                render_synthetic_bool_assertion(
                    out,
                    f,
                    &assertion.assertion_type,
                    format!("({result_var}.Chunks ?? new()).All(c => c.Embedding != null && c.Embedding.Count > 0)"),
                );
                return;
            }
            "chunks_have_heading_context" => {
                let result_var = &chunks_result_var(field_resolver, "csharp", result_var);
                render_synthetic_bool_assertion(
                    out,
                    f,
                    &assertion.assertion_type,
                    format!("({result_var}.Chunks ?? new()).All(c => c.Metadata?.HeadingContext != null)"),
                );
                return;
            }
            "first_chunk_starts_with_heading" => {
                let result_var = &chunks_result_var(field_resolver, "csharp", result_var);
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
                        if let Some(val) = &assertion.value
                            && let Some(n) = val.as_u64()
                        {
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
                    "count_min" => {
                        if let Some(val) = &assertion.value
                            && let Some(n) = val.as_u64()
                        {
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
                        if let Some(val) = &assertion.value
                            && let Some(n) = val.as_u64()
                        {
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
                    "greater_than" => {
                        if let Some(val) = &assertion.value
                            && let Some(n) = val.as_u64()
                        {
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
                let skipped_reason = FieldSkip::NotAvailableOnGeneratedCsharpResultType.message(f);
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
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        let skipped_reason = FieldSkip::NotAvailableOnResultType.message(f);
        let rendered = crate::e2e::template_env::render(
            "csharp/assertion.jinja",
            minijinja::context! {
                skipped_reason => skipped_reason,
            },
        );
        out.push_str(&rendered);
        return;
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

    if let Some(field) = assertion.field.as_deref().filter(|field| !field.is_empty())
        && super::discriminated::try_render_generic_union_assertion(
            out,
            assertion,
            field_resolver,
            &effective_result_var,
            field,
            assert_enum_fields,
        )
    {
        return;
    }

    // Bracket-wildcard traversal (`links[].link_type`) means "every element". This must run
    // before `field_expr` is built below, since that path lowers the wildcard to index 0 and
    // would assert on a single element while reading as whole-array coverage. ~keep
    if !result_is_simple
        && let Some(f) = assertion.field.as_deref()
        && !f.is_empty()
        && let Some((array_part, elem_part)) = field_resolver.wildcard_split(f)
    {
        render_wildcard_assertion(
            out,
            assertion,
            &effective_result_var,
            field_resolver,
            f,
            &array_part,
            &elem_part,
        );
        return;
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
            // `is_array` alone misses a collection field whose entries are only tracked via
            // their element paths in `fields_array` (e.g. `children[0]` for a recursive
            // `List<DataNode> Children`, never a bare `children` entry) — `is_collection_root`
            // catches that shape. Without it, `field_needs_json_serialize` stayed false for
            // `Children`, so `is_empty`'s template branch fell through to
            // `Assert.True(string.IsNullOrEmpty(field.ToString()))`: `List<T>.ToString()`
            // returns the type name, a non-empty string, so the assertion could never pass.
            // Checked against both the raw and resolved path, matching kotlin/swift's identical
            // `field_is_collection` guard. ~keep
            Some(f) if !f.is_empty() => {
                let resolved = field_resolver.resolve(f);
                field_resolver.is_array(f)
                    || field_resolver.is_array(resolved)
                    || field_resolver.is_collection_root(f)
                    || field_resolver.is_collection_root(resolved)
            }
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
    //
    // `field_resolver.is_enum` checks the hand-maintained `fields_enum` config first, then
    // falls back to the IR-derived classification (`with_ir_enum_map`) when the config never
    // mentioned the field. A config-only check here would default `false` for every enum-typed
    // field a consumer's `alef.toml` never lists, emitting `Assert.Equal("KeyValue", ...)`
    // against a `DataNodeKind` field — a CS1503 the field's declared C# type can't satisfy. ~keep
    let field_is_enum = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| field_resolver.is_enum(f));

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
                // Enum field equality bypasses the template (which would emit `.Trim()`, a
                // string-only API). `gen_enum` always attaches a type-level `[JsonConverter]`
                // with an exact per-variant wire mapping, so `JsonSerializer.Serialize`
                // reproduces the real wire string under any rename policy -- unlike a
                // previous version that guessed via `JsonNamingPolicy.SnakeCaseLower` on both
                // sides and disagreed with itself whenever the wire value wasn't snake_case
                // (e.g. `DataNodeKind`, which has no `rename_all`). ~keep
                if field_is_enum && expected.is_string() {
                    let expected_wire = expected.as_str().unwrap_or_default();
                    let serializer_options = super::UNESCAPED_JSON_SERIALIZER_OPTIONS;
                    let _ = writeln!(
                        out,
                        "        Assert.Equal(\"{}\", {field_expr} == null ? null : System.Text.Json.JsonSerializer.Serialize({field_expr}, {serializer_options}).Trim('\"'));",
                        escape_csharp(expected_wire)
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
                let cs_val = json_to_csharp(expected);

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
                let values_cs_lower: Vec<String> = values.iter().map(json_to_csharp).collect();

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
            for expected in assertion.expected_values() {
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
            // The non-collection arm boxes the field and pattern-matches it, so it needs no
            // nullability guess to stay compilable.
            let rendered = crate::e2e::template_env::render(
                "csharp/assertion.jinja",
                minijinja::context! {
                    assertion_type => "not_empty",
                    field_expr => field_expr.clone(),
                    field_needs_json_serialize => field_needs_json_serialize,
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
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
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
        "max_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
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
        "count_min" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
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
        "count_equals" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
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
            // An uncaught exception already fails the test, but the template's
            // "not_error" branch (`csharp/assertion.jinja`) intentionally renders
            // nothing — before this fix, a fixture whose only assertion was
            // `not_error` left `assertions_body` empty, and the caller
            // (`csharp.rs`'s misleadingly-named `has_usable_assertion`, which is
            // just `!expects_error && !returns_void` and does not actually check
            // whether anything renders) still bound `result_var` and emitted no
            // assertion at all — a vacuous test. Mirror the byte[] branch's
            // existing `Assert.NotNull` idiom instead. Not reached for streaming
            // fixtures: `csharp/test_method.jinja`'s `is_streaming` branch takes
            // priority over `assertions_body` and never references it. ~keep
            //
            // WHETHER to render `Assert.NotNull` at all is decided once, centrally, by
            // `not_error_presence::may_assert_presence` (a sibling assertion or an
            // `Option<T>` result both make it unsafe — see that function's doc); this arm
            // only decides how. ~keep
            if not_error_may_assert_presence {
                let _ = writeln!(out, "        Assert.NotNull({result_var});");
            }
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

#[cfg(test)]
mod ir_oracle_wiring_tests {
    use super::render_assertion;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn make_assertion(field: &str, value: &str) -> Assertion {
        Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Default::default()
        }
    }

    /// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
    /// (present, non-`binding_excluded`, on some IR type) but missing from the
    /// hand-maintained `result_fields` config must still render a real assertion,
    /// not a "skipped: field not available" comment — `csharp.rs`'s per-call
    /// resolver construction now threads `FieldResolver::ir_field_sets(type_defs)`
    /// into `with_ir_fields`. ~keep
    #[test]
    fn csharp_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
        let reachable: HashSet<String> = ["data".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(reachable, HashSet::new(), HashSet::new());
        let assertion = make_assertion("data", "hello");
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            "SampleException",
            &resolver,
            false,
            false,
            false,
            false,
            &HashMap::new(),
            false,
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
    fn csharp_ir_excluded_field_present_in_result_fields_is_still_skipped() {
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
        let assertion = make_assertion("internal_diagnostics", "hello");
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            "SampleException",
            &resolver,
            false,
            false,
            false,
            false,
            &HashMap::new(),
            false,
        );
        assert!(out.contains("skipped"), "got: {out}");
    }
}

#[cfg(test)]
mod not_error_vacuous_test_fix_tests {
    use super::render_assertion;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;

    fn empty_resolver() -> FieldResolver {
        FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        )
    }

    fn not_error_assertion() -> Assertion {
        Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }
    }

    /// Regression test for the not_error vacuous-test defect: `csharp/assertion.jinja`
    /// previously rendered nothing at all for `not_error` ("already handled by call
    /// succeeding"), and the caller's `has_usable_assertion` flag (`csharp.rs`) does
    /// not actually check whether anything renders — it's just `!expects_error &&
    /// !returns_void` — so a fixture whose only assertion was `not_error` bound
    /// `result_var` and asserted nothing. Must emit a real `Assert.NotNull` instead.
    #[test]
    fn not_error_emits_a_real_assert_not_null_on_the_result() {
        let resolver = empty_resolver();
        let assertion = not_error_assertion();
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            "SampleException",
            &resolver,
            false,
            false,
            false,
            false,
            &std::collections::HashMap::new(),
            true,
        );
        assert_eq!(out, "        Assert.NotNull(result);\n");
    }

    /// Control: the byte[] result path already had this idiom (`Assert.NotNull`)
    /// before this fix — confirm the general path now matches it byte-for-byte.
    #[test]
    fn not_error_on_bytes_result_matches_the_general_path_idiom() {
        let resolver = empty_resolver();
        let assertion = not_error_assertion();
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            "SampleException",
            &resolver,
            false,
            false,
            false,
            true,
            &std::collections::HashMap::new(),
            true,
        );
        assert_eq!(out, "        Assert.NotNull(result);\n");
    }
}

/// Lambda-parameter suffix keyed to the assertion.
///
/// Generated C# test methods bind locals named after fixture fields, and two wildcard
/// assertions in one method must not reuse a parameter name that shadows one. Hashing the
/// assertion's discriminating fields keeps it unique and stable across regenerations. ~keep
fn wildcard_lambda_param(assertion: &Assertion) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    assertion.assertion_type.hash(&mut hasher);
    assertion.field.hash(&mut hasher);
    assertion
        .value
        .as_ref()
        .map(std::string::ToString::to_string)
        .unwrap_or_default()
        .hash(&mut hasher);
    format!("e{:x}", hasher.finish() & 0xffff_ffff)
}

/// Emit `Assert.True(<array>?.Any(e => …) == true)` for a bracket-wildcard path.
///
/// The null-conditional `?.` plus `== true` covers both nullable and non-nullable list
/// getters without needing the element type name to build an empty-list fallback. ~keep
fn render_wildcard_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    field: &str,
    array_part: &str,
    elem_part: &str,
) {
    // `wildcard_split` consumes the first `[].` only, so a doubly-nested path leaves a second
    // wildcard in `elem_part` that the element accessor below would lower to index 0. ~keep
    if let Some(line) = nested_wildcard_skip_line("        ", "//", field, elem_part) {
        let _ = writeln!(out, "{line}");
        return;
    }
    let array_accessor = if array_part.is_empty() {
        result_var.to_string()
    } else {
        field_resolver.accessor(array_part, "csharp", result_var)
    };
    let param = wildcard_lambda_param(assertion);
    // `element_accessor`, not `accessor`: the path is already element-relative, so the
    // result-anchoring `accessor` applies would re-prefix it with the container. ~keep
    let elem_accessor = field_resolver.element_accessor(elem_part, "csharp", &param);

    let any_expr = |value: &serde_json::Value| -> Option<(String, String)> {
        let serde_json::Value::String(s) = value else {
            return None;
        };
        let escaped = escape_csharp(s);
        Some((
            format!(
                "{array_accessor}?.Any({param} => Convert.ToString({elem_accessor})!.Contains(\"{escaped}\")) == true"
            ),
            escaped,
        ))
    };

    match assertion.assertion_type.as_str() {
        "contains" | "not_contains" if assertion.value.is_some() => {
            let value = assertion.value.as_ref().expect("guarded by the match arm");
            let Some((expr, escaped)) = any_expr(value) else {
                let _ = writeln!(
                    out,
                    "        // skipped: non-string value for '{field}' traversal assertion"
                );
                return;
            };
            let verb = if assertion.assertion_type == "contains" {
                "True"
            } else {
                "False"
            };
            let _ = writeln!(
                out,
                "        Assert.{verb}({expr}, \"element of '{field}': {escaped}\");"
            );
        }
        "contains" | "contains_all" | "not_contains" => {
            let Some(values) = &assertion.values else {
                let _ = writeln!(out, "        // skipped: '{field}' traversal assertion has no values");
                return;
            };
            let verb = if assertion.assertion_type == "not_contains" {
                "False"
            } else {
                "True"
            };
            for value in values {
                let Some((expr, escaped)) = any_expr(value) else {
                    continue;
                };
                let _ = writeln!(
                    out,
                    "        Assert.{verb}({expr}, \"element of '{field}': {escaped}\");"
                );
            }
        }
        "not_empty" => {
            let _ = writeln!(
                out,
                "        Assert.True({array_accessor}?.Any({param} => \
                 !string.IsNullOrEmpty(Convert.ToString({elem_accessor}))) == true, \
                 \"expected some element of '{field}' to be non-empty\");"
            );
        }
        other => {
            let _ = writeln!(
                out,
                "        // skipped: unsupported traversal assertion '{other}' on '{field}'"
            );
        }
    }
}

#[cfg(test)]
#[path = "assertions/wildcard_traversal_tests.rs"]
mod wildcard_traversal_tests;
