use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{FieldDef, FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use std::collections::HashMap;

pub(super) fn make_trait_bridge_config(super_trait: Option<&str>, register_fn: Option<&str>) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: super_trait.map(str::to_string),
        registry_getter: None,
        register_fn: register_fn.map(str::to_string),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }
}

pub(super) fn make_alias_bridge(trait_name: &str, alias: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        type_alias: Some(alias.to_string()),
        ..TraitBridgeConfig::default()
    }
}

pub(super) fn make_type_def(name: &str, rust_path: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: rust_path.to_string(),
        original_rust_path: rust_path.to_string(),
        fields: vec![],
        methods,
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
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
        version: Default::default(),
    }
}

pub(super) fn make_method(
    name: &str,
    params: Vec<ParamDef>,
    return_type: TypeRef,
    is_async: bool,
    has_default_impl: bool,
    trait_source: Option<&str>,
    error_type: Option<&str>,
) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        is_static: false,
        error_type: error_type.map(str::to_string),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: trait_source.map(str::to_string),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

pub(super) fn make_func(name: &str, params: Vec<ParamDef>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("mylib::{name}"),
        original_rust_path: String::new(),
        params,
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
        version: Default::default(),
    }
}

pub(super) fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: Default::default(),
        vec_inner_core_wrapper: Default::default(),
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

pub(super) fn make_param(name: &str, ty: TypeRef, is_ref: bool) -> ParamDef {
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
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

pub(super) fn make_spec<'a>(
    trait_def: &'a TypeDef,
    bridge_config: &'a TraitBridgeConfig,
    wrapper_prefix: &'a str,
    type_paths: HashMap<String, String>,
) -> TraitBridgeSpec<'a> {
    TraitBridgeSpec {
        trait_def,
        bridge_config,
        core_import: "mylib",
        wrapper_prefix,
        type_paths,
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "MyError".to_string(),
        error_constructor: "MyError::from({msg})".to_string(),
    }
}

pub(super) struct MockBridgeGenerator;

impl TraitBridgeGenerator for MockBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "Py<PyAny>"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec!["pyo3::prelude::*".to_string(), "pyo3::types::PyString".to_string()]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        format!("// sync body for {}", method.name)
    }

    fn gen_async_method_body(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        format!("// async body for {}", method.name)
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        format!(
            "impl {} {{\n    pub fn new(obj: Py<PyAny>) -> Self {{ Self {{ inner: obj, cached_name: String::new() }} }}\n}}",
            spec.wrapper_name()
        )
    }

    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let fn_name = spec.bridge_config.register_fn.as_deref().unwrap_or("register");
        format!("pub fn {fn_name}(obj: Py<PyAny>) {{ /* register */ }}")
    }
}

pub(super) fn make_bridge(
    type_alias: Option<&str>,
    param_name: Option<&str>,
    bind_via: BridgeBinding,
    options_type: Option<&str>,
    options_field: Option<&str>,
    context_type: Option<&str>,
    result_type: Option<&str>,
) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: type_alias.map(str::to_string),
        param_name: param_name.map(str::to_string),
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via,
        options_type: options_type.map(str::to_string),
        options_field: options_field.map(str::to_string),
        context_type: context_type.map(str::to_string),
        result_type: result_type.map(str::to_string),
    }
}
