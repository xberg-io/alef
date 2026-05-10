//! Tests for KDoc documentation emission in Kotlin backend.

use alef_backend_kotlin::KotlinBackend;
use alef_core::backend::Backend;
use alef_core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef_core::ir::{ApiSurface, FunctionDef, TypeRef};

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin", "ffi"]

[[crates]]
name = "test"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.kotlin]
package = "dev.test"
target = "jvm"
"#,
    )
}

#[test]
fn test_kdoc_emitted_for_function() {
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "do_something".to_string(),
            rust_path: "test::do_something".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: None,
            doc: "This function does something useful.".to_string(),
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
    let backend = KotlinBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    assert!(!files.is_empty());
    let content = &files[0].content;
    assert!(content.contains("/**"), "KDoc opening should be present");
    assert!(
        content.contains("This function does something useful."),
        "Function doc should appear in output"
    );
    assert!(content.contains("*/"), "KDoc closing should be present");
}
