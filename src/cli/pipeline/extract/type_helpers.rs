use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, FieldDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use std::collections::HashMap;
use tracing::debug;

pub(super) fn inject_declared_opaque_types(api: &mut ApiSurface, config: &ResolvedCrateConfig) {
    let mut sorted_opaques: Vec<_> = config.opaque_types.iter().collect();
    sorted_opaques.sort_by_key(|(name, _)| (*name).clone());
    for (name, rust_path) in sorted_opaques {
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
                has_private_fields: false,
                version: Default::default(),
            });
            debug!("Injected declared opaque type: {name} -> {rust_path}");
        }
    }
}

/// Remove cfg-gated fields whose feature is not enabled in the source crate config.
pub(super) fn strip_cfg_fields(api: &mut ApiSurface, enabled_features: &[String]) {
    for typ in &mut api.types {
        let original_count = typ.fields.len();
        let cfg_count = typ.fields.iter().filter(|f| f.cfg.is_some()).count();
        typ.fields.retain(|f| match &f.cfg {
            None => true,
            Some(cfg_str) => cfg_condition_enabled(cfg_str, enabled_features),
        });
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
    let normalized: String = {
        let t = cfg_str.trim();

        t.replace(" (", "(")
    };
    let cfg_str = normalized.as_str();

    if let Some(feature) = cfg_str.strip_prefix("feature = \"").and_then(|s| s.strip_suffix('"')) {
        return enabled_features.iter().any(|ef| ef == feature);
    }
    if let Some(inner) = cfg_str.strip_prefix("any(").and_then(|s| s.strip_suffix(')')) {
        return parse_cfg_list(inner)
            .iter()
            .any(|cond| cfg_condition_enabled(cond, enabled_features));
    }
    if let Some(inner) = cfg_str.strip_prefix("all(").and_then(|s| s.strip_suffix(')')) {
        return parse_cfg_list(inner)
            .iter()
            .all(|cond| cfg_condition_enabled(cond, enabled_features));
    }
    if let Some(inner) = cfg_str.strip_prefix("not(").and_then(|s| s.strip_suffix(')')) {
        return !cfg_condition_enabled(inner.trim(), enabled_features);
    }
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
pub(super) fn dedup_api_surface(api: &mut ApiSurface) {
    let enum_names: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
    api.types.retain(|t| !enum_names.contains(&t.name));

    let error_names: AHashSet<String> = api.errors.iter().map(|e| e.name.clone()).collect();
    api.types.retain(|t| !error_names.contains(&t.name));

    // gates — a real re-export under `#[cfg(feature = "X")]` plus a stub fallback under
    // `#[cfg(any(not(feature = "X"), ...))]` (e.g. `LlmBackend` on the android-x86_64 emulator,
    {
        let mut best: AHashMap<String, usize> = AHashMap::new();
        let mut group_indices: AHashMap<String, Vec<usize>> = AHashMap::new();
        for (i, t) in api.types.iter().enumerate() {
            group_indices.entry(t.name.clone()).or_default().push(i);
            best.entry(t.name.clone())
                .and_modify(|prev_i| {
                    if api.types[i].rust_path.len() < api.types[*prev_i].rust_path.len() {
                        *prev_i = i;
                    }
                })
                .or_insert(i);
        }
        let merged_cfg_by_canonical: AHashMap<usize, Option<String>> = group_indices
            .iter()
            .filter(|(_, indices)| {
                indices.len() > 1 && {
                    let first = &api.types[indices[0]].cfg;
                    indices.iter().any(|&i| &api.types[i].cfg != first)
                }
            })
            .filter_map(|(name, indices)| {
                best.get(name).map(|&canonical| {
                    let merged =
                        crate::codegen::fn_dedup::merge_cfgs(indices.iter().map(|&i| api.types[i].cfg.as_deref()));
                    (canonical, merged)
                })
            })
            .collect();
        for (&canonical, merged) in &merged_cfg_by_canonical {
            api.types[canonical].cfg = merged.clone();
        }
        let keep: AHashSet<usize> = best.values().copied().collect();
        let mut idx = 0;
        api.types.retain(|_| {
            let k = keep.contains(&idx);
            idx += 1;
            k
        });
    }

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

    // (which synthesises a paired entry under a disjoint cfg for `#[cfg(X)] pub use mod::fn`
    {
        let mut best: AHashMap<(String, Option<String>), usize> = AHashMap::new();
        for (i, f) in api.functions.iter().enumerate() {
            best.entry((f.name.clone(), f.cfg.clone()))
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

    let mut seen_errors: AHashSet<String> = AHashSet::new();
    api.errors.retain(|e| seen_errors.insert(e.name.clone()));
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

/// Fill missing field `type_rust_path` values from the canonical `rust_path` of the
/// same-named type or enum in the (post-dedup) surface. Existing explicit field paths
/// are preserved because they may intentionally point at a public re-export or an
/// external source crate path.
pub(super) fn normalize_field_type_paths(api: &mut ApiSurface) {
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

    let fix = |fields: &mut Vec<FieldDef>| {
        for field in fields {
            if field.type_rust_path.is_none() {
                if let Some(name) = named_name(&field.ty) {
                    if let Some(path) = canonical.get(name) {
                        field.type_rust_path = Some(path.clone());
                    }
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
pub(super) fn apply_path_mappings(api: &mut ApiSurface, config: &ResolvedCrateConfig) {
    let mappings = config.effective_path_mappings();
    if mappings.is_empty() {
        return;
    }
    for typ in &mut api.types {
        if typ.original_rust_path.is_empty() {
            typ.original_rust_path = typ.rust_path.clone();
        }
        typ.rust_path = rewrite_path(&typ.rust_path, &mappings);
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
