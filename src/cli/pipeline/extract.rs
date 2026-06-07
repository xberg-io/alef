use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use anyhow::Context as _;
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info};

use crate::cli::cache;

use super::version_core::read_version;

const IR_CACHE_SCHEMA_VERSION: &str = "ir-cache-v2";

/// Ensure required entries are in `.gitignore` — creates the file if absent.
/// Adds `.alef/` (cache) and language-specific build artifacts based on config.
pub fn ensure_gitignore(base_dir: &Path, config: &ResolvedCrateConfig) {
    use crate::core::config::Language;

    let gitignore_path = base_dir.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
    let existing_lines: AHashSet<&str> = existing.lines().map(str::trim).collect();

    let mut entries: Vec<&str> = vec![".alef/"];

    for lang in &config.languages {
        match lang {
            Language::Python => {
                entries.extend_from_slice(&["__pycache__/", "*.so", "*.pyd", ".venv/", "*.egg-info/", "dist/"])
            }
            Language::Node => entries.extend_from_slice(&["node_modules/", "*.node"]),
            Language::Ruby => entries.extend_from_slice(&[".gems/", "vendor/bundle/"]),
            Language::Php => entries.extend_from_slice(&["vendor/"]),
            Language::Ffi => entries.push("*.h.bak"),
            Language::Go => entries.push("*.test"),
            Language::Java => entries.extend_from_slice(&["target/", "*.class"]),
            Language::Csharp => entries.extend_from_slice(&["bin/", "obj/", "*.nupkg"]),
            // pkg/ intentionally NOT gitignored — npm publish needs it for WASM artifacts
            Language::Wasm => {}
            _ => {}
        }
    }

    let mut to_add = Vec::new();
    for entry in &entries {
        if !existing_lines.contains(entry) {
            to_add.push(*entry);
        }
    }

    if to_add.is_empty() {
        return;
    }

    let separator = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let additions = to_add.join("\n");
    let new_content = format!("{existing}{separator}{additions}\n");

    if let Err(e) = std::fs::write(&gitignore_path, new_content) {
        debug!("Could not update .gitignore: {e}");
    } else {
        debug!("Updated .gitignore with {} entries", to_add.len());
    }
}

/// Run extraction, with caching.
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
    let config_hash = extraction_config_hash(config)?;
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
    let service_errors = crate::extract::extractor::service::extract_services(&mut api, config);
    if !service_errors.is_empty() {
        let formatted = service_errors
            .iter()
            .map(|message| format!("- {message}"))
            .collect::<Vec<_>>()
            .join("\n");
        anyhow::bail!("service extraction failed:\n{formatted}");
    }

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
    if !config.exclude.methods.is_empty() {
        for typ in &mut api.types {
            typ.methods.retain(|m| {
                let key = format!("{}.{}", typ.name, m.name);
                !config.exclude.methods.contains(&key)
            });
        }
    }

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

fn extraction_config_hash(config: &ResolvedCrateConfig) -> anyhow::Result<String> {
    let config_toml = toml::to_string(config).context("failed to serialize resolved config for IR cache key")?;
    Ok(blake3::hash(config_toml.as_bytes()).to_hex().to_string())
}

