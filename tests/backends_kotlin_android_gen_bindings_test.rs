use alef::backends::kotlin_android::KotlinAndroidBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};
use std::borrow::Cow;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let toml = if toml.contains("[crates.package_metadata]") {
        Cow::Borrowed(toml)
    } else {
        Cow::Owned(format!(
            r#"{toml}

[crates.package_metadata]
repository = "https://github.com/example/demo"
license = "MIT"
"#
        ))
    };
    let cfg: NewAlefConfig = toml::from_str(&toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_streaming_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.sample_crate"

[crates.kotlin_android]
package = "dev.sample_crate.demo.android"
namespace = "dev.sample_crate.demo.android"
artifact_id = "demo-android"
group_id = "dev.sample_crate"

[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "chat_stream"
owner_type = "DefaultClient"
item_type = "ChatCompletionChunk"
error_type = "DemoError"
request_type = "demo::ChatCompletionRequest"

[[crates.adapters.params]]
name = "req"
type = "ChatCompletionRequest"
"#,
    )
}

fn make_trait_carrier_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.sample_crate"

[crates.kotlin_android]
package = "dev.sample_crate.demo.android"
namespace = "dev.sample_crate.demo.android"
artifact_id = "demo-android"
group_id = "dev.sample_crate"

[[crates.trait_bridges]]
trait_name = "Renderer"
type_alias = "RendererHandle"
param_name = "renderer"
context_type = "HiddenCarrier"
result_type = "PublicCarrier"
"#,
    )
}

