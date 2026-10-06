//! Table-driven coverage for the structural TS mapper in `ts_union.rs`.
//!
//! Each shape here mirrors a real consumer type (see the module doc comment on `ts_union.rs`):
//! `EmbeddingInput`/`ModerationInput` (scalar-or-vec), `RerankDocument` (newtype-or-struct),
//! `UserContent`/`AssistantContent` (scalar-or-vec-of-struct, forcing an interface), and
//! `ToolChoice` (newtype-of-struct plus newtype-of-unit-enum, forcing a string-literal union).

use super::*;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

pub(super) fn empty_api() -> ApiSurface {
    ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::BTreeMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

pub(super) fn tuple_variant(name: &str, ty: TypeRef) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields: vec![FieldDef {
            name: "_0".to_string(),
            ty,
            ..Default::default()
        }],
        is_tuple: true,
        ..Default::default()
    }
}

/// Builds the plan for a single enum and returns its shared `custom_section` text — the
/// equivalent of what used to be the per-enum `declaration` field before untagged enums started
/// sharing one combined custom section (see `AllUntaggedEnumsTsPlan`).
pub(super) fn plan_for(enum_def: &EnumDef, api: &ApiSurface) -> String {
    let exclude_types = AHashSet::default();
    let opaque_types = AHashSet::default();
    build_untagged_enum_ts_plans(&[enum_def], api, &exclude_types, &opaque_types, "Alef").custom_section
}

// ---------------------------------------------------------------------------------------------
// Real consumer shapes
// ---------------------------------------------------------------------------------------------