fn validate_extracted_api(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<()> {
    let bridged_trait_names: AHashSet<&str> = config
        .trait_bridges
        .iter()
        .map(|bridge| bridge.trait_name.as_str())
        .collect();
    let validation_report =
        crate::core::validation::validate_api_surface_with_bridged_traits(api, &bridged_trait_names);
    for diagnostic in validation_report.warnings() {
        tracing::warn!("{diagnostic}");
    }
    let (suppressed, fatal): (Vec<_>, Vec<_>) = validation_report.errors().partition(|d| {
        !crate::core::validation::is_critical_unsuppressible(d.code)
            && config
                .suppress_validation_codes
                .iter()
                .any(|code| code == &d.code.to_string())
    });
    for diagnostic in suppressed {
        tracing::warn!("[suppressed] {diagnostic}");
    }
    if !fatal.is_empty() {
        let formatted = fatal
            .iter()
            .map(|d| {
                let path = d
                    .item_path
                    .as_deref()
                    .map(|p| format!(" item `{p}`"))
                    .unwrap_or_default();
                format!("- [{}]{path} {}", d.code, d.reason)
            })
            .collect::<Vec<_>>()
            .join("\n");
        anyhow::bail!("{}", formatted);
    }
    Ok(())
}

/// Shared raw extraction logic: parse sources, produce raw `ApiSurface`.
///
/// Groups source files by their owning crate (derived from `crates/{name}/src/` path
/// patterns) and extracts each group with the correct crate name. This ensures types
/// get accurate `rust_path` values reflecting their actual defining crate, not the
/// facade crate name from config.
fn extract_raw(config: &ResolvedCrateConfig, _config_path: &Path) -> anyhow::Result<ApiSurface> {
    info!("Extracting API surface from Rust source...");
    let version = read_version(&config.version_from)?;
    let workspace_root = config.workspace_root.as_deref();
    let default_name = &config.name;

    // Build source groups: use explicit source_crates config when available,
    // otherwise derive crate names from file paths in the flat sources list.
    let mut groups: std::collections::BTreeMap<String, Vec<&Path>> = std::collections::BTreeMap::new();
    if !config.source_crates.is_empty() {
        for sc in &config.source_crates {
            let crate_name = sc.name.replace('-', "_");
            for source in &sc.sources {
                groups.entry(crate_name.clone()).or_default().push(source.as_path());
            }
        }
    } else {
        for source in &config.sources {
            let crate_name = derive_crate_name_from_path(source, default_name);
            groups.entry(crate_name).or_default().push(source.as_path());
        }
    }

    // Extract each group with its own crate name, then merge
    let mut merged = ApiSurface {
        crate_name: default_name.to_string(),
        version: version.clone(),
        ..ApiSurface::default()
    };

    for (crate_name, sources) in &groups {
        let api = crate::extract::extractor::extract(sources, crate_name, &version, workspace_root)
            .with_context(|| format!("failed to extract API surface from crate {crate_name}"))?;
        merged.types.extend(api.types);
        merged.functions.extend(api.functions);
        merged.enums.extend(api.enums);
        merged.errors.extend(api.errors);
        merged.excluded_type_paths.extend(api.excluded_type_paths);
        merged.excluded_trait_names.extend(api.excluded_trait_names);
        merged.unsupported_public_items.extend(api.unsupported_public_items);
    }

    // Re-run the return-type marking against the merged surface so that a
    // function in crate A that returns a type whose canonical home is crate B
    // (a common pattern when the public facade `pub use`s items from internal
    // crates) still gets its TypeDef.is_return_type flagged. The per-crate
    // extractor only marks types that share its own surface, so cross-crate
    // function→type pairs would otherwise stay false here.
    let return_type_names: ahash::AHashSet<String> = merged
        .functions
        .iter()
        .filter_map(|f| match &f.return_type {
            crate::core::ir::TypeRef::Named(name) => Some(name.clone()),
            _ => None,
        })
        .collect();
    for typ in &mut merged.types {
        if return_type_names.contains(&typ.name) {
            typ.is_return_type = true;
        }
    }

    Ok(merged)
}

/// Derive the crate name from a source file path.
///
/// Matches `crates/{name}/src/` pattern and converts hyphens to underscores.
/// Falls back to the provided default name if the pattern doesn't match.
fn derive_crate_name_from_path(path: &Path, default: &str) -> String {
    let path_str = path.to_string_lossy();
    // Match both "crates/foo-bar/src/" and "/abs/path/crates/foo-bar/src/"
    if let Some(after_crates) = path_str.split("crates/").nth(1) {
        if let Some(name) = after_crates.split('/').next() {
            if path_str.contains(&format!("crates/{name}/src/")) {
                return name.replace('-', "_");
            }
        }
    }
    default.to_string()
}

/// Inject declared opaque types from config into the API surface.
/// These are external crate types that alef can't extract but needs to generate wrappers for.
fn inject_declared_opaque_types(api: &mut ApiSurface, config: &ResolvedCrateConfig) {
    let mut sorted_opaques: Vec<_> = config.opaque_types.iter().collect();
    sorted_opaques.sort_by_key(|(name, _)| (*name).clone());
    for (name, rust_path) in sorted_opaques {
        // Only add if not already in the API surface
        if !api.types.iter().any(|t| t.name == *name) && !api.enums.iter().any(|e| e.name == *name) {
            api.types.push(crate::core::ir::TypeDef {
                name: name.clone(),
                rust_path: rust_path.clone(),
                original_rust_path: rust_path.clone(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                doc: String::new(),
                cfg: None,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
            });
            debug!("Injected declared opaque type: {name} -> {rust_path}");
        }
    }
}

/// Replace `TypeRef::Named(name)` references that don't exist in the API surface
/// with `TypeRef::String`. This handles trait objects, generic bounds, and other types
/// that were extracted but filtered out or never existed as concrete types.
fn sanitize_unknown_types(api: &mut ApiSurface) {
    let known_types: AHashSet<String> = api.types.iter().map(|t| t.name.clone()).collect();
    let known_enums: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    // Build a set of known rust_paths for types and enums.
    // This enables disambiguation of types with the same short name but different
    // module paths (e.g., `sample_core::types::OutputFormat` vs `sample_core::OutputFormat`).
    // Normalize hyphens to underscores in paths for consistent comparison.
    let known_type_paths: AHashSet<String> = api.types.iter().map(|t| t.rust_path.replace('-', "_")).collect();
    let known_enum_paths: AHashSet<String> = api.enums.iter().map(|e| e.rust_path.replace('-', "_")).collect();

    for typ in &mut api.types {
        for field in &mut typ.fields {
            let original = extract_tuple_vec_original_type(&field.ty);
            if sanitize_type_ref(&mut field.ty, &known_types, &known_enums).is_lossy() {
                field.sanitized = true;
                if let Some(orig) = original {
                    field.original_type = Some(orig);
                }
            }
            // Second pass: check type_rust_path for name-collision disambiguation.
            // If a field has a type_rust_path that doesn't match any known type/enum rust_path,
            // it references a different type that happens to share the same short name
            // (e.g., crate::types::OutputFormat vs crate::core::config::OutputFormat).
            if !field.sanitized {
                if let Some(ref path) = field.type_rust_path {
                    let normalized_path = path.replace('-', "_");
                    if let TypeRef::Named(ref name) = field.ty {
                        // Only check if the name matches a known type/enum — otherwise it's
                        // already handled by the standard sanitization above.
                        if known_types.contains(name.as_str()) || known_enums.contains(name.as_str()) {
                            // Check if the full path's last segment matches any known type/enum path's last segment.
                            // This handles cases where module paths differ but the type is the same
                            // (e.g., crate::metadata::HtmlMetadata vs sample-markdown-rs::HtmlMetadata).
                            let path_type_name = normalized_path.rsplit("::").next().unwrap_or("");
                            let path_matches = known_type_paths
                                .iter()
                                .chain(known_enum_paths.iter())
                                .any(|kp| kp.rsplit("::").next().unwrap_or("") == path_type_name);
                            if !path_matches {
                                field.ty = TypeRef::String;
                                field.sanitized = true;
                            }
                        }
                    }
                    // Also check Named types inside Optional/Vec wrappers
                    if let TypeRef::Vec(ref inner) = field.ty {
                        if let TypeRef::Named(ref name) = **inner {
                            let vec_path_type = normalized_path.rsplit("::").next().unwrap_or("");
                            let vec_matches = known_type_paths
                                .iter()
                                .chain(known_enum_paths.iter())
                                .any(|kp| kp.rsplit("::").next().unwrap_or("") == vec_path_type);
                            if (known_types.contains(name.as_str()) || known_enums.contains(name.as_str()))
                                && !vec_matches
                            {
                                field.ty = TypeRef::String;
                                field.sanitized = true;
                            }
                        }
                    }
                }
            }
        }
        let type_name = typ.name.clone();
        let is_trait = typ.is_trait;
        for method in &mut typ.methods {
            // Trait method params and return types must match the original Rust trait
            // signature exactly — bridge codegen emits `impl Trait for Wrapper { fn ... }`
            // and the impl must satisfy the trait. Sanitizing these would cause
            // E0053 (incompatible type) trait coherence errors. Internal-only param
            // types are handled by per-backend JSON serialization in the bridge body.
            if is_trait {
                continue;
            }
            let mut method_sanitized = false;
            for param in &mut method.params {
                if sanitize_type_ref(&mut param.ty, &known_types, &known_enums).is_lossy() {
                    param.sanitized = true;
                    method_sanitized = true;
                }
            }
            // Skip sanitizing return type if it's Named(parent_type) — builder/factory pattern.
            // Methods that return their own type (e.g. with_foo(&self) -> Self) should keep
            // the Named return so codegen can delegate them correctly.
            let is_self_return = matches!(&method.return_type, TypeRef::Named(n) if n == &type_name);
            if !is_self_return && sanitize_type_ref(&mut method.return_type, &known_types, &known_enums).is_lossy() {
                method_sanitized = true;
            }
            if method_sanitized {
                method.sanitized = true;
            }
        }
    }
    for func in &mut api.functions {
        let mut func_sanitized = false;
        for param in &mut func.params {
            if sanitize_type_ref(&mut param.ty, &known_types, &known_enums).is_lossy() {
                param.sanitized = true;
                func_sanitized = true;
            }
        }
        if sanitize_type_ref(&mut func.return_type, &known_types, &known_enums).is_lossy() {
            func_sanitized = true;
            func.return_sanitized = true;
        }
        if func_sanitized {
            func.sanitized = true;
        }
    }
    // Sanitize enum variant fields — tuples and other unknown types in data enum
    // variants must be replaced with String, otherwise backends emit invalid code
    // (e.g., Go emitting `[](String, String)` for Vec<(String, String)>).
    for enum_def in &mut api.enums {
        for variant in &mut enum_def.variants {
            for field in &mut variant.fields {
                let original = extract_tuple_vec_original_type(&field.ty);
                if sanitize_type_ref(&mut field.ty, &known_types, &known_enums).is_lossy() {
                    field.sanitized = true;
                    if let Some(orig) = original {
                        field.original_type = Some(orig);
                    }
                }
            }
        }
    }
    // Sanitize error variant fields as well.
    for error_def in &mut api.errors {
        for variant in &mut error_def.variants {
            for field in &mut variant.fields {
                let original = extract_tuple_vec_original_type(&field.ty);
                if sanitize_type_ref(&mut field.ty, &known_types, &known_enums).is_lossy() {
                    field.sanitized = true;
                    if let Some(orig) = original {
                        field.original_type = Some(orig);
                    }
                }
            }
        }
    }
}

fn strip_binding_excluded(api: &mut ApiSurface) -> anyhow::Result<()> {
    // --- Item-level exclusions: types, enums, errors, functions ---

    // Capture rust_paths of excluded types/enums/errors before removal so that
    // trait-bridge codegen can still reference them by qualified path.
    for typ in &api.types {
        if typ.binding_excluded {
            let reason = typ
                .binding_exclusion_reason
                .as_deref()
                .unwrap_or("source binding exclusion");
            info!("Stripping excluded type: {} ({})", typ.name, reason);
            api.excluded_type_paths
                .insert(typ.name.clone(), typ.rust_path.replace('-', "_"));
            // Preserve trait-ness across the strip so trait-bridge codegen can tell
            // an excluded trait (`&dyn Trait` → non-bridgeable, skip the method) from
            // an excluded struct/enum (`&HiddenDocument` → reference by qualified path).
            if typ.is_trait {
                api.excluded_trait_names.insert(typ.name.clone());
            }
        }
    }
    for enm in &api.enums {
        if enm.binding_excluded {
            let reason = enm
                .binding_exclusion_reason
                .as_deref()
                .unwrap_or("source binding exclusion");
            info!("Stripping excluded enum: {} ({})", enm.name, reason);
            api.excluded_type_paths
                .insert(enm.name.clone(), enm.rust_path.replace('-', "_"));
        }
    }
    for err in &api.errors {
        if err.binding_excluded {
            let reason = err
                .binding_exclusion_reason
                .as_deref()
                .unwrap_or("source binding exclusion");
            info!("Stripping excluded error: {} ({})", err.name, reason);
            api.excluded_type_paths
                .insert(err.name.clone(), err.rust_path.replace('-', "_"));
        }
    }

    api.types.retain(|t| !t.binding_excluded);
    api.enums.retain(|e| !e.binding_excluded);
    api.errors.retain(|e| !e.binding_excluded);

    for func in &api.functions {
        if func.binding_excluded {
            let reason = func
                .binding_exclusion_reason
                .as_deref()
                .unwrap_or("source binding exclusion");
            info!("Stripping excluded function: {} ({})", func.name, reason);
        }
    }
    api.functions.retain(|f| !f.binding_excluded);

    // --- Method-level exclusions on retained types ---
    for typ in &mut api.types {
        let excluded_methods: Vec<String> = typ
            .methods
            .iter()
            .filter(|m| m.binding_excluded)
            .map(|m| {
                let reason = m
                    .binding_exclusion_reason
                    .as_deref()
                    .unwrap_or("source binding exclusion");
                format!("{}.{} ({})", typ.name, m.name, reason)
            })
            .collect();
        if !excluded_methods.is_empty() {
            info!("Stripping excluded methods: {}", excluded_methods.join(", "));
        }
        typ.methods.retain(|m| !m.binding_excluded);
    }

    // --- Field-level exclusions ---
    // Keep excluded fields in IR so conversion generators can still initialize the
    // core field (usually with Default::default()) while public binding DTOs hide it.
    for typ in &api.types {
        let excluded: Vec<_> = typ
            .fields
            .iter()
            .filter(|field| field.binding_excluded)
            .map(|field| {
                let reason = field
                    .binding_exclusion_reason
                    .as_deref()
                    .unwrap_or("source binding exclusion");
                format!("{}.{} ({reason})", typ.name, field.name)
            })
            .collect();
        if !excluded.is_empty() {
            info!("Hiding binding-excluded fields: {}", excluded.join(", "));
        }
    }

    // Enum variant binding_excluded fields are RETAINED in the IR (like struct fields) so
    // that "to core" conversion codegen can initialize them with Default::default().
    // The `originally_had_data_fields` flag is set when all fields are binding_excluded so
    // that codegen can emit wildcard patterns on the core-type side. Mirror emitters skip
    // binding_excluded fields when building the public binding surface.
    for enum_def in &mut api.enums {
        let excluded: Vec<String> = enum_def
            .variants
            .iter()
            .flat_map(|variant| {
                variant.fields.iter().filter(|f| f.binding_excluded).map(|f| {
                    let reason = f
                        .binding_exclusion_reason
                        .as_deref()
                        .unwrap_or("source binding exclusion");
                    format!("{}::{}.{} ({reason})", enum_def.name, variant.name, f.name)
                })
            })
            .collect();
        if !excluded.is_empty() {
            info!("Hiding binding-excluded enum variant fields: {}", excluded.join(", "));
        }
        for variant in &mut enum_def.variants {
            // Set flag when ALL fields are binding_excluded so codegen knows the core type
            // still has data fields even though the mirror shows a unit variant.
            if !variant.fields.is_empty() && variant.fields.iter().all(|f| f.binding_excluded) {
                variant.originally_had_data_fields = true;
            }
            // Do NOT strip — retain fields so to-core conversion codegen can use them.
        }
    }

    // Error variants: same retention policy for binding_excluded fields.
    for error_def in &mut api.errors {
        for variant in &mut error_def.variants {
            // Fields are retained; the is_tuple flag (set during extraction) lets codegen
            // distinguish tuple vs struct variants for wildcard/default-init patterns.
            let _ = variant; // retention is implicit — no retain() call
        }
    }

    Ok(())
}

/// Mark methods referenced by `[[crates.adapters]].core_path` as `binding_excluded`.
///
/// Adapter codegen emits the binding for these methods directly (handling lossy
/// types like `BoxStream` / `BoxFuture` per-language), so the public-API validator
/// should treat them as already-handled. The methods are *not* removed from the
/// IR — backends look up parameter info from `typ.methods` to generate adapter
/// wrappers. Backends that iterate `typ.methods` for normal method emission
/// already filter on `binding_excluded` (or skip via `streaming_method_keys` /
/// equivalent).
fn mark_adapter_handled_methods(api: &mut ApiSurface, config: &ResolvedCrateConfig) {
    use ahash::AHashSet;

    let adapter_handled: AHashSet<(String, String)> = config
        .adapters
        .iter()
        .filter_map(|adapter| {
            adapter
                .owner_type
                .as_deref()
                .map(|owner| (owner.to_string(), adapter.core_path.clone()))
        })
        .collect();

    if adapter_handled.is_empty() {
        return;
    }

    for typ in &mut api.types {
        for method in &mut typ.methods {
            if adapter_handled.contains(&(typ.name.clone(), method.name.clone())) && !method.binding_excluded {
                method.binding_excluded = true;
                if method.binding_exclusion_reason.is_none() {
                    method.binding_exclusion_reason =
                        Some(format!("handled by [[crates.adapters]] entry `{}`", method.name));
                }
            }
        }
    }
}

/// If `ty` is `Vec<(...)>` or `Option<Vec<(...)>>` — a Vec whose inner element is a tuple
/// type name — return a human-readable string capturing the original shape before sanitization
/// (e.g. `"Vec<(String, String)>"`).  Returns `None` for all other shapes.
///
/// This is called *before* `sanitize_type_ref` rewrites the inner `Named("(String, String)")`
/// to `String`, so backends can store this string in `FieldDef::original_type` and later emit
/// language-native pair types instead of a plain list.
fn extract_tuple_vec_original_type(ty: &TypeRef) -> Option<String> {
    fn inner_tuple_name(ty: &TypeRef) -> Option<String> {
        if let TypeRef::Vec(inner) = ty {
            if let TypeRef::Named(name) = inner.as_ref() {
                if name.trim_start().starts_with('(') {
                    return Some(format!("Vec<{name}>"));
                }
            }
        }
        None
    }
    /// Detect fixed-size tuple-array strings like `[(u32, u32); 4]`.
    ///
    /// The extractor emits these as `TypeRef::Named("[(u32, u32); 4]")` because there is no
    /// dedicated IR variant for fixed-size arrays.  We capture the string before sanitization
    /// so the wasm backend can reconstruct the type via `serde_wasm_bindgen::from_value`.
    fn fixed_tuple_array_name(name: &str) -> Option<String> {
        let s = name.trim();
        if s.starts_with("[(") && s.contains(");") {
            Some(s.to_string())
        } else {
            None
        }
    }
    match ty {
        TypeRef::Vec(_) => inner_tuple_name(ty),
        TypeRef::Optional(inner) => inner_tuple_name(inner),
        // Fixed-size tuple arrays arrive as Named("[(T, U); N]") from the extractor.
        TypeRef::Named(name) => fixed_tuple_array_name(name),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeSanitization {
    Unchanged,
    Lossless,
    Lossy,
}

impl TypeSanitization {
    fn is_lossy(self) -> bool {
        self == Self::Lossy
    }

    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Lossy, _) | (_, Self::Lossy) => Self::Lossy,
            (Self::Lossless, _) | (_, Self::Lossless) => Self::Lossless,
            (Self::Unchanged, Self::Unchanged) => Self::Unchanged,
        }
    }
}

/// Sanitize a type reference while preserving whether the change is lossy.
fn sanitize_type_ref(
    ty: &mut TypeRef,
    known_types: &AHashSet<String>,
    known_enums: &AHashSet<String>,
) -> TypeSanitization {
    match ty {
        TypeRef::Named(name) if !known_types.contains(name.as_str()) && !known_enums.contains(name.as_str()) => {
            // `Value` and `JsonValue` are bare names for serde_json::Value that the extractor
            // preserves as Named types. They are not unknown types to be collapsed to String,
            // but rather pseudo-types that should be preserved through Option/Vec/Map wrappers
            // so that type mappers can handle them appropriately. Do not sanitize.
            if name == "Value" || name == "JsonValue" {
                return TypeSanitization::Unchanged;
            }
            // Detect homogeneous numeric tuple types such as `(u32, u32)` that serde serializes
            // as JSON arrays.  Map them to Vec<ElemType> so backends emit array types (e.g.
            // `[]uint32` in Go) rather than falling back to `string`.  This preserves round-trip
            // JSON compatibility: `null | [800, 600]` unmarshals correctly into `*[]uint32`.
            if let Some(elem_ty) = parse_homogeneous_tuple(name) {
                *ty = TypeRef::Vec(Box::new(elem_ty));
                return TypeSanitization::Lossy; // Sanitized — the core type is a tuple, not a Vec
            }
            *ty = TypeRef::String;
            TypeSanitization::Lossy
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => sanitize_type_ref(inner, known_types, known_enums),
        TypeRef::Map(k, v) => {
            if contains_ambiguous_bare_value(k) || contains_ambiguous_bare_value(v) {
                return TypeSanitization::Lossy;
            }
            // Sanitize inner key and value types (e.g. Named("str") → String) so
            // backends receive clean Map(String, Json) rather than Map(Named("str"), Json).
            let key_status = sanitize_map_inner_type(k, known_types, known_enums);
            let value_status = sanitize_map_inner_type(v, known_types, known_enums);
            key_status.combine(value_status)
        }
        _ => TypeSanitization::Unchanged,
    }
}

fn sanitize_map_inner_type(
    ty: &mut TypeRef,
    known_types: &AHashSet<String>,
    known_enums: &AHashSet<String>,
) -> TypeSanitization {
    if matches!(ty, TypeRef::Named(name) if name == "str") {
        *ty = TypeRef::String;
        return TypeSanitization::Lossless;
    }
    sanitize_type_ref(ty, known_types, known_enums)
}

fn contains_ambiguous_bare_value(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Named(name) => name == "Value" || name == "JsonValue",
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => contains_ambiguous_bare_value(inner),
        TypeRef::Map(key, value) => contains_ambiguous_bare_value(key) || contains_ambiguous_bare_value(value),
        _ => false,
    }
}

/// Parse a homogeneous numeric tuple type string such as `"(u32,u32)"` or `"(u64, u64)"`.
///
/// Returns `Some(TypeRef)` for the element type when all comma-separated elements inside the
/// parentheses are the same primitive type.  Returns `None` for heterogeneous tuples, non-tuple
/// strings, or unsupported element types.
///
/// This lets `sanitize_type_ref` map `Option<(u32, u32)>` → `Optional(Vec(Primitive(U32)))`
/// instead of falling back to `String`, preserving JSON array round-trip compatibility.
fn parse_homogeneous_tuple(name: &str) -> Option<TypeRef> {
    use crate::core::ir::PrimitiveType;
    let name = name.trim();
    let inner = name.strip_prefix('(')?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.is_empty() {
        return None;
    }
    let first = parts[0];
    if !parts.iter().all(|p| *p == first) {
        return None;
    }
    // Homogeneous String tuples (e.g. `(String, String)`) serialize as JSON arrays of strings,
    // so map them to Vec<String> like numeric homogeneous tuples.
    if first == "String" {
        return Some(TypeRef::String);
    }
    let prim = match first {
        "u8" => PrimitiveType::U8,
        "u16" => PrimitiveType::U16,
        "u32" => PrimitiveType::U32,
        "u64" => PrimitiveType::U64,
        "i8" => PrimitiveType::I8,
        "i16" => PrimitiveType::I16,
        "i32" => PrimitiveType::I32,
        "i64" => PrimitiveType::I64,
        "f32" => PrimitiveType::F32,
        "f64" => PrimitiveType::F64,
        "usize" => PrimitiveType::Usize,
        "isize" => PrimitiveType::Isize,
        _ => return None,
    };
    Some(TypeRef::Primitive(prim))
}

/// Deduplicate API surface items by name to prevent conflicting definitions.
/// This resolves:
/// 1. Type-enum collisions: If a name exists in both types and enums, keep only the enum
/// 2. Remove fields with `#[cfg(...)]` conditions from all types.
///
/// Binding crates may have different feature sets than the core crate,
/// so including cfg-gated fields causes compilation errors.
fn strip_cfg_fields(api: &mut ApiSurface, enabled_features: &[String]) {
    for typ in &mut api.types {
        let original_count = typ.fields.len();
        let cfg_count = typ.fields.iter().filter(|f| f.cfg.is_some()).count();
        // Retain non-cfg fields and cfg fields whose feature condition is satisfied
        // by the source crate. Per-binding feature filtering happens later in codegen,
        // which evaluates `field.cfg` against each binding's effective feature set —
        // so we keep the cfg attribute on retained fields rather than clearing it.
        typ.fields.retain(|f| match &f.cfg {
            None => true,
            Some(cfg_str) => cfg_condition_enabled(cfg_str, enabled_features),
        });
        // Mark if any cfg fields were actually stripped (not enabled).
        if cfg_count > 0 && typ.fields.len() < original_count {
            typ.has_stripped_cfg_fields = true;
        }
    }
}

/// Evaluate a `#[cfg(...)]` condition string against a set of enabled features.
///
/// Handles:
/// - `feature = "name"` — single feature check
/// - `any(feature = "a", feature = "b", ...)` — any feature enabled
/// - `all(feature = "a", feature = "b", ...)` — all features enabled
///
/// Defaults to `false` (strip the field) for unrecognized patterns.
fn cfg_condition_enabled(cfg_str: &str, enabled_features: &[String]) -> bool {
    // Normalize: trim outer whitespace and collapse spaces adjacent to punctuation.
    // proc-macro2's `to_string()` inserts spaces between tokens, so
    // `any(feature = "a")` becomes `any (feature = "a")`.
    // We normalise by removing spaces before `(` and around `=`.
    let normalized: String = {
        let t = cfg_str.trim();
        // Remove spaces before `(`: `any (` → `any(`
        let t = t.replace(" (", "(");
        // Remove spaces around `=`: `feature = "a"` stays (already fine), but
        // in case of `feature ="a"` or `feature= "a"` etc.
        // The proc-macro2 representation is `feature = "a"`, which after
        // `strip_prefix("feature = \"")` works correctly, so we only need the `any (` fix.
        t
    };
    let cfg_str = normalized.as_str();

    // Simple: `feature = "name"`
    if let Some(feature) = cfg_str.strip_prefix("feature = \"").and_then(|s| s.strip_suffix('"')) {
        return enabled_features.iter().any(|ef| ef == feature);
    }
    // `any(...)` — enabled if at least one condition matches
    if let Some(inner) = cfg_str.strip_prefix("any(").and_then(|s| s.strip_suffix(')')) {
        return parse_cfg_list(inner)
            .iter()
            .any(|cond| cfg_condition_enabled(cond, enabled_features));
    }
    // `all(...)` — enabled if all conditions match
    if let Some(inner) = cfg_str.strip_prefix("all(").and_then(|s| s.strip_suffix(')')) {
        return parse_cfg_list(inner)
            .iter()
            .all(|cond| cfg_condition_enabled(cond, enabled_features));
    }
    // `not(...)` — invert the inner condition
    if let Some(inner) = cfg_str.strip_prefix("not(").and_then(|s| s.strip_suffix(')')) {
        return !cfg_condition_enabled(inner.trim(), enabled_features);
    }
    // Unknown pattern — strip the field (conservative)
    false
}

/// Split a comma-separated list of cfg conditions, respecting nested parentheses.
fn parse_cfg_list(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    result.push(trimmed);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        result.push(trimmed);
    }
    result
}

