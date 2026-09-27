use super::*;

#[cfg(test)]
mod strict_field_availability_marker_tests {
    use super::{RenderAssertionContext, render_assertion};
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    /// Regression test for alef task #81: Kotlin's "skipped: field not available"
    /// comment text must survive as the exact marker the shared
    /// `crate::e2e::codegen::fail_on_unavailable_field_markers` mechanism matches on
    /// (wired into `kotlin/test_method.rs`, shared by `kotlin` and `kotlin_android`),
    /// so arming `ALEF_E2E_STRICT_FIELD_AVAILABILITY` turns a dropped field assertion
    /// into a generation-time failure instead of a silently-passing comment.
    #[test]
    fn unavailable_field_skip_comment_carries_the_strict_mode_marker() {
        let result_fields: HashSet<String> = ["content".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        );
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("nonexistent_field".to_string()),
            value: Some(serde_json::json!("x")),
            ..Assertion::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            RenderAssertionContext {
                result_var: "result",
                class_name: "SampleClient",
                field_resolver: &resolver,
                result_is_simple: false,
                result_is_option: false,
                enum_fields: &HashSet::new(),
                json_scalar_fields: &HashSet::new(),
                fields_c_types: &HashMap::new(),
                is_streaming: false,
                kotlin_android_style: false,
                not_error_may_assert_presence: true,
            },
        );
        assert!(out.contains("field 'nonexistent_field' not available"), "got: {out}");
    }
}