fn make_streaming_api() -> ApiSurface {
    let chat_method = MethodDef {
        name: "chat".into(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: true,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: None,
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
        name: "DefaultClient".into(),
        rust_path: "demo::DefaultClient".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![chat_method],
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
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_trait_carrier_api() -> ApiSurface {
    let render_method = MethodDef {
        name: "render".into(),
        params: vec![ParamDef {
            name: "carrier".into(),
            ty: TypeRef::Named("HiddenCarrier".into()),
            optional: false,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("HiddenCarrier".into()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: None,
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
    let trait_type = TypeDef {
        name: "Renderer".into(),
        rust_path: "demo::Renderer".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![render_method],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: true,
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
    let public_carrier = TypeDef {
        name: "PublicCarrier".into(),
        rust_path: "demo::PublicCarrier".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: true,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let mut excluded_type_paths = ::std::collections::HashMap::new();
    excluded_type_paths.insert("HiddenCarrier".to_string(), "demo::HiddenCarrier".to_string());

    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type, public_carrier],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths,
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// `KotlinAndroidBackend` must advertise streaming support so downstream
/// consumers (e.g. sample apps) can rely on the `Flow<T>` surface.
#[test]
fn supports_streaming_capability_is_true() {
    assert!(
        KotlinAndroidBackend.capabilities().supports_streaming,
        "KotlinAndroidBackend must report supports_streaming = true"
    );
}

#[test]
fn trait_interface_projects_configured_excluded_carrier_to_public_result() {
    let api = make_trait_carrier_api();
    let config = make_trait_carrier_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let interface = files
        .iter()
        .find(|file| file.path.file_name().and_then(|name| name.to_str()) == Some("IRenderer.kt"))
        .expect("trait bridge interface should be emitted");
    let content = &interface.content;

    assert!(
        content.contains("fun render(carrier: PublicCarrier): PublicCarrier"),
        "{content}"
    );
    assert!(!content.contains("HiddenCarrier"), "{content}");
}

/// A streaming adapter owned by a client type must produce a `Flow<T>`
/// wrapper (via `callbackFlow`) in the generated `DefaultClient.kt`, and JNI
/// `external fun` declarations in the `DemoBridge.kt` object (not in any
/// Java file), when the kotlin-android backend generates bindings in JNI mode.
#[test]
fn streaming_adapter_emits_flow_wrapper_in_kotlin_android() {
    let api = make_streaming_api();
    let config = make_streaming_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    // No Java files must be emitted — the AAR is pure-Kotlin JNI.
    let java_files: Vec<_> = files
        .iter()
        .filter(|f| f.path.extension().and_then(|e| e.to_str()) == Some("java"))
        .collect();
    assert!(
        java_files.is_empty(),
        "kotlin-android must not emit Java files; got: {java_files:?}"
    );

    // DefaultClient.kt must contain the callbackFlow wrapper.
    let client_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("DefaultClient.kt"))
        .expect("DefaultClient.kt must be emitted when a client type has methods");

    let kt_content = &client_kt.content;
    assert!(
        kt_content.contains("fun chatStream("),
        "expected chatStream method on DefaultClient.kt: {kt_content}"
    );
    assert!(
        kt_content.contains("Flow<ChatCompletionChunk>"),
        "expected Flow<ChatCompletionChunk> return type: {kt_content}"
    );
    assert!(
        kt_content.contains("callbackFlow"),
        "expected callbackFlow wrapper: {kt_content}"
    );
    assert!(
        kt_content.contains("nativeDefaultClientChatStreamStart("),
        "expected nativeDefaultClientChatStreamStart: {kt_content}"
    );
    assert!(
        kt_content.contains("nativeDefaultClientChatStreamNext("),
        "expected nativeDefaultClientChatStreamNext: {kt_content}"
    );
    assert!(
        kt_content.contains("nativeDefaultClientChatStreamFree("),
        "expected nativeDefaultClientChatStreamFree: {kt_content}"
    );
    assert!(
        kt_content.contains("awaitClose"),
        "expected awaitClose for stream handle cleanup: {kt_content}"
    );

    // DemoBridge.kt must contain the JNI external fun declarations (not Java).
    let bridge_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("DemoBridge.kt"))
        .expect("DemoBridge.kt must be emitted in JNI mode");

    let bridge_content = &bridge_kt.content;
    assert!(
        bridge_content.contains("external fun nativeDefaultClientChatStreamStart("),
        "expected nativeDefaultClientChatStreamStart external fun in DemoBridge.kt: {bridge_content}"
    );
    assert!(
        bridge_content.contains("external fun nativeDefaultClientChatStreamNext("),
        "expected nativeDefaultClientChatStreamNext external fun in DemoBridge.kt: {bridge_content}"
    );
    assert!(
        bridge_content.contains("external fun nativeDefaultClientChatStreamFree("),
        "expected nativeDefaultClientChatStreamFree external fun in DemoBridge.kt: {bridge_content}"
    );

    // Snapshot the DefaultClient.kt so regressions are caught.
    insta::assert_snapshot!("streaming_flow_default_client_kt", kt_content);
}

// ---------------------------------------------------------------------------
// Bug 1 regression: wrapper facade `createClient` must return `DefaultClient`
// ---------------------------------------------------------------------------

fn make_opaque_factory_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.sample_crate"

[crates.kotlin_android]
package = "dev.sample_crate.demo.android"
namespace = "dev.sample_crate.demo.android"
artifact_id = "demo-android"
group_id = "dev.sample_crate"
"#,
    )
}

fn make_opaque_factory_api() -> ApiSurface {
    // A top-level `create_client(api_key: String) -> DefaultClient` function where
    // DefaultClient is an opaque type with at least one instance method.
    let chat_method = MethodDef {
        name: "chat".into(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: true,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: None,
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
        name: "DefaultClient".into(),
        rust_path: "demo::DefaultClient".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![chat_method],
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
    let create_client_fn = FunctionDef {
        name: "create_client".into(),
        rust_path: "demo::create_client".into(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "api_key".into(),
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
        return_type: TypeRef::Named("DefaultClient".into()),
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
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![create_client_fn],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// Regression test for Bug 1: the `Demo.kt` wrapper facade must emit
/// `fun createClient(...): DefaultClient = DefaultClient(DemoBridge.nativeCreateClient(...))`
/// rather than `fun createClient(...): String = DemoBridge.nativeCreateClient(...)`.
///
/// The JNI Bridge emits `external fun nativeCreateClient(...): Long` for opaque
/// handle return types; the wrapper must construct the concrete `DefaultClient`
/// from that raw `Long` handle.
#[test]
fn module_kt_create_client_returns_default_client_not_string() {
    let api = make_opaque_factory_api();
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted when visible free functions exist");

    let content = &module_kt.content;

    // Must return DefaultClient, not String.
    assert!(
        content.contains("): DefaultClient ="),
        "createClient must return DefaultClient, got:\n{content}"
    );
    assert!(
        !content.contains("): String =") || !content.contains("createClient"),
        "createClient must NOT return String, got:\n{content}"
    );
    // Must construct DefaultClient from the bridge call.
    assert!(
        content.contains("DefaultClient(DemoBridge.nativeCreateClient("),
        "must wrap bridge call in DefaultClient(...) constructor, got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// Bug 2 regression: handle-only opaque type (sample_crawler shape) — top-level
// fns taking and returning an opaque handle that has NO instance methods.
// ---------------------------------------------------------------------------

/// Build an API with the sample_crawler shape:
/// - `CrawlEngineHandle` is an opaque type with NO instance methods.
/// - `create_engine() -> CrawlEngineHandle` is a top-level fn returning the handle.
/// - `scrape(engine: &CrawlEngineHandle, url: &str) -> String` takes the handle.
fn make_handle_only_api() -> ApiSurface {
    let engine_type = TypeDef {
        name: "CrawlEngineHandle".into(),
        rust_path: "demo::CrawlEngineHandle".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![], // No instance methods — handle-only shape.
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
    let create_engine_fn = FunctionDef {
        name: "create_engine".into(),
        rust_path: "demo::create_engine".into(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::Named("CrawlEngineHandle".into()),
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
    };
    let scrape_fn = FunctionDef {
        name: "scrape".into(),
        rust_path: "demo::scrape".into(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
                name: "engine".into(),
                ty: TypeRef::Named("CrawlEngineHandle".into()),
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
            },
            ParamDef {
                name: "url".into(),
                ty: TypeRef::String,
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
            },
        ],
        return_type: TypeRef::String,
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
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![engine_type],
        functions: vec![create_engine_fn, scrape_fn],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// Regression test for the sample_crawler-shape facade bug: when a top-level fn
/// returns an opaque handle type with NO instance methods, the facade must
/// return the wrapper class (not `String`), and fns taking the handle as a
/// param must accept the wrapper class (not `String`).  A separate
/// `<TypeName>.kt` wrapper class file must be emitted with `AutoCloseable`
/// and `close()` calling the bridge's `nativeFree<TypeName>` destructor.
#[test]
fn handle_only_opaque_returns_wrapper_class_and_accepts_wrapper_params() {
    let api = make_handle_only_api();
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    // The facade Demo.kt — must use the wrapper class everywhere.
    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted when visible free functions exist");
    let module_content = &module_kt.content;

    assert!(
        module_content.contains("fun createEngine(): CrawlEngineHandle ="),
        "createEngine must return CrawlEngineHandle, got:\n{module_content}"
    );
    assert!(
        module_content.contains("CrawlEngineHandle(DemoBridge.nativeCreateEngine("),
        "createEngine must wrap bridge call in CrawlEngineHandle(...), got:\n{module_content}"
    );
    assert!(
        module_content.contains("fun scrape(engine: CrawlEngineHandle, url: String): String ="),
        "scrape must accept CrawlEngineHandle for engine, got:\n{module_content}"
    );
    assert!(
        module_content.contains("DemoBridge.nativeScrape(engine.handle, url)"),
        "scrape must unpack engine.handle into the bridge call, got:\n{module_content}"
    );
    assert!(
        !module_content.contains("engine: String"),
        "engine param must NOT be String, got:\n{module_content}"
    );

    // The wrapper class file CrawlEngineHandle.kt — must be AutoCloseable
    // and call nativeFreeCrawlEngineHandle from close().
    let handle_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("CrawlEngineHandle.kt"))
        .expect("CrawlEngineHandle.kt wrapper must be emitted for handle-only opaque types");
    let handle_content = &handle_kt.content;

    assert!(
        handle_content
            .contains("class CrawlEngineHandle internal constructor(internal val handle: Long) : AutoCloseable"),
        "wrapper must implement AutoCloseable with internal val handle: Long, got:\n{handle_content}"
    );
    assert!(
        handle_content.contains("DemoBridge.nativeFreeCrawlEngineHandle(handle)"),
        "wrapper close() must call nativeFreeCrawlEngineHandle(handle), got:\n{handle_content}"
    );
}

// ---------------------------------------------------------------------------
// Bug 5 regression: optional params must get Kotlin default values so that
// e2e callers that only pass `apiKey` and `baseUrl` still compile.
// ---------------------------------------------------------------------------

fn make_optional_params_api() -> ApiSurface {
    // A top-level `create_client` with three optional params (the sample-app shape):
    //   create_client(api_key: String, base_url: String, timeout_secs: Option<u64>,
    //                 max_retries: Option<u32>, model_hint: Option<String>) -> DefaultClient
    let chat_method = MethodDef {
        name: "chat".into(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: true,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: None,
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
        name: "DefaultClient".into(),
        rust_path: "demo::DefaultClient".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![chat_method],
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
    use alef::core::ir::PrimitiveType;
    let create_client_fn = FunctionDef {
        name: "create_client".into(),
        rust_path: "demo::create_client".into(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
                name: "api_key".into(),
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
            },
            ParamDef {
                name: "base_url".into(),
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
            },
            // timeout_secs: Option<u64> → mapped as Long in JNI, optional=true
            ParamDef {
                name: "timeout_secs".into(),
                ty: TypeRef::Primitive(PrimitiveType::U64),
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
            // max_retries: Option<u32> → mapped as Int in JNI, optional=true
            ParamDef {
                name: "max_retries".into(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
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
            // model_hint: Option<String> → mapped as String in JNI, optional=true
            ParamDef {
                name: "model_hint".into(),
                ty: TypeRef::String,
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
        return_type: TypeRef::Named("DefaultClient".into()),
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
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![create_client_fn],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// Regression test: optional params in the facade must emit Kotlin default
/// values (`= 0L`, `= 0`, `= ""`) so that e2e callers that only pass
/// `apiKey` and `baseUrl` still compile.
#[test]
fn optional_params_get_kotlin_default_values_in_facade() {
    let api = make_optional_params_api();
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted when visible free functions exist");

    let content = &module_kt.content;

    // Required params must NOT have a default value.
    assert!(
        content.contains("apiKey: String,"),
        "apiKey must be required (no default), got:\n{content}"
    );
    assert!(
        content.contains("baseUrl: String,"),
        "baseUrl must be required (no default), got:\n{content}"
    );

    // Optional Long param must use nullable type with null default.
    assert!(
        content.contains("timeoutSecs: Long? = null"),
        "timeoutSecs must be nullable with null default, got:\n{content}"
    );

    // Optional Int param must use nullable type with null default.
    assert!(
        content.contains("maxRetries: Int? = null"),
        "maxRetries must be nullable with null default, got:\n{content}"
    );

    // Optional String param must use nullable type with null default.
    assert!(
        content.contains("modelHint: String? = null"),
        "modelHint must be nullable with null default, got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// Bug 6 regression: nullable primitive bridge args must null-coalesce to JNI
// zero values so the non-nullable `external fun` signature is satisfied.
// ---------------------------------------------------------------------------

fn make_nullable_primitives_api() -> ApiSurface {
    // A top-level free function with every nullable primitive scalar type:
    //   fn nullable_all(
    //       s: Option<String>,
    //       l: Option<i64>,
    //       i: Option<i32>,
    //       d: Option<f64>,
    //       b: Option<bool>,
    //   ) -> String
    use alef::core::ir::PrimitiveType;
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "nullable_all".into(),
            rust_path: "demo::nullable_all".into(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "s".into(),
                    ty: TypeRef::String,
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
                    name: "l".into(),
                    ty: TypeRef::Primitive(PrimitiveType::I64),
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
                    name: "i".into(),
                    ty: TypeRef::Primitive(PrimitiveType::I32),
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
                    name: "d".into(),
                    ty: TypeRef::Primitive(PrimitiveType::F64),
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
                    name: "b".into(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
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
            return_type: TypeRef::String,
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// Regression test for Bug 6: nullable primitive scalar / String params in the
/// facade bridge call must null-coalesce to the JNI zero-value (`?: ""`,
/// `?: 0L`, `?: 0`, `?: 0.0`, `?: false`) so the non-nullable `external fun`
/// signature is satisfied.  Without the fix these are passed as bare `name`,
/// causing a Kotlin type-mismatch compile error at the call site.
#[test]
fn nullable_primitive_bridge_args_null_coalesce_to_jni_defaults() {
    let api = make_nullable_primitives_api();
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted when visible free functions exist");

    let content = &module_kt.content;

    // The bridge call must null-coalesce each nullable param to its JNI zero value.
    assert!(
        content.contains("s ?: \"\""),
        "nullable String param must coalesce to \\\"\\\": got:\n{content}"
    );
    assert!(
        content.contains("l ?: 0L"),
        "nullable Long param must coalesce to 0L, got:\n{content}"
    );
    assert!(
        content.contains("i ?: 0"),
        "nullable Int param must coalesce to 0, got:\n{content}"
    );
    assert!(
        content.contains("d ?: 0.0"),
        "nullable Double param must coalesce to 0.0, got:\n{content}"
    );
    assert!(
        content.contains("b ?: false"),
        "nullable Boolean param must coalesce to false, got:\n{content}"
    );

    // The facade signature must still use nullable types with null defaults.
    assert!(
        content.contains("s: String? = null"),
        "s must be String? = null in facade signature, got:\n{content}"
    );
    assert!(
        content.contains("l: Long? = null"),
        "l must be Long? = null in facade signature, got:\n{content}"
    );
    assert!(
        content.contains("i: Int? = null"),
        "i must be Int? = null in facade signature, got:\n{content}"
    );
    assert!(
        content.contains("d: Double? = null"),
        "d must be Double? = null in facade signature, got:\n{content}"
    );
    assert!(
        content.contains("b: Boolean? = null"),
        "b must be Boolean? = null in facade signature, got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// Feature: payload-derived sealed variant param names
// ---------------------------------------------------------------------------

fn make_sealed_variants_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.sample_crate"

[crates.kotlin_android]
package = "dev.sample_crate.demo.android"
namespace = "dev.sample_crate.demo.android"
artifact_id = "demo-android"
group_id = "dev.sample_crate"
"#,
    )
}

use alef::core::ir::{CoreWrapper, EnumDef, EnumVariant, FieldDef, PrimitiveType};

fn make_sealed_field(name: &str, ty: TypeRef) -> FieldDef {
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

fn make_sealed_variant(name: &str, fields: Vec<FieldDef>, is_tuple: bool) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        is_tuple,
        binding_excluded: false,
        binding_exclusion_reason: None,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    }
}

fn make_sealed_variants_api() -> ApiSurface {
    // Create an enum with tuple variants having different payload types:
    // - Pdf(PdfMetadata): named type, derives "metadata"
    // - Custom(String): primitive type, derives "value"
    // - Multi(String, Int): multiple primitives, derives "value0", "value1"
    let format_metadata_enum = EnumDef {
        name: "FormatMetadata".into(),
        rust_path: "demo::FormatMetadata".into(),
        original_rust_path: "demo::FormatMetadata".into(),
        variants: vec![
            make_sealed_variant(
                "Pdf",
                vec![make_sealed_field("_0", TypeRef::Named("PdfMetadata".into()))],
                true,
            ),
            make_sealed_variant("Custom", vec![make_sealed_field("_0", TypeRef::String)], true),
            make_sealed_variant(
                "Multi",
                vec![
                    make_sealed_field("_0", TypeRef::String),
                    make_sealed_field("_1", TypeRef::Primitive(PrimitiveType::I32)),
                ],
                true,
            ),
            make_sealed_variant("Struct", vec![make_sealed_field("reason", TypeRef::String)], false),
        ],
        methods: vec![],
        doc: "Test enum with various payload types".into(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![format_metadata_enum],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// Test that sealed class tuple variants use payload-derived field names instead of `field0`.
///
/// Expected behavior:
/// - `Pdf(PdfMetadata)` → `val metadata: PdfMetadata` (strip common prefix "Pdf")
/// - `Custom(String)` → `val value: String` (generic name for primitive)
/// - `Multi(String, Int)` → `val value0: String, val value1: Int` (generic names for multiple)
/// - `Struct { reason: String }` → `val reason: String` (use field name directly)
#[test]
fn sealed_variant_tuple_params_use_payload_derived_names() {
    let api = make_sealed_variants_api();
    let config = make_sealed_variants_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let format_metadata_kt = files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.ends_with("FormatMetadata.kt"))
                .unwrap_or(false)
        })
        .expect("FormatMetadata.kt must be emitted");

    let content = &format_metadata_kt.content;

    // Pdf(PdfMetadata) should derive "metadata" by stripping "Pdf" prefix.
    // Per the ktfmt 100-char heuristic introduced in v0.16.22, short
    // declarations are emitted single-line; the assertions below check the
    // single-line shape (the long multi-line form is exercised elsewhere).
    assert!(
        content.contains("data class Pdf(val metadata: PdfMetadata)"),
        "Pdf variant should use payload-derived name 'metadata', got:\n{content}"
    );

    // Custom(String) should use generic "value" for primitive
    assert!(
        content.contains("data class Custom(val value: String)"),
        "Custom variant should use generic name 'value' for primitive payload, got:\n{content}"
    );

    // Multi(String, Int) should use "value0" and "value1"
    assert!(
        content.contains("data class Multi(val value0: String, val value1: Int)"),
        "Multi variant should use generic names 'value0', 'value1', got:\n{content}"
    );

    // Struct { reason: String } should use the original field name
    assert!(
        content.contains("data class Struct(val reason: String)"),
        "Struct variant should preserve the field name 'reason', got:\n{content}"
    );

    // Should NOT use placeholder "field0" anywhere for tuple variants
    assert!(
        !content.contains("field0"),
        "should not use placeholder 'field0', got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// Track 2.1 — Error message template interpolation
// ---------------------------------------------------------------------------

fn make_tuple_error_api() -> ApiSurface {
    use alef::core::ir::{ErrorDef, ErrorVariant, FieldDef};
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "ConversionError".to_string(),
            rust_path: "demo::ConversionError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                // Single-field tuple variant with `{0}` in the template.
                ErrorVariant {
                    name: "ParseError".to_string(),
                    message_template: Some("HTML parsing error: {0}".to_string()),
                    fields: vec![FieldDef {
                        name: "_0".to_string(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: alef::core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    }],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    is_tuple: false,
                    doc: String::new(),
                },
                // Multi-field variant with `{0}` and `{1}`.
                ErrorVariant {
                    name: "Located".to_string(),
                    message_template: Some("Error at {0}:{1}".to_string()),
                    fields: vec![
                        FieldDef {
                            name: "_0".to_string(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            doc: String::new(),
                            sanitized: false,
                            is_boxed: false,
                            type_rust_path: None,
                            cfg: None,
                            typed_default: None,
                            core_wrapper: alef::core::ir::CoreWrapper::None,
                            vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                            newtype_wrapper: None,
                            serde_rename: None,
                            serde_flatten: false,
                            binding_excluded: false,
                            binding_exclusion_reason: None,
                            original_type: None,
                        },
                        FieldDef {
                            name: "_1".to_string(),
                            ty: TypeRef::Primitive(PrimitiveType::U32),
                            optional: false,
                            default: None,
                            doc: String::new(),
                            sanitized: false,
                            is_boxed: false,
                            type_rust_path: None,
                            cfg: None,
                            typed_default: None,
                            core_wrapper: alef::core::ir::CoreWrapper::None,
                            vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                            newtype_wrapper: None,
                            serde_rename: None,
                            serde_flatten: false,
                            binding_excluded: false,
                            binding_exclusion_reason: None,
                            original_type: None,
                        },
                    ],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    is_tuple: false,
                    doc: String::new(),
                },
                // Unit variant (no fields) — template must not emit `{0}`.
                ErrorVariant {
                    name: "Unknown".to_string(),
                    message_template: Some("unknown error".to_string()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: String::new(),
                },
            ],
            doc: "Conversion errors.".to_string(),
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
    }
}

/// Track 2.1 regression: `{N}` placeholder tokens in error message templates
/// must be interpolated as Kotlin string-template references (`${fieldN}`), not
/// emitted literally.
#[test]
fn error_tuple_variant_message_template_interpolates_field_refs() {
    let api = make_tuple_error_api();
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let error_kt = files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s == "ConversionError.kt")
                .unwrap_or(false)
        })
        .expect("ConversionError.kt must be emitted");

    let content = &error_kt.content;

    // Single-field variant: `{0}` → `$field0` (no braces — next char `"` is not
    // an identifier continuation, so the brace form would be redundant per
    // ktlint's `standard:string-template` rule).
    assert!(
        content.contains(r#"ConversionError("HTML parsing error: $field0")"#),
        "ParseError must interpolate field0, got:\n{content}"
    );

    // Multi-field variant: `{0}:{1}` → `$field0:$field1`. The `:` and `"`
    // are not identifier-continuation chars so neither slot needs braces.
    assert!(
        content.contains(r#"ConversionError("Error at $field0:$field1")"#),
        "Located must interpolate field0 and field1, got:\n{content}"
    );

    // Unit variant: no fields, no interpolation — literal message only.
    assert!(
        content.contains(r#"object Unknown : ConversionError("unknown error")"#),
        "Unknown unit variant must emit literal message, got:\n{content}"
    );

    // Must NOT contain any literal `{0}` or `{1}` placeholder tokens.
    assert!(
        !content.contains("{0}"),
        "content must not contain literal {{0}} token, got:\n{content}"
    );
    assert!(
        !content.contains("{1}"),
        "content must not contain literal {{1}} token, got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// Track 2.2 — High-level convert() wrapper with Jackson deserialization
// ---------------------------------------------------------------------------

fn make_convert_api() -> ApiSurface {
    // Simulate a parser crate's convert(html: String, options: Option<ParseOptions>) ->
    // ParseOutput shape where ParseOptions and ParseOutput are
    // non-opaque named types (DTOs).
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "ParseOptions".into(),
                rust_path: "demo::ParseOptions".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: String::new(),
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
                version: Default::default(),
            },
            TypeDef {
                name: "ParseOutput".into(),
                rust_path: "demo::ParseOutput".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: String::new(),
                cfg: None,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![FunctionDef {
            name: "convert".into(),
            rust_path: "demo::convert".into(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "html".into(),
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
                },
                ParamDef {
                    name: "options".into(),
                    ty: TypeRef::Named("ParseOptions".into()),
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
            return_type: TypeRef::Named("ParseOutput".into()),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// Track 2.2 regression: when the API surface has functions returning non-opaque
/// Named types (DTOs like `ParseOutput`), the facade object must:
/// - Accept typed options (`ParseOptions? = null`) not raw JSON strings.
/// - Deserialize the result JSON via Jackson into the typed return class.
/// - Emit a `suspend fun convertAsync` companion via `withContext(Dispatchers.IO)`.
/// - Import `jacksonObjectMapper`, `Dispatchers`, and `withContext`.
#[test]
fn typed_dto_return_emits_jackson_wrapper_and_suspend_async() {
    let api = make_convert_api();
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted");

    let content = &module_kt.content;

    // Options param must be typed, not raw String.
    assert!(
        content.contains("options: ParseOptions? = null"),
        "options param must be typed ParseOptions? = null, got:\n{content}"
    );

    // Return type must be the named DTO, not String.
    assert!(
        content.contains("): ParseOutput {"),
        "convert must return ParseOutput, got:\n{content}"
    );

    // Jackson deserialization must be present.
    assert!(
        content.contains("mapper.readValue(resultJson, ParseOutput::class.java)"),
        "must deserialize result via Jackson, got:\n{content}"
    );

    // Options must be serialized when non-null.
    assert!(
        content.contains("mapper.writeValueAsString"),
        "must serialize options via Jackson, got:\n{content}"
    );

    // Suspend async companion must be emitted.
    assert!(
        content.contains("suspend fun convertAsync("),
        "must emit suspend fun convertAsync, got:\n{content}"
    );
    assert!(
        content.contains("withContext(Dispatchers.IO)"),
        "convertAsync must use withContext(Dispatchers.IO), got:\n{content}"
    );

    // Jackson import must be present.
    assert!(
        content.contains("import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper"),
        "must import jacksonObjectMapper, got:\n{content}"
    );

    // Must NOT use raw String for options in bridge call.
    assert!(
        !content.contains("options: String"),
        "options param must not be raw String, got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// Defect T1.6 — Batch functions must return typed List<T> not raw String
// ---------------------------------------------------------------------------

fn make_batch_function_api() -> ApiSurface {
    // Simulate sample_crate's batch_extract_files and batch_extract_bytes:
    //   batch_extract_files(items: Vec<BatchFileItem>) -> Result<Vec<ParseResult>, _>
    //   batch_extract_bytes(items: Vec<BatchBytesItem>) -> Result<Vec<ParseResult>, _>
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "DemoItem".into(),
                rust_path: "demo::DemoItem".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: String::new(),
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
                version: Default::default(),
            },
            TypeDef {
                name: "DemoResult".into(),
                rust_path: "demo::DemoResult".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: String::new(),
                cfg: None,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![FunctionDef {
            name: "batch_demo".into(),
            rust_path: "demo::batch_demo".into(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "items".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("DemoItem".into()))),
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
            return_type: TypeRef::Vec(Box::new(TypeRef::Named("DemoResult".into()))),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// Defect T1.6 regression: batch functions returning `Vec<DTO>` must:
/// - Accept typed list params (`List<DemoItem>`) not raw JSON strings.
/// - Return typed list (`List<DemoResult>`) not raw JSON strings.
/// - Deserialize the result JSON via Jackson into `List<DemoResult>`.
/// - Emit a `suspend fun batchDemoAsync` companion via `withContext(Dispatchers.IO)`.
#[test]
fn batch_function_returns_typed_list_with_jackson_deserialization() {
    let api = make_batch_function_api();
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted");

    let content = &module_kt.content;

    // Items param must be typed List<DemoItem>, not raw String.
    assert!(
        content.contains("items: List<DemoItem>"),
        "items param must be typed List<DemoItem>, got:\n{content}"
    );

    // Return type must be List<DemoResult>, not raw String.
    assert!(
        content.contains("): List<DemoResult> {"),
        "batchDemo must return List<DemoResult>, NOT String, got:\n{content}"
    );

    // Must NOT return raw String.
    assert!(
        !content.contains("fun batchDemo(items: List<DemoItem>): String"),
        "batchDemo must not return String, got:\n{content}"
    );

    // Jackson deserialization with TypeReference must be present.
    assert!(
        content.contains("mapper.readValue"),
        "must deserialize result via Jackson mapper.readValue, got:\n{content}"
    );
    assert!(
        content.contains("TypeReference<List<DemoResult>>"),
        "must use TypeReference<List<DemoResult>> for deserialization, got:\n{content}"
    );

    // Items must be serialized to JSON when passed to bridge.
    assert!(
        content.contains("mapper.writeValueAsString(items)"),
        "must serialize items via Jackson, got:\n{content}"
    );

    // Suspend async companion must be emitted.
    assert!(
        content.contains("suspend fun batchDemoAsync("),
        "must emit suspend fun batchDemoAsync, got:\n{content}"
    );
    assert!(
        content.contains("withContext(Dispatchers.IO)"),
        "batchDemoAsync must use withContext(Dispatchers.IO), got:\n{content}"
    );

    // Jackson and coroutine imports must be present.
    assert!(
        content.contains("import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper"),
        "must import jacksonObjectMapper, got:\n{content}"
    );
    assert!(
        content.contains("import com.fasterxml.jackson.core.type.TypeReference"),
        "must import TypeReference for List deserialization, got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// KDoc emission regression: every declaration in the Kotlin source the
// backend writes itself (module-facade fns and handle-only wrapper classes)
// must carry KDoc derived from the IR's `doc` field.
// ---------------------------------------------------------------------------

/// Helper: build a handle-only API with a documented opaque type and a
/// documented free function so KDoc emission can be asserted on both
/// emitted Kotlin files.
fn make_documented_handle_only_api() -> ApiSurface {
    let mut api = make_handle_only_api();
    api.types[0].doc = "Opaque crawl engine handle.".to_string();
    // `create_engine` is the first function; tag it with a rustdoc summary.
    api.functions[0].doc = "Allocate a fresh crawl engine handle.".to_string();
    api
}

#[test]
fn module_facade_function_emits_kdoc_from_ir_doc() {
    let api = make_documented_handle_only_api();
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted when visible free functions exist");
    let content = &module_kt.content;

    assert!(
        content.contains("    /** Allocate a fresh crawl engine handle. */"),
        "createEngine must carry a KDoc block derived from its rustdoc, got:\n{content}"
    );
}

#[test]
fn handle_only_wrapper_emits_kdoc_from_type_doc() {
    let api = make_documented_handle_only_api();
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let handle_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("CrawlEngineHandle.kt"))
        .expect("CrawlEngineHandle.kt wrapper must be emitted for handle-only opaque types");
    let content = &handle_kt.content;

    assert!(
        content.contains("/** Opaque crawl engine handle. */"),
        "CrawlEngineHandle wrapper must carry a KDoc block derived from the IR type doc, got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// iter-9 Stream B: generic-container return routing through Jackson
// `TypeReference<T>` instead of an invalid `Class<T>::class.java` literal.
// Covers `Vec<Primitive>`, `Vec<String>`, `HashMap<K, V>`, `Option<Vec<_>>`,
// and the regression boundary for scalar named DTO returns (still use
// `::class.java`).
// ---------------------------------------------------------------------------

fn make_generic_container_api(return_ty: TypeRef, fn_name: &str) -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: fn_name.into(),
            rust_path: format!("demo::{fn_name}"),
            original_rust_path: String::new(),
            params: vec![],
            return_type: return_ty,
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

#[test]
fn vec_of_primitive_string_return_uses_type_reference_list_string() {
    let api = make_generic_container_api(TypeRef::Vec(Box::new(TypeRef::String)), "available_languages");
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();
    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted");
    let content = &module_kt.content;

    assert!(
        content.contains("): List<String> {"),
        "availableLanguages must return List<String>, got:\n{content}"
    );
    assert!(
        content.contains("object : TypeReference<List<String>>() {})"),
        "Vec<String> return must deserialize through TypeReference<List<String>>, got:\n{content}"
    );
    assert!(
        !content.contains("List<String>::class.java"),
        "must never emit invalid List<String>::class.java, got:\n{content}"
    );
    assert!(
        content.contains("import com.fasterxml.jackson.core.type.TypeReference"),
        "TypeReference must be imported, got:\n{content}"
    );
}

#[test]
fn hashmap_string_long_return_uses_type_reference_map_string_long() {
    let map_ty = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Primitive(alef::core::ir::PrimitiveType::U64)),
    );
    let api = make_generic_container_api(map_ty, "stats");
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();
    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted");
    let content = &module_kt.content;

    assert!(
        content.contains("): Map<String, Long> {"),
        "stats must return Map<String, Long>, got:\n{content}"
    );
    assert!(
        content.contains("object : TypeReference<Map<String, Long>>() {})"),
        "Map return must deserialize through TypeReference<Map<String, Long>>, got:\n{content}"
    );
}

#[test]
fn optional_vec_string_return_uses_type_reference_with_nullable_list() {
    let opt_vec = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));
    let api = make_generic_container_api(opt_vec, "maybe_languages");
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();
    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted");
    let content = &module_kt.content;

    assert!(
        content.contains("object : TypeReference<List<String>?>() {})"),
        "Option<Vec<String>> must route through TypeReference<List<String>?>, got:\n{content}"
    );
}

#[test]
fn vec_of_named_dto_return_still_uses_type_reference_list_dto() {
    // Regression boundary: the legacy Vec<NamedDto> path must continue to
    // emit a typed TypeReference<List<DemoResult>>.
    let api = make_batch_function_api();
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();
    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted");
    let content = &module_kt.content;

    assert!(
        content.contains("TypeReference<List<DemoResult>>"),
        "Vec<NamedDto> regression: must still emit TypeReference<List<DemoResult>>, got:\n{content}"
    );
}

#[test]
fn scalar_named_dto_return_still_uses_class_java_literal() {
    // Regression boundary: scalar Named DTO returns (`ParseOutput`)
    // must keep the `ParseOutput::class.java` deserialization path —
    // generic-container routing is reserved for Vec/Map shapes.
    let api = make_convert_api();
    let config = make_opaque_factory_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();
    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted");
    let content = &module_kt.content;

    assert!(
        content.contains("mapper.readValue(resultJson, ParseOutput::class.java)"),
        "scalar DTO return must use ParseOutput::class.java, got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// exclude_types: types listed in [crates.kotlin_android].exclude_types must
// not appear as emitted .kt data-class / enum / error files.
// ---------------------------------------------------------------------------

fn make_exclude_types_config(excluded: &[&str]) -> ResolvedCrateConfig {
    let exclude_list = excluded
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ");
    resolved_one(&format!(
        r#"
[workspace]
languages = ["kotlin_android", "java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.sample_crate"

[crates.kotlin_android]
package = "dev.sample_crate.demo.android"
namespace = "dev.sample_crate.demo.android"
artifact_id = "demo-android"
group_id = "dev.sample_crate"
exclude_types = [{exclude_list}]
"#
    ))
}

fn make_exclude_types_api() -> ApiSurface {
    use alef::core::ir::{EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef};
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "TowerRequest".into(),
                rust_path: "demo::TowerRequest".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: String::new(),
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
                version: Default::default(),
            },
            TypeDef {
                name: "TowerResponse".into(),
                rust_path: "demo::TowerResponse".into(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: String::new(),
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
                version: Default::default(),
            },
            TypeDef {
                name: "KeepMe".into(),
                rust_path: "demo::KeepMe".into(),
                original_rust_path: String::new(),
                fields: vec![make_sealed_field("value", TypeRef::String)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: String::new(),
                cfg: None,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![EnumDef {
            name: "TowerStatus".into(),
            rust_path: "demo::TowerStatus".into(),
            original_rust_path: "demo::TowerStatus".into(),
            variants: vec![EnumVariant {
                name: "Ok".into(),
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
            }],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![ErrorDef {
            name: "TowerError".into(),
            rust_path: "demo::TowerError".into(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Generic".into(),
                message_template: Some("error".into()),
                fields: vec![FieldDef {
                    name: "msg".into(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            }],
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
    }
}

/// Types named in `[crates.kotlin_android].exclude_types` must not produce
/// `.kt` files in the generated output. Types NOT in the exclusion list must
/// still be emitted normally.
#[test]
fn exclude_types_suppresses_listed_types_enums_and_errors() {
    let api = make_exclude_types_api();
    let config = make_exclude_types_config(&["TowerRequest", "TowerResponse", "TowerStatus", "TowerError"]);
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let kt_filenames: Vec<&str> = files
        .iter()
        .filter_map(|f| f.path.file_name()?.to_str())
        .filter(|n| n.ends_with(".kt"))
        .collect();

    assert!(
        !kt_filenames.contains(&"TowerRequest.kt"),
        "TowerRequest.kt must be suppressed by exclude_types, got: {kt_filenames:?}"
    );
    assert!(
        !kt_filenames.contains(&"TowerResponse.kt"),
        "TowerResponse.kt must be suppressed by exclude_types, got: {kt_filenames:?}"
    );
    assert!(
        !kt_filenames.contains(&"TowerStatus.kt"),
        "TowerStatus.kt (enum) must be suppressed by exclude_types, got: {kt_filenames:?}"
    );
    assert!(
        !kt_filenames.contains(&"TowerError.kt"),
        "TowerError.kt (error) must be suppressed by exclude_types, got: {kt_filenames:?}"
    );
    assert!(
        kt_filenames.contains(&"KeepMe.kt"),
        "KeepMe.kt must still be emitted when not in exclude_types, got: {kt_filenames:?}"
    );
}

// ---------------------------------------------------------------------------
// Trait-bridge emission
// ---------------------------------------------------------------------------

fn make_trait_api() -> ApiSurface {
    use alef::core::ir::{MethodDef, TypeDef};
    let trait_def = TypeDef {
        name: "TextBackend".into(),
        rust_path: "demo::TextBackend".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "process_image".into(),
            params: vec![ParamDef {
                name: "image".into(),
                ty: TypeRef::Bytes,
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
            is_async: true,
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
        }],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: true,
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
        types: vec![trait_def],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_trait_bridge_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.sample_crate"

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
artifact_id = "demo-android"
group_id = "dev.sample_crate"

[[crates.trait_bridges]]
trait_name = "TextBackend"
super_trait = "demo::Plugin"
registry_getter = "demo::get_text_registry"
register_fn = "register_text_backend"
unregister_fn = "unregister_text_backend"
clear_fn = "clear_text_backends"
"#,
    )
}

/// Every `[[crates.trait_bridges]]` entry must emit three `external fun`
/// declarations on the bridge object (register / unregister / clear) plus an
/// `I<Trait>.kt` interface file, mirroring the Java/Panama backend's contract.
#[test]
fn trait_bridge_emits_native_funs_and_interface_file() {
    let api = make_trait_api();
    let config = make_trait_bridge_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let bridge_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("DemoBridge.kt"))
        .expect("DemoBridge.kt must be emitted");
    let body = &bridge_kt.content;

    assert!(
        body.contains("external fun nativeRegisterTextBackend(impl: dev.sample_crate.ITextBackend)"),
        "register native fun missing: {body}"
    );
    assert!(
        body.contains("external fun nativeUnregisterTextBackend(name: String)"),
        "unregister native fun missing: {body}"
    );
    assert!(
        body.contains("external fun nativeClearTextBackends()"),
        "clear native fun missing: {body}"
    );

    let iface = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("ITextBackend.kt"))
        .expect("ITextBackend.kt must be emitted alongside the trait-bridge native funs");
    let iface_body = &iface.content;
    assert!(
        iface_body.contains("interface ITextBackend"),
        "I<Trait> interface declaration missing: {iface_body}"
    );
    // Super-trait `Plugin` surfaces the canonical lifecycle methods.
    assert!(
        iface_body.contains("fun name(): String"),
        "name() lifecycle method missing: {iface_body}"
    );
    // Async IR methods become Kotlin `suspend` functions.
    assert!(
        iface_body.contains("suspend fun processImage(image: ByteArray): String"),
        "process_image suspend signature missing: {iface_body}"
    );
}

/// Trait bridges that list `kotlin_android` in `exclude_languages` must not
/// produce any of the three native funs nor the I<Trait>.kt interface file.
#[test]
fn trait_bridge_exclude_languages_skips_kotlin_android() {
    let api = make_trait_api();
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.sample_crate"

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
artifact_id = "demo-android"
group_id = "dev.sample_crate"

[[crates.trait_bridges]]
trait_name = "TextBackend"
register_fn = "register_text_backend"
unregister_fn = "unregister_text_backend"
clear_fn = "clear_text_backends"
exclude_languages = ["kotlin_android"]
"#,
    );
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();
    let bridge_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("DemoBridge.kt"))
        .expect("DemoBridge.kt must be emitted");
    let body = &bridge_kt.content;
    assert!(
        !body.contains("nativeRegisterTextBackend"),
        "register native fun must be suppressed when kotlin_android is excluded: {body}"
    );
    assert!(
        !files
            .iter()
            .any(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("ITextBackend.kt")),
        "I<Trait>.kt must be suppressed when kotlin_android is excluded"
    );
}

// ---------------------------------------------------------------------------
// Regression: `binding_excluded` types and enums must not produce .kt files.
//
// The IR field `binding_excluded` is set by upstream `#[cfg_attr(alef, alef(skip))]`
// (or `#[doc(hidden)]`) annotations on the Rust source. The kotlin-android emitter
// must honour the flag for both DTOs and enums, matching the behaviour of every
// other backend (PHP, WASM, NAPI, etc.). Without this filter, the sample_crate AAR
// ships ~28 stale wrapper files (`LegacyPlainTextConfig.kt`, `Table2.kt`,
// `LegacyPipelineConfig.kt`, ...) corresponding to types the Rust source has marked
// as binding-excluded.
// ---------------------------------------------------------------------------

fn make_minimal_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.sample_crate"

[crates.kotlin_android]
package = "dev.sample_crate.demo.android"
namespace = "dev.sample_crate.demo.android"
artifact_id = "demo-android"
group_id = "dev.sample_crate"
"#,
    )
}

#[test]
fn skipped_types_and_enums_are_not_emitted_as_kt_files() {
    use alef::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

    let included_type = TypeDef {
        name: "IncludedDto".into(),
        rust_path: "demo::IncludedDto".into(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            name: "value".into(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: Default::default(),
            vec_inner_core_wrapper: Default::default(),
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
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
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: true,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let skipped_type = TypeDef {
        name: "SkippedDto".into(),
        rust_path: "demo::SkippedDto".into(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            name: "value".into(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: Default::default(),
            vec_inner_core_wrapper: Default::default(),
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
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
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: true,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: true,
        binding_exclusion_reason: Some("alef(skip)".into()),
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let included_enum = EnumDef {
        name: "IncludedMode".into(),
        rust_path: "demo::IncludedMode".into(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "On".into(),
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
        }],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: true,
        has_serde: true,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let skipped_enum = EnumDef {
        name: "SkippedMode".into(),
        rust_path: "demo::SkippedMode".into(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "Off".into(),
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
        }],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: true,
        has_serde: true,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: true,
        binding_exclusion_reason: Some("alef(skip)".into()),
        excluded_variants: vec![],
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![included_type, skipped_type],
        functions: vec![],
        enums: vec![included_enum, skipped_enum],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_minimal_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let kt_names: Vec<String> = files
        .iter()
        .filter_map(|f| f.path.file_name().and_then(|n| n.to_str()).map(String::from))
        .collect();

    assert!(
        kt_names.iter().any(|n| n == "IncludedDto.kt"),
        "non-excluded DTO must be emitted; got: {kt_names:?}"
    );
    assert!(
        kt_names.iter().any(|n| n == "IncludedMode.kt"),
        "non-excluded enum must be emitted; got: {kt_names:?}"
    );
    assert!(
        !kt_names.iter().any(|n| n == "SkippedDto.kt"),
        "binding_excluded DTO must NOT be emitted; got: {kt_names:?}"
    );
    assert!(
        !kt_names.iter().any(|n| n == "SkippedMode.kt"),
        "binding_excluded enum must NOT be emitted; got: {kt_names:?}"
    );
}

// ---------------------------------------------------------------------------
// AGP Parser Cascade Fix: Long interface method signatures
// ---------------------------------------------------------------------------

fn make_long_signature_api() -> ApiSurface {
    // Create a trait with methods that have long signatures (>=115 chars) to
    // trigger the AGP 8.13.0 + Kotlin 2.3.21 parser cascade on single-line form.
    let extract_method = MethodDef {
        name: "extract_file".into(),
        params: vec![
            ParamDef {
                name: "path".into(),
                ty: TypeRef::Named("java.nio.file.Path".into()),
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
            },
            ParamDef {
                name: "mime_type".into(),
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
            },
            ParamDef {
                name: "config".into(),
                ty: TypeRef::Named("ExtractionConfig".into()),
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
            },
        ],
        return_type: TypeRef::Named("ParseResult".into()),
        is_async: true,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: None,
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

    let short_method = MethodDef {
        name: "extract".into(),
        params: vec![ParamDef {
            name: "data".into(),
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
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: None,
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

    let zero_param_method = MethodDef {
        name: "get_version".into(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: None,
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

    let trait_type = TypeDef {
        name: "DocumentExtractor".into(),
        rust_path: "demo::DocumentExtractor".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![extract_method, short_method, zero_param_method],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: true,
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
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_long_signature_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.sample_crate"

[crates.kotlin_android]
package = "dev.sample_crate.demo.android"
namespace = "dev.sample_crate.demo.android"
artifact_id = "demo-android"
group_id = "dev.sample_crate"

[[crates.trait_bridges]]
trait_name = "DocumentExtractor"
type_alias = "DocumentExtractorHandle"
"#,
    )
}

/// Test 1: Short method signature stays single-line
#[test]
fn long_signature_short_method_stays_single_line() {
    let api = make_long_signature_api();
    let config = make_long_signature_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let interface_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("IDocumentExtractor.kt"))
        .expect("IDocumentExtractor.kt must be emitted");

    let content = &interface_kt.content;

    // Short method should be single-line
    assert!(
        content.contains("fun extract(data: String): String"),
        "short method must stay single-line, got:\n{content}"
    );
    // Should NOT wrap into multi-line
    assert!(
        !content.contains("fun extract(\n"),
        "short method must not wrap, got:\n{content}"
    );
}

/// Test 2: Trait method signatures use visible binding types before wrap decisions
#[test]
fn long_signature_trait_method_substitutes_hidden_types_before_wrapping() {
    let api = make_long_signature_api();
    let config = make_long_signature_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let interface_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("IDocumentExtractor.kt"))
        .expect("IDocumentExtractor.kt must be emitted");

    let content = &interface_kt.content;

    // Named types that are not emitted as visible bindings are substituted with
    // String before the signature-length decision, keeping this fixture short.
    assert!(
        content.contains("suspend fun extractFile(path: String, mimeType: String, config: String): String"),
        "trait method must use substituted visible types in single-line form, got:\n{content}"
    );
    assert!(
        !content.contains("suspend fun extractFile(\n"),
        "substituted short trait method must not wrap, got:\n{content}"
    );
}

/// Test 3: Zero-param method stays single-line even if return type is long
#[test]
fn long_signature_zero_param_method_always_single_line() {
    let api = make_long_signature_api();
    let config = make_long_signature_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let interface_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("IDocumentExtractor.kt"))
        .expect("IDocumentExtractor.kt must be emitted");

    let content = &interface_kt.content;

    // Zero-param method should always be single-line
    assert!(
        content.contains("fun getVersion(): String"),
        "zero-param method must stay single-line, got:\n{content}"
    );
    assert!(
        !content.contains("fun getVersion(\n"),
        "zero-param method must not wrap, got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// Unit test for format_method_signature helper function
// ---------------------------------------------------------------------------

use alef::backends::kotlin_android::gen_bindings::format_method_signature;

#[test]
fn format_method_signature_short_stays_single_line() {
    let sig = format_method_signature("", "process", "data: String", "Boolean");
    assert!(
        !sig.trim().contains('\n'),
        "short signature should be single-line, got:\n{sig}"
    );
    assert!(
        sig.contains("fun process(data: String): Boolean"),
        "short signature format incorrect, got:\n{sig}"
    );
}

#[test]
fn format_method_signature_long_wraps_with_trailing_commas() {
    let sig = format_method_signature(
        "suspend ",
        "extractFile",
        "path: java.nio.file.Path, mimeType: String, config: ExtractionConfig",
        "ParseResult",
    );
    // Should be multi-line with trailing commas
    assert!(
        sig.contains("suspend fun extractFile(\n"),
        "long suspend signature should wrap, got:\n{sig}"
    );
    assert!(
        sig.contains("path: java.nio.file.Path,\n"),
        "path param should have trailing comma, got:\n{sig}"
    );
    assert!(
        sig.contains("mimeType: String,\n"),
        "mimeType param should have trailing comma, got:\n{sig}"
    );
    assert!(
        sig.contains("config: ExtractionConfig,\n"),
        "config param should have trailing comma, got:\n{sig}"
    );
    assert!(
        sig.contains("): ParseResult"),
        "return type should be on closing paren line, got:\n{sig}"
    );
}

#[test]
fn format_method_signature_empty_params_always_single_line() {
    let sig = format_method_signature(
        "",
        "getVersion",
        "",
        "VeryLongReturnTypeNameThatWouldExceedThresholdIfParamsWerePresent",
    );
    assert!(
        !sig.trim().contains('\n'),
        "empty-param signature should always be single-line, got:\n{sig}"
    );
    assert!(
        sig.contains("fun getVersion(): VeryLongReturnTypeNameThatWouldExceedThresholdIfParamsWerePresent"),
        "empty-param signature format incorrect, got:\n{sig}"
    );
}

#[test]
fn format_method_signature_unit_return_type() {
    let sig = format_method_signature("", "initialize", "", "Unit");
    assert!(
        sig.contains("fun initialize()\n"),
        "Unit return should omit return type, got:\n{sig}"
    );
    assert!(
        !sig.contains(": Unit"),
        "Unit return should not appear in signature, got:\n{sig}"
    );
}

/// Regression: optional ByteArray/Vec<u8> parameters in facade functions
/// must be declared as `ByteArray?` not `String?`.
///
/// This ensures the Kotlin wrapper function signature matches what the
/// caller expects. The bridge argument encoding (base64) happens transparently
/// in the bridge call, not in the public API type.
#[test]
fn optional_bytes_param_declared_as_bytearray() {
    let analyze_fn = FunctionDef {
        name: "analyze_document".into(),
        rust_path: "demo::analyze_document".into(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "document_bytes".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::Bytes)),
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
        }],
        return_type: TypeRef::String,
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
    };
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![analyze_fn],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_minimal_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();

    let module_kt = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("Demo.kt"))
        .expect("Demo.kt must be emitted");

    // Check that the optional bytes parameter is declared as ByteArray? not String?
    assert!(
        module_kt.content.contains("documentBytes: ByteArray? = null"),
        "Expected 'documentBytes: ByteArray? = null' in generated Demo.kt, got:\n{}",
        module_kt.content
    );

    // Check that the bridge call correctly base64-encodes it
    assert!(
        module_kt
            .content
            .contains("java.util.Base64.getEncoder().encodeToString(it)"),
        "Expected base64 encoding of documentBytes in bridge call"
    );
}
