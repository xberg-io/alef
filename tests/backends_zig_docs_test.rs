//! Tests for Zig documentation emission in Zig backend.

use alef::backends::zig::ZigBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "test"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn test_zig_doc_emitted_for_function() {
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
            doc: "A test function.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_config();
    let backend = ZigBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    assert!(!files.is_empty());
    let content = &files[0].content;
    assert!(
        content.contains("/// A test function."),
        "Zig doc should appear in output"
    );
}
