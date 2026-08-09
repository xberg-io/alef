#[cfg(test)]
mod cfg_variant_e2e_tests;
mod components;
mod functions;
mod helpers;
mod native;
mod public_api;
mod public_api_args;
mod public_api_delegates;
mod public_api_opaque_methods;
mod public_api_render;
mod public_files;
mod rust_items;
mod service_api;
#[cfg(test)]
mod tests;
mod types;

use crate::core::backend::{
    Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, TraitBridgeRegistrationSurface,
};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

pub struct RustlerBackend;

impl Backend for RustlerBackend {
    fn name(&self) -> &str {
        "rustler"
    }

    fn language(&self) -> Language {
        Language::Elixir
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_service_api: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        crate::codegen::config_gen::validate_rust_default_functions(api)?;
        native::generate_bindings(&crate::backends::ir_order::with_sorted_items(api), config)
    }

    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        public_api::generate_public_api(&crate::backends::ir_order::with_sorted_items(api), config)
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        service_api::generate(&crate::backends::ir_order::with_sorted_items(api), config)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "mix",
            crate_suffix: "-rustler",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }

    /// Elixir consumers call the delegates on the generated app module, which forward to the
    /// same-named NIFs on `<AppModule>.Native`. `trait_bridge::active_bridge_trait` is the gate
    /// `public_api_delegates` applies — `exclude_languages` under both the `elixir` and `rustler`
    /// spellings, plus the trait resolving in the `ApiSurface`. ~keep
    fn trait_bridge_registration_surface(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> Vec<TraitBridgeRegistrationSurface> {
        use heck::ToPascalCase;
        use public_api_delegates::elixir_delegate_name;

        let app_module = config.elixir_app_name().to_pascal_case();
        let qualified = |configured: &Option<String>| {
            configured
                .as_deref()
                .map(|name| format!("{app_module}.{}", elixir_delegate_name(name)))
        };
        config
            .trait_bridges
            .iter()
            .filter(|bridge| crate::backends::rustler::trait_bridge::active_bridge_trait(bridge, api).is_some())
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
