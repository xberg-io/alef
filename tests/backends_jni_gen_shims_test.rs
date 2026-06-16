use alef::backends::jni::JniBackend;
use alef::core::backend::Backend;
use alef::core::config::workspace::{ClientConstructorConfig, ConstructorParam};
use alef::core::config::{AdapterConfig, AdapterParam, AdapterPattern, NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
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
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

// ---------------------------------------------------------------------------
// Shared Demo fixture (richer surface for snapshot coverage)
// ---------------------------------------------------------------------------

/// Build the Demo fixture API surface:
/// - one top-level function: `create_client(api_key: String) -> Named("DemoClient")` [async, opaque return]
/// - TypeDef `DemoClient` (opaque) with:
///   * `do_thing(input: String) -> String`  (async, 1 param, String return)
///   * `ping() -> bool`                      (async, no params, Bool return)
///   * `fetch_blob() -> Vec<u8>`             (async, no params, ByteArray return)
fn make_demo_api() -> ApiSurface {
    let client_type = TypeDef {
        name: "DemoClient".to_string(),
        rust_path: "demo::DemoClient".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![
            make_method(
                "do_thing",
                vec![make_param("input", TypeRef::String)],
                TypeRef::String,
                true,
            ),
            make_method("ping", vec![], TypeRef::Primitive(PrimitiveType::Bool), true),
            make_method(
                "fetch_blob",
                vec![],
                TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8))),
                true,
            ),
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let config_type = TypeDef {
        name: "DemoConfig".to_string(),
        rust_path: "demo::DemoConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("model", TypeRef::String),
            make_field(
                "timeout_secs",
                TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
            ),
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type, config_type],
        functions: vec![FunctionDef {
            name: "create_client".into(),
            rust_path: "demo::create_client".into(),
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![EnumDef {
            name: "DemoModel".to_string(),
            rust_path: "demo::DemoModel".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Standard".to_string(),
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
            }],
            doc: "Available models.".to_string(),
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
                is_tuple: false,
                doc: String::new(),
            }],
            doc: "Errors from demo operations.".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
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
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
    )
}

/// Build a demo config with a streaming adapter.
fn make_demo_config_with_streaming() -> ResolvedCrateConfig {
    let mut config = make_demo_config();
    config.adapters.push(AdapterConfig {
        name: "stream_data".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "demo::DemoClient::stream_data".to_string(),
        params: vec![AdapterParam {
            name: "request".to_string(),
            ty: "StreamRequest".to_string(),
            optional: false,
        }],
        returns: Some("DataChunk".to_string()),
        error_type: None,
        owner_type: Some("DemoClient".to_string()),
        item_type: Some("DataChunk".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    });
    config
}

// ---------------------------------------------------------------------------
// 1. Full lib.rs snapshot
// ---------------------------------------------------------------------------

/// Snapshot: the entire emitted `lib.rs` for the richer Demo fixture with streaming.
#[test]
fn snapshot_full_lib_rs() {
    let api = make_demo_api();
    let config = make_demo_config_with_streaming();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 1, "JNI backend should emit exactly one file");
    let lib = &files[0];
    assert!(
        lib.path.ends_with("lib.rs"),
        "emitted file must be lib.rs, got {:?}",
        lib.path
    );
    insta::assert_snapshot!("snapshot_full_lib_rs", &lib.content);
}

// ---------------------------------------------------------------------------
// 2. Runtime helpers present
// ---------------------------------------------------------------------------

#[test]
fn snapshot_runtime_helpers_present() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(content.contains("fn runtime()"), "must contain runtime() helper");
    assert!(
        content.contains("fn jstring_to_string"),
        "must contain jstring_to_string helper"
    );
    assert!(
        content.contains("fn throw_jni_error"),
        "must contain throw_jni_error helper"
    );
    assert!(
        content.contains("const ERROR_CLASS: &str = \"dev/sample_crate/demo/DemoBridgeException\""),
        "must contain correct ERROR_CLASS; got:\n{content}"
    );
    insta::assert_snapshot!("snapshot_runtime_helpers_present", content);
}

// ---------------------------------------------------------------------------
// 3. Constructor symbol and body
// ---------------------------------------------------------------------------

#[test]
fn snapshot_constructor_symbol_and_body() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("Java_dev_sample_1crate_demo_DemoBridge_nativeCreateClient"),
        "constructor symbol missing; got:\n{content}"
    );
    // Must return jlong (raw pointer) — NOT a JSON-encoded jstring.
    assert!(
        content.contains("-> jlong"),
        "constructor must return jlong; got:\n{content}"
    );
    // Must do Box::into_raw to return a handle.
    assert!(
        content.contains("Box::into_raw(Box::new(v)) as jlong"),
        "constructor must Box::into_raw the result; got:\n{content}"
    );
    insta::assert_snapshot!("snapshot_constructor_symbol_and_body", content);
}