/// 2. Duplicate types: Keep only the first occurrence of each type name
/// 3. Duplicate enums: Keep only the first occurrence of each enum name
/// 4. Duplicate functions: Keep only the first occurrence of each function name
fn dedup_api_surface(api: &mut ApiSurface) {
    // Remove types that collide with enums (enums win)
    let enum_names: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
    api.types.retain(|t| !enum_names.contains(&t.name));

    // Remove types that collide with errors (errors win).
    // This catches the case where extract_impl_block previously created an opaque TypeDef
    // for a thiserror error enum that also had inherent impl methods.
    let error_names: AHashSet<String> = api.errors.iter().map(|e| e.name.clone()).collect();
    api.types.retain(|t| !error_names.contains(&t.name));

    // Dedup types by name — prefer shorter rust_path (closer to crate root).
    // This handles name collisions like sample_core::Table vs sample_core::extraction::docx::parser::Table.
    {
        let mut best: AHashMap<String, usize> = AHashMap::new();
        for (i, t) in api.types.iter().enumerate() {
            best.entry(t.name.clone())
                .and_modify(|prev_i| {
                    if api.types[i].rust_path.len() < api.types[*prev_i].rust_path.len() {
                        *prev_i = i;
                    }
                })
                .or_insert(i);
        }
        let keep: AHashSet<usize> = best.values().copied().collect();
        let mut idx = 0;
        api.types.retain(|_| {
            let k = keep.contains(&idx);
            idx += 1;
            k
        });
    }

    // Dedup enums by name — prefer shorter rust_path.
    {
        let mut best: AHashMap<String, usize> = AHashMap::new();
        for (i, e) in api.enums.iter().enumerate() {
            best.entry(e.name.clone())
                .and_modify(|prev_i| {
                    if api.enums[i].rust_path.len() < api.enums[*prev_i].rust_path.len() {
                        *prev_i = i;
                    }
                })
                .or_insert(i);
        }
        let keep: AHashSet<usize> = best.values().copied().collect();
        let mut idx = 0;
        api.enums.retain(|_| {
            let k = keep.contains(&idx);
            idx += 1;
            k
        });
    }

    // Dedup functions by name — prefer shorter rust_path (closer to crate root).
    // This resolves C2: when the same function name exists at multiple definition
    // sites (e.g. sample_core::utils::clean_extracted_text and
    // sample_core::text::quality::clean_extracted_text), prefer the one re-exported
    // nearest to the crate root, which is the one users call via module = sample_core.
    {
        let mut best: AHashMap<String, usize> = AHashMap::new();
        for (i, f) in api.functions.iter().enumerate() {
            best.entry(f.name.clone())
                .and_modify(|prev_i| {
                    if api.functions[i].rust_path.len() < api.functions[*prev_i].rust_path.len() {
                        *prev_i = i;
                    }
                })
                .or_insert(i);
        }
        let keep: AHashSet<usize> = best.values().copied().collect();
        let mut idx = 0;
        api.functions.retain(|_| {
            let k = keep.contains(&idx);
            idx += 1;
            k
        });
    }

    // Dedup errors by name (keep first)
    let mut seen_errors: AHashSet<String> = AHashSet::new();
    api.errors.retain(|e| seen_errors.insert(e.name.clone()));
}

