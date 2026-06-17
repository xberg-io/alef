use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use ahash::{AHashMap, AHashSet};

/// Returns `true` if the type is a simple leaf type (primitive, String, Bytes, Path, etc.)
/// rather than a complex Named, collection, or Optional type.
fn is_simple_type(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Primitive(_)
            | TypeRef::String
            | TypeRef::Bytes
            | TypeRef::Path
            | TypeRef::Unit
            | TypeRef::Duration
            | TypeRef::Json
    )
}

/// Resolve newtype wrappers in the API surface.
///
/// Single-field tuple structs (`pub struct Foo(T)`) are identified by having exactly
/// one field named `_0`, no methods, and a simple inner type (primitive, String, etc.).
/// For each such newtype, all `TypeRef::Named("Foo")` references throughout the surface
/// are replaced with the inner type `T`, and the newtype TypeDef itself is removed.
/// This makes newtypes fully transparent to backends.
///
/// Tuple structs wrapping complex Named types (e.g., builders) are kept as-is.
pub(super) fn resolve_newtypes(surface: &mut ApiSurface) {
    // Build a map of newtype name → inner TypeRef.
    let newtype_map: AHashMap<String, TypeRef> = surface
        .types
        .iter()
        .filter(|t| t.fields.len() == 1 && t.fields[0].name == "_0" && is_simple_type(&t.fields[0].ty))
        .map(|t| (t.name.clone(), t.fields[0].ty.clone()))
        .collect();

    if newtype_map.is_empty() {
        return;
    }

    // Capture the full rust_path for each newtype before removing them.
    // This is needed by codegen to re-wrap resolved primitives when calling core methods.
    let newtype_rust_paths: AHashMap<String, String> = surface
        .types
        .iter()
        .filter(|t| newtype_map.contains_key(&t.name))
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .collect();

    // Remove newtype TypeDefs from the surface.
    surface.types.retain(|t| !newtype_map.contains_key(&t.name));

    // Walk all TypeRefs in the surface and replace Named references to newtypes.
    for typ in &mut surface.types {
        for field in &mut typ.fields {
            // Record the newtype wrapper path before resolving, so codegen can wrap/unwrap correctly.
            if let TypeRef::Named(name) = &field.ty {
                if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                    field.newtype_wrapper = Some(rust_path.clone());
                }
            }
            // Also handle Optional<NewtypeT> — record wrapper on the optional field
            if let TypeRef::Optional(inner) = &field.ty {
                if let TypeRef::Named(name) = inner.as_ref() {
                    if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                        field.newtype_wrapper = Some(rust_path.clone());
                    }
                }
            }
            // And Vec<NewtypeT>
            if let TypeRef::Vec(inner) = &field.ty {
                if let TypeRef::Named(name) = inner.as_ref() {
                    if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                        field.newtype_wrapper = Some(rust_path.clone());
                    }
                }
            }
            resolve_typeref(&newtype_map, &mut field.ty);
        }
        for method in &mut typ.methods {
            for param in &mut method.params {
                // Record the newtype wrapper path before resolving, so codegen can re-wrap when calling core.
                if let TypeRef::Named(name) = &param.ty {
                    if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                        param.newtype_wrapper = Some(rust_path.clone());
                    }
                }
                resolve_typeref(&newtype_map, &mut param.ty);
            }
            // Record return newtype wrapper before resolving — only for direct Named returns
            // (not Optional/Vec wrappers; those would require different unwrap patterns).
            if let TypeRef::Named(name) = &method.return_type {
                if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                    method.return_newtype_wrapper = Some(rust_path.clone());
                }
            }
            resolve_typeref(&newtype_map, &mut method.return_type);
        }
    }
    for func in &mut surface.functions {
        for param in &mut func.params {
            if let TypeRef::Named(name) = &param.ty {
                if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                    param.newtype_wrapper = Some(rust_path.clone());
                }
            }
            resolve_typeref(&newtype_map, &mut param.ty);
        }
        // Record return newtype wrapper for free functions too
        if let TypeRef::Named(name) = &func.return_type {
            if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                func.return_newtype_wrapper = Some(rust_path.clone());
            }
        }
        resolve_typeref(&newtype_map, &mut func.return_type);
    }
    for enum_def in &mut surface.enums {
        for variant in &mut enum_def.variants {
            for field in &mut variant.fields {
                resolve_typeref(&newtype_map, &mut field.ty);
            }
        }
    }
}

