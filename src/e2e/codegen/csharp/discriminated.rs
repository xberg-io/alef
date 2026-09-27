//! C# discriminated-union assertion helpers.

use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
use crate::e2e::escape::escape_csharp;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use heck::ToUpperCamelCase;
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};

use super::json_to_csharp;

/// Render an assertion against a discriminated union variant's inner field.
/// `variant_var` is the unwrapped union variant (e.g., `variant` from pattern match).
/// `inner_field` is the field to access on the variant's Value (e.g., `sheet_count`).
///
/// ~keep Split into `render_bare_variant_payload_assertion` (no inner field — assert against
/// the payload value itself) and `render_discriminated_scalar_assertion` (the per-assertion-type
/// match) to stay under the file's function-length cap. Every statement below is a verbatim
/// extraction from the original single function; no behavior changed.
pub(super) struct DiscriminatedUnionAssertionContext<'a> {
    pub(super) variant_var: &'a str,
    pub(super) inner_field: &'a str,
    pub(super) field_is_collection: bool,
    pub(super) _result_is_vec: bool,
    pub(super) assert_enum_fields: &'a std::collections::HashMap<String, String>,
}

pub(super) fn render_discriminated_union_assertion(
    out: &mut String,
    assertion: &Assertion,
    context: DiscriminatedUnionAssertionContext,
) {
    let DiscriminatedUnionAssertionContext {
        variant_var,
        inner_field,
        field_is_collection,
        _result_is_vec,
        assert_enum_fields,
    } = context;
    if inner_field.is_empty() {
        render_bare_variant_payload_assertion(out, assertion, variant_var, field_is_collection);
        return;
    }

    let field_pascal = inner_field.to_upper_camel_case();
    let mut field_expr = format!("{variant_var}.Value.{field_pascal}");

    // Wrap enum fields with display helper
    if let Some(type_name) = assert_enum_fields.get(&field_pascal) {
        field_expr = format!("{type_name}Display.ToDisplayString({field_expr})");
    }

    render_discriminated_scalar_assertion(out, assertion, &field_expr, field_is_collection);
}

