use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

pub(super) fn is_type_excluded(name: &str, rust_path: &str, exclude_list: &[String]) -> bool {
    exclude_list.iter().any(|entry| {
        if entry.contains("::") {
            let normalised = rust_path.replace('-', "_");
            normalised == entry.as_str()
        } else {
            name == entry.as_str()
        }
    })
}

/// Reason recorded on `binding_excluded` fields matched via `[crates.exclude].fields`,
/// mirroring the reason strings `alef(skip)`/`doc(hidden)` record for attribute-based exclusion.
const EXCLUDE_FIELDS_REASON: &str = "exclude.fields config";

/// Apply `[crates.exclude].fields` (`"TypeName.field_name"` entries) by marking matching
/// fields `binding_excluded`, using the exact same IR flag that `#[cfg_attr(alef, alef(skip))]`
/// sets on struct fields — every backend already filters on `binding_excluded`, so this makes
/// a globally-excluded field disappear from all bindings for free, without touching any
/// backend/language generator.
///
/// Matches both struct fields (`api.types`) and named enum variant fields (`api.enums`),
/// since attribute-based `alef(skip)` already supports exclusion on both (see
/// `extract_field` used by both `extract_field` call sites for struct and named-variant
/// fields). Malformed entries (not exactly one `.` splitting a non-empty type and field
/// name) are logged and skipped rather than panicking.
pub(super) fn apply_exclude_fields(api: &mut ApiSurface, fields: &[String]) {
    for entry in fields {
        let Some((type_name, field_name)) = entry.rsplit_once('.') else {
            tracing::warn!(entry = %entry, "exclude.fields entry must be \"TypeName.field_name\"; skipping");
            continue;
        };
        if type_name.is_empty() || field_name.is_empty() {
            tracing::warn!(entry = %entry, "exclude.fields entry must be \"TypeName.field_name\"; skipping");
            continue;
        }

        let mut matched = false;
        for typ in &mut api.types {
            if typ.name != type_name {
                continue;
            }
            for field in &mut typ.fields {
                if field.name == field_name {
                    field.binding_excluded = true;
                    field.binding_exclusion_reason = Some(EXCLUDE_FIELDS_REASON.to_string());
                    matched = true;
                }
            }
        }
        for enm in &mut api.enums {
            if enm.name != type_name {
                continue;
            }
            for variant in &mut enm.variants {
                for field in &mut variant.fields {
                    if field.name == field_name {
                        field.binding_excluded = true;
                        field.binding_exclusion_reason = Some(EXCLUDE_FIELDS_REASON.to_string());
                        matched = true;
                    }
                }
            }
        }

        if !matched {
            tracing::warn!(entry = %entry, "exclude.fields entry did not match any known type field");
        }
    }
}

pub(super) fn apply_filters(mut api: ApiSurface, config: &ResolvedCrateConfig) -> ApiSurface {
    let exclude = &config.exclude;
    let include = &config.include;

    let mut expanded_include: Option<AHashSet<String>> = None;
    if !include.types.is_empty() {
        let expanded = expand_include_list(&api, &include.types, &include.functions);
        api.types.retain(|t| expanded.contains(&t.name));
        api.enums.retain(|e| expanded.contains(&e.name));
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

    // attribute-based skip check (`#[alef::skip]`, `#[doc(hidden)]`) is necessarily
    api.unsupported_public_items.retain(|item| {
        let short_name = item.item_path.rsplit("::").next().unwrap_or(item.item_path.as_str());
        let by_type_name = is_type_excluded(short_name, &item.item_path, &exclude.types);
        let by_fn_name = item.item_kind == "function" && exclude.functions.contains(&short_name.to_string());
        let by_method_name = item.item_kind == "method" && exclude.methods.contains(&short_name.to_string());
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

    if !exclude.methods.is_empty() {
        for typ in &mut api.types {
            typ.methods.retain(|m| {
                let key = format!("{}.{}", typ.name, m.name);
                !exclude.methods.contains(&key)
            });
        }
        for service in &mut api.services {
            service.configurators.retain(|m| {
                let key = format!("{}.{}", service.name, m.name);
                !exclude.methods.contains(&key)
            });
        }
    }

    if !exclude.fields.is_empty() {
        apply_exclude_fields(&mut api, &exclude.fields);
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

    let all_types: AHashMap<String, &TypeDef> = api.types.iter().map(|t| (t.name.clone(), t)).collect();
    let all_enums: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

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
                    if field.binding_excluded {
                        continue;
                    }
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
