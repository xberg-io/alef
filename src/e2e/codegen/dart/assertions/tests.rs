use super::*;

/// Whether `dart` runs, not merely resolves: a version-manager shim (e.g. asdf, fvm) spawns fine
/// then exits non-zero, so checking only that the process spawned (`.output().is_err()`) would
/// leave the skip below unreachable and fire the assert everywhere Dart is absent. Shared by
/// `not_empty_nullability_tests` and `tagged_union_assertion_tests` below, which both gate a real
/// `dart analyze` compile gate the same way. ~keep
#[cfg(test)]
fn dart_is_runnable() -> bool {
    static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNABLE.get_or_init(|| {
        std::process::Command::new("dart")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

#[cfg(test)]
mod wildcard_tests {
    use super::{field_to_dart_accessor, render_assertion_dart};
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
        render_assertion_dart(&mut out, &assertion, "result", false, resolver);
        out
    }

    /// Baseline: a single wildcard still quantifies over the whole list, so the refusal added
    /// for the nested case cannot have been implemented by refusing wildcards generally. ~keep
    #[test]
    fn single_wildcard_still_quantifies_over_every_element() {
        let out = render_contains(&array_resolver("links"), "links[].url", "example.test");
        assert!(out.contains("result.links.any((e) => e.url"), "got: {out}");
    }

    fn array_resolver_with_enum_field(field: &str) -> FieldResolver {
        array_resolver(field.split("[].").next().unwrap_or(field)).with_enum_fields([field.to_string()].into())
    }

    /// Regression: `structure[].kind` is a data-carrying Rust enum. flutter_rust_bridge/freezed
    /// stringifies it as `'StructureKind.function()'` (lowerCamelCase constructor call), which a
    /// fixture's PascalCase variant name (`'Function'`) never case-sensitively matches — and
    /// there is no `wireValue` extension for a data-carrying enum to fall back on (alef only
    /// emits `wireValue` for unit-only enums). `.runtimeType.toString()` instead yields alef's
    /// generated concrete subclass name in the variant's original casing
    /// (`'StructureKind_Function'`), which does contain the fixture's PascalCase value. ~keep
    #[test]
    fn contains_on_an_enum_typed_array_element_field_compares_the_runtime_type_name() {
        let out = render_contains(
            &array_resolver_with_enum_field("structure[].kind"),
            "structure[].kind",
            "Function",
        );
        assert!(
            out.contains("e.kind.runtimeType.toString().contains"),
            "an enum-typed element field must compare against the runtime type name, not \
             toString(), got:\n{out}"
        );
        assert!(!out.contains("e.kind.toString().contains"), "got: {out}");
    }

    /// Negative control: a plain (non-enum) element field, e.g. a `String`, must keep comparing
    /// its actual stringified content — `.runtimeType.toString()` on a `String` yields the type
    /// name `'String'`, not the value, so switching this field too would break every
    /// currently-passing `imports[].source`-style assertion.
    #[test]
    fn contains_on_a_non_enum_array_element_field_still_compares_the_stringified_value() {
        let out = render_contains(&array_resolver("imports"), "imports[].source", "example.test");
        assert!(
            out.contains("e.source.toString().contains"),
            "a non-enum element field must keep comparing its stringified content, got:\n{out}"
        );
        assert!(!out.contains("runtimeType"), "got: {out}");
    }

    /// The evidence that Dart's inner collapse was never even index-0-shaped: the element
    /// accessor renders the surviving bare bracket verbatim. `links![]` is not valid Dart, so
    /// a doubly-nested fixture would have produced a file that fails to analyze. ~keep
    #[test]
    fn the_element_accessor_renders_a_surviving_wildcard_as_invalid_dart() {
        assert_eq!(field_to_dart_accessor("links[].url"), "links![].url");
    }

    /// Pre-guard this test fails: the skip line is absent and the emitted `any((e) => ...)`
    /// closure names `e.links![].url`. ~keep
    #[test]
    fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
        let out = render_contains(&array_resolver("pages"), "pages[].links[].url", "example.test");
        assert_eq!(
            out, "    // skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
            "got: {out}"
        );
    }

    fn resolver_with_alias(alias_from: &str, alias_to: &str, result_field: &str) -> FieldResolver {
        let aliases: HashMap<String, String> = [(alias_from.to_string(), alias_to.to_string())].into_iter().collect();
        let result_fields: HashSet<String> = [result_field.to_string()].into_iter().collect();
        FieldResolver::new(
            &aliases,
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    /// Regression for the validation-before-resolution bug: `hreflang[].lang` is aliased to
    /// `metadata.hreflangs[].lang`, which renames the ARRAY HEAD segment (`hreflang` ->
    /// `metadata.hreflangs`), not just the sub-field. Validating the raw, unresolved head
    /// (`"hreflang"`) against `is_valid_for_result` — as the pre-fix code did — checks a name
    /// absent from `result_fields` and wrongly skips the assertion even though the renamed
    /// field exists. ~keep
    #[test]
    fn alias_renaming_the_array_head_segment_still_resolves() {
        let out = render_contains(
            &resolver_with_alias("hreflang[].lang", "metadata.hreflangs[].lang", "metadata"),
            "hreflang[].lang",
            "en",
        );
        assert!(!out.contains("skipped"), "got: {out}");
        assert!(
            out.contains("result.metadata.hreflangs.any((e) => e.lang"),
            "got: {out}"
        );
    }

    /// Control for the test above: a sub-field-only rename (the array head itself,
    /// `assets`, is untouched) must keep resolving too. This is the shape the pre-fix
    /// code's own comment cited as its example, so it passed whether or not the head-rename
    /// case above was fixed — pairing it here guards against a fix that only special-cases
    /// the head. ~keep
    #[test]
    fn alias_renaming_only_the_sub_field_still_resolves() {
        let out = render_contains(
            &resolver_with_alias("assets[].category", "assets[].asset_category", "assets"),
            "assets[].category",
            "books",
        );
        assert!(!out.contains("skipped"), "got: {out}");
        assert!(out.contains("result.assets.any((e) => e.assetCategory"), "got: {out}");
    }
}

#[cfg(test)]
mod is_true_optional_field_tests {
    use super::render_assertion_dart;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn optional_resolver(field: &str) -> FieldResolver {
        let optional: HashSet<String> = [field.to_string()].into_iter().collect();
        FieldResolver::new(
            &HashMap::new(),
            &optional,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    fn render(resolver: &FieldResolver, assertion: &Assertion) -> String {
        let mut out = String::new();
        render_assertion_dart(&mut out, assertion, "result", false, resolver);
        out
    }

    fn is_true_assertion(field: &str) -> Assertion {
        Assertion {
            assertion_type: "is_true".to_string(),
            field: Some(field.to_string()),
            ..Assertion::default()
        }
    }

    /// `Option<DataNode>` (FRB v2 maps this to `DataNode?`) presence: before the fix this
    /// rendered `expect(result.data, isTrue)`, which requires the value to literally be the
    /// bool `true` -- never the case for a present `DataNode?`.
    #[test]
    fn is_true_on_optional_struct_field_checks_presence() {
        let out = render(&optional_resolver("data"), &is_true_assertion("data"));
        assert_eq!(out, "    expect(result.data, isNotNull);\n");
    }

    #[test]
    fn is_false_on_optional_struct_field_checks_absence() {
        let out = render(
            &optional_resolver("data"),
            &Assertion {
                assertion_type: "is_false".to_string(),
                field: Some("data".to_string()),
                ..Assertion::default()
            },
        );
        assert_eq!(out, "    expect(result.data, isNull);\n");
    }

    #[test]
    fn is_true_on_non_optional_field_is_unchanged() {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let out = render(&resolver, &is_true_assertion("active"));
        assert_eq!(out, "    expect(result.active, isTrue);\n");
    }
}

#[cfg(test)]
mod is_empty_branch_tests {
    use super::render_assertion_dart;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn render(resolver: &FieldResolver, field: &str) -> String {
        let assertion = Assertion {
            assertion_type: "is_empty".to_string(),
            field: Some(field.to_string()),
            ..Assertion::default()
        };
        let mut out = String::new();
        render_assertion_dart(&mut out, &assertion, "result", false, resolver);
        out
    }

    fn no_arrays_resolver() -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    /// Regression: `not_empty` branches on `is_array` so a struct-shaped field never has
    /// `.isEmpty` called on it directly (structs have no such getter -- `NoSuchMethodError`
    /// at runtime). `is_empty` had no such branch and called `.isEmpty` unconditionally via
    /// `anyOf(isNull, isEmpty)`. `document` here is not in `array_fields`, so it takes the
    /// non-collection path.
    #[test]
    fn is_empty_on_struct_field_does_not_call_isempty_directly() {
        let out = render(&no_arrays_resolver(), "document");
        assert_eq!(out, "    expect((result.document?.toString() ?? ''), isEmpty);\n");
    }

    /// Control: a field the resolver classifies as an array keeps the original
    /// `anyOf(isNull, isEmpty)` form, since `List`/`Map`/`String` all have a real
    /// `.isEmpty` getter.
    #[test]
    fn is_empty_on_array_field_keeps_anyof_isnull_isempty() {
        let array_fields: HashSet<String> = ["items".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &array_fields,
            &HashSet::new(),
        );
        let out = render(&resolver, "items");
        assert_eq!(out, "    expect(result.items, anyOf(isNull, isEmpty));\n");
    }
}

#[cfg(test)]
mod not_empty_nullability_tests {
    use super::render_assertion_dart;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn render(field: &str, optional_fields: &[&str]) -> String {
        let optional = optional_fields.iter().map(|field| (*field).to_string()).collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &optional,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let assertion = Assertion {
            assertion_type: "not_empty".to_string(),
            field: Some(field.to_string()),
            ..Assertion::default()
        };
        let mut out = String::new();
        render_assertion_dart(&mut out, &assertion, "result", false, &resolver);
        out
    }

    #[test]
    fn non_optional_scalar_does_not_emit_an_unnecessary_null_aware_call() {
        assert_eq!(
            render("chunks[0].content", &[]),
            "    expect(result.chunks[0].content.toString(), isNotEmpty);\n"
        );
    }

    #[test]
    fn optional_scalar_keeps_its_null_aware_call() {
        assert_eq!(
            render("metadata", &["metadata"]),
            "    expect(result.metadata?.toString(), isNotEmpty);\n"
        );
    }

    #[test]
    fn non_optional_leaf_after_optional_parent_does_not_add_a_second_null_aware_call() {
        assert_eq!(
            render("summary.text", &["summary"]),
            "    expect(result.summary?.text.toString(), isNotEmpty);\n"
        );
    }

    #[test]
    fn emitted_not_empty_calls_are_analyzer_clean_and_the_warning_check_is_not_vacuous() {
        if !super::dart_is_runnable() {
            return;
        }
        let assertions = [render("chunks[0].content", &[]), render("summary.text", &["summary"])].join("");
        let source = format!(
            "class Chunk {{ final String content; Chunk(this.content); }}\nclass Summary {{ final String text; Summary(this.text); }}\nclass Result {{ final List<Chunk> chunks; final Summary? summary; Result(this.chunks, this.summary); }}\nObject get isNotEmpty => Object();\nvoid expect(Object? actual, Object? matcher) {{}}\nvoid main() {{ final result = Result([Chunk('content')], Summary('summary'));\n{assertions}}}\n"
        );
        let analyze = |source: &str| {
            let temporary = tempfile::tempdir().expect("temporary Dart project");
            std::fs::write(temporary.path().join("not_empty.dart"), source).expect("write Dart source");
            std::process::Command::new("dart")
                .args(["analyze", "--fatal-infos", "--fatal-warnings", "not_empty.dart"])
                .current_dir(temporary.path())
                .status()
                .expect("run Dart analyzer")
        };
        assert!(
            analyze(&source).success(),
            "generated assertions emitted warnings:\n{source}"
        );
        let sabotaged = source.replace("content.toString()", "content?.toString()");
        assert!(
            !analyze(&sabotaged).success(),
            "analyzer accepted the unnecessary null-aware call; warning check was vacuous"
        );
    }
}

#[cfg(test)]
mod enum_wire_value_assertion_tests {
    use super::render_assertion_dart;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn enum_resolver(field: &str) -> FieldResolver {
        let names: HashSet<String> = [field.to_string()].into_iter().collect();
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_enum_fields(names)
    }

    fn optional_enum_resolver(field: &str) -> FieldResolver {
        let names: HashSet<String> = [field.to_string()].into_iter().collect();
        FieldResolver::new(
            &HashMap::new(),
            &names,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_enum_fields(names)
    }

    fn render(resolver: &FieldResolver, assertion: &Assertion) -> String {
        let mut out = String::new();
        render_assertion_dart(&mut out, assertion, "result", false, resolver);
        out
    }

    fn equals_assertion(field: &str, value: &str) -> Assertion {
        Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Assertion::default()
        }
    }

    /// Regression: an enum `equals` assertion must compare the fixture's serde wire literal
    /// VERBATIM against the binding's `.wireValue` getter. The prior `_alefE2eText` helper
    /// reconstructed a wire value from `.toString()` via an unconditional camelCase ->
    /// snake_case heuristic, so it could never reproduce a wire value with no `rename_all`
    /// (e.g. `KeyValue`, which stays PascalCase on the wire) -- it always emitted `key_value`.
    #[test]
    fn equals_on_enum_field_asserts_wire_value_verbatim() {
        let out = render(&enum_resolver("kind"), &equals_assertion("kind", "KeyValue"));
        assert_eq!(out, "    expect(result.kind.wireValue, equals('KeyValue'));\n");
    }

    /// `Option<Enum>` maps to `Enum?` in FRB Dart -- `.wireValue` needs safe navigation.
    #[test]
    fn equals_on_optional_enum_field_uses_safe_navigation() {
        let out = render(&optional_enum_resolver("kind"), &equals_assertion("kind", "KeyValue"));
        assert_eq!(out, "    expect(result.kind?.wireValue, equals('KeyValue'));\n");
    }

    #[test]
    fn not_equals_on_enum_field_asserts_wire_value_verbatim() {
        let assertion = Assertion {
            assertion_type: "not_equals".to_string(),
            field: Some("kind".to_string()),
            value: Some(serde_json::Value::String("Sequence".to_string())),
            ..Assertion::default()
        };
        let out = render(&enum_resolver("kind"), &assertion);
        assert_eq!(out, "    expect(result.kind.wireValue, isNot(equals('Sequence')));\n");
    }
}

#[cfg(test)]
mod tagged_union_assertion_tests {
    use super::render_assertion_dart;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        let optional = matches!(ty, TypeRef::Optional(_));
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            ..FieldDef::default()
        }
    }

    fn resolver() -> FieldResolver {
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
                fields: vec![
                    field(
                        "format",
                        TypeRef::Optional(Box::new(TypeRef::Named("FormatMetadata".to_string()))),
                    ),
                    field("direct", TypeRef::Named("DirectOuter".to_string())),
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "HtmlMetadata".to_string(),
                fields: vec![
                    field("title", TypeRef::String),
                    field(
                        "headers",
                        TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
                    ),
                    field("detail", TypeRef::Named("DetailUnion".to_string())),
                    field(
                        "details",
                        TypeRef::Optional(Box::new(TypeRef::Named("Details".to_string()))),
                    ),
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Details".to_string(),
                fields: vec![field("kind", TypeRef::Named("DetailUnion".to_string()))],
                ..TypeDef::default()
            },
            TypeDef {
                name: "StatsMetadata".to_string(),
                fields: vec![field("count", TypeRef::Primitive(crate::core::ir::PrimitiveType::U32))],
                ..TypeDef::default()
            },
        ];
        let enums = vec![
            EnumDef {
                name: "FormatMetadata".to_string(),
                variants: vec![EnumVariant {
                    name: "Html".to_string(),
                    fields: vec![field("_0", TypeRef::Named("HtmlMetadata".to_string()))],
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            },
            EnumDef {
                name: "DetailUnion".to_string(),
                variants: vec![EnumVariant {
                    name: "Stats".to_string(),
                    fields: vec![field("stats_payload", TypeRef::Named("StatsMetadata".to_string()))],
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            },
            EnumDef {
                name: "DirectOuter".to_string(),
                variants: vec![EnumVariant {
                    name: "Wrapped".to_string(),
                    fields: vec![field("_0", TypeRef::Named("DirectInner".to_string()))],
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            },
            EnumDef {
                name: "DirectInner".to_string(),
                variants: vec![EnumVariant {
                    name: "Value".to_string(),
                    fields: vec![field("_0", TypeRef::Named("StatsMetadata".to_string()))],
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            },
        ];
        let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&types);
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_result_fields(
            FieldResolver::ir_result_field_facts(&types, "dart"),
            Some("ExtractionResult".to_string()),
        )
        .with_ir_enum_map(
            FieldResolver::ir_enum_fields(&types, &enums),
            Some("ExtractionResult".to_string()),
        )
        .with_ir_collection_map(
            FieldResolver::ir_collection_fields(&types),
            Some("ExtractionResult".to_string()),
        )
        .with_ir_fields(reachable, excluded, optional)
    }

    fn render(assertion: Assertion) -> String {
        let mut out = String::new();
        render_assertion_dart(&mut out, &assertion, "result", false, &resolver());
        out
    }

    #[test]
    fn equals_narrows_a_freezed_union_before_reading_its_payload() {
        let out = render(Assertion {
            assertion_type: "equals".to_string(),
            field: Some("results[0].metadata.format.html.title".to_string()),
            value: Some(serde_json::json!("Simple Table Test")),
            ..Assertion::default()
        });
        assert_eq!(
            out,
            "    expect((result.results[0].metadata.format as FormatMetadata_Html).field0.title.toString(), equals('Simple Table Test'.toString()));\n"
        );
    }

    #[test]
    fn count_min_narrows_a_freezed_union_and_counts_the_payload_collection() {
        let out = render(Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("results[0].metadata.format.html.headers".to_string()),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        });
        assert_eq!(
            out,
            "    expect((result.results[0].metadata.format as FormatMetadata_Html).field0.headers?.length ?? 0, greaterThanOrEqualTo(2));\n"
        );
    }

    #[test]
    fn equals_narrows_each_freezed_union_in_a_two_crossing_path() {
        let out = render(Assertion {
            assertion_type: "equals".to_string(),
            field: Some("results[0].metadata.format.html.detail.stats.count".to_string()),
            value: Some(serde_json::json!(3)),
            ..Assertion::default()
        });
        assert_eq!(
            out,
            "    expect(((result.results[0].metadata.format as FormatMetadata_Html).field0.detail as DetailUnion_Stats).statsPayload.count, equals(3));\n"
        );
    }

    #[test]
    fn nested_union_access_uses_null_navigation_for_an_optional_intermediate_struct() {
        let out = render(Assertion {
            assertion_type: "equals".to_string(),
            field: Some("results[0].metadata.format.html.details.kind.stats.count".to_string()),
            value: Some(serde_json::json!(3)),
            ..Assertion::default()
        });
        assert_eq!(
            out,
            "    expect(((result.results[0].metadata.format as FormatMetadata_Html).field0.details?.kind as DetailUnion_Stats).statsPayload.count, equals(3));\n"
        );
    }

    #[test]
    fn directly_nested_union_payload_narrows_the_inner_union() {
        let out = render(Assertion {
            assertion_type: "equals".to_string(),
            field: Some("results[0].metadata.direct.wrapped.value.count".to_string()),
            value: Some(serde_json::json!(3)),
            ..Assertion::default()
        });
        assert_eq!(
            out,
            "    expect(((result.results[0].metadata.direct as DirectOuter_Wrapped).field0 as DirectInner_Value).field0.count, equals(3));\n"
        );
    }

    /// ~keep The old renderer only recognized consumer-configured method-call crossings. This
    /// control keeps the fixture deliberately free of method-call metadata so the IR must fire.
    #[test]
    fn tagged_union_rendering_does_not_require_fields_method_calls_configuration() {
        let out = render(Assertion {
            assertion_type: "equals".to_string(),
            field: Some("results[0].metadata.format.html.title".to_string()),
            value: Some(serde_json::json!("title")),
            ..Assertion::default()
        });
        assert!(
            !out.contains(".format.html."),
            "flat sealed-class access survived: {out}"
        );
        assert!(!out.contains("skipped"), "the meaningful assertion was skipped: {out}");
    }

    fn dart_analyze(source: &str) -> std::process::ExitStatus {
        let temporary = tempfile::tempdir().expect("temporary Dart project");
        let source_path = temporary.path().join("union_assertion.dart");
        std::fs::write(&source_path, source).expect("write Dart source");
        std::process::Command::new("dart")
            .args(["analyze", "union_assertion.dart"])
            .current_dir(temporary.path())
            .status()
            .expect("run Dart analyzer")
    }

    /// ~keep Compile the emitted expression against the actual Freezed concrete-subclass shape,
    /// then sabotage its subtype name to prove the analyzer check observes this assertion.
    #[test]
    fn emitted_freezed_union_assertions_compile_and_the_type_check_is_not_vacuous() {
        if !super::dart_is_runnable() {
            return;
        }
        let assertions = [
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("results[0].metadata.format.html.title".to_string()),
                value: Some(serde_json::json!("Simple Table Test")),
                ..Assertion::default()
            },
            Assertion {
                assertion_type: "count_min".to_string(),
                field: Some("results[0].metadata.format.html.headers".to_string()),
                value: Some(serde_json::json!(2)),
                ..Assertion::default()
            },
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("results[0].metadata.format.html.detail.stats.count".to_string()),
                value: Some(serde_json::json!(3)),
                ..Assertion::default()
            },
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("results[0].metadata.format.html.details.kind.stats.count".to_string()),
                value: Some(serde_json::json!(3)),
                ..Assertion::default()
            },
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("results[0].metadata.direct.wrapped.value.count".to_string()),
                value: Some(serde_json::json!(3)),
                ..Assertion::default()
            },
        ]
        .into_iter()
        .map(render)
        .collect::<String>();
        let source = format!(
            "class FormatMetadata {{}}\nclass FormatMetadata_Html extends FormatMetadata {{ final HtmlMetadata field0; FormatMetadata_Html(this.field0); }}\nclass DetailUnion {{}}\nclass DetailUnion_Stats extends DetailUnion {{ final StatsMetadata statsPayload; DetailUnion_Stats(this.statsPayload); }}\nclass DirectOuter {{}}\nclass DirectOuter_Wrapped extends DirectOuter {{ final DirectInner field0; DirectOuter_Wrapped(this.field0); }}\nclass DirectInner {{}}\nclass DirectInner_Value extends DirectInner {{ final StatsMetadata field0; DirectInner_Value(this.field0); }}\nclass StatsMetadata {{ final int count; StatsMetadata(this.count); }}\nclass Details {{ final DetailUnion kind; Details(this.kind); }}\nclass HtmlMetadata {{ final String title; final List<String>? headers; final DetailUnion detail; final Details? details; HtmlMetadata(this.title, this.headers, this.detail, this.details); }}\nclass Metadata {{ final FormatMetadata? format; final DirectOuter direct; Metadata(this.format, this.direct); }}\nclass Document {{ final Metadata metadata; Document(this.metadata); }}\nclass Result {{ final List<Document> results; Result(this.results); }}\nObject equals(Object? value) => value!;\nObject greaterThanOrEqualTo(Object? value) => value!;\nvoid expect(Object? actual, Object? matcher) {{}}\nvoid main() {{ final stats = StatsMetadata(3); final result = Result([Document(Metadata(FormatMetadata_Html(HtmlMetadata('Simple Table Test', ['a', 'b'], DetailUnion_Stats(stats), Details(DetailUnion_Stats(stats)))), DirectOuter_Wrapped(DirectInner_Value(stats))))]);\n{assertions}}}\n"
        );
        assert!(
            dart_analyze(&source).success(),
            "generated union assertion did not analyze:\n{source}"
        );
        let numeric_payload = source.replace("FormatMetadata_Html).field0.title", "FormatMetadata_Html).0.title");
        assert!(
            !dart_analyze(&numeric_payload).success(),
            "Dart analyzer accepted a numeric tuple payload accessor; tuple naming check was vacuous"
        );
        let sabotaged = source.replace("DirectInner_Value).field0", "MissingVariant).field0");
        assert!(
            !dart_analyze(&sabotaged).success(),
            "Dart analyzer accepted a nonexistent union subtype; compile check was vacuous"
        );
        let unsafe_optional = source.replace("details?.kind", "details.kind");
        assert!(
            !dart_analyze(&unsafe_optional).success(),
            "Dart analyzer accepted an unguarded optional intermediate; nullability check was vacuous"
        );
    }
}