/// Returns true if `name` (short) or `rust_path` (fully qualified) matches any entry in
/// `exclude_list`.
///
/// Entries that contain `::` are treated as fully-qualified Rust paths and matched against
/// `rust_path` (with `-` normalized to `_`). Plain entries (no `::`) are matched against
/// `name` only. This allows precise disambiguation when two types share the same short name
/// but live in different modules, e.g.:
///
/// ```toml
/// [crates.exclude]
/// types = [
///   "sample_core::core::config::formats::OutputFormat",  # internal; matched by rust_path
///   "OutputFormat",                                    # would match by name (all variants)
/// ]
/// ```
fn is_type_excluded(name: &str, rust_path: &str, exclude_list: &[String]) -> bool {
    exclude_list.iter().any(|entry| {
        if entry.contains("::") {
            // Fully-qualified path: match against rust_path (normalise hyphens to underscores).
            let normalised = rust_path.replace('-', "_");
            normalised == entry.as_str()
        } else {
            // Short name: match against the simple type name.
            name == entry.as_str()
        }
    })
}

fn apply_filters(mut api: ApiSurface, config: &ResolvedCrateConfig) -> ApiSurface {
    let exclude = &config.exclude;
    let include = &config.include;

    // Apply includes first (whitelist), expanding to transitively referenced types.
    //
    // The expansion seeds from BOTH `include.types` and the parameter/return types
    // of `include.functions`. Without the function seed, wrapper return types like
    // `BatchScrapeResults` (declared alongside the function that returns them) are
    // silently dropped when the user lists only the per-element type in `include.types`
    // — codegen then sees `return_type = String` after `sanitize_unknown_types` collapses
    // the unknown Named reference, and every binding facade emits the wrong signature.
    //
    // Including types reachable from included functions is the conservative fix: the
    // user already opted into the function via `include.functions`, so its public
    // signature (return type + params) is implicitly part of the binding surface.
    let mut expanded_include: Option<AHashSet<String>> = None;
    if !include.types.is_empty() {
        let expanded = expand_include_list(&api, &include.types, &include.functions);
        api.types.retain(|t| expanded.contains(&t.name));
        api.enums.retain(|e| expanded.contains(&e.name));
        // Errors are NOT filtered by include list — they're always extracted
        // when [generate] errors = true (controlled by the generation layer, not include)
        expanded_include = Some(expanded);
    }
    if !include.functions.is_empty() {
        api.functions.retain(|f| include.functions.contains(&f.name));
    }
    if expanded_include.is_some() || !include.functions.is_empty() {
        api.unsupported_public_items.retain(|item| {
            let short_name = item.item_path.rsplit("::").next().unwrap_or(item.item_path.as_str());
            let owner_name = short_name.split('.').next().unwrap_or(short_name);
            let included_type = expanded_include
                .as_ref()
                .is_some_and(|expanded| expanded.contains(owner_name));
            let included_function =
                item.item_kind == "function" && include.functions.iter().any(|name| name == owner_name);
            included_type || included_function
        });
    }

    // Then apply excludes (blacklist).
    // Entries containing `::` are matched against rust_path (fully-qualified); others by name.
    //
    // Capture rust_paths of excluded types BEFORE dropping them, so trait_bridge
    // codegen can still reference them by qualified path when they appear in trait
    // method signatures (preserves `impl Trait for Wrapper { fn render(&self,
    // doc: &sample_core::types::internal::HiddenDocument) }`).
    for typ in &api.types {
        if is_type_excluded(&typ.name, &typ.rust_path, &exclude.types) {
            api.excluded_type_paths
                .insert(typ.name.clone(), typ.rust_path.replace('-', "_"));
        }
    }
    for enm in &api.enums {
        if is_type_excluded(&enm.name, &enm.rust_path, &exclude.types) {
            api.excluded_type_paths
                .insert(enm.name.clone(), enm.rust_path.replace('-', "_"));
        }
    }

    api.types
        .retain(|t| !is_type_excluded(&t.name, &t.rust_path, &exclude.types));
    api.functions.retain(|f| !exclude.functions.contains(&f.name));
    api.enums
        .retain(|e| !is_type_excluded(&e.name, &e.rust_path, &exclude.types));
    api.errors
        .retain(|e| !is_type_excluded(&e.name, &e.rust_path, &exclude.types));

    // Filter `unsupported_public_items` against the same config-level excludes so that
    // a generic item the user has already opted out of via `[crates.exclude]` does not
    // surface as a fatal `unsupported_generic_item` diagnostic. The extractor's own
    // attribute-based skip check (`#[alef::skip]`, `#[doc(hidden)]`) is necessarily
    // narrower because it cannot see the user's `alef.toml` at extraction time.
    api.unsupported_public_items.retain(|item| {
        let short_name = item.item_path.rsplit("::").next().unwrap_or(item.item_path.as_str());
        let by_type_name = is_type_excluded(short_name, &item.item_path, &exclude.types);
        let by_fn_name = item.item_kind == "function" && exclude.functions.contains(&short_name.to_string());
        // `item_path` for methods is `crate::module::TypeName.method_name`; the tail after
        // the last `::` is `TypeName.method_name`, which is exactly the format users write in
        // `[crates.exclude] methods = ["TypeName.method_name"]`.
        let by_method_name = item.item_kind == "method" && exclude.methods.contains(&short_name.to_string());
        // Also skip a method on an excluded parent type — when the user excludes
        // `RequestContext`, every `RequestContext.<method>` should follow it out.
        let by_parent_excluded = if item.item_kind == "method" {
            if let Some((owner_short, _)) = short_name.split_once('.') {
                let owner_full = item
                    .item_path
                    .rsplit_once('.')
                    .map(|(p, _)| p)
                    .unwrap_or(item.item_path.as_str());
                is_type_excluded(owner_short, owner_full, &exclude.types)
            } else {
                false
            }
        } else {
            false
        };
        !(by_type_name || by_fn_name || by_method_name || by_parent_excluded)
    });

    // Apply method-level excludes: "TypeName.method_name"
    if !exclude.methods.is_empty() {
        for typ in &mut api.types {
            typ.methods.retain(|m| {
                let key = format!("{}.{}", typ.name, m.name);
                !exclude.methods.contains(&key)
            });
        }
        // Service-extractor configurators are populated separately from the regular
        // `impl T` walk; apply the same `OwnerType.method_name` exclude here so entries
        // in `exclude.methods` are honored by the per-binding service codegen, which would
        // otherwise emit a non-delegatable Rust shim and fail compilation.
        for service in &mut api.services {
            service.configurators.retain(|m| {
                let key = format!("{}.{}", service.name, m.name);
                !exclude.methods.contains(&key)
            });
        }
    }

    api
}