#[cfg(test)]
mod is_true_optional_field_tests {
    use super::{RenderAssertionContext, render_assertion};
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn render(assertion: &Assertion, optional_field: &str, kotlin_android_style: bool) -> String {
        let optional: HashSet<String> = [optional_field.to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &optional,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let mut out = String::new();
        render_assertion(
            &mut out,
            assertion,
            RenderAssertionContext {
                result_var: "result",
                class_name: "SampleClient",
                field_resolver: &resolver,
                result_is_simple: false,
                result_is_option: false,
                enum_fields: &HashSet::new(),
                json_scalar_fields: &HashSet::new(),
                fields_c_types: &HashMap::new(),
                is_streaming: false,
                kotlin_android_style,
                not_error_may_assert_presence: true,
            },
        );
        out
    }

    fn is_true_assertion(field: &str) -> Assertion {
        Assertion {
            assertion_type: "is_true".to_string(),
            field: Some(field.to_string()),
            ..Assertion::default()
        }
    }

    /// `Option<DataNode>` presence on the Kotlin (JVM) target: before the fix this rendered
    /// `assertTrue(result.data() == true, ...)`, which compiles (`==` is Any?-to-Any?
    /// structural equality) but is always false for a present non-Boolean nullable.
    #[test]
    fn kotlin_is_true_on_optional_struct_field_checks_presence() {
        let out = render(&is_true_assertion("data"), "data", false);
        assert_eq!(
            out,
            "        assertTrue(result.data() != null, \"expected true (non-null)\")\n"
        );
    }

    /// Same fixture, kotlin_android target: properties (no `()`), same nullability fix.
    #[test]
    fn kotlin_android_is_true_on_optional_struct_field_checks_presence() {
        let out = render(&is_true_assertion("data"), "data", true);
        assert_eq!(
            out,
            "        assertTrue(result.data != null, \"expected true (non-null)\")\n"
        );
    }

    #[test]
    fn kotlin_android_is_false_on_optional_struct_field_checks_absence() {
        let out = render(
            &Assertion {
                assertion_type: "is_false".to_string(),
                field: Some("data".to_string()),
                ..Assertion::default()
            },
            "data",
            true,
        );
        assert_eq!(
            out,
            "        assertTrue(result.data == null, \"expected false (null)\")\n"
        );
    }

    #[test]
    fn kotlin_android_is_true_on_non_optional_field_is_unchanged() {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let mut out = String::new();
        render_assertion(
            &mut out,
            &is_true_assertion("active"),
            RenderAssertionContext {
                result_var: "result",
                class_name: "SampleClient",
                field_resolver: &resolver,
                result_is_simple: false,
                result_is_option: false,
                enum_fields: &HashSet::new(),
                json_scalar_fields: &HashSet::new(),
                fields_c_types: &HashMap::new(),
                is_streaming: false,
                kotlin_android_style: true,
                not_error_may_assert_presence: true,
            },
        );
        assert_eq!(out, "        assertTrue(result.active == true, \"expected true\")\n");
    }
}

/// Task #367: a fixture field path that traverses a tagged-union variant boundary
/// (`<union>.<variant>.<field>`, e.g. `shape.circle.radius`) is not a flat member chain in
/// Kotlin — `ShapeKind` is a sealed class and `radius` only exists on the `Circle` variant's
/// payload. These tests exercise the IR-general narrowing path added alongside the existing
/// hand-maintained `metadata.format.<variant>` parser
/// (`FieldResolver::tagged_union_split` + `FieldResolver::union_variant_payload`, driven by
/// `discriminated::try_render_generic_union_assertion`), using neutral fixture names rather
/// than any consumer's real domain types.
#[cfg(test)]
mod union_traversal_tests {
    use super::{RenderAssertionContext, render_assertion};
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
            is_tuple: true,
            ..EnumVariant::default()
        }
    }

    /// `Report { shape: ShapeKind }`, `ShapeKind::Circle(CircleData)` (single payload, the
    /// `metadata.format.excel.sheet_count` shape), `ShapeKind::Square(width, height)` (two
    /// payload fields — no single type to narrow into), `CircleData { radius: u32 }`.
    fn shape_resolver(method_calls: &HashSet<String>, kotlin_android_style: bool) -> FieldResolver {
        let type_defs = vec![
            TypeDef {
                name: "Report".to_string(),
                fields: vec![field("shape", TypeRef::Named("ShapeKind".to_string()))],
                ..TypeDef::default()
            },
            TypeDef {
                name: "CircleData".to_string(),
                fields: vec![field("radius", TypeRef::Primitive(PrimitiveType::U32))],
                ..TypeDef::default()
            },
        ];
        let enums = vec![EnumDef {
            name: "ShapeKind".to_string(),
            variants: vec![
                variant("Circle", vec![field("_0", TypeRef::Named("CircleData".to_string()))]),
                variant(
                    "Square",
                    vec![
                        field("width", TypeRef::Primitive(PrimitiveType::U32)),
                        field("height", TypeRef::Primitive(PrimitiveType::U32)),
                    ],
                ),
            ],
            ..EnumDef::default()
        }];
        let _ = kotlin_android_style;
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            method_calls,
        )
        .with_ir_enum_map(
            FieldResolver::ir_enum_fields(&type_defs, &enums),
            Some("Report".to_string()),
        )
    }

    fn render(field_path: &str, resolver: &FieldResolver, kotlin_android_style: bool) -> String {
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field_path.to_string()),
            value: Some(serde_json::json!(5)),
            ..Assertion::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            RenderAssertionContext {
                result_var: "result",
                class_name: "SampleClient",
                field_resolver: resolver,
                result_is_simple: false,
                result_is_option: false,
                enum_fields: &HashSet::new(),
                json_scalar_fields: &HashSet::new(),
                fields_c_types: &HashMap::new(),
                is_streaming: false,
                kotlin_android_style,
                not_error_may_assert_presence: true,
            },
        );
        out
    }

    /// A single-payload variant narrows correctly on the plain (JVM, non-android) Kotlin
    /// target: `when (val union<Variant> = result.<field>()) { is <Union>.<Variant> -> { ... } }`,
    /// with the payload property name (`data`) computed from the IR the same way
    /// `kotlin_field_name_with_type` names it in the real generated binding — not hardcoded.
    #[test]
    fn single_payload_variant_narrows_on_plain_kotlin() {
        let method_calls: HashSet<String> = ["shape.circle".to_string()].into_iter().collect();
        let resolver = shape_resolver(&method_calls, false);
        let out = render("shape.circle.radius", &resolver, false);
        assert_eq!(
            out,
            "        when (val unionCircle = result.shape()) {\n\
             \x20           is ShapeKind.Circle -> {\n\
             \x20               assertEquals(5, unionCircle.data.radius!!, \"expected: 5\")\n\
             \x20           }\n\
             \x20           else -> kotlin.test.assertTrue(false, \"Expected Circle variant\")\n\
             \x20       }\n",
            "got: {out}"
        );
    }

    #[test]
    fn legacy_format_union_fails_when_the_runtime_variant_differs() {
        let result_fields: HashSet<String> = [
            "metadata".to_string(),
            "metadata.format".to_string(),
            "metadata.format.excel".to_string(),
            "metadata.format.excel.sheet_count".to_string(),
        ]
        .into_iter()
        .collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        );
        let out = render("metadata.format.excel.sheet_count", &resolver, true);

        assert!(
            out.contains("else -> kotlin.test.assertTrue(false, \"Expected Excel variant\")"),
            "got: {out}"
        );
    }

    /// Same union, kotlin_android style: the accessor for the union field itself switches to
    /// property syntax (`result.shape`, no parens) while the narrowing shape is unchanged.
    #[test]
    fn single_payload_variant_narrows_on_kotlin_android() {
        let method_calls: HashSet<String> = ["shape.circle".to_string()].into_iter().collect();
        let resolver = shape_resolver(&method_calls, true);
        let out = render("shape.circle.radius", &resolver, true);
        assert!(out.contains("when (val unionCircle = result.shape) {"), "got: {out}");
        assert!(out.contains("is ShapeKind.Circle -> {"), "got: {out}");
        assert!(
            out.contains("assertEquals(5, unionCircle.data.radius!!, \"expected: 5\")"),
            "got: {out}"
        );
    }

    /// A variant with two payload fields has no single type to narrow into
    /// (`union_variant_payload` returns `None`), so this must render the loud, named
    /// `UnionTraversalNotImplementedForKotlin` skip — never fall through to a flat accessor
    /// chain like `.square().width()` against a sealed class, which would not compile.
    #[test]
    fn multi_field_variant_emits_the_named_gap_marker_instead_of_a_broken_accessor() {
        let method_calls: HashSet<String> = ["shape.square".to_string()].into_iter().collect();
        let resolver = shape_resolver(&method_calls, false);
        let out = render("shape.square.width", &resolver, false);
        assert_eq!(
            out,
            "        // skipped: field 'shape.square.width' crosses a tagged-union variant \
             boundary alef does not yet lower for this variant shape in Kotlin\n",
            "got: {out}"
        );
        assert!(
            !out.contains(".square()"),
            "must not emit a flat accessor into the sealed class: {out}"
        );
    }

    /// Sabotage-adjacent control: a plain non-union path through the SAME resolver must be
    /// completely unaffected — `tagged_union_split` only fires when `method_calls` names the
    /// exact traversed prefix, so an unrelated field never routes through this new code.
    #[test]
    fn unrelated_field_is_unaffected_by_the_union_detector() {
        let method_calls: HashSet<String> = ["shape.circle".to_string()].into_iter().collect();
        let resolver = shape_resolver(&method_calls, false);
        let out = render("shape", &resolver, false);
        assert!(out.contains("assertEquals(5, result.shape())"), "got: {out}");
    }
}

