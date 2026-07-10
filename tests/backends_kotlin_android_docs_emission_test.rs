//! Coverage tests for KDoc emission in the kotlin-android (AAR/JNI) backend.
//!
//! The kotlin-android backend reuses the shared `alef-backend-kotlin` emitter
//! helpers (`emit_type_pub`, `emit_enum_pub`, `emit_error_type_pub`,
//! `emit_kdoc_pub`) plus a small set of locally-emitted constructs: the
//! `<Module>Bridge` object, the `<Module>` free-function facade, and the
//! one-off wrapper class for handle-only opaque types.
//!
//! iter-10 Stream A4 closed the data-class field and enum-variant gaps
//! upstream in `alef-backend-kotlin`; these tests prove the doc lands on the
//! kotlin-android-emitted output too.

use alef::backends::kotlin_android::KotlinAndroidBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example"

[crates.package_metadata]
repository = "https://github.com/example/demo"
license = "MIT"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn field_with_doc(name: &str, ty: TypeRef, doc: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: doc.to_string(),
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

fn render(api: ApiSurface) -> Vec<(String, String)> {
    let backend = KotlinAndroidBackend;
    let files = backend.generate_bindings(&api, &make_config()).unwrap();
    files
        .into_iter()
        .map(|f| (f.path.display().to_string(), f.content))
        .collect()
}

#[test]
fn data_class_field_carries_kdoc() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Config".into(),
            rust_path: "demo::Config".into(),
            original_rust_path: String::new(),
            fields: vec![
                field_with_doc(
                    "timeout_secs",
                    TypeRef::Primitive(PrimitiveType::U64),
                    "Maximum time to wait before aborting the call.",
                ),
                field_with_doc("label", TypeRef::String, "Human-readable label."),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "Demo configuration block.".into(),
            cfg: None,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let files = render(api);
    let config_kt = files
        .iter()
        .find(|(p, _)| p.ends_with("Config.kt"))
        .expect("Config.kt emitted");
    let body = &config_kt.1;
    assert!(
        body.contains("Maximum time to wait before aborting the call."),
        "field doc missing:\n{body}"
    );
    assert!(body.contains("Human-readable label."), "label doc missing:\n{body}");
}

#[test]
fn enum_variants_carry_kdoc() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Mode".into(),
            rust_path: "demo::Mode".into(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Fast".into(),
                    fields: vec![],
                    doc: "Optimise for low latency.".into(),
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
                    name: "Accurate".into(),
                    fields: vec![],
                    doc: "Optimise for output quality.".into(),
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
            doc: "Execution mode.".into(),
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
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let files = render(api);
    let mode_kt = files
        .iter()
        .find(|(p, _)| p.ends_with("Mode.kt"))
        .expect("Mode.kt emitted");
    let body = &mode_kt.1;
    assert!(
        body.contains("Optimise for low latency."),
        "Fast variant doc missing:\n{body}"
    );
    assert!(
        body.contains("Optimise for output quality."),
        "Accurate variant doc missing:\n{body}"
    );
}

#[test]
fn module_free_function_facade_carries_kdoc() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "ping".into(),
            rust_path: "demo::ping".into(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "target".into(),
                ty: TypeRef::String,
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
                core_wrapper: alef::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Send a ping request to the target host and return the response.".into(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let files = render(api);
    let module_kt = files
        .iter()
        .find(|(p, _)| p.ends_with("Demo.kt"))
        .expect("Demo.kt module emitted");
    let body = &module_kt.1;
    assert!(
        body.contains("Send a ping request to the target host and return the response."),
        "free-fn KDoc missing:\n{body}"
    );
}

#[test]
fn error_type_with_methods_emits_abstract_properties() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "NotFound".into(),
                message_template: Some("not found".into()),
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![
                MethodDef {
                    name: "status_code".into(),
                    params: vec![],
                    return_type: TypeRef::Primitive(PrimitiveType::U16),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: String::new(),
                    receiver: Some(ReceiverKind::Ref),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                },
                MethodDef {
                    name: "is_transient".into(),
                    params: vec![],
                    return_type: TypeRef::Primitive(PrimitiveType::Bool),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: String::new(),
                    receiver: Some(ReceiverKind::Ref),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                },
                MethodDef {
                    name: "error_type".into(),
                    params: vec![],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: String::new(),
                    receiver: Some(ReceiverKind::Ref),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                },
            ],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let files = render(api);
    let error_kt = files
        .iter()
        .find(|(p, _)| p.ends_with("DemoError.kt"))
        .expect("DemoError.kt must be emitted");
    let body = &error_kt.1;
    assert!(
        body.contains("open val statusCode: Short = 0"),
        "missing statusCode open property: {body}"
    );
    assert!(
        body.contains("open val isTransient: Boolean = false"),
        "missing isTransient open property: {body}"
    );
    assert!(
        body.contains("open val errorType: String = \"\""),
        "missing errorType open property: {body}"
    );
}
