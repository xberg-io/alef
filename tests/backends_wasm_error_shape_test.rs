//! WASM error shape and stream refusal.
//!
//! A call that fails must reject with an `Error` that carries the kind, the HTTP status and the
//! retry delay under the same property names the Node binding uses. A streaming call must reject
//! when the server refuses the stream, not on the first `next()` call. These tests read the
//! generated Rust source; the regen job in the pull request runs the built package under Node.

use alef::backends::wasm::WasmBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, ResolvedCrateConfig};
use alef::core::ir::*;

fn config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.wasm]
"#,
    )
    .unwrap();
    let mut resolved = cfg.resolve().unwrap().remove(0);
    resolved.adapters = vec![AdapterConfig {
        name: "open_stream".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "open_stream".to_string(),
        params: Vec::new(),
        returns: None,
        error_type: Some("ProcessError".to_string()),
        owner_type: Some("Client".to_string()),
        item_type: Some("Chunk".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: Some("test_lib::Request".to_string()),
        skip_languages: Vec::new(),
    }];
    resolved
}

fn method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type,
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    }
}

fn fallible_method(name: &str, is_async: bool) -> MethodDef {
    MethodDef {
        is_async,
        error_type: Some("ProcessError".to_string()),
        ..method(name, TypeRef::String)
    }
}

fn function(name: &str, is_async: bool) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        return_type: TypeRef::String,
        is_async,
        error_type: Some("ProcessError".to_string()),
        ..Default::default()
    }
}

fn unit_variant(name: &str, message: &str) -> ErrorVariant {
    ErrorVariant {
        name: name.to_string(),
        error_code: None,
        fields: vec![],
        doc: String::new(),
        message_template: Some(message.to_string()),
        has_source: false,
        has_from: false,
        is_unit: true,
        is_tuple: false,
    }
}

fn api() -> ApiSurface {
    let mut open_stream = fallible_method("open_stream", true);
    open_stream.return_type = TypeRef::Unit;
    open_stream.params = vec![ParamDef {
        name: "req".to_string(),
        ty: TypeRef::Named("Request".to_string()),
        ..ParamDef::default()
    }];
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "Client".to_string(),
                rust_path: "test_lib::Client".to_string(),
                is_opaque: true,
                is_clone: true,
                methods: vec![
                    fallible_method("get_async", true),
                    fallible_method("get_sync", false),
                    open_stream,
                ],
                ..Default::default()
            },
            TypeDef {
                name: "Request".to_string(),
                rust_path: "test_lib::Request".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "url".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..Default::default()
            },
            TypeDef {
                name: "Chunk".to_string(),
                rust_path: "test_lib::Chunk".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "text".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..Default::default()
            },
        ],
        functions: vec![function("failing", true), function("failing_sync", false)],
        errors: vec![ErrorDef {
            exception_base: None,
            name: "ProcessError".to_string(),
            rust_path: "test_lib::ProcessError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                unit_variant("RateLimited", "rate limited"),
                unit_variant("Timeout", "timeout"),
            ],
            doc: String::new(),
            methods: vec![
                method("status_code", TypeRef::Primitive(PrimitiveType::U16)),
                method("is_transient", TypeRef::Primitive(PrimitiveType::Bool)),
                method("error_type", TypeRef::String),
                method("retry_after", TypeRef::Optional(Box::new(TypeRef::Duration))),
            ],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        ..Default::default()
    }
}

fn lib_rs() -> String {
    let files = WasmBackend.generate_bindings(&api(), &config()).unwrap();
    files
        .into_iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs")
        .content
}

