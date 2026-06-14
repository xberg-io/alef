use super::reexports::{UseFilter, collect_use_names, find_crate_source, merge_surface, merge_surface_filtered};
use super::*;
use crate::core::ir::{PrimitiveType, ReceiverKind, TypeRef};

/// Helper: parse source and extract into an ApiSurface.
fn extract_from_source(source: &str) -> ApiSurface {
    let file = syn::parse_str::<syn::File>(source).expect("failed to parse test source");
    let mut surface = ApiSurface {
        crate_name: "test_crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let mut visited = Vec::new();
    let mut rwa = ahash::AHashSet::new();
    extract_items(
        &file.items,
        Path::new("test.rs"),
        "test_crate",
        "",
        &mut surface,
        None,
        &mut visited,
        &mut rwa,
    )
    .unwrap();
    resolve_newtypes(&mut surface);
    surface
}

mod defaults;
mod exclusions;
mod extraction_area;
mod futures_returns;
mod reexports;
mod serde;
mod unsupported_generics;
mod versioning;
