#[cfg(test)]
mod cfg_variant_e2e_tests;
mod components;
mod functions;
mod helpers;
mod opaque_files;
mod php_types;
mod public_api;
mod rust_bindings;
mod rust_items;
mod serde_defaults;
pub mod service_api;
#[cfg(test)]
mod tests;
mod type_stubs;
pub mod types;
#[cfg(test)]
mod visitor_interface_tests;

use crate::core::backend::{
    Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, TraitBridgeRegistrationSurface,
};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

pub struct PhpBackend;

impl Backend for PhpBackend {
    fn name(&self) -> &str {
        "php"
    }

    fn language(&self) -> Language {
        Language::Php
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: false,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_service_api: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // Order the IR once, before anything reads it: every emission loop below concatenates
        // api.types/enums/functions/errors into a single generated file in Vec order. ~keep
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        let api = &sorted_api;
        crate::codegen::config_gen::validate_rust_default_functions(api)?;
        rust_bindings::generate_bindings(api, config)
    }

    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        // Order the IR once, before anything reads it: every emission loop below concatenates
        // api.types/enums/functions/errors into a single generated file in Vec order. ~keep
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        let api = &sorted_api;
        public_api::generate_public_api(api, config)
    }

    fn generate_type_stubs(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        // Order the IR once, before anything reads it: every emission loop below concatenates
        // api.types/enums/functions/errors into a single generated file in Vec order. ~keep
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        let api = &sorted_api;
        type_stubs::generate_type_stubs(api, config)
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        // Order the IR once, before anything reads it: every emission loop below concatenates
        // api.types/enums/functions/errors into a single generated file in Vec order. ~keep
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        let api = &sorted_api;
        service_api::generate(api, config)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-php",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }

    /// PHP consumers call the static methods on the public wrapper class emitted by
    /// `generate_public_api`, which forward to the identically named methods on the `…Api`
    /// extension class. Both wrapper passes gate on
    /// `php::trait_bridge::active_bridge_trait`, so this asks the same question and reports
    /// only what those passes emitted. ~keep
    fn trait_bridge_registration_surface(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> Vec<TraitBridgeRegistrationSurface> {
        use crate::backends::php::naming::{php_bridge_method_name, php_public_class_name};

        let class_name = php_public_class_name(&config.php_extension_name());
        let qualified = |configured: &Option<String>| {
            configured
                .as_deref()
                .map(|name| format!("{class_name}::{}", php_bridge_method_name(name)))
        };
        config
            .trait_bridges
            .iter()
            .filter(|bridge| crate::backends::php::trait_bridge::active_bridge_trait(bridge, api).is_some())
            .filter(|bridge| {
                bridge.register_fn.is_some() || bridge.unregister_fn.is_some() || bridge.clear_fn.is_some()
            })
            .map(|bridge| TraitBridgeRegistrationSurface {
                trait_name: bridge.trait_name.clone(),
                register_symbol: qualified(&bridge.register_fn),
                unregister_symbol: qualified(&bridge.unregister_fn),
                clear_symbol: qualified(&bridge.clear_fn),
            })
            .collect()
    }
}
