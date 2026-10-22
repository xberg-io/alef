//! The generated Ruby and PHP bindings raise typed errors from the calls the product makes, and
//! refuse JSON text that does not parse. Each test generates a whole binding file and reads the
//! call sites, not the helper that builds the exception.

use alef::backends::magnus::MagnusBackend;
use alef::backends::php::PhpBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::*;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn ruby_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"
"#,
    )
}

fn php_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "test_lib"
"#,
    )
}

fn method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type,
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    }
}

fn client_error() -> ErrorDef {
    ErrorDef {
        exception_base: None,
        name: "ClientError".to_string(),
        rust_path: "test_lib::ClientError".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            ErrorVariant {
                name: "RateLimited".to_string(),
                message_template: Some("rate limited".to_string()),
                is_unit: true,
                ..Default::default()
            },
            ErrorVariant {
                name: "Timeout".to_string(),
                message_template: Some("timeout".to_string()),
                is_unit: true,
                ..Default::default()
            },
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
    }
}

fn param(name: &str, ty: TypeRef, optional: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional,
        ..Default::default()
    }
}

fn api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        errors: vec![client_error()],
        functions: vec![
            FunctionDef {
                name: "ping".to_string(),
                rust_path: "test_lib::ping".to_string(),
                return_type: TypeRef::String,
                error_type: Some("ClientError".to_string()),
                ..Default::default()
            },
            FunctionDef {
                name: "connect".to_string(),
                rust_path: "test_lib::connect".to_string(),
                params: vec![
                    param("key", TypeRef::String, false),
                    param("timeout_secs", TypeRef::Primitive(PrimitiveType::U64), true),
                    param("retries", TypeRef::Primitive(PrimitiveType::U32), true),
                ],
                return_type: TypeRef::String,
                error_type: Some("ClientError".to_string()),
                ..Default::default()
            },
            FunctionDef {
                name: "send_payload".to_string(),
                rust_path: "test_lib::send_payload".to_string(),
                params: vec![
                    param("payload", TypeRef::Json, false),
                    param("extra", TypeRef::Json, true),
                ],
                return_type: TypeRef::String,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn generated(backend: &dyn Backend, config: &ResolvedCrateConfig, file: &str) -> String {
    backend
        .generate_bindings(&api(), config)
        .expect("generate bindings")
        .into_iter()
        .find(|f| f.path.ends_with(file))
        .unwrap_or_else(|| panic!("generated {file}"))
        .content
}

#[test]
fn ruby_core_calls_raise_the_typed_exception() {
    let lib = generated(&MagnusBackend, &ruby_config(), "lib.rs");
    assert!(
        lib.contains("fn alef_magnus_err<E"),
        "dispatch function missing:\n{lib}"
    );
    assert!(lib.contains("fn client_error_to_magnus_err("), "{lib}");
    assert!(
        lib.contains("let result = test_lib::ping().map_err(alef_magnus_err)?;"),
        "the function call must route its error through the dispatch:\n{lib}"
    );
    assert!(lib.contains("define_error(\"RateLimitedError\""), "{lib}");
    assert!(lib.contains("define_attr(\"status_code\""), "{lib}");
    assert!(lib.contains("define_attr(\"retry_after\""), "{lib}");
    assert!(
        !lib.contains("e.to_string())).map_err"),
        "no call may keep the message-only RuntimeError:\n{lib}"
    );
}

#[test]
fn ruby_optional_primitive_arguments_accept_nil() {
    let lib = generated(&MagnusBackend, &ruby_config(), "lib.rs");
    // Control: the optional string and the optional primitives are scanned the same way.
    assert!(
        lib.contains("scan_args::<(String,), (Option<magnus::Value>, Option<magnus::Value>,), (), (), (), ()>"),
        "{lib}"
    );
    assert!(
        lib.contains("Some(v) if !v.is_nil() => Some(u64::try_convert(v)?)"),
        "an explicit nil for an optional integer must read as absent:\n{lib}"
    );
    assert!(
        lib.contains("Some(v) if !v.is_nil() => Some(u32::try_convert(v)?)"),
        "{lib}"
    );
}

#[test]
fn php_core_calls_throw_the_typed_exception() {
    let lib = generated(&PhpBackend, &php_config(), "lib.rs");
    assert!(lib.contains("fn alef_php_err<E"), "dispatch function missing:\n{lib}");
    assert!(lib.contains("fn client_error_to_php_err("), "{lib}");
    assert!(
        lib.contains("test_lib::ping().map_err(client_error_to_php_err)"),
        "the function call must route its error through the dispatch:\n{lib}"
    );
    assert!(lib.contains("info_status_code"), "{lib}");
    assert!(lib.contains("#[php(name = \"statusCode\")]"), "{lib}");
    assert!(
        lib.contains("TestLibException"),
        "the root exception class is missing:\n{lib}"
    );
    assert!(!lib.contains("PhpException::from_message(e.to_string()))"), "{lib}");
}

#[test]
fn php_json_parameters_refuse_text_that_does_not_parse() {
    let lib = generated(&PhpBackend, &php_config(), "lib.rs");
    assert!(
        lib.contains("invalid JSON in `payload`") && lib.contains("invalid JSON in `extra`"),
        "the refusal must name each parameter:\n{lib}"
    );
    assert!(
        lib.contains("send_payload"),
        "control: the function is generated:\n{lib}"
    );
    assert!(
        !lib.contains("serde_json::from_str(&payload).unwrap_or_default()"),
        "a Json parameter must not default silently:\n{lib}"
    );
    assert!(!lib.contains("from_str(s).ok()"), "{lib}");
}
