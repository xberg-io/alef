//! Regression coverage for capsule function return type mapping.
//!
//! Capsule functions return `usize` through swift-bridge so the Swift forwarder can reconstruct an
//! `OpaquePointer`. The zero sentinel avoids unsupported `Result<*ptr, _>` and `Option<*ptr>` shapes.

use super::emit_extern_block_for_functions;
use crate::core::config::HostCapsuleTypeConfig;
use crate::core::ir::{FunctionDef, TypeRef};
use std::collections::{HashMap, HashSet};

fn language_capsule() -> HostCapsuleTypeConfig {
    HostCapsuleTypeConfig {
        host_type: "SwiftTreeSitter.Language".to_string(),
        package: "https://github.com/tree-sitter/swift-tree-sitter".to_string(),
        package_version: "0.25.0".to_string(),
        construct_expr: "SwiftTreeSitter.Language({ptr})".to_string(),
    }
}

#[test]
fn capsule_function_returns_usize() {
    let mut capsule_types = HashMap::new();
    capsule_types.insert("Language".to_string(), language_capsule());

    let functions = vec![FunctionDef {
        name: "get_language".to_string(),
        params: vec![],
        return_type: TypeRef::Named("Language".to_string()),
        error_type: None,
        ..Default::default()
    }];

    let handle_returned = HashSet::new();
    let enum_names = HashSet::new();
    let deferred_empty = HashSet::new();

    let block = emit_extern_block_for_functions(
        &functions,
        &handle_returned,
        &enum_names,
        &deferred_empty,
        &capsule_types,
    );

    assert!(
        block.contains("fn get_language() -> usize"),
        "capsule function must return usize, got:\n{block}"
    );
    assert!(
        !block.contains("fn get_language() -> Language"),
        "capsule function should not return opaque handle Language:\n{block}"
    );
    assert!(
        !block.contains("fn get_language() -> OpaquePointer"),
        "capsule function should not return OpaquePointer:\n{block}"
    );
}

#[test]
fn fallible_capsule_function_returns_usize() {
    let mut capsule_types = HashMap::new();
    capsule_types.insert("Language".to_string(), language_capsule());

    let functions = vec![FunctionDef {
        name: "get_language".to_string(),
        params: vec![],
        return_type: TypeRef::Named("Language".to_string()),
        error_type: Some("String".to_string()),
        ..Default::default()
    }];

    let handle_returned = HashSet::new();
    let enum_names = HashSet::new();
    let deferred_empty = HashSet::new();

    let block = emit_extern_block_for_functions(
        &functions,
        &handle_returned,
        &enum_names,
        &deferred_empty,
        &capsule_types,
    );

    assert!(
        block.contains("fn get_language() -> usize"),
        "fallible capsule function must return usize, got:\n{block}"
    );
}

#[test]
fn non_capsule_function_unaffected() {
    let capsule_types = HashMap::new();

    let functions = vec![FunctionDef {
        name: "get_metadata".to_string(),
        params: vec![],
        return_type: TypeRef::Named("Metadata".to_string()),
        error_type: None,
        ..Default::default()
    }];

    let mut handle_returned = HashSet::new();
    handle_returned.insert("Metadata".to_string());
    let enum_names = HashSet::new();
    let deferred_empty = HashSet::new();

    let block = emit_extern_block_for_functions(
        &functions,
        &handle_returned,
        &enum_names,
        &deferred_empty,
        &capsule_types,
    );

    assert!(
        block.contains("fn get_metadata() -> Metadata"),
        "non-capsule handle-returning function must return Metadata, got:\n{block}"
    );
}
