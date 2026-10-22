//! A Swift caller catches the typed error of the Rust crate, not a bare `RustString`.
//!
//! The error carries the kind, the HTTP status and the retry delay of the Rust error. The Rust
//! bridge crate builds one `AlefBridgeError` for every failed call, and the Swift facade converts
//! it once, in `alefBridgedCall` / `alefBridgedCallAsync`, before it reaches the caller.

use alef::backends::swift::SwiftBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType, TypeRef,
};

fn config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

fn variant(name: &str, fields: Vec<FieldDef>) -> ErrorVariant {
    ErrorVariant {
        name: name.to_string(),
        message_template: Some(name.to_string()),
        is_unit: fields.is_empty(),
        fields,
        ..ErrorVariant::default()
    }
}

fn method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type,
        receiver: Some(alef::core::ir::ReceiverKind::Ref),
        ..MethodDef::default()
    }
}

/// An error shaped like a client library error: one variant carries a status, one a retry delay,
/// one carries neither, and the type reports `status_code` and `retry_after`.
fn api() -> ApiSurface {
    let error = ErrorDef {
        name: "LlmError".to_string(),
        rust_path: "demo::LlmError".to_string(),
        variants: vec![
            variant(
                "Authentication",
                vec![
                    field("message", TypeRef::String),
                    field("status", TypeRef::Primitive(PrimitiveType::U16)),
                ],
            ),
            variant(
                "RateLimited",
                vec![
                    field("message", TypeRef::String),
                    field("retry_after", TypeRef::Optional(Box::new(TypeRef::Duration))),
                ],
            ),
            variant("ContextWindowExceeded", vec![field("message", TypeRef::String)]),
            variant("Timeout", vec![]),
        ],
        methods: vec![
            method("status_code", TypeRef::Primitive(PrimitiveType::U16)),
            method("retry_after", TypeRef::Optional(Box::new(TypeRef::Duration))),
        ],
        ..ErrorDef::default()
    };
    let function = |name: &str, is_async: bool| FunctionDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        params: vec![ParamDef {
            name: "prompt".to_string(),
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
            core_wrapper: CoreWrapper::None,
        }],
        return_type: TypeRef::String,
        is_async,
        error_type: Some("LlmError".to_string()),
        ..FunctionDef::default()
    };
    ApiSurface {
        crate_name: "demo".to_string(),
        version: "0.1.0".to_string(),
        functions: vec![function("chat", false), function("chat_later", true)],
        errors: vec![error],
        ..ApiSurface::default()
    }
}

fn generated() -> Vec<alef::core::backend::GeneratedFile> {
    SwiftBackend.generate_bindings(&api(), &config()).unwrap()
}

fn file<'a>(files: &'a [alef::core::backend::GeneratedFile], name: &str) -> &'a str {
    &files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some(name))
        .unwrap_or_else(|| panic!("{name} must be generated"))
        .content
}

#[test]
fn the_bridge_crate_reads_kind_status_and_retry_delay_from_the_rust_error() {
    let files = generated();
    let lib = file(&files, "lib.rs");

    assert!(lib.contains("type AlefBridgeError;"), "{lib}");
    assert!(
        lib.contains("fn chat(prompt: String) -> Result<String, AlefBridgeError>;"),
        "a fallible bridge function must return the bridge error, not a String: {lib}"
    );
    assert!(
        lib.contains("demo::LlmError::RateLimited { .. } => \"RateLimited\""),
        "the kind is the variant name of the Rust error: {lib}"
    );
    assert!(
        lib.contains("status: i32::from(e.status_code())"),
        "the status comes from the Rust error: {lib}"
    );
    assert!(
        lib.contains("retry_after_ms: e.retry_after().map_or(-1, |d| i64::try_from(d.as_millis())"),
        "the retry delay comes from the Rust error in milliseconds: {lib}"
    );
    assert!(
        lib.contains("demo::chat(prompt).map_err(alef_bridge_error)"),
        "the shim must convert the core error with the central function: {lib}"
    );
}

#[test]
fn the_swift_error_is_built_from_the_bridge_error_with_status_and_retry_delay() {
    let files = generated();
    let swift = &files[0].content;

    assert!(
        swift.contains("init(bridgeError error: RustBridge.AlefBridgeError)"),
        "{swift}"
    );
    assert!(
        swift.contains("self = .authentication(message: message, status: UInt16(clamping: status))"),
        "a variant with a status field gets the status of the bridge error: {swift}"
    );
    assert!(
        swift.contains(
            "self = .rateLimited(message: message, retryAfter: retryAfterMs < 0 ? nil : Duration.milliseconds(retryAfterMs), status: UInt16(clamping: status))"
        ),
        "a variant with a retry delay gets it, and a variant without a status field gets one: {swift}"
    );
    assert!(
        swift.contains("self = .timeout(status: UInt16(clamping: status))"),
        "a unit variant keeps its case and gains the status: {swift}"
    );
    assert!(
        swift.contains("public var kind: String"),
        "the caller reads the kind of the error: {swift}"
    );
    assert!(
        swift.contains("case .rateLimited: return \"RateLimited\""),
        "the kind is the variant name of the Rust error: {swift}"
    );
}

#[test]
fn every_bridge_call_goes_through_the_conversion_helpers() {
    let files = generated();
    let swift = &files[0].content;

    assert!(
        swift.contains("func alefBridgedCall<T>(_ body: () throws -> T) throws -> T"),
        "{swift}"
    );
    assert!(
        swift.contains("func alefBridgedCallAsync<T>(_ body: () async throws -> T) async throws -> T"),
        "{swift}"
    );
    assert!(
        swift.contains("try alefBridgedCall { try RustBridge.chat(_rb_prompt) }"),
        "a synchronous forwarder must convert the thrown bridge error: {swift}"
    );
    assert!(
        swift.contains("try await alefBridgedCallAsync { try await RustBridge.chatLater(_rb_prompt) }"),
        "an asynchronous forwarder must convert the thrown bridge error: {swift}"
    );
    assert!(
        !swift.contains("try RustBridge.chat(") && !swift.contains("try await RustBridge.chatLater("),
        "no fallible call may reach the caller without the conversion: {swift}"
    );
}

#[test]
fn the_bridge_error_is_a_swift_error() {
    let files = generated();
    let conformance = file(&files, "ZSwiftBridgeError.swift");

    assert!(
        conformance.contains("extension AlefBridgeError: Swift.Error, @unchecked Sendable"),
        "{conformance}"
    );
}
