use alef_backend_kotlin_android::KotlinAndroidBackend;
use alef_core::backend::Backend;
use alef_core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};

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
