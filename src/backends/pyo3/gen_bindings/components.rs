//! Python-facing component manager generation.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use anyhow::Result;

pub(super) fn gen_component_runtime(api: &ApiSurface, config: &ResolvedCrateConfig) -> Result<String> {
    let runtime = crate::backends::native_components::generate(config, "/components.lock.json");
    let activation = crate::backends::native_components::generate_activation(api, config)?;
    Ok(format!(
        r#"{runtime}

{activation}

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
}}

/// Download, verify, load, and register `component`'s contracts so core-crate
/// code that looks them up through `alef_component_abi::provider` finds them.
/// Call this explicitly before relying on a component; it is never triggered
/// automatically on import.
#[pyfunction]
fn component_activate(py: Python<'_>, component: String) -> PyResult<()> {{
    py.detach(|| alef_component_activate(&component))
        .map_err(alef_component_python_error)
}}"#,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{ComponentConfig, ComponentContractConfig, ComponentProvidesConfig};
    use crate::core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};

    fn fixture() -> (ApiSurface, ResolvedCrateConfig) {
        let api = ApiSurface {
            crate_name: "demo_core".into(),
            version: "1.0.0".into(),
            types: vec![TypeDef {
                name: "Engine".into(),
                rust_path: "demo_core::Engine".into(),
                is_trait: true,
                is_opaque: true,
                methods: vec![MethodDef {
                    name: "run".into(),
                    return_type: TypeRef::Bytes,
                    receiver: Some(ReceiverKind::Ref),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        };
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
        (api, config)
    }

    #[test]
    fn embeds_lock_and_exposes_lazy_component_api() {
        let (api, config) = fixture();
        let generated = gen_component_runtime(&api, &config).unwrap();
        assert!(generated.contains("components.lock.json"));
        assert!(generated.contains("fn component_prefetch"));
        assert!(generated.contains("DEMO_CORE_COMPONENT_CACHE"));
    }

    #[test]
    fn exposes_an_explicit_activation_entry_point() {
        let (api, config) = fixture();
        let generated = gen_component_runtime(&api, &config).unwrap();
        assert!(generated.contains("fn component_activate(py: Python<'_>, component: String)"));
        assert!(generated.contains("alef_component_activate(&component)"));
        assert!(generated.contains("pub struct EngineProxy"));
    }
}
