//! Kotlin discriminated-union (sealed class) assertion helpers.
//!
//! Mirrors `csharp/discriminated.rs` but emits Kotlin `is` pattern matching
//! against `FormatMetadata` subclasses.  Sealed class subclasses expose the
//! payload as a single `metadata` property (see `FormatMetadata.Excel(val metadata: ExcelMetadata)`),
//! so the inner field is accessed as `variant.metadata.<innerCamelCase>`.
//!
//! [`parse_discriminated_union_access`] only recognizes that one hand-maintained fixture
//! shape. `kotlin/assertions.rs::render_assertion` also drives
//! [`render_discriminated_union_assertion`] from a second, IR-general detector
//! (`FieldResolver::tagged_union_split` + `FieldResolver::union_variant_payload`) for any
//! other tagged union, computing the payload property name from the IR itself via
//! `kotlin_field_name_with_type` instead of assuming `"metadata"`.

use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::escape::escape_kotlin;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::values::json_to_kotlin;

/// Detect if a field path navigates a discriminated union variant.
/// Pattern: `metadata.format.<variant_name>(.<inner_field>)?`
/// Returns: Some((variant_pascal, inner_field_snake)) if matched.
pub(super) fn parse_discriminated_union_access(field: &str) -> Option<(String, String)> {
    // Strip a leading list-index prefix (e.g. "results[0].") so both single-result
    // (`metadata.format.excel.sheet_count`) and list-result
    // (`results[0].metadata.format.excel.sheet_count`) field paths are recognized.
    let field = field.split_once("].").map(|(_, rest)| rest).unwrap_or(field);
    let parts: Vec<&str> = field.split('.').collect();
    if !(parts.len() == 3 || parts.len() == 4) {
        return None;
    }
    if parts[0] != "metadata" || parts[1] != "format" {
        return None;
    }
    let variant_name = parts[2];
    let known_variants = [
        "pdf",
        "docx",
        "excel",
        "email",
        "pptx",
        "archive",
        "image",
        "xml",
        "text",
        "html",
        "ocr",
        "csv",
        "bibtex",
        "citation",
        "fiction_book",
        "dbf",
        "jats",
        "epub",
        "pst",
        "code",
    ];
    if !known_variants.contains(&variant_name) {
        return None;
    }
    let variant_pascal = variant_name.to_upper_camel_case();
    let inner_field = if parts.len() == 4 {
        parts[3].to_string()
    } else {
        String::new()
    };
    Some((variant_pascal, inner_field))
}

/// Render an assertion against a sealed-class variant's inner field.
///
/// `variant_var` is the bound name from `is FormatMetadata.<Variant> -> { … }`
/// (e.g. `format_excel`). `payload_field` is the sealed-class subclass's own
/// property that carries the variant's payload (e.g. `"metadata"`) — every hand-maintained
/// caller of this function passes the literal `"metadata"` because every variant in that
/// consumer's fixtures happens to wrap a type named `<Variant>Metadata`, but the property
/// name is a per-variant fact (see `kotlin_field_name_with_type` and its callers), not a
/// universal constant, so it is a parameter rather than baked in here.
///
/// ~keep Split into `render_bare_variant_payload_assertion` (no inner field — assert against the
/// payload value itself) and `render_discriminated_scalar_assertion` (the per-assertion-type
/// match) to stay under the file's function-length cap. Every statement below is a verbatim
/// extraction from the original single function; no behavior changed.
pub(super) fn render_discriminated_union_assertion(
    out: &mut String,
    assertion: &Assertion,
    variant_var: &str,
    payload_field: &str,
    inner_field: &str,
    field_is_collection: bool,
) {
    if inner_field.is_empty() {
        render_bare_variant_payload_assertion(out, assertion, variant_var, payload_field, field_is_collection);
        return;
    }

    let field_camel = inner_field.to_lower_camel_case();
    // The variant payload field (`variant.<payload_field>.<inner>`) is frequently Optional in
    // the alef-generated Kotlin types (e.g. `ExcelMetadata.sheetCount: Int?`).  The fixture
    // assertion only fires when the variant matched, so a null inner field would itself be
    // a test failure — assert non-null with `!!.` before the comparison so kotlinc accepts
    // arithmetic and ordering operators (`>=`, `>`, etc.) on the receiver.
    let field_expr = format!("{variant_var}.{payload_field}.{field_camel}!!");

    render_discriminated_scalar_assertion(out, assertion, &field_expr, field_is_collection);
}

