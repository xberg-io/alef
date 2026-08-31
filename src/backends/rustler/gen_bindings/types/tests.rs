use super::*;
use crate::core::ir::{EnumVariant, FieldDef, PrimitiveType};

fn unit_enum() -> EnumDef {
    EnumDef {
        name: "Color".to_string(),
        rust_path: "my_crate::Color".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Red".into(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Blue".into(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

fn data_enum() -> EnumDef {
    EnumDef {
        name: "SecuritySchemeInfo".to_string(),
        rust_path: "my_crate::SecuritySchemeInfo".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Http".into(),
                fields: vec![
                    FieldDef {
                        version: Default::default(),
                        name: "scheme".into(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        serde_with: None,
                        serde_skip_serializing_if: false,
                        serde_skip: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    },
                    FieldDef {
                        version: Default::default(),
                        name: "bearer_format".into(),
                        ty: TypeRef::Optional(Box::new(TypeRef::String)),
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        serde_with: None,
                        serde_skip_serializing_if: false,
                        serde_skip: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    },
                ],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "ApiKey".into(),
                fields: vec![
                    FieldDef {
                        version: Default::default(),
                        name: "location".into(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        serde_with: None,
                        serde_skip_serializing_if: false,
                        serde_skip: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    },
                    FieldDef {
                        version: Default::default(),
                        name: "name".into(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        serde_with: None,
                        serde_skip_serializing_if: false,
                        serde_skip: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    },
                ],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

/// Unit enums must still lower to NifUnitEnum (atoms on the Elixir side).
#[test]
fn test_gen_enum_unit_uses_nif_unit_enum() {
    let result = gen_enum(&unit_enum(), "SampleCrate", &ApiSurface::default(), "mylib", None);
    assert!(
        result.contains("NifUnitEnum"),
        "unit enum should use NifUnitEnum; got:\n{result}"
    );
    assert!(
        !result.contains("NifTaggedEnum"),
        "unit enum must not use NifTaggedEnum; got:\n{result}"
    );
    assert!(result.contains("Red,"), "should contain Red variant; got:\n{result}");
    assert!(result.contains("Blue,"), "should contain Blue variant; got:\n{result}");
}

/// Data enums must lower to NifTaggedEnum and preserve all variant fields.
#[test]
fn test_gen_enum_data_uses_nif_tagged_enum() {
    let result = gen_enum(&data_enum(), "SampleCrate", &ApiSurface::default(), "mylib", None);
    assert!(
        result.contains("NifTaggedEnum"),
        "data enum should use NifTaggedEnum; got:\n{result}"
    );
    assert!(
        !result.contains("NifUnitEnum"),
        "data enum must not use NifUnitEnum; got:\n{result}"
    );
    assert!(
        result.contains("scheme"),
        "Http variant must preserve `scheme` field; got:\n{result}"
    );
    assert!(
        result.contains("bearer_format"),
        "Http variant must preserve `bearer_format` field; got:\n{result}"
    );
    assert!(
        result.contains("location"),
        "ApiKey variant must preserve `location` field; got:\n{result}"
    );
    assert!(
        result.contains("name"),
        "ApiKey variant must preserve `name` field; got:\n{result}"
    );
}

// --- `#[expect(clippy::large_enum_variant, ...)]` gating (alef #545) ------------------------
//
// Rustler's NifTaggedEnum path emits a real Rust `enum` with data-carrying variants, which is
// the shape `clippy::large_enum_variant` inspects. These two tests are a matched pair: the
// first proves the attribute appears when one variant's payload genuinely dwarfs its siblings,
// the second proves it does NOT appear for an otherwise-identical NifTaggedEnum whose variants
// are comparably sized. The second test is the one that would catch an unconditional emission
// -- an `#[expect]` on an enum the real lint never fires on is a hard
// `unfulfilled_lint_expectation` compile error, not a warning.

fn string_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        ..FieldDef::default()
    }
}

/// A struct with enough `String` fields that its estimated size clears
/// `enum_variant_size::EXPECT_GAP_THRESHOLD_BYTES` against a small sibling variant.
fn heavy_config_type() -> TypeDef {
    TypeDef {
        name: "RemoteProviderConfig".to_string(),
        rust_path: "sample_crate::RemoteProviderConfig".to_string(),
        fields: (0..30).map(|i| string_field(&format!("setting_{i}"))).collect(),
        ..TypeDef::default()
    }
}

fn struct_variant(name: &str, field_name: &str, ty: TypeRef) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields: vec![FieldDef {
            name: field_name.to_string(),
            ty,
            ..FieldDef::default()
        }],
        is_tuple: false,
        ..EnumVariant::default()
    }
}

/// One struct variant wraps a large `Named` payload (`RemoteProviderConfig`); its siblings are
/// tiny. Mirrors the reported shape: a struct-field variant (`Llm { llm: LlmConfig }` in the
/// consumer's report), not a tuple variant, so this cannot route through the flat-struct
/// lowering (`is_flat_data_enum` requires every data variant to be a tuple variant).
fn enum_with_one_oversized_struct_variant() -> EnumDef {
    EnumDef {
        name: "ProviderKind".to_string(),
        rust_path: "sample_crate::ProviderKind".to_string(),
        variants: vec![
            struct_variant("Remote", "config", TypeRef::Named("RemoteProviderConfig".to_string())),
            EnumVariant {
                name: "Local".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                is_tuple: true,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "None".to_string(),
                fields: vec![],
                is_tuple: false,
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

/// Same shape as [`enum_with_one_oversized_struct_variant`] -- a struct variant among tuple and
/// unit siblings, routing through the identical NifTaggedEnum codepath -- but every variant's
/// payload is small, so no variant should be estimated as dwarfing its siblings.
fn enum_with_similarly_sized_struct_variant() -> EnumDef {
    EnumDef {
        name: "RequestKind".to_string(),
        rust_path: "sample_crate::RequestKind".to_string(),
        variants: vec![
            struct_variant("Configured", "name", TypeRef::String),
            EnumVariant {
                name: "Simple".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                is_tuple: true,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "None".to_string(),
                fields: vec![],
                is_tuple: false,
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

/// Positive control: a struct variant whose payload genuinely dwarfs its siblings must get the
/// narrow `#[expect(...)]`, and the emitted enum must still be a real `NifTaggedEnum` (no
/// silent fallback to a different lowering).
#[test]
fn gen_enum_emits_expect_for_genuinely_oversized_variant() {
    let mut api = ApiSurface::default();
    api.types.push(heavy_config_type());

    let result = gen_enum(
        &enum_with_one_oversized_struct_variant(),
        "SampleCrate",
        &api,
        "mylib",
        None,
    );

    assert!(
        result.contains("#[expect(clippy::large_enum_variant, reason ="),
        "expected a narrow #[expect(clippy::large_enum_variant, ...)] attribute; got:\n{result}"
    );
    assert!(
        result.contains("NifTaggedEnum"),
        "must still lower to NifTaggedEnum, not a boxed or flattened shape; got:\n{result}"
    );
    assert!(
        !result.contains("Box<"),
        "the consumer's chosen remedy is the narrow #[expect], not boxing; got:\n{result}"
    );
}

/// Negative control: an otherwise-identical NifTaggedEnum whose variants are comparably sized
/// must NOT get the attribute. This is the test that fails if `gen_enum` starts emitting
/// `#[expect(clippy::large_enum_variant, ...)]` unconditionally on every NifTaggedEnum --
/// exactly the trap task #545 calls out, since `#[expect]` hard-errors
/// (`unfulfilled_lint_expectation`) when the lint it names never fires.
#[test]
fn gen_enum_does_not_emit_expect_for_similarly_sized_variants() {
    let api = ApiSurface::default();

    let result = gen_enum(
        &enum_with_similarly_sized_struct_variant(),
        "SampleCrate",
        &api,
        "mylib",
        None,
    );

    assert!(
        result.contains("NifTaggedEnum"),
        "sanity check: this fixture must exercise the NifTaggedEnum path; got:\n{result}"
    );
    assert!(
        !result.contains("clippy::large_enum_variant"),
        "no variant here dwarfs its siblings; an unconditional #[expect] would hard-error via \
         unfulfilled_lint_expectation on this exact shape; got:\n{result}"
    );
}

#[test]
fn data_enum_emits_adjacent_serde_representation() {
    let mut enum_def = data_enum();
    enum_def.serde_tag = Some("type".to_string());
    enum_def.serde_content = Some("output".to_string());

    let result = gen_enum(&enum_def, "SampleCrate", &ApiSurface::default(), "mylib", None);

    assert!(result.contains(r#"#[serde(tag = "type", content = "output""#));
}

/// Data enums with tuple variants containing Named types should use flat NifStruct.
#[test]
fn test_gen_enum_tuple_named_uses_nif_struct() {
    let format_enum = EnumDef {
        name: "FormatMetadata".to_string(),
        rust_path: "my_crate::FormatMetadata".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Excel".into(),
                fields: vec![FieldDef {
                    version: Default::default(),
                    name: "_0".into(),
                    ty: TypeRef::Named("ExcelMetadata".into()),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    serde_with: None,
                    serde_skip_serializing_if: false,
                    serde_skip: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Pdf".into(),
                fields: vec![FieldDef {
                    version: Default::default(),
                    name: "_0".into(),
                    ty: TypeRef::Named("String".into()),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    serde_with: None,
                    serde_skip_serializing_if: false,
                    serde_skip: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_content: None,
        serde_tag: Some("format_type".into()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_enum(&format_enum, "SampleCrate", &ApiSurface::default(), "mylib", None);
    assert!(
        result.contains("NifStruct"),
        "tuple data enum with named types should use NifStruct; got:\n{result}"
    );
    assert!(
        !result.contains("NifTaggedEnum"),
        "tuple data enum with named types must not use NifTaggedEnum; got:\n{result}"
    );
    assert!(
        result.contains("format_type: String"),
        "should have format_type discriminator; got:\n{result}"
    );
    assert!(
        result.contains("excel: Option<ExcelMetadata>"),
        "should have optional excel field; got:\n{result}"
    );
    assert!(
        result.contains("pdf: Option<String>"),
        "should have optional pdf field; got:\n{result}"
    );
}

/// Data enum From impls must destructure fields, not use Default::default().
#[test]
fn test_data_enum_from_impls_destructure_fields() {
    let e = data_enum();
    let cfg = crate::codegen::conversions::ConversionConfig {
        binding_enums_have_data: true,
        ..Default::default()
    };
    let binding_to_core = crate::codegen::conversions::gen_enum_from_binding_to_core_cfg(&e, "my_crate", &cfg);
    assert!(
        !binding_to_core.contains("Default::default()"),
        "binding->core From must not use Default::default() for data enum fields; got:\n{binding_to_core}"
    );
    assert!(
        binding_to_core.contains("scheme"),
        "binding->core From must destructure `scheme`; got:\n{binding_to_core}"
    );
    assert!(
        binding_to_core.contains("bearer_format"),
        "binding->core From must destructure `bearer_format`; got:\n{binding_to_core}"
    );

    let core_to_binding = crate::codegen::conversions::gen_enum_from_core_to_binding_cfg(&e, "my_crate", &cfg);
    assert!(
        core_to_binding.contains("scheme"),
        "core->binding From must destructure `scheme`; got:\n{core_to_binding}"
    );
    assert!(
        !core_to_binding.contains(".."),
        "core->binding From must not discard fields with `..`; got:\n{core_to_binding}"
    );
}

/// Flat data enum From impls must use the enum's full `rust_path`, not
/// the short `{core_import}::{name}` form. Regression for sample_core's
/// elixir NIF emitting `impl From<sample_core::DrawingType> for DrawingType`
/// instead of `impl From<sample_core::extraction::docx::drawing::DrawingType>`
/// — the short form fails to compile because DrawingType is not re-exported
/// from the crate root.
#[test]
fn test_flat_data_enum_from_core_uses_full_rust_path() {
    let enum_def = EnumDef {
        name: "DrawingType".to_string(),
        rust_path: "sample_crate::extraction::docx::drawing::DrawingType".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Inline".into(),
                fields: vec![],
                doc: String::new(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Anchored".into(),
                fields: vec![FieldDef {
                    version: Default::default(),
                    name: "_0".into(),
                    ty: TypeRef::Named("AnchorProperties".into()),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: true,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    serde_with: None,
                    serde_skip_serializing_if: false,
                    serde_skip: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_content: None,
        serde_tag: Some("format_type".into()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let from_core = gen_rustler_flat_data_enum_from_core(&enum_def, "sample_crate", None);
    assert!(
        from_core.contains("sample_crate::extraction::docx::drawing::DrawingType"),
        "flat data enum From<core> must use full rust_path; got:\n{from_core}"
    );
    assert!(
        !from_core.contains("From<sample_crate::DrawingType>"),
        "flat data enum From<core> must not collapse to {{core_import}}::{{name}}; got:\n{from_core}"
    );

    let to_core = gen_rustler_flat_data_enum_to_core(&enum_def, "sample_crate");
    assert!(
        to_core.contains("sample_crate::extraction::docx::drawing::DrawingType"),
        "flat data enum From<binding> for core must use full rust_path; got:\n{to_core}"
    );
    assert!(
        !to_core.contains("for sample_crate::DrawingType "),
        "flat data enum From<binding> must target full rust_path; got:\n{to_core}"
    );
}

/// Primitive field type mapping for NifTaggedEnum variants.
#[test]
fn test_field_type_for_rustler_primitives() {
    let bool_field = FieldDef {
        version: Default::default(),
        name: "flag".into(),
        ty: TypeRef::Primitive(PrimitiveType::Bool),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: crate::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    assert_eq!(field_type_for_rustler(&bool_field), "bool");
    let str_field = FieldDef {
        name: "s".into(),
        ty: TypeRef::String,
        ..bool_field.clone()
    };
    assert_eq!(field_type_for_rustler(&str_field), "String");
    let opt_field = FieldDef {
        name: "o".into(),
        ty: TypeRef::Optional(Box::new(TypeRef::String)),
        ..bool_field
    };
    assert_eq!(field_type_for_rustler(&opt_field), "Option<String>");
}

// --- gen_struct deserialize-delegation wiring -------------------------------------------------

fn f64_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty: TypeRef::Primitive(PrimitiveType::F64),
        ..Default::default()
    }
}

fn container_conversion() -> crate::core::ir::SerdeContainerConversion {
    crate::core::ir::SerdeContainerConversion {
        from: Some("WireShape".to_string()),
        into: Some("WireShape".to_string()),
        try_from: None,
        transparent: false,
    }
}

fn point_type(conversion: crate::core::ir::SerdeContainerConversion) -> TypeDef {
    TypeDef {
        name: "Point".to_string(),
        rust_path: "my_crate::Point".to_string(),
        fields: vec![f64_field("x"), f64_field("y")],
        is_opaque: false,
        has_serde: true,
        serde_container_conversion: conversion,
        ..Default::default()
    }
}

/// Extracts the `#[derive(...)]` line so assertions can't be fooled by "serde::Deserialize"
/// also appearing inside the delegating impl's body text.
fn derive_line(rendered: &str) -> &str {
    rendered
        .lines()
        .find(|line| line.trim_start().starts_with("#[derive("))
        .expect("rendered struct has a derive line")
}

#[test]
fn gen_struct_delegates_for_sound_two_field_pair() {
    let typ = point_type(container_conversion());
    let delegatable: AHashSet<String> = ["Point".to_string()].into_iter().collect();
    let rendered = gen_struct(
        &typ,
        &crate::backends::rustler::type_map::RustlerMapper,
        "MyApp.Native",
        &AHashSet::new(),
        "my_crate",
        &[],
        &delegatable,
    );

    assert!(
        !derive_line(rendered.as_str()).contains("serde::Deserialize"),
        "derive line must drop Deserialize when delegating: {rendered}"
    );
    assert!(
        rendered.contains("impl<'de> serde::Deserialize<'de> for Point {"),
        "expected a delegating Deserialize impl in: {rendered}"
    );
    assert!(
        rendered.contains("<my_crate::Point as serde::Deserialize>::deserialize(deserializer).map(Into::into)"),
        "delegating impl must read the core type: {rendered}"
    );
    assert!(
        derive_line(rendered.as_str()).contains("rustler::NifStruct"),
        "rustler's own term codec derive must be untouched: {rendered}"
    );
}

#[test]
fn gen_struct_keeps_derive_when_not_in_delegatable_set() {
    // Sound fields and a real container conversion, but the caller never proved a matching
    // `From<core::Type>` impl will exist for this run (empty delegation set) -- must NOT delegate.
    let typ = point_type(container_conversion());
    let rendered = gen_struct(
        &typ,
        &crate::backends::rustler::type_map::RustlerMapper,
        "MyApp.Native",
        &AHashSet::new(),
        "my_crate",
        &[],
        &AHashSet::new(),
    );

    assert!(derive_line(rendered.as_str()).contains("serde::Deserialize"));
    assert!(!rendered.contains("impl<'de> serde::Deserialize<'de> for Point"));
}

#[test]
fn gen_struct_keeps_derive_when_unsound_opaque_field() {
    let mut typ = point_type(container_conversion());
    typ.fields = vec![FieldDef {
        name: "handle".into(),
        ty: TypeRef::Named("OpaqueHandle".to_string()),
        ..Default::default()
    }];
    let delegatable: AHashSet<String> = ["Point".to_string()].into_iter().collect();
    let opaque_names = ["OpaqueHandle".to_string()];
    let rendered = gen_struct(
        &typ,
        &crate::backends::rustler::type_map::RustlerMapper,
        "MyApp.Native",
        &AHashSet::new(),
        "my_crate",
        &opaque_names,
        &delegatable,
    );

    // Falls back to the derived, field-by-field Deserialize -- the existing
    // SerdeContainerConversionUnsupported diagnostic keeps naming the real gap here.
    assert!(derive_line(rendered.as_str()).contains("serde::Deserialize"));
    assert!(!rendered.contains("impl<'de> serde::Deserialize<'de> for Point"));
}

#[test]
fn gen_struct_never_delegates_without_container_conversion() {
    let typ = point_type(Default::default());
    let delegatable: AHashSet<String> = ["Point".to_string()].into_iter().collect();
    let rendered = gen_struct(
        &typ,
        &crate::backends::rustler::type_map::RustlerMapper,
        "MyApp.Native",
        &AHashSet::new(),
        "my_crate",
        &[],
        &delegatable,
    );

    assert!(derive_line(rendered.as_str()).contains("serde::Deserialize"));
    assert!(!rendered.contains("impl<'de> serde::Deserialize<'de> for Point"));
}
