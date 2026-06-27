use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EnumDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use anyhow::Context as _;
use std::path::Path;

pub(super) fn merge_external_type_roots(api: &mut ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<()> {
    let external_type_crates: Vec<_> = config
        .source_crates
        .iter()
        .filter(|source_crate| !source_crate.roots.is_empty())
        .collect();
    if external_type_crates.is_empty() {
        return Ok(());
    }

    let version = api.version.clone();
    let workspace_root = config.workspace_root.as_deref();

    for source_crate in external_type_crates {
        let crate_name = source_crate.name.replace('-', "_");
        let sources: Vec<&Path> = source_crate.sources.iter().map(std::path::PathBuf::as_path).collect();
        let external_api = crate::extract::extractor::extract(&sources, &crate_name, &version, workspace_root)
            .with_context(|| format!("failed to extract external type roots from crate {crate_name}"))?;

        let root_names = resolve_root_names(&external_api, &source_crate.roots, &crate_name)?;

        let needed = expand_external_dto_roots(&external_api, &root_names);
        let selected_types: Vec<TypeDef> = external_api
            .types
            .into_iter()
            .filter(|typ| needed.contains(&typ.name))
            .map(strip_external_methods)
            .collect();
        let selected_enums: Vec<EnumDef> = external_api
            .enums
            .into_iter()
            .filter(|enm| needed.contains(&enm.name))
            .collect();

        reject_conflicting_names(api, &selected_types, &selected_enums, &crate_name)?;
        api.types.extend(selected_types);
        api.enums.extend(selected_enums);
    }

    Ok(())
}

fn resolve_root_names(api: &ApiSurface, roots: &[String], crate_name: &str) -> anyhow::Result<Vec<String>> {
    let mut root_names = Vec::with_capacity(roots.len());
    for root in roots {
        let short_name = root.rsplit("::").next().unwrap_or(root);
        let found = if root.contains("::") {
            api.types
                .iter()
                .find(|typ| qualified_root_matches(root, short_name, &typ.rust_path))
                .map(|typ| typ.name.clone())
                .or_else(|| {
                    api.enums
                        .iter()
                        .find(|enm| qualified_root_matches(root, short_name, &enm.rust_path))
                        .map(|enm| enm.name.clone())
                })
        } else {
            let type_match = api.types.iter().find(|typ| typ.name == *short_name);
            let enum_match = api.enums.iter().find(|enm| enm.name == *short_name);
            type_match
                .map(|typ| typ.name.clone())
                .or_else(|| enum_match.map(|enm| enm.name.clone()))
        };

        if let Some(name) = found {
            root_names.push(name);
        } else {
            anyhow::bail!("external type root `{root}` was not found in crate `{crate_name}`");
        }
    }
    Ok(root_names)
}

fn qualified_root_matches(root: &str, short_name: &str, rust_path: &str) -> bool {
    if rust_path == root {
        return true;
    }

    let mut segments = root.split("::");
    let Some(root_crate) = segments.next() else {
        return false;
    };
    let Some(root_type) = segments.next() else {
        return false;
    };
    if segments.next().is_some() || root_type != short_name {
        return false;
    }

    rust_path
        .strip_prefix(root_crate)
        .is_some_and(|suffix| suffix.starts_with("::") && suffix.ends_with(&format!("::{short_name}")))
}

fn strip_external_methods(mut typ: TypeDef) -> TypeDef {
    typ.methods.clear();
    typ
}

fn expand_external_dto_roots(api: &ApiSurface, root_names: &[String]) -> AHashSet<String> {
    let all_types: AHashMap<String, &TypeDef> = api.types.iter().map(|typ| (typ.name.clone(), typ)).collect();
    let all_enums: AHashMap<String, &EnumDef> = api.enums.iter().map(|enm| (enm.name.clone(), enm)).collect();
    let mut needed: AHashSet<String> = root_names.iter().cloned().collect();
    let mut changed = true;

    while changed {
        changed = false;
        let current: Vec<String> = needed.iter().cloned().collect();
        for type_name in current {
            if let Some(typ) = all_types.get(&type_name) {
                for field in &typ.fields {
                    if field.binding_excluded {
                        continue;
                    }
                    collect_named_types(&field.ty, &all_types, &all_enums, &mut needed, &mut changed);
                }
            }
            if let Some(enm) = all_enums.get(&type_name) {
                for variant in &enm.variants {
                    if variant.binding_excluded {
                        continue;
                    }
                    for field in &variant.fields {
                        if field.binding_excluded {
                            continue;
                        }
                        collect_named_types(&field.ty, &all_types, &all_enums, &mut needed, &mut changed);
                    }
                }
            }
        }
    }

    needed
}

fn collect_named_types(
    ty: &TypeRef,
    all_types: &AHashMap<String, &TypeDef>,
    all_enums: &AHashMap<String, &EnumDef>,
    needed: &mut AHashSet<String>,
    changed: &mut bool,
) {
    match ty {
        TypeRef::Named(name)
            if (all_types.contains_key(name) || all_enums.contains_key(name)) && needed.insert(name.clone()) =>
        {
            *changed = true;
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => {
            collect_named_types(inner, all_types, all_enums, needed, changed);
        }
        TypeRef::Map(key, value) => {
            collect_named_types(key, all_types, all_enums, needed, changed);
            collect_named_types(value, all_types, all_enums, needed, changed);
        }
        _ => {}
    }
}

fn reject_conflicting_names(
    api: &ApiSurface,
    selected_types: &[TypeDef],
    selected_enums: &[EnumDef],
    crate_name: &str,
) -> anyhow::Result<()> {
    for external_type in selected_types {
        if let Some(existing_type) = api.types.iter().find(|typ| typ.name == external_type.name)
            && existing_type.rust_path != external_type.rust_path
        {
            anyhow::bail!(
                "external type `{}` from crate `{crate_name}` conflicts with existing type path `{}`",
                external_type.rust_path,
                existing_type.rust_path
            );
        }
        if let Some(existing_enum) = api.enums.iter().find(|enm| enm.name == external_type.name) {
            anyhow::bail!(
                "external type `{}` from crate `{crate_name}` conflicts with existing enum path `{}`",
                external_type.rust_path,
                existing_enum.rust_path
            );
        }
    }

    for external_enum in selected_enums {
        if let Some(existing_type) = api.types.iter().find(|typ| typ.name == external_enum.name) {
            anyhow::bail!(
                "external enum `{}` from crate `{crate_name}` conflicts with existing type path `{}`",
                external_enum.rust_path,
                existing_type.rust_path
            );
        }
        if let Some(existing_enum) = api.enums.iter().find(|enm| enm.name == external_enum.name)
            && existing_enum.rust_path != external_enum.rust_path
        {
            anyhow::bail!(
                "external enum `{}` from crate `{crate_name}` conflicts with existing enum path `{}`",
                external_enum.rust_path,
                existing_enum.rust_path
            );
        }
    }

    Ok(())
}