/// Expand the include list by transitively discovering all types referenced by fields,
/// method parameters, and return types of the included types, plus the signatures
/// (return type and params) of `include_functions`.
fn expand_include_list(api: &ApiSurface, include_types: &[String], include_functions: &[String]) -> AHashSet<String> {
    let mut needed: AHashSet<String> = include_types.iter().cloned().collect();
    let mut changed = true;

    // Build a map of all available types for lookup
    let all_types: AHashMap<String, &TypeDef> = api.types.iter().map(|t| (t.name.clone(), t)).collect();
    let all_enums: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    // Seed `needed` with type references from the signatures of included functions
    // before the fixed-point loop. The user has explicitly opted into these functions
    // via `include.functions`, so the types they expose at their public boundary must
    // survive the include-list filter — otherwise the function's return type gets
    // sanitized away to `String` later in the pipeline (regression for a batch fixture's
    // `BatchScrapeResults` / `BatchCrawlResults` wrapper structs).
    let include_function_set: AHashSet<&str> = include_functions.iter().map(String::as_str).collect();
    if !include_function_set.is_empty() {
        for func in &api.functions {
            if !include_function_set.contains(func.name.as_str()) {
                continue;
            }
            collect_named_types(&func.return_type, &mut needed, &all_types, &all_enums, &mut changed);
            for param in &func.params {
                collect_named_types(&param.ty, &mut needed, &all_types, &all_enums, &mut changed);
            }
        }
    }

    while changed {
        changed = false;
        let current: Vec<String> = needed.iter().cloned().collect();
        for type_name in &current {
            if let Some(typ) = all_types.get(type_name) {
                for field in &typ.fields {
                    collect_named_types(&field.ty, &mut needed, &all_types, &all_enums, &mut changed);
                }
                for method in &typ.methods {
                    collect_named_types(&method.return_type, &mut needed, &all_types, &all_enums, &mut changed);
                    for param in &method.params {
                        collect_named_types(&param.ty, &mut needed, &all_types, &all_enums, &mut changed);
                    }
                }
            }
        }
    }
    needed
}

