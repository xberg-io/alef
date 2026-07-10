//! Service-API file orchestration.

use crate::backends::rustler::gen_bindings::service_api::elixir::gen_service_ex;
use crate::backends::rustler::gen_bindings::service_api::rust::gen_service_rs;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use std::path::PathBuf;

/// Generate all service-API files for the rustler backend.
///
/// Returns up to two `GeneratedFile`s per non-empty service list:
/// - `{output_dir}/service.rs`   — Rust rustler glue
/// - `{elixir_pkg}/service.ex`   — idiomatic Elixir module
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(
        config.output_paths.get("elixir"),
        &config.name,
        "packages/elixir/native/{name}_nif/src/",
    );

    let service_rs = gen_service_rs(api, config);

    let (_, module_prefix) = crate::backends::rustler::gen_bindings::helpers::get_module_info(api, config);
    let service_ex = gen_service_ex(api, &module_prefix);

    let elixir_pkg = config.output_paths.get("elixir").map(PathBuf::from).unwrap_or_else(|| {
        let app_name = config.elixir_app_name();
        PathBuf::from(format!("packages/elixir/lib/{}", app_name))
    });

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: elixir_pkg.join("service.ex"),
            content: service_ex,
            generated_header: true,
        },
    ])
}
