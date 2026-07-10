//! Service-API codegen for the PHP (ext-php-rs) backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **`service.rs`** — Rust ext-php-rs glue that wraps each registered PHP
//!    callable as `Arc<dyn <HandlerContractDef::trait_name>>` via a blocking
//!    callback bridge (PHP is single-threaded per request), builds the core
//!    service via the owner type's registration and run entrypoints, and exposes
//!    a `#[php_function]` entry point.
//!
//! 2. **`service.php`** — An idiomatic PHP class mirroring the service's
//!    constructor, configurator methods, and registration methods, with a
//!    `run(...)` method that delegates to the native extension.
//!
//! All names are derived entirely from the [`ApiSurface`] IR — no transport-
//! or domain-specific assumptions are made anywhere in this module.

mod helpers;
mod php;
mod rust;
mod type_mapping;

#[cfg(test)]
mod tests;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use std::path::PathBuf;

pub(super) use php::gen_service_php;
pub(super) use rust::gen_service_rs;

/// Generate all service-API files for the PHP backend.
///
/// Returns up to two `GeneratedFile`s per non-empty service list:
/// - `{output_dir}/service.rs`   — Rust ext-php-rs glue
/// - `{php_pkg}/Service.php`     — idiomatic PHP class
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(config.output_paths.get("php"), &config.name, "crates/{name}-php/src/");

    let service_rs = gen_service_rs(api, config);

    let extension_name = config.name.replace('-', "_");
    let service_php = gen_service_php(api, &extension_name);

    let output_base = config
        .php
        .as_ref()
        .and_then(|p| p.stubs.as_ref())
        .map(|s| PathBuf::from(&s.output))
        .unwrap_or_else(|| {
            let package_name = config.name.replace('-', "_");
            PathBuf::from(format!("packages/php/{}", package_name))
        });

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: output_base.join("Service.php"),
            content: service_php,
            generated_header: true,
        },
    ])
}