/// Recursively replace `TypeRef::Named(name)` with the newtype's inner type.
fn resolve_typeref(newtype_map: &AHashMap<String, TypeRef>, ty: &mut TypeRef) {
    match ty {
        TypeRef::Named(name) => {
            if let Some(inner) = newtype_map.get(name.as_str()) {
                *ty = inner.clone();
            }
        }
        TypeRef::Optional(inner) => resolve_typeref(newtype_map, inner),
        TypeRef::Vec(inner) => resolve_typeref(newtype_map, inner),
        TypeRef::Map(k, v) => {
            resolve_typeref(newtype_map, k);
            resolve_typeref(newtype_map, v);
        }
        _ => {}
    }
}

/// Resolve unresolved `trait_source` on methods after all source files have been processed.
///
/// When `impl Trait for Type` is encountered before the trait definition has been extracted
/// (e.g., `pub mod extractors` comes before `pub mod plugins` in lib.rs), the single-segment
/// trait name lookup fails because the trait `TypeDef` doesn't exist yet. This pass retroactively
/// resolves those methods by matching method names against trait types' method lists.
pub(super) fn resolve_trait_sources(surface: &mut ApiSurface) {
    // Build a map of trait method names -> trait rust_path for all known trait types.
    let mut trait_method_map: AHashMap<String, Vec<(String, String)>> = AHashMap::new();
    // Also build a map of trait_name -> set of method names, for disambiguation.
    let mut trait_methods_set: AHashMap<String, Vec<String>> = AHashMap::new();

    for typ in &surface.types {
        if !typ.is_trait {
            continue;
        }
        let method_names: Vec<String> = typ.methods.iter().map(|m| m.name.clone()).collect();
        trait_methods_set.insert(typ.name.clone(), method_names.clone());
        for method_name in &method_names {
            trait_method_map
                .entry(method_name.clone())
                .or_default()
                .push((typ.name.clone(), typ.rust_path.replace('-', "_")));
        }
    }

    if trait_method_map.is_empty() {
        return;
    }

    // For each non-trait type, collect unresolved method names first, then resolve.
    for typ in &mut surface.types {
        if typ.is_trait {
            continue;
        }

        // Collect the names of all unresolved methods on this type (for disambiguation).
        let unresolved_names: Vec<String> = typ
            .methods
            .iter()
            .filter(|m| m.trait_source.is_none())
            .map(|m| m.name.clone())
            .collect();

        for method in &mut typ.methods {
            if method.trait_source.is_some() {
                continue;
            }
            let Some(candidates) = trait_method_map.get(&method.name) else {
                continue;
            };

            if candidates.len() == 1 {
                method.trait_source = Some(candidates[0].1.clone());
            } else {
                // Pick the trait whose method set has the most overlap with this type's unresolved methods.
                let best = candidates.iter().max_by_key(|(trait_name, _)| {
                    trait_methods_set
                        .get(trait_name)
                        .map(|trait_methods| {
                            trait_methods
                                .iter()
                                .filter(|method_name| unresolved_names.contains(method_name))
                                .count()
                        })
                        .unwrap_or(0)
                });
                if let Some((_, rust_path)) = best {
                    method.trait_source = Some(rust_path.clone());
                }
            }
        }
    }
}

