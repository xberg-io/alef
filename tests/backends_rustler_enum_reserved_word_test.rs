//! Verifies that Elixir enum variants named with reserved words (end, fn, do, etc.)
//! are properly escaped in generated Elixir modules.
//!
//! The bug: enum variant `End` gets converted to `end()` which is an invalid function
//! definition in Elixir (end is a reserved word). The fix: append `_val` to create `end_val()`.

use alef::backends::rustler::RustlerBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::{ApiSurface, EnumDef, EnumVariant, TypeDef};

fn make_config(app_name: &str) -> alef::core::config::ResolvedCrateConfig {
    let crate_name = app_name.replace('_', "-");
    let toml = format!(
        r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "{app_name}"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn enum_variant_with_reserved_word_end_escapes_in_module() {
    // Build an enum with an "End" variant (converts to "end" in snake_case)
    let boundary_reason = EnumDef {
        name: "BoundaryReason".to_string(),
        rust_path: "my_crate::BoundaryReason".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Start".to_string(),
                fields: Vec::new(),
                doc: String::new(),
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
                name: "End".to_string(),
                fields: Vec::new(),
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
            EnumVariant {
                name: "Middle".to_string(),
                fields: Vec::new(),
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
        doc: "Enum with reserved word variant".to_string(),
        cfg: None,
        is_copy: true,
        has_serde: true,
        has_default: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    // Create a dummy struct that references the enum to force its generation
    let dummy_type = TypeDef {
        name: "Message".to_string(),
        rust_path: "my_crate::Message".to_string(),
        original_rust_path: String::new(),
        fields: vec![], // Empty struct to force enum generation
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_serde: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        has_lifetime_params: false,
        is_variant_wrapper: false,
        has_private_fields: false,
        version: Default::default(),
    };

    let config = make_config("my_app");
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![dummy_type],
        functions: vec![],
        errors: vec![],
        enums: vec![boundary_reason],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };

    let backend = RustlerBackend;
    let files = backend
        .generate_public_api(&api, &config)
        .expect("code generation succeeds");

    // Find the enum module file
    let enum_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("boundary_reason.ex"))
        .expect("enum module file is generated");

    let module_content = &enum_file.content;

    // Verify that the generated module does NOT contain `@spec end()` or `def end()`
    // These would be syntax errors in Elixir.
    assert!(
        !module_content.contains("@spec end()"),
        "Module should not contain `@spec end()` (reserved word), got:\n{}",
        module_content
    );
    assert!(
        !module_content.contains("def end()"),
        "Module should not contain `def end()` (reserved word), got:\n{}",
        module_content
    );

    // Verify that the safe version exists instead: `end_val` (from `elixir_safe_param_name`)
    assert!(
        module_content.contains("end_val"),
        "Module should contain escaped variant name `end_val`, got:\n{}",
        module_content
    );

    // Verify the type definition contains the safe atom reference
    assert!(
        module_content.contains(":end_val"),
        "Type definition should contain `:end_val` atom, got:\n{}",
        module_content
    );

    // Verify that normal variants are not affected
    assert!(
        module_content.contains(":start"),
        "start variant should be unaffected, got:\n{}",
        module_content
    );
    assert!(
        module_content.contains(":middle"),
        "middle variant should be unaffected, got:\n{}",
        module_content
    );
}

#[test]
fn enum_variant_with_multiple_reserved_words() {
    // Test multiple reserved words: Do, Fn, When
    let keywords_enum = EnumDef {
        name: "Keywords".to_string(),
        rust_path: "my_crate::Keywords".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Do".to_string(),
                fields: Vec::new(),
                doc: String::new(),
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
                name: "Fn".to_string(),
                fields: Vec::new(),
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
            EnumVariant {
                name: "When".to_string(),
                fields: Vec::new(),
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
        doc: "Enum with multiple reserved word variants".to_string(),
        cfg: None,
        is_copy: true,
        has_serde: true,
        has_default: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    // Create a dummy struct that references the enum to force its generation
    let dummy_type = TypeDef {
        name: "Message".to_string(),
        rust_path: "my_crate::Message".to_string(),
        original_rust_path: String::new(),
        fields: vec![], // Empty struct to force enum generation
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_serde: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        has_lifetime_params: false,
        is_variant_wrapper: false,
        has_private_fields: false,
        version: Default::default(),
    };

    let config = make_config("my_app");
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![dummy_type],
        functions: vec![],
        errors: vec![],
        enums: vec![keywords_enum],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };

    let backend = RustlerBackend;
    let files = backend
        .generate_public_api(&api, &config)
        .expect("code generation succeeds");

    // Find the enum module file
    let enum_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("keywords.ex"))
        .expect("enum module file is generated");

    let module_content = &enum_file.content;

    // Verify escaped variants exist
    assert!(
        module_content.contains("do_val"),
        "Module should contain escaped variant name `do_val`, got:\n{}",
        module_content
    );
    assert!(
        module_content.contains("fn_val"),
        "Module should contain escaped variant name `fn_val`, got:\n{}",
        module_content
    );
    assert!(
        module_content.contains("when_val"),
        "Module should contain escaped variant name `when_val`, got:\n{}",
        module_content
    );

    // Verify atoms are escaped
    assert!(
        module_content.contains(":do_val"),
        "Type should contain `:do_val` atom, got:\n{}",
        module_content
    );
    assert!(
        module_content.contains(":fn_val"),
        "Type should contain `:fn_val` atom, got:\n{}",
        module_content
    );
    assert!(
        module_content.contains(":when_val"),
        "Type should contain `:when_val` atom, got:\n{}",
        module_content
    );
}