/// `enum EmbeddingInput { Single(String), Multiple(Vec<String>) }` -> `string | string[]`.
#[test]
fn embedding_input_maps_to_string_or_string_array() {
    let enum_def = EnumDef {
        name: "EmbeddingInput".to_string(),
        rust_path: "test_lib::EmbeddingInput".to_string(),
        variants: vec![
            tuple_variant("Single", TypeRef::String),
            tuple_variant("Multiple", TypeRef::Vec(Box::new(TypeRef::String))),
        ],
        serde_untagged: true,
        ..Default::default()
    };
    let api = empty_api();
    let all_plans =
        build_untagged_enum_ts_plans(&[&enum_def], &api, &AHashSet::default(), &AHashSet::default(), "Alef");
    assert!(
        all_plans
            .custom_section
            .contains("export type AlefEmbeddingInput = string | string[];"),
        "actual:\n{}",
        all_plans.custom_section
    );
    let plan = all_plans.plans.get("EmbeddingInput").expect("plan for EmbeddingInput");
    assert!(
        plan.extern_type_declaration
            .contains(r#"typescript_type = "AlefEmbeddingInput""#)
    );
    assert!(
        plan.extern_type_declaration
            .contains("pub type AlefEmbeddingInputValue;")
    );
}

/// `ModerationInput` has the identical scalar-or-vec shape as `EmbeddingInput`.
#[test]
fn moderation_input_maps_to_string_or_string_array() {
    let enum_def = EnumDef {
        name: "ModerationInput".to_string(),
        rust_path: "test_lib::ModerationInput".to_string(),
        variants: vec![
            tuple_variant("Single", TypeRef::String),
            tuple_variant("Multiple", TypeRef::Vec(Box::new(TypeRef::String))),
        ],
        serde_untagged: true,
        ..Default::default()
    };
    let plan = plan_for(&enum_def, &empty_api());
    assert!(
        plan.contains("export type AlefModerationInput = string | string[];"),
        "actual:\n{}",
        plan
    );
}

/// `enum RerankDocument { Text(String), Object { text: String } }` ->
/// `string | { text: string }` — a newtype variant next to a struct variant.
#[test]
fn rerank_document_maps_newtype_and_struct_variant() {
    let enum_def = EnumDef {
        name: "RerankDocument".to_string(),
        rust_path: "test_lib::RerankDocument".to_string(),
        variants: vec![
            tuple_variant("Text", TypeRef::String),
            EnumVariant {
                name: "Object".to_string(),
                fields: vec![FieldDef {
                    name: "text".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                is_tuple: false,
                ..Default::default()
            },
        ],
        serde_untagged: true,
        ..Default::default()
    };
    let plan = plan_for(&enum_def, &empty_api());
    assert!(
        plan.contains("export type AlefRerankDocument = string | { text: string; };"),
        "actual:\n{}",
        plan
    );
}

pub(super) fn content_part_type() -> TypeDef {
    TypeDef {
        name: "ContentPart".to_string(),
        rust_path: "test_lib::ContentPart".to_string(),
        fields: vec![
            FieldDef {
                name: "text".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            },
            FieldDef {
                name: "kind".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// `enum UserContent { Text(String), Parts(Vec<ContentPart>) }` ->
/// `string | AlefContentPartWire[]` plus an emitted `AlefContentPartWire` interface.
#[test]
fn user_content_maps_to_string_or_content_part_array() {
    let enum_def = EnumDef {
        name: "UserContent".to_string(),
        rust_path: "test_lib::UserContent".to_string(),
        variants: vec![
            tuple_variant("Text", TypeRef::String),
            tuple_variant(
                "Parts",
                TypeRef::Vec(Box::new(TypeRef::Named("ContentPart".to_string()))),
            ),
        ],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![content_part_type()];
    let plan = plan_for(&enum_def, &api);

    assert!(
        plan.contains("export type AlefUserContent = string | AlefContentPartWire[];"),
        "actual:\n{}",
        plan
    );
    assert!(
        plan.contains("export interface AlefContentPartWire {\n    text: string;\n    kind: string;\n}"),
        "actual:\n{}",
        plan
    );
}

/// `AssistantContent` has the identical scalar-or-vec-of-struct shape, over `AssistantPart`.
#[test]
fn assistant_content_maps_to_string_or_assistant_part_array() {
    let enum_def = EnumDef {
        name: "AssistantContent".to_string(),
        rust_path: "test_lib::AssistantContent".to_string(),
        variants: vec![
            tuple_variant("Text", TypeRef::String),
            tuple_variant(
                "Parts",
                TypeRef::Vec(Box::new(TypeRef::Named("AssistantPart".to_string()))),
            ),
        ],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "AssistantPart".to_string(),
        rust_path: "test_lib::AssistantPart".to_string(),
        fields: vec![FieldDef {
            name: "text".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let plan = plan_for(&enum_def, &api);
    assert!(
        plan.contains("export type AlefAssistantContent = string | AlefAssistantPartWire[];"),
        "actual:\n{}",
        plan
    );
}

/// `enum ToolChoice { Mode(ToolChoiceMode), Specific(SpecificToolChoice) }` where
/// `ToolChoiceMode` is a fieldless enum -> a string-literal union, and `SpecificToolChoice` is a
/// struct -> an interface. Covers both auxiliary-declaration kinds in one union.
#[test]
fn tool_choice_maps_unit_enum_and_struct_newtype_variants() {
    let mode_enum = EnumDef {
        name: "ToolChoiceMode".to_string(),
        rust_path: "test_lib::ToolChoiceMode".to_string(),
        variants: vec![
            EnumVariant {
                name: "Auto".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Required".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "None".to_string(),
                ..Default::default()
            },
        ],
        serde_rename_all: Some("snake_case".to_string()),
        is_copy: true,
        ..Default::default()
    };
    let enum_def = EnumDef {
        name: "ToolChoice".to_string(),
        rust_path: "test_lib::ToolChoice".to_string(),
        variants: vec![
            tuple_variant("Mode", TypeRef::Named("ToolChoiceMode".to_string())),
            tuple_variant("Specific", TypeRef::Named("SpecificToolChoice".to_string())),
        ],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.enums = vec![mode_enum];
    api.types = vec![TypeDef {
        name: "SpecificToolChoice".to_string(),
        rust_path: "test_lib::SpecificToolChoice".to_string(),
        fields: vec![FieldDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let plan = plan_for(&enum_def, &api);

    assert!(
        // `ToolChoiceMode` gets a `Wire` suffix here: the bare name is already claimed by the
        // real wasm-bindgen `enum WasmToolChoiceMode` (numeric ABI discriminant) emitted
        // unconditionally for every fieldless enum — a different, incompatible runtime shape
        // from this union member's serde wire string. See `map_named_enum`'s `~keep` note.
        plan.contains("export type AlefToolChoice = AlefToolChoiceModeWire | AlefSpecificToolChoiceWire;"),
        "actual:\n{}",
        plan
    );
    assert!(
        plan.contains(r#"export type AlefToolChoiceModeWire = "auto" | "required" | "none";"#),
        "actual:\n{}",
        plan
    );
    assert!(
        plan.contains("export interface AlefSpecificToolChoiceWire {\n    name: string;\n}"),
        "actual:\n{}",
        plan
    );
}

// ---------------------------------------------------------------------------------------------
// Primitive / container mapping
// ---------------------------------------------------------------------------------------------

#[test]
fn primitive_types_map_to_expected_ts() {
    let cases: &[(TypeRef, &str)] = &[
        (TypeRef::Primitive(PrimitiveType::Bool), "boolean"),
        (TypeRef::Primitive(PrimitiveType::U32), "number"),
        (TypeRef::Primitive(PrimitiveType::F64), "number"),
        (TypeRef::Primitive(PrimitiveType::U64), "bigint"),
        (TypeRef::Primitive(PrimitiveType::I64), "bigint"),
        (TypeRef::String, "string"),
        (TypeRef::Char, "string"),
        (TypeRef::Path, "string"),
        (TypeRef::Bytes, "Uint8Array"),
        (TypeRef::Unit, "null"),
        (TypeRef::Json, "any"),
        (TypeRef::Duration, "number"),
    ];
    let api = empty_api();
    for (ty, expected) in cases {
        let mut ctx = TsMapContext {
            api: &api,
            exclude_types: &AHashSet::default(),
            opaque_type_names: &AHashSet::default(),
            prefix: "Alef",
            in_progress: AHashMap::default(),
            resolved_names: ahash::AHashMap::default(),
            decls: Vec::new(),
            casing: super::WireCasing::Serde,
        };
        assert_eq!(ctx.map_type(ty), *expected, "mapping {ty:?}");
    }
}

#[test]
fn vec_maps_to_element_array() {
    let api = empty_api();
    let mut ctx = TsMapContext {
        api: &api,
        exclude_types: &AHashSet::default(),
        opaque_type_names: &AHashSet::default(),
        prefix: "Alef",
        in_progress: AHashMap::default(),
        resolved_names: ahash::AHashMap::default(),
        decls: Vec::new(),
        casing: super::WireCasing::Serde,
    };
    let ty = TypeRef::Vec(Box::new(TypeRef::String));
    assert_eq!(ctx.map_type(&ty), "string[]");
}

/// `Vec<Option<String>>` must render as `(string | undefined)[]`, not `string | undefined[]`.
/// TypeScript's `[]` suffix binds tighter than `|`, so an unparenthesized union followed by `[]`
/// only arrays the union's last operand — `string | undefined[]` parses as
/// `string | (undefined[])`, describing a completely different (and wrong) type: a value that is
/// either a bare string or an array of `undefined`s, never an array containing possibly-absent
/// strings.
///
/// Revert symptom: reverting the `TypeRef::Vec` match arm back to the unconditional
/// `format!("{}[]", ...)` makes this assert `"(string | undefined)[]"` fail because `map_type`
/// instead returns the unparenthesized `"string | undefined[]"`.
#[test]
fn vec_of_optional_wraps_union_before_appending_array_suffix() {
    let api = empty_api();
    let mut ctx = TsMapContext {
        api: &api,
        exclude_types: &AHashSet::default(),
        opaque_type_names: &AHashSet::default(),
        prefix: "Alef",
        in_progress: AHashMap::default(),
        resolved_names: ahash::AHashMap::default(),
        decls: Vec::new(),
        casing: super::WireCasing::Serde,
    };
    let ty = TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(TypeRef::String))));
    assert_eq!(ctx.map_type(&ty), "(string | undefined)[]");
}

/// The same nested shape one level deeper (`Vec<Vec<Option<String>>>`) must parenthesize at the
/// level the union actually occurs, not just the outermost `[]`.
///
/// Revert symptom: reverting the fix makes this assert `"(string | undefined)[][]"` fail because
/// the inner recursive call returns the unparenthesized `"string | undefined[]"`, which the outer
/// `Vec` arm then suffixes as `"string | undefined[][]"`.
#[test]
fn nested_vec_of_optional_parenthesizes_at_the_right_level() {
    let api = empty_api();
    let mut ctx = TsMapContext {
        api: &api,
        exclude_types: &AHashSet::default(),
        opaque_type_names: &AHashSet::default(),
        prefix: "Alef",
        in_progress: AHashMap::default(),
        resolved_names: ahash::AHashMap::default(),
        decls: Vec::new(),
        casing: super::WireCasing::Serde,
    };
    let ty = TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(
        TypeRef::String,
    ))))));
    assert_eq!(ctx.map_type(&ty), "(string | undefined)[][]");
}

#[test]
fn optional_appends_undefined() {
    let api = empty_api();
    let mut ctx = TsMapContext {
        api: &api,
        exclude_types: &AHashSet::default(),
        opaque_type_names: &AHashSet::default(),
        prefix: "Alef",
        in_progress: AHashMap::default(),
        resolved_names: ahash::AHashMap::default(),
        decls: Vec::new(),
        casing: super::WireCasing::Serde,
    };
    let ty = TypeRef::Optional(Box::new(TypeRef::String));
    assert_eq!(ctx.map_type(&ty), "string | undefined");
}

#[test]
fn string_keyed_map_becomes_record() {
    let api = empty_api();
    let mut ctx = TsMapContext {
        api: &api,
        exclude_types: &AHashSet::default(),
        opaque_type_names: &AHashSet::default(),
        prefix: "Alef",
        in_progress: AHashMap::default(),
        resolved_names: ahash::AHashMap::default(),
        decls: Vec::new(),
        casing: super::WireCasing::Serde,
    };
    let ty = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Primitive(PrimitiveType::U32)),
    );
    assert_eq!(ctx.map_type(&ty), "Record<string, number>");
}

#[test]
fn non_string_keyed_map_falls_back_to_any() {
    let api = empty_api();
    let mut ctx = TsMapContext {
        api: &api,
        exclude_types: &AHashSet::default(),
        opaque_type_names: &AHashSet::default(),
        prefix: "Alef",
        in_progress: AHashMap::default(),
        resolved_names: ahash::AHashMap::default(),
        decls: Vec::new(),
        casing: super::WireCasing::Serde,
    };
    let ty = TypeRef::Map(
        Box::new(TypeRef::Primitive(PrimitiveType::U32)),
        Box::new(TypeRef::String),
    );
    assert_eq!(ctx.map_type(&ty), "any");
}

// ---------------------------------------------------------------------------------------------
// Fallback: unmappable payload degrades ONLY that variant, not the whole union
// ---------------------------------------------------------------------------------------------

/// An excluded named type in one variant falls back to `any` for that variant only — the
/// sibling variant keeps its real structural type.
#[test]
fn excluded_type_falls_back_to_any_for_that_variant_only() {
    let enum_def = EnumDef {
        name: "Mixed".to_string(),
        rust_path: "test_lib::Mixed".to_string(),
        variants: vec![
            tuple_variant("Plain", TypeRef::String),
            tuple_variant("Opaque", TypeRef::Named("SecretHandle".to_string())),
        ],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "SecretHandle".to_string(),
        rust_path: "test_lib::SecretHandle".to_string(),
        ..Default::default()
    }];
    let exclude_types: AHashSet<String> = ["SecretHandle".to_string()].into_iter().collect();
    let plan =
        build_untagged_enum_ts_plans(&[&enum_def], &api, &exclude_types, &AHashSet::default(), "Alef").custom_section;
    assert!(
        plan.contains("export type AlefMixed = string | any;"),
        "excluding one variant's type must not collapse the whole union to any;\nactual:\n{}",
        plan
    );
}

/// An opaque (handle) type used as a payload has no public fields to describe structurally, so
/// it falls back to `any` for that variant only.
#[test]
fn opaque_type_falls_back_to_any_for_that_variant_only() {
    let enum_def = EnumDef {
        name: "Mixed".to_string(),
        rust_path: "test_lib::Mixed".to_string(),
        variants: vec![
            tuple_variant("Plain", TypeRef::String),
            tuple_variant("Handle", TypeRef::Named("ClientHandle".to_string())),
        ],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "ClientHandle".to_string(),
        rust_path: "test_lib::ClientHandle".to_string(),
        is_opaque: true,
        ..Default::default()
    }];
    let opaque_types: AHashSet<String> = ["ClientHandle".to_string()].into_iter().collect();
    let plan =
        build_untagged_enum_ts_plans(&[&enum_def], &api, &AHashSet::default(), &opaque_types, "Alef").custom_section;
    assert!(
        plan.contains("export type AlefMixed = string | any;"),
        "actual:\n{}",
        plan
    );
}

/// A named type that resolves to neither a struct nor an enum (e.g. an unresolved generic
/// parameter) falls back to `any` rather than panicking.
#[test]
fn unresolvable_named_type_falls_back_to_any() {
    let enum_def = EnumDef {
        name: "Mixed".to_string(),
        rust_path: "test_lib::Mixed".to_string(),
        variants: vec![tuple_variant("Generic", TypeRef::Named("T".to_string()))],
        serde_untagged: true,
        ..Default::default()
    };
    let plan = plan_for(&enum_def, &empty_api());
    assert!(plan.contains("export type AlefMixed = any;"), "actual:\n{}", plan);
}

// ---------------------------------------------------------------------------------------------
// Recursion guard
// ---------------------------------------------------------------------------------------------

/// A struct that (transitively) contains itself must not blow the stack, and must reference its
/// own interface name for the recursive field rather than re-expanding it.
#[test]
fn self_referential_struct_terminates_and_reuses_its_own_name() {
    let enum_def = EnumDef {
        name: "TreeInput".to_string(),
        rust_path: "test_lib::TreeInput".to_string(),
        variants: vec![tuple_variant("Node", TypeRef::Named("TreeNode".to_string()))],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "TreeNode".to_string(),
        rust_path: "test_lib::TreeNode".to_string(),
        fields: vec![
            FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            },
            FieldDef {
                name: "children".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("TreeNode".to_string()))),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    let plan = plan_for(&enum_def, &api);

    assert!(
        plan.contains("export interface AlefTreeNodeWire {\n    value: string;\n    children: AlefTreeNodeWire[];\n}"),
        "the self-reference must resolve to the interface's own name, not re-expand or fall back to any;\nactual:\n{}",
        plan
    );
    // The interface must be emitted exactly once, not once per reference.
    assert_eq!(
        plan.matches("export interface AlefTreeNodeWire").count(),
        1,
        "actual:\n{}",
        plan
    );
}

/// A mutually-recursive pair (A references B, B references A) must also terminate and de-dupe.
#[test]
fn mutually_recursive_structs_terminate_and_dedupe() {
    let enum_def = EnumDef {
        name: "PairInput".to_string(),
        rust_path: "test_lib::PairInput".to_string(),
        variants: vec![tuple_variant("A", TypeRef::Named("NodeA".to_string()))],
        serde_untagged: true,
        ..Default::default()
    };
    let mut api = empty_api();
    api.types = vec![
        TypeDef {
            name: "NodeA".to_string(),
            rust_path: "test_lib::NodeA".to_string(),
            fields: vec![FieldDef {
                name: "b".to_string(),
                ty: TypeRef::Named("NodeB".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "NodeB".to_string(),
            rust_path: "test_lib::NodeB".to_string(),
            fields: vec![FieldDef {
                name: "a".to_string(),
                ty: TypeRef::Named("NodeA".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        },
    ];
    let plan = plan_for(&enum_def, &api);

    assert_eq!(
        plan.matches("export interface AlefNodeAWire").count(),
        1,
        "actual:\n{}",
        plan
    );
    assert_eq!(
        plan.matches("export interface AlefNodeBWire").count(),
        1,
        "actual:\n{}",
        plan
    );
}

// ---------------------------------------------------------------------------------------------
// Optional / shared-reference field handling
// ---------------------------------------------------------------------------------------------
