use alef_core::ir::ApiSurface;
use std::collections::HashMap;

/// Build a lookup map from short type name to its fully-qualified Rust path.
///
/// Types in the IR carry a `rust_path` field containing the path as extracted from
/// the source file (e.g. `kreuzberg::extraction::docx::drawing::AnchorProperties`).
/// Backends that emit source-crate references must use this path rather than
/// naively constructing `{source_crate}::{name}`, which only works for types
/// re-exported at the crate root.
///
/// The lookup covers structs (`api.types`), enums (`api.enums`), and excluded types
/// (`api.excluded_type_paths`).  Excluded types are not part of the binding surface
/// (e.g. `InternalDocument`) but may still be referenced by trait method signatures;
/// including them here ensures their fully-qualified paths are available to backends
/// that generate trait bridge impls.
///
/// When `rust_path` is empty the entry is omitted; callers fall back to
/// `{source_crate}::{name}` for those cases.
pub fn build_type_path_lookup(api: &ApiSurface) -> HashMap<String, String> {
    let mut paths = HashMap::new();
    for ty in &api.types {
        if !ty.rust_path.is_empty() {
            paths.insert(ty.name.clone(), ty.rust_path.replace('-', "_"));
        }
    }
    for en in &api.enums {
        if !en.rust_path.is_empty() {
            paths.insert(en.name.clone(), en.rust_path.replace('-', "_"));
        }
    }
    // Include excluded types so trait bridge impls that reference them (e.g. `&InternalDocument`)
    // emit fully-qualified paths rather than bare type names.
    //
    // IMPORTANT: use `entry().or_insert()` rather than `insert()` so that a visible binding
    // type (already inserted from api.types/api.enums above) is never overwritten by an
    // excluded internal type with the same short name. Example: `Table` is a public type at
    // `kreuzberg::Table` *and* an excluded internal type at
    // `kreuzberg::extraction::docx::parser::Table`; the public path must win.
    for (name, path) in &api.excluded_type_paths {
        if !path.is_empty() {
            paths.entry(name.clone()).or_insert_with(|| path.replace('-', "_"));
        }
    }
    paths
}

/// Resolve the fully-qualified source-crate path for a named type.
///
/// If `name` is present in the lookup map the stored `rust_path` is returned.
/// Otherwise falls back to `"{source_crate}::{name}"` for types that are
/// available at the crate root (e.g. re-exported via `pub use`).
pub fn resolve_type_path(name: &str, source_crate: &str, type_paths: &HashMap<String, String>) -> String {
    match type_paths.get(name) {
        Some(path) => path.clone(),
        None => format!("{source_crate}::{name}"),
    }
}
