use alef_backend_kotlin::KotlinBackend;
use alef_core::backend::Backend;
use alef_core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, TypeDef, TypeRef,
};

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin", "java", "ffi"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.kreuzberg"

[crates.kotlin]
package = "dev.kreuzberg"
target = "jvm"
"#,
    )
}

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
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

fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields,
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
        has_serde: false,
        super_traits: vec![],
    }
}

#[test]
fn struct_emits_data_class() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_type(
            "Point",
            vec![
                make_field("x_coord", TypeRef::Primitive(PrimitiveType::I32), false),
                make_field("y_coord", TypeRef::Primitive(PrimitiveType::I32), false),
            ],
        )],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = KotlinBackend.generate_bindings(&api, &make_config()).unwrap();
    assert_eq!(files.len(), 1);
    let content = &files[0].content;
    // Kotlin emits a `typealias` aliased to the Java facade type so values
    // pass straight through to the JNA bridge without conversion. The actual
    // record fields (xCoord/yCoord) come from the Java side.
    assert!(content.contains("package dev.kreuzberg"), "missing package: {content}");
    assert!(
        content.contains("typealias Point = dev.kreuzberg.Point"),
        "missing typealias for Point: {content}"
    );
}

#[test]
fn function_emits_object_member() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "greet_user".into(),
            rust_path: "demo::greet_user".into(),
            original_rust_path: String::new(),
            params: vec![make_param("user_name", TypeRef::String)],
            return_type: TypeRef::Primitive(PrimitiveType::I32),
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = KotlinBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("object DemoCrate {"),
        "missing object wrapper: {content}"
    );
    assert!(content.contains("fun greetUser(userName: String): Int"));
    assert!(
        content.contains("Bridge.greetUser(userName)"),
        "missing Native bridge call: {content}"
    );
}

#[test]
fn unit_enum_emits_enum_class() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".into(),
            rust_path: "demo::Status".into(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    is_tuple: false,
                },
                EnumVariant {
                    name: "Inactive".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    is_tuple: false,
                },
            ],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,

            is_copy: false,
            has_serde: false,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = KotlinBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("typealias Status = dev.kreuzberg.Status"),
        "missing typealias for Status enum: {content}"
    );
}

#[test]
fn optional_field_uses_kotlin_nullable() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_type(
            "Maybe",
            vec![make_field("value", TypeRef::Optional(Box::new(TypeRef::String)), false)],
        )],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = KotlinBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    // Optional fields are owned by the Java record; Kotlin only emits a typealias.
    assert!(
        content.contains("typealias Maybe = dev.kreuzberg.Maybe"),
        "missing typealias for Maybe: {content}"
    );
}

#[test]
fn async_function_emits_suspend() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "fetch".into(),
            rust_path: "demo::fetch".into(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: true,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = KotlinBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(content.contains("suspend fun fetch()"), "missing suspend: {content}");
    assert!(
        content.contains("withContext(Dispatchers.IO)"),
        "missing withContext: {content}"
    );
    assert!(
        content.contains("Bridge.fetch()"),
        "missing Native bridge call: {content}"
    );
}

#[test]
fn unit_error_variant_emits_sealed_class() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "ApiError".into(),
            rust_path: "demo::ApiError".into(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "NotFound".into(),
                    message_template: Some("Resource not found".into()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    doc: String::new(),
                },
                ErrorVariant {
                    name: "Timeout".into(),
                    message_template: Some("Request timed out".into()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    doc: String::new(),
                },
            ],
            doc: String::new(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = KotlinBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    // Errors alias the Java exception type with the `Exception` suffix to avoid
    // collision with same-named non-error structs in `api.types`.
    assert!(
        content.contains("typealias ApiErrorException = dev.kreuzberg.ApiErrorException"),
        "missing error typealias: {content}"
    );
}

#[test]
fn error_variant_with_fields_emits_data_class() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "ParseError".into(),
            rust_path: "demo::ParseError".into(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "InvalidFormat".into(),
                message_template: Some("Invalid format at line {0}".into()),
                fields: vec![make_field("line_number", TypeRef::Primitive(PrimitiveType::I32), false)],
                has_source: false,
                has_from: false,
                is_unit: false,
                doc: String::new(),
            }],
            doc: String::new(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = KotlinBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("typealias ParseErrorException = dev.kreuzberg.ParseErrorException"),
        "missing error typealias: {content}"
    );
}

