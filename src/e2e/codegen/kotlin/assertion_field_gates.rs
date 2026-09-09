//! Kotlin `render_assertion` field-shape gates.
//!
//! Each function here is a self-contained, independent early-exit special case
//! `render_assertion` consults, in order, before falling through to its generic
//! scalar-assertion pipeline: a streaming `usage`/`usage.*` field, a streaming virtual field, a
//! field absent from the result type, the two discriminated-union navigation paths, and a
//! bracket-wildcard traversal. Split out of `assertions.rs` at the concept boundary -- every
//! function returns `true` when it fully rendered the assertion (the caller returns immediately)
//! and `false` when its own guard condition did not match at all, in which case the caller must
//! keep trying its other gates exactly as it did when this was one long function. Verbatim
//! extraction: each function body is byte-identical to the original `if` block it replaces,
//! except the bare `return;` inside becomes `return true;` so the boolean the caller now checks
//! carries the same "handled" meaning the early `return` used to. ~keep

use heck::ToLowerCamelCase;
use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::{FieldSkip, nested_wildcard_skip_line};
use crate::e2e::codegen::payload_union_skip::{UnionLoweringTarget, payload_union_skip_line};
use crate::e2e::escape::escape_kotlin;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

/// Try every field-shape gate `render_assertion` consults, in order, before falling through to
/// its generic scalar-assertion pipeline. Returns `true` (and has already written the full
/// assertion) the moment one gate handles the fixture; `false` once every gate has declined,
/// meaning the caller must continue into its own scalar pipeline. `accessor_lang` is computed
/// here as a cheap, side-effect-free duplicate of the one `render_assertion` also computes for
/// its own later, non-gate use -- the two never need to agree on anything but the same
/// `kotlin_android_style` input, so recomputing it is simpler and safer than threading it
/// through as a shared parameter. ~keep
#[allow(clippy::too_many_arguments)]
pub(super) fn try_render_field_shape_gates(
    out: &mut String,
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    result_var: &str,
    result_is_simple: bool,
    is_streaming: bool,
    kotlin_android_style: bool,
    fields_c_types: &std::collections::HashMap<String, String>,
) -> bool {
    if try_render_streaming_usage_field_assertion(out, assertion, is_streaming, kotlin_android_style, fields_c_types) {
        return true;
    }
    if try_render_streaming_virtual_field_assertion(out, assertion, is_streaming, kotlin_android_style) {
        return true;
    }
    if try_skip_field_not_available_on_result_type(out, assertion, field_resolver) {
        return true;
    }
    if try_render_discriminated_union_android_assertion(
        out,
        assertion,
        field_resolver,
        result_var,
        kotlin_android_style,
    ) {
        return true;
    }
    if try_render_generic_union_fallback(out, assertion, field_resolver, result_var, kotlin_android_style) {
        return true;
    }
    let accessor_lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    if try_render_wildcard_traversal_assertion(
        out,
        assertion,
        result_var,
        field_resolver,
        result_is_simple,
        accessor_lang,
    ) {
        return true;
    }
    if try_render_display_text_presence(
        out,
        assertion,
        field_resolver,
        result_var,
        result_is_simple,
        kotlin_android_style,
    ) {
        return true;
    }
    try_skip_payload_union_scalar_lowering(out, assertion, field_resolver, kotlin_android_style)
}

fn try_render_display_text_presence(
    out: &mut String,
    assertion: &Assertion,
    resolver: &FieldResolver,
    result_var: &str,
    result_is_simple: bool,
    android_style: bool,
) -> bool {
    if !assertion
        .field
        .as_deref()
        .is_some_and(|field| resolver.is_display_as_text(field))
    {
        return false;
    }
    let (template, predicate) = match assertion.assertion_type.as_str() {
        "not_empty" => ("kotlin/not_empty_assertion.kt.jinja", "isNotEmpty"),
        "is_empty" => ("kotlin/is_empty_assertion.kt.jinja", "isEmpty"),
        _ => return false,
    };
    let language = if android_style { "kotlin_android" } else { "kotlin" };
    let field = super::assertion_scalar_context::resolve_field_expr(
        assertion,
        resolver,
        result_var,
        result_is_simple,
        language,
    );
    let optional = super::assertion_scalar_context::resolve_field_is_optional(
        result_is_simple,
        &field,
        assertion,
        resolver,
        android_style,
    );
    let text = super::assertion_scalar_context::resolve_string_field_expr(&field, true, false, false, optional);
    out.push_str(&crate::e2e::template_env::render(
        template,
        minijinja::context! { predicate => format!("{text}.{predicate}()") },
    ));
    true
}

