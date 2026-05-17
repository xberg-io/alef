//! PyO3 module initialisation function generation.

use ahash::AHashSet;
use alef_core::config::{Language, ResolvedCrateConfig};
use alef_core::ir::ApiSurface;

/// Generate the async runtime initialization function.
pub(super) fn gen_async_runtime_init() -> String {
    r#"#[pyfunction]
pub fn init_async_runtime() -> PyResult<()> {
    // Tokio runtime auto-initializes on first future_into_py call
    Ok(())
}"#
    .to_string()
}

/// Generate the module initialization function.
pub(super) fn gen_module_init(module_name: &str, api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let mut lines = vec![
        "#[pymodule]".to_string(),
        format!("pub fn {module_name}(m: &Bound<'_, PyModule>) -> PyResult<()> {{"),
    ];

    // Check if we have async functions
    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));

    if has_async {
        lines.push("    m.add_function(wrap_pyfunction!(init_async_runtime, m)?)?;".to_string());
    }

    // Custom registrations (before generated ones so hand-written classes are registered first)
    if let Some(reg) = config.custom_registrations.for_language(Language::Python) {
        for class in &reg.classes {
            lines.push(format!("    m.add_class::<{class}>()?;"));
        }
        for func in &reg.functions {
            lines.push(format!("    m.add_function(wrap_pyfunction!({func}, m)?)?;"));
        }
        for call in &reg.init_calls {
            lines.push(format!("    {call}"));
        }
    }

    let mod_exclude_functions: ahash::AHashSet<String> = config
        .python
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let mod_exclude_types: ahash::AHashSet<String> = config
        .python
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    // Capsule types have no #[pyclass] struct — emitting m.add_class::<T>() for them
    // causes a compile error because the struct was never generated.
    let capsule_type_names: AHashSet<String> = config
        .python
        .as_ref()
        .map(|c| c.capsule_types.keys().cloned().collect())
        .unwrap_or_default();

    // Error types are registered via m.add(...) with the exception types, not m.add_class.
    let error_type_names: AHashSet<&str> = api.errors.iter().map(|e| e.name.as_str()).collect();

    // Deduplicate registered types and enums
    let mut registered: AHashSet<String> = AHashSet::new();
    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !mod_exclude_types.contains(&typ.name))
    {
        // Error types are handled by gen_pyo3_error_registration below.
        if error_type_names.contains(typ.name.as_str()) {
            continue;
        }
        // Capsule types have no #[pyclass] — skip them in the module init.
        if capsule_type_names.contains(typ.name.as_str()) {
            continue;
        }
        if registered.insert(typ.name.clone()) {
            lines.push(format!("    m.add_class::<{}>()?;", typ.name));
        }
    }
    for enum_def in &api.enums {
        if registered.insert(enum_def.name.clone()) {
            lines.push(format!("    m.add_class::<{}>()?;", enum_def.name));
        }
    }

    for func in &api.functions {
        if mod_exclude_functions.contains(&func.name) {
            continue;
        }
        lines.push(format!("    m.add_function(wrap_pyfunction!({}, m)?)?;", func.name));
    }

    // Register trait bridge registration functions
    for register_fn in crate::trait_bridge::collect_bridge_register_fns(&config.trait_bridges) {
        lines.push(format!("    m.add_function(wrap_pyfunction!({register_fn}, m)?)?;"));
    }
    // Register trait bridge unregister functions. The emitted Rust symbol is
    // `_alef_<unregister_fn>` (see trait_bridge/unregistration_fn.jinja) but pyo3 exposes
    // it under the bare `unregister_*` name via `#[pyo3(name = ...)]`. Without this
    // `m.add_function` call the symbol is not part of the native module and Python
    // callers see `AttributeError: module ... has no attribute 'unregister_*'`.
    for unregister_fn in crate::trait_bridge::collect_bridge_unregister_fns(&config.trait_bridges) {
        lines.push(format!(
            "    m.add_function(wrap_pyfunction!(_alef_{unregister_fn}, m)?)?;"
        ));
    }
    // Register trait bridge clear functions. Same `_alef_<clear_fn>` symbol convention
    // as unregister — must be added to the module so Python callers can access them.
    for clear_fn in crate::trait_bridge::collect_bridge_clear_fns(&config.trait_bridges) {
        lines.push(format!("    m.add_function(wrap_pyfunction!(_alef_{clear_fn}, m)?)?;"));
    }

    // Register error exception types
    let mut seen_registrations = AHashSet::new();
    for error in &api.errors {
        for reg_line in alef_codegen::error_gen::gen_pyo3_error_registration(error, &mut seen_registrations) {
            lines.push(reg_line);
        }
    }

    lines.push("    Ok(())".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{gen_async_runtime_init, gen_module_init};
    use alef_core::ir::ApiSurface;

    fn make_config() -> alef_core::config::ResolvedCrateConfig {
        let cfg: alef_core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.python]
module_name = "_test_lib"
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    /// gen_async_runtime_init produces a valid #[pyfunction] stub.
    #[test]
    fn gen_async_runtime_init_produces_pyfunction() {
        let result = gen_async_runtime_init();
        assert!(result.contains("#[pyfunction]"));
        assert!(result.contains("init_async_runtime"));
    }

    /// gen_module_init with empty API produces a function wrapping Ok(()).
    #[test]
    fn gen_module_init_empty_api_produces_ok() {
        let api = ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
        };
        let config = make_config();
        let result = gen_module_init("_test_lib", &api, &config);
        assert!(result.contains("#[pymodule]"));
        assert!(result.contains("Ok(())"));
    }
}
