//! Regression coverage for host-native capsule (Language-passthrough) generation in the Java
//! facade. The raw FFI class already constructs the host runtime's `Language`; the facade must
//! declare that same host type as its return so the delegating `return raw.method(...)` body
//! compiles. Without the fix the facade declared the package-local opaque `Language`, producing
//! `incompatible types: io.github.treesitter.jtreesitter.Language cannot be converted to ...`.

use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};

fn capsule_function(name: &str) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("tree_sitter::{name}"),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::Named("Language".to_string()),
        error_type: None,
        is_async: false,
        doc: "Get a language grammar.".to_string(),
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

fn api_with_capsule(fn_name: &str) -> ApiSurface {
    ApiSurface {
        crate_name: "tree_sitter".to_string(),
        version: "0.25.0".to_string(),
        types: vec![],
        functions: vec![capsule_function(fn_name)],
        enums: vec![],
        errors: vec![],
        services: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    }
}

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// The generated facade class is the public wrapper (main class without the `Rs` suffix).
fn facade_content(files: &[alef::core::backend::GeneratedFile]) -> String {
    let file = files
        .iter()
        .find(|f| {
            let path = f.path.to_string_lossy();
            path.ends_with(".java") && f.content.contains("public static") && f.content.contains("getLanguage")
        });

    if let Some(f) = file {
        f.content.clone()
    } else {
        // Debug: list all generated files
        let mut debug_msg = String::from("facade class containing getLanguage not found\nGenerated files:\n");
        for f in files {
            debug_msg.push_str(&format!("  {}\n", f.path.display()));
        }
        panic!("{}", debug_msg);
    }
}

#[test]
fn facade_declares_configured_host_capsule_return_type() {
    let api = api_with_capsule("get_language");
    let config = resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "tree_sitter"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "tree_sitter"
error_style = "last_error"

[crates.java]
package = "dev.kreuzberg.treesitterlanguagepack"

[crates.java.capsule_types.Language]
host_type = "io.github.treesitter.jtreesitter.Language"
package = "io.github.tree-sitter:jtreesitter"
package_version = "0.26.0"
construct_expr = "new io.github.treesitter.jtreesitter.Language({ptr})"
"#,
    );

    let files = JavaBackend.generate_bindings(&api, &config).expect("Java generation failed");
    let content = facade_content(&files);

    assert!(
        content.contains("io.github.treesitter.jtreesitter.Language getLanguage"),
        "facade should declare the host capsule return type. Content:\n{content}"
    );
    // The buggy form declared the package-local opaque handle as the return type.
    assert!(
        !content.contains("static Language getLanguage"),
        "facade must not declare the opaque local Language as the return type. Content:\n{content}"
    );
}

#[test]
fn facade_falls_back_to_opaque_handle_without_capsule_config() {
    let api = api_with_capsule("get_language");
    let config = resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "tree_sitter"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "tree_sitter"
error_style = "last_error"

[crates.java]
package = "dev.kreuzberg.treesitterlanguagepack"
"#,
    );

    let files = JavaBackend.generate_bindings(&api, &config).expect("Java generation failed");
    let content = facade_content(&files);

    assert!(
        content.contains("static Language getLanguage"),
        "without capsule config the facade keeps the opaque local Language return. Content:\n{content}"
    );
}