/// The last gate, deliberately after the union-traversal and wildcard gates above: those two have
/// their own, correct answers for a path that *crosses* a union or names a union-typed element,
/// and only a path whose own leaf IS the union reaches here.
///
/// ~keep Neither Kotlin target has a scalar wire accessor on that leaf — kotlin_android's
/// `toWire()` is declared on the `enum class` branch `object_wrapper::enums::emit_enum` takes only
/// when every variant is fieldless, and the JVM target's `getValue()` follows
/// `backends::java::gen_bindings::types::enums::emits_get_value`, which withholds it for a
/// `serde(tag)`/`serde(untagged)` union wrapper while still folding an externally tagged data enum
/// down to a plain Java `enum`. The two therefore disagree, and `UnionLoweringTarget` keeps them
/// apart. Refusing rather than falling through matters here: `resolve_string_expr`'s non-enum
/// branch would hand `render_equals_arm` the bare accessor, emitting
/// `assertEquals("key_value", result.kind)` — a `String` compared to a sealed-class instance, which
/// Kotlin accepts and which is false at runtime for every fixture.
fn try_skip_payload_union_scalar_lowering(
    out: &mut String,
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    kotlin_android_style: bool,
) -> bool {
    let target = if kotlin_android_style {
        UnionLoweringTarget::KotlinAndroid
    } else {
        UnionLoweringTarget::KotlinJvm
    };
    let Some(line) = payload_union_skip_line("        ", "//", field_resolver, assertion.field.as_deref(), target)
    else {
        return false;
    };
    let _ = writeln!(out, "{line}");
    true
}

/// In streaming context, `usage` and `usage.*` fields must be read from the
/// last collected chunk, not from the stream iterator (which has no `usage()` method).
/// Route them through `StreamingFieldResolver::accessor("usage", ...)` + deep-tail
/// rendering, using `chunks.last().usage()` as the base expression.
pub(super) fn try_render_streaming_usage_field_assertion(
    out: &mut String,
    assertion: &Assertion,
    is_streaming: bool,
    kotlin_android_style: bool,
    fields_c_types: &std::collections::HashMap<String, String>,
) -> bool {
    if is_streaming
        && let Some(f) = &assertion.field
        && (f == "usage" || f.starts_with("usage."))
    {
        let expr = resolve_streaming_usage_expr(f, kotlin_android_style);

        // Determine if the field maps to a 64-bit C type requiring `L` suffix.
        let field_is_long = fields_c_types
            .get(f.as_str())
            .is_some_and(|t| matches!(t.as_str(), "uint64_t" | "int64_t"));

        let line = render_streaming_usage_line(assertion, f, &expr, field_is_long);
        out.push_str(&line);
        return true;
    }
    false
}

/// The `usage`/`usage.*` streaming gate's base accessor plus deep-tail rendering, split out of
/// `try_render_streaming_usage_field_assertion` to keep it under the file's function-length cap
/// -- verbatim extraction, no behavior change. ~keep
fn resolve_streaming_usage_expr(f: &str, kotlin_android_style: bool) -> String {
    let stream_lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    let base_expr =
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor("usage", stream_lang, "chunks")
            .unwrap_or_else(|| {
                if kotlin_android_style {
                    "(if (chunks.isEmpty()) null else chunks.last().usage)".to_string()
                } else {
                    "(if (chunks.isEmpty()) null else chunks.last().usage())".to_string()
                }
            });

    // For a deep path like `usage.total_tokens`, render the tail `.total_tokens`
    // in a language-appropriate accessor style.
    if let Some(tail) = f.strip_prefix("usage.") {
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
    }
}

/// The `usage`/`usage.*` streaming gate's own per-assertion-type dispatch, split out of
/// `try_render_streaming_usage_field_assertion` to keep it under the file's function-length
/// cap -- verbatim extraction, no behavior change. ~keep
fn render_streaming_usage_line(assertion: &Assertion, f: &str, expr: &str, field_is_long: bool) -> String {
    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = if field_is_long && expected.is_number() && !expected.is_f64() {
                    format!("{}L", expected)
                } else {
                    super::values::json_to_kotlin(expected)
                };
                format!("        assertEquals({kotlin_val}, {expr}!!)\n")
            } else {
                streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
            }
        }
        // ~keep This arm covered every assertion type but `equals` and rendered an empty
        // string, so a `not_empty`/`greater_than`/... against a streaming `usage.*` path
        // disappeared with no line for any funnel to count. The renderer really does only
        // implement `equals` here, which is alef's gap to close, not the fixture's.
        _ => streaming_assertion_type_skip_line("        ", "//", f, &assertion.assertion_type) + "\n",
    }
}

