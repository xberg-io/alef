use alef::backends::pyo3::Pyo3Backend;
use alef::backends::pyo3::trait_bridge::{Pyo3BridgeGenerator, gen_trait_bridge};
use alef::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, PythonConfig, ResolvedCrateConfig, StubsConfig, TraitBridgeConfig};
use alef::core::ir::*;
use std::collections::HashMap;

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
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_test_lib"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

// ---------------------------------------------------------------------------
// Trait bridge helpers
// ---------------------------------------------------------------------------

fn make_trait_def(name: &str, rust_path: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: rust_path.to_string(),
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

fn make_method_def(
    name: &str,
    params: Vec<ParamDef>,
    return_type: TypeRef,
    is_async: bool,
    has_error: bool,
    has_default_impl: bool,
) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        is_static: false,
        error_type: if has_error {
            Some("Box<dyn std::error::Error + Send + Sync>".to_string())
        } else {
            None
        },
        doc: format!("Documentation for {name}."),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl,
        trait_source: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_param_def(name: &str, ty: TypeRef, is_ref: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref,
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

fn make_bridge_generator(core_import: &str) -> Pyo3BridgeGenerator {
    Pyo3BridgeGenerator {
        core_import: core_import.to_string(),
        type_paths: HashMap::new(),
        error_type: "Error".to_string(),
    }
}

fn make_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    }
}

fn make_api_surface() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    }
}
#[path = "backends_pyo3_gen_bindings/adapters_and_serde.rs"]
mod adapters_and_serde;
#[path = "backends_pyo3_gen_bindings/alef44_api.rs"]
mod alef44_api;
#[path = "backends_pyo3_gen_bindings/capsule_types.rs"]
mod capsule_types;
#[path = "backends_pyo3_gen_bindings/enum_options_regressions.rs"]
mod enum_options_regressions;
#[path = "backends_pyo3_gen_bindings/generation.rs"]
mod generation;
#[path = "backends_pyo3_gen_bindings/module_api.rs"]
mod module_api;
#[path = "backends_pyo3_gen_bindings/option_fields.rs"]
mod option_fields;
#[path = "backends_pyo3_gen_bindings/options_exports.rs"]
mod options_exports;
#[path = "backends_pyo3_gen_bindings/static_return_wrappers.rs"]
mod static_return_wrappers;
#[path = "backends_pyo3_gen_bindings/trait_bridge_generation.rs"]
mod trait_bridge_generation;
#[path = "backends_pyo3_gen_bindings/trait_bridge_methods.rs"]
mod trait_bridge_methods;
#[path = "backends_pyo3_gen_bindings/trait_bridge_registry.rs"]
mod trait_bridge_registry;
