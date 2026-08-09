//! Downloadable-component bridge functions for flutter_rust_bridge.

use crate::core::config::ResolvedCrateConfig;

pub(super) fn generate(config: &ResolvedCrateConfig) -> Option<String> {
    if config.components.is_empty() {
        return None;
    }
    let runtime = crate::backends::native_components::generate(config, "/components.lock.json");
    Some(format!(
        r#"{runtime}

pub fn component_load(component: String) -> Result<(), String> {{
    alef_component_load(&component)
}}

pub fn component_prefetch(component: Option<String>) -> Result<Vec<String>, String> {{
    alef_component_prefetch(component.map(|name| vec![name]))
}}

#[frb(sync)]
pub fn component_status(component: String) -> Result<String, String> {{
    alef_component_status(&component)
}}

#[frb(sync)]
pub fn component_cache_path(component: String) -> Result<String, String> {{
    alef_component_cache_path(&component)
}}"#,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ComponentProfileConfig;

    fn component_config() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: "demo-core".into(),
            components: vec![ComponentProfileConfig {
                name: "fast".into(),
                contract: "engine".into(),
                implementation: "demo_core::FastEngine".into(),
                features: vec!["fast".into()],
                default_features: false,
                targets: vec!["aarch64-apple-darwin".into()],
            }],
            ..ResolvedCrateConfig::default()
        }
    }

    #[test]
    fn emits_non_blocking_frb_component_api_and_shared_runtime() {
        let generated = generate(&component_config()).expect("component API");
        assert!(generated.contains("pub fn component_load"));
        assert!(!generated.contains("#[frb(sync)]\npub fn component_load"));
        assert!(generated.contains("pub fn component_prefetch"));
        assert!(generated.contains("components.lock.json"));
        assert!(generated.contains("unsupported on this host"));
        assert!(generated.contains("DEMO_CORE_COMPONENT_CACHE"));
    }

    #[test]
    fn configured_component_dependencies_are_not_duplicated() {
        let mut config = component_config();
        config.dart = Some(crate::core::config::DartConfig {
            extra_dependencies: [
                (
                    "alef-component-runtime".to_string(),
                    toml::Value::String("9.9.9".to_string()),
                ),
                ("directories".to_string(), toml::Value::String("5".to_string())),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        });
        let manifest = super::super::cargo::emit_cargo_toml(
            "packages/dart/rust",
            &crate::core::ir::ApiSurface::default(),
            &config,
            "demo_core",
        )
        .content;

        assert_eq!(manifest.matches("alef-component-runtime =").count(), 1);
        assert_eq!(manifest.matches("directories =").count(), 1);
        assert_eq!(manifest.matches("alef-component-abi =").count(), 1);
    }
}
