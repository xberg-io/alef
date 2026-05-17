//! Tests for Dartdoc documentation emission in Dart backend.

use alef_backend_dart::DartBackend;
use alef_core::backend::Backend;
use alef_core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef_core::ir::{ApiSurface, FunctionDef, TypeRef};

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "test"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn test_dartdoc_emitted_for_bridge_function() {
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
            doc: "A test function that returns a string.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };

    let config = make_config();
    let backend = DartBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    assert!(!files.is_empty());
    let has_doc = files
        .iter()
        .any(|f| f.content.contains("/// A test function that returns a string."));
    assert!(has_doc, "Dartdoc should appear in generated Dart bridge crate");
}
