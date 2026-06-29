//! PyO3 module initialisation function generation.

use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use ahash::AHashSet;

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
        // Provision a worker-thread stack large enough for deep async pipelines. The default
        // pyo3-async-runtimes multi-thread runtime gives workers a small (~2 MB) stack; a deep
        // consumer future (e.g. a multi-stage document/OCR pipeline) overflows it and aborts the
        // process with SIGBUS. This runs at import, before the first `future_into_py` builds the
        // lazy runtime. `tokio` is a direct dependency of every async pyo3 binding crate.
        lines.push("    {".to_string());
        lines.push("        let mut __rt_builder = tokio::runtime::Builder::new_multi_thread();".to_string());
        lines.push("        __rt_builder.enable_all();".to_string());
        lines.push("        __rt_builder.thread_stack_size(16 * 1024 * 1024);".to_string());
        lines.push("        pyo3_async_runtimes::tokio::init(__rt_builder);".to_string());
        lines.push("    }".to_string());
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

    // Service-API entrypoint functions are generated in `service.rs` as
    // `{service_snake}_{entrypoint}` `#[pyfunction]`s — register each so the Python
    // `service.py` wrapper can call them through the native module.
    {
        use heck::ToSnakeCase as _;
        for service in &api.services {
            let service_snake = service.name.to_snake_case();
            for ep in &service.entrypoints {
                lines.push(format!(
                    "    m.add_function(wrap_pyfunction!(service::{service_snake}_{}, m)?)?;",
                    ep.method
                ));
            }
        }
    }

    let mod_exclude_functions: ahash::AHashSet<String> = config
        .python
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let mut mod_exclude_types: ahash::AHashSet<String> = config
        .python
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    // Declared opaque types from `[workspace.opaque_types]` that have a per-language
    // capsule_types override are external host-runtime references — the per-binding wrapper
    // loop skips emitting `#[pyclass]` structs for them, so the module-init loop must also
    // skip the corresponding `m.add_class::<T>` call to avoid `cannot find type` errors.
    // Opaque types WITHOUT a capsule override get a binding-side wrapper struct emitted
    // in mod.rs and MUST be registered here.
    let python_capsule_types: AHashSet<String> = config
        .python
        .as_ref()
        .map(|c| c.capsule_types.keys().cloned().collect())
        .unwrap_or_default();
    for name in config.opaque_types.keys() {
        if python_capsule_types.contains(name) {
            mod_exclude_types.insert(name.clone());
        }
    }
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
        .filter(|typ| !typ.is_trait && !typ.binding_excluded && !mod_exclude_types.contains(&typ.name))
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

    // Register binding-side wrapper structs for opaque_types that have no capsule override.
    // These are emitted as #[pyclass(unsendable)] structs in mod.rs and must be registered
    // here so Python callers can inspect their type (e.g. `isinstance(lang, Language)`).
    for name in config.opaque_types.keys() {
        if !python_capsule_types.contains(name) && registered.insert(name.clone()) {
            lines.push(format!("    m.add_class::<{}>()?;", name));
        }
    }

    // Register trait marker classes — empty structs that represent plugin trait interfaces.
    // The Rust struct is named `Py<Trait>Marker` to avoid shadowing the trait import; the PyO3
    // name (declared via `#[pyclass(name = "<Trait>")]` in mod.rs) still matches the trait so
    // Python imports from the generated native module resolve to the marker class.
    for bridge_cfg in &config.trait_bridges {
        let trait_name = &bridge_cfg.trait_name;
        if registered.insert(trait_name.clone()) {
            lines.push(format!("    m.add_class::<Py{trait_name}Marker>()?;"));
        }
    }

    for func in &api.functions {
        if mod_exclude_functions.contains(&func.name) {
            continue;
        }
        lines.push(format!("    m.add_function(wrap_pyfunction!({}, m)?)?;", func.name));
    }

    // Register trait bridge registration functions
    for register_fn in crate::backends::pyo3::trait_bridge::collect_bridge_register_fns(&config.trait_bridges) {
        lines.push(format!("    m.add_function(wrap_pyfunction!({register_fn}, m)?)?;"));
    }
    // Register trait bridge unregister functions. The emitted Rust symbol is
    // `_alef_<unregister_fn>` (see trait_bridge/unregistration_fn.jinja) but pyo3 exposes
    // it under the bare `unregister_*` name via `#[pyo3(name = ...)]`. Without this
    // `m.add_function` call the symbol is not part of the native module and Python
    // callers see `AttributeError: module ... has no attribute 'unregister_*'`.
    for unregister_fn in crate::backends::pyo3::trait_bridge::collect_bridge_unregister_fns(&config.trait_bridges) {
        lines.push(format!(
            "    m.add_function(wrap_pyfunction!(_alef_{unregister_fn}, m)?)?;"
        ));
    }
    // Register trait bridge clear functions. Same `_alef_<clear_fn>` symbol convention
    // as unregister — must be added to the module so Python callers can access them.
    for clear_fn in crate::backends::pyo3::trait_bridge::collect_bridge_clear_fns(&config.trait_bridges) {
        lines.push(format!("    m.add_function(wrap_pyfunction!(_alef_{clear_fn}, m)?)?;"));
    }

    // Register error exception types and companion info classes/functions.
    // Errors with introspection methods also get a companion `{Name}Info` pyclass
    // and a free `{snake_name}_info` pyfunction to build it from an exception.
    let mut seen_registrations = AHashSet::new();
    for error in &api.errors {
        for reg_line in crate::codegen::error_gen::gen_pyo3_error_registration(error, &mut seen_registrations) {
            lines.push(reg_line);
        }
        if crate::codegen::error_gen::pyo3_error_has_methods(error) {
            let info_struct = crate::codegen::error_gen::pyo3_error_info_struct_name(error);
            let info_fn = crate::codegen::error_gen::pyo3_error_info_fn_name(error);
            lines.push(format!("    m.add_class::<{info_struct}>()?;"));
            lines.push(format!("    m.add_function(wrap_pyfunction!({info_fn}, m)?)?;"));
        }
    }

    lines.push("    Ok(())".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{gen_async_runtime_init, gen_module_init};
    use crate::core::ir::{ApiSurface, FunctionDef};

    fn make_config() -> crate::core::config::ResolvedCrateConfig {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
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
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let config = make_config();
        let result = gen_module_init("_test_lib", &api, &config);
        assert!(result.contains("#[pymodule]"));
        assert!(result.contains("Ok(())"));
        // Sync-only API: no async runtime, so no worker-stack provisioning.
        assert!(!result.contains("thread_stack_size"));
    }

    /// When the API has async functions, the module init provisions an enlarged worker-thread
    /// stack on the pyo3-async-runtimes runtime before the first `future_into_py`. Without it,
    /// a deep async pipeline (e.g. OCR) overflows the default ~2 MB worker stack and the process
    /// aborts with SIGBUS.
    #[test]
    fn gen_module_init_provisions_async_runtime_worker_stack() {
        let api = ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "do_async".to_string(),
                is_async: true,
                ..Default::default()
            }],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let config = make_config();
        let result = gen_module_init("_test_lib", &api, &config);
        assert!(
            result.contains("thread_stack_size"),
            "async module init must enlarge the worker-thread stack:\n{result}"
        );
        assert!(
            result.contains("pyo3_async_runtimes::tokio::init"),
            "the enlarged-stack runtime must be installed before first use:\n{result}"
        );
    }
}
