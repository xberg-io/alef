//! Python-facing component manager generation.

use crate::core::config::ResolvedCrateConfig;

pub(super) fn gen_component_runtime(config: &ResolvedCrateConfig) -> String {
    let runtime = crate::backends::native_components::generate(config, "/components.lock.json");
    format!(
        r#"{runtime}

fn alef_component_python_error(error: String) -> pyo3::PyErr {{
    pyo3::exceptions::PyRuntimeError::new_err(error)
}}

#[pyfunction]
fn component_load(py: Python<'_>, component: String) -> PyResult<()> {{
    py.detach(|| alef_component_load(&component))
        .map_err(alef_component_python_error)
}}

#[pyfunction]
#[pyo3(signature = (components=None))]
fn component_prefetch(py: Python<'_>, components: Option<Vec<String>>) -> PyResult<Vec<String>> {{
    py.detach(|| alef_component_prefetch(components))
        .map_err(alef_component_python_error)
}}

#[pyfunction]
fn component_status(component: String) -> PyResult<String> {{
    alef_component_status(&component).map_err(alef_component_python_error)
}}

#[pyfunction]
fn component_cache_path(component: String) -> PyResult<String> {{
    alef_component_cache_path(&component).map_err(alef_component_python_error)
}}"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{ComponentContractConfig, ComponentProfileConfig};

    #[test]
    fn embeds_lock_and_exposes_lazy_component_api() {
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
        let generated = gen_component_runtime(&config);
        assert!(generated.contains("components.lock.json"));
        assert!(generated.contains("fn component_prefetch"));
        assert!(generated.contains("DEMO_CORE_COMPONENT_CACHE"));
    }
}