/// The text of the generated function whose signature starts with `signature`, up to its closing
/// brace at the same indentation.
fn function_body<'a>(lib: &'a str, signature: &str) -> &'a str {
    let start = lib
        .find(signature)
        .unwrap_or_else(|| panic!("`{signature}` must be generated:\n{lib}"));
    let rest = &lib[start..];
    let indent: String = lib[..start]
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let end = rest.find(&format!("\n{indent}}}\n")).unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn every_call_site_converts_errors_through_the_typed_converter() {
    let lib = lib_rs();
    assert!(
        lib.contains("fn alef_wasm_err<E: std::fmt::Display + 'static>(e: E) -> wasm_bindgen::JsValue"),
        "the shared conversion must exist:\n{lib}"
    );
    assert!(
        lib.contains("boxed.downcast::<test_lib::ProcessError>()")
            && lib.contains("return process_error_to_js_value(*typed)"),
        "the shared conversion must route ProcessError to its typed converter:\n{lib}"
    );
    for signature in [
        "pub async fn get_async(",
        "pub fn get_sync(",
        "pub async fn failing(",
        "pub fn failing_sync(",
    ] {
        let body = function_body(&lib, signature);
        assert!(
            body.contains(".map_err(alef_wasm_err)"),
            "`{signature}` must convert its error through the shared conversion:\n{body}"
        );
        assert!(
            !body.contains("JsValue::from_str(&e.to_string())"),
            "`{signature}` must not reject with a bare string:\n{body}"
        );
    }
}

#[test]
fn the_converter_builds_an_error_with_the_node_property_names() {
    let lib = lib_rs();
    let converter = function_body(&lib, "fn process_error_to_js_value(");
    assert!(
        converter.contains("js_sys::Error::new(&message)"),
        "the rejection must be an Error instance:\n{converter}"
    );
    for name in [
        "kind",
        "errorCode",
        "statusCode",
        "isTransient",
        "errorType",
        "retryAfter",
    ] {
        assert!(
            converter.contains(&format!("&\"{name}\".into()")),
            "the converter must set `{name}`, the name the Node binding uses:\n{converter}"
        );
    }
    assert!(
        converter.contains("e.retry_after().map(|d| d.as_millis() as i64)"),
        "retryAfter must be whole milliseconds, null when absent:\n{converter}"
    );
    assert!(
        converter.contains("\"RateLimited\""),
        "kind must be the variant name the Rust error has:\n{converter}"
    );
}

#[test]
fn the_stream_call_opens_the_stream_before_it_resolves() {
    let lib = lib_rs();
    let body = function_body(&lib, "pub async fn open_stream(");
    let open = body
        .find("inner.open_stream(")
        .unwrap_or_else(|| panic!("the stream call must open the stream:\n{body}"));
    let spawn = body
        .find("spawn_local")
        .unwrap_or_else(|| panic!("the forward loop must run in a spawned task:\n{body}"));
    assert!(
        open < spawn,
        "the stream must be open before the forward task starts, so a refusal rejects the call:\n{body}"
    );
    assert_eq!(
        body.matches("inner.open_stream(").count(),
        1,
        "the stream must be opened once:\n{body}"
    );
    assert!(
        body[open..spawn].contains(".await") && body[open..spawn].contains("alef_wasm_err"),
        "the open must be awaited and its error converted before the task starts:\n{body}"
    );
}

#[test]
fn a_stream_cut_mid_reply_rejects_next_with_the_typed_error() {
    let lib = lib_rs();
    let iterator = function_body(&lib, "pub struct OpenStreamIterator");
    assert!(
        iterator.contains("Result<WasmChunk, JsValue>"),
        "the iterator must carry typed errors, not strings:\n{iterator}"
    );
    assert!(
        !lib.contains("Some(Err(e)) => Err(JsValue::from_str(&e))"),
        "next() must not rebuild a string error:\n{lib}"
    );
    let body = function_body(&lib, "pub async fn open_stream(");
    assert!(
        body.contains("tx.send(Err(alef_wasm_err(e)))") && !body.contains("tx.send(Err(e.to_string()))"),
        "a mid-stream error must be converted with the shared conversion:\n{body}"
    );
}
