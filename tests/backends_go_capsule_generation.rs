use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};

fn make_go_config_with_capsule() -> ResolvedCrateConfig {
    let toml_str = r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "tree_sitter"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "tree_sitter"
error_style = "last_error"

[crates.go]
module = "github.com/test/tree-sitter"

[crates.go.capsule_types.Language]
host_type = "*tree_sitter.Language"
package = "github.com/tree-sitter/go-tree-sitter"
package_version = "v0.24.0"
construct_expr = "tree_sitter.NewLanguage(unsafe.Pointer({ptr}))"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_str).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn capsule_function() -> FunctionDef {
    FunctionDef {
        name: "get_language".to_string(),
        rust_path: "tree_sitter::get_language".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "name".to_string(),
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
            core_wrapper: alef::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Named("Language".to_string()),
        error_type: Some("Error".to_string()),
        is_async: false,
        doc: "Get a language grammar by name.".to_string(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_api() -> ApiSurface {
    ApiSurface {
        crate_name: "tree_sitter".to_string(),
        version: "0.25.0".to_string(),
        types: vec![],
        functions: vec![capsule_function()],
        enums: vec![],
        errors: vec![],
        services: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    }
}

/// A fallible capsule function must declare the host return type and construct it.
#[test]
fn go_capsule_wrapper_constructs_host_type() {
    let files = GoBackend
        .generate_bindings(&make_api(), &make_go_config_with_capsule())
        .unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go not generated");
    let content = &binding.content;

    assert!(
        content.contains("(*tree_sitter.Language, error)"),
        "capsule wrapper must declare the host return type. Content:\n{content}"
    );
    assert!(
        content.contains("tree_sitter.NewLanguage(unsafe.Pointer("),
        "capsule wrapper must construct the host type from the raw pointer. Content:\n{content}"
    );
}

/// Regression: the host package import must be emitted whenever a capsule wrapper
/// references the host type — otherwise the generated Go file fails to compile with
/// an "undefined: tree_sitter" error.
#[test]
fn go_capsule_wrapper_emits_host_package_import() {
    let files = GoBackend
        .generate_bindings(&make_api(), &make_go_config_with_capsule())
        .unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go not generated");
    let content = &binding.content;

    assert!(
        content.contains("\"github.com/tree-sitter/go-tree-sitter\""),
        "capsule host package import must be present in the import block. Content:\n{content}"
    );
    assert!(
        content.contains("tree_sitter \"github.com/tree-sitter/go-tree-sitter\""),
        "capsule host package import must be aliased with the body qualifier. Content:\n{content}"
    );
}
