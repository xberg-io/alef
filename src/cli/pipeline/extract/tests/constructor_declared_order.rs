//! `[[crates]] constructor_declared_order` marks the named types on the extracted surface.

use super::*;

fn marked(surface: &ApiSurface, name: &str) -> bool {
    surface
        .types
        .iter()
        .find(|typ| typ.name == name)
        .unwrap_or_else(|| panic!("fixture type {name} missing"))
        .version
        .declared_constructor_order
}

fn run(entries: &[&str]) -> ApiSurface {
    let surface = surface_with(vec![make_typedef("Layout"), make_typedef("Other")], vec![]);
    let config = ResolvedCrateConfig {
        constructor_declared_order: entries.iter().map(|entry| (*entry).to_string()).collect(),
        ..Default::default()
    };
    apply_filters(surface, &config).expect("filters resolve")
}

#[test]
fn only_the_listed_type_is_marked() {
    let surface = run(&["Layout"]);

    assert!(marked(&surface, "Layout"));
    assert!(!marked(&surface, "Other"));
}

#[test]
fn a_qualified_path_entry_marks_the_type() {
    let surface = run(&["my_crate::Layout"]);

    assert!(marked(&surface, "Layout"));
    assert!(!marked(&surface, "Other"));
}

#[test]
fn the_wildcard_marks_every_type() {
    let surface = run(&["*"]);

    assert!(marked(&surface, "Layout") && marked(&surface, "Other"));
}

#[test]
fn an_empty_list_marks_nothing() {
    let surface = run(&[]);

    assert!(!marked(&surface, "Layout") && !marked(&surface, "Other"));
}

#[test]
fn an_unmatched_entry_marks_nothing() {
    let surface = run(&["Typo"]);

    assert!(!marked(&surface, "Layout") && !marked(&surface, "Other"));
}
