use ahash::{AHashMap, AHashSet};
use alef_core::ir::{ApiSurface, EnumDef, FieldDef, PrimitiveType, TypeDef, TypeRef};

use crate::conversions::ConversionConfig;

/// Collect all Named type names that appear in the API surface — both as
/// function/method input parameters AND as function/method return types.
/// These are types that need binding→core `From` impls.
///
/// Return types need binding→core From impls because:
/// - Users may construct binding types and convert them to core types
/// - Generated code may use `.into()` on nested Named fields in From impls
/// - Round-trip conversion completeness ensures the API is fully usable
///
/// The result includes transitive dependencies: if `ConversionResult` is a
/// return type and it has a field `metadata: HtmlMetadata`, then `HtmlMetadata`
/// is also included.
pub fn input_type_names(surface: &ApiSurface) -> AHashSet<String> {
    let mut names = AHashSet::new();

    // Collect Named types from function params
    for func in &surface.functions {
        for param in &func.params {
            collect_named_types(&param.ty, &mut names);
        }
    }
    // Collect Named types from method params
    for typ in surface.types.iter().filter(|typ| !typ.is_trait) {
        for method in &typ.methods {
            for param in &method.params {
                collect_named_types(&param.ty, &mut names);
            }
        }
    }
    // Collect Named types from function return types.
    // Return types and their transitive field types need binding→core From impls
    // for round-trip conversion completeness.
    for func in &surface.functions {
        collect_named_types(&func.return_type, &mut names);
    }
    // Collect Named types from method return types.
    for typ in surface.types.iter().filter(|typ| !typ.is_trait) {
        for method in &typ.methods {
            collect_named_types(&method.return_type, &mut names);
        }
    }
    // Collect Named types from fields of non-opaque types that have methods.
    // When a non-opaque type has methods, codegen generates binding→core struct conversion
    // (gen_lossy_binding_to_core_fields) which calls `.into()` on Named fields.
    // Those field types need binding→core From impls.
    for typ in surface.types.iter().filter(|typ| !typ.is_trait) {
        if !typ.is_opaque && !typ.methods.is_empty() {
            for field in &typ.fields {
                if !field.sanitized {
                    collect_named_types(&field.ty, &mut names);
                }
            }
        }
    }

    // Collect Named types from enum variant fields — enum variants that are
    // used as input types need From impls for their nested Named fields too.
    for e in &surface.enums {
        if names.contains(&e.name) {
            for variant in &e.variants {
                for field in &variant.fields {
                    collect_named_types(&field.ty, &mut names);
                }
            }
        }
    }

    // Transitive closure: if type A is an input and has field of type B, B is also an input
    let mut changed = true;
    while changed {
        changed = false;
        let snapshot: Vec<String> = names.iter().cloned().collect();
        for name in &snapshot {
            if let Some(typ) = surface.types.iter().find(|t| t.name == *name) {
                for field in &typ.fields {
                    let mut field_names = AHashSet::new();
                    collect_named_types(&field.ty, &mut field_names);
                    for n in field_names {
                        if names.insert(n) {
                            changed = true;
                        }
                    }
                }
            }
            // Also walk enum variant fields in the transitive closure
            if let Some(e) = surface.enums.iter().find(|e| e.name == *name) {
                for variant in &e.variants {
                    for field in &variant.fields {
                        let mut field_names = AHashSet::new();
                        collect_named_types(&field.ty, &mut field_names);
                        for n in field_names {
                            if names.insert(n) {
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }

    names
}

/// Recursively collect all `Named(name)` from a TypeRef.
fn collect_named_types(ty: &TypeRef, out: &mut AHashSet<String>) {
    match ty {
        TypeRef::Named(name) => {
            out.insert(name.clone());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named_types(inner, out),
        TypeRef::Map(k, v) => {
            collect_named_types(k, out);
            collect_named_types(v, out);
        }
        _ => {}
    }
}

/// Check if a TypeRef references a Named type that is in the exclude list.
/// Used to skip fields whose types were excluded from binding generation,
/// preventing references to non-existent wrapper types (e.g. Js* in WASM).
pub fn field_references_excluded_type(ty: &TypeRef, exclude_types: &[String]) -> bool {
    match ty {
        TypeRef::Named(name) => exclude_types.iter().any(|e| e == name),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => field_references_excluded_type(inner, exclude_types),
        TypeRef::Map(k, v) => {
            field_references_excluded_type(k, exclude_types) || field_references_excluded_type(v, exclude_types)
        }
        _ => false,
    }
}

/// Returns true if a primitive type needs i64 casting (NAPI/PHP — JS/PHP lack native u64).
pub(crate) fn needs_i64_cast(p: &PrimitiveType) -> bool {
    matches!(p, PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize)
}

/// Returns true if a primitive type needs i32 casting (extendr — R maps small ints to i32).
pub(crate) fn needs_i32_cast(p: &PrimitiveType) -> bool {
    matches!(
        p,
        PrimitiveType::U8 | PrimitiveType::U16 | PrimitiveType::U32 | PrimitiveType::I8 | PrimitiveType::I16
    )
}

/// Returns true if a primitive type needs f64 casting (extendr — R maps large ints and f32 to f64).
///
/// Includes `I64` because the extendr `TypeMapper` maps `PrimitiveType::I64` to `"f64"` (R has
/// no native i64), so binding structs store `i64` fields as `f64`. Cast conversions must mirror
/// this mapping in both binding→core and core→binding From impls.
pub(crate) fn needs_f64_cast(p: &PrimitiveType) -> bool {
    matches!(
        p,
        PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize | PrimitiveType::F32
    )
}

/// Returns the core primitive type string for cast primitives.
pub(crate) fn core_prim_str(p: &PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::U64 => "u64",
        PrimitiveType::Usize => "usize",
        PrimitiveType::Isize => "isize",
        PrimitiveType::F32 => "f32",
        PrimitiveType::Bool => "bool",
        PrimitiveType::U8 => "u8",
        PrimitiveType::U16 => "u16",
        PrimitiveType::U32 => "u32",
        PrimitiveType::I8 => "i8",
        PrimitiveType::I16 => "i16",
        PrimitiveType::I32 => "i32",
        PrimitiveType::I64 => "i64",
        PrimitiveType::F64 => "f64",
    }
}

/// Returns the binding primitive type string for cast primitives (core→binding direction).
pub(crate) fn binding_prim_str(p: &PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize => "i64",
        PrimitiveType::F32 => "f64",
        PrimitiveType::Bool => "bool",
        PrimitiveType::U8 | PrimitiveType::U16 | PrimitiveType::U32 => "i32",
        PrimitiveType::I8 | PrimitiveType::I16 | PrimitiveType::I32 => "i32",
        PrimitiveType::I64 => "i64",
        PrimitiveType::F64 => "f64",
    }
}

/// Build the set of types that can have core→binding From safely generated.
/// More permissive than binding→core: allows sanitized fields (uses format!("{:?}"))
/// and accepts data enums (data discarded with `..` in match arms).
pub fn core_to_binding_convertible_types(surface: &ApiSurface) -> AHashSet<String> {
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
                let ok = typ.fields.iter().all(|f| {
                    if f.sanitized {
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
                let ok = typ.fields.iter().all(|f| {
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

/// Derive the Rust import path from rust_path, replacing hyphens with underscores.
///
/// Prefers `original_rust_path` (the path before auto_path_mappings rewriting)
/// so that `From` impls reference the actual defining crate, avoiding orphan
/// rule violations when `core_import` is a re-export facade.
pub fn core_type_path(typ: &TypeDef, core_import: &str) -> String {
    core_type_path_remapped(typ, core_import, &[])
}

/// Like [`core_type_path`] but rewrites the leading crate segment when it matches
/// a known source→override mapping.
///
/// When `core_crate_override` is set for a language, IR `rust_path` values still
/// contain the original source crate prefix (e.g. `mylib_core::Method`). The
/// `remaps` slice contains `(original_crate, override_crate)` pairs; when the
/// leading crate segment of `rust_path` matches `original_crate`, it is replaced
/// with `override_crate`.
pub fn core_type_path_remapped(typ: &TypeDef, core_import: &str, remaps: &[(&str, &str)]) -> String {
    // Always use rust_path (post path-mapping) — this is the import name that
    // binding crates can actually resolve. The original_rust_path preserves the
    // pre-mapping crate name (e.g. "html_to_markdown") which may not be importable
    // when the dependency is renamed (e.g. "html-to-markdown-rs" in Cargo.toml).
    let path = typ.rust_path.replace('-', "_");
    if path.contains("::") {
        apply_crate_remaps(&path, remaps)
    } else {
        format!("{core_import}::{}", typ.name)
    }
}

/// Apply source→override crate remaps to a fully-qualified Rust path.
///
/// If the leading crate segment of `path` (the part before the first `::`)
/// matches any entry in `remaps`, that segment is replaced with the override.
/// Returns `path` unchanged when no remap applies.
pub fn apply_crate_remaps(path: &str, remaps: &[(&str, &str)]) -> String {
    if remaps.is_empty() {
        return path.to_string();
    }
    if let Some(sep) = path.find("::") {
        let leading = &path[..sep];
        if let Some(&(_, override_crate)) = remaps.iter().find(|(orig, _)| *orig == leading) {
            return format!("{override_crate}{}", &path[sep..]);
        }
    }
    path.to_string()
}

/// Check if a type has any sanitized fields (binding→core conversion is lossy).
pub fn has_sanitized_fields(typ: &TypeDef) -> bool {
    typ.fields.iter().any(|f| f.sanitized)
}

/// Derive the Rust import path for an enum, replacing hyphens with underscores.
pub fn core_enum_path(enum_def: &EnumDef, core_import: &str) -> String {
    core_enum_path_remapped(enum_def, core_import, &[])
}

/// Like [`core_enum_path`] but rewrites the leading crate segment when it matches
/// a known source→override mapping. See [`core_type_path_remapped`] for details.
pub fn core_enum_path_remapped(enum_def: &EnumDef, core_import: &str, remaps: &[(&str, &str)]) -> String {
    let path = enum_def.rust_path.replace('-', "_");
    if path.starts_with(core_import) || path.contains("::") {
        // Path is already fully-qualified — apply remaps and return.
        apply_crate_remaps(&path, remaps)
    } else {
        // Bare unqualified name: prefix with the facade crate.
        format!("{core_import}::{}", enum_def.name)
    }
}

/// Build a map from type/enum short name to full rust_path.
///
/// Used by backends to resolve `TypeRef::Named(name)` to the correct qualified path
/// instead of assuming `core_import::name` (which fails for types not re-exported at crate root).
pub fn build_type_path_map(surface: &ApiSurface, core_import: &str) -> AHashMap<String, String> {
    let mut map = AHashMap::new();
    for typ in surface.types.iter().filter(|typ| !typ.is_trait) {
        let path = typ.rust_path.replace('-', "_");
        let resolved = if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", typ.name)
        };
        map.insert(typ.name.clone(), resolved);
    }
    for en in &surface.enums {
        let path = en.rust_path.replace('-', "_");
        let resolved = if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", en.name)
        };
        map.insert(en.name.clone(), resolved);
    }
    map
}

/// Resolve a `TypeRef::Named` short name to its full qualified path.
///
/// If the name is in the path map, returns the full path; otherwise falls back
/// to `core_import::name`.
pub fn resolve_named_path(name: &str, core_import: &str, path_map: &AHashMap<String, String>) -> String {
    if let Some(path) = path_map.get(name) {
        path.clone()
    } else {
        format!("{core_import}::{name}")
    }
}

/// Generate a match arm for binding -> core direction.
/// Binding enums are always unit-variant-only. Core enums may have data variants.
/// For data variants: `BindingEnum::Variant => CoreEnum::Variant(Default::default(), ...)`
pub fn binding_to_core_match_arm(binding_prefix: &str, variant_name: &str, fields: &[FieldDef]) -> String {
    binding_to_core_match_arm_ext(binding_prefix, variant_name, fields, false)
}

/// Like `binding_to_core_match_arm` but `binding_has_data` controls whether the binding
/// enum has the variant's fields (true) or is unit-only (false, e.g. Rustler/Elixir).
/// Generate match arm for binding->core conversion with config (handles type conversions).
pub fn binding_to_core_match_arm_ext_cfg(
    binding_prefix: &str,
    variant_name: &str,
    fields: &[FieldDef],
    binding_has_data: bool,
    config: &ConversionConfig,
) -> String {
    use super::binding_to_core::field_conversion_to_core_cfg;

    if fields.is_empty() {
        format!("{binding_prefix}::{variant_name} => Self::{variant_name},")
    } else if !binding_has_data {
        // Binding is unit-only: use Default for core fields
        if is_tuple_variant(fields) {
            let defaults: Vec<&str> = fields.iter().map(|_| "Default::default()").collect();
            format!(
                "{binding_prefix}::{variant_name} => Self::{variant_name}({}),",
                defaults.join(", ")
            )
        } else {
            let defaults: Vec<String> = fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            format!(
                "{binding_prefix}::{variant_name} => Self::{variant_name} {{ {} }},",
                defaults.join(", ")
            )
        }
    } else if is_tuple_variant(fields) {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let binding_pattern = field_names.join(", ");
        let core_args: Vec<String> = fields
            .iter()
            .map(|f| {
                let name = &f.name;
                // Sanitized fields: binding uses String, use serde_json to deserialize back.
                if f.sanitized {
                    let expr = if let TypeRef::Vec(_) = &f.ty {
                        // Vec<String> sanitized: each element is a debug-formatted value,
                        // parse each one individually.
                        format!("{name}.iter().filter_map(|s| serde_json::from_str(s).ok()).collect()")
                    } else {
                        format!("serde_json::from_str(&{name}).unwrap_or_default()")
                    };
                    return if f.is_boxed { format!("Box::new({expr})") } else { expr };
                }
                // Fields referencing excluded types: they appear as String in the binding.
                // Use serde_json deserialization to convert back to the core type.
                if !config.exclude_types.is_empty() && field_references_excluded_type(&f.ty, config.exclude_types) {
                    let expr = format!("serde_json::from_str(&{name}).unwrap_or_default()");
                    return if f.is_boxed { format!("Box::new({expr})") } else { expr };
                }
                // Use the conversion logic from field_conversion_to_core_cfg.
                // In an enum match arm, fields are bound by destructuring (not via `val.field`),
                // so replace `val.{name}` with just `{name}` in the generated expression.
                let conv = field_conversion_to_core_cfg(name, &f.ty, f.optional, config);
                // Extract the RHS from "name: expr" format
                let expr = if let Some(expr) = conv.strip_prefix(&format!("{name}: ")) {
                    let expr = expr.replace(&format!("val.{name}"), name);
                    expr.to_string()
                } else {
                    conv
                };
                if f.is_boxed { format!("Box::new({expr})") } else { expr }
            })
            .collect();
        format!(
            "{binding_prefix}::{variant_name} {{ {binding_pattern} }} => Self::{variant_name}({}),",
            core_args.join(", ")
        )
    } else {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let pattern = field_names.join(", ");
        let core_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                // Sanitized fields: the binding stores a simplified type (String for any complex type
                // like Vec<(String,String)>, Vec<Named>, etc.). Use serde_json to deserialize back.
                if f.sanitized {
                    // For Vec<String> sanitized (representing Vec<Tuple> like Vec<(String, String)>),
                    // each element is a debug-formatted string. Cannot safely use serde_json::from_str
                    // on Vec<String>. Default to empty collection.
                    if let TypeRef::Vec(_) = &f.ty {
                        return format!(
                            "{}: {}.iter().filter_map(|s| serde_json::from_str(s).ok()).collect()",
                            f.name, f.name
                        );
                    }
                    return format!("{}: serde_json::from_str(&{}).unwrap_or_default()", f.name, f.name);
                }
                // Use the conversion logic from field_conversion_to_core_cfg.
                // In an enum match arm, fields are bound by destructuring (not via `val.field`),
                // so replace `val.{name}` with just `{name}` in the generated expression.
                let conv = field_conversion_to_core_cfg(&f.name, &f.ty, f.optional, config);
                // Extract the RHS from "name: expr" format
                if let Some(expr) = conv.strip_prefix(&format!("{}: ", f.name)) {
                    let expr = expr.replace(&format!("val.{}", f.name), &f.name);
                    format!("{}: {}", f.name, expr)
                } else {
                    conv
                }
            })
            .collect();
        format!(
            "{binding_prefix}::{variant_name} {{ {pattern} }} => Self::{variant_name} {{ {} }},",
            core_fields.join(", ")
        )
    }
}

pub fn binding_to_core_match_arm_ext(
    binding_prefix: &str,
    variant_name: &str,
    fields: &[FieldDef],
    binding_has_data: bool,
) -> String {
    if fields.is_empty() {
        format!("{binding_prefix}::{variant_name} => Self::{variant_name},")
    } else if !binding_has_data {
        // Binding is unit-only: use Default for core fields
        if is_tuple_variant(fields) {
            let defaults: Vec<&str> = fields.iter().map(|_| "Default::default()").collect();
            format!(
                "{binding_prefix}::{variant_name} => Self::{variant_name}({}),",
                defaults.join(", ")
            )
        } else {
            let defaults: Vec<String> = fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            format!(
                "{binding_prefix}::{variant_name} => Self::{variant_name} {{ {} }},",
                defaults.join(", ")
            )
        }
    } else if is_tuple_variant(fields) {
        // Binding uses struct syntax with _0, _1 etc., core uses tuple syntax
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let binding_pattern = field_names.join(", ");
        // Wrap boxed fields with Box::new() and convert Named types with .into()
        let core_args: Vec<String> = fields
            .iter()
            .map(|f| {
                let name = &f.name;
                let expr = if matches!(&f.ty, TypeRef::Named(_)) {
                    format!("{name}.into()")
                } else if f.sanitized {
                    format!("serde_json::from_str(&{name}).unwrap_or_default()")
                } else {
                    name.clone()
                };
                if f.is_boxed { format!("Box::new({expr})") } else { expr }
            })
            .collect();
        format!(
            "{binding_prefix}::{variant_name} {{ {binding_pattern} }} => Self::{variant_name}({}),",
            core_args.join(", ")
        )
    } else {
        // Destructure binding named fields and pass to core, with .into() for Named types
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let pattern = field_names.join(", ");
        let core_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                if matches!(&f.ty, TypeRef::Named(_)) {
                    format!("{}: {}.into()", f.name, f.name)
                } else if f.sanitized {
                    // Sanitized fields have a simplified type in the binding (e.g. String)
                    // but the core type is complex (e.g. Vec<(String,String)>).
                    // Deserialize from JSON string for the binding→core conversion.
                    format!("{}: serde_json::from_str(&{}).unwrap_or_default()", f.name, f.name)
                } else {
                    format!("{0}: {0}", f.name)
                }
            })
            .collect();
        format!(
            "{binding_prefix}::{variant_name} {{ {pattern} }} => Self::{variant_name} {{ {} }},",
            core_fields.join(", ")
        )
    }
}

/// Generate a match arm for core -> binding direction.
/// When the binding also has data variants, destructure and forward fields.
/// When the binding is unit-variant-only, discard core data with `..`.
pub fn core_to_binding_match_arm(core_prefix: &str, variant_name: &str, fields: &[FieldDef]) -> String {
    core_to_binding_match_arm_ext(core_prefix, variant_name, fields, false)
}

/// Like `core_to_binding_match_arm` but `binding_has_data` controls whether the binding
/// enum has the variant's fields (true) or is unit-only (false).
/// Generate match arm for core->binding conversion with config (handles type conversions).
pub fn core_to_binding_match_arm_ext_cfg(
    core_prefix: &str,
    variant_name: &str,
    fields: &[FieldDef],
    binding_has_data: bool,
    config: &ConversionConfig,
) -> String {
    use super::core_to_binding::field_conversion_from_core_cfg;
    use ahash::AHashSet;

    if fields.is_empty() {
        format!("{core_prefix}::{variant_name} => Self::{variant_name},")
    } else if !binding_has_data {
        // Binding is unit-only: discard core data
        if is_tuple_variant(fields) {
            format!("{core_prefix}::{variant_name}(..) => Self::{variant_name},")
        } else {
            format!("{core_prefix}::{variant_name} {{ .. }} => Self::{variant_name},")
        }
    } else if is_tuple_variant(fields) {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let core_pattern = field_names.join(", ");
        let binding_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                // Use the conversion logic from field_conversion_from_core_cfg.
                // In an enum match arm, fields are bound by destructuring (not via `val.field`),
                // so replace `val.{name}` with just `{name}` in the generated expression.
                let conv =
                    field_conversion_from_core_cfg(&f.name, &f.ty, f.optional, f.sanitized, &AHashSet::new(), config);
                // Extract the RHS from "name: expr" format
                if let Some(expr) = conv.strip_prefix(&format!("{}: ", f.name)) {
                    let mut expr = expr.replace(&format!("val.{}", f.name), &f.name);
                    // Boxed fields in core tuple variants need dereferencing before conversion
                    if f.is_boxed {
                        expr = expr.replace(&format!("{}.into()", f.name), &format!("(*{}).into()", f.name));
                    }
                    format!("{}: {}", f.name, expr)
                } else {
                    conv
                }
            })
            .collect();
        format!(
            "{core_prefix}::{variant_name}({core_pattern}) => Self::{variant_name} {{ {} }},",
            binding_fields.join(", ")
        )
    } else {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let pattern = field_names.join(", ");
        let binding_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                // Use the conversion logic from field_conversion_from_core_cfg.
                // In an enum match arm, fields are bound by destructuring (not via `val.field`),
                // so replace `val.{name}` with just `{name}` in the generated expression.
                let conv =
                    field_conversion_from_core_cfg(&f.name, &f.ty, f.optional, f.sanitized, &AHashSet::new(), config);
                // Extract the RHS from "name: expr" format
                if let Some(expr) = conv.strip_prefix(&format!("{}: ", f.name)) {
                    let expr = expr.replace(&format!("val.{}", f.name), &f.name);
                    format!("{}: {}", f.name, expr)
                } else {
                    conv
                }
            })
            .collect();
        format!(
            "{core_prefix}::{variant_name} {{ {pattern} }} => Self::{variant_name} {{ {} }},",
            binding_fields.join(", ")
        )
    }
}

pub fn core_to_binding_match_arm_ext(
    core_prefix: &str,
    variant_name: &str,
    fields: &[FieldDef],
    binding_has_data: bool,
) -> String {
    if fields.is_empty() {
        format!("{core_prefix}::{variant_name} => Self::{variant_name},")
    } else if !binding_has_data {
        // Binding is unit-only: discard core data
        if is_tuple_variant(fields) {
            format!("{core_prefix}::{variant_name}(..) => Self::{variant_name},")
        } else {
            format!("{core_prefix}::{variant_name} {{ .. }} => Self::{variant_name},")
        }
    } else if is_tuple_variant(fields) {
        // Core uses tuple syntax, binding uses struct syntax with _0, _1 etc.
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let core_pattern = field_names.join(", ");
        // Unbox and convert Named types with .into()
        let binding_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                let name = &f.name;
                let expr = if f.is_boxed && matches!(&f.ty, TypeRef::Named(_)) {
                    format!("(*{name}).into()")
                } else if f.is_boxed {
                    format!("*{name}")
                } else if matches!(&f.ty, TypeRef::Named(_)) {
                    format!("{name}.into()")
                } else if f.sanitized {
                    format!("serde_json::to_string(&{name}).unwrap_or_default()")
                } else {
                    name.clone()
                };
                format!("{name}: {expr}")
            })
            .collect();
        format!(
            "{core_prefix}::{variant_name}({core_pattern}) => Self::{variant_name} {{ {} }},",
            binding_fields.join(", ")
        )
    } else {
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let pattern = field_names.join(", ");
        let binding_fields: Vec<String> = fields
            .iter()
            .map(|f| {
                if matches!(&f.ty, TypeRef::Named(_)) {
                    format!("{}: {}.into()", f.name, f.name)
                } else if f.sanitized {
                    // Sanitized fields have a simplified type in the binding (e.g. String)
                    // but the core type is complex (e.g. Vec<(String,String)>).
                    // Serialize to JSON string for the conversion.
                    format!("{}: serde_json::to_string(&{}).unwrap_or_default()", f.name, f.name)
                } else {
                    format!("{0}: {0}", f.name)
                }
            })
            .collect();
        format!(
            "{core_prefix}::{variant_name} {{ {pattern} }} => Self::{variant_name} {{ {} }},",
            binding_fields.join(", ")
        )
    }
}
