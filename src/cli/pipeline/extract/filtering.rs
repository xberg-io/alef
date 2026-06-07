use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

pub(super) fn is_type_excluded(name: &str, rust_path: &str, exclude_list: &[String]) -> bool {
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

pub(super) fn apply_filters(mut api: ApiSurface, config: &ResolvedCrateConfig) -> ApiSurface {
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
pub(super) fn expand_include_list(
    api: &ApiSurface,
    include_types: &[String],
    include_functions: &[String],
) -> AHashSet<String> {
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
