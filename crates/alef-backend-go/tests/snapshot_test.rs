//! Snapshot-style tests for Godoc emission on methods and free functions.
//!
//! These tests drive the public `GoBackend::generate_bindings` path with
//! rustdoc-annotated IR and assert that emitted `.go` files carry idiomatic
//! Godoc comments — symbol-prefixed first line plus translated `# Arguments`,
//! `# Returns`, `# Errors`, and `# Example` sections.

use alef_backend_go::GoBackend;
use alef_core::backend::Backend;
use alef_core::config::ResolvedCrateConfig;
use alef_core::config::new_config::NewAlefConfig;
use alef_core::ir::*;

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.go]
module = "github.com/test/test-lib"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_method(name: &str, doc: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: doc.to_string(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        trait_source: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
}

fn make_opaque_type(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{}", name),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: format!("Opaque handle for {}.", name),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
}

fn surface_for_type(typ: TypeDef) -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![typ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    }
}

fn binding_content(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let backend = GoBackend;
    let files = backend.generate_bindings(api, config).unwrap();
    files
        .iter()
        .find(|f| f.path.ends_with("binding.go"))
        .expect("binding.go should be generated")
        .content
        .clone()
}

/// Opaque receiver method with `# Returns` rustdoc emits `// RootNode returns ...`
/// Godoc on the line directly preceding the `func (...) RootNode()` declaration.
#[test]
fn godoc_on_method_with_returns_section_prefixes_symbol_name() {
    let doc =
        "Returns the root node of the parse tree.\n\n# Returns\n\nThe root node, which spans the entire source.\n";
    let typ = make_opaque_type(
        "Tree",
        vec![make_method("root_node", doc, TypeRef::Primitive(PrimitiveType::U64))],
    );
    let api = surface_for_type(typ);

    let content = binding_content(&api, &make_config());

    // Header comment must start with the Go method name.
    assert!(
        content.contains("// RootNode returns the root node of the parse tree."),
        "expected symbol-prefixed Godoc header on RootNode, got:\n{}",
        content,
    );
    // # Returns section translated to "// Returns ...".
    assert!(
        content.contains("// Returns the root node"),
        "expected `// Returns ...` translation of `# Returns` section, got:\n{}",
        content,
    );
    // Godoc paragraph separator before sections.
    let header_idx = content
        .find("// RootNode returns the root node of the parse tree.")
        .expect("header present");
    let returns_idx = content.find("// Returns the root node").expect("returns line present");
    let between = &content[header_idx..returns_idx];
    assert!(
        between.contains("//\n"),
        "expected blank `//` separator between summary and Returns section:\n{}",
        between,
    );

    // Header must immediately precede the func declaration.
    let func_idx = content.find("func (h *Tree) RootNode(").expect("func decl present");
    let header_full = content[..func_idx].rfind("// RootNode").expect("header before func");
    let tail = &content[header_full..func_idx];
    assert!(
        !tail.contains("\n\n"),
        "no blank line should separate Godoc from func declaration:\n{}",
        tail,
    );
}

/// Free function with `# Arguments` + `# Errors` emits per-arg bullets and a
/// `// Errors are returned when ...` line, with name-prefixed header.
#[test]
fn godoc_on_free_function_emits_arguments_bullets_and_errors() {
    let doc = "Parses the given source code into a syntax tree.\n\n\
        # Arguments\n\n\
        * `source` - source code bytes to parse\n\
        * `language` - the grammar to use\n\n\
        # Errors\n\n\
        the source is empty or the language is unsupported.\n";

    let func = FunctionDef {
        name: "parse_source".to_string(),
        rust_path: "test_lib::parse_source".to_string(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
                name: "source".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            },
            ParamDef {
                name: "language".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            },
        ],
        return_type: TypeRef::String,
        is_async: false,
        error_type: Some("Error".to_string()),
        doc: doc.to_string(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![func],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };

    let content = binding_content(&api, &make_config());

    assert!(
        content.contains("// ParseSource parses the given source code into a syntax tree."),
        "expected symbol-prefixed Godoc header on free function ParseSource, got:\n{}",
        content,
    );
    assert!(
        content.contains("// Arguments:"),
        "expected `// Arguments:` section header, got:\n{}",
        content,
    );
    assert!(
        content.contains("//   - source: source code bytes to parse"),
        "expected `source` bullet, got:\n{}",
        content,
    );
    assert!(
        content.contains("//   - language: the grammar to use"),
        "expected `language` bullet, got:\n{}",
        content,
    );
    assert!(
        content.contains("// Errors are returned when the source is empty"),
        "expected `// Errors are returned when ...` translation, got:\n{}",
        content,
    );
}

/// When the rustdoc summary already starts with the Go-cased method name, the
/// helper must NOT double-prefix (e.g. avoid `// RootNode RootNode returns ...`).
#[test]
fn godoc_does_not_double_prefix_when_summary_already_starts_with_name() {
    let doc = "RootNode returns the root node of the parse tree.";
    let typ = make_opaque_type(
        "Tree",
        vec![make_method("root_node", doc, TypeRef::Primitive(PrimitiveType::U64))],
    );
    let api = surface_for_type(typ);

    let content = binding_content(&api, &make_config());

    assert!(
        content.contains("// RootNode returns the root node of the parse tree."),
        "expected single-prefix header, got:\n{}",
        content,
    );
    assert!(
        !content.contains("// RootNode RootNode"),
        "must not double-prefix the symbol name:\n{}",
        content,
    );
}