/// No field inside the payload — the fixture asserts against the payload VALUE itself (e.g.
/// `outcome.found` where `Found` wraps `Vec<Item>` directly, rather than a struct with an inner
/// field). `field_is_collection` here comes from `union_variant_payload_is_collection` (the
/// caller switches source per `Self::union_variant_field_is_collection`'s doc comment), so
/// `count_min` is the one assertion type this shape can substantiate today; every other shape
/// stays the same registered skip a callsite bug used to render as nothing at all. ~keep
fn render_bare_variant_payload_assertion(
    out: &mut String,
    assertion: &Assertion,
    variant_var: &str,
    payload_field: &str,
    field_is_collection: bool,
) {
    if assertion.assertion_type == "count_min" && field_is_collection {
        if let Some(count) = assertion.value.as_ref().and_then(serde_json::Value::as_u64) {
            let payload_expr = format!("{variant_var}.{payload_field}!!");
            let _ = writeln!(
                out,
                "                assertTrue({payload_expr}.size >= {count}, \"expected count >= {count}\")"
            );
        } else {
            render_unsupported_assertion(out, assertion);
        }
    } else {
        render_unsupported_assertion(out, assertion);
    }
}

/// Dispatch on assertion type once `field_expr` (the variant's inner-field accessor) is known.
/// `not_empty`/`is_empty` stay inline — each is a single statement — every other arm is a
/// verbatim-extracted helper. ~keep
fn render_discriminated_scalar_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_expr: &str,
    field_is_collection: bool,
) {
    match assertion.assertion_type.as_str() {
        "equals" => render_discriminated_equals(out, assertion, field_expr),
        "greater_than_or_equal" => render_discriminated_greater_than_or_equal(out, assertion, field_expr),
        "less_than_or_equal" => render_discriminated_less_than_or_equal(out, assertion, field_expr),
        "greater_than" => render_discriminated_greater_than(out, assertion, field_expr),
        "less_than" => render_discriminated_less_than(out, assertion, field_expr),
        "contains" => render_discriminated_contains(out, assertion, field_expr),
        "contains_all" => render_discriminated_contains_all(out, assertion, field_expr),
        "not_empty" => {
            let _ = writeln!(
                out,
                "                assertTrue({field_expr}.toString().isNotEmpty(), \"expected non-empty value\")"
            );
        }
        "is_empty" => {
            let _ = writeln!(
                out,
                "                assertTrue({field_expr}.toString().isEmpty(), \"expected empty value\")"
            );
        }
        "count_min" if field_is_collection => render_discriminated_count_min(out, assertion, field_expr),
        _ => {
            render_unsupported_assertion(out, assertion);
        }
    }
}

fn render_discriminated_equals(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(expected) = &assertion.value {
        let kt_val = json_to_kotlin(expected);
        if expected.is_string() {
            let _ = writeln!(
                out,
                "                assertEquals({kt_val}, {field_expr}.trim(), \"expected: {}\")",
                escape_kotlin(expected.as_str().unwrap_or(""))
            );
        } else if expected.as_bool() == Some(true) {
            let _ = writeln!(
                out,
                "                assertTrue({field_expr} == true, \"expected true\")"
            );
        } else if expected.as_bool() == Some(false) {
            let _ = writeln!(
                out,
                "                assertTrue({field_expr} == false, \"expected false\")"
            );
        } else {
            let _ = writeln!(
                out,
                "                assertEquals({kt_val}, {field_expr}, \"expected: {kt_val}\")"
            );
        }
    }
}

fn render_discriminated_greater_than_or_equal(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(val) = &assertion.value {
        let kt_val = json_to_kotlin(val);
        let _ = writeln!(
            out,
            "                assertTrue({field_expr} >= {kt_val}, \"expected >= {kt_val}\")"
        );
    }
}