// ---------------------------------------------------------------------------
// 4. Method with String param
// ---------------------------------------------------------------------------

#[test]
fn snapshot_method_with_param() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // `do_thing` takes a String input, returns String.
    assert!(
        content.contains("nativeDemoClientDoThing"),
        "nativeDemoClientDoThing must be emitted; got:\n{content}"
    );
    // Must unmarshal request_json.
    assert!(
        content.contains("request_json: JString"),
        "do_thing must accept request_json: JString; got:\n{content}"
    );
    let do_thing = extract_fn_section(content, "nativeDemoClientDoThing");
    assert!(
        !do_thing.contains("serde_json::to_string(&v)"),
        "do_thing returns a plain String and must not JSON-serialize it; got:\n{content}"
    );
    // jni 0.22+: helpers take `&mut Env<'_>`, shim wraps body in
    // `env.with_env(|env| ...)`, so inner calls pass the already-mutable
    // reference `env` (no extra `&mut`).
    assert!(
        do_thing.contains("string_to_jstring(env, &v)"),
        "do_thing must return jstring; got:\n{content}"
    );
    insta::assert_snapshot!("snapshot_method_with_param", content);
}

// ---------------------------------------------------------------------------
// 5. No-param method returning bool
// ---------------------------------------------------------------------------

#[test]
fn snapshot_method_no_params_bool() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // `ping` takes no params, returns bool.
    assert!(
        content.contains("nativeDemoClientPing"),
        "nativeDemoClientPing must be emitted; got:\n{content}"
    );
    // No request_json param.
    let ping_section = extract_fn_section(content, "nativeDemoClientPing");
    assert!(
        !ping_section.contains("request_json"),
        "ping must NOT have request_json param; section:\n{ping_section}"
    );
    assert!(
        content.contains("-> jboolean"),
        "ping must return jboolean; got:\n{content}"
    );
    // jni 0.22's `jboolean` is `bool`, so no cast is emitted — the value is returned as-is.
    assert!(
        ping_section.contains("\n    v\n"),
        "ping must return bool as-is (no cast under jni 0.22); section:\n{ping_section}"
    );
    insta::assert_snapshot!("snapshot_method_no_params_bool", content);
}

// ---------------------------------------------------------------------------
// 6. No-param method returning Vec<u8>
// ---------------------------------------------------------------------------

#[test]
fn snapshot_method_no_params_bytes() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // `fetch_blob` takes no params, returns Vec<u8>.
    assert!(
        content.contains("nativeDemoClientFetchBlob"),
        "nativeDemoClientFetchBlob must be emitted; got:\n{content}"
    );
    assert!(
        content.contains("-> jbyteArray"),
        "fetch_blob must return jbyteArray; got:\n{content}"
    );
    assert!(
        content.contains("env.byte_array_from_slice(&v)"),
        "fetch_blob must use byte_array_from_slice; got:\n{content}"
    );
    insta::assert_snapshot!("snapshot_method_no_params_bytes", content);
}

// ---------------------------------------------------------------------------
// 7. Streaming Start/Next/Free
// ---------------------------------------------------------------------------

#[test]
fn snapshot_streaming_start_next_free() {
    let api = make_demo_api();
    let config = make_demo_config_with_streaming();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // All three streaming symbols present.
    assert!(
        content.contains("nativeDemoClientStreamDataStart"),
        "Start shim missing; got:\n{content}"
    );
    assert!(
        content.contains("nativeDemoClientStreamDataNext"),
        "Next shim missing; got:\n{content}"
    );
    assert!(
        content.contains("nativeDemoClientStreamDataFree"),
        "Free shim missing; got:\n{content}"
    );

    // Start: returns jlong. jni 0.22+ uses EnvUnowned in extern signatures.
    assert!(
        content.contains("nativeDemoClientStreamDataStart(\n    mut env: EnvUnowned,\n    _class: JClass,\n    client_handle: jlong,\n    request_json: JString,\n) -> jlong"),
        "Start must have correct signature; got:\n{content}"
    );
    // Next: polls stream, returns jstring.
    assert!(
        content.contains("stream.next()"),
        "Next must poll stream.next(); got:\n{content}"
    );
    assert!(
        content.contains("serde_json::to_string(&chunk)"),
        "Next must serialize chunk; got:\n{content}"
    );
    // Free: drops the handle.
    assert!(
        content.contains("Box::from_raw(stream_handle as *mut"),
        "Free must Box::from_raw the handle; got:\n{content}"
    );

    insta::assert_snapshot!("snapshot_streaming_start_next_free", content);
}

// ---------------------------------------------------------------------------
// 8. Destructor
// ---------------------------------------------------------------------------