/// Merge duplicate function entries that share the same `name` but carry disjoint cfg gates.
///
/// The common pattern in Rust crates is:
///
/// ```rust,ignore
/// #[cfg(feature = "X")]
/// pub use real_mod::fn;                     // real implementation
///
/// #[cfg(all(feature = "X-presets", not(feature = "X")))]
/// pub fn fn(...) -> ... { Err(...) }        // stub / error fallback
/// ```
///
/// Both blocks are extracted into `surface.functions` as separate `FunctionDef`s with the same
/// `name`.  The FFI emitter picks whichever entry it encounters first and gates the emitted C
/// symbol under that entry's (narrow) cfg — so the symbol disappears whenever the other branch's
/// cfg is active.
///
/// This post-pass groups entries by `name`, and for each group with >1 distinct cfg value it:
///
/// 1. Selects the *canonical* entry — the one most likely to be the real implementation,
///    preferring entries whose parameter names do **not** all start with `_` (the stub
///    convention uses underscore-prefixed names like `_texts`, `_config`).
/// 2. Computes the OR of all cfgs in the group:
///    - If any entry has `cfg = None` (unconditional), the merged entry is also unconditional.
///    - Otherwise the merged cfg is `any(<a>, <b>, ...)`.
/// 3. Replaces the group with the single canonical entry bearing the merged cfg.
///
/// Groups where all entries already have identical cfgs, or where only one entry exists, are
/// left untouched.
pub(crate) fn merge_same_named_function_cfgs(surface: &mut ApiSurface) {
    let groups = collect_function_groups(&surface.functions);
    let groups_to_merge = groups_to_merge(&groups, &surface.functions);
    if groups_to_merge.is_empty() {
        return;
    }

    let mut canonical_by_first_index: AHashMap<usize, FunctionDef> = AHashMap::new();
    let mut skipped_indices: AHashSet<usize> = AHashSet::new();
    for indices in &groups_to_merge {
        let merged_cfg = merge_cfgs(indices.iter().map(|&i| surface.functions[i].cfg.as_deref()));
        let canonical_idx = pick_canonical_entry(indices, &surface.functions);
        let mut canonical = surface.functions[canonical_idx].clone();
        canonical.cfg = merged_cfg;

        let first_idx = *indices.iter().min().expect("merge group indices are non-empty");
        canonical_by_first_index.insert(first_idx, canonical);

        for &idx in indices {
            if idx != first_idx {
                skipped_indices.insert(idx);
            }
        }
    }

    let mut merged_functions = Vec::with_capacity(surface.functions.len() - skipped_indices.len());
    for (idx, function) in surface.functions.iter().cloned().enumerate() {
        if let Some(canonical) = canonical_by_first_index.remove(&idx) {
            merged_functions.push(canonical);
        } else if !skipped_indices.contains(&idx) {
            merged_functions.push(function);
        }
    }
    surface.functions = merged_functions;
}

fn collect_function_groups(functions: &[FunctionDef]) -> AHashMap<String, Vec<usize>> {
    let mut name_to_indices: AHashMap<String, Vec<usize>> = AHashMap::new();
    for (idx, func) in functions.iter().enumerate() {
        name_to_indices.entry(func.name.clone()).or_default().push(idx);
    }
    name_to_indices
}

fn groups_to_merge(groups: &AHashMap<String, Vec<usize>>, functions: &[FunctionDef]) -> Vec<Vec<usize>> {
    groups
        .values()
        .filter(|indices| should_merge_cfg_group(indices, functions))
        .cloned()
        .collect()
}

fn should_merge_cfg_group(indices: &[usize], functions: &[FunctionDef]) -> bool {
    if indices.len() <= 1 {
        return false;
    }
    let first_cfg = &functions[indices[0]].cfg;
    indices.iter().any(|&idx| &functions[idx].cfg != first_cfg)
}

/// Compute the OR-merge of a set of cfg strings.
///
/// - If any cfg is `None` (unconditional), returns `None`.
/// - If there is exactly one distinct value, returns it unchanged.
/// - Otherwise wraps all distinct values in `any(...)`.
fn merge_cfgs<'a>(cfgs: impl Iterator<Item = Option<&'a str>>) -> Option<String> {
    let mut distinct: Vec<&str> = Vec::new();
    for cfg in cfgs {
        match cfg {
            None => return None, // unconditional wins — no cfg gate at all
            Some(s) => {
                if !distinct.contains(&s) {
                    distinct.push(s);
                }
            }
        }
    }
    match distinct.len() {
        0 => None,
        1 => Some(distinct[0].to_string()),
        _ => Some(format!("any({})", distinct.join(", "))),
    }
}

/// Pick the index (within `surface.functions`) of the "canonical" (real) entry from a group.
///
/// Prefers an entry whose params are NOT all underscore-prefixed (the stub convention).
/// Falls back to the first entry in the group.
fn pick_canonical_entry(indices: &[usize], functions: &[FunctionDef]) -> usize {
    for &idx in indices {
        let func = &functions[idx];
        let all_underscore = !func.params.is_empty() && func.params.iter().all(|p| p.name.starts_with('_'));
        if !all_underscore {
            return idx;
        }
    }
    // All entries use underscore params (or have no params) — fall back to first.
    indices[0]
}
