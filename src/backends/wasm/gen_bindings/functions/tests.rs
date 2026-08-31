use super::*;
use crate::backends::wasm::type_map::WasmMapper;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
use ahash::AHashSet;
use std::collections::HashMap;

fn param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

fn async_function(params: Vec<ParamDef>) -> FunctionDef {
    FunctionDef {
        name: "interact".to_string(),
        rust_path: "sample_fixture::interact".to_string(),
        original_rust_path: String::new(),
        params,
        return_type: TypeRef::Unit,
        is_async: true,
        error_type: Some("CrawlError".to_string()),
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[test]
fn gen_env_shims_emits_expected_signatures_for_all_supported_names() {
    let names: Vec<String> = [
        "iswspace",
        "iswalnum",
        "towupper",
        "iswalpha",
        "iswlower",
        "iswupper",
        "iswxdigit",
        "towlower",
        "memchr",
        "strcmp",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();

    let out = gen_env_shims(&names);

    assert!(out.contains("#[cfg(target_arch = \"wasm32\")]"), "{out}");
    assert_eq!(out.matches("#[unsafe(no_mangle)]").count(), names.len(), "{out}");

    for name in ["iswspace", "iswalnum", "iswalpha", "iswlower", "iswupper", "iswxdigit"] {
        let sig = format!("pub extern \"C\" fn {name}(c: u32) -> i32");
        assert!(out.contains(&sig), "missing signature `{sig}` in:\n{out}");
    }

    for name in ["towupper", "towlower"] {
        let sig = format!("pub extern \"C\" fn {name}(c: u32) -> u32");
        assert!(out.contains(&sig), "missing signature `{sig}` in:\n{out}");
    }

    assert!(
        out.contains("pub unsafe extern \"C\" fn memchr(s: *const u8, c: i32, n: usize) -> *const u8"),
        "{out}"
    );
    assert!(
        out.contains("pub unsafe extern \"C\" fn strcmp(a: *const u8, b: *const u8) -> i32"),
        "{out}"
    );
}

#[test]
fn gen_env_shims_ignores_unknown_names() {
    let names = vec!["not_a_real_shim".to_string()];
    let out = gen_env_shims(&names);
    assert!(!out.contains("#[unsafe(no_mangle)]"), "{out}");
}

#[test]
fn async_vec_named_params_convert_to_core_vec() {
    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let func = async_function(vec![param(
        "actions",
        TypeRef::Vec(Box::new(TypeRef::Named("PageAction".to_string()))),
    )]);
    let api = crate::core::ir::ApiSurface {
        crate_name: "sample_fixture".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &api,
        &AHashSet::new(),
    );

    assert!(out.contains("actions: Vec<WasmPageAction>"));
    assert!(
        out.contains(
            "let actions_core: Vec<sample_fixture::PageAction> = actions.into_iter().map(Into::into).collect();"
        ),
        "{out}"
    );
    assert!(out.contains("sample_fixture::interact(actions_core).await"), "{out}");
}

#[test]
fn input_dtos_dedup_flag_skips_generation() {
    let _emitted_dtos: AHashSet<String> = ["OcrConfig".to_string()].iter().cloned().collect();
    use crate::core::ir::{CoreWrapper, FieldDef, PrimitiveType, TypeDef};

    let make_type = |name: &str, field_name: &str, has_default: bool, has_serde: bool| TypeDef {
        name: name.to_string(),
        rust_path: format!("sample::{name}"),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            version: Default::default(),
            name: field_name.to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
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
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };

    assert!(should_have_input_dto(&make_type("OcrOptions", "max_depth", true, true)));
    assert!(!should_have_input_dto(&make_type("OcrConfig", "depth", true, true)));
    assert!(!should_have_input_dto(&make_type(
        "ExtractionOptions",
        "max_depth",
        false,
        true
    )));
    assert!(!should_have_input_dto(&make_type(
        "ExtractionOptions",
        "max_depth",
        true,
        false
    )));
}

#[test]
fn vec_vec_string_collect_has_explicit_type() {
    use crate::codegen::conversions::field_conversion_from_core;

    let ty = TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));
    let conv = field_conversion_from_core("attributes", &ty, false, true, &AHashSet::new());

    assert!(
        conv.contains("collect::<Vec<Vec<String>>>"),
        "collect() must have explicit type ascription for Vec<Vec<String>>: {conv}"
    );

    let ty_opt = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(
        TypeRef::String,
    ))))));
    let conv_opt = field_conversion_from_core("attributes", &ty_opt, true, true, &AHashSet::new());
    assert!(
        conv_opt.contains("collect::<Vec<Vec<String>>>"),
        "optional variant must also have explicit type: {conv_opt}"
    );
}

#[test]
fn sanitized_string_field_uses_json_deserialize() {
    let ty_string = TypeRef::String;

    let conv_normal = dto_field_conversion(&ty_string, false, false);
    assert_eq!(conv_normal, "v.into()", "non-sanitized String should use .into()");

    let conv_sanitized = dto_field_conversion(&ty_string, true, false);
    assert_eq!(
        conv_sanitized, "serde_json::from_str(&v).unwrap_or_default()",
        "sanitized String should use JSON deserialization: {conv_sanitized}"
    );
}

