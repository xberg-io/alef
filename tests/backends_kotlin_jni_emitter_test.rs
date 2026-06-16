use alef::backends::jni::JniBackend;
use alef::backends::kotlin::KotlinBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef};

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
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

/// Config that enables JNI mode (`ffi_style = "jni"`).
fn make_jni_config_with_streaming() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.kotlin]
package = "dev.sample_crate"
ffi_style = "jni"

[[crates.adapters]]
name = "stream_data"
pattern = "streaming"
core_path = "stream_data"
owner_type = "DefaultClient"
item_type = "DataChunk"

[[crates.adapters.params]]
name = "req"
type = "StreamRequest"
"#,
    )
}

fn make_jni_config_no_streaming() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.kotlin]
package = "dev.sample_crate"
ffi_style = "jni"
"#,
    )
}

fn make_jni_api_with_client_and_function() -> ApiSurface {
    let foo_function = FunctionDef {
        name: "foo".into(),
        rust_path: "demo::foo".into(),
        original_rust_path: String::new(),
        params: vec![make_param("value", TypeRef::Primitive(PrimitiveType::I32))],
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
    let client_method = MethodDef {
        name: "do_thing".into(),
        params: vec![make_param("input", TypeRef::String)],
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
        methods: vec![client_method],
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
        functions: vec![foo_function],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_simple_api() -> ApiSurface {
    let foo_function = FunctionDef {
        name: "foo".into(),
        rust_path: "demo::foo".into(),
        original_rust_path: String::new(),
        params: vec![make_param("value", TypeRef::Primitive(PrimitiveType::I32))],
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
        types: vec![],
        functions: vec![foo_function],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_top_level_bytes_api() -> ApiSurface {
    let mut payload = make_param("payload", TypeRef::Bytes);
    payload.is_ref = true;
    let upload_function = FunctionDef {
        name: "upload".into(),
        rust_path: "demo::upload".into(),
        original_rust_path: String::new(),
        params: vec![payload],
        return_type: TypeRef::Bytes,
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
        types: vec![],
        functions: vec![upload_function],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// Snapshot the Bridge object emitted in JNI mode.
///
/// Asserts that:
/// - `init { System.loadLibrary("...") }` is present
/// - `external fun nativeFoo(...)` is emitted for the `foo` function
/// - streaming `external fun nativeDefaultClientStreamDataStart/Next/Free` are emitted
#[test]
fn snapshot_jni_bridge_object() {
    let api = make_jni_api_with_client_and_function();
    let config = make_jni_config_with_streaming();
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let bridge_file = files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with("Bridge.kt"))
                .unwrap_or(false)
        })
        .expect("DemoBridge.kt must be emitted in JNI mode");

    let content = &bridge_file.content;

    // System.loadLibrary is present in the init block.
    assert!(
        content.contains("System.loadLibrary("),
        "missing System.loadLibrary in JNI bridge: {content}"
    );
    // Regular function external declaration.
    assert!(
        content.contains("external fun nativeFoo("),
        "missing nativeFoo external fun: {content}"
    );
    // Streaming adapter external declarations.
    assert!(
        content.contains("external fun nativeDefaultClientStreamDataStart("),
        "missing nativeDefaultClientStreamDataStart: {content}"
    );
    assert!(
        content.contains("external fun nativeDefaultClientStreamDataNext("),
        "missing nativeDefaultClientStreamDataNext: {content}"
    );
    assert!(
        content.contains("external fun nativeDefaultClientStreamDataFree("),
        "missing nativeDefaultClientStreamDataFree: {content}"
    );
    // @Throws must appear before every non-destructor external fun so that
    // JNI exceptions (Rust Result::Err / panics) surface as typed exceptions
    // rather than being swallowed or wrapped in UndeclaredThrowableException.
    assert!(
        content.contains("@Throws(DemoBridgeException::class)"),
        "@Throws annotation missing from Bridge.kt: {content}"
    );
    // Destructors are infallible — they must NOT carry @Throws.
    let free_idx = content
        .find("nativeFreeDefaultClient")
        .expect("destructor must be present");
    let throws_before_free = content[..free_idx].rfind("@Throws");
    let external_before_free = content[..free_idx].rfind("external fun native");
    // @Throws must not appear between the last non-destructor external fun and nativeFree.
    assert!(
        throws_before_free < external_before_free || external_before_free.is_none(),
        "destructor nativeFreeDefaultClient must not have a preceding @Throws: {content}"
    );

    insta::assert_snapshot!("snapshot_jni_bridge_object", content);
}

#[test]
fn kotlin_jni_top_level_byte_api_matches_string_encoded_rust_boundary() {
    let api = make_top_level_bytes_api();
    let config = make_pairing_config();

    let kotlin_files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let bridge_content = &kotlin_files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("Bridge.kt"))
        })
        .expect("DemoBridge.kt")
        .content;

    let rust_files = JniBackend.generate_bindings(&api, &config).unwrap();
    let rust_content = &rust_files[0].content;

    assert!(
        bridge_content.contains("external fun nativeUpload(payload: String): ByteArray"),
        "Kotlin JNI declaration must expose string-encoded byte params and ByteArray returns: {bridge_content}"
    );
    assert!(
        rust_content.contains(
            "nativeUpload(\n    mut env: EnvUnowned,\n    _class: JClass,\n    payload: JString,\n) -> jbyteArray"
        ),
        "Rust JNI symbol must expose the matching string-encoded byte param signature: {rust_content}"
    );
    assert!(
        rust_content.contains("base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &payload_str)"),
        "Rust JNI symbol must decode string-encoded byte params before invoking core: {rust_content}"
    );
}

/// Snapshot the DefaultClient class emitted in JNI mode.
///
/// Asserts that:
/// - `class DefaultClient internal constructor(internal val handle: Long) : AutoCloseable`
/// - JSON marshalling pattern (mapper) is present
/// - `close()` calls `nativeFreeDefaultClient(handle)` or the close bridge method
#[test]
fn snapshot_jni_default_client() {
    let api = make_jni_api_with_client_and_function();
    let config = make_jni_config_with_streaming();
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let client_file = files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "DefaultClient.kt")
                .unwrap_or(false)
        })
        .expect("DefaultClient.kt must be emitted in JNI mode when client types exist");

    let content = &client_file.content;

    // Class declaration: holds a Long handle and implements AutoCloseable.
    assert!(
        content.contains("class DefaultClient internal constructor(internal val handle: Long) : AutoCloseable"),
        "missing DefaultClient class with Long handle: {content}"
    );
    // JSON marshalling (ObjectMapper) pattern.
    assert!(
        content.contains("ObjectMapper"),
        "missing ObjectMapper pattern in DefaultClient: {content}"
    );
    // close() calls the free bridge method with the handle.
    assert!(
        content.contains("nativeFreeDefaultClient(handle)"),
        "missing nativeFreeDefaultClient(handle) in close(): {content}"
    );

    insta::assert_snapshot!("snapshot_jni_default_client", content);
}