fn render_discriminated_less_than_or_equal(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(val) = &assertion.value {
        let kt_val = json_to_kotlin(val);
        let _ = writeln!(
            out,
            "                assertTrue({field_expr} <= {kt_val}, \"expected <= {kt_val}\")"
        );
    }
}

fn render_discriminated_greater_than(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(val) = &assertion.value {
        let kt_val = json_to_kotlin(val);
        let _ = writeln!(
            out,
            "                assertTrue({field_expr} > {kt_val}, \"expected > {kt_val}\")"
        );
    }
}

fn render_discriminated_less_than(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(val) = &assertion.value {
        let kt_val = json_to_kotlin(val);
        let _ = writeln!(
            out,
            "                assertTrue({field_expr} < {kt_val}, \"expected < {kt_val}\")"
        );
    }
}

fn render_discriminated_contains(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(expected) = &assertion.value
        && let Some(s) = expected.as_str()
    {
        let lower = s.to_lowercase();
        let _ = writeln!(
            out,
            "                assertTrue({field_expr}.orEmpty().toString().lowercase().contains(\"{}\".lowercase()), \"expected to contain: {}\")",
            escape_kotlin(&lower),
            escape_kotlin(s)
        );
    }
}

fn render_discriminated_contains_all(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(values) = &assertion.values {
        for val in values {
            if let Some(s) = val.as_str() {
                let lower = s.to_lowercase();
                let _ = writeln!(
                    out,
                    "                assertTrue({field_expr}.orEmpty().toString().lowercase().contains(\"{}\".lowercase()), \"expected to contain: {}\")",
                    escape_kotlin(&lower),
                    escape_kotlin(s)
                );
            }
        }
    }
}

fn render_discriminated_count_min(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(count) = assertion.value.as_ref().and_then(serde_json::Value::as_u64) {
        let _ = writeln!(
            out,
            "                assertTrue({field_expr}.size >= {count}, \"expected count >= {count}\")"
        );
    } else {
        render_unsupported_assertion(out, assertion);
    }
}

fn render_unsupported_assertion(out: &mut String, assertion: &Assertion) {
    let reason = AssertionTypeSkip::DiscriminatedUnionAssertionTypeNotSupported.message(&assertion.assertion_type);
    let _ = writeln!(out, "                // skipped: {reason}");
}

/// An empty suffix means the fixture path named only the variant — no field is checked, so
/// `union_variant_field_is_collection` (which requires a non-empty field name) always answers
/// `false` for it. Whether the variant's PAYLOAD itself is a collection is a different,
/// IR-backed question for that shape. Shared shape with csharp's identically-named helper —
/// each backend keeps its own copy since they are separate modules with no shared parent for
/// this cross-backend concern. ~keep
fn resolve_union_field_is_collection(
    field_resolver: &FieldResolver,
    prefix: &str,
    union_type: &str,
    variant: &str,
    suffix: &str,
) -> bool {
    if suffix.is_empty() {
        field_resolver.union_variant_payload_is_collection(union_type, variant)
    } else {
        field_resolver.union_variant_field_is_collection(prefix, variant, suffix)
    }
}

/// The union-traversal skip for a variant whose IR-declared payload `try_render_generic_union_assertion`
/// could not resolve to a single named field (`union_variant_payload` returned `None`). ~keep
fn render_union_traversal_not_implemented_skip(out: &mut String, f: &str) {
    let _ = writeln!(
        out,
        "        // skipped: {}",
        FieldSkip::UnionTraversalNotImplementedForKotlin.message(f)
    );
}

/// Inputs to [`resolve_kotlin_union_variant_binding`], grouped to keep the function under poly's
/// `too-many-parameters` limit. Field order matches the parameter order it replaces -- no
/// behavior change.
struct KotlinUnionVariantBindingInputs<'a> {
    field_resolver: &'a FieldResolver,
    prefix: &'a str,
    variant_pascal: &'a str,
    payload_field_name: &'a str,
    payload_type: &'a str,
    result_var: &'a str,
    kotlin_android_style: bool,
}

