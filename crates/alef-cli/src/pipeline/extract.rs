use ahash::{AHashMap, AHashSet};
use alef_core::config::AlefConfig;
use alef_core::ir::{ApiSurface, TypeDef, TypeRef};
use anyhow::Context as _;
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info};

use crate::cache;

use super::version::read_version;

/// Ensure required entries are in `.gitignore` — creates the file if absent.
/// Adds `.alef/` (cache) and language-specific build artifacts based on config.
pub fn ensure_gitignore(base_dir: &Path, config: &AlefConfig) {
    use alef_core::config::Language;

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
pub fn extract(config: &AlefConfig, config_path: &Path, clean: bool) -> anyhow::Result<ApiSurface> {
    // Ensure .gitignore has required entries
    if let Some(parent) = config_path.parent() {
        ensure_gitignore(parent, config);
    }

    let source_hash = cache::compute_source_hash(&config.crate_config.sources, config_path)
        .context("failed to compute source hash")?;

    if !clean && cache::is_ir_cached(&source_hash) {
        info!("Using cached IR");
        return cache::read_cached_ir().context("failed to read cached IR");
    }

    let mut api = extract_raw(config, config_path)?;

    // Apply global filters (includes and excludes)
    api = apply_filters(api, config);

    // Inject declared opaque types from config (external crate types alef can't extract)
    inject_declared_opaque_types(&mut api, config);

    // Remove cfg-gated fields unless their feature is in [crate].features.
    // Binding crates may have different features enabled than the core crate,
    // so cfg-gated fields are only included when explicitly listed.
    strip_cfg_fields(&mut api, &config.crate_config.features);

    // Replace references to types not in the API surface with String
    sanitize_unknown_types(&mut api);

    // Deduplicate types, enums, and functions by name
    dedup_api_surface(&mut api);

    // Apply path mappings to rewrite rust_path fields
    apply_path_mappings(&mut api, config);

    cache::write_ir_cache(&api, &source_hash).context("failed to write IR cache")?;
    info!(
        "Extracted {} types, {} functions, {} enums",
        api.types.len(),
        api.functions.len(),
        api.enums.len()
    );

    Ok(api)
}

/// Extract the full, unfiltered API surface for documentation generation.
///
/// This skips `[include]`/`[exclude]` binding filters and type sanitization so
/// that docs contain ALL public types from source files, not just the subset
/// that survives binding codegen filters.  Deduplication, cfg-field stripping,
/// and path mappings are still applied because they improve doc quality without
/// removing types.
pub fn extract_unfiltered(config: &AlefConfig, config_path: &Path) -> anyhow::Result<ApiSurface> {
    // Ensure .gitignore has required entries
    if let Some(parent) = config_path.parent() {
        ensure_gitignore(parent, config);
    }

    let source_hash = cache::compute_source_hash(&config.crate_config.sources, config_path)
        .context("failed to compute source hash")?;

    let unfiltered_hash = format!("{source_hash}-unfiltered");
    if cache::is_ir_cached_as(&unfiltered_hash, "ir-unfiltered") {
        info!("Using cached unfiltered IR");
        return cache::read_cached_ir_as("ir-unfiltered").context("failed to read cached unfiltered IR");
    }

    let mut api = extract_raw(config, config_path)?;

    // Skip apply_filters — we want ALL types for docs.
    // Skip sanitize_unknown_types — docs don't need compilable type references.

    // Inject declared opaque types (docs should show these too)
    inject_declared_opaque_types(&mut api, config);

    // Strip cfg-gated fields (these are legitimately conditional)
    strip_cfg_fields(&mut api, &config.crate_config.features);

    // Deduplicate types, enums, and functions by name
    dedup_api_surface(&mut api);

    // Apply path mappings to rewrite rust_path fields
    apply_path_mappings(&mut api, config);

    cache::write_ir_cache_as(&api, &unfiltered_hash, "ir-unfiltered").context("failed to write unfiltered IR cache")?;
    info!(
        "Extracted (unfiltered) {} types, {} functions, {} enums",
        api.types.len(),
        api.functions.len(),
        api.enums.len()
    );

    Ok(api)
}

/// Shared raw extraction logic: parse sources, produce raw `ApiSurface`.
fn extract_raw(config: &AlefConfig, _config_path: &Path) -> anyhow::Result<ApiSurface> {
    info!("Extracting API surface from Rust source...");
    let sources: Vec<&Path> = config.crate_config.sources.iter().map(|p| p.as_path()).collect();

    // Read version from Cargo.toml
    let version = read_version(&config.crate_config.version_from)?;

    let workspace_root = config.crate_config.workspace_root.as_deref();
    alef_extract::extractor::extract(&sources, &config.crate_config.name, &version, workspace_root)
        .context("failed to extract API surface")
}

/// Inject declared opaque types from config into the API surface.
/// These are external crate types that alef can't extract but needs to generate wrappers for.
fn inject_declared_opaque_types(api: &mut ApiSurface, config: &AlefConfig) {
    for (name, rust_path) in &config.opaque_types {
        // Only add if not already in the API surface
        if !api.types.iter().any(|t| t.name == *name) && !api.enums.iter().any(|e| e.name == *name) {
            api.types.push(alef_core::ir::TypeDef {
                name: name.clone(),
                rust_path: rust_path.clone(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                doc: String::new(),
                cfg: None,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
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
    // module paths (e.g., `kreuzberg::types::OutputFormat` vs `kreuzberg::OutputFormat`).
    // Normalize hyphens to underscores in paths for consistent comparison.
    let known_type_paths: AHashSet<String> = api.types.iter().map(|t| t.rust_path.replace('-', "_")).collect();
    let known_enum_paths: AHashSet<String> = api.enums.iter().map(|e| e.rust_path.replace('-', "_")).collect();

    for typ in &mut api.types {
        for field in &mut typ.fields {
            if sanitize_type_ref(&mut field.ty, &known_types, &known_enums) {
                field.sanitized = true;
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
                            // (e.g., crate::metadata::HtmlMetadata vs html-to-markdown-rs::HtmlMetadata).
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
        for method in &mut typ.methods {
            let mut method_sanitized = false;
            for param in &mut method.params {
                if sanitize_type_ref(&mut param.ty, &known_types, &known_enums) {
                    param.sanitized = true;
                    method_sanitized = true;
                }
            }
            // Skip sanitizing return type if it's Named(parent_type) — builder/factory pattern.
            // Methods that return their own type (e.g. with_foo(&self) -> Self) should keep
            // the Named return so codegen can delegate them correctly.
            let is_self_return = matches!(&method.return_type, TypeRef::Named(n) if n == &type_name);
            if !is_self_return && sanitize_type_ref(&mut method.return_type, &known_types, &known_enums) {
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
            if sanitize_type_ref(&mut param.ty, &known_types, &known_enums) {
                param.sanitized = true;
                func_sanitized = true;
            }
        }
        if sanitize_type_ref(&mut func.return_type, &known_types, &known_enums) {
            func_sanitized = true;
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
                if sanitize_type_ref(&mut field.ty, &known_types, &known_enums) {
                    field.sanitized = true;
                }
            }
        }
    }
    // Sanitize error variant fields as well.
    for error_def in &mut api.errors {
        for variant in &mut error_def.variants {
            for field in &mut variant.fields {
                if sanitize_type_ref(&mut field.ty, &known_types, &known_enums) {
                    field.sanitized = true;
                }
            }
        }
    }
}

/// Returns true if the type was sanitized (changed from original).
fn sanitize_type_ref(ty: &mut TypeRef, known_types: &AHashSet<String>, known_enums: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Named(name) if !known_types.contains(name.as_str()) && !known_enums.contains(name.as_str()) => {
            *ty = TypeRef::String;
            true
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => sanitize_type_ref(inner, known_types, known_enums),
        TypeRef::Map(k, v) => {
            let a = sanitize_type_ref(k, known_types, known_enums);
            let b = sanitize_type_ref(v, known_types, known_enums);
            a || b
        }
        _ => false,
    }
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
        // Retain non-cfg fields and cfg fields whose feature is enabled.
        typ.fields.retain(|f| match &f.cfg {
            None => true,
            Some(cfg_str) => cfg_str
                .strip_prefix("feature = \"")
                .and_then(|s| s.strip_suffix('"'))
                .is_some_and(|feature| enabled_features.iter().any(|ef| ef == feature)),
        });
        // Clear cfg on retained fields so codegen treats them as unconditional.
        for field in &mut typ.fields {
            field.cfg = None;
        }
        // Mark if any cfg fields were actually stripped (not enabled).
        if cfg_count > 0 && typ.fields.len() < original_count {
            typ.has_stripped_cfg_fields = true;
        }
    }
}

/// 2. Duplicate types: Keep only the first occurrence of each type name
/// 3. Duplicate enums: Keep only the first occurrence of each enum name
/// 4. Duplicate functions: Keep only the first occurrence of each function name
fn dedup_api_surface(api: &mut ApiSurface) {
    // Remove types that collide with enums (enums win)
    let enum_names: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
    api.types.retain(|t| !enum_names.contains(&t.name));

    // Dedup types by name — prefer shorter rust_path (closer to crate root).
    // This handles name collisions like kreuzberg::Table vs kreuzberg::extraction::docx::parser::Table.
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

    // Dedup functions by name (keep first)
    let mut seen_fns: AHashSet<String> = AHashSet::new();
    api.functions.retain(|f| seen_fns.insert(f.name.clone()));

    // Dedup errors by name (keep first)
    let mut seen_errors: AHashSet<String> = AHashSet::new();
    api.errors.retain(|e| seen_errors.insert(e.name.clone()));
}

fn apply_filters(mut api: ApiSurface, config: &AlefConfig) -> ApiSurface {
    let exclude = &config.exclude;
    let include = &config.include;

    // Apply includes first (whitelist), expanding to transitively referenced types
    if !include.types.is_empty() {
        let expanded = expand_include_list(&api, &include.types);
        api.types.retain(|t| expanded.contains(&t.name));
        api.enums.retain(|e| expanded.contains(&e.name));
        // Errors are NOT filtered by include list — they're always extracted
        // when [generate] errors = true (controlled by the generation layer, not include)
    }
    if !include.functions.is_empty() {
        api.functions.retain(|f| include.functions.contains(&f.name));
    }

    // Then apply excludes (blacklist)
    api.types.retain(|t| !exclude.types.contains(&t.name));
    api.functions.retain(|f| !exclude.functions.contains(&f.name));
    api.enums.retain(|e| !exclude.types.contains(&e.name));
    api.errors.retain(|e| !exclude.types.contains(&e.name));

    // Apply method-level excludes: "TypeName.method_name"
    if !exclude.methods.is_empty() {
        for typ in &mut api.types {
            typ.methods.retain(|m| {
                let key = format!("{}.{}", typ.name, m.name);
                !exclude.methods.contains(&key)
            });
        }
    }

    api
}

/// Expand the include list by transitively discovering all types referenced by fields,
/// method parameters, and return types of the included types.
fn expand_include_list(api: &ApiSurface, include_types: &[String]) -> AHashSet<String> {
    let mut needed: AHashSet<String> = include_types.iter().cloned().collect();
    let mut changed = true;

    // Build a map of all available types for lookup
    let all_types: AHashMap<String, &TypeDef> = api.types.iter().map(|t| (t.name.clone(), t)).collect();
    let all_enums: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

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

/// Apply path_mappings to rewrite all rust_path fields in the API surface.
fn apply_path_mappings(api: &mut ApiSurface, config: &AlefConfig) {
    if config.crate_config.path_mappings.is_empty() {
        return;
    }
    for typ in &mut api.types {
        typ.rust_path = rewrite_path(&typ.rust_path, &config.crate_config.path_mappings);
    }
    for func in &mut api.functions {
        func.rust_path = rewrite_path(&func.rust_path, &config.crate_config.path_mappings);
    }
    for enum_def in &mut api.enums {
        enum_def.rust_path = rewrite_path(&enum_def.rust_path, &config.crate_config.path_mappings);
    }
    for error_def in &mut api.errors {
        error_def.rust_path = rewrite_path(&error_def.rust_path, &config.crate_config.path_mappings);
    }
}
