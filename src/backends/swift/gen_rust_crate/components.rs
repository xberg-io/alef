//! Rust component-manager surface bridged into Swift.

use crate::core::config::ResolvedCrateConfig;

pub(super) fn extern_block(config: &ResolvedCrateConfig) -> Option<String> {
    (!config.components.is_empty()).then(|| {
        "    extern \"Rust\" {\n\
             fn component_load(component: String) -> String;\n\
             fn component_prefetch(component: Option<String>) -> String;\n\
             fn component_status(component: String) -> String;\n\
             fn component_cache_path(component: String) -> String;\n\
             }\n"
        .to_string()
    })
}

pub(super) fn implementation(config: &ResolvedCrateConfig) -> Option<String> {
    if config.components.is_empty() {
        return None;
    }
    let runtime = crate::backends::native_components::generate(config, "/components.lock.json");
    Some(format!(
        r#"{runtime}

fn alef_swift_component_response<T: serde::Serialize>(result: Result<T, String>) -> String {{
    match result {{
        Ok(value) => serde_json::json!({{ "ok": value }}).to_string(),
        Err(error) => serde_json::json!({{ "err": error }}).to_string(),
    }}
}}

pub fn component_load(component: String) -> String {{
    alef_swift_component_response(alef_component_load(&component).map(|_| true))
}}

pub fn component_prefetch(component: Option<String>) -> String {{
    alef_swift_component_response(alef_component_prefetch(component.map(|name| vec![name])))
}}

pub fn component_status(component: String) -> String {{
    alef_swift_component_response(alef_component_status(&component))
}}

pub fn component_cache_path(component: String) -> String {{
    alef_swift_component_response(alef_component_cache_path(&component))
}}"#,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ComponentProfileConfig;

    fn config() -> ResolvedCrateConfig {
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
    fn emits_manager_bridge_from_shared_runtime_without_native_result_string() {
        let config = config();
        let declarations = extern_block(&config).expect("extern block");
        let implementation = implementation(&config).expect("implementation");
        assert!(declarations.contains("fn component_prefetch(component: Option<String>) -> String"));
        assert!(!declarations.contains("Result<String, String>"));
        assert!(implementation.contains("components.lock.json"));
        assert!(implementation.contains("unsupported on this host"));
        assert!(implementation.contains("DEMO_CORE_COMPONENT_CACHE"));
        assert!(implementation.contains("alef_swift_component_response"));
    }
}