/// Streaming virtual fields resolve against the `chunks` collected-list variable.
/// Intercept before is_valid_for_result so they are never skipped.
/// Gate on `is_streaming` so non-streaming fixtures (e.g. consumers whose real
/// result struct has a literal `chunks` field) don't divert into the virtual
/// accessor path — they should fall through to the normal field resolver.
pub(super) fn try_render_streaming_virtual_field_assertion(
    out: &mut String,
    assertion: &Assertion,
    is_streaming: bool,
    kotlin_android_style: bool,
) -> bool {
    if let Some(f) = &assertion.field
        && is_streaming
        && !f.is_empty()
        && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f)
    {
        let stream_lang = if kotlin_android_style {
            "kotlin_android"
        } else {
            "kotlin"
        };
        if let Some(expr) =
            crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, stream_lang, "chunks")
        {
            let line = render_streaming_virtual_field_line(assertion, f, &expr);
            out.push_str(&line);
        } else {
            // ~keep The accessor returns `None` for reachable inputs (a `stream.has_*_event`
            // predicate never resolves through `accessor`, which supplies no item type), and this
            // branch used to be absent: the assertion vanished with no line for
            // `fail_on_unavailable_field_markers` to see. alef's streaming adapter owns the gap,
            // so it is counted, never fatal.
            let _ = writeln!(
                out,
                "        // skipped: {}",
                FieldSkip::StreamingAssertionOnUnsupportedField.message(f)
            );
        }
        return true;
    }
    false
}

/// The streaming virtual-field gate's own per-assertion-type dispatch, split out of
/// `try_render_streaming_virtual_field_assertion` to keep it under the file's function-length
/// cap -- verbatim extraction, no behavior change. ~keep
fn render_streaming_virtual_field_line(assertion: &Assertion, f: &str, expr: &str) -> String {
    match assertion.assertion_type.as_str() {
        "count_min" | "count_equals" => render_streaming_virtual_field_count_line(assertion, f, expr),
        "equals" => {
            if let Some(serde_json::Value::String(s)) = &assertion.value {
                let literal = super::values::kotlin_string_literal(s);
                format!("        assertEquals({literal}, {expr})\n")
            } else if let Some(b) = assertion.value.as_ref().and_then(|v| v.as_bool()) {
                format!("        assertEquals({b}, {expr})\n")
            } else {
                streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
            }
        }
        "not_empty" => {
            format!("        assertFalse({expr}.isEmpty(), \"expected non-empty\")\n")
        }
        "is_empty" => {
            format!("        assertTrue({expr}.isEmpty(), \"expected empty\")\n")
        }
        "is_true" => {
            format!("        assertTrue({expr} == true, \"expected true\")\n")
        }
        "is_false" => {
            format!("        assertTrue({expr} == false, \"expected false\")\n")
        }
        "greater_than" => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                format!("        assertTrue({expr} > {n}, \"expected > {n}\")\n")
            } else {
                streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
            }
        }
        "contains" => render_streaming_virtual_field_contains_line(assertion, f, expr),
        _ => format!(
            "{}\n",
            streaming_assertion_type_skip_line("        ", "//", f, &assertion.assertion_type)
        ),
    }
}

/// The streaming virtual-field gate's `count_min`/`count_equals` arms, split out of
/// `render_streaming_virtual_field_line` to keep it under the file's function-length cap --
/// verbatim extraction, no behavior change. ~keep
fn render_streaming_virtual_field_count_line(assertion: &Assertion, f: &str, expr: &str) -> String {
    match assertion.assertion_type.as_str() {
        "count_min" => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                format!("        assertTrue({expr}.size >= {n}, \"expected >= {n} chunks\")\n")
            } else {
                streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
            }
        }
        "count_equals" => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                format!("        assertEquals({n}.toLong(), {expr}.size.toLong(), \"expected exactly {n} elements\")\n")
            } else {
                streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
            }
        }
        _ => unreachable!("only called for count_min/count_equals"),
    }
}

