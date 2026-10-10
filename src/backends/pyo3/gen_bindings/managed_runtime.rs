use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;

pub(super) fn enabled(config: &ResolvedCrateConfig) -> bool {
    config
        .python
        .as_ref()
        .and_then(|python| python.async_runtime.as_deref())
        == Some("managed")
}

pub(super) fn has_async(api: &ApiSurface) -> bool {
    api.functions
        .iter()
        .any(|function| function.is_async && !function.sanitized && !function.binding_excluded)
        || api.types.iter().any(|ty| {
            !ty.binding_excluded
                && ty
                    .methods
                    .iter()
                    .any(|method| method.is_async && !method.sanitized && !method.binding_excluded)
        })
        || api
            .services
            .iter()
            .any(|service| service.entrypoints.iter().any(|entry| entry.is_async))
}

pub(super) fn rewrite(content: String) -> String {
    content
        .replace("pyo3_async_runtimes::tokio::", "crate::alef_async_runtime::")
        .replace(
            "crate::alef_async_runtime::get_runtime()",
            "crate::alef_async_runtime::get_runtime_checked()?",
        )
        .replace(
            "tokio::task::spawn_blocking(",
            "crate::alef_async_runtime::spawn_blocking(",
        )
}

pub(super) fn support() -> String {
    crate::backends::pyo3::template_env::render("managed_runtime.rs.jinja", minijinja::context! {})
}
