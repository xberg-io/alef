//! PHP-facing downloadable component management.

use crate::core::config::ResolvedCrateConfig;

pub(super) fn generate(config: &ResolvedCrateConfig) -> String {
    let runtime = crate::backends::native_components::generate(config, "/components.lock.json");
    format!(
        r#"{runtime}

fn alef_component_php_error(error: String) -> ext_php_rs::exception::PhpException {{
    ext_php_rs::exception::PhpException::default(error)
}}

#[php_function]
pub fn component_load(component: String) -> Result<(), ext_php_rs::exception::PhpException> {{
    alef_component_load(&component).map_err(alef_component_php_error)
}}

#[php_function]
pub fn component_prefetch(
    components: Option<Vec<String>>,
) -> Result<Vec<String>, ext_php_rs::exception::PhpException> {{
    alef_component_prefetch(components).map_err(alef_component_php_error)
}}

#[php_function]
pub fn component_status(component: String) -> Result<String, ext_php_rs::exception::PhpException> {{
    alef_component_status(&component).map_err(alef_component_php_error)
}}

#[php_function]
pub fn component_cache_path(component: String) -> Result<String, ext_php_rs::exception::PhpException> {{
    alef_component_cache_path(&component).map_err(alef_component_php_error)
}}"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{ComponentContractConfig, ComponentProfileConfig};

    #[test]
    fn exposes_php_functions_using_binding_crate_lock() {
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
        assert!(generated.contains("/components.lock.json"));
        assert_eq!(generated.matches("#[php_function]").count(), 4);
        assert!(generated.contains("pub fn component_load"));
        assert!(generated.contains("pub fn component_prefetch"));
        assert!(generated.contains("pub fn component_status"));
        assert!(generated.contains("pub fn component_cache_path"));
    }
}
