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

    let source_hash = cache::sources_hash(&config.crate_config.sources).context("failed to compute sources hash")?;

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

    // Apply path mappings to rewrite rust_path fields before dedup so that
    // two types that had different raw paths but map to the same rewritten
    // path are correctly collapsed into one.
    apply_path_mappings(&mut api, config);

    // Deduplicate types, enums, and functions by name (after path mapping so
    // rewritten paths are used for the shortest-path preference heuristic).
    dedup_api_surface(&mut api);

    cache::write_ir_cache(&api, &source_hash).context("failed to write IR cache")?;
    info!(
        "Extracted {} types, {} functions, {} enums",
        api.types.len(),
        api.functions.len(),
        api.enums.len()
    );

    Ok(api)
}

/// Shared raw extraction logic: parse sources, produce raw `ApiSurface`.
///
/// Groups source files by their owning crate (derived from `crates/{name}/src/` path
/// patterns) and extracts each group with the correct crate name. This ensures types
/// get accurate `rust_path` values reflecting their actual defining crate, not the
/// facade crate name from config.
fn extract_raw(config: &AlefConfig, _config_path: &Path) -> anyhow::Result<ApiSurface> {
    info!("Extracting API surface from Rust source...");
    let version = read_version(&config.crate_config.version_from)?;
    let workspace_root = config.crate_config.workspace_root.as_deref();
    let default_name = &config.crate_config.name;

    // Build source groups: use explicit source_crates config when available,
    // otherwise derive crate names from file paths in the flat sources list.
    let mut groups: std::collections::BTreeMap<String, Vec<&Path>> = std::collections::BTreeMap::new();
    if !config.crate_config.source_crates.is_empty() {
        for sc in &config.crate_config.source_crates {
            let crate_name = sc.name.replace('-', "_");
            for source in &sc.sources {
                groups.entry(crate_name.clone()).or_default().push(source.as_path());
            }
        }
    } else {
        for source in &config.crate_config.sources {
            let crate_name = derive_crate_name_from_path(source, default_name);
            groups.entry(crate_name).or_default().push(source.as_path());
        }
    }

    // Extract each group with its own crate name, then merge
    let mut merged = ApiSurface {
        crate_name: default_name.to_string(),
        version: version.clone(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    for (crate_name, sources) in &groups {
        let api = alef_extract::extractor::extract(sources, crate_name, &version, workspace_root)
            .with_context(|| format!("failed to extract API surface from crate {crate_name}"))?;
        merged.types.extend(api.types);
        merged.functions.extend(api.functions);
        merged.enums.extend(api.enums);
        merged.errors.extend(api.errors);
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
fn inject_declared_opaque_types(api: &mut ApiSurface, config: &AlefConfig) {
    let mut sorted_opaques: Vec<_> = config.opaque_types.iter().collect();
    sorted_opaques.sort_by_key(|(name, _)| (*name).clone());
    for (name, rust_path) in sorted_opaques {
        // Only add if not already in the API surface
        if !api.types.iter().any(|t| t.name == *name) && !api.enums.iter().any(|e| e.name == *name) {
            api.types.push(alef_core::ir::TypeDef {
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
            // Detect homogeneous numeric tuple types such as `(u32, u32)` that serde serializes
            // as JSON arrays.  Map them to Vec<ElemType> so backends emit array types (e.g.
            // `[]uint32` in Go) rather than falling back to `string`.  This preserves round-trip
            // JSON compatibility: `null | [800, 600]` unmarshals correctly into `*[]uint32`.
            if let Some(elem_ty) = parse_homogeneous_tuple(name) {
                *ty = TypeRef::Vec(Box::new(elem_ty));
                return true; // Sanitized — the core type is a tuple, not a Vec
            }
            *ty = TypeRef::String;
            true
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => sanitize_type_ref(inner, known_types, known_enums),
        TypeRef::Map(k, v) => {
            // Sanitize inner key and value types (e.g. Named("str") → String) so
            // backends receive clean Map(String, Json) rather than Map(Named("str"), Json).
            // However, the Map *structure itself* is always valid — all backends have explicit
            // TypeRef::Map handling — so do NOT propagate sanitized=true to the caller.
            // If we returned true here, the field would be flagged as sanitized and the
            // conversion codegen would fall through to the Debug-format fallback
            // (format!("{:?}", val.field)), which is wrong for every backend.
            sanitize_type_ref(k, known_types, known_enums);
            sanitize_type_ref(v, known_types, known_enums);
            false
        }
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
    use alef_core::ir::PrimitiveType;
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
        // Retain non-cfg fields and cfg fields whose feature condition is satisfied.
        typ.fields.retain(|f| match &f.cfg {
            None => true,
            Some(cfg_str) => cfg_condition_enabled(cfg_str, enabled_features),
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

    // Dedup functions by name — prefer shorter rust_path (closer to crate root).
    // This resolves C2: when the same function name exists at multiple definition
    // sites (e.g. kreuzberg::utils::clean_extracted_text and
    // kreuzberg::text::quality::clean_extracted_text), prefer the one re-exported
    // nearest to the crate root, which is the one users call via module = kreuzberg.
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
///
/// Uses [`AlefConfig::effective_path_mappings`] which merges auto-derived mappings
/// (from `auto_path_mappings`) with explicit `path_mappings` entries.
fn apply_path_mappings(api: &mut ApiSurface, config: &AlefConfig) {
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
    /// but must NOT mark the Map itself as sanitized. Returning sanitized=true for a
    /// Map causes downstream backends to fall through to the Debug-format fallback
    /// (`format!("{:?}", val.field)`) instead of emitting the correct HashMap/JsValue
    /// conversion. Regression for AHashMap<Cow<'static, str>, serde_json::Value>.
    #[test]
    fn sanitize_map_with_cow_key_preserves_map_structure_and_returns_not_sanitized() {
        let known_types = AHashSet::default();
        let known_enums = AHashSet::default();
        // "str" is NOT in known_types — it represents the inner type of Cow<'static, str>.
        // The key starts as Named("str") which the type_resolver emits for Cow<'static, str>.

        let mut ty = TypeRef::Map(Box::new(TypeRef::Named("str".into())), Box::new(TypeRef::Json));

        let sanitized = sanitize_type_ref(&mut ty, &known_types, &known_enums);

        // The Map must be preserved — NOT converted to String.
        assert!(
            matches!(&ty, TypeRef::Map(k, v)
                if matches!(k.as_ref(), TypeRef::String)
                && matches!(v.as_ref(), TypeRef::Json)),
            "expected Map(String, Json) but got {ty:?}"
        );

        // sanitize_type_ref must return false: the Map structure is valid and backends
        // have explicit Map handling. Returning true would mark field.sanitized=true
        // and trigger the wrong conversion code path in all backends.
        assert!(
            !sanitized,
            "sanitize_type_ref returned sanitized=true for Map — this triggers the Debug-format fallback"
        );

        // Verify the symmetric case: Map(String, Json) with already-resolved key
        // should also return false (key is already String, no unknown Named types).
        let _ = known_types;
        let mut ty2 = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json));
        let sanitized2 = sanitize_type_ref(&mut ty2, &AHashSet::default(), &AHashSet::default());
        assert!(!sanitized2, "Map(String, Json) should not be sanitized");
        assert!(
            matches!(&ty2, TypeRef::Map(k, v)
                if matches!(k.as_ref(), TypeRef::String)
                && matches!(v.as_ref(), TypeRef::Json)),
            "Map(String, Json) must not be mutated when already clean"
        );
    }

    /// Map(String, String) — the old case that was already handled correctly downstream —
    /// must also return sanitized=false after this fix. Backends must handle it via the
    /// normal (non-sanitized) Map conversion path.
    #[test]
    fn sanitize_map_with_both_string_types_returns_not_sanitized() {
        let mut ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());
        assert!(!sanitized);
        assert!(matches!(
            &ty,
            TypeRef::Map(k, v)
                if matches!(k.as_ref(), TypeRef::String) && matches!(v.as_ref(), TypeRef::String)
        ));
    }

    /// Primitive field (not a Map) with unknown Named inner type still gets sanitized=true.
    /// This ensures we didn't break non-Map sanitization.
    #[test]
    fn sanitize_named_unknown_type_returns_sanitized_true() {
        let mut ty = TypeRef::Named("UnknownForeignType".into());
        let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());
        assert!(sanitized);
        assert!(matches!(ty, TypeRef::String));
    }

    /// Vec<Named("unknown")> should still return sanitized=true (inner Named replaced with String).
    #[test]
    fn sanitize_vec_with_unknown_named_returns_sanitized_true() {
        let mut ty = TypeRef::Vec(Box::new(TypeRef::Named("MyForeignStruct".into())));
        let sanitized = sanitize_type_ref(&mut ty, &AHashSet::default(), &AHashSet::default());
        assert!(sanitized);
        assert!(matches!(
            &ty,
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String)
        ));
    }
}
