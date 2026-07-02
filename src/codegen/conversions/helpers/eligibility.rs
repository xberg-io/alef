use crate::codegen::conversions::helpers::type_discovery::field_references_excluded_type;
use crate::codegen::shared::binding_fields;
use crate::core::ir::{ApiSurface, EnumDef, FieldDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

/// Build the set of types that can have core→binding From safely generated.
/// More permissive than binding→core: allows sanitized fields (uses format!("{:?}"))
/// and accepts data enums (data discarded with `..` in match arms).
///
/// `excluded_field_types` lists type names that the calling backend excludes from
/// its binding surface (e.g. wasm `exclude_types`). Fields whose type appears in
/// this list are skipped in the binding struct AND the From impl, so they cannot
/// make a parent type non-convertible. Pass `&[]` from backends that have no
/// such exclusions.
pub fn core_to_binding_convertible_types(surface: &ApiSurface, excluded_field_types: &[String]) -> AHashSet<String> {
    let convertible_enums: AHashSet<&str> = surface
        .enums
        .iter()
        .filter(|e| can_generate_enum_conversion_from_core(e))
        .map(|e| e.name.as_str())
        .collect();

    let opaque_type_names: AHashSet<&str> = surface
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.as_str())
        .collect();

    // Data enums (enums with data variants) are generated as opaque wrappers in most
    // backends. They don't have From conversions but are still "known" types — structs
    // containing them can still generate core→binding conversions (the field uses the
    // opaque wrapper's From impl or format!("{:?}")).
    let data_enum_names: AHashSet<&str> = surface
        .enums
        .iter()
        .filter(|e| e.variants.iter().any(|v| !v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();

    // Build rust_path maps for detecting type_rust_path mismatches.
    let (enum_paths, type_paths) = build_rust_path_maps(surface);

    // All non-opaque types are candidates (sanitized fields use format!("{:?}"))
    let mut convertible: AHashSet<String> = surface
        .types
        .iter()
        .filter(|t| !t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    let mut changed = true;
    while changed {
        changed = false;
        let snapshot: Vec<String> = convertible.iter().cloned().collect();
        let mut known: AHashSet<&str> = convertible.iter().map(|s| s.as_str()).collect();
        known.extend(&opaque_type_names);
        known.extend(&data_enum_names);
        let mut to_remove = Vec::new();
        for type_name in &snapshot {
            if let Some(typ) = surface.types.iter().find(|t| t.name == *type_name) {
                let ok = binding_fields(&typ.fields).all(|f| {
                    if f.sanitized {
                        true
                    } else if !excluded_field_types.is_empty()
                        && field_references_excluded_type(&f.ty, excluded_field_types)
                    {
                        // Field is excluded from the binding surface by the backend;
                        // its type does not need to be convertible.
                        true
                    } else if field_has_path_mismatch(f, &enum_paths, &type_paths) {
                        false
                    } else {
                        is_field_convertible(&f.ty, &convertible_enums, &known)
                    }
                });
                if !ok {
                    to_remove.push(type_name.clone());
                }
            }
        }
        for name in to_remove {
            if convertible.remove(&name) {
                changed = true;
            }
        }
    }
    convertible
}

/// Build the set of types that can have binding→core From safely generated.
/// Strict: excludes types with sanitized fields (lossy conversion).
/// This is transitive: a type is convertible only if all its Named field types
/// are also convertible (or are enums with From/Into support).
pub fn convertible_types(surface: &ApiSurface) -> AHashSet<String> {
    // Build set of enums that have From/Into impls (unit-variant enums only)
    let convertible_enums: AHashSet<&str> = surface
        .enums
        .iter()
        .filter(|e| can_generate_enum_conversion(e))
        .map(|e| e.name.as_str())
        .collect();

    // Build set of all known type names (including opaques) — opaque Named fields
    // are convertible because we wrap/unwrap them via Arc.
    let _all_type_names: AHashSet<&str> = surface.types.iter().map(|t| t.name.as_str()).collect();

    // Build set of Named types that implement Default — sanitized fields referencing
    // Named types without Default would cause a compile error in the generated From impl.
    let default_type_names: AHashSet<&str> = surface
        .types
        .iter()
        .filter(|t| t.has_default)
        .map(|t| t.name.as_str())
        .collect();

    // Start with all non-opaque types as candidates.
    // Types with sanitized fields use Default::default() for the sanitized field
    // in the binding→core direction — but only if the field type implements Default.
    let mut convertible: AHashSet<String> = surface
        .types
        .iter()
        .filter(|t| !t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    // Set of opaque type names — Named fields referencing opaques are always convertible
    // (they use Arc wrap/unwrap), so include them in the known-types check.
    let opaque_type_names: AHashSet<&str> = surface
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.as_str())
        .collect();

    // Data enums (enums with data variants) are generated as opaque wrappers — include
    // them in the known set so structs containing them remain convertible.
    let data_enum_names: AHashSet<&str> = surface
        .enums
        .iter()
        .filter(|e| e.variants.iter().any(|v| !v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();

    // Build rust_path maps for detecting type_rust_path mismatches.
    let (enum_paths, type_paths) = build_rust_path_maps(surface);

    // Iteratively remove types whose fields reference non-convertible Named types.
    // We check against `convertible ∪ opaque_types ∪ data_enums` so that types
    // referencing excluded types are transitively removed, while opaque and data
    // enum Named fields remain valid.
    let mut changed = true;
    while changed {
        changed = false;
        let snapshot: Vec<String> = convertible.iter().cloned().collect();
        let mut known: AHashSet<&str> = convertible.iter().map(|s| s.as_str()).collect();
        known.extend(&opaque_type_names);
        known.extend(&data_enum_names);
        let mut to_remove = Vec::new();
        for type_name in &snapshot {
            if let Some(typ) = surface.types.iter().find(|t| t.name == *type_name) {
                let ok = binding_fields(&typ.fields).all(|f| {
                    if f.sanitized {
                        sanitized_field_has_default(&f.ty, &default_type_names)
                    } else if field_has_path_mismatch(f, &enum_paths, &type_paths) {
                        false
                    } else {
                        is_field_convertible(&f.ty, &convertible_enums, &known)
                    }
                });
                if !ok {
                    to_remove.push(type_name.clone());
                }
            }
        }
        for name in to_remove {
            if convertible.remove(&name) {
                changed = true;
            }
        }
    }
    convertible
}

/// Check if a sanitized field's type can produce a valid `Default::default()` expression.
/// Primitive types, strings, collections, Options, and Named types with `has_default` are fine.
/// Named types without `has_default` are not — generating `Default::default()` for them would
/// fail to compile.
fn sanitized_field_has_default(ty: &TypeRef, default_types: &AHashSet<&str>) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration
        | TypeRef::Json => true,
        // Option<T> defaults to None regardless of T
        TypeRef::Optional(_) => true,
        // Vec<T> defaults to empty vec regardless of T
        TypeRef::Vec(_) => true,
        // Map<K, V> defaults to empty map regardless of K/V
        TypeRef::Map(_, _) => true,
        TypeRef::Named(name) => {
            if is_tuple_type_name(name) {
                // Tuple types are always passthrough
                true
            } else {
                // Named type must have has_default to be safely used via Default::default()
                default_types.contains(name.as_str())
            }
        }
    }
}

/// Check if a specific type is in the convertible set.
pub fn can_generate_conversion(typ: &TypeDef, convertible: &AHashSet<String>) -> bool {
    convertible.contains(&typ.name)
}

pub(crate) fn is_field_convertible(
    ty: &TypeRef,
    convertible_enums: &AHashSet<&str>,
    known_types: &AHashSet<&str>,
) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration => true,
        TypeRef::Json => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_field_convertible(inner, convertible_enums, known_types),
        TypeRef::Map(k, v) => {
            is_field_convertible(k, convertible_enums, known_types)
                && is_field_convertible(v, convertible_enums, known_types)
        }
        // Tuple types are passthrough — always convertible
        TypeRef::Named(name) if is_tuple_type_name(name) => true,
        // Unit-variant enums and known types (including opaques, which use Arc wrap/unwrap) are convertible.
        TypeRef::Named(name) => convertible_enums.contains(name.as_str()) || known_types.contains(name.as_str()),
    }
}

/// Check if a field's `type_rust_path` is compatible with the known type/enum rust_paths.
///
/// When a struct field has a `type_rust_path` that differs from the `rust_path` of the
/// enum or type with the same short name, the `.into()` conversion will fail because
/// the `From` impl targets a different type. This detects such mismatches.
fn field_has_path_mismatch(
    field: &FieldDef,
    enum_rust_paths: &AHashMap<&str, &str>,
    type_rust_paths: &AHashMap<&str, &str>,
) -> bool {
    let name = match &field.ty {
        TypeRef::Named(n) => n.as_str(),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => n.as_str(),
            _ => return false,
        },
        _ => return false,
    };

    if let Some(field_path) = &field.type_rust_path {
        if let Some(enum_path) = enum_rust_paths.get(name) {
            if !paths_compatible(field_path, enum_path) {
                return true;
            }
        }
        if let Some(type_path) = type_rust_paths.get(name) {
            if !paths_compatible(field_path, type_path) {
                return true;
            }
        }
    }
    false
}