/// Recursively collect all named type references from a TypeRef into the needed set.
fn collect_named_types(
    ty: &TypeRef,
    needed: &mut AHashSet<String>,
    all_types: &AHashMap<String, &TypeDef>,
    all_enums: &AHashSet<String>,
    changed: &mut bool,
) {
    match ty {
        TypeRef::Named(name)
            if (all_types.contains_key(name) || all_enums.contains(name)) && needed.insert(name.clone()) =>
        {
            *changed = true;
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => {
            collect_named_types(inner, needed, all_types, all_enums, changed);
        }
        TypeRef::Map(k, v) => {
            collect_named_types(k, needed, all_types, all_enums, changed);
            collect_named_types(v, needed, all_types, all_enums, changed);
        }
        _ => {}
    }
}

/// Rewrite a rust_path using path_mappings.
/// Matches the longest prefix first.
fn rewrite_path(path: &str, mappings: &HashMap<String, String>) -> String {
    let mut sorted: Vec<_> = mappings.iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
    for (from, to) in sorted {
        if path.starts_with(from.as_str()) {
            return format!("{}{}", to, &path[from.len()..]);
        }
    }
    path.to_string()
}

/// Rewrite each field's `type_rust_path` to the canonical `rust_path` of the same-named
/// type or enum in the (post-dedup) surface. Keeps field references and their resolved type
/// definitions in agreement so downstream path-compatibility checks don't spuriously fail.
fn normalize_field_type_paths(api: &mut ApiSurface) {
    // Innermost `Named` short name of a field type, looking through Optional/Vec/Map(value).
    fn named_name(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => named_name(inner),
            TypeRef::Map(_, v) => named_name(v),
            _ => None,
        }
    }

    let mut canonical: AHashMap<String, String> = AHashMap::new();
    for t in &api.types {
        canonical.insert(t.name.clone(), t.rust_path.clone());
    }
    for e in &api.enums {
        canonical.entry(e.name.clone()).or_insert_with(|| e.rust_path.clone());
    }

    let fix = |fields: &mut Vec<crate::core::ir::FieldDef>| {
        for field in fields {
            if field.type_rust_path.is_none() {
                continue;
            }
            if let Some(name) = named_name(&field.ty) {
                if let Some(path) = canonical.get(name) {
                    field.type_rust_path = Some(path.clone());
                }
            }
        }
    };

    for typ in &mut api.types {
        fix(&mut typ.fields);
    }
    for en in &mut api.enums {
        for variant in &mut en.variants {
            fix(&mut variant.fields);
        }
    }
}

