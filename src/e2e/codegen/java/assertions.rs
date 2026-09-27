//! Java assertion rendering helpers.
//!
//! ~keep This file is already over the repo's 1,000-line file-modularization cap. The
//! `not_error_may_assert_presence` unification (routing `not_error` through
//! `not_error_presence::may_assert_presence`) added one parameter to `render_assertion`,
//! required at every call site, plus removed the old ad hoc `result_is_option && bare_field`
//! special case for `not_error` (now folded into the general arm) — a net small growth of
//! wiring and doc comments, not new unrelated functionality.

use crate::e2e::codegen::assertion_recipes::chunks_result_var;
use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::escape::escape_java;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use heck::ToLowerCamelCase;

use super::assertion_wildcard::render_wildcard_assertion;
use super::enum_lowering::{EnumLowering, classify_enum_lowering};
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
    returns_void: bool,
    fractional_fields: &std::collections::HashSet<String>,
    not_error_may_assert_presence: bool,
) {
    // `mock.*` request-count virtual fields (alef issue #443): resolve against the mock
    // server's own request log via the once-per-suite helper, never a struct field --
    // intercept before EVERY other branch below, including the `result_is_option`/
    // `result_is_bytes` special cases immediately following, which would otherwise
    // misclassify a `mock.*` field as a bare-result assertion.
    if super::assertion_mock_capture::try_render_mock_capture_assertion(out, assertion) {
        return;
    }

    // Bare-result is_empty / not_empty on Option<T> returns: the Java facade exposes
    // these as `@Nullable T` (via `.orElse(null)`) rather than `Optional<T>`, so the
    // template's `.isEmpty()` call would not compile for record types. Emit a
    // null-check instead — mirrors the kotlin / zig codegen behaviour.
    //
    // `not_error` is deliberately absent from this match: WHETHER it may assert presence is
    // decided once, centrally, by the caller via `not_error_presence::may_assert_presence`
    // (which already accounts for `result_is_option`) and handled in the general `not_error`
    // arm below, alongside every other backend's identical decision point. ~keep
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

    // `not_error` never carries a `field` and has no `java/assertion.jinja` branch —
    // that template's if/elif chain has no `else`, so before this the call silently
    // rendered nothing. An uncaught exception already fails the `@Test` method, but a
    // fixture whose only assertion is `not_error` must still leave a real, visible
    // assertion instead of a vacuous body. Mirrors the `assertNotNull` idiom the
    // byte[] branch above already uses. For streaming fixtures, assert on the
    // drained `chunks` list (bound by `collect_snippet` before this runs) rather
    // than the raw `result_var`, so a lazily-consumed stream that errors only on
    // iteration is still caught. `returns_void` calls bind no `result_var` at all
    // (`java/test_method.jinja`'s `{% if returns_void %}` branch calls without
    // assigning), so asserting on a variable here would not compile — that case is
    // handled at the call-emission site instead: `test_method.rs`'s `void_not_error`
    // flag wraps `call_expr` itself in `assertDoesNotThrow(() -> ...)`, so this arm
    // stays a no-op purely because the real assertion lives one level up, not because
    // nothing is asserted. WHETHER the plain (non-void, non-streaming) case below may
    // assert presence at all is decided once, centrally, by
    // `not_error_presence::may_assert_presence` — this arm only decides how. ~keep
    if assertion.assertion_type == "not_error" {
        if returns_void {
            // Handled by `test_method.rs`'s `void_not_error` wrapping the call in
            // assertDoesNotThrow — nothing to render into assertions_body here.
        } else if is_streaming {
            out.push_str("        assertNotNull(chunks, \"expected drained chunks list\");\n");
        } else if not_error_may_assert_presence {
            out.push_str(&format!(
                "        assertNotNull({result_var}, \"expected non-null response\");\n"
            ));
        }
        return;
    }

    // Handle synthetic/virtual fields that are computed rather than direct record accessors.
    if let Some(f) = &assertion.field {
        if let Some(reason) = crate::e2e::codegen::assertion_recipes::chunks_synthetic_skip_reason(f, field_resolver) {
            out.push_str(&format!("        // skipped: {reason}\n"));
            return;
        }

        match f.as_str() {
            // ---- ProcessingResult chunk-level computed predicates ----
            "chunks_have_content" => {
                let result_var = &chunks_result_var(field_resolver, "java", result_var);
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
                let result_var = &chunks_result_var(field_resolver, "java", result_var);
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
                let result_var = &chunks_result_var(field_resolver, "java", result_var);
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
                let result_var = &chunks_result_var(field_resolver, "java", result_var);
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
    if let Some(f) = &assertion.field
        && is_streaming
        && !f.is_empty()
        && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f)
    {
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
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "count_equals" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        format!("        assertEquals({n}, {expr}.size());\n")
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "equals" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let literal = super::values::java_string_literal(s);
                        format!("        assertEquals({literal}, {expr});\n")
                    } else if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        format!("        assertEquals({n}, {expr});\n")
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
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
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "greater_than_or_equal" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        format!("        assertTrue({expr} >= {n}, \"expected >= {n}\");\n")
                    } else {
                        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                    }
                }
                "contains" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let escaped = crate::e2e::escape::escape_java(s);
                        format!(
                            "        assertTrue({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\");\n"
                        )
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
            // ~keep The accessor returns `None` for reachable inputs (a `stream.has_*_event`
            // predicate whose item type this call never resolved, for one), and this branch used
            // to be absent: the assertion vanished with no line for
            // `fail_on_unavailable_field_markers` to see, so a clean strict-gate run was
            // indistinguishable from one that dropped it. alef's streaming adapter owns the gap,
            // so it is counted, never fatal.
            out.push_str(&format!(
                "        // skipped: {}\n",
                crate::e2e::codegen::field_skip::FieldSkip::StreamingAssertionOnUnsupportedField.message(f)
            ));
        }
        return;
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        out.push_str(&crate::e2e::template_env::render(
            "java/synthetic_assertion.jinja",
            minijinja::context! {
                assertion_kind => "skipped",
                field_name => f,
            },
        ));
        return;
    }

    let EnumLowering {
        sealed_display_type,
        is_sealed_display_field,
        field_is_enum,
    } = classify_enum_lowering(
        assertion.field.as_deref(),
        field_resolver,
        enum_fields,
        assert_enum_types,
    );

    // Runs BEFORE the wildcard lowering below, not after: `render_wildcard_assertion` returns
    // without consulting anything downstream, so a gate placed after it decides nothing for a
    // wildcard path. See `payload_union_gate`'s module doc for the family-by-family reasoning. ~keep
    if !result_is_simple
        && !is_sealed_display_field
        && super::payload_union_gate::try_skip_unsupported_family(out, assertion, field_resolver)
    {
        return;
    }

    // Bracket-wildcard traversal (`links[].linkType`) means "every element". This must run
    // before `field_expr` is built below, since that path lowers the wildcard to index 0 and
    // would assert on a single element while reading as whole-array coverage. ~keep
    if let Some(f) = assertion.field.as_deref()
        && !f.is_empty()
        && let Some((array_part, elem_part)) = field_resolver.wildcard_split(f)
    {
        render_wildcard_assertion(out, assertion, result_var, field_resolver, f, &array_part, &elem_part);
        return;
    }

    // Determine if this field is an array (List<T>) — needed to choose .toString() for
    // contains assertions, since List.contains(Object) uses equals() which won't match
    // strings against complex record types like StructureItem.
    let (field_is_array, field_is_object) = super::field_shape::classify(assertion.field.as_deref(), field_resolver);

    // A fixture's `equals` assertion carrying a literal JSON `null` against a field the IR
    // proves is a genuine (non-`Option`) collection -- e.g. `Vec<T>` with `#[serde(default,
    // skip_serializing_if = "Vec::is_empty")]` -- can never pass: the generated binding's
    // Jackson deserializer and its `Builder`'s own default both materialize an absent/omitted
    // collection as an empty `List`/`Map`, never `null` -- see
    // `backends::java::gen_bindings::types::builders::gen_builder_nested_class`'s
    // `field_is_optional_in_binding` branch, which only ever defaults to `null` when the IR's
    // `field.optional` is true. Both `is_collection_root` and `is_optional` here read that
    // same IR fact (via `with_ir_collection_map`/`with_ir_fields`), so this is the assertion
    // side asking the binding side's own question, not a second independent guess at the
    // answer. Recognize the case and assert the same emptiness the binding actually produces
    // instead of a null-equality check that can never pass. ~keep
    let is_doomed_null_equals_on_required_collection = assertion.assertion_type == "equals"
        && matches!(assertion.value, Some(serde_json::Value::Null))
        && assertion.field.as_deref().is_some_and(|f| {
            !f.is_empty()
                && field_resolver.is_valid_for_result(f)
                && field_resolver.is_collection_root(field_resolver.resolve(f))
                && !field_resolver.is_optional(field_resolver.resolve(f))
        });
    if is_doomed_null_equals_on_required_collection {
        let accessor = field_resolver.accessor(assertion.field.as_deref().unwrap_or_default(), "java", result_var);
        out.push_str(&format!(
            "        assertTrue({accessor}.isEmpty(), \"expected empty (binding never returns null for a non-optional collection)\");\n"
        ));
        return;
    }

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
                // Fields in `fields_display_as_text` have an `Option<T>` inner type
                // that is not a plain `String` (e.g. `AssistantContent`). Their Java
                // binding exposes a `.text()` accessor returning `String`. Using
                // `Objects::toString` on these would produce the class-name representation,
                // not the textual content.
                let field_is_display_as_text = field_resolver.is_display_as_text(f);
                if field_resolver.is_optional(resolved) && !field_resolver.has_map_access(f) {
                    // All nullable fields in the Java binding return @Nullable types, not Optional<T>.
                    // Wrap them in Optional.ofNullable() so e2e tests can use .orElse() fallbacks.
                    let optional_expr = format!("java.util.Optional.ofNullable({accessor})");
                    // Enum-typed optional fields need .map(v -> v.getValue()) to coerce to String
                    // before the orElse("") fallback can type-check (Optional<Enum>.orElse("") would
                    // be a type mismatch — Optional<String>.orElse("") is the only safe form).
                    if field_is_enum {
                        match assertion.assertion_type.as_str() {
                            // `is_true`/`is_false` on an Optional field mean "present"/"absent" --
                            // matching not_empty/is_empty, the raw Optional is returned so the
                            // template's presence switch (not a `.map(...).orElse(...)` string
                            // coercion, which produced a non-boolean `assertTrue` argument) decides.
                            "not_empty" | "is_empty" | "is_true" | "is_false" => optional_expr,
                            _ => {
                                // `field_is_enum` already excludes sealed-interface types
                                // (is_sealed_display_field), so any remaining enum type
                                // has .getValue() available.
                                format!("{optional_expr}.map(v -> v.getValue()).orElse(\"\")")
                            }
                        }
                    } else if field_is_display_as_text {
                        // Non-String content union (e.g. AssistantContent): call `.text()`
                        // to get the textual representation instead of `Objects::toString`
                        // which would return the class name.
                        match assertion.assertion_type.as_str() {
                            // `is_true`/`is_false` on an Optional field mean "present"/"absent" --
                            // matching not_empty/is_empty, the raw Optional is returned so the
                            // template's presence switch (not a `.map(...).orElse(...)` string
                            // coercion, which produced a non-boolean `assertTrue` argument) decides.
                            "not_empty" | "is_empty" | "is_true" | "is_false" => optional_expr,
                            _ => format!("{optional_expr}.map(v -> v.text()).orElse(\"\")"),
                        }
                    } else {
                        match assertion.assertion_type.as_str() {
                            // `not_empty`/`is_empty`/`is_true`/`is_false` on an Optional field all
                            // return the raw Optional so the template's presence switch decides --
                            // for is_true/is_false this replaces a `.map(...).orElse(...)` string
                            // coercion that produced a non-boolean `assertTrue` argument.
                            "not_empty" | "is_empty" | "is_true" | "is_false" => optional_expr,
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
                            //
                            // Fractional fields (`f32`/`f64`, e.g. `Optional<Double>
                            // qualityScore`) must NOT go through `Number::longValue()` — that
                            // truncates the boxed value to zero before the comparison ever
                            // runs, turning a `[0.0, 1.0]` range assertion into a tautology
                            // (every legal value truncates to `0L`, so both bounds always
                            // hold). Route these through `Number::doubleValue()` instead so
                            // the comparison actually observes the fractional value. ~keep
                            "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" => {
                                if field_resolver.is_array(resolved) {
                                    format!("{optional_expr}.orElse(java.util.List.of())")
                                } else if is_fractional_field(fractional_fields, resolved) {
                                    format!("{optional_expr}.map(Number::doubleValue).orElse(0.0)")
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
            field_is_object,
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

/// Leaf segment of a (possibly dotted / bracketed) resolved field path, e.g.
/// `"results[0].quality_score"` -> `"quality_score"`.
fn leaf_field_name(path: &str) -> &str {
    let last_dot = path.rsplit('.').next().unwrap_or(path);
    last_dot.split('[').next().unwrap_or(last_dot)
}

/// True when `resolved`'s leaf field name is known (via [`fractional_scalar_fields`]) to
/// carry an `f32`/`f64` Rust type — directly or through `Option<T>`.
fn is_fractional_field(fractional_fields: &std::collections::HashSet<String>, resolved: &str) -> bool {
    fractional_fields.contains(leaf_field_name(resolved))
}

/// Field names (bare leaf, e.g. `"quality_score"`) whose Rust type — or `Option<T>` inner
/// type — is `f32`/`f64` on at least one IR type in `type_defs`.
///
/// Consulted before defaulting an `Optional` numeric-range coercion to
/// `Number::longValue()`: that truncates a fractional value to zero before the comparison
/// runs, turning e.g. a `[0.0, 1.0]` range assertion on a `Double` `qualityScore` into a
/// tautology (every legal value truncates to `0L`, so both bounds always hold). ~keep
pub(super) fn fractional_scalar_fields(type_defs: &[crate::core::ir::TypeDef]) -> std::collections::HashSet<String> {
    use crate::core::ir::{PrimitiveType, TypeRef};
    let mut fractional = std::collections::HashSet::new();
    for type_def in type_defs {
        for field in &type_def.fields {
            let ty = match &field.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            if matches!(
                ty,
                TypeRef::Primitive(PrimitiveType::F32) | TypeRef::Primitive(PrimitiveType::F64)
            ) {
                fractional.insert(field.name.clone());
            }
        }
    }
    fractional
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

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;

    fn make_resolver(optional: HashSet<String>, dat: HashSet<String>) -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &optional,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_display_as_text_fields(dat)
    }

    fn make_equals_assertion(field: &str, value: &str) -> Assertion {
        Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Default::default()
        }
    }

    fn make_contains_assertion(field: &str, value: &str) -> Assertion {
        Assertion {
            assertion_type: "contains".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Default::default()
        }
    }

    fn render_bare(assertion: &Assertion) -> String {
        let resolver = make_resolver(HashSet::new(), HashSet::new());
        let mut out = String::new();
        render_assertion(
            &mut out,
            assertion,
            "result",
            "Result",
            &resolver,
            false,
            false,
            false,
            false,
            None,
            &HashSet::new(),
            &HashMap::new(),
            false,
            &HashSet::new(),
            true,
        );
        out
    }

    fn render_with_optional(assertion: &Assertion, optional_field: &str) -> String {
        let optional: HashSet<String> = [optional_field.to_string()].into_iter().collect();
        let resolver = make_resolver(optional, HashSet::new());
        let mut out = String::new();
        render_assertion(
            &mut out,
            assertion,
            "result",
            "Result",
            &resolver,
            false,
            false,
            false,
            false,
            None,
            &HashSet::new(),
            &HashMap::new(),
            false,
            &HashSet::new(),
            true,
        );
        out
    }

    fn is_true_assertion(field: &str) -> Assertion {
        Assertion {
            assertion_type: "is_true".to_string(),
            field: Some(field.to_string()),
            ..Default::default()
        }
    }

    /// `Option<DataNode>` presence: before the fix this fell through to the generic
    /// `.map(Objects::toString).orElse("")` string-coercion arm, so `assertTrue` received
    /// a `String` argument -- a compile error, since `assertTrue` requires `boolean`.
    #[test]
    fn is_true_on_optional_struct_field_checks_presence() {
        let out = render_with_optional(&is_true_assertion("data"), "data");
        assert_eq!(
            out,
            "        assertTrue(java.util.Optional.ofNullable(result.data()).isPresent(), \"expected true (present)\");\n"
        );
    }

    #[test]
    fn is_false_on_optional_struct_field_checks_absence() {
        let out = render_with_optional(
            &Assertion {
                assertion_type: "is_false".to_string(),
                field: Some("data".to_string()),
                ..Default::default()
            },
            "data",
        );
        assert_eq!(
            out,
            "        assertTrue(java.util.Optional.ofNullable(result.data()).isEmpty(), \"expected false (absent)\");\n"
        );
    }

    /// A follow-on member access through the same optional field must still compile: the
    /// leaf (`equals` on `data.kind`) is unaffected by the `is_true` fix, so it continues to
    /// route through the existing `Optional.ofNullable(...).map(Objects::toString).orElse("")`
    /// coercion rather than needing an unwrap of its own -- Java's binding returns `@Nullable`
    /// types, not `Optional<T>`, so `result.data().kind()` already compiles regardless of
    /// nullability. ~keep
    #[test]
    fn equals_on_nested_field_through_optional_parent_is_unchanged() {
        let out = render_with_optional(&make_equals_assertion("data.kind", "KeyValue"), "data");
        assert!(out.contains("result.data().kind()"), "got: {out}");
    }

    #[test]
    fn is_true_on_non_optional_field_is_unchanged() {
        let out = render_bare(&is_true_assertion("active"));
        assert_eq!(out, "        assertTrue(result.active(), \"expected true\");\n");
    }
    #[cfg(test)]
    #[path = "wildcard_tests.rs"]
    mod wildcard_tests;

    /// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
    /// (present, non-`binding_excluded`, on some IR type) but missing from the
    /// hand-maintained `result_fields` config must still render a real assertion,
    /// not a "skipped: field not available" stub — `java/test_method.rs` now
    /// threads `FieldResolver::ir_field_sets(type_defs)` into `with_ir_fields`. ~keep
    #[test]
    fn java_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
        let reachable: HashSet<String> = ["data".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(reachable, HashSet::new(), HashSet::new());
        let assertion = make_equals_assertion("data", "hello");
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            &resolver,
            false,
            false,
            false,
            false,
            None,
            &HashSet::new(),
            &HashMap::new(),
            false,
            &HashSet::new(),
            true,
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
    fn java_ir_excluded_field_present_in_result_fields_is_still_skipped() {
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
        let assertion = make_equals_assertion("internal_diagnostics", "hello");
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            &resolver,
            false,
            false,
            false,
            false,
            None,
            &HashSet::new(),
            &HashMap::new(),
            false,
            &HashSet::new(),
            true,
        );
        assert!(out.contains("skipped"), "got: {out}");
    }

    /// A plain `Option<String>` field should use `Objects::toString` in the
    /// Java equals expression — NOT `.text()`. Guards against DAT path bleeding
    /// into regular optional string fields.
    #[test]
    fn java_plain_optional_string_uses_objects_to_string() {
        let mut optional = HashSet::new();
        optional.insert("content".to_string());
        let resolver = make_resolver(optional, HashSet::new());
        let assertion = make_equals_assertion("content", "hello");
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            &resolver,
            false,
            false,
            false,
            false,
            None,
            &HashSet::new(),
            &HashMap::new(),
            false,
            &HashSet::new(),
            true,
        );
        assert!(
            out.contains("Objects::toString"),
            "plain optional string field must use Objects::toString; got: {out}"
        );
        assert!(
            !out.contains(".text()"),
            "plain optional string must NOT use .text(); got: {out}"
        );
    }

    /// A `display_as_text` field (e.g. `Option<AssistantContent>`) should use
    /// `.map(v -> v.text()).orElse("")` so the Java assertion sees the textual
    /// representation, not the class-name string from `Objects::toString`.
    #[test]
    fn java_display_as_text_optional_uses_text_accessor() {
        let mut optional = HashSet::new();
        optional.insert("content".to_string());
        let mut dat = HashSet::new();
        dat.insert("content".to_string());
        let resolver = make_resolver(optional, dat);
        let assertion = make_equals_assertion("content", "Hello, world!");
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            &resolver,
            false,
            false,
            false,
            false,
            None,
            &HashSet::new(),
            &HashMap::new(),
            false,
            &HashSet::new(),
            true,
        );
        assert!(
            out.contains(".map(v -> v.text()).orElse(\"\")"),
            "display_as_text field must use .map(v -> v.text()).orElse(\"\"); got: {out}"
        );
        assert!(
            !out.contains("Objects::toString"),
            "display_as_text field must NOT use Objects::toString; got: {out}"
        );
    }

    fn make_not_error_assertion() -> Assertion {
        Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }
    }

    /// Regression test for the not_error vacuous-test defect: `java/assertion.jinja`'s
    /// if/elif chain has no `not_error` branch and no final `else`, so before this fix
    /// a fixture whose only assertion was `not_error` rendered nothing at all — not
    /// even a comment. Must emit a real `assertNotNull` instead.
    #[test]
    fn not_error_emits_a_real_assert_not_null_on_the_result() {
        let resolver = make_resolver(HashSet::new(), HashSet::new());
        let assertion = make_not_error_assertion();
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            &resolver,
            false,
            false,
            false,
            false,
            None,
            &HashSet::new(),
            &HashMap::new(),
            false,
            &HashSet::new(),
            true,
        );
        assert_eq!(out, "        assertNotNull(result, \"expected non-null response\");\n");
    }

    #[test]
    fn not_error_on_a_streaming_fixture_asserts_on_drained_chunks_not_result() {
        let resolver = make_resolver(HashSet::new(), HashSet::new());
        let assertion = make_not_error_assertion();
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            &resolver,
            false,
            false,
            false,
            true,
            None,
            &HashSet::new(),
            &HashMap::new(),
            false,
            &HashSet::new(),
            true,
        );
        assert_eq!(
            out,
            "        assertNotNull(chunks, \"expected drained chunks list\");\n"
        );
    }

    /// A `returns_void` call binds no `result_var` at all (see
    /// `java/test_method.jinja`'s `{% if returns_void %}` branch) — asserting on it
    /// would not compile. The real assertion for this case lives one level up: see
    /// `test_method.rs`'s `void_not_error_call_wraps_call_expr_in_assert_does_not_throw`,
    /// which wraps `call_expr` in `assertDoesNotThrow` at the call-emission site instead.
    #[test]
    fn not_error_on_a_returns_void_call_emits_nothing() {
        let resolver = make_resolver(HashSet::new(), HashSet::new());
        let assertion = make_not_error_assertion();
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            &resolver,
            false,
            false,
            false,
            false,
            None,
            &HashSet::new(),
            &HashMap::new(),
            true,
            &HashSet::new(),
            true,
        );
        assert!(
            out.is_empty(),
            "a returns_void call must not reference an unbound result_var, got: {out}"
        );
    }

    fn make_range_assertion(assertion_type: &str, field: &str, value: f64) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value: serde_json::Number::from_f64(value).map(serde_json::Value::Number),
            ..Default::default()
        }
    }

    /// Regression test for the `qualityScore` range-assertion defect: an
    /// `Optional<Double>` field's range comparators must NOT coerce through
    /// `Number::longValue()` — that truncates every legal fractional value to `0L`
    /// before the comparison runs, so a `[0.0, 1.0]` range check on a `Double` becomes
    /// a tautology that can never fail. With the field registered in
    /// `fractional_fields`, the emitted comparison must use `Number::doubleValue()`
    /// instead, so it can actually observe (and fail on) an out-of-range value. ~keep
    #[test]
    fn fractional_optional_field_range_assertion_uses_double_value_not_long_value() {
        let mut optional = HashSet::new();
        optional.insert("quality_score".to_string());
        let resolver = make_resolver(optional, HashSet::new());
        let fractional: HashSet<String> = ["quality_score".to_string()].into_iter().collect();
        let assertion = make_range_assertion("greater_than_or_equal", "quality_score", 0.0);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            &resolver,
            false,
            false,
            false,
            false,
            None,
            &HashSet::new(),
            &HashMap::new(),
            false,
            &fractional,
            true,
        );
        assert!(
            out.contains("Number::doubleValue"),
            "fractional Optional field must coerce via Number::doubleValue, got: {out}"
        );
        assert!(
            !out.contains("Number::longValue"),
            "fractional Optional field must NOT truncate via Number::longValue, got: {out}"
        );
    }

    /// Negative control: an integer `Optional` field (e.g. `sheetCount`, correctly
    /// handled at `SmokeTest.java:149`) is absent from `fractional_fields` and must
    /// keep using `Number::longValue()` — the fractional-type fix must not regress
    /// the already-correct integer path.
    #[test]
    fn integer_optional_field_range_assertion_still_uses_long_value() {
        let mut optional = HashSet::new();
        optional.insert("sheet_count".to_string());
        let resolver = make_resolver(optional, HashSet::new());
        let assertion = make_range_assertion("greater_than_or_equal", "sheet_count", 1.0);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClass",
            &resolver,
            false,
            false,
            false,
            false,
            None,
            &HashSet::new(),
            &HashMap::new(),
            false,
            &HashSet::new(),
            true,
        );
        assert!(
            out.contains("Number::longValue"),
            "integer Optional field must keep Number::longValue, got: {out}"
        );
        assert!(
            !out.contains("Number::doubleValue"),
            "integer Optional field must not use Number::doubleValue, got: {out}"
        );
    }

    /// `fractional_scalar_fields` must recognize `f64`/`f32` fields, including
    /// through `Option<T>`, and must NOT flag integer fields.
    #[test]
    fn fractional_scalar_fields_detects_float_types_through_optional() {
        use crate::core::ir::{FieldDef, PrimitiveType, TypeDef, TypeRef};

        let type_defs = vec![TypeDef {
            name: "SampleResult".to_string(),
            fields: vec![
                FieldDef {
                    name: "quality_score".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::F64))),
                    ..Default::default()
                },
                FieldDef {
                    name: "ratio".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::F32),
                    ..Default::default()
                },
                FieldDef {
                    name: "sheet_count".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }];

        let fractional = fractional_scalar_fields(&type_defs);
        assert!(fractional.contains("quality_score"), "got: {fractional:?}");
        assert!(fractional.contains("ratio"), "got: {fractional:?}");
        assert!(
            !fractional.contains("sheet_count"),
            "integer field must not be classified as fractional, got: {fractional:?}"
        );
    }
}