/// The streaming virtual-field gate's `contains` arm, split out of
/// `render_streaming_virtual_field_line` to keep it under the file's function-length cap --
/// verbatim extraction, no behavior change. ~keep
fn render_streaming_virtual_field_contains_line(assertion: &Assertion, f: &str, expr: &str) -> String {
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
        streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
    }
}

/// Skip assertions on fields that don't exist on the result type.
pub(super) fn try_skip_field_not_available_on_result_type(
    out: &mut String,
    assertion: &Assertion,
    field_resolver: &FieldResolver,
) -> bool {
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        let _ = writeln!(
            out,
            "        // skipped: {}",
            FieldSkip::NotAvailableOnResultType.message(f)
        );
        return true;
    }
    false
}

/// Discriminated-union navigation (sealed `FormatMetadata` in Kotlin).
/// Field paths like `metadata.format.excel.sheet_count` cannot be expressed as
/// a flat property chain because `FormatMetadata` is a sealed class with
/// variant subclasses (`FormatMetadata.Excel`, `FormatMetadata.Pdf`, …); each
/// variant exposes its payload through a `.metadata` property of the variant
/// type.  Emit an `is`-pattern `when` block that binds the variant, then
/// delegate the leaf assertion to `render_discriminated_union_assertion`.
pub(super) fn try_render_discriminated_union_android_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    result_var: &str,
    kotlin_android_style: bool,
) -> bool {
    if kotlin_android_style
        && let Some(f) = assertion.field.as_deref().filter(|f| !f.is_empty())
        && let Some((variant_pascal, inner_field)) = super::discriminated::parse_discriminated_union_access(f)
    {
        let variant_var = format!("format{variant_pascal}");
        let (container, field_is_collection) =
            resolve_discriminated_union_android_binding(field_resolver, f, result_var, &variant_pascal, &inner_field);
        let _ = writeln!(out, "        when (val {variant_var} = {container}) {{");
        let _ = writeln!(out, "            is FormatMetadata.{variant_pascal} -> {{");
        super::discriminated::render_discriminated_union_assertion(
            out,
            assertion,
            &variant_var,
            "metadata",
            &inner_field,
            field_is_collection,
        );
        let _ = writeln!(out, "            }}");
        let _ = writeln!(
            out,
            "            else -> kotlin.test.assertTrue(false, \"Expected {variant_pascal} variant\")"
        );
        let _ = writeln!(out, "        }}");
        return true;
    }
    false
}

/// Resolve the discriminated-union container (`…metadata.format`) through the field resolver so
/// list-result field paths (`results[0].metadata.format.…`) index into `.results.first()` like
/// the flat-field assertions do, instead of hardcoding `{result_var}.metadata.format` (metadata
/// lives on each result, not the top-level ExtractionResult, so batch results would not
/// compile), alongside whether the matched variant's payload is itself a collection. Split out
/// of `try_render_discriminated_union_android_assertion` to keep it under the file's
/// function-length cap -- verbatim extraction, no behavior change.
///
/// An empty `inner_field` means the fixture path named only the variant (e.g.
/// `metadata.format.pdf`) — no field inside the payload is being checked, so
/// `union_variant_field_is_collection` (which requires a non-empty field name) always answers
/// `false`. Whether the `FormatMetadata` variant's payload itself is a collection is the
/// distinct question `union_variant_payload_is_collection` answers instead. ~keep
fn resolve_discriminated_union_android_binding(
    field_resolver: &FieldResolver,
    f: &str,
    result_var: &str,
    variant_pascal: &str,
    inner_field: &str,
) -> (String, bool) {
    let format_path = match f.find(".format") {
        Some(idx) => &f[..idx + ".format".len()],
        None => f,
    };
    let container = field_resolver.accessor(format_path, "kotlin_android", result_var);
    let field_is_collection = if inner_field.is_empty() {
        field_resolver.union_variant_payload_is_collection("FormatMetadata", variant_pascal)
    } else {
        field_resolver.union_variant_field_is_collection(format_path, variant_pascal, inner_field)
    };
    (container, field_is_collection)
}

/// IR-general fallback for any OTHER tagged-union traversal `parse_discriminated_union_access`
/// does not recognize (a different union entirely, or the same union reached from a config
/// that never declared it under `metadata.format`), plus the loud named skip for a boundary
/// it detects but cannot lower. See `discriminated::try_render_generic_union_assertion`. ~keep
pub(super) fn try_render_generic_union_fallback(
    out: &mut String,
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    result_var: &str,
    kotlin_android_style: bool,
) -> bool {
    if let Some(f) = assertion.field.as_deref().filter(|f| !f.is_empty())
        && super::discriminated::try_render_generic_union_assertion(
            out,
            assertion,
            field_resolver,
            result_var,
            kotlin_android_style,
            f,
        )
    {
        return true;
    }
    false
}

