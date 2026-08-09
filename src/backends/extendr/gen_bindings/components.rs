//! R-facing downloadable component management.

use crate::core::config::ResolvedCrateConfig;

pub(super) fn generate(config: &ResolvedCrateConfig) -> String {
    let runtime = crate::backends::native_components::generate(config, "/../../components.lock.json");
    format!(
        r#"{runtime}

#[extendr]
fn component_load(component: &str) -> extendr_api::Result<()> {{
    alef_component_load(component).map_err(extendr_api::Error::Other)
}}

#[extendr]
fn component_prefetch(components: Option<Vec<String>>) -> extendr_api::Result<Vec<String>> {{
    alef_component_prefetch(components).map_err(extendr_api::Error::Other)
}}

#[extendr]
fn component_status(component: &str) -> extendr_api::Result<String> {{
    alef_component_status(component).map_err(extendr_api::Error::Other)
}}

#[extendr]
fn component_cache_path(component: &str) -> extendr_api::Result<String> {{
    alef_component_cache_path(component).map_err(extendr_api::Error::Other)
}}"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{ComponentContractConfig, ComponentProfileConfig};

    #[test]
    fn exposes_registered_r_operations_using_package_lock() {
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
        assert!(generated.contains("/../../components.lock.json"));
        assert_eq!(generated.matches("#[extendr]").count(), 4);
        assert!(generated.contains("fn component_load"));
        assert!(generated.contains("fn component_prefetch"));
        assert!(generated.contains("fn component_status"));
        assert!(generated.contains("fn component_cache_path"));
    }
}
