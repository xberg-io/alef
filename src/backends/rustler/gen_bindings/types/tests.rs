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
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
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
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    },
                    FieldDef {
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
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    },
                    FieldDef {
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
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
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
    let result = gen_enum(&unit_enum(), "SampleCrate");
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
    let result = gen_enum(&data_enum(), "SampleCrate");
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
        serde_tag: Some("format_type".into()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_enum(&format_enum, "SampleCrate");
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
        serde_tag: Some("format_type".into()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let from_core = gen_rustler_flat_data_enum_from_core(&enum_def, "sample_crate");
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
