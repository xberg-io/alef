use super::*;
use crate::core::ir::{DeprecationInfo, VersionAnnotation};

/// Config with two bindings whose feature sets differ: `wasm` disables the
/// `tree-sitter` feature (its feature set is `["wasm-target"]`) while the
/// default (used by `python`) enables `full`. Mirrors the xberg layout where
/// `#[cfg(feature = "tree-sitter")]` items compile out of the wasm binding.
fn cfg_gating_config() -> ResolvedCrateConfig {
    config_from_toml(
        r#"
[workspace]
languages = ["python", "wasm"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
features = ["full"]

[crates.wasm]
features = ["wasm-target"]
"#,
    )
}

fn gated_type(name: &str, cfg: Option<&str>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("mylib::{name}"),
        cfg: cfg.map(str::to_string),
        doc: format!("Doc for {name}."),
        ..TypeDef::default()
    }
}

#[test]
fn cfg_gated_type_excluded_when_feature_off_included_when_on() {
    use crate::core::ir::{CoreWrapper, FieldDef};

    let extraction_config = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "mylib::ExtractionConfig".to_string(),
        doc: "Extraction configuration.".to_string(),
        fields: vec![
            FieldDef {
                name: "use_cache".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::Bool),
                doc: "Whether to cache.".to_string(),
                cfg: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                ..FieldDef::default()
            },
            FieldDef {
                name: "tree_sitter".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("TreeSitterConfig".to_string()))),
                optional: true,
                doc: "Tree-sitter config.".to_string(),
                cfg: Some("feature = \"tree-sitter\"".to_string()),
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    };

    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            gated_type("TreeSitterConfig", Some("feature = \"tree-sitter\"")),
            extraction_config,
        ],
        ..ApiSurface::default()
    };
    let config = cfg_gating_config();
    let files = generate_docs(&api, &config, &[Language::Python, Language::Wasm], "out").unwrap();

    let wasm = &files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-wasm"))
        .unwrap()
        .content;
    assert!(
        !wasm.contains("TreeSitterConfig"),
        "cfg-gated type must be omitted from wasm docs; got:\n{wasm}"
    );
    assert!(
        !wasm.contains("tree_sitter"),
        "cfg-gated field must be omitted from wasm docs; got:\n{wasm}"
    );
    assert!(
        wasm.contains("ExtractionConfig") && wasm.contains("useCache"),
        "ungated type/field must remain in wasm docs; got:\n{wasm}"
    );

    let python = &files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap()
        .content;
    assert!(
        python.contains("TreeSitterConfig"),
        "cfg-gated type must remain in python docs (full feature set); got:\n{python}"
    );
    assert!(
        python.contains("tree_sitter"),
        "cfg-gated field must remain in python docs; got:\n{python}"
    );
}

