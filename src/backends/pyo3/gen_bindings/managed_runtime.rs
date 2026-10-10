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
    api.functions.iter().any(|function| function.is_async)
        || api
            .types
            .iter()
            .any(|ty| ty.methods.iter().any(|method| method.is_async))
}

pub(super) fn rewrite(content: String) -> String {
    content
        .replace("pyo3_async_runtimes::tokio::", "crate::alef_async_runtime::")
        .replace(
            "tokio::task::spawn_blocking(",
            "crate::alef_async_runtime::get_runtime().spawn_blocking(",
        )
}

pub(super) fn support() -> String {
    crate::backends::pyo3::template_env::render("managed_runtime.rs.jinja", crate::alef_context! {})
}
