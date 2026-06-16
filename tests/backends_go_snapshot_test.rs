//! Snapshot-style tests for Godoc emission on methods and free functions.
//!
//! These tests drive the public `GoBackend::generate_bindings` path with
//! rustdoc-annotated IR and assert that emitted `.go` files carry idiomatic
//! Godoc comments — symbol-prefixed first line plus translated `# Arguments`,
//! `# Returns`, `# Errors`, and `# Example` sections.

use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::*;

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
        version: Default::default(),
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
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
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
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: alef::core::ir::CoreWrapper::None,
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
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: alef::core::ir::CoreWrapper::None,
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
        version: Default::default(),
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
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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

/// Multi-line summary (blank line + continuation, no `# Section` heading) must have
/// every continuation line prefixed with `//` — Go rejects bare lines between the
/// comment block and the `func` declaration as syntax errors.
#[test]
fn godoc_multiline_summary_continuation_lines_are_prefixed() {
    let doc = "Returns the canonical HTTP status code associated with this error.\n\n\
        Maps error variants to their originating HTTP status code as set by\n\
        SampleLlmError::from_status. Used by e2e assertions that check\n\
        error.status_code against the expected HTTP status.";
    let typ = make_opaque_type(
        "Error",
        vec![make_method("status_code", doc, TypeRef::Primitive(PrimitiveType::U16))],
    );
    let api = surface_for_type(typ);
    let content = binding_content(&api, &make_config());

    // Every line in the doc block must start with `//` — no bare continuation lines.
    let func_marker = "func (h *Error) StatusCode(";
    let func_pos = content.find(func_marker).unwrap_or_else(|| {
        panic!("func declaration not found in:\n{content}");
    });
    // Walk backward to find the start of the doc comment block.
    let comment_start = content[..func_pos]
        .rfind("// StatusCode")
        .unwrap_or_else(|| panic!("doc comment header not found in:\n{content}"));
    let comment_block = &content[comment_start..func_pos];
    for line in comment_block.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            assert!(
                trimmed.starts_with("//"),
                "continuation line missing `//` prefix: {:?}\nfull comment block:\n{}",
                line,
                comment_block,
            );
        }
    }
    // The blank separator between first line and continuation must be `//` not a true blank line.
    assert!(
        !comment_block.contains("\n\n"),
        "blank line (no `//`) found inside doc block — Go parser would reject it:\n{}",
        comment_block,
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

/// Option<String> returns are null-checked and boxed, not dereferenced directly.
/// The generated code must return `*string`, not `string`.
#[test]
fn option_string_return_null_checks_and_boxes_value() {
    let func = FunctionDef {
        name: "get_optional_name".to_string(),
        rust_path: "test_lib::get_optional_name".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::Optional(Box::new(TypeRef::String)),
        is_async: false,
        error_type: None,
        doc: "Returns an optional name.".to_string(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
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
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let content = binding_content(&api, &make_config());

    // Function signature must return *string, not string
    assert!(
        content.contains("func GetOptionalName() *string"),
        "expected return type `*string`, got:\n{}",
        content,
    );

    // Body must contain the conversion and boxing, NOT a bare return C.GoString(ptr)
    let has_conversion_boxed = content.contains("s := C.GoString(ptr)") && content.contains("return &s");
    let has_bad_pattern = content.contains("return C.GoString(ptr)") && !content.contains("&s");

    assert!(
        has_conversion_boxed && !has_bad_pattern,
        "expected boxed string return `s := C.GoString(ptr); return &s`, not bare C.GoString(ptr):\n{}",
        content,
    );
}