#[test]
fn cfg_gated_enum_variant_and_function_respect_feature_set() {
    use crate::core::ir::EnumVariant;

    let content_mode = EnumDef {
        name: "CodeContentMode".to_string(),
        rust_path: "mylib::CodeContentMode".to_string(),
        cfg: Some("feature = \"tree-sitter\"".to_string()),
        doc: "Code content mode.".to_string(),
        variants: vec![EnumVariant {
            name: "Full".to_string(),
            doc: "Full content.".to_string(),
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    };
    let output_format = EnumDef {
        name: "OutputFormat".to_string(),
        rust_path: "mylib::OutputFormat".to_string(),
        cfg: None,
        doc: "Output format.".to_string(),
        variants: vec![
            EnumVariant {
                name: "Markdown".to_string(),
                doc: "Markdown.".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "CodeAst".to_string(),
                doc: "Code AST output.".to_string(),
                cfg: Some("feature = \"tree-sitter\"".to_string()),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    };

    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![content_mode, output_format],
        functions: vec![FunctionDef {
            name: "extract_code_intelligence".to_string(),
            rust_path: "mylib::extract_code_intelligence".to_string(),
            cfg: Some("feature = \"tree-sitter\"".to_string()),
            return_type: TypeRef::String,
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };
    let config = cfg_gating_config();
    let files = generate_docs(&api, &config, &[Language::Python, Language::Wasm], "out").unwrap();

    let wasm = &files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-wasm"))
        .unwrap()
        .content;
    assert!(
        !wasm.contains("CodeContentMode"),
        "cfg-gated enum must be omitted from wasm docs; got:\n{wasm}"
    );
    assert!(
        !wasm.contains("extract_code_intelligence"),
        "cfg-gated function must be omitted from wasm docs; got:\n{wasm}"
    );
    assert!(
        !wasm.contains("CodeAst") && !wasm.contains("CODE_AST"),
        "cfg-gated enum variant must be omitted from wasm docs; got:\n{wasm}"
    );
    assert!(
        wasm.contains("OutputFormat"),
        "ungated enum must remain in wasm docs; got:\n{wasm}"
    );

    let python = &files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap()
        .content;
    assert!(
        python.contains("CodeContentMode") && python.contains("extract_code_intelligence"),
        "cfg-gated items must remain in python docs; got:\n{python}"
    );
    assert!(
        python.contains("CODE_AST"),
        "cfg-gated enum variant must remain in python docs; got:\n{python}"
    );
}

#[test]
fn test_generate_docs_with_function_renders_signature_and_params() {
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "parse_document".to_string(),
            rust_path: "mylib::parse_document".to_string(),
            original_rust_path: String::new(),
            params: vec![make_param("input", TypeRef::String, false)],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Parses a document into plain text.".to_string(),
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
    };
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let lang_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap();
    assert!(lang_file.content.contains("parse_document()"));
    assert!(lang_file.content.contains("Parses a document into plain text."));
    assert!(lang_file.content.contains("**Signature:**"));
    assert!(lang_file.content.contains("**Parameters:**"));
}

#[test]
fn test_generate_docs_with_enum_renders_python_screaming_case_variants() {
    use crate::core::ir::EnumVariant;
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "OutputFormat".to_string(),
            rust_path: "mylib::OutputFormat".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Markdown".to_string(),
                    fields: vec![],
                    doc: "Markdown output.".to_string(),
                    is_default: true,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Plain".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: "The output format.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let lang_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap();
    assert!(lang_file.content.contains("OutputFormat"));
    assert!(
        lang_file.content.contains("MARKDOWN"),
        "Python variant must be SCREAMING_SNAKE"
    );
    assert!(lang_file.content.contains("PLAIN"));
}

#[test]
fn test_generate_docs_with_type_renders_fields_and_doc() {
    use crate::core::ir::{CoreWrapper, FieldDef};
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ParseOptions".to_string(),
            rust_path: "mylib::ParseOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "max_length".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                optional: true,
                default: None,
                doc: "Maximum output length.".to_string(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "Options for the conversion.".to_string(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let lang_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap();
    assert!(lang_file.content.contains("ParseOptions"));
    assert!(lang_file.content.contains("max_length"));
    assert!(lang_file.content.contains("Maximum output length."));
}

#[test]
fn test_generate_docs_with_error_appears_in_lang_page_and_errors_md() {
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "ConversionError".to_string(),
            rust_path: "mylib::ConversionError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                crate::core::ir::ErrorVariant {
                    name: "InvalidInput".to_string(),
                    message_template: Some("Invalid input: {0}".to_string()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    is_tuple: false,
                    doc: String::new(),
                },
                crate::core::ir::ErrorVariant {
                    name: "IoError".to_string(),
                    message_template: None,
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: "An I/O error occurred.".to_string(),
                },
            ],
            doc: "Errors from the conversion API.".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let lang_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap();
    assert!(lang_file.content.contains("ConversionError"));
    assert!(lang_file.content.contains("InvalidInput"));
    assert!(lang_file.content.contains("IoError"));

    let errors_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("errors"))
        .unwrap();
    assert!(errors_file.content.contains("ConversionError"));
    assert!(errors_file.content.contains("Invalid input: {0}"));
}

#[test]
fn test_function_with_since_renders_version_badge() {
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![FunctionDef {
            name: "new_fn".to_string(),
            rust_path: "mylib::new_fn".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: VersionAnnotation {
                since: Some("0.5.0".to_string()),
                deprecated: None,
            },
        }],
        types: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let content = &files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap()
        .content;
    assert!(
        content.contains("**Since:** `v0.5`"),
        "expected since badge, got:\n{content}"
    );
}

#[test]
fn test_function_deprecated_renders_warning_admonition() {
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "2.0.0".to_string(),
        functions: vec![FunctionDef {
            name: "old_fn".to_string(),
            rust_path: "mylib::old_fn".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: VersionAnnotation {
                since: None,
                deprecated: Some(DeprecationInfo {
                    since: Some("1.5.0".to_string()),
                    note: Some("use new_fn instead".to_string()),
                }),
            },
        }],
        types: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let content = &files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap()
        .content;
    assert!(
        content.contains(":::caution"),
        "expected caution admonition, got:\n{content}"
    );
    assert!(
        content.contains("Deprecated"),
        "expected Deprecated text, got:\n{content}"
    );
    assert!(
        content.contains("1.5"),
        "expected deprecated since version, got:\n{content}"
    );
    assert!(
        content.contains("use new_fn instead"),
        "expected deprecation note, got:\n{content}"
    );
}

#[test]
fn test_enum_variant_with_since_renders_inline_in_table() {
    use crate::core::ir::{EnumVariant, VersionAnnotation};
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![],
        types: vec![],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "mylib::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    doc: "Currently active.".to_string(),
                    is_default: true,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: VersionAnnotation {
                        since: Some("0.5.0".to_string()),
                        deprecated: None,
                    },
                },
                EnumVariant {
                    name: "Legacy".to_string(),
                    fields: vec![],
                    doc: "Old name.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: VersionAnnotation {
                        since: None,
                        deprecated: Some(DeprecationInfo {
                            since: Some("0.5.0".to_string()),
                            note: Some("use Active".to_string()),
                        }),
                    },
                },
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let content = &files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap()
        .content;
    assert!(
        content.contains("Since:") && content.contains("v0.5"),
        "variant since badge must appear inline in table, got:\n{content}"
    );
    assert!(
        content.contains("Deprecated since") && content.contains("use Active"),
        "variant deprecated note must appear inline in table, got:\n{content}"
    );
}