#[test]
fn function_imports_native_facade() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = KotlinBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("import dev.kreuzberg.DemoCrate as Bridge"),
        "missing Java facade import alias: {content}"
    );
}

/// Streaming adapters (pattern = `streaming`) owned by a client type must appear
/// as `Flow<T>` methods (using `callbackFlow`) on the generated Kotlin
/// `DefaultClient` class. The previous implementation emitted an `Iterator<T>`
/// delegation; it is now replaced with a coroutine-native `Flow<T>` wrapper that
/// calls the three JNI native methods (`native{Owner}{Adapter}Start/Next/Free`)
/// emitted on the Java facade class.
#[test]
fn streaming_adapter_emits_flow_method_on_client_class() {
    // Config with a streaming adapter owned by DefaultClient.
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin", "java", "ffi"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.kreuzberg"

[crates.kotlin]
package = "dev.kreuzberg"
target = "jvm"

[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "chat_stream"
owner_type = "DefaultClient"
item_type = "ChatCompletionChunk"

[[crates.adapters.params]]
name = "req"
type = "ChatCompletionRequest"
"#,
    );

    // Minimal API surface: one opaque client type with a non-sanitized async method
    // so `emit_jvm_client_class` creates a DefaultClient wrapper class.
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
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    // DefaultClient.kt is a second generated file alongside LiterLlm.kt.
    let client_file = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("DefaultClient.kt"));
    let content = client_file.map(|f| f.content.as_str()).unwrap_or("");
    // Streaming method emits a callbackFlow wrapper.
    assert!(
        content.contains("fun chatStream("),
        "expected chatStream method on DefaultClient: {content}"
    );
    // Return type is Flow<T>, not Iterator<T>.
    assert!(
        content.contains("Flow<ChatCompletionChunk>"),
        "expected Flow<ChatCompletionChunk> return type: {content}"
    );
    assert!(
        !content.contains("Iterator<ChatCompletionChunk>"),
        "must not emit Iterator<ChatCompletionChunk> any more: {content}"
    );
    // callbackFlow is the wrapper mechanism.
    assert!(
        content.contains("callbackFlow"),
        "expected callbackFlow in chatStream: {content}"
    );
    // JNI start/next/free are called via Bridge.
    assert!(
        content.contains("Bridge.nativeDefaultClientChatStreamStart("),
        "expected nativeDefaultClientChatStreamStart call: {content}"
    );
    assert!(
        content.contains("Bridge.nativeDefaultClientChatStreamNext("),
        "expected nativeDefaultClientChatStreamNext call: {content}"
    );
    assert!(
        content.contains("Bridge.nativeDefaultClientChatStreamFree("),
        "expected nativeDefaultClientChatStreamFree call in awaitClose: {content}"
    );
    // awaitClose is used for resource cleanup.
    assert!(
        content.contains("awaitClose"),
        "expected awaitClose in chatStream: {content}"
    );
    // Streaming methods must NOT be suspend — they return Flow.
    assert!(
        !content.contains("suspend fun chatStream"),
        "chatStream must not be suspend: {content}"
    );
    // Flow imports are present.
    assert!(
        content.contains("import kotlinx.coroutines.flow.Flow"),
        "expected Flow import: {content}"
    );
    assert!(
        content.contains("import kotlinx.coroutines.flow.callbackFlow"),
        "expected callbackFlow import: {content}"
    );
    assert!(
        content.contains("import kotlinx.coroutines.channels.awaitClose"),
        "expected awaitClose import: {content}"
    );
}

/// Snapshot the generated `DefaultClient.kt` for a streaming adapter so that
/// the exact emitted source is pinned and regressions are caught.
#[test]
fn snapshot_streaming_flow_default_client_kt() {
    use alef_core::ir::MethodDef;

    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin", "java", "ffi"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.kreuzberg"

[crates.kotlin]
package = "dev.kreuzberg"
target = "jvm"

[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "chat_stream"
owner_type = "DefaultClient"
item_type = "ChatCompletionChunk"

[[crates.adapters.params]]
name = "req"
type = "ChatCompletionRequest"
"#,
    );

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
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let client_file = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("DefaultClient.kt"))
        .expect("DefaultClient.kt must be emitted");

    insta::assert_snapshot!("snapshot_streaming_flow_default_client_kt", &client_file.content);
}
