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
        serde_content: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let dummy_type = TypeDef {
        name: "Message".to_string(),
        rust_path: "my_crate::Message".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_serde: true,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
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

    let enum_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("boundary_reason.ex"))
        .expect("enum module file is generated");

    let module_content = &enum_file.content;

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

    assert!(
        module_content.contains("end_val"),
        "Module should contain escaped variant name `end_val`, got:\n{}",
        module_content
    );

    // ~keep The atom itself must stay `:end`, not `:end_val`: `elixir_variant_atom` deliberately
    // does not escape reserved words, because the value has to match what Rustler's
    // `NifUnitEnum` actually decodes to at runtime (`pascal_to_snake(variant.name)`). Escaping it
    // here would leave `wire_value/1`'s `:end` clause unreachable -- the exact
    // `FunctionClauseError` fixed in commit 7fcf57e8f ("make wire_value/1's clauses reachable").
    // Only the accessor/attribute *identifier* gets the `_val` suffix.
    assert!(
        module_content.contains("def wire_value(:end)"),
        "wire_value/1 should dispatch on the true Rustler atom `:end`, got:\n{}",
        module_content
    );

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
        serde_content: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let dummy_type = TypeDef {
        name: "Message".to_string(),
        rust_path: "my_crate::Message".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_serde: true,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
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

    let enum_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("keywords.ex"))
        .expect("enum module file is generated");

    let module_content = &enum_file.content;

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

    // ~keep Same invariant as `enum_variant_with_reserved_word_end_escapes_in_module`: the atom
    // must stay unescaped (`:do`, `:fn`, `:when`) because that is what Rustler's `NifUnitEnum`
    // actually produces; only the accessor/attribute identifier gets the `_val` suffix. Asserting
    // `:do_val` etc. here would pin the pre-fix behavior that made `wire_value/1`'s clauses
    // unreachable (commit 7fcf57e8f).
    assert!(
        module_content.contains("def wire_value(:do)"),
        "wire_value/1 should dispatch on the true Rustler atom `:do`, got:\n{}",
        module_content
    );
    assert!(
        module_content.contains("def wire_value(:fn)"),
        "wire_value/1 should dispatch on the true Rustler atom `:fn`, got:\n{}",
        module_content
    );
    assert!(
        module_content.contains("def wire_value(:when)"),
        "wire_value/1 should dispatch on the true Rustler atom `:when`, got:\n{}",
        module_content
    );
}
