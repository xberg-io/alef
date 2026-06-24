use super::bridge_generator::NapiBridgeGenerator;
use super::visitor_bridge::gen_visitor_bridge;
use crate::codegen::generators::trait_bridge::{BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, TypeDef};
use std::collections::HashMap;

pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &ApiSurface,
) -> anyhow::Result<BridgeOutput> {
    // Build type name → rust_path lookup (converted to String-owned HashMap)
    let type_paths: HashMap<String, String> = api
        .types
        .iter()
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .chain(
            api.enums
                .iter()
                .map(|e| (e.name.clone(), e.rust_path.replace('-', "_"))),
        )
        // Include excluded types so trait methods referencing them (for example, `&HiddenDoc`)
        // are qualified with the full Rust path rather than emitting the bare type name.
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
        )
        .collect();

    // Visitor-style bridge: all methods have defaults, no registry, no super-trait.
    let is_visitor_bridge = bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl);

    if is_visitor_bridge {
        let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Js", bridge_cfg);
        let trait_path = trait_type.rust_path.replace('-', "_");
        let code = gen_visitor_bridge(
            trait_type,
            bridge_cfg,
            &struct_name,
            &trait_path,
            core_import,
            &type_paths,
            api,
        )?;
        Ok(BridgeOutput { imports: vec![], code })
    } else {
        // Use the IR-driven TraitBridgeGenerator infrastructure.
        //
        // Classify which callback params get native-object marshalling using the SHARED rule
        // (`native_marshalled_struct_params`) so the allowlist is identical to what other backends
        // consult. For such params the bridge hands the host the binding's native JS object (the
        // `#[napi(object)]` DTO, built via the same `From<core::T>` conversion used for return
        // values) instead of a debug/JSON string. The DTO is named with the `"Js"` prefix — the
        // same prefix the bridge uses for its wrapper struct and the binding's default node prefix.
        let struct_param_types =
            crate::codegen::generators::trait_bridge::native_marshalled_struct_params(trait_type, api);
        let generator = NapiBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
            struct_param_types,
            type_prefix: "Js".to_string(),
        };
        let lifetime_type_names: std::collections::HashSet<String> = api
            .types
            .iter()
            .filter(|t| t.has_lifetime_params)
            .map(|t| t.name.clone())
            .collect();
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "Js",
            type_paths,
            lifetime_type_names,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };

        // For NAPI bridges, we generate the struct with a cancellation_token field manually
        // to support explicit async cleanup via dispose()
        let imports = generator.bridge_imports();
        let mut code = String::with_capacity(4096);

        // Custom NAPI struct with cancellation_token field
        let wrapper_name = spec.wrapper_name();
        code.push_str(&crate::backends::napi::template_env::render(
            "napi_bridge_struct.jinja",
            minijinja::context! {
                wrapper_name => wrapper_name,
            },
        ));
        code.push_str("\n\n");

        // Debug impl (required by Plugin super-trait Debug bound)
        code.push_str(&crate::codegen::generators::trait_bridge::gen_bridge_debug_impl(&spec));
        code.push_str("\n\n");

        // Constructor (impl block with new() and dispose())
        code.push_str(&generator.gen_constructor(&spec));
        code.push_str("\n\n");

        // Plugin super-trait impl (if applicable)
        if let Some(plugin_impl) = crate::codegen::generators::trait_bridge::gen_bridge_plugin_impl(&spec, &generator) {
            code.push_str(&plugin_impl);
            code.push_str("\n\n");
        }

        // Trait impl
        code.push_str(&crate::codegen::generators::trait_bridge::gen_bridge_trait_impl(
            &spec, &generator,
        ));

        // Registration function — only when register_fn is configured
        if let Some(reg_fn_code) =
            crate::codegen::generators::trait_bridge::gen_bridge_registration_fn(&spec, &generator)
        {
            code.push_str("\n\n");
            code.push_str(&reg_fn_code);
        }

        // Unregistration function — only when unregister_fn is configured AND
        // the backend has opted in (non-empty body).
        if let Some(unreg_fn_code) =
            crate::codegen::generators::trait_bridge::gen_bridge_unregistration_fn(&spec, &generator)
        {
            code.push_str("\n\n");
            code.push_str(&unreg_fn_code);
        }

        // Clear-all function — only when clear_fn is configured AND the backend
        // has opted in (non-empty body).
        if let Some(clear_fn_code) = crate::codegen::generators::trait_bridge::gen_bridge_clear_fn(&spec, &generator) {
            code.push_str("\n\n");
            code.push_str(&clear_fn_code);
        }

        Ok(BridgeOutput { imports, code })
    }
}
