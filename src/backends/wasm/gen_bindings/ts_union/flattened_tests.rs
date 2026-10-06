//! WASM TS-union optional, nested and flattened-enum coverage, split from tests.rs.

use super::tests::*;
use super::*;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};
use ahash::AHashSet;

/// A field marked `optional` (bool flag, not wrapped in `TypeRef::Optional`) gets `| undefined`
/// appended once, matching the convention used elsewhere for `Option<T>` fields.
#[test]
fn optional_flag_field_appends_undefined_once() {
    let enum_def = EnumDef {
        name: "WithOptionalField".to_string(),
        rust_path: "test_lib::WithOptionalField".to_string(),
        variants: vec![tuple_variant("Data", TypeRef::Named("HasOptional".to_string()))],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "HasOptional".to_string(),
        rust_path: "test_lib::HasOptional".to_string(),
        fields: vec![FieldDef {
            name: "maybe".to_string(),
            ty: TypeRef::String,
            optional: true,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let plan = plan_for(&enum_def, &api);
    assert!(
        plan.contains("export interface AlefHasOptionalWire {\n    maybe: string | undefined;\n}"),
        "actual:\n{}",
        plan
    );
}

/// Two variants referencing the same struct type must emit exactly one interface.
#[test]
fn shared_struct_reference_is_emitted_once() {
    let enum_def = EnumDef {
        name: "SharedRef".to_string(),
        rust_path: "test_lib::SharedRef".to_string(),
        variants: vec![
            tuple_variant("First", TypeRef::Named("Shared".to_string())),
            tuple_variant("Second", TypeRef::Vec(Box::new(TypeRef::Named("Shared".to_string())))),
        ],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "Shared".to_string(),
        rust_path: "test_lib::Shared".to_string(),
        fields: vec![FieldDef {
            name: "id".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let plan = plan_for(&enum_def, &api);
    assert!(
        plan.contains("export type AlefSharedRef = AlefSharedWire | AlefSharedWire[];"),
        "actual:\n{}",
        plan
    );
    assert_eq!(
        plan.matches("export interface AlefSharedWire").count(),
        1,
        "actual:\n{}",
        plan
    );
}

/// Two different top-level untagged unions that both carry the same fieldless enum in a variant
/// (e.g. two request shapes that both accept a `Mode`) must not each independently emit their
/// own `AlefModeWire` alias — `tsc` rejects two `type` declarations of the same name even when
/// byte-identical (`TS2300: Duplicate identifier`), unlike `interface`, which merges. This is
/// exactly why `build_untagged_enum_ts_plans` shares one `TsMapContext` (and therefore one
/// dedup registry) across every enum instead of building each union's plan independently.
#[test]
fn fieldless_enum_shared_by_two_unions_is_declared_once() {
    let mode = EnumDef {
        name: "Mode".to_string(),
        rust_path: "test_lib::Mode".to_string(),
        variants: vec![
            EnumVariant {
                name: "Fast".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Slow".to_string(),
                ..Default::default()
            },
        ],
        serde_rename_all: Some("snake_case".to_string()),
        is_copy: true,
        ..Default::default()
    };
    let first = EnumDef {
        name: "FirstChoice".to_string(),
        rust_path: "test_lib::FirstChoice".to_string(),
        variants: vec![tuple_variant("Mode", TypeRef::Named("Mode".to_string()))],
        serde_untagged: true,
        ..Default::default()
    };
    let second = EnumDef {
        name: "SecondChoice".to_string(),
        rust_path: "test_lib::SecondChoice".to_string(),
        variants: vec![tuple_variant("Mode", TypeRef::Named("Mode".to_string()))],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.enums = vec![mode];
    let all_plans = build_untagged_enum_ts_plans(
        &[&first, &second],
        &api,
        &AHashSet::default(),
        &AHashSet::default(),
        "Alef",
    );

    assert_eq!(
        all_plans.custom_section.matches("export type AlefModeWire").count(),
        1,
        "the shared alias must be declared exactly once across both unions;\nactual:\n{}",
        all_plans.custom_section
    );
    assert!(
        all_plans
            .custom_section
            .contains("export type AlefFirstChoice = AlefModeWire;"),
        "actual:\n{}",
        all_plans.custom_section
    );
    assert!(
        all_plans
            .custom_section
            .contains("export type AlefSecondChoice = AlefModeWire;"),
        "actual:\n{}",
        all_plans.custom_section
    );
}

/// A struct shared by two different top-level unions is deduped exactly like the fieldless-enum
/// case above (the shared `TsMapContext` registry spans every enum in one
/// `build_untagged_enum_ts_plans` call) — even though, unlike a `type` alias, TypeScript would
/// have tolerated a duplicate `interface` here (confirmed with `tsc`: identical `interface`
/// declarations merge). Both unions must still reference the single declaration correctly.
#[test]
fn struct_shared_by_two_unions_is_referenced_by_both() {
    let shared = TypeDef {
        name: "Shared".to_string(),
        rust_path: "test_lib::Shared".to_string(),
        fields: vec![FieldDef {
            name: "id".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let first = EnumDef {
        name: "FirstHolder".to_string(),
        rust_path: "test_lib::FirstHolder".to_string(),
        variants: vec![tuple_variant("Item", TypeRef::Named("Shared".to_string()))],
        serde_untagged: true,
        ..Default::default()
    };
    let second = EnumDef {
        name: "SecondHolder".to_string(),
        rust_path: "test_lib::SecondHolder".to_string(),
        variants: vec![tuple_variant("Item", TypeRef::Named("Shared".to_string()))],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![shared];
    let all_plans = build_untagged_enum_ts_plans(
        &[&first, &second],
        &api,
        &AHashSet::default(),
        &AHashSet::default(),
        "Alef",
    );

    assert!(
        all_plans
            .custom_section
            .contains("export type AlefFirstHolder = AlefSharedWire;"),
        "actual:\n{}",
        all_plans.custom_section
    );
    assert!(
        all_plans
            .custom_section
            .contains("export type AlefSecondHolder = AlefSharedWire;"),
        "actual:\n{}",
        all_plans.custom_section
    );
    assert_eq!(
        all_plans
            .custom_section
            .matches("export interface AlefSharedWire")
            .count(),
        1,
        "actual:\n{}",
        all_plans.custom_section
    );
}

/// A top-level untagged enum whose only reference in `untagged_enums` comes from a SIBLING
/// union's variant (never appearing bare/unreferenced elsewhere) must still get its own `type`
/// alias declared — the nested reference must not mark it "already resolved" without actually
/// queuing its declaration, or its `typescript_type = "AlefNested"` extern type would point at a
/// name nothing ever declares.
#[test]
fn nested_reference_to_a_sibling_top_level_union_still_declares_it() {
    let nested = EnumDef {
        name: "Nested".to_string(),
        rust_path: "test_lib::Nested".to_string(),
        variants: vec![tuple_variant("Value", TypeRef::String)],
        serde_untagged: true,
        ..Default::default()
    };
    let outer = EnumDef {
        name: "Outer".to_string(),
        rust_path: "test_lib::Outer".to_string(),
        variants: vec![tuple_variant("Inner", TypeRef::Named("Nested".to_string()))],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.enums = vec![nested.clone()];
    // `outer` first so its variant reaches `Nested` before `Nested`'s own top-level turn runs —
    // the ordering that triggers the bug this test guards against.
    let all_plans = build_untagged_enum_ts_plans(
        &[&outer, &nested],
        &api,
        &AHashSet::default(),
        &AHashSet::default(),
        "Alef",
    );

    assert!(
        all_plans.custom_section.contains("export type AlefOuter = AlefNested;"),
        "actual:\n{}",
        all_plans.custom_section
    );
    assert!(
        all_plans.custom_section.contains("export type AlefNested = string;"),
        "Nested must still be declared even though it was only ever reached as a nested \
         reference before its own top-level turn ran;\nactual:\n{}",
        all_plans.custom_section
    );
}

/// Two top-level untagged enums that reference each other (directly mutually recursive) must
/// both still get declared, and must terminate rather than looping.
#[test]
fn mutually_recursive_top_level_unions_both_get_declared() {
    let a = EnumDef {
        name: "MutualA".to_string(),
        rust_path: "test_lib::MutualA".to_string(),
        variants: vec![
            tuple_variant("Leaf", TypeRef::String),
            tuple_variant("Ref", TypeRef::Named("MutualB".to_string())),
        ],
        serde_untagged: true,
        ..Default::default()
    };
    let b = EnumDef {
        name: "MutualB".to_string(),
        rust_path: "test_lib::MutualB".to_string(),
        variants: vec![
            tuple_variant("Leaf", TypeRef::String),
            tuple_variant("Ref", TypeRef::Named("MutualA".to_string())),
        ],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.enums = vec![a.clone(), b.clone()];
    let all_plans = build_untagged_enum_ts_plans(&[&a, &b], &api, &AHashSet::default(), &AHashSet::default(), "Alef");

    assert!(
        all_plans
            .custom_section
            .contains("export type AlefMutualA = string | AlefMutualB;"),
        "actual:\n{}",
        all_plans.custom_section
    );
    assert!(
        all_plans
            .custom_section
            .contains("export type AlefMutualB = string | AlefMutualA;"),
        "actual:\n{}",
        all_plans.custom_section
    );
}

// ---------------------------------------------------------------------------------------------
// Variant-level `#[serde(untagged)]` (mixed with default-tagged unit variants)
// ---------------------------------------------------------------------------------------------

/// `enum OutputFormat { Plain, Markdown, ..., #[serde(untagged)] Custom(String) }` -- a
/// default-tagged enum whose only data-carrying variant opts out via its own
/// `#[serde(untagged)]` -- gets the flat literal-union-plus-widening-tail shape from
/// [`build_variant_untagged_string_enum_ts_plans`], NOT the "unit variant renders as `null`"
/// shape [`build_untagged_enum_ts_plans`] produces for a container-level `#[serde(untagged)]`
/// enum (see `embedding_input_maps_to_string_or_string_array` above for that shape). ~keep
#[test]
fn variant_untagged_output_format_maps_to_literal_union_with_string_tail() {
    let enum_def = EnumDef {
        name: "OutputFormat".to_string(),
        rust_path: "test_lib::OutputFormat".to_string(),
        variants: vec![
            EnumVariant {
                name: "Plain".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Markdown".to_string(),
                ..Default::default()
            },
            EnumVariant {
                serde_untagged: true,
                ..tuple_variant("Custom", TypeRef::String)
            },
        ],
        serde_rename_all: Some("lowercase".to_string()),
        ..Default::default()
    };
    let api = empty_api();
    let all_plans = build_variant_untagged_string_enum_ts_plans(
        &[&enum_def],
        &api,
        &AHashSet::default(),
        &AHashSet::default(),
        "Alef",
    );

    assert!(
        all_plans
            .custom_section
            .contains(r#"export type AlefOutputFormat = "plain" | "markdown" | (string & {});"#),
        "actual:\n{}",
        all_plans.custom_section
    );
    let plan = all_plans.plans.get("OutputFormat").expect("plan for OutputFormat");
    assert!(
        plan.extern_type_declaration
            .contains(r#"typescript_type = "AlefOutputFormat""#)
    );
}

/// A unit-only enum has no data-carrying variant at all, so `gets_a_variant_untagged_string_enum_ts_union`
/// must not claim it via the `_for_api` entry point that the real pipeline calls — it stays a
/// plain `#[wasm_bindgen]` C-style enum via `gen_enum`.
#[test]
fn unit_only_enum_gets_no_variant_untagged_plan() {
    let enum_def = EnumDef {
        name: "Status".to_string(),
        rust_path: "test_lib::Status".to_string(),
        variants: vec![
            EnumVariant {
                name: "Active".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Inactive".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    assert!(!super::super::enums::is_variant_untagged_string_enum(&enum_def));
    let mut api = empty_api();
    api.enums = vec![enum_def];
    let all_plans = build_variant_untagged_string_enum_ts_plan_for_api(
        &api,
        &[],
        &AHashSet::default(),
        &AHashSet::default(),
        "Alef",
    );
    assert!(all_plans.plans.is_empty());
    assert!(all_plans.custom_section.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Fully-flattened internally-tagged enums (`is_fully_flattened_internal_enum`)
// ---------------------------------------------------------------------------------------------

/// `#[serde(tag = "format_type")] enum FormatMetadata { Excel(ExcelMetadata), Pdf }` -> each
/// data variant intersects the tag literal with the payload's own structural shape, and a unit
/// variant is just the tag literal.
#[test]
fn flattened_internal_enum_intersects_tag_literal_with_payload_shape() {
    let enum_def = EnumDef {
        name: "FormatMetadata".to_string(),
        rust_path: "test_lib::FormatMetadata".to_string(),
        serde_tag: Some("format_type".to_string()),
        variants: vec![
            tuple_variant("Excel", TypeRef::Named("ExcelMetadata".to_string())),
            EnumVariant {
                name: "Pdf".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "ExcelMetadata".to_string(),
        rust_path: "test_lib::ExcelMetadata".to_string(),
        fields: vec![FieldDef {
            name: "sheet_count".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            ..Default::default()
        }],
        ..Default::default()
    }];
    api.enums = vec![enum_def.clone()];
    assert!(super::super::enums::is_fully_flattened_internal_enum(
        &enum_def, &api.types
    ));

    let all_plans =
        build_flattened_internal_enum_ts_plan_for_api(&api, &[], &AHashSet::default(), &AHashSet::default(), "Alef");
    // The tag key is camelCased (`formatType`, not `format_type`) and the payload interface gets
    // the `CamelWire` suffix, not the plain `Wire` suffix a `Serde`-cased plan would use -- both
    // match the runtime `serde_wasm_bindgen` recasing `ConversionConfig::wasm_camel_recased_enums`
    // now applies for exactly this shape. See `flattened_internal_enum_tag_key_uses_the_camel_case_wire_name`
    // and `flattened_internal_enum_payload_fields_use_camel_case_wire_names_including_nested_structs`
    // below for the dedicated regression coverage of each half.
    assert!(
        all_plans.custom_section.contains(concat!(
            "export type AlefFormatMetadata = ",
            "({ formatType: \"Excel\" } & AlefExcelMetadataCamelWire) | { formatType: \"Pdf\" };"
        )),
        "actual:\n{}",
        all_plans.custom_section
    );
    assert!(
        all_plans
            .custom_section
            .contains("interface AlefExcelMetadataCamelWire"),
        "actual:\n{}",
        all_plans.custom_section
    );
    let plan = all_plans.plans.get("FormatMetadata").expect("plan for FormatMetadata");
    assert!(
        plan.extern_type_declaration
            .contains(r#"typescript_type = "AlefFormatMetadata""#),
        "actual:\n{}",
        plan.extern_type_declaration
    );
}

/// A mixed enum — some variants flattened newtypes, some struct-field variants with real field
/// names — does not qualify: only `is_fully_flattened_internal_enum` (ALL data variants
/// flattened) routes through the JsValue bridge; a mixed enum keeps its nominal
/// `gen_tagged_enum_as_struct` type instead, so it must not appear in this plan.
#[test]
fn mixed_enum_is_not_a_flattened_internal_enum_and_gets_no_plan() {
    let enum_def = EnumDef {
        name: "AuthConfig".to_string(),
        rust_path: "test_lib::AuthConfig".to_string(),
        serde_tag: Some("type".to_string()),
        variants: vec![
            tuple_variant("Bearer", TypeRef::Named("BearerToken".to_string())),
            EnumVariant {
                name: "Basic".to_string(),
                fields: vec![
                    FieldDef {
                        name: "username".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    },
                    FieldDef {
                        name: "password".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    },
                ],
                is_tuple: false,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    assert!(!super::super::enums::is_fully_flattened_internal_enum(&enum_def, &[]));

    let mut api = empty_api();
    api.enums = vec![enum_def];
    let all_plans =
        build_flattened_internal_enum_ts_plan_for_api(&api, &[], &AHashSet::default(), &AHashSet::default(), "Alef");
    assert!(all_plans.plans.is_empty());
    assert!(all_plans.custom_section.is_empty());
}

/// Adjacent (`tag`+`content`) tagging never flattens — the payload keeps a real key under
/// `content` — so it must not be claimed by `is_fully_flattened_internal_enum` even though its
/// single variant is otherwise newtype-shaped exactly like the internal case above.
#[test]
fn adjacent_tagging_is_not_a_flattened_internal_enum() {
    let enum_def = EnumDef {
        name: "DiffLine".to_string(),
        rust_path: "test_lib::DiffLine".to_string(),
        serde_tag: Some("kind".to_string()),
        serde_content: Some("text".to_string()),
        variants: vec![tuple_variant("Added", TypeRef::String)],
        ..Default::default()
    };
    assert!(!super::super::enums::is_fully_flattened_internal_enum(&enum_def, &[]));
}

/// The runtime conversion for a flattened-internal-tagged enum field now re-cases through
/// `codegen::json_wire_types` + `ConversionConfig::wasm_camel_recased_enums`
/// (`codegen::conversions::{core_to_binding,binding_to_core}::fields`'s `camel_jsvalue`/
/// `camel_core_value` helpers) instead of bridging `serde_wasm_bindgen` straight against the CORE
/// type, so the declaration must be camelCase too -- including NESTED payloads:
/// `ExcelMetadata.sheet_count` must declare as `sheetCount`, and a struct it embeds
/// (`SheetInfo.row_count`) must ALSO be camelCased, two levels deep -- proving the declaration
/// matches the runtime shape at every depth, not only at the top level.
#[test]
fn flattened_internal_enum_payload_fields_use_camel_case_wire_names_including_nested_structs() {
    let enum_def = EnumDef {
        name: "FormatMetadata".to_string(),
        rust_path: "test_lib::FormatMetadata".to_string(),
        serde_tag: Some("format_type".to_string()),
        variants: vec![tuple_variant("Excel", TypeRef::Named("ExcelMetadata".to_string()))],
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![
        TypeDef {
            name: "ExcelMetadata".to_string(),
            rust_path: "test_lib::ExcelMetadata".to_string(),
            fields: vec![
                FieldDef {
                    name: "sheet_count".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    ..Default::default()
                },
                FieldDef {
                    name: "primary_sheet".to_string(),
                    ty: TypeRef::Named("SheetInfo".to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        TypeDef {
            name: "SheetInfo".to_string(),
            rust_path: "test_lib::SheetInfo".to_string(),
            fields: vec![FieldDef {
                name: "row_count".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            }],
            ..Default::default()
        },
    ];
    api.enums = vec![enum_def.clone()];

    let all_plans =
        build_flattened_internal_enum_ts_plan_for_api(&api, &[], &AHashSet::default(), &AHashSet::default(), "Alef");

    assert!(
        all_plans.custom_section.contains("sheetCount: number;"),
        "top-level payload field must declare the camelCase wire name, actual:\n{}",
        all_plans.custom_section
    );
    assert!(
        !all_plans.custom_section.contains("sheet_count:"),
        "must not declare the snake_case name the runtime no longer produces, actual:\n{}",
        all_plans.custom_section
    );
    assert!(
        all_plans
            .custom_section
            .contains("primarySheet: AlefSheetInfoCamelWire"),
        "the field naming the nested struct must itself use the camelCase wire name and reference \
         the CamelWire-suffixed interface, actual:\n{}",
        all_plans.custom_section
    );
    assert!(
        all_plans.custom_section.contains("rowCount: number;"),
        "a field NESTED inside the referenced struct must also be camelCased, actual:\n{}",
        all_plans.custom_section
    );
}

/// An explicit `#[serde(rename = "...")]` is author intent and must be emitted VERBATIM, never
/// recased.
#[test]
fn flattened_internal_enum_payload_field_with_explicit_rename_is_kept_verbatim() {
    let enum_def = EnumDef {
        name: "FormatMetadata".to_string(),
        rust_path: "test_lib::FormatMetadata".to_string(),
        serde_tag: Some("format_type".to_string()),
        variants: vec![tuple_variant("Excel", TypeRef::Named("ExcelMetadata".to_string()))],
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "ExcelMetadata".to_string(),
        rust_path: "test_lib::ExcelMetadata".to_string(),
        fields: vec![FieldDef {
            name: "internal_name".to_string(),
            ty: TypeRef::String,
            serde_rename: Some("external_name".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }];
    api.enums = vec![enum_def.clone()];

    let all_plans =
        build_flattened_internal_enum_ts_plan_for_api(&api, &[], &AHashSet::default(), &AHashSet::default(), "Alef");

    assert!(
        all_plans.custom_section.contains("external_name: string;"),
        "an explicit serde rename must be emitted verbatim, actual:\n{}",
        all_plans.custom_section
    );
    assert!(
        !all_plans.custom_section.contains("internalName") && !all_plans.custom_section.contains("externalName"),
        "must not recase an explicit rename in either direction, actual:\n{}",
        all_plans.custom_section
    );
}

/// The discriminant KEY is a field name on this boundary same as any payload field, so it now
/// gets the same camelCase treatment (`formatType`, not `format_type`) as every payload field
/// around it -- a camelCase payload with a snake_case discriminant would be exactly the kind of
/// inconsistency this declaration exists to avoid, per `build_flattened_internal_enum_ts_plans`'s
/// own `~keep` note. The discriminant VALUE (`"Pdf"`) is data, not an identifier, and stays
/// untouched either way.
///
/// The fixture needs a genuine data-carrying variant (`Excel`) alongside the unit variant: with
/// only unit variants, `is_fully_flattened_internal_enum` has no data variant to check and
/// answers `false`, so `gets_a_flattened_internal_enum_ts_union` drops the enum before it ever
/// reaches this builder and `custom_section` comes back empty -- silently vacuous, not a real
/// assertion of the tag key's casing.
#[test]
fn flattened_internal_enum_tag_key_uses_the_camel_case_wire_name() {
    let enum_def = EnumDef {
        name: "FormatMetadata".to_string(),
        rust_path: "test_lib::FormatMetadata".to_string(),
        serde_tag: Some("format_type".to_string()),
        variants: vec![
            tuple_variant("Excel", TypeRef::Named("ExcelMetadata".to_string())),
            EnumVariant {
                name: "Pdf".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "ExcelMetadata".to_string(),
        rust_path: "test_lib::ExcelMetadata".to_string(),
        fields: vec![FieldDef {
            name: "sheet_count".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            ..Default::default()
        }],
        ..Default::default()
    }];
    api.enums = vec![enum_def.clone()];
    assert!(
        super::super::enums::is_fully_flattened_internal_enum(&enum_def, &api.types),
        "fixture must actually be a flattened internal enum, or this test checks nothing"
    );

    let all_plans =
        build_flattened_internal_enum_ts_plan_for_api(&api, &[], &AHashSet::default(), &AHashSet::default(), "Alef");
    assert!(
        !all_plans.custom_section.is_empty(),
        "the fixture must produce a union declaration"
    );

    assert!(
        all_plans.custom_section.contains("{ formatType: \"Pdf\" }"),
        "the discriminant key must declare the camelCase wire name `formatType`, actual:\n{}",
        all_plans.custom_section
    );
    assert!(
        !all_plans.custom_section.contains("format_type:"),
        "must not declare the snake_case discriminant key the runtime no longer produces, actual:\n{}",
        all_plans.custom_section
    );
}
