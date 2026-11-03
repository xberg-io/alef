use super::Pyo3Backend;
use crate::core::backend::{Backend, BuildConfig, Capabilities, GeneratedFile};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

#[derive(Default)]
pub(super) struct ConfiguredPyo3Backend {
    pub(super) host_aliases: crate::extensions::python_host_aliases::PythonHostAliases,
}

impl ConfiguredPyo3Backend {
    pub(super) fn validate_host_aliases(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<()> {
        self.host_aliases.validate(api)?;
        for typ in api
            .types
            .iter()
            .filter(|typ| !self.host_aliases.for_trait(typ).is_empty())
        {
            anyhow::ensure!(
                config.trait_bridges_for(Language::Python).any(|bridge| {
                    crate::backends::pyo3::trait_bridge::active_bridge_trait(bridge, api)
                        .is_some_and(|active| active.rust_path == typ.rust_path)
                }),
                "Python host aliases: trait `{}` has no active Python bridge",
                typ.rust_path
            );
        }
        Ok(())
    }
}

impl Pyo3Backend {
    /// Configure explicit Result callback aliases for this backend instance.
    pub fn with_host_aliases(raw: &toml::Value) -> anyhow::Result<impl Backend + use<>> {
        Ok(Self::with_parsed_host_aliases(
            crate::extensions::python_host_aliases::PythonHostAliases::from_raw(raw)?,
        ))
    }

    pub(crate) fn with_parsed_host_aliases(
        host_aliases: crate::extensions::python_host_aliases::PythonHostAliases,
    ) -> impl Backend + use<> {
        ConfiguredPyo3Backend { host_aliases }
    }
}

impl Backend for Pyo3Backend {
    fn name(&self) -> &str {
        "pyo3"
    }
    fn language(&self) -> Language {
        Language::Python
    }
    fn capabilities(&self) -> Capabilities {
        ConfiguredPyo3Backend::default().capabilities()
    }
    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        ConfiguredPyo3Backend::default().generate_bindings(api, config)
    }
    fn generate_type_stubs(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        ConfiguredPyo3Backend::default().generate_type_stubs(api, config)
    }
    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        ConfiguredPyo3Backend::default().generate_public_api(api, config)
    }
    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        ConfiguredPyo3Backend::default().generate_service_api(api, config)
    }
    fn build_config(&self) -> Option<BuildConfig> {
        ConfiguredPyo3Backend::default().build_config()
    }
    fn trait_bridge_registration_surface(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> Vec<crate::core::backend::TraitBridgeRegistrationSurface> {
        ConfiguredPyo3Backend::default().trait_bridge_registration_surface(api, config)
    }
}
