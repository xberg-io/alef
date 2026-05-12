//! Tests for Gleam documentation emission in Gleam backend.

use alef_backend_gleam::GleamBackend;
use alef_core::backend::Backend;
use alef_core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef_core::ir::{ApiSurface, FunctionDef, TypeRef};

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["gleam"]

[[crates]]
name = "test"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn test_gleam_doc_emitted_for_function() {
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "test_func".to_string(),
            rust_path: "test::test_func".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "This is a test function.".to_string(),
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

    let config = make_config();
    let backend = GleamBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    assert!(!files.is_empty());
    let content = &files[0].content;
    assert!(
        content.contains("/// This is a test function."),
        "Gleam doc should appear in output"
    );
}