#[test]
fn snapshot_destructor() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("nativeFreeDemoClient"),
        "destructor must be emitted; got:\n{content}"
    );
    assert!(
        content.contains("Box::from_raw(handle as *mut core_crate::DemoClient)"),
        "destructor must drop via Box::from_raw; got:\n{content}"
    );
    insta::assert_snapshot!("snapshot_destructor", content);
}

// ---------------------------------------------------------------------------
// 9. Validation: kotlin_android required
// ---------------------------------------------------------------------------

#[test]
fn snapshot_validation_requires_kotlin_android() {
    let api = make_demo_api();
    // Build a config via kotlin_android + jni, then strip the kotlin_android field
    // to simulate a "jni only" scenario that the backend itself must reject.
    let mut config = make_demo_config();
    config.kotlin_android = None;

    let result = JniBackend.generate_bindings(&api, &config);
    assert!(result.is_err(), "must return Err when kotlin_android is missing");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("kotlin-android"),
        "error must mention 'kotlin-android'; got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// 10. No sample_llm leakage
// ---------------------------------------------------------------------------

#[test]
fn snapshot_no_sample_llm_leakage() {
    let api = make_demo_api();
    let config = make_demo_config_with_streaming();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    for forbidden in &["sample_llm", "SampleLlm", "samplellm"] {
        assert!(
            !content.contains(forbidden),
            "emitted output must not contain '{forbidden}'; got:\n{content}"
        );
    }
}

// ---------------------------------------------------------------------------
// Legacy non-snapshot tests (kept from original file)
// ---------------------------------------------------------------------------

/// Verify that every JNI symbol in the emitted output starts with `Java_` and
/// uses the package from `[crates.kotlin_android] package`.
#[test]
fn emitted_symbols_match_kotlin_package() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // Package `dev.sample_crate.demo` encodes as `dev_sample_1crate_demo`.
    assert!(
        content.contains("Java_dev_sample_1crate_demo_"),
        "symbols must use package prefix `dev_sample_1crate_demo_`; got:\n{content}"
    );
    // Bridge class `DemoBridge` appears after the package prefix.
    assert!(
        content.contains("DemoBridge"),
        "symbols must reference `DemoBridge`; got:\n{content}"
    );
}

