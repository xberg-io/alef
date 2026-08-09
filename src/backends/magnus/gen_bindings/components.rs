//! Ruby-facing downloadable component management.

use crate::core::config::ResolvedCrateConfig;

pub(super) fn generate(config: &ResolvedCrateConfig) -> String {
    let runtime = crate::backends::native_components::generate(config, "/../../../components.lock.json");
    format!(
        r#"{runtime}

fn alef_component_ruby_error(error: String) -> magnus::Error {{
    // SAFETY: every exported function is called by Ruby with the GVL held.
    let ruby = unsafe {{ magnus::Ruby::get_unchecked() }};
    magnus::Error::new(ruby.exception_runtime_error(), error)
}}

fn component_load(component: String) -> Result<(), magnus::Error> {{
    alef_component_load(&component).map_err(alef_component_ruby_error)
}}

fn component_prefetch(components: Option<Vec<String>>) -> Result<Vec<String>, magnus::Error> {{
    alef_component_prefetch(components).map_err(alef_component_ruby_error)
}}

fn component_status(component: String) -> Result<String, magnus::Error> {{
    alef_component_status(&component).map_err(alef_component_ruby_error)
}}

fn component_cache_path(component: String) -> Result<String, magnus::Error> {{
    alef_component_cache_path(&component).map_err(alef_component_ruby_error)
}}"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{ComponentContractConfig, ComponentProfileConfig};

    #[test]
    fn exposes_ruby_component_operations_from_package_root_lock() {
        let config = ResolvedCrateConfig {
            name: "demo-core".into(),
            component_contracts: vec![ComponentContractConfig {
                name: "engine".into(),
                trait_path: "demo_core::Engine".into(),
                interface_version: 1,
            }],
            components: vec![ComponentProfileConfig {
                name: "fast".into(),
                contract: "engine".into(),
                implementation: "demo_core::FastEngine".into(),
                features: vec!["fast".into()],
                default_features: false,
                targets: vec!["x86_64-unknown-linux-gnu".into()],
            }],
            ..ResolvedCrateConfig::default()
        };

        let generated = generate(&config);
        assert!(generated.contains("/../../../components.lock.json"));
        assert!(generated.contains("fn component_load"));
        assert!(generated.contains("fn component_prefetch"));
        assert!(generated.contains("fn component_status"));
        assert!(generated.contains("fn component_cache_path"));
    }
}