#[test]
fn dto_vec_field_conversion_uses_target_inferred_collect() {
    let ty = TypeRef::Vec(Box::new(TypeRef::String));

    let conv = dto_field_conversion(&ty, false, false);

    assert_eq!(conv, "v.into_iter().collect()");
    assert!(
        !conv.contains("collect::<Vec<_>>()"),
        "collection target must be inferred from the core field: {conv}"
    );
    assert!(
        !conv.contains("Into::into"),
        "plain Vec fields must not wrap target-inferred collect in Into::into: {conv}"
    );
}

#[test]
fn dto_optional_vec_field_conversion_uses_target_inferred_collect() {
    let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));

    let conv = dto_field_conversion(&ty, false, false);

    assert_eq!(conv, "v.map(|items| items.into_iter().collect())");
    assert!(
        !conv.contains("collect::<Vec<_>>()"),
        "optional collection target must be inferred from the core field: {conv}"
    );
}

#[test]
fn dto_vec_field_conversion_wraps_some_when_core_is_optional() {
    let ty = TypeRef::Vec(Box::new(TypeRef::String));

    let conv = dto_field_conversion(&ty, false, true);

    assert_eq!(conv, "Some(v.into_iter().collect())");
}

#[test]
fn gen_input_dto_excludes_binding_excluded_fields() {
    use crate::core::ir::{CoreWrapper, FieldDef};

    let make_field = |name: &str, ty: TypeRef, binding_excluded: bool, sanitized: bool| FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty,
        optional: true,
        default: None,
        doc: String::new(),
        sanitized,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded,
        binding_exclusion_reason: None,
        original_type: None,
    };

    let type_def = crate::core::ir::TypeDef {
        name: "CrawlConfig".to_string(),
        rust_path: "sample_fixture::CrawlConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field(
                "max_depth",
                TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
                false,
                false,
            ),
            make_field("bypass", TypeRef::String, true, true),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };

    let (code, _name) = gen_input_dto_for_type("CrawlConfig", "sample_fixture", &type_def);

    assert!(
        code.contains("max_depth"),
        "normal field must appear in input DTO: {code}"
    );
    assert!(
        !code.contains("bypass"),
        "binding_excluded field must not appear in input DTO: {code}"
    );
}

#[test]
fn feature_gated_fields_get_cfg_guards() {
    // Regression test: gen_input_dto_for_type_with_cfg should emit #[cfg(...)]
    use crate::core::ir::{CoreWrapper, FieldDef};

    let make_field = |name: &str, ty: TypeRef, cfg: Option<String>| FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty,
        optional: true,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
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

    let type_def = crate::core::ir::TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "mylib::ExtractionConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field(
                "enabled",
                TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                None,
            ),
            make_field(
                "layout_config",
                TypeRef::Named("LayoutDetectionConfig".to_string()),
                Some("feature = \"layout\"".to_string()),
            ),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };

    let (code_no_layout, _) = gen_input_dto_for_type_with_cfg(
        "ExtractionConfig",
        "mylib",
        &type_def,
        &[],
        &["streaming".to_string()],
        &std::collections::HashSet::new(),
    );

    assert!(
        code_no_layout.contains("#[cfg(feature = \"layout\")]"),
        "Feature-gated field should have #[cfg] guard when feature not enabled: {}",
        code_no_layout
    );
    // It should also have #[serde(skip)] since it's not deserializable without the feature
    assert!(
        code_no_layout.contains("#[serde(skip)]"),
        "Feature-gated field should have #[serde(skip)]: {}",
        code_no_layout
    );

    let (code_with_layout, _) = gen_input_dto_for_type_with_cfg(
        "ExtractionConfig",
        "mylib",
        &type_def,
        &[],
        &["layout".to_string()],
        &std::collections::HashSet::new(),
    );

    assert!(
        !code_with_layout.contains("layout_config: {{ field.ty }},\n{%- endfor %}"),
        "When feature is enabled, field should not be skipped: {}",
        code_with_layout
    );
    assert!(
        code_with_layout.contains("#[cfg(feature = \"layout\")]"),
        "Field should still have cfg guard even when enabled: {}",
        code_with_layout
    );
}

#[test]
fn to_turbofish_from_inserts_turbofish_for_generic_type() {
    assert_eq!(to_turbofish_from("Vec<WasmEntity>"), "Vec::<WasmEntity>");
    assert_eq!(to_turbofish_from("Option<WasmFoo>"), "Option::<WasmFoo>");
    assert_eq!(to_turbofish_from("WasmEntity"), "WasmEntity");
    assert_eq!(to_turbofish_from("HashMap<String, i64>"), "HashMap::<String, i64>");
}

#[test]
fn to_turbofish_from_bare_named_type_is_unchanged() {
    assert_eq!(to_turbofish_from("WasmEntity"), "WasmEntity");
    assert_eq!(to_turbofish_from("ExtractionResult"), "ExtractionResult");
}

