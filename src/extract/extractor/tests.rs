use super::reexports::{UseFilter, collect_use_names, find_crate_source, merge_surface, merge_surface_filtered};
use super::*;
use crate::core::ir::{PrimitiveType, ReceiverKind, TypeRef};

/// Helper: parse source and extract into an ApiSurface.
fn extract_from_source(source: &str) -> ApiSurface {
    // `Result` alias hints live in thread-local state, so a single-threaded test run would
    // otherwise carry one test's aliases into the next. ~keep
    type_resolver::reset_result_error_hints();
    let file = syn::parse_str::<syn::File>(source).expect("failed to parse test source");
    let mut surface = ApiSurface {
        crate_name: "test_crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let mut visited = Vec::new();
    let mut rwa = ahash::AHashSet::new();
    let mut pending_serde_defaults = SerdeDefaultsByType::default();
    extract_items(
        &file.items,
        Path::new("test.rs"),
        "test_crate",
        "",
        &mut surface,
        None,
        &mut visited,
        &mut rwa,
        &mut pending_serde_defaults,
    )
    .unwrap();
    resolve_public_default_functions(&mut surface);
    resolve_newtypes(&mut surface);
    resolve_enum_field_defaults(&mut surface);
    surface
}

mod cfg_test_gating;
mod defaults;
mod duplicate_items;
mod exclusions;
mod extraction_area;
mod futures_returns;
mod reexports;
mod result_alias;
mod serde;
mod std_traits;
mod unsupported_generics;
mod versioning;
