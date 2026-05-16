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

// ---------------------------------------------------------------------------
// Bug 5 regression: optional params must get Kotlin default values so that
// e2e callers that only pass `apiKey` and `baseUrl` still compile.
// ---------------------------------------------------------------------------

fn make_optional_params_api() -> ApiSurface {
    // A top-level `create_client` with three optional params (the liter-llm shape):
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
    use alef_core::ir::PrimitiveType;
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
package = "dev.kreuzberg"

[crates.kotlin_android]
package = "dev.kreuzberg.demo.android"
namespace = "dev.kreuzberg.demo.android"
artifact_id = "demo-android"
group_id = "dev.kreuzberg"
"#,
    )
}

use alef_core::ir::{CoreWrapper, EnumDef, EnumVariant, FieldDef, PrimitiveType};

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
            make_sealed_variant(
                "Struct",
                vec![make_sealed_field("reason", TypeRef::String)],
                false,
            ),
        ],
        doc: "Test enum with various payload types".into(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
    };

    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![format_metadata_enum],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
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

    // Pdf(PdfMetadata) should derive "metadata" by stripping "Pdf" prefix
    assert!(
        content.contains("data class Pdf(\n        val metadata: PdfMetadata\n    )"),
        "Pdf variant should use payload-derived name 'metadata', got:\n{content}"
    );

    // Custom(String) should use generic "value" for primitive
    assert!(
        content.contains("data class Custom(\n        val value: String\n    )"),
        "Custom variant should use generic name 'value' for primitive payload, got:\n{content}"
    );

    // Multi(String, Int) should use "value0" and "value1"
    assert!(
        content.contains("data class Multi(\n        val value0: String,\n        val value1: Int\n    )"),
        "Multi variant should use generic names 'value0', 'value1', got:\n{content}"
    );

    // Struct { reason: String } should use the original field name
    assert!(
        content.contains("data class Struct(\n        val reason: String\n    )"),
        "Struct variant should preserve the field name 'reason', got:\n{content}"
    );

    // Should NOT use placeholder "field0" anywhere for tuple variants
    assert!(
        !content.contains("field0"),
        "should not use placeholder 'field0', got:\n{content}"
    );
}