/// Content invariant: JNI-mode files must NOT contain Panama imports (`java.lang.foreign`).
#[test]
fn snapshot_jni_no_panama_imports() {
    let api = make_jni_api_with_client_and_function();
    let config = make_jni_config_with_streaming();
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    for file in &files {
        let content = &file.content;
        assert!(
            !content.contains("java.lang.foreign"),
            "JNI-mode file {:?} must not contain java.lang.foreign import: {content}",
            file.path
        );
    }

    // Also verify flat-function-only JNI mode (no client types).
    let simple_api = make_simple_api();
    let simple_config = make_jni_config_no_streaming();
    let simple_files = KotlinBackend.generate_bindings(&simple_api, &simple_config).unwrap();
    for file in &simple_files {
        let content = &file.content;
        assert!(
            !content.contains("java.lang.foreign"),
            "JNI-mode file {:?} must not contain java.lang.foreign import: {content}",
            file.path
        );
    }
}

/// Verify Panama mode (default, no `ffi_style`) still emits the Java bridge import and
/// not any JNI `external fun` declarations. This guards against regressions in Panama
/// snapshot compatibility.
#[test]
fn panama_mode_unchanged_after_jni_addition() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin", "java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.sample_crate"

[crates.kotlin]
package = "dev.sample_crate"
target = "jvm"
"#,
    );

    let foo_fn = FunctionDef {
        name: "ping".into(),
        rust_path: "demo::ping".into(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::Unit,
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
        functions: vec![foo_fn],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // Panama mode: Java Bridge import alias is emitted.
    assert!(
        content.contains("import dev.sample_crate.Demo as Bridge"),
        "Panama mode missing Java bridge import: {content}"
    );
    // Panama mode: no `external fun` JNI declarations.
    assert!(
        !content.contains("external fun"),
        "Panama mode must not emit external fun declarations: {content}"
    );
}

// ---------------------------------------------------------------------------
// A2 snapshot tests: full surface, JSON marshalling, ByteArray, Unit returns
// ---------------------------------------------------------------------------

fn make_jni_config_full_surface() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.kotlin]
package = "dev.sample_crate"
ffi_style = "jni"
"#,
    )
}

