use alef::backends::csharp::CsharpBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};

fn make_csharp_config_with_capsule() -> ResolvedCrateConfig {
    let toml_str = r#"
[workspace]
languages = ["csharp", "ffi"]

[[crates]]
name = "tree_sitter"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "tree_sitter"
error_style = "last_error"

[crates.csharp]
namespace = "TreeSitter"

[crates.csharp.capsule_types.Language]
host_type = "TreeSitter.Language"
package = "TreeSitter.DotNet"
package_version = "0.8.0"
construct_expr = "new TreeSitter.Language({ptr})"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_str).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_csharp_config_with_default_capsule() -> ResolvedCrateConfig {
    let toml_str = r#"
[workspace]
languages = ["csharp", "ffi"]

[[crates]]
name = "tree_sitter"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "tree_sitter"
error_style = "last_error"

[crates.csharp]
namespace = "TreeSitter"

[crates.csharp.capsule_types.Language]
host_type = "TreeSitter.Language"
package = "TreeSitter.DotNet"
package_version = "0.8.0"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_str).unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn test_csharp_capsule_function_generation() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "tree_sitter".to_string(),
        version: "0.25.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "language_rust".to_string(),
            rust_path: "tree_sitter::language_rust".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Named("Language".to_string()),
            error_type: None,
            is_async: false,
            doc: "Get the Rust language grammar.".to_string(),
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
        services: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };

    let config = make_csharp_config_with_capsule();
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate C# bindings");

    let files = result.unwrap();

    let wrapper_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TreeSitterConverter.cs"))
        .expect("TreeSitterConverter.cs wrapper not found");

    let content = &wrapper_file.content;

    assert!(
        content.contains("public static TreeSitter.Language LanguageRust"),
        "Capsule wrapper method signature not found. Content:\n{}",
        content
    );

    assert!(
        content.contains("new TreeSitter.Language(nativeResult)"),
        "Capsule construction not found. Content:\n{}",
        content
    );

    assert!(
        content.contains("if (nativeResult == IntPtr.Zero)"),
        "Null guard not found. Content:\n{}",
        content
    );
}

#[test]
fn test_csharp_capsule_requires_construct_expr() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "tree_sitter".to_string(),
        version: "0.25.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "language_python".to_string(),
            rust_path: "tree_sitter::language_python".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Named("Language".to_string()),
            error_type: None,
            is_async: false,
            doc: "Get the Python language grammar.".to_string(),
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
        services: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };

    let config = make_csharp_config_with_default_capsule();
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate C# bindings");

    let files = result.unwrap();
    let wrapper_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TreeSitterConverter.cs"))
        .expect("TreeSitterConverter.cs wrapper not found");

    let content = &wrapper_file.content;

    assert!(
        content.contains("ALEF ERROR") && content.contains("construct_expr"),
        "Missing construct_expr should emit a generated diagnostic. Content:\n{}",
        content
    );
}
