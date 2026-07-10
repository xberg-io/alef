use alef::backends::extendr::ExtendrBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::*;

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
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
        has_default: true,
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

fn make_function(name: &str, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        params: vec![],
        return_type,
        is_async: false,
        error_type: None,
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
fn extendr_deeply_nested_transitive_types() {
    let deep_c = make_type("DeepC", vec![make_field("value", TypeRef::String)]);

    let deep_b = make_type("DeepB", vec![make_field("nested", TypeRef::Named("DeepC".to_string()))]);

    let deep_a = make_type("DeepA", vec![make_field("nested", TypeRef::Named("DeepB".to_string()))]);

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![deep_c, deep_b, deep_a],
        functions: vec![make_function("get_deep_a", TypeRef::Named("DeepA".to_string()))],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };

    let files = ExtendrBackend
        .generate_bindings(&api, &make_config())
        .expect("generation succeeds");
    let content = &files[0].content;

    assert!(
        content.contains("impl From<test_lib::DeepA> for DeepA"),
        "DeepA should have From<core::DeepA> impl"
    );
    assert!(
        content.contains("impl From<test_lib::DeepB> for DeepB"),
        "DeepB should have From<core::DeepB> impl (transitive via DeepA)"
    );
    assert!(
        content.contains("impl From<test_lib::DeepC> for DeepC"),
        "DeepC should have From<core::DeepC> impl (deeply transitive)"
    );
}

#[test]
fn extendr_vec_of_vec_nested_types() {
    let item = make_type("Item", vec![make_field("id", TypeRef::Primitive(PrimitiveType::U32))]);

    let container = make_type(
        "Container",
        vec![make_field(
            "nested",
            TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))))),
        )],
    );

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![item, container],
        functions: vec![make_function("get_container", TypeRef::Named("Container".to_string()))],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };

    let files = ExtendrBackend
        .generate_bindings(&api, &make_config())
        .expect("generation succeeds");
    let content = &files[0].content;

    assert!(
        content.contains("impl From<test_lib::Container> for Container"),
        "Container should have From<core::Container> impl"
    );
    assert!(
        content.contains("impl From<test_lib::Item> for Item"),
        "Item should have From<core::Item> impl (nested in Vec<Vec<>>)"
    );
}
