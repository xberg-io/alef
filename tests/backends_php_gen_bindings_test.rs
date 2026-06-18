use alef::backends::php::PhpBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::*;

/// Helper to create a config with a specific extension name for namespace testing.
#[allow(dead_code)]
fn make_config_with_extension(extension_name: &str) -> ResolvedCrateConfig {
    let toml = format!(
        r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "{extension_name}"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

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

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "test_lib"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn toml_basic_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn make_config_with_php_output(output_path: &std::path::Path) -> ResolvedCrateConfig {
    let output = toml_basic_string(&output_path.to_string_lossy());
    let toml = format!(
        r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.output]
php = {output}

[crates.php]
extension_name = "test_lib"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn make_config_with_php_excludes() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "test_lib"
exclude_functions = ["hidden_function"]
exclude_types = ["HiddenConfig"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

// ---------------------------------------------------------------------------
// PHP trait bridge helpers
// ---------------------------------------------------------------------------

fn make_trait_def_php(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("my_lib::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    }
}

fn make_method_php(name: &str, return_type: TypeRef, has_error: bool, has_default: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: false,
        is_static: false,
        error_type: if has_error {
            Some("Box<dyn std::error::Error + Send + Sync>".to_string())
        } else {
            None
        },
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: has_default,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_async_method_php(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: true,
        is_static: false,
        error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_node_context_php() -> TypeDef {
    TypeDef {
        name: "NodeContext".to_string(),
        rust_path: "my_lib::NodeContext".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("node_id", TypeRef::String, false)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    }
}

fn make_visit_result_php() -> EnumDef {
    EnumDef {
        name: "VisitResult".to_string(),
        rust_path: "my_lib::VisitResult".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Continue".to_string(),
                fields: vec![],
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
                name: "Stop".to_string(),
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
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: Some("snake_case".to_string()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

fn make_api_php() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![make_node_context_php()],
        functions: vec![],
        enums: vec![make_visit_result_php()],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_plugin_bridge_cfg_php(trait_name: &str) -> alef::core::config::TraitBridgeConfig {
    alef::core::config::TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }
}

fn make_visitor_bridge_cfg_php(trait_name: &str, type_alias: &str) -> alef::core::config::TraitBridgeConfig {
    alef::core::config::TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: Some(type_alias.to_string()),
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: Some("NodeContext".to_string()),
        result_type: Some("VisitResult".to_string()),
    }
}

// ---------------------------------------------------------------------------
fn make_field_with_doc(name: &str, ty: TypeRef, optional: bool, doc: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: doc.to_string(),
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

#[path = "backends_php_gen_bindings/docs_defaults.rs"]
mod docs_defaults;
#[path = "backends_php_gen_bindings/generation.rs"]
mod generation;
#[path = "backends_php_gen_bindings/methods_errors.rs"]
mod methods_errors;
#[path = "backends_php_gen_bindings/params_formatting.rs"]
mod params_formatting;
#[path = "backends_php_gen_bindings/public_api_stubs.rs"]
mod public_api_stubs;
#[path = "backends_php_gen_bindings/scaffold_pie_url.rs"]
mod scaffold_pie_url;
#[path = "backends_php_gen_bindings/stubs_dtos.rs"]
mod stubs_dtos;
#[path = "backends_php_gen_bindings/tagged_enums.rs"]
mod tagged_enums;
#[path = "backends_php_gen_bindings/trait_bridge.rs"]
mod trait_bridge;
