//! The adapter must bind the bridged call to `result` only when the call produces a value.
//!
//! Every test here has a control asserting the value-returning shape still binds, so a
//! regression that stops binding anywhere cannot pass.

use super::*;
use crate::core::config::BridgeBinding;
use crate::core::ir::{MethodDef, PrimitiveType};

fn trait_def_with(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("samplecrate::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn method(name: &str, return_type: TypeRef, error_type: Option<&str>) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: false,
        is_static: false,
        error_type: error_type.map(str::to_string),
        doc: String::new(),
        receiver: None,
        cfg: None,
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

fn bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        exclude_functions: Vec::new(),
        trait_name: trait_name.to_string(),
        param_name: None,
        type_alias: None,
        exclude_languages: vec![],
        super_trait: None,
        registry_getter: None,
        register_fn: Some(format!("register{trait_name}")),
        unregister_fn: None,
        clear_fn: None,
        register_extra_args: None,
        bind_via: BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    }
}

fn adapter_source(trait_name: &str, methods: Vec<MethodDef>) -> String {
    let trait_def = trait_def_with(trait_name, methods);
    let cfg = bridge_cfg(trait_name);
    let bridges = vec![(trait_name.to_string(), &cfg, &trait_def)];
    let files = gen_trait_bridge_files(&bridges, &HashSet::new(), &HashSet::new());
    files
        .into_iter()
        .find(|(name, _)| name == &format!("Swift{trait_name}Bridge.swift"))
        .unwrap_or_else(|| panic!("Swift{trait_name}Bridge.swift must be generated"))
        .1
}

#[test]
fn should_not_bind_result_when_throwing_bridge_method_returns_unit() {
    let swift = adapter_source("SampleSink", vec![method("accept", TypeRef::Unit, Some("Error"))]);

    assert!(
        swift.contains("            try self.bridge.accept()\n"),
        "a Void-returning throwing bridge method must be called as a bare statement:\n{swift}"
    );
    assert!(
        !swift.contains("let result = try self.bridge.accept("),
        "binding a Void call to `result` warns twice (unexpected Void inference + unused \
         variable) and fails a warnings-as-errors build:\n{swift}"
    );
}

#[test]
fn should_still_bind_result_when_throwing_bridge_method_returns_a_value() {
    let swift = adapter_source("SampleReader", vec![method("read", TypeRef::String, Some("Error"))]);

    assert!(
        swift.contains("let result = try self.bridge.read()"),
        "a value-returning throwing bridge method must still bind `result` -- the success body \
         marshals it:\n{swift}"
    );
}

#[test]
fn should_not_bind_result_when_non_throwing_bridge_method_returns_unit() {
    let swift = adapter_source("SampleObserver", vec![method("notify", TypeRef::Unit, None)]);

    assert!(
        swift.contains("        self.bridge.notify()\n"),
        "a Void-returning non-throwing bridge method must be called as a bare statement:\n{swift}"
    );
    assert!(
        !swift.contains("let result = self.bridge.notify("),
        "binding a Void call to `result` warns twice and fails a warnings-as-errors build:\n{swift}"
    );
}

#[test]
fn should_still_bind_result_when_non_throwing_bridge_method_returns_a_value() {
    let swift = adapter_source(
        "SampleProbe",
        vec![method("is_ready", TypeRef::Primitive(PrimitiveType::Bool), None)],
    );

    assert!(
        swift.contains("let result = self.bridge.isReady()"),
        "a value-returning non-throwing bridge method must still bind and return `result`:\n{swift}"
    );
    assert!(
        swift.contains("return result"),
        "the bound value must still be returned:\n{swift}"
    );
}
