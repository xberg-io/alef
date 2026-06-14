//! NAPI-RS-specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to JavaScript objects via NAPI-RS.

mod bridge;
mod bridge_functions;
mod bridge_generator;
mod options_field_bridge;
mod typescript_bridge;
mod visitor_bridge;

use crate::core::config::TraitBridgeConfig;

pub use bridge::gen_trait_bridge;
pub use bridge_functions::gen_bridge_function;
pub use bridge_generator::NapiBridgeGenerator;
pub use options_field_bridge::gen_options_field_bridge_function;
pub use typescript_bridge::gen_typescript_trait_bridge_files;

/// Find the first parameter index and bridge config where the parameter's named type
/// matches a trait bridge's `type_alias`.
///
/// Returns `None` when no bridge applies.
pub use crate::codegen::generators::trait_bridge::find_bridge_param;

/// Find a bridge config that uses options_field binding and a parameter of the options_type.
/// This complements find_bridge_param which only handles FunctionParam bindings.
pub fn find_options_field_binding<'a>(
    func: &crate::core::ir::FunctionDef,
    bridges: &'a [TraitBridgeConfig],
) -> Option<(usize, &'a TraitBridgeConfig)> {
    for bridge in bridges {
        if bridge.bind_via != crate::core::config::BridgeBinding::OptionsField {
            continue;
        }
        if let Some(options_type) = &bridge.options_type {
            for (idx, param) in func.params.iter().enumerate() {
                let matches = match &param.ty {
                    crate::core::ir::TypeRef::Named(n) => n == options_type,
                    crate::core::ir::TypeRef::Optional(inner) => {
                        if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
                            n == options_type
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                if matches {
                    return Some((idx, bridge));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn visitor_bridge_uses_configured_context_and_result_metadata() {
        let (api, trait_type, bridge) = crate::codegen::visitor_context::test_support::neutral_visitor_fixture();
        let output = super::gen_trait_bridge(
            &trait_type,
            &bridge,
            "sample_core",
            "SampleError",
            "SampleError::Message { message: {msg} }",
            &api,
        )
        .expect("visitor bridge should generate");

        crate::codegen::visitor_context::test_support::assert_neutral_visitor_output(&output.code);
        assert!(output.code.contains("displayName"));
    }

    #[test]
    fn plugin_trait_bridge_emits_dispose_method_on_rust_struct() {
        // Regression: napi-rs `ThreadsafeFunction` handles held by trait-bridge
        // wrappers kept the Node event loop alive. The generated Rust bridge
        // struct must expose `pub async fn dispose()` so TypeScript callers can
        // release the TSFN and allow test workers to exit.
        use crate::core::config::{BridgeBinding, TraitBridgeConfig};
        use crate::core::ir::ApiSurface;

        let trait_def = crate::core::ir::TypeDef {
            name: "TextProcessor".to_string(),
            rust_path: "sample_core::TextProcessor".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
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
        };

        let bridge_cfg = TraitBridgeConfig {
            trait_name: "TextProcessor".to_string(),
            super_trait: Some("Plugin".to_string()),
            registry_getter: Some("sample_core::get_text_processor_registry".to_string()),
            register_fn: Some("register_text_processor".to_string()),
            unregister_fn: None,
            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: vec![],
            ffi_skip_methods: vec![],
            bind_via: BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
        };

        let api = ApiSurface {
            crate_name: "sample-core".to_string(),
            version: "0.1.0".to_string(),
            types: vec![trait_def.clone()],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: Default::default(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: vec![],
        };

        let output = super::gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "sample_core",
            "SampleCoreError",
            "SampleCoreError::from({msg})",
            &api,
        )
        .expect("gen_trait_bridge must succeed for TextProcessor");

        assert!(
            output.code.contains("pub async fn dispose"),
            "bridge struct must expose `dispose()` to release TSFN and allow vitest workers to exit;\nactual code:\n{}",
            output.code
        );
    }
}