/// Bracket-wildcard traversal (`links[].link_type`) means "any element", so it must
/// render an `any { … }` quantifier. Falling through to `accessor` would lower the
/// wildcard to index 0 and silently assert against only the first element. Keyed off
/// the fixture path alone — config sets (`fields_json_scalar` etc.) also use the `[]`
/// spelling for fields whose fixture paths carry explicit indices. ~keep
pub(super) fn try_render_wildcard_traversal_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    accessor_lang: &str,
) -> bool {
    if !result_is_simple
        && let Some(f) = assertion.field.as_deref().filter(|f| !f.is_empty())
        && let Some((array_part, elem_part)) = field_resolver.wildcard_split(f)
    {
        // `wildcard_split` consumes the first `[].` only, so a doubly-nested path leaves a
        // second wildcard in `elem_part`. Kotlin's renderer lowers it to `.first()` rather than
        // a visible `[0]`, which makes the collapse even harder to spot in review. ~keep
        if let Some(line) = nested_wildcard_skip_line("        ", "//", f, &elem_part) {
            let _ = writeln!(out, "{line}");
            return true;
        }
        let array_accessor = resolve_wildcard_array_accessor(field_resolver, result_var, &array_part, accessor_lang);
        // `element_accessor`, not `accessor`: the path is already element-relative, so the
        // result-anchoring `accessor` applies would re-prefix it with the container. ~keep
        let elem_accessor = field_resolver.element_accessor(&elem_part, accessor_lang, "e");
        render_wildcard_traversal_match(out, assertion, f, &array_accessor, &elem_accessor);
        return true;
    }
    false
}

/// The wildcard-traversal gate's array-receiver accessor, split out of
/// `try_render_wildcard_traversal_assertion` to keep it under the file's function-length cap --
/// verbatim extraction, no behavior change.
///
/// A nullable array receiver cannot take `.any {}` directly; `orEmpty()` yields an empty list,
/// which makes the quantifier false rather than a null-pointer. ~keep
fn resolve_wildcard_array_accessor(
    field_resolver: &FieldResolver,
    result_var: &str,
    array_part: &str,
    accessor_lang: &str,
) -> String {
    let raw_array_accessor = if array_part.is_empty() {
        result_var.to_string()
    } else {
        field_resolver.accessor(array_part, accessor_lang, result_var)
    };
    let array_is_nullable =
        raw_array_accessor.contains("?.") || (!array_part.is_empty() && field_resolver.is_optional(array_part));
    if array_is_nullable {
        format!("{raw_array_accessor}.orEmpty()")
    } else {
        raw_array_accessor
    }
}

/// The wildcard-traversal gate's own per-assertion-type dispatch, split out of
/// `try_render_wildcard_traversal_assertion` to keep it under the file's function-length cap --
/// verbatim extraction, no behavior change. ~keep
fn render_wildcard_traversal_match(
    out: &mut String,
    assertion: &Assertion,
    f: &str,
    array_accessor: &str,
    elem_accessor: &str,
) {
    match assertion.assertion_type.as_str() {
        "contains" | "contains_all" | "not_contains" => {
            let negated = assertion.assertion_type == "not_contains";
            let assert_fn = if negated { "assertFalse" } else { "assertTrue" };
            let expectation = if negated {
                "expected NOT to contain: "
            } else {
                "expected to contain: "
            };
            for expected in assertion.expected_values() {
                let kotlin_val = super::values::json_to_kotlin(expected);
                let _ = writeln!(
                    out,
                    "        {assert_fn}({array_accessor}.any {{ e -> {elem_accessor}.toString().contains({kotlin_val}) }}, \"{expectation}\" + {kotlin_val})"
                );
            }
        }
        "not_empty" => {
            let _ = writeln!(
                out,
                "        assertTrue({array_accessor}.any {{ e -> {elem_accessor}.toString().isNotEmpty() }}, \"expected a non-empty element in '{f}'\")"
            );
        }
        other => {
            let _ = writeln!(
                out,
                "        // skipped: unsupported traversal assertion '{other}' on '{f}'"
            );
        }
    }
}
