//! Same-name function entries must collapse to one documented entry per page.
//!
//! The extractor deliberately keeps two `FunctionDef`s for one public function when it is
//! reachable under two different `cfg` gates (a `#[cfg(...)] pub use` re-export paired with the
//! defining module's own gate). `src/codegen/fn_dedup.rs` documents that every *emitting*
//! consumer must collapse those groups locally. The docs generator renders one page section per
//! function, so it is an emitting consumer too: without the collapse it prints the same function
//! twice, byte-identical, under one `### Functions` heading.

use super::*;
use crate::docs::test_helpers::{make_function, make_param, make_test_config};

/// Two entries for one function, differing only in `cfg`, both surviving the docs cfg filter.
///
/// The gates are target predicates rather than feature gates so that
/// `ApiSurface::with_cfg_filtered_deep` keeps both entries (a non-feature leaf is indeterminate
/// and is conservatively kept) — that is what puts two same-name entries in front of the
/// emitter, which is the condition under test. ~keep
fn api_with_cfg_paired_function() -> ApiSurface {
    let make = |cfg: &str| {
        let mut func = make_function(
            "compute_total",
            vec![make_param("value", TypeRef::Primitive(PrimitiveType::U32), false)],
            TypeRef::Primitive(PrimitiveType::U32),
            false,
            None,
        );
        func.cfg = Some(cfg.to_string());
        func.doc = "Compute the total.".to_string();
        func
    };

    ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![make("target_family = \"unix\""), make("target_os = \"windows\"")],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn heading_occurrences(content: &str, heading: &str) -> usize {
    content.lines().filter(|line| line.trim_end() == heading).count()
}

#[test]
fn should_document_cfg_paired_function_once_on_rust_page() {
    let api = api_with_cfg_paired_function();
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Rust], "out").unwrap();
    let page = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-rust"))
        .expect("api-rust page must be generated");

    assert_eq!(
        heading_occurrences(&page.content, "#### compute_total()"),
        1,
        "a function reachable under two cfg gates must be documented once, not once per gate:\n{}",
        page.content
    );
}

#[test]
fn should_document_cfg_paired_function_once_on_binding_page() {
    let api = api_with_cfg_paired_function();
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let page = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .expect("api-python page must be generated");

    assert_eq!(
        heading_occurrences(&page.content, "#### compute_total()"),
        1,
        "the duplicate is not Rust-specific; every language page renders one entry per function:\n{}",
        page.content
    );
}
