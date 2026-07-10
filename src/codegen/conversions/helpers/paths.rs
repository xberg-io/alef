use crate::core::ir::{ApiSurface, EnumDef, TypeDef};
use ahash::AHashMap;

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

/// Derive the Rust import path for an enum, replacing hyphens with underscores.
pub fn core_enum_path(enum_def: &EnumDef, core_import: &str) -> String {
    core_enum_path_remapped(enum_def, core_import, &[])
}

/// Like [`core_enum_path`] but rewrites the leading crate segment when it matches
/// a known source→override mapping. See [`core_type_path_remapped`] for details.
pub fn core_enum_path_remapped(enum_def: &EnumDef, core_import: &str, remaps: &[(&str, &str)]) -> String {
    let path = enum_def.rust_path.replace('-', "_");
    if path.starts_with(core_import) || path.contains("::") {
        apply_crate_remaps(&path, remaps)
    } else {
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