/// Apply path_mappings to rewrite all rust_path fields in the API surface.
///
/// Uses [`ResolvedCrateConfig::effective_path_mappings`] which merges auto-derived mappings
/// (from `auto_path_mappings`) with explicit `path_mappings` entries.
fn apply_path_mappings(api: &mut ApiSurface, config: &ResolvedCrateConfig) {
    let mappings = config.effective_path_mappings();
    if mappings.is_empty() {
        return;
    }
    for typ in &mut api.types {
        if typ.original_rust_path.is_empty() {
            typ.original_rust_path = typ.rust_path.clone();
        }
        typ.rust_path = rewrite_path(&typ.rust_path, &mappings);
        // Also rewrite type_rust_path on fields so that field-level path mismatch
        // checks compare against the same (post-mapping) crate root.
        for field in &mut typ.fields {
            if let Some(ref mut path) = field.type_rust_path {
                *path = rewrite_path(path, &mappings);
            }
        }
    }
    for func in &mut api.functions {
        if func.original_rust_path.is_empty() {
            func.original_rust_path = func.rust_path.clone();
        }
        func.rust_path = rewrite_path(&func.rust_path, &mappings);
    }
    for enum_def in &mut api.enums {
        if enum_def.original_rust_path.is_empty() {
            enum_def.original_rust_path = enum_def.rust_path.clone();
        }
        enum_def.rust_path = rewrite_path(&enum_def.rust_path, &mappings);
    }
    for error_def in &mut api.errors {
        if error_def.original_rust_path.is_empty() {
            error_def.original_rust_path = error_def.rust_path.clone();
        }
        error_def.rust_path = rewrite_path(&error_def.rust_path, &mappings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// sanitize_type_ref must resolve Map inner types (e.g. Named("str") → String)
    /// without marking the Map as lossy. Lossy map inner changes are still reported
    /// separately so validation can block them before codegen.
    #[test]
    fn sanitize_map_with_cow_key_preserves_map_structure_and_returns_lossless() {
        let known_types = AHashSet::default();
        let known_enums = AHashSet::default();
        // "str" is NOT in known_types — it represents the inner type of Cow<'static, str>.
        // The key starts as Named("str") which the type_resolver emits for Cow<'static, str>.

        let mut ty = TypeRef::Map(Box::new(TypeRef::Named("str".into())), Box::new(TypeRef::Json));

        let status = sanitize_type_ref(&mut ty, &known_types, &known_enums);

        // The Map must be preserved — NOT converted to String.
        assert!(
            matches!(&ty, TypeRef::Map(k, v)
                if matches!(k.as_ref(), TypeRef::String)
                && matches!(v.as_ref(), TypeRef::Json)),
            "expected Map(String, Json) but got {ty:?}"
        );

        assert_eq!(status, TypeSanitization::Lossless);

        // Verify the symmetric case: Map(String, Json) with already-resolved key
        // should also return false (key is already String, no unknown Named types).
        let _ = known_types;
        let mut ty2 = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json));
        let sanitized2 = sanitize_type_ref(&mut ty2, &AHashSet::default(), &AHashSet::default());
        assert_eq!(sanitized2, TypeSanitization::Unchanged);
        assert!(
            matches!(&ty2, TypeRef::Map(k, v)
                if matches!(k.as_ref(), TypeRef::String)
                && matches!(v.as_ref(), TypeRef::Json)),
            "Map(String, Json) must not be mutated when already clean"
        );
    }

    #[test]
    fn sanitize_map_with_bare_value_is_reported_as_sanitized() {
        let mut ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Value".to_string())));

        let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());

        assert!(
            sanitized.is_lossy(),
            "ambiguous bare Value inside Map must not be silently accepted"
        );
        assert!(
            matches!(&ty, TypeRef::Map(_, value) if matches!(value.as_ref(), TypeRef::Named(name) if name == "Value")),
            "ambiguous bare Value must remain visible for validation, got {ty:?}"
        );
    }

    /// Map(String, String) — the old case that was already handled correctly downstream —
    /// must also return sanitized=false after this fix. Backends must handle it via the
    /// normal (non-sanitized) Map conversion path.
    #[test]
    fn sanitize_map_with_both_string_types_returns_not_sanitized() {
        let mut ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());
        assert_eq!(sanitized, TypeSanitization::Unchanged);
        assert!(matches!(
            &ty,
            TypeRef::Map(k, v)
                if matches!(k.as_ref(), TypeRef::String) && matches!(v.as_ref(), TypeRef::String)
        ));
    }

    #[test]
    fn sanitize_map_with_unknown_value_type_returns_lossy() {
        let mut ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Named("ForeignPayload".into())),
        );

        let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());

        assert_eq!(sanitized, TypeSanitization::Lossy);
        assert!(
            matches!(&ty, TypeRef::Map(_, value) if matches!(value.as_ref(), TypeRef::String)),
            "unknown map value should be visibly sanitized for validation, got {ty:?}"
        );
    }

    /// Primitive field (not a Map) with unknown Named inner type still gets sanitized=true.
    /// This ensures we didn't break non-Map sanitization.
    #[test]
    fn sanitize_named_unknown_type_returns_sanitized_true() {
        let mut ty = TypeRef::Named("UnknownForeignType".into());
        let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());
        assert!(sanitized.is_lossy());
        assert!(matches!(ty, TypeRef::String));
    }

    /// Vec<Named("unknown")> should still return sanitized=true (inner Named replaced with String).
    #[test]
    fn sanitize_vec_with_unknown_named_returns_sanitized_true() {
        let mut ty = TypeRef::Vec(Box::new(TypeRef::Named("MyForeignStruct".into())));
        let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());
        assert!(sanitized.is_lossy());
        assert!(matches!(
            &ty,
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String)
        ));
    }

    #[test]
    fn validate_extracted_api_does_not_suppress_critical_codes() {
        let api = ApiSurface {
            crate_name: "sample-lib".to_string(),
            functions: vec![crate::core::ir::FunctionDef {
                name: "render".to_string(),
                rust_path: "sample_lib::render".to_string(),
                original_rust_path: String::new(),
                params: vec![crate::core::ir::ParamDef {
                    name: "payload".to_string(),
                    ty: TypeRef::Named("MissingPayload".to_string()),
                    ..crate::core::ir::ParamDef::default()
                }],
                return_type: TypeRef::String,
                error_type: None,
                doc: String::new(),
                is_async: false,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            ..ApiSurface::default()
        };

        let config = ResolvedCrateConfig::default();
        let err = validate_extracted_api(&api, &config).expect_err("must stay fatal");

        assert!(
            err.to_string().contains("unknown_named_type"),
            "unexpected error: {err}"
        );
    }

    // ---------------------------------------------------------------------------
    // is_type_excluded — fully-qualified path matching
    // ---------------------------------------------------------------------------

    /// Plain (no-`::`) entries match by short name only.
    #[test]
    fn is_type_excluded_plain_entry_matches_by_name() {
        let exclude = vec!["OutputFormat".to_string()];

        // Short name hit
        assert!(
            is_type_excluded("OutputFormat", "sample_crate::types::OutputFormat", &exclude),
            "plain entry must match when name matches"
        );

        // Different name — no match
        assert!(
            !is_type_excluded("SomethingElse", "sample_crate::types::SomethingElse", &exclude),
            "plain entry must not match when name differs"
        );
    }

    /// Fully-qualified entries match only the specific rust_path, not any type
    /// that merely shares the same short name.
    ///
    /// Regression: sample_core::core::config::formats::OutputFormat must be excluded
    /// while sample_core::types::OutputFormat is retained.
    #[test]
    fn is_type_excluded_qualified_entry_matches_rust_path_not_name() {
        let exclude = vec!["sample_crate::core::config::formats::OutputFormat".to_string()];

        // The internal variant — must be excluded.
        assert!(
            is_type_excluded(
                "OutputFormat",
                "sample_crate::core::config::formats::OutputFormat",
                &exclude
            ),
            "qualified entry must match the exact rust_path"
        );

        // The public variant that shares the same short name — must NOT be excluded.
        assert!(
            !is_type_excluded("OutputFormat", "sample_crate::types::OutputFormat", &exclude),
            "qualified entry must NOT match a different rust_path with the same short name"
        );
    }

    /// Hyphens in rust_path are normalised to underscores before comparison, matching
    /// the convention used throughout alef's path mapping layer.
    #[test]
    fn is_type_excluded_normalises_hyphens_in_rust_path() {
        let exclude = vec!["my_crate::some_module::Foo".to_string()];

        // rust_path with hyphen in crate name — normalised to underscore before compare.
        assert!(
            is_type_excluded("Foo", "my-crate::some_module::Foo", &exclude),
            "hyphens in rust_path should be normalised to underscores"
        );
    }

    // ---------------------------------------------------------------------------
    // expand_include_list — function-signature seeding
    // ---------------------------------------------------------------------------

    fn make_typedef(name: &str) -> crate::core::ir::TypeDef {
        crate::core::ir::TypeDef {
            name: name.to_string(),
            rust_path: format!("my_crate::{name}"),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            doc: String::new(),
            cfg: None,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }
    }

    fn make_funcdef(name: &str, return_type: TypeRef, param_types: Vec<TypeRef>) -> crate::core::ir::FunctionDef {
        crate::core::ir::FunctionDef {
            name: name.to_string(),
            rust_path: format!("my_crate::{name}"),
            original_rust_path: String::new(),
            params: param_types
                .into_iter()
                .enumerate()
                .map(|(i, ty)| crate::core::ir::ParamDef {
                    name: format!("arg{i}"),
                    ty,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                })
                .collect(),
            return_type,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    fn surface_with(types: Vec<crate::core::ir::TypeDef>, functions: Vec<crate::core::ir::FunctionDef>) -> ApiSurface {
        ApiSurface {
            crate_name: "my_crate".into(),
            version: "0.1.0".into(),
            types,
            functions,
            enums: vec![],
            errors: vec![],
            excluded_type_paths: std::collections::HashMap::new(),
            excluded_trait_names: std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    /// Regression for a batch-result include bug: a function listed in
    /// `[crates.include].functions` returns a wrapper struct that is NOT in
    /// `[crates.include].types`. Before the fix, the include filter dropped the
    /// wrapper struct (it was unreachable from the included types), and the later
    /// `sanitize_unknown_types` pass collapsed the function's `return_type` to
    /// `String`, breaking every binding facade.
    ///
    /// After the fix, `expand_include_list` seeds itself from included functions'
    /// signatures so the wrapper is retained.
    #[test]
    fn expand_include_list_seeds_from_included_function_signatures() {
        let surface = surface_with(
            vec![
                make_typedef("BatchScrapeResult"),
                make_typedef("BatchScrapeResults"),
                make_typedef("UnusedType"),
            ],
            vec![make_funcdef(
                "batch_scrape",
                TypeRef::Named("BatchScrapeResults".into()),
                vec![TypeRef::Vec(Box::new(TypeRef::String))],
            )],
        );

        let include_types = vec!["BatchScrapeResult".to_string()];
        let include_functions = vec!["batch_scrape".to_string()];

        let expanded = expand_include_list(&surface, &include_types, &include_functions);

        assert!(
            expanded.contains("BatchScrapeResult"),
            "per-element type explicitly listed must be present; got: {expanded:?}"
        );
        assert!(
            expanded.contains("BatchScrapeResults"),
            "wrapper return type of included function must be auto-included; got: {expanded:?}"
        );
        assert!(
            !expanded.contains("UnusedType"),
            "unrelated type must not be pulled in; got: {expanded:?}"
        );
    }

    /// Function parameter types must also be retained — a function listed in
    /// `include.functions` that accepts a custom config struct must keep that
    /// struct in the surface even if the user forgot to list it under
    /// `include.types`.
    #[test]
    fn expand_include_list_seeds_from_included_function_param_types() {
        let surface = surface_with(
            vec![make_typedef("CrawlConfig"), make_typedef("EngineHandle")],
            vec![make_funcdef(
                "create_engine",
                TypeRef::Named("EngineHandle".into()),
                vec![TypeRef::Optional(Box::new(TypeRef::Named("CrawlConfig".into())))],
            )],
        );

        let include_types = vec!["EngineHandle".to_string()];
        let include_functions = vec!["create_engine".to_string()];

        let expanded = expand_include_list(&surface, &include_types, &include_functions);

        assert!(
            expanded.contains("CrawlConfig"),
            "param type referenced through Optional must be retained; got: {expanded:?}"
        );
    }

    /// When no functions are in the include list, behaviour is unchanged —
    /// expansion stays anchored to `include_types` only.
    #[test]
    fn expand_include_list_with_empty_functions_matches_legacy_behaviour() {
        let surface = surface_with(
            vec![make_typedef("Kept"), make_typedef("Dropped")],
            vec![make_funcdef("do_thing", TypeRef::Named("Dropped".into()), vec![])],
        );

        let include_types = vec!["Kept".to_string()];
        let include_functions: Vec<String> = vec![];

        let expanded = expand_include_list(&surface, &include_types, &include_functions);
        assert!(expanded.contains("Kept"));
        assert!(
            !expanded.contains("Dropped"),
            "function not in include.functions must not pull in its return type; got: {expanded:?}"
        );
    }

    // ---------------------------------------------------------------------------
    // apply_filters — exclude.methods suppresses unsupported_public_items
    // ---------------------------------------------------------------------------

    fn make_unsupported_method(type_name: &str, method_name: &str) -> crate::core::ir::UnsupportedPublicItem {
        crate::core::ir::UnsupportedPublicItem {
            item_kind: "method".to_string(),
            item_path: format!("my_crate::module::{type_name}.{method_name}"),
            reason: "public generic trait methods cannot be represented without explicit monomorphization metadata"
                .to_string(),
            suggested_fix: "exclude the method".to_string(),
        }
    }

    fn make_unsupported_function(fn_name: &str) -> crate::core::ir::UnsupportedPublicItem {
        crate::core::ir::UnsupportedPublicItem {
            item_kind: "function".to_string(),
            item_path: format!("my_crate::{fn_name}"),
            reason: "generic function".to_string(),
            suggested_fix: "exclude the function".to_string(),
        }
    }

    /// A method item whose `TypeName.method_name` tail appears in `exclude.methods`
    /// must be removed from `unsupported_public_items`.
    #[test]
    fn apply_filters_removes_unsupported_method_when_excluded_by_methods_list() {
        let mut surface = surface_with(vec![], vec![]);
        surface
            .unsupported_public_items
            .push(make_unsupported_method("NodeContext", "serialize"));

        let mut config = ResolvedCrateConfig::default();
        config.exclude.methods = vec!["NodeContext.serialize".to_string()];

        let result = apply_filters(surface, &config);

        assert!(
            result.unsupported_public_items.is_empty(),
            "method listed in exclude.methods must be removed from unsupported_public_items; \
             remaining: {:?}",
            result.unsupported_public_items
        );
    }

    /// A method item whose tail is NOT in `exclude.methods` must be retained so the
    /// diagnostic still surfaces as a fatal error.
    #[test]
    fn apply_filters_retains_unsupported_method_when_not_in_exclude_list() {
        let mut surface = surface_with(vec![], vec![]);
        surface
            .unsupported_public_items
            .push(make_unsupported_method("NodeContext", "serialize"));

        let mut config = ResolvedCrateConfig::default();
        config.exclude.methods = vec!["NodeContext.other_method".to_string()];

        let result = apply_filters(surface, &config);

        assert_eq!(
            result.unsupported_public_items.len(),
            1,
            "method NOT in exclude.methods must remain in unsupported_public_items"
        );
    }

    /// Non-method items (kind == "function") must be unaffected by `exclude.methods` —
    /// they are only suppressed by `exclude.functions`.
    #[test]
    fn apply_filters_exclude_methods_does_not_affect_unsupported_function_items() {
        let mut surface = surface_with(vec![], vec![]);
        surface
            .unsupported_public_items
            .push(make_unsupported_function("generic_helper"));

        // Deliberately add the function path tail to exclude.methods — must have no effect.
        let mut config = ResolvedCrateConfig::default();
        config.exclude.methods = vec!["generic_helper".to_string()];

        let result = apply_filters(surface, &config);

        assert_eq!(
            result.unsupported_public_items.len(),
            1,
            "exclude.methods must not suppress items with item_kind == 'function'"
        );
    }

    #[test]
    fn apply_filters_retains_unsupported_function_when_included_by_function_list() {
        let mut surface = surface_with(vec![], vec![]);
        surface
            .unsupported_public_items
            .push(make_unsupported_function("generic_helper"));
        surface
            .unsupported_public_items
            .push(make_unsupported_function("unused_generic"));

        let mut config = ResolvedCrateConfig::default();
        config.include.functions = vec!["generic_helper".to_string()];

        let result = apply_filters(surface, &config);

        assert_eq!(
            result
                .unsupported_public_items
                .iter()
                .map(|item| item.item_path.as_str())
                .collect::<Vec<_>>(),
            vec!["my_crate::generic_helper"],
            "include.functions must retain diagnostics only for included generic functions"
        );
    }

    #[test]
    fn apply_filters_retains_unsupported_method_when_parent_type_is_included() {
        let mut surface = surface_with(vec![make_typedef("NodeContext"), make_typedef("OtherType")], vec![]);
        surface
            .unsupported_public_items
            .push(make_unsupported_method("NodeContext", "serialize"));
        surface
            .unsupported_public_items
            .push(make_unsupported_method("OtherType", "serialize"));

        let mut config = ResolvedCrateConfig::default();
        config.include.types = vec!["NodeContext".to_string()];

        let result = apply_filters(surface, &config);

        assert_eq!(
            result
                .unsupported_public_items
                .iter()
                .map(|item| item.item_path.as_str())
                .collect::<Vec<_>>(),
            vec!["my_crate::module::NodeContext.serialize"],
            "include.types must retain diagnostics only for methods owned by included public types"
        );
    }

    // ---------------------------------------------------------------------------
    // Regression: configurator methods survive the exclude-methods post-service pass
    // ---------------------------------------------------------------------------

    /// A method declared in `[[crates.services]].configurators` must remain in
    /// `service.configurators` even when the same `OwnerType.method_name` key also
    /// appears in `[crates.exclude].methods`.
    ///
    /// Background: the exclude list is intended to suppress the *generic struct-level*
    /// method emission (preventing non-delegatable stubs in binding codegen). It must
    /// not remove the method from the *service IR*, where its presence drives dedicated
    /// C/host-language configurator entrypoints. Both intents are independent.
    ///
    /// The fixture uses a purely synthetic owner type so no consumer-library names
    /// appear in the test.
    #[test]
    fn configurator_survives_exclude_methods_post_service_pass() {
        use crate::core::config::service::{EntrypointSpec, ServiceConfig};
        use crate::core::ir::{MethodDef, ReceiverKind, ServiceDef, TypeRef};

        // Build a minimal service with one configurator (`setup`) already populated.
        let configurator_method = MethodDef {
            name: "setup".to_string(),
            params: vec![],
            return_type: TypeRef::Named("Foo".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(ReceiverKind::Owned),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };
        let constructor_method = MethodDef {
            name: "new".to_string(),
            params: vec![],
            return_type: TypeRef::Named("Foo".to_string()),
            is_async: false,
            is_static: true,
            error_type: None,
            doc: String::new(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };
        let service = ServiceDef {
            name: "Foo".to_string(),
            rust_path: "test_crate::Foo".to_string(),
            constructor: constructor_method,
            configurators: vec![configurator_method],
            registrations: vec![],
            entrypoints: vec![],
            doc: String::new(),
            cfg: None,
        };

        // Wire up a ResolvedCrateConfig that:
        // - declares `setup` as a configurator via services[].configurators
        // - also lists `Foo.setup` in exclude.methods (the scenario that previously cleared it)
        let mut config = ResolvedCrateConfig {
            name: "test_crate".to_string(),
            services: vec![ServiceConfig {
                owner_type: "Foo".to_string(),
                constructor: Some("new".to_string()),
                configurators: vec!["setup".to_string()],
                registrations: vec![],
                entrypoints: vec![EntrypointSpec {
                    method: "run".to_string(),
                    kind: "run".to_string(),
                }],
                skip_languages: vec![],
                host_app_inner_accessor: None,
            }],
            ..Default::default()
        };
        config.exclude.methods = vec!["Foo.setup".to_string()];

        // Simulate the pipeline state just after extract_services has populated services.
        let mut api = ApiSurface {
            crate_name: "test_crate".to_string(),
            services: vec![service],
            ..ApiSurface::default()
        };

        // Apply the post-extract_services exclude pass (the fix under test).
        if !config.exclude.methods.is_empty() {
            for typ in &mut api.types {
                typ.methods.retain(|m| {
                    let key = format!("{}.{}", typ.name, m.name);
                    !config.exclude.methods.contains(&key)
                });
            }
        }

        // The configurator must still be present: the exclude list controls struct-level
        // method emission, NOT the service IR configurator list.
        assert_eq!(api.services.len(), 1, "service must be present after the exclude pass");
        assert_eq!(
            api.services[0].configurators.len(),
            1,
            "configurator `setup` must survive the exclude-methods post-service pass; got {:?}",
            api.services[0]
                .configurators
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            api.services[0].configurators[0].name, "setup",
            "configurator name must be `setup`"
        );
    }
}
