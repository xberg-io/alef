use alef_backend_kotlin_android::KotlinAndroidBackend;
use alef_core::backend::Backend;
use alef_core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef_core::ir::{ApiSurface, FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
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
package = "dev.kreuzberg"

[crates.kotlin_android]
package = "dev.kreuzberg.demo.android"
namespace = "dev.kreuzberg.demo.android"
artifact_id = "demo-android"
group_id = "dev.kreuzberg"

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
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    }
}

/// `KotlinAndroidBackend` must advertise streaming support so downstream
/// consumers (e.g. liter-llm) can rely on the `Flow<T>` surface.
#[test]
fn supports_streaming_capability_is_true() {
    assert!(
        KotlinAndroidBackend.capabilities().supports_streaming,
        "KotlinAndroidBackend must report supports_streaming = true"
    );
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
package = "dev.kreuzberg"

[crates.kotlin_android]
package = "dev.kreuzberg.demo.android"
namespace = "dev.kreuzberg.demo.android"
artifact_id = "demo-android"
group_id = "dev.kreuzberg"
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
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![create_client_fn],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
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
// Bug 2 regression: handle-only opaque type (kreuzcrawl shape) — top-level
// fns taking and returning an opaque handle that has NO instance methods.
// ---------------------------------------------------------------------------

/// Build an API with the kreuzcrawl shape:
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
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![engine_type],
        functions: vec![create_engine_fn, scrape_fn],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    }
}

/// Regression test for the kreuzcrawl-shape facade bug: when a top-level fn
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