fn make_api_with_chat_and_embeddings() -> ApiSurface {
    let chat_method = MethodDef {
        name: "chat".into(),
        params: vec![make_param("request", TypeRef::Named("ChatCompletionRequest".into()))],
        return_type: TypeRef::Named("ChatCompletionResponse".into()),
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
    let embeddings_method = MethodDef {
        name: "embeddings".into(),
        params: vec![make_param("request", TypeRef::Named("EmbeddingsRequest".into()))],
        return_type: TypeRef::Named("EmbeddingsResponse".into()),
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
        methods: vec![chat_method, embeddings_method],
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

/// Snapshot: Bridge object emits `external fun` for both `chat` and `embeddings` methods.
#[test]
fn snapshot_jni_bridge_object_full_surface() {
    let api = make_api_with_chat_and_embeddings();
    let config = make_jni_config_full_surface();
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let bridge_file = files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with("Bridge.kt"))
                .unwrap_or(false)
        })
        .expect("DemoBridge.kt must be emitted");

    let content = &bridge_file.content;

    assert!(
        content.contains("external fun nativeDefaultClientChat(handle: Long, requestJson: String): String"),
        "missing nativeDefaultClientChat external fun: {content}"
    );
    assert!(
        content.contains("external fun nativeDefaultClientEmbeddings(handle: Long, requestJson: String): String"),
        "missing nativeDefaultClientEmbeddings external fun: {content}"
    );

    insta::assert_snapshot!("snapshot_jni_bridge_object_full_surface", content);
}

/// Snapshot: DefaultClient.kt emits a `suspend fun chat(...)` wrapper with JSON marshalling.
#[test]
fn snapshot_jni_default_client_chat_complete() {
    let api = make_api_with_chat_and_embeddings();
    let config = make_jni_config_full_surface();
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let client_file = files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "DefaultClient.kt")
                .unwrap_or(false)
        })
        .expect("DefaultClient.kt must be emitted");

    let content = &client_file.content;

    assert!(
        content.contains("suspend fun chat("),
        "missing suspend fun chat: {content}"
    );
    assert!(
        content.contains("withContext(Dispatchers.IO)"),
        "missing withContext(Dispatchers.IO): {content}"
    );
    assert!(
        content.contains("MAPPER.writeValueAsString(request)"),
        "missing MAPPER.writeValueAsString: {content}"
    );
    assert!(
        content.contains("DemoBridge.nativeDefaultClientChat(handle, "),
        "missing bridge call: {content}"
    );
    assert!(
        content.contains("MAPPER.readValue(responseJson, ChatCompletionResponse::class.java)"),
        "missing MAPPER.readValue for return: {content}"
    );

    insta::assert_snapshot!("snapshot_jni_default_client_chat_complete", content);
}

fn make_api_with_speech_method() -> ApiSurface {
    let speech_method = MethodDef {
        name: "speech".into(),
        params: vec![make_param("request", TypeRef::Named("SpeechRequest".into()))],
        return_type: TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8))),
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
        methods: vec![speech_method],
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

/// Snapshot: a method returning `Vec<u8>` produces `ByteArray` in the Bridge and
/// passes the result through without JSON deserialisation in the wrapper.
#[test]
fn snapshot_jni_byte_array_return() {
    let api = make_api_with_speech_method();
    let config = make_jni_config_full_surface();
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let bridge_file = files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with("Bridge.kt"))
                .unwrap_or(false)
        })
        .expect("DemoBridge.kt must be emitted");
    let bridge_content = &bridge_file.content;
    assert!(
        bridge_content.contains("external fun nativeDefaultClientSpeech(handle: Long, requestJson: String): ByteArray"),
        "missing ByteArray return type in Bridge: {bridge_content}"
    );

    let client_file = files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "DefaultClient.kt")
                .unwrap_or(false)
        })
        .expect("DefaultClient.kt must be emitted");
    let client_content = &client_file.content;
    assert!(
        client_content.contains("fun speech("),
        "missing speech method: {client_content}"
    );
    // ByteArray pass-through: no readValue call.
    assert!(
        !client_content.contains("MAPPER.readValue"),
        "ByteArray return must not use MAPPER.readValue: {client_content}"
    );

    insta::assert_snapshot!("snapshot_jni_byte_array_return_bridge", bridge_content);
    insta::assert_snapshot!("snapshot_jni_byte_array_return_client", client_content);
}

