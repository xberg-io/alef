use super::bridge_generator::NapiBridgeGenerator;
use super::visitor_bridge::gen_visitor_bridge;
use crate::codegen::generators::trait_bridge::{BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, TypeDef};
use std::collections::HashMap;

/// Whether this bridge takes the "visitor" flavor (a per-call callback object living on an
/// options-struct field, e.g. `HtmlVisitor`) rather than the "plugin" flavor (a registry-held
/// `Arc<dyn Trait>`, e.g. `OcrBackend`). Only the plugin flavor gets `ThreadsafeFunction`
/// fields/`AlefJsReply` decoding (see [`NapiBridgeGenerator`]) — a visitor's entry point runs
/// synchronously on the JS thread inside a live N-API frame, so a `ThreadsafeFunction` there
/// would deadlock (it would enqueue onto an event loop blocked in that very stack frame).
///
/// Exposed so callers that decide whether to emit shared per-crate scaffolding (the
/// `AlefJsReply` runtime preamble; the `serde-json` napi feature; `async-trait`) ask the same
/// question this function's own caller does, instead of a coarser "does this crate have ANY
/// `[[crates.trait_bridges]]` entry" check — which would emit that scaffolding for a crate whose
/// only bridge is visitor-flavored and needs none of it (confirmed against a real downstream
/// consumer whose one configured bridge is visitor-only).
pub fn is_visitor_bridge(trait_type: &TypeDef, bridge_cfg: &TraitBridgeConfig) -> bool {
    bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl)
}

pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &ApiSurface,
) -> anyhow::Result<BridgeOutput> {
    let type_paths: HashMap<String, String> = api
        .types
        .iter()
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .chain(
            api.enums
                .iter()
                .map(|e| (e.name.clone(), e.rust_path.replace('-', "_"))),
        )
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
        )
        .collect();

    if is_visitor_bridge(trait_type, bridge_cfg) {
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
        // `#[napi(object)]` DTO, built via the same `From<core::T>` conversion used for return
        let struct_param_types =
            crate::codegen::generators::trait_bridge::native_marshalled_struct_params(trait_type, api);
        let forwardable_defaulted =
            crate::codegen::generators::trait_bridge::forwardable_defaulted_method_names(trait_type, api);
        let generator = NapiBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
            struct_param_types,
            type_prefix: "Js".to_string(),
            forwardable_defaulted,
            api: api.clone(),
            unsupported_args: Default::default(),
            extra_impl_items: Default::default(),
            writeback_conflicts: Default::default(),
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

        let imports = generator.bridge_imports();
        let mut code = String::with_capacity(4096);

        let wrapper_name = spec.wrapper_name();

        let tsfn_aliases = generator.gen_tsfn_aliases(&spec);
        if !tsfn_aliases.is_empty() {
            code.push_str(&tsfn_aliases);
            code.push_str("\n\n");
        }

        let extra_fields: Vec<minijinja::Value> = generator
            .extra_bridge_fields(&spec)
            .into_iter()
            .map(|(name, ty)| crate::alef_context! { name => name, ty => ty })
            .collect();
        code.push_str(&crate::backends::napi::template_env::render(
            "napi_bridge_struct.jinja",
            crate::alef_context! {
                wrapper_name => wrapper_name,
                extra_fields => extra_fields,
            },
        ));
        code.push_str("\n\n");

        code.push_str(&crate::codegen::generators::trait_bridge::gen_bridge_debug_impl(&spec));
        code.push_str("\n\n");

        code.push_str(&generator.gen_constructor(&spec));
        code.push_str("\n\n");

        if let Some(plugin_impl) = crate::codegen::generators::trait_bridge::gen_bridge_plugin_impl(&spec, &generator) {
            code.push_str(&plugin_impl);
            code.push_str("\n\n");
        }

        code.push_str(&crate::codegen::generators::trait_bridge::gen_bridge_trait_impl(
            &spec, &generator,
        ));

        let delegates = crate::codegen::generators::trait_bridge::gen_bridge_default_delegates(&spec, &generator);
        if !delegates.is_empty() {
            code.push_str("\n\n");
            code.push_str(&delegates);
        }

        if let Some(reg_fn_code) =
            crate::codegen::generators::trait_bridge::gen_bridge_registration_fn(&spec, &generator)
        {
            code.push_str("\n\n");
            code.push_str(&reg_fn_code);
        }

        if let Some(unreg_fn_code) =
            crate::codegen::generators::trait_bridge::gen_bridge_unregistration_fn(&spec, &generator)
        {
            code.push_str("\n\n");
            code.push_str(&unreg_fn_code);
        }

        if let Some(clear_fn_code) = crate::codegen::generators::trait_bridge::gen_bridge_clear_fn(&spec, &generator) {
            code.push_str("\n\n");
            code.push_str(&clear_fn_code);
        }

        let extra_impl_items = generator.take_extra_impl_items();
        for item in &extra_impl_items {
            code.push_str("\n\n");
            code.push_str(item);
        }

        let unsupported = generator.take_unsupported_args();
        if !unsupported.is_empty() {
            let sites = unsupported
                .iter()
                .map(|u| {
                    format!(
                        "  - {}::{}({}: {})",
                        u.trait_name, u.method_name, u.param_name, u.type_desc
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::bail!(
                "napi trait bridge for '{}' cannot marshal {} parameter(s) to JS natively:\n{}",
                trait_type.name,
                unsupported.len(),
                sites
            );
        }

        let writeback_conflicts = generator.take_writeback_conflicts();
        if !writeback_conflicts.is_empty() {
            anyhow::bail!(
                "napi trait bridge for '{}' cannot write back {} method(s):\n  - {}",
                trait_type.name,
                writeback_conflicts.len(),
                writeback_conflicts.join("\n  - ")
            );
        }

        Ok(BridgeOutput { imports, code })
    }
}