#[test]
fn type_has_default_lookup_returns_correct_value() {
    use crate::core::ir::ApiSurface;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        handler_contracts: vec![],
        services: vec![],
        unsupported_public_items: Vec::new(),
    };

    assert!(
        !type_has_default("NonExistentType", &api),
        "Unknown type should return false"
    );
    assert!(
        !type_has_default("AnyType", &api),
        "Empty API should return false for any type"
    );
}

#[test]
fn gen_wasm_unimplemented_body_string_return_fails_loudly() {
    let body = gen_wasm_unimplemented_body(&TypeRef::String, "extract_text", false);
    assert!(
        body.contains("compile_error!"),
        "non-fallible String return must fail the build: {body}"
    );
    assert!(
        !body.contains("[unimplemented:"),
        "must not fabricate a placeholder string: {body}"
    );
}

#[test]
fn gen_wasm_unimplemented_body_vec_return_fails_loudly() {
    let body = gen_wasm_unimplemented_body(&TypeRef::Vec(Box::new(TypeRef::String)), "list_entries", false);
    assert!(
        body.contains("compile_error!"),
        "non-fallible Vec return must fail the build: {body}"
    );
    assert!(!body.contains("Vec::new()"), "must not fabricate an empty Vec: {body}");
}

#[test]
fn gen_wasm_unimplemented_body_optional_return_fails_loudly() {
    let body = gen_wasm_unimplemented_body(&TypeRef::Optional(Box::new(TypeRef::String)), "find_entry", false);
    assert!(
        body.contains("compile_error!"),
        "non-fallible Optional return must fail the build: {body}"
    );
    assert!(!body.contains("\"None\""), "must not fabricate a None literal: {body}");
}

#[test]
fn gen_wasm_unimplemented_body_primitive_return_fails_loudly() {
    let body = gen_wasm_unimplemented_body(
        &TypeRef::Primitive(crate::core::ir::PrimitiveType::I64),
        "count_entries",
        false,
    );
    assert!(
        body.contains("compile_error!"),
        "non-fallible primitive return must fail the build: {body}"
    );
}

#[test]
fn gen_wasm_unimplemented_body_unit_return_stays_void() {
    let body = gen_wasm_unimplemented_body(&TypeRef::Unit, "run_side_effect", false);
    assert_eq!(body, "()", "Unit return must stay a legitimate void value: {body}");
}

#[test]
fn gen_wasm_unimplemented_body_with_error_type_raises_runtime_error() {
    let body = gen_wasm_unimplemented_body(&TypeRef::String, "extract_text", true);
    assert_eq!(
        body, "Err(JsValue::from_str(\"Not implemented: extract_text\"))",
        "fallible functions must keep raising a real runtime error: {body}"
    );
    assert!(
        !body.contains("compile_error!"),
        "fallible path must not also emit compile_error!: {body}"
    );
}

/// Fixture for the return-conversion tests below: an async free function whose return type is a
/// `Named` the mapper may or may not back with a generated wrapper.
fn async_function_returning(name: &str) -> FunctionDef {
    FunctionDef {
        return_type: TypeRef::Named(name.to_string()),
        ..async_function(vec![])
    }
}

fn empty_surface() -> crate::core::ir::ApiSurface {
    crate::core::ir::ApiSurface {
        crate_name: "sample_fixture".to_string(),
        version: "0.1.0".to_string(),
        ..Default::default()
    }
}

/// The async wrapper converts its result with `{mapped}::from(result)`, which only compiles when
/// the mapped type has a `From<CoreType>`. A `wasm.type_overrides` entry can redirect the return
/// type to the opaque `JsValue`, which implements no such conversion — the signature and the body
/// are both written from the mapper, so the body must follow it into the serde bridge.
#[test]
fn async_free_function_returning_js_value_mapped_type_uses_serde() {
    let mut overrides = HashMap::new();
    overrides.insert("Report".to_string(), "JsValue".to_string());
    let mapper = WasmMapper::new(overrides, "Wasm".to_string());

    let out = gen_function_with_emitted_dtos(
        &async_function_returning("Report"),
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &empty_surface(),
        &AHashSet::new(),
    );

    assert!(
        out.contains("serde_wasm_bindgen::to_value(&result)"),
        "a JsValue-mapped return must be serialized:\n{out}"
    );
    assert!(
        !out.contains("JsValue::from(result)"),
        "JsValue implements no From<Report>:\n{out}"
    );
}

/// Positive control: with no override the mapper renders `WasmReport`, whose `From<core::Report>`
/// alef generates itself, so the direct conversion must be unchanged.
#[test]
fn async_free_function_returning_wrapper_mapped_type_keeps_from() {
    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());

    let out = gen_function_with_emitted_dtos(
        &async_function_returning("Report"),
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &empty_surface(),
        &AHashSet::new(),
    );

    assert!(
        out.contains("WasmReport::from(result)"),
        "a wrapper-mapped return must keep the direct From conversion:\n{out}"
    );
    assert!(
        !out.contains("serde_wasm_bindgen::to_value(&result)"),
        "a wrapper-mapped return must not detour through serde:\n{out}"
    );
}
