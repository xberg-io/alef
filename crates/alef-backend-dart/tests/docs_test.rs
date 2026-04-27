//! Tests for Dartdoc documentation emission in Dart backend.

use alef_backend_dart::DartBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig};
use alef_core::ir::{ApiSurface, FunctionDef, TypeRef};

fn make_config() -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "test".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        gleam: None,
        go: None,
        java: None,
        kotlin: None,
        dart: None,
        swift: None,
        csharp: None,
        r: None,
        zig: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: Default::default(),
        custom_registrations: Default::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: Default::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: Default::default(),
        format: Default::default(),
        format_overrides: std::collections::HashMap::new(),
    }
}

#[test]
fn test_dartdoc_emitted_for_bridge_function() {
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
            FunctionDef {
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
            }
        ],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();
    let backend = DartBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    
    assert!(!files.is_empty());
    let has_doc = files.iter().any(|f| {
        f.content.contains("/// A test function that returns a string.")
    });
    assert!(has_doc, "Dartdoc should appear in generated Dart bridge crate");
}
