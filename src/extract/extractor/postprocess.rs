use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashMap;

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
    let newtype_map: AHashMap<String, TypeRef> = surface
        .types
        .iter()
        .filter(|t| t.fields.len() == 1 && t.fields[0].name == "_0" && is_simple_type(&t.fields[0].ty))
        .map(|t| (t.name.clone(), t.fields[0].ty.clone()))
        .collect();

    if newtype_map.is_empty() {
        return;
    }

    let newtype_rust_paths: AHashMap<String, String> = surface
        .types
        .iter()
        .filter(|t| newtype_map.contains_key(&t.name))
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .collect();

    surface.types.retain(|t| !newtype_map.contains_key(&t.name));

    for typ in &mut surface.types {
        for field in &mut typ.fields {
            if let TypeRef::Named(name) = &field.ty {
                if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                    field.newtype_wrapper = Some(rust_path.clone());
                }
            }
            if let TypeRef::Optional(inner) = &field.ty {
                if let TypeRef::Named(name) = inner.as_ref() {
                    if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                        field.newtype_wrapper = Some(rust_path.clone());
                    }
                }
            }
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
                if let TypeRef::Named(name) = &param.ty {
                    if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                        param.newtype_wrapper = Some(rust_path.clone());
                    }
                }
                resolve_typeref(&newtype_map, &mut param.ty);
            }
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
    let mut trait_method_map: AHashMap<String, Vec<(String, String)>> = AHashMap::new();
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

    for typ in &mut surface.types {
        if typ.is_trait {
            continue;
        }

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
