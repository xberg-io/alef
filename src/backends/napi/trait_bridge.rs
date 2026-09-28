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

pub use bridge::{gen_trait_bridge, is_visitor_bridge};
pub use bridge_functions::gen_bridge_function;
pub use bridge_generator::NapiBridgeGenerator;
pub use options_field_bridge::gen_options_field_bridge_function;
pub use typescript_bridge::gen_typescript_trait_bridge_files;

/// Find the first parameter index and bridge config where the parameter's named type
/// matches a trait bridge's `type_alias`.
///
/// Returns `None` when no bridge applies.
pub use crate::codegen::generators::trait_bridge::find_bridge_param;

/// The `exclude_languages` spellings that name this target: the language (`"node"`) and the
/// backend (`"napi"`). Both are honoured so a consumer who names either one gets the same
/// answer from every site that asks. ~keep
pub const TARGET_SPELLINGS: [&str; 2] = ["node", "napi"];

/// Whether `bridge` is emitted for the NAPI target at all.
///
/// Every NAPI site that decides whether a bridge exists — the `#[napi]` items, the options-field
/// wiring, and `NapiBackend::trait_bridge_registration_surface` — asks this, so they cannot
/// disagree and leave one pass referring to a struct another pass never wrote. ~keep
pub fn targets_napi(bridge: &TraitBridgeConfig) -> bool {
    crate::codegen::generators::trait_bridge::bridge_targets_language(bridge, &TARGET_SPELLINGS)
}

/// Whether `config` has at least one active, NON-visitor (plugin-flavor) trait bridge for napi —
/// the ones that need `ThreadsafeFunction` fields, `AlefJsReply` decoding, and the runtime
/// scaffolding that go with them (the `AlefJsReply` preamble; the napi `serde-json` feature).
/// A crate whose only configured bridge is visitor-flavored (e.g. an HTML-node-visitor-style
/// callback) needs none of it — see [`is_visitor_bridge`]'s doc for why.
pub fn has_non_visitor_trait_bridges(
    config: &crate::core::config::ResolvedCrateConfig,
    api: &crate::core::ir::ApiSurface,
) -> bool {
    config.trait_bridges.iter().any(|bridge_cfg| {
        active_bridge_trait(bridge_cfg, api)
            .map(|trait_type| !is_visitor_bridge(trait_type, bridge_cfg))
            .unwrap_or(false)
    })
}

/// The trait a bridge wraps, when NAPI emits that bridge at all.
pub fn active_bridge_trait<'a>(
    bridge: &TraitBridgeConfig,
    api: &'a crate::core::ir::ApiSurface,
) -> Option<&'a crate::core::ir::TypeDef> {
    crate::codegen::generators::trait_bridge::active_bridge_trait_def(bridge, api, &TARGET_SPELLINGS)
}

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

    /// The object branch of a visitor result guarded `coerce_to_object` inside a second `if`
    /// whose outer block held nothing else — `clippy::collapsible_if` in edition 2024, which the
    /// generated napi crate is built with under `-D warnings`. It must be one let-chain. ~keep
    #[test]
    fn visitor_result_object_branch_uses_a_let_chain_not_a_nested_if() {
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

        assert!(
            output.code.contains(
                "if val.get_type().ok() == Some(napi::bindgen_prelude::ValueType::Object)\n                    && let Ok(obj) = val.coerce_to_object()\n                {"
            ),
            "object branch must be a let-chain:\n{}",
            output.code
        );
        assert!(
            !output
                .code
                .contains("{\n                    if let Ok(obj) = val.coerce_to_object() {"),
            "clippy::collapsible_if: the object branch must not stay a nested if:\n{}",
            output.code
        );
    }

    #[test]
    fn plugin_trait_bridge_emits_dispose_method_on_rust_struct() {
        use crate::core::config::{BridgeBinding, TraitBridgeConfig};
        use crate::core::ir::{ApiSurface, MethodDef, TypeRef};

        let process_method = MethodDef {
            name: "process".to_string(),
            return_type: TypeRef::Named("String".to_string()),
            ..Default::default()
        };

        let trait_def = crate::core::ir::TypeDef {
            name: "TextProcessor".to_string(),
            rust_path: "sample_core::TextProcessor".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![process_method],
            is_opaque: false,
            is_clone: false,
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
        };

        let bridge_cfg = TraitBridgeConfig {
            exclude_functions: Vec::new(),
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
            unresolved_modules: Vec::new(),
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

        let code = &output.code;

        assert!(
            code.contains("pub async fn dispose"),
            "bridge struct must expose `dispose()` to release TSFN and allow vitest workers to exit;\nactual code:\n{code}"
        );

        assert!(
            code.contains("ObjectRef<false>"),
            "bridge must hold an unref'able ObjectRef<false>, not a borrowed Object;\nactual code:\n{code}"
        );
        assert!(
            !code.contains("inner: napi::bindgen_prelude::Object<'static>"),
            "bridge must NOT store a borrowed Object transmuted to 'static (event-loop pin);\nactual code:\n{code}"
        );

        assert!(
            code.contains("fn release_ref") && code.contains(".unref(") && code.contains("self.release_ref()"),
            "bridge must release the napi reference via unref() in dispose()/Drop;\nactual code:\n{code}"
        );
        assert!(
            code.contains(&format!("impl Drop for {}", "JsTextProcessorBridge")),
            "bridge must impl Drop to defensively release the reference;\nactual code:\n{code}"
        );

        assert!(
            code.contains("self.obj(&__env)") && !code.contains("self.inner"),
            "method bodies must fetch the object via the disposable self.obj helper, not self.inner;\nactual code:\n{code}"
        );
    }
}
