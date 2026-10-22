//! Elixir typed errors: a NIF that fails with a core error returns `{:error, %App.Error{}}`
//! whose `kind`, `status` and `retry_after_ms` fields carry the values the Rust error has.
//! Callers match on those fields, never on the message text.

use alef::backends::rustler::RustlerBackend;
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::*;

fn config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "test_lib"

[[crates.adapters]]
name = "open_stream"
pattern = "streaming"
core_path = "open_stream"
owner_type = "Client"
item_type = "Chunk"
error_type = "test_lib::TestLibError"

[[crates.adapters.params]]
name = "request"
type = "String"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type,
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    }
}

fn function(name: &str, is_async: bool) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        return_type: TypeRef::String,
        is_async,
        error_type: Some("TestLibError".to_string()),
        ..Default::default()
    }
}

fn variant(name: &str) -> ErrorVariant {
    ErrorVariant {
        name: name.to_string(),
        error_code: None,
        fields: vec![],
        doc: String::new(),
        message_template: Some(name.to_string()),
        has_source: false,
        has_from: false,
        is_unit: true,
        is_tuple: false,
    }
}

fn api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Client".to_string(),
            rust_path: "test_lib::Client".to_string(),
            is_opaque: true,
            is_clone: true,
            methods: vec![MethodDef {
                is_async: true,
                error_type: Some("TestLibError".to_string()),
                ..method("chat", TypeRef::String)
            }],
            ..Default::default()
        }],
        functions: vec![function("failing", true), function("failing_sync", false)],
        errors: vec![ErrorDef {
            exception_base: None,
            name: "TestLibError".to_string(),
            rust_path: "test_lib::TestLibError".to_string(),
            original_rust_path: String::new(),
            variants: vec![variant("RateLimited"), variant("NetworkFailure")],
            doc: String::new(),
            methods: vec![
                method("status_code", TypeRef::Primitive(PrimitiveType::U16)),
                method("retry_after", TypeRef::Optional(Box::new(TypeRef::Duration))),
            ],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        ..Default::default()
    }
}

fn native_lib() -> String {
    let files = RustlerBackend.generate_bindings(&api(), &config()).unwrap();
    files
        .into_iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("native lib.rs")
        .content
}

fn public_module() -> String {
    let files: Vec<GeneratedFile> = RustlerBackend.generate_public_api(&api(), &config()).unwrap();
    files
        .into_iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "test_lib.ex"))
        .expect("test_lib.ex")
        .content
}

#[test]
fn the_nif_error_carries_kind_status_and_retry_delay_as_fields() {
    let lib = native_lib();
    assert!(
        lib.contains("enum AlefNifError"),
        "the NIF error type must exist:\n{lib}"
    );
    assert!(
        lib.contains("\"Elixir.TestLib.Error\""),
        "the term must name the exception module of the app:\n{lib}"
    );
    for atom in [
        "__struct__",
        "__exception__",
        "kind",
        "message",
        "status",
        "retry_after_ms",
    ] {
        assert!(
            lib.contains(&format!("\"{atom}\"")),
            "the encoded exception must have the {atom} key:\n{lib}"
        );
    }
    assert!(
        lib.contains("TestLibError::RateLimited") && lib.contains("\"rate_limited\""),
        "the kind is the snake_case variant name:\n{lib}"
    );
    assert!(lib.contains("\"network_failure\""), "every variant has a kind:\n{lib}");
    assert!(
        lib.contains("e.status_code()"),
        "the status comes from the error's status_code method:\n{lib}"
    );
    assert!(
        lib.contains("e.retry_after().map(|d| d.as_millis() as u64)"),
        "the retry delay is whole milliseconds:\n{lib}"
    );
}

#[test]
fn the_shared_conversion_routes_each_core_error_to_its_typed_converter() {
    let lib = native_lib();
    assert!(
        lib.contains("fn alef_nif_err<E: std::fmt::Display + 'static>(e: E) -> AlefNifError"),
        "the shared conversion must exist:\n{lib}"
    );
    assert!(
        lib.contains("boxed.downcast::<test_lib::TestLibError>()")
            && lib.contains("return test_lib_error_to_nif_err(*typed)"),
        "the shared conversion must route TestLibError to its converter:\n{lib}"
    );
}

#[test]
fn every_fallible_call_site_converts_through_the_shared_conversion() {
    let lib = native_lib();
    assert!(
        lib.contains("-> Result<String, AlefNifError>"),
        "fallible NIFs must return the typed error:\n{lib}"
    );
    assert!(
        !lib.contains("-> Result<String, String>"),
        "no fallible NIF may keep the string error:\n{lib}"
    );
    assert!(
        lib.contains(".map_err(alef_nif_err)"),
        "call sites must use the shared conversion:\n{lib}"
    );
    for name in ["failing", "failing_sync"] {
        let start = lib
            .find(&format!("pub fn {name}("))
            .unwrap_or_else(|| panic!("{name} nif:\n{lib}"));
        let body = &lib[start..];
        let end = body[1..].find("#[rustler::nif").map_or(body.len(), |i| i + 1);
        let body = &body[..end];
        assert!(
            body.contains(".map_err(alef_nif_err)"),
            "the core call of {name} must convert through the shared conversion:\n{body}"
        );
    }
}

#[test]
fn the_stream_nifs_convert_core_errors_through_the_shared_conversion() {
    let lib = native_lib();
    let start = lib.find("fn client_open_stream_start").expect("stream start nif");
    let next = lib.find("fn client_open_stream_next").expect("stream next nif");
    let stream = &lib[start..];
    assert!(
        stream.contains("-> std::result::Result<rustler::ResourceArc<ClientOpenStreamHandle>, AlefNifError>"),
        "the start NIF must return the typed error:\n{stream}"
    );
    assert!(
        stream[next - start..].contains("Err(alef_nif_err(e))"),
        "a mid-stream core error must keep its type:\n{stream}"
    );
}

#[test]
fn the_package_defines_the_error_exception() {
    let module = public_module();
    assert!(module.contains("defmodule TestLib.Error do"), "{module}");
    assert!(
        module.contains("defexception [:kind, :message, :status, :retry_after_ms]"),
        "the exception must declare the typed fields:\n{module}"
    );
}

#[test]
fn the_stream_error_copies_the_typed_fields_from_the_reason() {
    let module = public_module();
    assert!(
        module.contains("defexception [:message, :reason, :adapter, :kind, :status, :retry_after_ms]"),
        "StreamError must declare the typed fields:\n{module}"
    );
    assert!(
        module.contains("kind: Map.get(fields, :kind)")
            && module.contains("status: Map.get(fields, :status)")
            && module.contains("retry_after_ms: Map.get(fields, :retry_after_ms)"),
        "the raise must copy the fields from the reason:\n{module}"
    );
}

#[test]
fn a_raising_wrapper_re_raises_a_typed_error_instead_of_wrapping_it_in_argument_error() {
    let module = public_module();
    assert!(
        module.contains("{:error, %{__exception__: true} = error} -> raise error"),
        "an exception struct must be raised as is:\n{module}"
    );
}