#[test]
fn jni_optional_byte_array_method_uses_nullable_facade_and_empty_array_sentinel() {
    let mut payload = make_param("payload", TypeRef::Bytes);
    payload.optional = true;
    payload.is_ref = true;
    let method = MethodDef {
        name: "upload".into(),
        params: vec![payload],
        return_type: TypeRef::Optional(Box::new(TypeRef::Bytes)),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = KotlinBackend
        .generate_bindings(&api, &make_jni_config_no_streaming())
        .unwrap();
    let bridge_content = &files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("Bridge.kt"))
        })
        .expect("bridge file")
        .content;
    let client_content = &files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("DefaultClient.kt"))
        .expect("client file")
        .content;

    assert!(
        bridge_content.contains("external fun nativeDefaultClientUpload(handle: Long, payload: ByteArray): ByteArray?"),
        "JNI bridge must expose binary params/returns as ByteArray, got:\n{bridge_content}"
    );
    assert!(
        client_content.contains("suspend fun upload(payload: ByteArray? = null): ByteArray?"),
        "public JNI facade must expose nullable ByteArray, got:\n{client_content}"
    );
    assert!(
        client_content.contains("DemoBridge.nativeDefaultClientUpload(handle, payload ?: ByteArray(0))"),
        "nullable ByteArray must use the empty-array sentinel for JNI, got:\n{client_content}"
    );
    assert!(
        !client_content.contains("writeValueAsString(payload"),
        "ByteArray params must not be JSON-encoded for direct JNI methods, got:\n{client_content}"
    );
}

fn make_api_with_unit_return_method() -> ApiSurface {
    let cancel_method = MethodDef {
        name: "cancel".into(),
        params: vec![make_param("request_id", TypeRef::String)],
        return_type: TypeRef::Unit,
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
        methods: vec![cancel_method],
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

/// Snapshot: a method returning `Unit` produces a Bridge `external fun` with no return type
/// annotation and a wrapper that does not assign or return the bridge call result.
#[test]
fn snapshot_jni_unit_return() {
    let api = make_api_with_unit_return_method();
    let config = make_jni_config_full_surface();
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let bridge_file = files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with("Bridge.kt"))
                .unwrap_or(false)
        })
        .expect("DemoBridge.kt must be emitted");
    let bridge_content = &bridge_file.content;
    // Unit return: no `: Unit` or `: <Type>` after the param list.
    assert!(
        bridge_content.contains("external fun nativeDefaultClientCancel(handle: Long, requestJson: String)"),
        "missing Unit-return external fun (no return type annotation): {bridge_content}"
    );
    assert!(
        !bridge_content.contains("nativeDefaultClientCancel(handle: Long, requestJson: String): "),
        "Unit-return external fun must not have a return type annotation: {bridge_content}"
    );

    let client_file = files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "DefaultClient.kt")
                .unwrap_or(false)
        })
        .expect("DefaultClient.kt must be emitted");
    let client_content = &client_file.content;
    assert!(
        client_content.contains("suspend fun cancel("),
        "missing suspend fun cancel: {client_content}"
    );
    // Unit: no `return` or `readValue`.
    assert!(
        !client_content.contains("return withContext"),
        "Unit-return wrapper must not use `return withContext`: {client_content}"
    );
    assert!(
        !client_content.contains("MAPPER.readValue"),
        "Unit-return wrapper must not deserialise response: {client_content}"
    );

    insta::assert_snapshot!("snapshot_jni_unit_return_bridge", bridge_content);
    insta::assert_snapshot!("snapshot_jni_unit_return_client", client_content);
}

// ---------------------------------------------------------------------------
// Pairing-drift sentinel
// ---------------------------------------------------------------------------

/// Build a config that both backends can consume:
/// - `[crates.kotlin]` with `ffi_style = "jni"` → KotlinBackend emits Bridge.kt
/// - `[crates.kotlin_android]` with the same package → JniBackend reads package for symbols
fn make_pairing_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin", "kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin]
package = "dev.sample_crate"
ffi_style = "jni"

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
"#,
    )
}