/// Verify that the top-level `create_client` function emits a `nativeCreateClient` symbol.
#[test]
fn top_level_function_emits_native_prefix() {
    let api = make_demo_api();
    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("nativeCreateClient"),
        "top-level function must emit `nativeCreateClient` symbol; got:\n{content}"
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
/// `alef::core::jni` helpers would produce for the same inputs.
///
/// This test encodes the JNI spec §5.11.3 contract between the Rust shim
/// emitter and the Kotlin bridge object emitter. If either side changes the
/// naming function the test fails, preventing silent drift.
#[test]
fn jni_symbols_agree_with_alef_core_jni_helpers() {
    use alef::core::jni::{bridge_class_name, bridge_method_name, destructor_method_name, jni_symbol};

    let package = "dev.sample_crate.demo";
    let bridge = bridge_class_name("demo");
    assert_eq!(&bridge, "DemoBridge");

    // Top-level function symbol.
    let fn_method = bridge_method_name("", "create_client");
    let fn_sym = jni_symbol(package, &bridge, &fn_method);
    assert_eq!(fn_sym, "Java_dev_sample_1crate_demo_DemoBridge_nativeCreateClient");

    // Instance method symbol.
    let method = bridge_method_name("DemoClient", "ping");
    let method_sym = jni_symbol(package, &bridge, &method);
    assert_eq!(
        method_sym,
        "Java_dev_sample_1crate_demo_DemoBridge_nativeDemoClientPing"
    );

    // Destructor symbol.
    let dtor = destructor_method_name("DemoClient");
    let dtor_sym = jni_symbol(package, &bridge, &dtor);
    assert_eq!(dtor_sym, "Java_dev_sample_1crate_demo_DemoBridge_nativeFreeDemoClient");
}

/// Streaming adapter shims (Start/Next/Free) are emitted for a `Streaming`
/// adapter that has `owner_type = "DemoClient"`.
#[test]
fn streaming_adapter_shims_are_emitted() {
    let api = make_demo_api();
    let config = make_demo_config_with_streaming();

    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("nativeDemoClientStreamDataStart"),
        "Start shim must be emitted; got:\n{content}"
    );
    assert!(
        content.contains("nativeDemoClientStreamDataNext"),
        "Next shim must be emitted; got:\n{content}"
    );
    assert!(
        content.contains("nativeDemoClientStreamDataFree"),
        "Free shim must be emitted; got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// 11. Real-IR-shape test: Optional<String>, &str, Result, async
// ---------------------------------------------------------------------------

/// Verifies the emitter handles sample-app-like IR shapes:
///   - Optional<String> params → `Some(name)` at the call site
///   - `&str` params (is_ref=true, String ty) → `&name` at the call site
///   - functions with error_type → `match result { Ok(v) => ..., Err(e) => ... }`
///   - async top-level functions → `runtime().block_on(...)`
///   - `use core_crate::*;` in the import block
#[test]
fn real_ir_shape_optional_ref_result_async() {
    // Build an API surface resembling a sample app's public surface.
    let client_type = TypeDef {
        name: "DemoClient".to_string(),
        rust_path: "demo::DemoClient".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![make_method(
            "chat_stream",
            vec![make_param("request", TypeRef::Named("ChatRequest".to_string()))],
            TypeRef::Named("ChatResponse".to_string()),
            true, // async
        )],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    // create_client(api_key: Option<String>, base_url: Option<String>, timeout_secs: Option<u64>,
    //               max_retries: Option<u32>, model_hint: Option<String>) -> DemoClient
    let create_client = FunctionDef {
        name: "create_client".to_string(),
        rust_path: "demo::create_client".to_string(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
                name: "api_key".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
                optional: true,
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
            },
            ParamDef {
                name: "base_url".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
                optional: true,
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
            },
            ParamDef {
                name: "timeout_secs".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
                optional: true,
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
            },
        ],
        return_type: TypeRef::Named("DemoClient".to_string()),
        is_async: false,
        error_type: Some("DemoError".to_string()),
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
    };

    // unregister_custom_provider(name: &str) -> bool
    let unregister_fn = FunctionDef {
        name: "unregister_custom_provider".to_string(),
        rust_path: "demo::unregister_custom_provider".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true, // &str in core
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: alef::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Primitive(PrimitiveType::Bool),
        is_async: false,
        error_type: Some("DemoError".to_string()),
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
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![create_client, unregister_fn],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".to_string(),
            rust_path: "demo::DemoError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Fail".to_string(),
                message_template: Some("fail".to_string()),
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // Fix 1: glob import present.
    assert!(
        content.contains("use core_crate::*;"),
        "must contain `use core_crate::*;`; got:\n{content}"
    );

    // Fix 2: Optional<String> params wrapped with Some().
    assert!(
        content.contains("Some(api_key)") && content.contains("Some(base_url)"),
        "Optional<String> params must be wrapped with Some(...); got:\n{content}"
    );

    // Fix 2b: Optional<u64> wrapped with Some().
    assert!(
        content.contains("Some(timeout_secs as u64)"),
        "Optional<u64> param must be wrapped with Some(timeout_secs as u64); got:\n{content}"
    );

    // Fix 3: &str params passed with & reference.
    assert!(
        content.contains("&name"),
        "`name` (is_ref=true, String) must be passed as `&name`; got:\n{content}"
    );

    // Fix 2+3 combined: Result-bearing functions use match result.
    assert!(
        content.contains("match result {"),
        "Result-bearing functions must emit 'match result {{'; got:\n{content}"
    );

    // The create_client call must pass Some(...) for optional params.
    let create_section = extract_fn_section(content, "nativeCreateClient");
    assert!(
        create_section.contains("Some(api_key)"),
        "createClient must wrap api_key with Some(); section:\n{create_section}"
    );
    assert!(
        create_section.contains("Some(base_url)"),
        "createClient must wrap base_url with Some(); section:\n{create_section}"
    );

    // The unregister call must pass &name.
    let unreg_section = extract_fn_section(content, "nativeUnregisterCustomProvider");
    assert!(
        unreg_section.contains("&name"),
        "unregisterCustomProvider must pass &name; section:\n{unreg_section}"
    );
    assert!(
        unreg_section.contains("match result {"),
        "unregisterCustomProvider (has error_type) must match result; section:\n{unreg_section}"
    );
}

// ---------------------------------------------------------------------------
// R1: &mut self receiver uses *mut T cast
// ---------------------------------------------------------------------------

/// Verifies that a method with `receiver = Some(ReceiverKind::RefMut)` emits
/// `&mut *(handle as *mut T)` instead of `&*(handle as *const T)`.
#[test]
fn method_ref_mut_receiver_emits_mut_cast() {
    let mut_method = MethodDef {
        name: "set_language".to_string(),
        params: vec![make_param("language", TypeRef::Named("Language".to_string()))],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        receiver: Some(ReceiverKind::RefMut),
        error_type: Some("ParserError".to_string()),
        doc: String::new(),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let client_type = TypeDef {
        name: "Parser".to_string(),
        rust_path: "demo::Parser".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![mut_method],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    let section = extract_fn_section(content, "nativeParserSetLanguage");
    // Must dereference as *mut, not *const.
    assert!(
        section.contains("&mut *(handle as *mut core_crate::Parser)"),
        "&mut self method must cast to *mut; section:\n{section}"
    );
    assert!(
        !section.contains("*const core_crate::Parser"),
        "&mut self method must NOT use *const; section:\n{section}"
    );
}

/// Verifies that a method with `receiver = Some(ReceiverKind::Ref)` (or `None`)
/// still emits `&*(handle as *const T)`.
#[test]
fn method_ref_receiver_emits_const_cast() {
    let ref_method = MethodDef {
        name: "kind".to_string(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        receiver: Some(ReceiverKind::Ref),
        error_type: None,
        doc: String::new(),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let client_type = TypeDef {
        name: "Node".to_string(),
        rust_path: "demo::Node".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![ref_method],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    let section = extract_fn_section(content, "nativeNodeKind");
    assert!(
        section.contains("&*(handle as *const core_crate::Node)"),
        "&self method must cast to *const; section:\n{section}"
    );
}

// ---------------------------------------------------------------------------
// R2: &[u8] / Vec<u8> / PathBuf params marshalled correctly (not as JSON)
// ---------------------------------------------------------------------------

/// Verifies that a method taking `source: Vec<u8>` (is_ref=true, so `&[u8]`)
/// receives a `jbyteArray` and uses `env.convert_byte_array`, not JSON.
#[test]
fn method_slice_u8_param_receives_jbytearray() {
    let parse_method = MethodDef {
        name: "parse_bytes".to_string(),
        params: vec![ParamDef {
            name: "source".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8))),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: alef::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Optional(Box::new(TypeRef::Named("Tree".to_string()))),
        is_async: false,
        is_static: false,
        receiver: Some(ReceiverKind::RefMut),
        error_type: None,
        doc: String::new(),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let client_type = TypeDef {
        name: "Parser".to_string(),
        rust_path: "demo::Parser".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![parse_method],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    let section = extract_fn_section(content, "nativeParserParseBytes");
    assert!(
        section.contains("source: jbyteArray"),
        "Vec<u8> param must be jbyteArray, not JString; section:\n{section}"
    );
    assert!(
        section.contains("env.convert_byte_array("),
        "Vec<u8> param must use env.convert_byte_array; section:\n{section}"
    );
    assert!(
        !section.contains("serde_json::from_str"),
        "Vec<u8> param must NOT use serde_json::from_str; section:\n{section}"
    );
}

#[test]
fn method_optional_bytes_param_and_return_use_jbytearray_nullability() {
    let payload = ParamDef {
        name: "payload".to_string(),
        ty: TypeRef::Bytes,
        optional: true,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
    };
    let upload_method = MethodDef {
        name: "upload".to_string(),
        params: vec![payload],
        return_type: TypeRef::Optional(Box::new(TypeRef::Bytes)),
        is_async: false,
        is_static: false,
        receiver: Some(ReceiverKind::Ref),
        error_type: None,
        doc: String::new(),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let client_type = TypeDef {
        name: "Parser".to_string(),
        rust_path: "demo::Parser".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![upload_method],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;
    let section = extract_fn_section(content, "nativeParserUpload");

    assert!(
        section.contains("payload: jbyteArray") && section.contains("-> jbyteArray"),
        "optional bytes param/return must use jbyteArray, section:\n{section}"
    );
    assert!(
        section.contains("Ok(0) => None"),
        "empty ByteArray sentinel must decode to None, section:\n{section}"
    );
    assert!(
        section.contains("None => std::ptr::null_mut()"),
        "optional bytes return None must map to null jbyteArray, section:\n{section}"
    );
    assert!(
        !section.contains("serde_json::from_str") && !section.contains("serde_json::to_string"),
        "optional bytes direct JNI path must not JSON round-trip, section:\n{section}"
    );
}

/// Verifies that a method taking `dir: PathBuf` receives a JString and
/// constructs `std::path::PathBuf::from(...)` without JSON decoding.
#[test]
fn method_pathbuf_param_receives_raw_string() {
    let add_dir_method = MethodDef {
        name: "add_extra_libs_dir".to_string(),
        params: vec![ParamDef {
            name: "dir".to_string(),
            ty: TypeRef::Path,
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
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        receiver: Some(ReceiverKind::RefMut),
        error_type: Some("RegistryError".to_string()),
        doc: String::new(),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let client_type = TypeDef {
        name: "LanguageRegistry".to_string(),
        rust_path: "demo::LanguageRegistry".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![add_dir_method],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    let section = extract_fn_section(content, "nativeLanguageRegistryAddExtraLibsDir");
    // PathBuf single-param still uses request_json: JString (raw string, not bytes).
    assert!(
        section.contains("request_json: JString"),
        "PathBuf param method must have request_json: JString; section:\n{section}"
    );
    assert!(
        section.contains("std::path::PathBuf::from("),
        "PathBuf param must use std::path::PathBuf::from(string); section:\n{section}"
    );
    assert!(
        !section.contains("serde_json::from_str"),
        "PathBuf param must NOT use serde_json::from_str; section:\n{section}"
    );
}

// ---------------------------------------------------------------------------
// R4: &[&str] params coerce from Vec<String>
// ---------------------------------------------------------------------------

/// Verifies that a method taking `names: &[&str]` (TypeRef::Vec(String) with
/// is_ref=true) deserializes JSON into `Vec<String>` and then collects
/// `Vec<&str>` refs before passing `&names_refs` to the core method.
#[test]
fn method_slice_str_param_coerces_to_str_refs() {
    let lookup_method = MethodDef {
        name: "set_included_ranges".to_string(),
        params: vec![ParamDef {
            name: "names".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::String)),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: true,
            map_is_btree: false,
            core_wrapper: alef::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        receiver: Some(ReceiverKind::RefMut),
        error_type: Some("ParseError".to_string()),
        doc: String::new(),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let client_type = TypeDef {
        name: "Parser".to_string(),
        rust_path: "demo::Parser".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![lookup_method],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    let section = extract_fn_section(content, "nativeParserSetIncludedRanges");
    // Must deserialize to Vec<String> first.
    assert!(
        section.contains("names_vec: Vec<String>"),
        "Vec<String> (is_ref) must deserialize to names_vec: Vec<String>; section:\n{section}"
    );
    // Must collect refs.
    assert!(
        section.contains("names_refs: Vec<&str>"),
        "must collect names_refs: Vec<&str>; section:\n{section}"
    );
    // Call site must pass &names_refs.
    assert!(
        section.contains("&names_refs"),
        "call site must pass &names_refs; section:\n{section}"
    );
}

// ---------------------------------------------------------------------------
// Regression: streaming handle type aliases prevent clippy::type_complexity
// ---------------------------------------------------------------------------

/// Regression: the streaming handle struct field must not inline the full
/// `Mutex<Option<BoxStream<'static, Result<T, Box<dyn Error + Send + Sync + 'static>>>>>>`
/// type directly — that 6-level nesting triggers `clippy::type_complexity` under
/// `-D warnings`. Instead, the emitter must emit two type aliases:
///   - `<Handle>Item = Result<ItemType, Box<dyn Error + ...>>`
///   - `<Handle>Stream = BoxStream<'static, <Handle>Item>`
///
/// The struct field then only references `Mutex<Option<<Handle>Stream>>`,
/// which stays within clippy's complexity threshold.
#[test]
fn streaming_handle_struct_uses_type_aliases_to_avoid_type_complexity() {
    let api = make_demo_api();
    let config = make_demo_config_with_streaming();

    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // The two type aliases must appear in the output.
    assert!(
        content.contains("type DemoClientStreamDataStreamHandleItem"),
        "Item alias must be emitted to reduce struct complexity;\ngot:\n{content}"
    );
    assert!(
        content.contains("type DemoClientStreamDataStreamHandleStream"),
        "Stream alias must be emitted to reduce struct complexity;\ngot:\n{content}"
    );

    // The struct field must reference the alias, NOT the inline nested type.
    assert!(
        content.contains("stream: Mutex<Option<DemoClientStreamDataStreamHandleStream>>"),
        "struct field must use the short alias type, not the inlined nested form;\ngot:\n{content}"
    );

    // The fully-inlined form that triggers clippy::type_complexity must NOT appear.
    assert!(
        !content.contains("stream: Mutex<Option<BoxStream<'static, std::result::Result<core_crate::DataChunk, Box<dyn"),
        "struct field must not inline the full nested type (would trigger clippy::type_complexity);\ngot:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// C1: no unwrap_or_default() on JSON serialization paths
// ---------------------------------------------------------------------------

/// Regression: the emitted JNI shim must NOT use `.unwrap_or_default()` on
/// any `serde_json::to_string(...)` call.  Silent serialization failures
/// previously caused Kotlin to receive an empty string and throw
/// `SampleLlmBridgeException at SampleLlmBridge.kt:-2`.  Every serialization
/// failure must route through `throw_jni_error` with the actual message.
#[test]
fn no_unwrap_or_default_on_json_serialization_path() {
    let api = make_demo_api();
    let config = make_demo_config_with_streaming();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // The only `unwrap_or_default` that may appear is on the `exclude_functions`
    // HashSet collection — NOT on any serialization path.  Verify none appear
    // adjacent to `serde_json::to_string`.
    for line in content.lines() {
        if line.contains("unwrap_or_default") {
            assert!(
                !line.contains("serde_json::to_string"),
                "serde_json::to_string must not use .unwrap_or_default(); found:\n{line}"
            );
        }
        if line.contains("serde_json::to_string") {
            assert!(
                !line.contains("unwrap_or_default"),
                "serde_json::to_string must not use .unwrap_or_default(); found:\n{line}"
            );
        }
    }

    // Confirm that explicit error handling IS present instead.
    assert!(
        content.contains("serialize: {e}"),
        "serialization errors must propagate via throw_jni_error with message; got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// Helper: extract the function body section for a named symbol
// ---------------------------------------------------------------------------

/// Extract the text from just before the `fn <sym>` line through the closing `}`.
fn extract_fn_section(content: &str, symbol: &str) -> String {
    let start = content.find(symbol).unwrap_or(0);
    // Walk forward to find the final closing brace of this function.
    let rest = &content[start..];
    let mut depth = 0usize;
    let mut end = rest.len();
    for (i, c) in rest.char_indices() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    rest[..end].to_string()
}

// ---------------------------------------------------------------------------
// Panic-safety: run_or_throw helper and block_on wrapping
// ---------------------------------------------------------------------------

#[test]
fn panic_safety_run_or_throw_replaces_bare_block_on() {
    let api = make_demo_api();
    let config = make_demo_config_with_streaming();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("fn run_or_throw"),
        "run_or_throw helper must be emitted"
    );
    assert!(
        content.contains("std::panic::catch_unwind"),
        "run_or_throw must use catch_unwind"
    );
    assert!(
        content.contains("native panic:"),
        "run_or_throw must prefix with 'native panic:'"
    );

    let bare_count = content.matches("= runtime().block_on(").count();
    assert_eq!(bare_count, 0, "no bare block_on must remain; found {bare_count}");

    let ctor = extract_fn_section(content, "nativeCreateClient");
    assert!(ctor.contains("run_or_throw"), "constructor must use run_or_throw");
    assert!(ctor.contains("return 0"), "constructor must return 0 sentinel on panic");

    let do_thing = extract_fn_section(content, "nativeDemoClientDoThing");
    assert!(do_thing.contains("run_or_throw"), "do_thing must use run_or_throw");

    let start = extract_fn_section(content, "nativeDemoClientStreamDataStart");
    assert!(start.contains("run_or_throw"), "streaming Start must use run_or_throw");

    let next = extract_fn_section(content, "nativeDemoClientStreamDataNext");
    assert!(next.contains("run_or_throw"), "streaming Next must use run_or_throw");
}

// ---------------------------------------------------------------------------
// client_constructors: nativeNew<TypeName> shim
// ---------------------------------------------------------------------------

/// When `client_constructors` contains an entry for an opaque type,
/// `emit_lib_rs` must emit a `nativeNew<TypeName>` shim that:
/// - receives each `*const c_char` param as `JString`
/// - unmarshals via `jstring_to_string`
/// - calls the configured body
/// - returns `jlong` (Box::into_raw on success) or `0` on error
#[test]
fn client_constructors_emits_native_new_shim() {
    let api = make_demo_api();
    let mut config = make_demo_config();
    config.client_constructors.insert(
        "DemoClient".to_string(),
        ClientConstructorConfig {
            params: vec![ConstructorParam {
                name: "api_key".to_string(),
                ty: "*const std::ffi::c_char".to_string(),
            }],
            body: "{source_path}::new(api_key)".to_string(),
            error_type: Some("DemoError".to_string()),
        },
    );

    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // The constructor shim symbol must use nativeNew<TypeName>.
    assert!(
        content.contains("nativeNewDemoClient"),
        "constructor shim must use nativeNewDemoClient; got:\n{content}"
    );

    let section = extract_fn_section(content, "nativeNewDemoClient");

    // Param must be JString (c_char maps to string).
    assert!(
        section.contains("api_key: JString"),
        "c_char param must receive JString; section:\n{section}"
    );

    // Must unmarshal via jstring_to_string.
    assert!(
        section.contains("jstring_to_string(env, api_key)"),
        "must unmarshal api_key via jstring_to_string; section:\n{section}"
    );

    // Must return jlong.
    assert!(
        section.contains("-> jlong"),
        "constructor shim must return jlong; section:\n{section}"
    );

    // Must box the result.
    assert!(
        section.contains("Box::into_raw(Box::new(v)) as jlong"),
        "constructor must Box::into_raw result; section:\n{section}"
    );

    // Must return 0 on error.
    assert!(
        section.contains("throw_jni_error"),
        "constructor must call throw_jni_error on failure; section:\n{section}"
    );
}

// ---------------------------------------------------------------------------
// Trait-bridge shim emission
// ---------------------------------------------------------------------------

fn trait_bridge_config(exclude_languages: &[&str]) -> ResolvedCrateConfig {
    let exclude_array = if exclude_languages.is_empty() {
        String::new()
    } else {
        format!(
            "\nexclude_languages = [{}]",
            exclude_languages
                .iter()
                .map(|l| format!("\"{l}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let toml = format!(
        r#"
[workspace]
languages = ["kotlin_android", "jni", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"

[[crates.trait_bridges]]
trait_name = "TextBackend"
super_trait = "demo::Plugin"
registry_getter = "demo::get_text_registry"
register_fn = "register_text_backend"
unregister_fn = "unregister_text_backend"
clear_fn = "clear_text_backends"{exclude_array}
"#,
    );
    resolved_one(&toml)
}

fn empty_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    }
}

/// For every `[[crates.trait_bridges]]` entry the JNI backend must emit
/// `Java_*_nativeRegister<Trait>`, `Java_*_nativeUnregister<Trait>`, and
/// `Java_*_nativeClear<Trait>s` extern functions paired with the Kotlin
/// declarations.
#[test]
fn trait_bridge_emits_jni_shim_symbols() {
    let api = empty_api();
    let config = trait_bridge_config(&[]);
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 1, "JNI backend emits a single lib.rs");
    let content = &files[0].content;

    assert!(
        content.contains("pub unsafe extern \"system\" fn Java_dev_sample_1crate_DemoBridge_nativeRegisterTextBackend"),
        "missing nativeRegisterTextBackend extern fn: {content}"
    );
    assert!(
        content
            .contains("pub unsafe extern \"system\" fn Java_dev_sample_1crate_DemoBridge_nativeUnregisterTextBackend"),
        "missing nativeUnregisterTextBackend extern fn: {content}"
    );
    assert!(
        content.contains("pub unsafe extern \"system\" fn Java_dev_sample_1crate_DemoBridge_nativeClearTextBackends"),
        "missing nativeClearTextBackends extern fn: {content}"
    );

    // Unregister wires through to the host-configured function.
    assert!(
        content.contains("core_crate::unregister_text_backend(&name)"),
        "unregister shim must call host unregister_fn: {content}"
    );
    // Clear wires through to the host-configured function.
    assert!(
        content.contains("core_crate::clear_text_backends"),
        "clear shim must call host clear_fn: {content}"
    );
}

/// Trait bridges with `kotlin_android` in `exclude_languages` must be omitted
/// from the JNI shim output entirely.
#[test]
fn trait_bridge_exclude_languages_suppresses_jni_shim() {
    let api = empty_api();
    let config = trait_bridge_config(&["kotlin_android"]);
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;
    assert!(
        !content.contains("nativeRegisterTextBackend"),
        "excluded trait bridge must not emit register shim: {content}"
    );
    assert!(
        !content.contains("nativeUnregisterTextBackend"),
        "excluded trait bridge must not emit unregister shim: {content}"
    );
    assert!(
        !content.contains("nativeClearTextBackends"),
        "excluded trait bridge must not emit clear shim: {content}"
    );
}

// ---------------------------------------------------------------------------
// Bug 2 regression — trait-method `use` clauses
// ---------------------------------------------------------------------------

/// When a type has a method whose `trait_source` points to a sub-module trait
/// (one that isn't re-exported at the crate root), the JNI emitted lib.rs MUST
/// emit a `use <full_trait_path>;` line so the method call resolves. The
/// lib_header's blanket `use core_crate::*;` only covers crate-root items.
///
/// Without this, the emitted `client.method(...)` fails to compile with
/// `no method named X found for reference &T`.
#[test]
fn lib_rs_emits_use_clauses_for_trait_method_paths() {
    let mut method = make_method("fetch_batch_for_polling", vec![], TypeRef::String, true);
    method.trait_source = Some("demo::client::BatchRetriever".to_string());
    method.receiver = Some(ReceiverKind::Ref);

    let client_type = TypeDef {
        name: "DefaultClient".to_string(),
        rust_path: "demo::DefaultClient".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![method],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("use demo::client::BatchRetriever;"),
        "lib.rs must emit explicit `use` for the trait path; got:\n{content}"
    );
    assert!(
        content.contains("fetch_batch_for_polling"),
        "lib.rs must still emit the method shim; got:\n{content}"
    );
}

/// When two traits share the same final segment (e.g. `a::Dependency` and
/// `b::Dependency`), `collect_trait_imports` deduplicates by last segment and
/// keeps the shortest path. The JNI emitter must therefore not emit duplicate
/// `use` clauses that would trip Rust's E0252.
#[test]
fn lib_rs_dedupes_trait_paths_by_last_segment() {
    let mut method_a = make_method("fetch_a", vec![], TypeRef::String, false);
    method_a.trait_source = Some("demo::short::Dependency".to_string());
    method_a.receiver = Some(ReceiverKind::Ref);

    let mut method_b = make_method("fetch_b", vec![], TypeRef::String, false);
    method_b.trait_source = Some("demo::very::long::path::Dependency".to_string());
    method_b.receiver = Some(ReceiverKind::Ref);

    let client_type = TypeDef {
        name: "DefaultClient".to_string(),
        rust_path: "demo::DefaultClient".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![method_a, method_b],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_demo_config();
    let files = JniBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // Exactly one `use` line should reference Dependency (the shorter path wins).
    let count = content.matches("::Dependency;").count();
    assert_eq!(
        count, 1,
        "expected exactly one Dependency import, got {count}; lib.rs:\n{content}"
    );
    assert!(
        content.contains("use demo::short::Dependency;"),
        "shorter path must be picked; got:\n{content}"
    );
}
