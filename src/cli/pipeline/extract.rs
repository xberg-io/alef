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
    // Ensure .gitignore has required entries
    if let Some(parent) = config_path.parent() {
        ensure_gitignore(parent, config);
    }

    cache::validate_cache_crate_name(&config.name).context("invalid crate name for cache")?;
    let source_hash = cache::sources_hash(&config.sources).context("failed to compute sources hash")?;
    // Mix the resolved workspace version into the cache key. The IR embeds
    // `api.version`, which is read fresh from `version_from` (Cargo.toml) at
    // extract time. Sources alone don't change when the version is bumped, so
    // without this the cache would hand back stale IR and downstream stages
    // (notably READMEs) would render the previous version's badges/snippets.
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

    // Apply global filters (includes and excludes)
    api = apply_filters(api, config);

    // Inject declared opaque types from config (external crate types alef can't extract)
    inject_declared_opaque_types(&mut api, config);

    // Remove cfg-gated fields unless their feature is in [crate].features.
    // Binding crates may have different features enabled than the core crate,
    // so cfg-gated fields are only included when explicitly listed.
    strip_cfg_fields(&mut api, &config.features);

    // Remove source-declared internal/runtime items (types, enums, errors,
    // functions, methods) and fields from the polyglot binding surface before
    // unknown-type sanitization can collapse them into fake String fields.
    strip_binding_excluded(&mut api)?;

    // Replace references to types not in the API surface with String
    sanitize_unknown_types(&mut api);

    // Apply path mappings to rewrite rust_path fields before dedup so that
    // two types that had different raw paths but map to the same rewritten
    // path are correctly collapsed into one.
    apply_path_mappings(&mut api, config);

    // Deduplicate types, enums, and functions by name (after path mapping so
    // rewritten paths are used for the shortest-path preference heuristic).
    dedup_api_surface(&mut api);

    // Normalize every field's `type_rust_path` to the canonical `rust_path` of the
    // same-named type/enum. After dedup there is exactly one type per short name, so a
    // field that references it must use the same crate-rooted path. Otherwise a field
    // path like `crate::sub::Foo` (root `crate`) can disagree with the resolved type's
    // path `crate_inner::Foo` (root `crate_inner`) — e.g. when a facade re-exports some
    // types but not others — and `field_has_path_mismatch` would wrongly drop the
    // owning type from conversion generation.
    normalize_field_type_paths(&mut api);

    // Run the service extraction pass last so all dedup / sanitization is
    // complete before we classify methods and build service/handler-contract
    // IR nodes. Configured services are public generation inputs, so failures
    // must stop extraction instead of leaving lossy generic fallback bindings.
    run_service_extraction(&mut api, config)?;

    // Let registered extensions augment the API surface (e.g. inject HTTP-domain IR).
    // Each extension receives its own `[extensions.<name>]` section from alef.toml
    // so it can read per-project configuration without coupling to the core config schema.
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

    // Methods declared as [[crates.adapters]].core_path are emitted via adapter
    // codegen which handles lossy types (BoxStream, BoxFuture). Mark them as
    // binding_excluded so the public-API validator skips them, but keep them in
    // the IR so adapter codegen can still look up parameter info.
    mark_adapter_handled_methods(&mut api, config);

    // Apply `[crates.exclude].methods = ["Owner.method"]` AFTER `extract_services` for
    // `api.types[Owner].methods`. `apply_filters` (above) already stripped excluded methods
    // from `api.types[*].methods`, but `extract_services` calls `recover_service_methods`
    // which RE-INJECTS configured methods back into the owner type so per-binding service
    // codegen can see them. Re-applying the exclude here is the defense-in-depth pass that
    // strips those re-injected methods from the regular method-emission path, preventing
    // backends from emitting a `compile_error!` non-delegatable stub.
    //
    // `service.configurators` are intentionally NOT subject to this strip: a method named in
    // `[[crates.services]].configurators` is an explicit declaration that the service IR must
    // contain the configurator entrypoint. The `[crates.exclude].methods` list controls only
    // the generic per-type method-emission path (struct codegen), not the service IR. If both
    // lists name the same method it means the entry in `exclude.methods` suppresses the
    // generic struct-level emission while the configurator entry drives the dedicated
    // C/host-language service entrypoint — both intents are honoured independently.
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