/// No field inside the payload — the fixture asserts against the payload VALUE itself (e.g.
/// `outcome.found` where `Found` wraps `List<Item>` directly, rather than a class with an inner
/// field). `field_is_collection` here comes from `union_variant_payload_is_collection` (the
/// caller switches source per `union_variant_field_is_collection`'s doc comment), so `count_min`
/// is the one assertion type this shape can substantiate today; every other shape stays the same
/// registered skip a callsite bug used to render as nothing at all. ~keep
fn render_bare_variant_payload_assertion(
    out: &mut String,
    assertion: &Assertion,
    variant_var: &str,
    field_is_collection: bool,
) {
    if assertion.assertion_type == "count_min" && field_is_collection {
        if let Some(count) = assertion.value.as_ref().and_then(serde_json::Value::as_u64) {
            let payload_expr = format!("{variant_var}.Value");
            let _ = writeln!(
                out,
                "            Assert.True(({payload_expr}?.Count ?? 0) >= {count}, \"expected count >= {count}\");"
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
        "contains_all" => render_discriminated_contains_all(out, assertion, field_expr),
        "contains" => render_discriminated_contains(out, assertion, field_expr),
        "not_empty" => {
            let _ = writeln!(out, "            Assert.NotEmpty({field_expr});");
        }
        "is_empty" => {
            let _ = writeln!(out, "            Assert.Empty({field_expr});");
        }
        "count_min" if field_is_collection => render_discriminated_count_min(out, assertion, field_expr),
        _ => {
            render_unsupported_assertion(out, assertion);
        }
    }
}

fn render_discriminated_equals(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(expected) = &assertion.value {
        let cs_val = json_to_csharp(expected);
        if expected.is_string() {
            let _ = writeln!(out, "            Assert.Equal({cs_val}, {field_expr}!.Trim());");
        } else if expected.as_bool() == Some(true) {
            let _ = writeln!(out, "            Assert.True({field_expr});");
        } else if expected.as_bool() == Some(false) {
            let _ = writeln!(out, "            Assert.False({field_expr});");
        } else if expected.is_number() && !expected.as_f64().is_some_and(|f| f.fract() != 0.0) {
            let _ = writeln!(out, "            Assert.True({field_expr} == {cs_val});");
        } else {
            let _ = writeln!(out, "            Assert.Equal({cs_val}, {field_expr});");
        }
    }
}

fn render_discriminated_greater_than_or_equal(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(val) = &assertion.value {
        let cs_val = json_to_csharp(val);
        let _ = writeln!(
            out,
            "            Assert.True({field_expr} >= {cs_val}, \"expected >= {cs_val}\");"
        );
    }
}

fn render_discriminated_contains_all(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(values) = &assertion.values {
        let field_as_str = format!("JsonSerializer.Serialize({field_expr})");
        for val in values {
            let lower_val = val.as_str().map(|s| s.to_lowercase());
            let cs_val = lower_val
                .as_deref()
                .map(|s| format!("\"{}\"", escape_csharp(s)))
                .unwrap_or_else(|| json_to_csharp(val));
            let _ = writeln!(out, "            Assert.Contains({cs_val}, {field_as_str}.ToLower());");
        }
    }
}

fn render_discriminated_contains(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(expected) = &assertion.value {
        let field_as_str = format!("JsonSerializer.Serialize({field_expr})");
        let lower_expected = expected.as_str().map(|s| s.to_lowercase());
        let cs_val = lower_expected
            .as_deref()
            .map(|s| format!("\"{}\"", escape_csharp(s)))
            .unwrap_or_else(|| json_to_csharp(expected));
        let _ = writeln!(out, "            Assert.Contains({cs_val}, {field_as_str}.ToLower());");
    }
}

fn render_discriminated_count_min(out: &mut String, assertion: &Assertion, field_expr: &str) {
    if let Some(count) = assertion.value.as_ref().and_then(serde_json::Value::as_u64) {
        let _ = writeln!(
            out,
            "            Assert.True(({field_expr}?.Count ?? 0) >= {count}, \"expected count >= {count}\");"
        );
    } else {
        render_unsupported_assertion(out, assertion);
    }
}

fn render_unsupported_assertion(out: &mut String, assertion: &Assertion) {
    let reason = AssertionTypeSkip::DiscriminatedUnionAssertionTypeNotSupported.message(&assertion.assertion_type);
    let _ = writeln!(out, "            // skipped: {reason}");
}

/// An empty suffix means the fixture path named only the variant — no field is checked, so
/// `union_variant_field_is_collection` (which requires a non-empty field name) always answers
/// `false` for it. Whether the variant's PAYLOAD itself is a collection is a different,
/// IR-backed question for that shape. Shared shape with kotlin's identically-named helper —
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

pub(super) fn try_render_generic_union_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    result_var: &str,
    field: &str,
    assert_enum_fields: &std::collections::HashMap<String, String>,
) -> bool {
    let Some((prefix, union_type, variant, suffix)) = field_resolver.ir_tagged_union_split(field) else {
        return false;
    };
    if field_resolver.union_variant_payload(&union_type, &variant).is_none() {
        render_unsupported_assertion(out, assertion);
        return true;
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    field.hash(&mut hasher);
    let variant_var = format!("variant_{:08x}", hasher.finish() as u32);
    let container = field_resolver.accessor(&prefix, "csharp", result_var);
    let field_is_collection =
        resolve_union_field_is_collection(field_resolver, &prefix, &union_type, &variant, &suffix);
    let _ = writeln!(out, "        if ({container} is {union_type}.{variant} {variant_var})");
    let _ = writeln!(out, "        {{");
    render_discriminated_union_assertion(
        out,
        assertion,
        DiscriminatedUnionAssertionContext {
            variant_var: &variant_var,
            inner_field: &suffix,
            field_is_collection,
            _result_is_vec: false,
            assert_enum_fields,
        },
    );
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        else");
    let _ = writeln!(out, "        {{");
    let _ = writeln!(out, "            Assert.Fail(\"Expected {variant} variant\");");
    let _ = writeln!(out, "        }}");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
    use crate::e2e::field_access::FieldResolver;

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

    fn xberg_html_resolver() -> FieldResolver {
        let types = vec![
            TypeDef {
                name: "ExtractionResult".to_string(),
                fields: vec![field(
                    "results",
                    TypeRef::Vec(Box::new(TypeRef::Named("ExtractedDocument".to_string()))),
                )],
                ..TypeDef::default()
            },
            TypeDef {
                name: "ExtractedDocument".to_string(),
                fields: vec![field("metadata", TypeRef::Named("Metadata".to_string()))],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Metadata".to_string(),
                fields: vec![field("format", TypeRef::Named("FormatMetadata".to_string()))],
                ..TypeDef::default()
            },
            TypeDef {
                name: "HtmlMetadata".to_string(),
                fields: vec![field(
                    "headers",
                    TypeRef::Vec(Box::new(TypeRef::Named("HeaderMetadata".to_string()))),
                )],
                ..TypeDef::default()
            },
            TypeDef {
                name: "HeaderMetadata".to_string(),
                ..TypeDef::default()
            },
        ];
        let enums = vec![EnumDef {
            name: "FormatMetadata".to_string(),
            variants: vec![EnumVariant {
                name: "Html".to_string(),
                fields: vec![field("value", TypeRef::Named("HtmlMetadata".to_string()))],
                ..EnumVariant::default()
            }],
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
            Some("ExtractionResult".to_string()),
        )
        .with_ir_collection_map(
            FieldResolver::ir_collection_fields(&types),
            Some("ExtractionResult".to_string()),
        )
    }

    #[test]
    fn count_min_on_a_union_payload_collection_renders_a_count_assertion() {
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
            DiscriminatedUnionAssertionContext {
                variant_var: "webVariant",
                inner_field: "entries",
                field_is_collection: resolver().union_variant_field_is_collection("details", "web", "entries"),
                _result_is_vec: false,
                assert_enum_fields: &std::collections::HashMap::new(),
            },
        );

        assert_eq!(
            out,
            "            Assert.True((webVariant.Value.Entries?.Count ?? 0) >= 2, \"expected count >= 2\");\n"
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
            DiscriminatedUnionAssertionContext {
                variant_var: "webVariant",
                inner_field: "label",
                field_is_collection: resolver().union_variant_field_is_collection("details", "web", "label"),
                _result_is_vec: false,
                assert_enum_fields: &std::collections::HashMap::new(),
            },
        );

        assert_eq!(
            out,
            "            // skipped: assertion type 'count_min' not yet supported for discriminated union fields\n"
        );
    }

    #[test]
    fn count_min_uses_ir_tagged_union_path_without_named_parser() {
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
            "details.web.entries",
            &std::collections::HashMap::new(),
        ));
        assert!(out.contains("DetailUnion.Web"));
        assert!(out.contains("Value.Entries?.Count ?? 0"));
        assert!(!out.contains("skipped:"));
    }

    /// Regression: a fixture path naming only the variant (`details.found`, no inner field)
    /// against a variant whose single payload field is ITSELF a collection
    /// (`Found(Vec<FoundEntry>)`) used to silently drop the assertion — `inner_field.is_empty()`
    /// returned before ever reaching a match arm, and `union_variant_field_is_collection`
    /// answers `false` for an empty field name regardless of the payload's real shape. It must
    /// now render a real count assertion against the payload value itself.
    #[test]
    fn count_min_on_a_bare_union_variant_collection_payload_renders_a_count_assertion() {
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
            "details.found",
            &std::collections::HashMap::new(),
        ));
        assert!(out.contains("DetailUnion.Found"));
        assert!(out.contains("Value?.Count ?? 0) >= 2"));
        assert!(!out.contains("skipped:"));
    }

    /// Negative control for the fix above: a bare-variant path (no inner field) against a
    /// variant whose single payload field is a CLASS, not a collection (`Web(WebPayload)`),
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
            "details.web",
            &std::collections::HashMap::new(),
        ));
        assert!(out.contains("DetailUnion.Web"));
        assert!(out.contains("// skipped: assertion type 'count_min' not yet supported"));
        assert!(out.contains("for discriminated union fields"));
    }

    #[test]
    fn xberg_html_headers_count_min_uses_the_ir_union_owner() {
        let field = "results[0].metadata.format.html.headers";
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
            &xberg_html_resolver(),
            "result",
            field,
            &std::collections::HashMap::new(),
        ));
        assert!(out.contains("result.Results[0].Metadata.Format is FormatMetadata.Html"));
        assert!(out.contains("Value.Headers?.Count ?? 0) >= 2"));
        assert!(!out.contains("skipped:"));
    }

    /// Scope-limit control: `Numbers(Vec<String>)` — a tuple variant whose payload is
    /// `Vec<primitive>` rather than `Vec<NamedType>` — has no entry in
    /// `variant_payload_types` at all (`named_type` cannot resolve a primitive element), so it
    /// is out of scope for `union_variant_payload_is_collection` and must fall through to the
    /// PRE-EXISTING "payload never resolved" skip. This proves the documented scope limit is a
    /// real, visible, registered skip — not a silent drop like the bug this change fixed.
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
            "details.numbers",
            &std::collections::HashMap::new(),
        ));
        assert!(!out.is_empty(), "must never render nothing at all");
        assert_eq!(
            out,
            "            // skipped: assertion type 'count_min' not yet supported for discriminated union fields\n"
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
                field,
                &std::collections::HashMap::new(),
            ));
            assert!(out.contains("skipped: assertion type 'count_min' not yet supported"));
        }
    }
}