/// Check if two rust paths refer to the same type.
///
/// Handles re-exports: `crate::module::Type` and `crate::Type` are compatible
/// when they share the same crate root and type name (the type is re-exported).
fn paths_compatible(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    // Normalize dashes to underscores for crate name comparison
    // (Cargo uses dashes in package names, Rust uses underscores in crate names)
    let a_norm = a.replace('-', "_");
    let b_norm = b.replace('-', "_");
    if a_norm == b_norm {
        return true;
    }
    // Direct suffix match (e.g., "foo::Bar" ends_with "Bar")
    if a_norm.ends_with(&b_norm) || b_norm.ends_with(&a_norm) {
        return true;
    }
    // Same crate root + same short name → likely a re-export
    let a_root = a_norm.split("::").next().unwrap_or("");
    let b_root = b_norm.split("::").next().unwrap_or("");
    let a_name = a_norm.rsplit("::").next().unwrap_or("");
    let b_name = b_norm.rsplit("::").next().unwrap_or("");
    a_root == b_root && a_name == b_name
}

/// Build maps of name -> rust_path for enums and types in the API surface.
fn build_rust_path_maps(surface: &ApiSurface) -> (AHashMap<&str, &str>, AHashMap<&str, &str>) {
    let enum_paths: AHashMap<&str, &str> = surface
        .enums
        .iter()
        .map(|e| (e.name.as_str(), e.rust_path.as_str()))
        .collect();
    let type_paths: AHashMap<&str, &str> = surface
        .types
        .iter()
        .map(|t| (t.name.as_str(), t.rust_path.as_str()))
        .collect();
    (enum_paths, type_paths)
}

