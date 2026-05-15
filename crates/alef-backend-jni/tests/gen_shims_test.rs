use alef_backend_jni::JniBackend;
use alef_core::backend::Backend;
use alef_core::config::{AdapterConfig, AdapterParam, AdapterPattern, NewAlefConfig, ResolvedCrateConfig};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef,
    ParamDef, PrimitiveType, TypeDef, TypeRef,
};

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
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
    }
}

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
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
    }
}

fn make_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, is_async: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        is_static: false,
        receiver: None,
        error_type: None,
        doc: String::new(),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    }
}

/// A minimal API: one top-level function + one opaque client type with instance methods.
fn make_demo_api() -> ApiSurface {
    let client_type = TypeDef {
        name: "DemoClient".to_string(),
        rust_path: "demo::DemoClient".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![
            make_method(
                "create",
                vec![make_param("api_key", TypeRef::String)],
                TypeRef::Named("DemoClient".to_string()),
                true,
            ),
            make_method("ping", vec![], TypeRef::Primitive(PrimitiveType::Bool), true),
        ],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: "Opaque demo client handle.".to_string(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
    };

    let config_type = TypeDef {
        name: "DemoConfig".to_string(),
        rust_path: "demo::DemoConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("model", TypeRef::String),
            make_field("timeout_secs", TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U32)))),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: "Demo configuration.".to_string(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
    };

    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type, config_type],
        functions: vec![FunctionDef {
            name: "new_client".into(),
            rust_path: "demo::new_client".into(),
            original_rust_path: String::new(),
            params: vec![make_param("api_key", TypeRef::String)],
            return_type: TypeRef::Named("DemoClient".to_string()),
            is_async: true,
            error_type: Some("DemoError".to_string()),
            doc: "Create a new demo client.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![EnumDef {
            name: "DemoModel".to_string(),
            rust_path: "demo::DemoModel".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Standard".to_string(),
                fields: vec![],
                is_tuple: false,
                doc: String::new(),
                is_default: true,
                serde_rename: None,
            }],
            doc: "Available models.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
        }],
        errors: vec![ErrorDef {
            name: "DemoError".to_string(),
            rust_path: "demo::DemoError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "ApiError".to_string(),
                message_template: Some("api error".to_string()),
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                doc: String::new(),
            }],
            doc: "Errors from demo operations.".to_string(),
        }],
        excluded_type_paths: std::collections::HashMap::new(),
    }
}

fn make_demo_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.kreuzberg.demo"
namespace = "dev.kreuzberg.demo"
"#,
    )
}

/// Snapshot test: verify the emitted `lib.rs` content for a basic API with an
/// opaque client type and a top-level function.
#[test]
fn snapshot_demo_lib_rs() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 1, "JNI backend should emit exactly one file");
    let lib = &files[0];
    assert!(
        lib.path.ends_with("lib.rs"),
        "emitted file must be lib.rs, got {:?}",
        lib.path
    );
    insta::assert_snapshot!("snapshot_demo_lib_rs", &lib.content);
}

/// Verify that every JNI symbol in the emitted output starts with `Java_` and
/// uses the package from `[crates.kotlin_android] package`.
#[test]
fn emitted_symbols_match_kotlin_package() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // Package `dev.kreuzberg.demo` encodes as `dev_kreuzberg_demo`.
    assert!(
        content.contains("Java_dev_kreuzberg_demo_"),
        "symbols must use package prefix `dev_kreuzberg_demo_`; got:\n{content}"
    );
    // Bridge class `DemoBridge` appears after the package prefix.
    assert!(
        content.contains("DemoBridge"),
        "symbols must reference `DemoBridge`; got:\n{content}"
    );
}

/// Verify that the top-level `new_client` function emits a `nativeNewClient` symbol.
#[test]
fn top_level_function_emits_native_prefix() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("nativeNewClient"),
        "top-level function must emit `nativeNewClient` symbol; got:\n{content}"
    );
}

/// Verify that the destructor shim for `DemoClient` is emitted as
/// `nativeFreeDemoClient`.
#[test]
fn destructor_shim_emitted_for_opaque_type() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("nativeFreeDemoClient"),
        "destructor shim `nativeFreeDemoClient` must appear; got:\n{content}"
    );
}

/// Verify that `#![allow(non_snake_case)]` is emitted so the JNI symbol names
/// don't trigger Rust naming-convention warnings.
#[test]
fn non_snake_case_allow_is_emitted() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("#![allow(non_snake_case)]"),
        "file must suppress non_snake_case warning; got:\n{content}"
    );
}

/// Pairing-drift sentinel: JNI symbol names must agree with what the
/// `alef_core::jni` helpers would produce for the same inputs.
///
/// This test encodes the JNI spec §5.11.3 contract between the Rust shim
/// emitter and the Kotlin bridge object emitter. If either side changes the
/// naming function the test fails, preventing silent drift.
#[test]
fn jni_symbols_agree_with_alef_core_jni_helpers() {
    use alef_core::jni::{bridge_class_name, bridge_method_name, destructor_method_name, jni_symbol};

    let package = "dev.kreuzberg.demo";
    let bridge = bridge_class_name("demo");
    assert_eq!(&bridge, "DemoBridge");

    // Top-level function symbol.
    let fn_method = bridge_method_name("", "new_client");
    let fn_sym = jni_symbol(package, &bridge, &fn_method);
    assert_eq!(fn_sym, "Java_dev_kreuzberg_demo_DemoBridge_nativeNewClient");

    // Instance method symbol.
    let method = bridge_method_name("DemoClient", "ping");
    let method_sym = jni_symbol(package, &bridge, &method);
    assert_eq!(method_sym, "Java_dev_kreuzberg_demo_DemoBridge_nativeDemoClientPing");

    // Destructor symbol.
    let dtor = destructor_method_name("DemoClient");
    let dtor_sym = jni_symbol(package, &bridge, &dtor);
    assert_eq!(dtor_sym, "Java_dev_kreuzberg_demo_DemoBridge_nativeFreeDemoClient");
}

/// Streaming adapter shims (Start/Next/Free) are emitted for a `Streaming`
/// adapter that has `owner_type = "DemoClient"`.
#[test]
fn streaming_adapter_shims_are_emitted() {
    let api = make_demo_api();
    let mut config = make_demo_config();

    // Inject a streaming adapter owned by DemoClient.
    config.adapters.push(AdapterConfig {
        name: "chat_stream".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "demo::DemoClient::chat_stream".to_string(),
        params: vec![AdapterParam {
            name: "request".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        returns: Some("String".to_string()),
        error_type: None,
        owner_type: Some("DemoClient".to_string()),
        item_type: Some("StreamChunk".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,
    });

    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("nativeDemoClientChatStreamStart"),
        "Start shim must be emitted; got:\n{content}"
    );
    assert!(
        content.contains("nativeDemoClientChatStreamNext"),
        "Next shim must be emitted; got:\n{content}"
    );
    assert!(
        content.contains("nativeDemoClientChatStreamFree"),
        "Free shim must be emitted; got:\n{content}"
    );

}
