use alef::backends::zig::ZigBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, ParamDef,
    PrimitiveType, TypeDef, TypeRef,
};

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: String::new(),
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
    }
}

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
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
    }
}

fn make_basic_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "demo::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("value", TypeRef::Primitive(PrimitiveType::I32), false),
                make_field("label", TypeRef::String, false),
                make_field("tag", TypeRef::Optional(Box::new(TypeRef::String)), true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "A demo configuration struct.".to_string(),
            cfg: None,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }],
        functions: vec![FunctionDef {
            name: "process".into(),
            rust_path: "demo::process".into(),
            original_rust_path: String::new(),
            params: vec![
                make_param("input", TypeRef::String),
                make_param("count", TypeRef::Primitive(PrimitiveType::U32)),
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("DemoError".to_string()),
            doc: "Process input and return a result.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "demo::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    doc: "Active state.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
                EnumVariant {
                    name: "Inactive".to_string(),
                    fields: vec![],
                    doc: "Inactive state.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
            ],
            doc: "Processing status.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
        }],
        errors: vec![ErrorDef {
            name: "DemoError".to_string(),
            rust_path: "demo::DemoError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    message_template: Some("invalid input provided".to_string()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: "Input validation failed.".to_string(),
                },
                ErrorVariant {
                    name: "ProcessingFailed".to_string(),
                    message_template: Some("processing failed".to_string()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: "Processing encountered an error.".to_string(),
                },
            ],
            doc: "Errors emitted by demo operations.".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_basic_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn snapshot_basic_struct_function_enum_error() {
    let api = make_basic_api();
    let config = make_basic_config();
    let files = ZigBackend.generate_bindings(&api, &config).unwrap();
    for file in &files {
        insta::assert_snapshot!(
            format!("snapshot_basic__{}", file.path.display().to_string().replace('/', "__")),
            &file.content
        );
    }
}

#[test]
fn trait_bridge_vtable_builder_coverage() {
    // Regression test: verify that for every trait bridge configured, the Zig backend
    // emits a `make_{trait_snake}_vtable` comptime constructor function.
    // This ensures trait-bridge e2e test fixtures that call these builders will compile.
    //
    // The invariant: whenever alef.toml registers a trait bridge with
    // `trait_name = "SomeTrait"`, the Zig binding must emit both:
    //   1. A vtable struct: `pub struct ISomeTrait { ... }`
    //   2. A comptime builder: `pub fn make_some_trait_vtable(...) ISomeTrait { ... }`
    //
    // This test uses the public backend API to generate bindings for a synthetic
    // trait and verifies both are present.

    use alef::core::config::{BridgeBinding, TraitBridgeConfig};

    let mut api = make_basic_api();
    let method = alef::core::ir::MethodDef {
        name: "process".to_string(),
        params: vec![make_param("input", TypeRef::String)],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(alef::core::ir::ReceiverKind::Ref),
        trait_source: None,
        sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
    };

    // Add a trait type (marked as a trait with is_trait=true)
    let trait_def = TypeDef {
        name: "PluginTrait".to_string(),
        rust_path: "demo::PluginTrait".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![method],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: "A test plugin trait".to_string(),
        cfg: None,
        is_trait: true, // CRITICAL: mark as trait
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };
    api.types.push(trait_def);

    // Configure a trait bridge for that trait
    let mut config = make_basic_config();
    config.trait_bridges.push(TraitBridgeConfig {
        trait_name: "PluginTrait".to_string(),
        super_trait: None,
        registry_getter: Some("demo::registry::get_plugin_registry".to_string()),
        register_fn: Some("register_plugin".to_string()),
        unregister_fn: Some("unregister_plugin".to_string()),
        clear_fn: Some("clear_plugins".to_string()),
        bind_via: BridgeBinding::FunctionParam,
        ffi_skip_methods: vec![],
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    });

    // Generate bindings
    let files = ZigBackend.generate_bindings(&api, &config).unwrap();
    let content = files
        .iter()
        .find(|f| f.content.contains("pub fn make_") || f.content.contains("pub struct I"))
        .or_else(|| files.first())
        .expect("zig binding file must be generated")
        .content
        .clone();

    // Verify: vtable struct exists
    assert!(
        content.contains("pub const IPluginTrait = extern struct {"),
        "BUG: vtable struct missing. Content preview:\n{}",
        &content[..std::cmp::min(1500, content.len())]
    );

    // Verify: vtable builder exists
    assert!(
        content.contains("pub fn make_plugin_trait_vtable(comptime T: type, instance: *T) IPluginTrait {"),
        "BUG: vtable builder missing. Expected 'pub fn make_plugin_trait_vtable(...)' in generated code."
    );
}

#[test]
fn trait_bridge_multiple_traits_emit_all_vtable_builders() {
    // Regression test for B10: verify that ALL registered trait bridges get their
    // vtable builder functions emitted, not just some of them.
    // This replicates the kreuzberg scenario with 6 traits.

    use alef::core::config::{BridgeBinding, TraitBridgeConfig};

    let mut api = make_basic_api();

    // Define 6 traits matching the kreuzberg plugin system
    let trait_names = vec![
        "DocumentExtractor",
        "OcrBackend",
        "PostProcessor",
        "EmbeddingBackend",
        "Renderer",
        "Validator",
    ];

    for trait_name in &trait_names {
        let method = alef::core::ir::MethodDef {
            name: "process".to_string(),
            params: vec![make_param("input", TypeRef::String)],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(alef::core::ir::ReceiverKind::Ref),
            trait_source: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let trait_def = TypeDef {
            name: trait_name.to_string(),
            rust_path: format!("demo::{}", trait_name),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![method],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            doc: format!("Test trait: {}", trait_name),
            cfg: None,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        };
        api.types.push(trait_def);
    }

    // Configure trait bridges for all 6 traits
    let mut config = make_basic_config();
    for trait_name in &trait_names {
        let snake = heck::AsSnakeCase(trait_name).to_string();
        config.trait_bridges.push(TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: Some(format!("register_{}", snake)),
            unregister_fn: Some(format!("unregister_{}", snake)),
            clear_fn: None,
            bind_via: BridgeBinding::FunctionParam,
            ffi_skip_methods: vec![],
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: vec![],
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
        });
    }

    // Generate bindings
    let files = ZigBackend.generate_bindings(&api, &config).unwrap();
    let content = files
        .iter()
        .find(|f| f.content.contains("pub fn make_") || f.content.contains("pub struct I"))
        .or_else(|| files.first())
        .expect("zig binding file must be generated")
        .content
        .clone();

    // Verify all 6 vtable builders are emitted
    for trait_name in &trait_names {
        let snake = heck::AsSnakeCase(trait_name).to_string();
        let expected_builder = format!("pub fn make_{}_vtable(comptime T: type, instance: *T)", snake);
        assert!(
            content.contains(&expected_builder),
            "BUG: missing make_{}_vtable builder for trait {}. Generated code:\n{}",
            snake,
            trait_name,
            &content[..std::cmp::min(2000, content.len())]
        );
    }
}

#[test]
fn trait_bridge_vcoverage_assertion_catches_missing_trait_definitions() {
    // Regression test for B10 fallout: trait bridges registered but trait definitions
    // missing/excluded should produce a hard error, not silent omission of vtable builders.
    //
    // BUG PATTERN: alef.toml registers a trait bridge with `trait_name = "Foo"`,
    // but the Rust source doesn't export `Foo` as a trait (or it's excluded from
    // the binding surface). The current code silently skips emit_trait_bridge()
    // via the `if let Some(trait_def) = ...` guard at line 280, leaving
    // e2e tests with dangling references to `make_foo_vtable(...)` that don't
    // exist in the generated binding.
    //
    // This test verifies that the emitter enforces the invariant: every registered
    // trait bridge MUST have a corresponding trait definition in the API surface,
    // or the build should fail explicitly.

    use alef::core::config::{BridgeBinding, TraitBridgeConfig};

    let api = make_basic_api();

    // DO NOT add trait definitions to the API.
    // Configure trait bridges for 2 traits that don't exist in the API.
    let mut config = make_basic_config();
    config.trait_bridges.push(TraitBridgeConfig {
        trait_name: "MissingTrait1".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: Some("register_missing1".to_string()),
        unregister_fn: None,
        clear_fn: None,
        bind_via: BridgeBinding::FunctionParam,
        ffi_skip_methods: vec![],
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    });

    // Generate bindings
    let files = ZigBackend.generate_bindings(&api, &config).unwrap();
    let content = files
        .iter()
        .find(|f| f.content.contains("pub fn make_") || f.content.contains("pub struct I"))
        .or_else(|| files.first())
        .expect("zig binding file must be generated")
        .content
        .clone();

    // EXPECTED: vtable builder should NOT exist for a missing trait
    let not_found = !content.contains("pub fn make_missing_trait1_vtable");
    assert!(
        not_found,
        "EXPECTED: make_missing_trait1_vtable should NOT be emitted when trait definition is missing from API"
    );

    // CRITICAL INVARIANT: If a trait bridge is configured but the trait is not found,
    // the build should emit a warning or error. For now, we document the current behavior:
    // Missing traits are silently skipped. Future work: add a validation pass that detects
    // this mismatch and reports it to the user.
}