#[cfg(test)]
mod wildcard_tests {
    use super::{RenderAssertionContext, render_assertion};
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn array_resolver(field: &str) -> FieldResolver {
        let names: HashSet<String> = [field.to_string()].into_iter().collect();
        FieldResolver::new(&HashMap::new(), &HashSet::new(), &names, &names, &HashSet::new())
    }

    fn render_contains(resolver: &FieldResolver, field: &str, value: &str) -> String {
        let assertion = Assertion {
            assertion_type: "contains".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Assertion::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            RenderAssertionContext {
                result_var: "result",
                class_name: "SampleClient",
                field_resolver: resolver,
                result_is_simple: false,
                result_is_option: false,
                enum_fields: &HashSet::new(),
                json_scalar_fields: &HashSet::new(),
                fields_c_types: &HashMap::new(),
                is_streaming: false,
                kotlin_android_style: false,
                not_error_may_assert_presence: true,
            },
        );
        out
    }

    /// Baseline: a single wildcard still quantifies over the whole list, so the refusal added
    /// for the nested case cannot have been implemented by refusing wildcards generally. ~keep
    #[test]
    fn single_wildcard_still_quantifies_over_every_element() {
        let out = render_contains(&array_resolver("links"), "links[].url", "example.test");
        assert!(out.contains(".any {"), "got: {out}");
        assert!(!out.contains(".first()"), "wildcard must not pin element 0, got: {out}");
    }

    /// `wildcard_split` consumes the first `[].` only, so before the guard the `any {}` ranged
    /// over `pages` while its lambda read `e.links().first().url()` — a whole-array claim that
    /// only ever inspected element zero of the inner list. Kotlin is the worst case for
    /// spotting this in review: `index == 0` renders as `.first()`, never as a literal `[0]`,
    /// so an index-free assertion would have passed vacuously. Pre-guard this test fails: the
    /// skip line is absent and `.first()` is present. ~keep
    #[test]
    fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
        let out = render_contains(&array_resolver("pages"), "pages[].links[].url", "example.test");
        assert_eq!(
            out, "        // skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
            "got: {out}"
        );
    }
}
