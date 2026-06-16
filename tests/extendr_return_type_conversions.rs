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
        version: Default::default(),
    }
}

fn make_enum(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants,
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
    }
}

fn make_variant(name: &str, fields: Vec<FieldDef>, is_tuple: bool) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple,
        originally_had_data_fields: false,
        cfg: None,
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
fn extendr_emits_from_core_for_types_in_return_values() {
    // Bug: when a function returns a struct that contains fields referencing
    // nested types (e.g., Bar inside Foo), the nested types don't get From<core::T>
    // and TryFrom<Robj> impls generated, causing compilation errors.
    //
    // Fix: collect all types reachable from return values and emit conversions for them.

    let bar = make_type("Bar", vec![make_field("value", TypeRef::String)]);
    let foo = make_type("Foo", vec![make_field("bar", TypeRef::Named("Bar".to_string()))]);

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![bar, foo],
        functions: vec![make_function("get_foo", TypeRef::Named("Foo".to_string()))],
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

    // Both Foo and Bar should have From<core::T> impls since Bar is referenced from Foo's return type
    assert!(
        content.contains("impl From<test_lib::Foo> for Foo"),
        "Foo should have From<core::Foo> impl: {content}"
    );
    assert!(
        content.contains("impl From<test_lib::Bar> for Bar"),
        "Bar should have From<core::Bar> impl (nested in return type): {content}"
    );
}

#[test]
fn extendr_emits_conversions_for_vec_named_struct_in_return() {
    // Bug: Vec<NamedStruct> in return types doesn't get TryFrom<Robj> impl.
    // Fix: emit a dedicated TryFrom<Robj> for Vec<Item> types.

    let item = make_type("Item", vec![make_field("id", TypeRef::Primitive(PrimitiveType::U32))]);
    let container = make_type(
        "Container",
        vec![make_field(
            "items",
            TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
        )],
    );

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![item, container],
        functions: vec![make_function("list_items", TypeRef::Named("Container".to_string()))],
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

    // Both Container and Item should have conversion impls
    assert!(
        content.contains("impl From<test_lib::Container> for Container"),
        "Container should have From<core::Container> impl: {content}"
    );
    assert!(
        content.contains("impl From<test_lib::Item> for Item"),
        "Item should have From<core::Item> impl (nested in Vec): {content}"
    );
}

#[test]
fn extendr_emits_conversions_for_struct_variant_enum_payloads() {
    // Bug: enum variants with struct fields (not tuple) don't get proper conversion impls
    // for the payload types when the enum is in a return type.
    //
    // Example: enum Result { Ok(Value), Err { code: u32, message: String } }
    // The struct variant Err has code/message fields that may not get converted.

    let value_type = make_type("Value", vec![make_field("data", TypeRef::String)]);

    let result_enum = make_enum(
        "Result",
        vec![
            make_variant(
                "Ok",
                vec![make_field("_0", TypeRef::Named("Value".to_string()))],
                true, // tuple variant
            ),
            make_variant(
                "Err",
                vec![
                    make_field("code", TypeRef::Primitive(PrimitiveType::U32)),
                    make_field("message", TypeRef::String),
                ],
                false, // struct variant
            ),
        ],
    );

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![value_type],
        functions: vec![make_function("operation", TypeRef::Named("Result".to_string()))],
        enums: vec![result_enum],
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

    // Value should have conversion impls since it's in an enum variant used in return
    assert!(
        content.contains("impl From<test_lib::Value> for Value"),
        "Value should have From<core::Value> impl (nested in enum variant): {content}"
    );

    // Result enum should have conversion impl
    assert!(
        content.contains("impl From<test_lib::Result> for Result")
            || content.contains("impl From<Result> for test_lib::Result"),
        "Result enum should have conversion impls: {content}"
    );
}