/// Check if an enum can have From/Into safely generated (both directions).
/// All enums are allowed — data variants use Default::default() for non-simple fields
/// in the binding→core direction.
pub fn can_generate_enum_conversion(enum_def: &EnumDef) -> bool {
    !enum_def.variants.is_empty()
}

/// Check if an enum can have core→binding From safely generated.
/// This is always possible: unit variants map 1:1, data variants discard data with `..`.
pub fn can_generate_enum_conversion_from_core(enum_def: &EnumDef) -> bool {
    // Always possible — data variants are handled by pattern matching with `..`
    !enum_def.variants.is_empty()
}

/// Returns true if fields represent a tuple variant (positional: _0, _1, ...).
pub fn is_tuple_variant(fields: &[FieldDef]) -> bool {
    !fields.is_empty()
        && fields[0]
            .name
            .strip_prefix('_')
            .is_some_and(|rest: &str| rest.chars().all(|c: char| c.is_ascii_digit()))
}

/// Returns true if a TypeDef represents a newtype struct (single unnamed field `_0`).
pub fn is_newtype(typ: &TypeDef) -> bool {
    typ.fields.len() == 1 && typ.fields[0].name == "_0"
}

/// Returns true if a type name looks like a tuple (starts with `(`).
/// Tuple types are passthrough — no conversion needed.
pub(crate) fn is_tuple_type_name(name: &str) -> bool {
    name.starts_with('(')
}

/// Check if a type has any sanitized fields (binding→core conversion is lossy).
pub fn has_sanitized_fields(typ: &TypeDef) -> bool {
    binding_fields(&typ.fields).any(|f| f.sanitized)
}