/// Resolve the Kotlin-specific binding for a matched union variant: the `when` binding name, the
/// container accessor to narrow, and the sealed-subclass property that carries the variant's
/// payload (kotlin vs. kotlin_android accessor style differs; the payload property name is
/// resolved from the IR via `kotlin_field_name_with_type`, the same helper the Kotlin binding
/// backend itself uses to name it). Split out of `try_render_generic_union_assertion` to keep it
/// under the file's function-length cap — verbatim extraction, no behavior change. ~keep
fn resolve_kotlin_union_variant_binding(inputs: KotlinUnionVariantBindingInputs) -> (String, String, String) {
    let KotlinUnionVariantBindingInputs {
        field_resolver,
        prefix,
        variant_pascal,
        payload_field_name,
        payload_type,
        result_var,
        kotlin_android_style,
    } = inputs;
    let style = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    let variant_var = format!("union{variant_pascal}");
    let container = field_resolver.accessor(prefix, style, result_var);
    let payload_field = crate::backends::kotlin::kotlin_field_name_with_type(
        payload_field_name,
        0,
        Some(payload_type),
        variant_pascal,
        1,
    );
    (variant_var, container, payload_field)
}

/// The IR-general sealed-class narrowing path: recognizes ANY tagged union
/// [`parse_discriminated_union_access`]'s hand-maintained `metadata.format.<variant>` parser
/// does not, using [`FieldResolver::tagged_union_split`] (the same detector Gleam, Dart and
/// Swift already consult, driven by the consumer's `fields_method_calls` config) to find the
/// boundary and [`FieldResolver::union_variant_payload`] to ask the IR which single field the
/// matched variant wraps. The payload property's real Kotlin name comes from
/// `kotlin_field_name_with_type` — the exact helper the Kotlin binding backend itself uses to
/// name it — rather than assuming `"metadata"`, so this also covers unions whose variants wrap
/// a differently-named or differently-typed payload.
///
/// Returns `true` when it rendered something (either a real narrowing block, or — for a
/// boundary it detected but could not lower, e.g. a multi-field variant, or a union type the
/// IR never anchored — the loud, named [`FieldSkip::UnionTraversalNotImplementedForKotlin`]
/// skip) and `false` when `f` does not cross a tagged-union boundary at all, in which case the
/// caller must keep trying its other field-shape branches. Never falls through to
/// `field_resolver.accessor` for the field paths it recognizes: that renderer walks flat
/// property/method chains and does not know pattern matching exists, so on a sealed class it
/// would emit a chain like `.excel().sheetCount()` that alef's own generated Kotlin does not
/// declare a method for. ~keep
pub(super) fn try_render_generic_union_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    result_var: &str,
    kotlin_android_style: bool,
    f: &str,
) -> bool {
    let Some((prefix, union_type, variant_pascal, suffix)) = field_resolver.ir_tagged_union_split(f) else {
        return false;
    };
    let Some((payload_field_name, payload_type)) = field_resolver.union_variant_payload(&union_type, &variant_pascal)
    else {
        render_union_traversal_not_implemented_skip(out, f);
        return true;
    };
    let payload_field_name = payload_field_name.to_string();
    let payload_type = payload_type.to_string();

    let (variant_var, container, payload_field) =
        resolve_kotlin_union_variant_binding(KotlinUnionVariantBindingInputs {
            field_resolver,
            prefix: &prefix,
            variant_pascal: &variant_pascal,
            payload_field_name: &payload_field_name,
            payload_type: &payload_type,
            result_var,
            kotlin_android_style,
        });
    let _ = writeln!(out, "        when (val {variant_var} = {container}) {{");
    let _ = writeln!(out, "            is {union_type}.{variant_pascal} -> {{");
    let field_is_collection =
        resolve_union_field_is_collection(field_resolver, &prefix, &union_type, &variant_pascal, &suffix);
    render_discriminated_union_assertion(
        out,
        assertion,
        &variant_var,
        &payload_field,
        &suffix,
        field_is_collection,
    );
    let _ = writeln!(out, "            }}");
    let _ = writeln!(
        out,
        "            else -> kotlin.test.assertTrue(false, \"Expected {variant_pascal} variant\")"
    );
    let _ = writeln!(out, "        }}");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn resolver() -> FieldResolver {
        let types = vec![
            TypeDef {
                name: "Envelope".to_string(),
                fields: vec![field("details", TypeRef::Named("DetailUnion".to_string()))],
                ..TypeDef::default()
            },
            TypeDef {
                name: "WebPayload".to_string(),
                fields: vec![
                    field("entries", TypeRef::Vec(Box::new(TypeRef::String))),
                    field("label", TypeRef::String),
                ],
                ..TypeDef::default()
            },
        ];
        let enums = vec![EnumDef {
            name: "DetailUnion".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Web".to_string(),
                    fields: vec![field("payload", TypeRef::Named("WebPayload".to_string()))],
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Empty".to_string(),
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Pair".to_string(),
                    fields: vec![field("left", TypeRef::String), field("right", TypeRef::String)],
                    ..EnumVariant::default()
                },
                // A tuple variant whose single field wraps a collection DIRECTLY
                // (`Found(Vec<FoundEntry>)`), rather than a struct with a collection field
                // inside it (`Web(WebPayload)`, `WebPayload.entries: Vec<String>`). A fixture
                // path naming only this variant (`details.found`, no inner field) asserts
                // against the payload value itself. ~keep
                EnumVariant {
                    name: "Found".to_string(),
                    fields: vec![field(
                        "_0",
                        TypeRef::Vec(Box::new(TypeRef::Named("FoundEntry".to_string()))),
                    )],
                    ..EnumVariant::default()
                },
                // A tuple variant whose single field is `Vec<primitive>` — no element type
                // name for `named_type` to resolve, so `variant_payload_types` (and therefore
                // `variant_payload_is_collection`) never records it at all: this shape stays
                // out of scope for the count_min-on-bare-payload fix and must fall through to
                // the pre-existing "union traversal not implemented" skip. ~keep
                EnumVariant {
                    name: "Numbers".to_string(),
                    fields: vec![field("_0", TypeRef::Vec(Box::new(TypeRef::String)))],
                    ..EnumVariant::default()
                },
            ],
            ..EnumDef::default()
        }];
        FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        )
        .with_ir_enum_map(
            FieldResolver::ir_enum_fields(&types, &enums),
            Some("Envelope".to_string()),
        )
        .with_ir_collection_map(
            FieldResolver::ir_collection_fields(&types),
            Some("Envelope".to_string()),
        )
    }

    #[test]
    fn count_min_on_a_union_payload_collection_renders_a_size_assertion() {
        let assertion = Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("details.web.entries".to_string()),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        };
        let mut out = String::new();

        render_discriminated_union_assertion(
            &mut out,
            &assertion,
            "webVariant",
            "payload",
            "entries",
            resolver().union_variant_field_is_collection("details", "web", "entries"),
        );

        assert_eq!(
            out,
            "                assertTrue(webVariant.payload.entries!!.size >= 2, \"expected count >= 2\")\n"
        );
    }

    #[test]
    fn count_min_on_a_union_payload_scalar_stays_an_explicit_skip() {
        let assertion = Assertion {
            assertion_type: "count_min".to_string(),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        };
        let mut out = String::new();

        render_discriminated_union_assertion(
            &mut out,
            &assertion,
            "webVariant",
            "payload",
            "label",
            resolver().union_variant_field_is_collection("details", "web", "label"),
        );

        assert_eq!(
            out,
            "                // skipped: assertion type 'count_min' not yet supported for discriminated union fields\n"
        );
    }

    #[test]
    fn count_min_uses_ir_tagged_union_path_without_method_call_config() {
        let assertion = Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("details.web.entries".to_string()),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        };
        let mut out = String::new();

        assert!(try_render_generic_union_assertion(
            &mut out,
            &assertion,
            &resolver(),
            "result",
            true,
            "details.web.entries",
        ));
        assert!(out.contains("DetailUnion.Web"));
        assert!(out.contains("payload.entries!!.size >= 2"));
        assert!(!out.contains("skipped:"));
    }

    /// Regression: a fixture path naming only the variant (`details.found`, no inner field)
    /// against a variant whose single payload field is ITSELF a collection
    /// (`Found(Vec<FoundEntry>)`) used to silently drop the assertion — `inner_field.is_empty()`
    /// returned before ever reaching a match arm, and `union_variant_field_is_collection`
    /// answers `false` for an empty field name regardless of the payload's real shape. It must
    /// now render a real size assertion against the payload value itself.
    #[test]
    fn count_min_on_a_bare_union_variant_collection_payload_renders_a_size_assertion() {
        let assertion = Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("details.found".to_string()),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        };
        let mut out = String::new();

        assert!(try_render_generic_union_assertion(
            &mut out,
            &assertion,
            &resolver(),
            "result",
            true,
            "details.found",
        ));
        assert_eq!(
            out,
            concat!(
                "        when (val unionFound = result.details) {\n",
                "            is DetailUnion.Found -> {\n",
                "                assertTrue(unionFound.entry!!.size >= 2, \"expected count >= 2\")\n",
                "            }\n",
                "            else -> kotlin.test.assertTrue(false, \"Expected Found variant\")\n",
                "        }\n",
            )
        );
    }

    /// Negative control for the fix above: a bare-variant path (no inner field) against a
    /// variant whose single payload field is a STRUCT, not a collection (`Web(WebPayload)`),
    /// must stay an explicit, registered skip rather than being mistaken for the
    /// collection-payload shape.
    #[test]
    fn count_min_on_a_bare_union_variant_scalar_payload_stays_an_explicit_skip() {
        let assertion = Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("details.web".to_string()),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        };
        let mut out = String::new();

        assert!(try_render_generic_union_assertion(
            &mut out,
            &assertion,
            &resolver(),
            "result",
            true,
            "details.web",
        ));
        assert_eq!(
            out,
            concat!(
                "        when (val unionWeb = result.details) {\n",
                "            is DetailUnion.Web -> {\n",
                "                // skipped: assertion type 'count_min' not yet supported ",
                "for discriminated union fields\n",
                "            }\n",
                "            else -> kotlin.test.assertTrue(false, \"Expected Web variant\")\n",
                "        }\n",
            )
        );
    }

    /// Scope-limit control: `Numbers(Vec<String>)` — a tuple variant whose payload is
    /// `Vec<primitive>` rather than `Vec<NamedType>` — has no entry in
    /// `variant_payload_types` at all (`named_type` cannot resolve a primitive element), so it
    /// is out of scope for `union_variant_payload_is_collection` and must fall through to the
    /// PRE-EXISTING "union traversal not implemented" skip. This proves the documented
    /// scope limit is a real, visible, registered skip — not a silent drop like the bug this
    /// change fixed.
    #[test]
    fn bare_union_variant_with_primitive_vec_payload_stays_a_registered_skip_not_silence() {
        let assertion = Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("details.numbers".to_string()),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        };
        let mut out = String::new();

        assert!(try_render_generic_union_assertion(
            &mut out,
            &assertion,
            &resolver(),
            "result",
            true,
            "details.numbers",
        ));
        assert!(!out.is_empty(), "must never render nothing at all");
        assert_eq!(
            out,
            concat!(
                "        // skipped: field 'details.numbers' crosses a tagged-union variant ",
                "boundary alef does not yet lower for this variant shape in Kotlin\n",
            )
        );
    }

    #[test]
    fn unsupported_union_shapes_stay_registered_skips() {
        for field in [
            "details.unknown.entries",
            "details.empty.entries",
            "details.pair.entries",
            "details.web.entries.value",
            "details.web.entries[0]",
        ] {
            let assertion = Assertion {
                assertion_type: "count_min".to_string(),
                field: Some(field.to_string()),
                value: Some(serde_json::json!(2)),
                ..Assertion::default()
            };
            let mut out = String::new();

            assert!(try_render_generic_union_assertion(
                &mut out,
                &assertion,
                &resolver(),
                "result",
                true,
                field,
            ));
            assert!(out.contains("skipped:"));
        }
    }
}
