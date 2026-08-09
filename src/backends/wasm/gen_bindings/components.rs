//! Explicit component API for WebAssembly targets.

use crate::core::config::ResolvedCrateConfig;

const UNSUPPORTED_MESSAGE: &str = "downloadable native components are unsupported on WebAssembly because wasm32 cannot dynamically load host libraries";

pub(super) fn generate(config: &ResolvedCrateConfig) -> Option<String> {
    if config.components.is_empty() {
        return None;
    }

    Some(format!(
        r#"fn alef_component_unsupported() -> wasm_bindgen::JsValue {{
    wasm_bindgen::JsValue::from_str("{UNSUPPORTED_MESSAGE}")
}}

/// Downloadable native components require host dynamic-library loading and are not available in WebAssembly.
#[wasm_bindgen::prelude::wasm_bindgen(js_name = componentLoad)]
pub fn component_load(_component: String) -> Result<(), wasm_bindgen::JsValue> {{
    Err(alef_component_unsupported())
}}

/// Downloadable native components require host dynamic-library loading and are not available in WebAssembly.
#[wasm_bindgen::prelude::wasm_bindgen(js_name = componentPrefetch)]
pub fn component_prefetch(_components: Option<js_sys::Array>) -> Result<js_sys::Array, wasm_bindgen::JsValue> {{
    Err(alef_component_unsupported())
}}

/// Downloadable native components require host dynamic-library loading and are not available in WebAssembly.
#[wasm_bindgen::prelude::wasm_bindgen(js_name = componentStatus)]
pub fn component_status(_component: String) -> Result<String, wasm_bindgen::JsValue> {{
    Err(alef_component_unsupported())
}}

/// Downloadable native components require host dynamic-library loading and are not available in WebAssembly.
#[wasm_bindgen::prelude::wasm_bindgen(js_name = componentCachePath)]
pub fn component_cache_path(_component: String) -> Result<String, wasm_bindgen::JsValue> {{
    Err(alef_component_unsupported())
}}"#,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ComponentProfileConfig;

    #[test]
    fn emits_explicit_unsupported_component_surface() {
        let config = ResolvedCrateConfig {
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
        let generated = generate(&config).expect("configured components must emit an API");

        for operation in [
            "component_load",
            "component_prefetch",
            "component_status",
            "component_cache_path",
        ] {
            assert!(generated.contains(operation), "missing {operation}:\n{generated}");
        }
        assert!(generated.contains("unsupported on WebAssembly"));
        assert!(generated.contains("js_name = componentLoad"));
    }

    #[test]
    fn omits_component_surface_when_no_components_are_configured() {
        assert!(generate(&ResolvedCrateConfig::default()).is_none());
    }
}
