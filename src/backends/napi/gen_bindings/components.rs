//! Node-facing downloadable component management.

use crate::core::config::ResolvedCrateConfig;

pub(super) fn generate(config: &ResolvedCrateConfig) -> String {
    let runtime = crate::backends::native_components::generate(config, "/components.lock.json");
    format!(
        r#"{runtime}

pub struct AlefComponentLoadTask {{
    component: String,
}}

impl napi::Task for AlefComponentLoadTask {{
    type Output = ();
    type JsValue = ();

    fn compute(&mut self) -> napi::Result<Self::Output> {{
        alef_component_load(&self.component)
            .map_err(|error| napi::Error::new(napi::Status::GenericFailure, error))
    }}

    fn resolve(&mut self, _env: napi::Env, output: Self::Output) -> napi::Result<Self::JsValue> {{
        Ok(output)
    }}
}}

#[napi]
pub fn component_load(component: String) -> napi::bindgen_prelude::AsyncTask<AlefComponentLoadTask> {{
    napi::bindgen_prelude::AsyncTask::new(AlefComponentLoadTask {{ component }})
}}

pub struct AlefComponentPrefetchTask {{
    components: Option<Vec<String>>,
}}

impl napi::Task for AlefComponentPrefetchTask {{
    type Output = Vec<String>;
    type JsValue = Vec<String>;

    fn compute(&mut self) -> napi::Result<Self::Output> {{
        alef_component_prefetch(self.components.take())
            .map_err(|error| napi::Error::new(napi::Status::GenericFailure, error))
    }}

    fn resolve(&mut self, _env: napi::Env, output: Self::Output) -> napi::Result<Self::JsValue> {{
        Ok(output)
    }}
}}

#[napi]
pub fn component_prefetch(
    components: Option<Vec<String>>,
) -> napi::bindgen_prelude::AsyncTask<AlefComponentPrefetchTask> {{
    napi::bindgen_prelude::AsyncTask::new(AlefComponentPrefetchTask {{ components }})
}}

#[napi]
pub fn component_status(component: String) -> napi::Result<String> {{
    alef_component_status(&component)
        .map_err(|error| napi::Error::new(napi::Status::GenericFailure, error))
}}

#[napi]
pub fn component_cache_path(component: String) -> napi::Result<String> {{
    alef_component_cache_path(&component)
        .map_err(|error| napi::Error::new(napi::Status::GenericFailure, error))
}}"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{ComponentContractConfig, ComponentProfileConfig};

    #[test]
    fn exposes_non_blocking_node_download_operations() {
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
        assert!(generated.contains("AsyncTask<AlefComponentLoadTask>"));
        assert!(generated.contains("impl napi::Task for AlefComponentPrefetchTask"));
        assert!(generated.contains("pub fn component_status"));
        assert!(generated.contains("pub fn component_cache_path"));
    }
}
