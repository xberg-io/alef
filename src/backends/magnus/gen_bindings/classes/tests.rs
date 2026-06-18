use super::*;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
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
    }
}

fn make_typedef(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    }
}

#[test]
fn gen_enum_unit_variants_emit_ruby_symbols() {
    let enum_def = EnumDef {
        name: "Status".to_string(),
        rust_path: "test_lib::Status".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Pending".to_string(),
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
                name: "Done".to_string(),
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
    };
    let code = gen_enum(&enum_def);
    assert!(code.contains("enum Status"), "must emit enum definition");
    assert!(code.contains("to_symbol"), "unit enums use Ruby symbols");
    assert!(code.contains("\"pending\""), "variant snake_case symbol key");
}

#[test]
fn gen_struct_emits_magnus_wrap_attribute() {
    let typ = make_typedef("Config", vec![make_field("value", TypeRef::String, false)]);
    let mapper = crate::backends::magnus::type_map::MagnusMapper;
    let api = crate::core::ir::ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let code = gen_struct(&typ, &mapper, "TestLib", &api, false, &[]);
    assert!(code.contains("magnus::wrap"), "struct must have magnus::wrap");
    assert!(code.contains("struct Config"), "must emit struct Config");
}

#[test]
fn gen_opaque_struct_emits_arc_inner() {
    let typ = make_typedef("Handle", vec![]);
    let code = gen_opaque_struct(&typ, "test_lib", "TestLib");
    assert!(code.contains("inner: Arc<"), "opaque struct must have Arc inner");
    assert!(code.contains("struct Handle"), "must emit struct Handle");
}
