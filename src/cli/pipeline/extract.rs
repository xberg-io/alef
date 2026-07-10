mod external_types;
mod filtering;
mod gitignore;
mod raw;
mod sanitizer;
mod services;
#[cfg(test)]
mod tests;
mod type_helpers;
mod validation;

use crate::cli::cache;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use anyhow::Context as _;
use std::path::Path;
use tracing::info;

use external_types::merge_external_type_roots;
use filtering::apply_filters;
pub use gitignore::ensure_gitignore;
use raw::extract_raw;
use sanitizer::{sanitize_unknown_types, strip_binding_excluded};
use services::{mark_adapter_handled_methods, run_service_extraction, strip_excluded_methods_from_types};
use type_helpers::{
    apply_path_mappings, dedup_api_surface, inject_declared_opaque_types, normalize_field_type_paths, strip_cfg_fields,
};
use validation::validate_extracted_api;

const IR_CACHE_SCHEMA_VERSION: &str = "ir-cache-v2";

pub fn extract(config: &ResolvedCrateConfig, config_path: &Path, clean: bool) -> anyhow::Result<ApiSurface> {
    if let Some(parent) = config_path.parent() {
        ensure_gitignore(parent, config);
    }

    cache::validate_cache_crate_name(&config.name).context("invalid crate name for cache")?;
    let source_hash = cache::sources_hash(&config.source_hash_paths()).context("failed to compute sources hash")?;
    let version_for_hash = config.resolved_version().unwrap_or_default();
    let config_hash = extraction_config_hash(config, config_path)?;
    let cache_key = format!("{IR_CACHE_SCHEMA_VERSION}:{source_hash}:{version_for_hash}:{config_hash}");

    if !clean && cache::is_ir_cached(&config.name, &cache_key) {
        info!("Using cached IR");
        let api = cache::read_cached_ir(&config.name).context("failed to read cached IR")?;
        validate_extracted_api(&api, config)?;
        return Ok(api);
    }

    let mut api = extract_raw(config, config_path)?;

    merge_external_type_roots(&mut api, config)?;

    api = apply_filters(api, config);

    inject_declared_opaque_types(&mut api, config);

    strip_cfg_fields(&mut api, &config.features);

    strip_binding_excluded(&mut api)?;

    sanitize_unknown_types(&mut api);

    apply_path_mappings(&mut api, config);

    dedup_api_surface(&mut api);

    normalize_field_type_paths(&mut api);

    run_service_extraction(&mut api, config)?;

    crate::with_extensions(|exts| {
        for ext in exts {
            let raw = crate::core::extension::read_extension_config(config_path, ext.name())
                .with_context(|| format!("extension `{}`: failed to read config from alef.toml", ext.name()))?;
            let cfg = ext
                .parse_config(raw.as_ref())
                .with_context(|| format!("extension `{}`: failed to parse config", ext.name()))?;
            ext.augment_surface(&mut api, &cfg)
                .with_context(|| format!("extension `{}`: augment_surface failed", ext.name()))?;
        }
        Ok::<(), anyhow::Error>(())
    })?;

    mark_adapter_handled_methods(&mut api, config);

    strip_excluded_methods_from_types(&mut api, config);

    validate_extracted_api(&api, config)?;

    cache::write_ir_cache(&config.name, &api, &cache_key).context("failed to write IR cache")?;
    info!(
        "Extracted {} types, {} functions, {} enums",
        api.types.len(),
        api.functions.len(),
        api.enums.len()
    );

    Ok(api)
}
fn extraction_config_hash(config: &ResolvedCrateConfig, config_path: &Path) -> anyhow::Result<String> {
    let config_toml = toml::to_string(config).context("failed to serialize resolved config for IR cache key")?;
    let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
    let mut hasher = blake3::Hasher::new();
    hasher.update(config_toml.as_bytes());
    hasher.update(&alef_toml_bytes);
    Ok(hasher.finalize().to_hex().to_string())
}
