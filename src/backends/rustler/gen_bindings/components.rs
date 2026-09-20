//! Elixir-facing downloadable component management.

use crate::core::config::ResolvedCrateConfig;

pub(super) fn generate(config: &ResolvedCrateConfig) -> String {
    let runtime = crate::backends::native_components::generate(config, "/../../components.lock.json");
    format!(
        r#"{runtime}

#[rustler::nif(schedule = "DirtyIo")]
pub fn component_load(component: String) -> Result<(), String> {{
    alef_component_load(&component)
}}

#[rustler::nif(schedule = "DirtyIo")]
pub fn component_prefetch(components: Option<Vec<String>>) -> Result<Vec<String>, String> {{
    alef_component_prefetch(components)
}}

#[rustler::nif(schedule = "DirtyIo")]
pub fn component_status(component: String) -> Result<String, String> {{
    alef_component_status(&component)
}}

#[rustler::nif(schedule = "DirtyIo")]
pub fn component_cache_path(component: String) -> Result<String, String> {{
    alef_component_cache_path(&component)
}}"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{ComponentConfig, ComponentContractConfig, ComponentProvidesConfig};

    #[test]
    fn schedules_downloads_on_dirty_io_and_reads_package_lock() {
        let config = ResolvedCrateConfig {
            name: "demo-core".into(),
            component_contracts: vec![ComponentContractConfig {
                name: "engine".into(),
                trait_path: "demo_core::Engine".into(),
                interface_version: 1,
            }],
            components: vec![ComponentConfig {
                name: "fast".into(),
                provides: vec![ComponentProvidesConfig {
                    contract: "engine".into(),
                    implementation: "demo_core::FastEngine".into(),
                }],
                features: vec!["fast".into()],
                default_features: false,
                targets: Some(vec!["x86_64-unknown-linux-gnu".into()]),
                bundled_on: Vec::new(),
            }],
            ..ResolvedCrateConfig::default()
        };

        let generated = generate(&config);
        assert!(generated.contains("/../../components.lock.json"));
        // `component_status` and `component_cache_path` stat the on-disk cache the same way
        // `component_load` and `component_prefetch` download into it, so all four NIFs must
        // run off the BEAM scheduler.
        assert_eq!(generated.matches("schedule = \"DirtyIo\"").count(), 4);
        assert!(generated.contains("pub fn component_load"));
        assert!(generated.contains("pub fn component_prefetch"));
        assert!(generated.contains("pub fn component_status"));
        assert!(generated.contains("pub fn component_cache_path"));
    }
}
