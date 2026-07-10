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
        has_private_fields: false,
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
        has_default: false,
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

fn make_variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    }
}

fn make_data_enum(name: &str, serde_tag: Option<&str>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants: vec![
            make_variant("Png", vec![]),
            make_variant("Jpeg", vec![make_field("quality", TypeRef::String, false)]),
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_tag: serde_tag.map(str::to_string),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

#[test]
fn gen_enum_wraps_string_for_internally_tagged_enum() {
    // For an internally-tagged enum (`#[serde(tag = "...")]`), serde cannot deserialize a bare
    let code = gen_enum(&make_data_enum("ImageOutputFormat", Some("type")));
    assert!(
        code.contains(r#".or_else(|_| serde_json::from_value(serde_json::json!({ "type": json_str })))"#),
        "expected tagged string wrap for internally-tagged enum: {code}"
    );
}

#[test]
fn gen_enum_keeps_bare_string_for_externally_tagged_enum() {
    // An externally-tagged data enum (no `#[serde(tag)]`) must not gain the tag-wrap branch.
    let code = gen_enum(&make_data_enum("ExternallyTagged", None));
    assert!(
        !code.contains("serde_json::from_value(serde_json::json!({"),
        "externally-tagged enum must not wrap the string in a tag object: {code}"
    );
    assert!(
        code.contains("serde_json::from_str(&json_str)"),
        "data enum must keep the from_str path: {code}"
    );
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

use crate::core::ir::MethodDef;

fn shape_enum() -> EnumDef {
    EnumDef {
        name: "Shape".to_string(),
        rust_path: "test_lib::Shape".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            make_variant("Circle", vec![make_field("radius", TypeRef::String, false)]),
            make_variant(
                "Rect",
                vec![
                    make_field("width", TypeRef::String, false),
                    make_field("height", TypeRef::String, false),
                ],
            ),
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

#[test]
fn variant_constructors_emit_singleton_per_struct_variant() {
    let code = gen_data_enum_variant_constructors(&shape_enum());

    assert!(code.contains("impl Shape {"), "must emit an impl block: {code}");
    assert!(
        code.contains("pub fn _factory_circle(radius: String) -> Self"),
        "{code}"
    );
    assert!(code.contains("Self::Circle { radius }"), "{code}");
    assert!(
        code.contains("pub fn _factory_rect(width: String, height: String) -> Self"),
        "{code}"
    );
    assert!(code.contains("Self::Rect { width, height }"), "{code}");
}

#[test]
fn variant_constructors_use_serde_shaped_named_field_type() {
    let def = EnumDef {
        name: "Wrapper".to_string(),
        rust_path: "test_lib::Wrapper".to_string(),
        original_rust_path: String::new(),
        variants: vec![make_variant(
            "Llm",
            vec![
                make_field("llm", TypeRef::Named("LlmConfig".to_string()), false),
                make_field(
                    "opts",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    false,
                ),
            ],
        )],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let code = gen_data_enum_variant_constructors(&def);

    assert!(
        code.contains("pub fn _factory_llm(llm: LlmConfig, opts: String) -> Self"),
        "{code}"
    );
    assert!(code.contains("Self::Llm { llm, opts }"), "{code}");
    assert!(
        !code.contains("_core"),
        "magnus enum is binding-shaped, no core conversion: {code}"
    );
}

#[test]
fn variant_constructors_skip_unit_tuple_and_excluded() {
    let mut tuple_variant = make_variant("Pair", vec![make_field("_0", TypeRef::String, false)]);
    tuple_variant.is_tuple = true;
    let mut excluded = make_variant("Hidden", vec![make_field("value", TypeRef::String, false)]);
    excluded.binding_excluded = true;

    let def = EnumDef {
        variants: vec![
            make_variant("Empty", vec![]),
            tuple_variant,
            excluded,
            make_variant("Real", vec![make_field("value", TypeRef::String, false)]),
        ],
        ..shape_enum()
    };

    let code = gen_data_enum_variant_constructors(&def);

    assert!(!code.contains("_factory_empty"), "{code}");
    assert!(!code.contains("_factory_pair"), "{code}");
    assert!(!code.contains("_factory_hidden"), "{code}");
    assert!(code.contains("pub fn _factory_real(value: String) -> Self"), "{code}");
}

#[test]
fn variant_constructors_yield_to_hand_written_method() {
    let def = EnumDef {
        methods: vec![MethodDef {
            name: "circle".to_string(),
            is_static: true,
            ..Default::default()
        }],
        ..shape_enum()
    };

    let code = gen_data_enum_variant_constructors(&def);

    assert!(
        !code.contains("Self::Circle"),
        "consumer method must win for Circle: {code}"
    );
    assert!(
        code.contains("pub fn _factory_rect(width: String, height: String) -> Self"),
        "{code}"
    );
}

#[test]
fn variant_constructors_empty_for_unit_only_enum() {
    let def = EnumDef {
        variants: vec![make_variant("A", vec![]), make_variant("B", vec![])],
        ..shape_enum()
    };
    let code = gen_data_enum_variant_constructors(&def);
    assert!(code.is_empty(), "expected no output for unit-only enum: {code}");
}
