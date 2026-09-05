//! Regression coverage for `&mut T` DTO writeback extern declarations (alef issue #380).
//!
//! `fn tag_record(record: &mut Record)` used to declare `fn tag_record(record: Record) -> ();`
//! in the `extern "Rust"` block, matching a Rust bridge fn that mutated a temporary and
//! dropped it -- silently discarding the caller's update. See `crate::codegen::mut_writeback`.

use super::emit_extern_block_for_functions;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
use std::collections::{BTreeSet, HashMap, HashSet};

fn mut_dto_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        is_ref: true,
        is_mut: true,
        ..ParamDef::default()
    }
}

fn immutable_dto_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        is_ref: true,
        is_mut: false,
        ..ParamDef::default()
    }
}

#[test]
fn mut_dto_param_on_a_unit_returning_fn_declares_the_dto_return() {
    let functions = vec![FunctionDef {
        name: "tag_record".to_string(),
        params: vec![mut_dto_param("record", "Record")],
        return_type: TypeRef::Unit,
        error_type: None,
        ..Default::default()
    }];

    let handle_returned = HashSet::new();
    let enum_names = HashSet::new();
    let unit_enum_names: HashSet<&str> = HashSet::new();
    let deferred_empty = BTreeSet::new();
    let capsule_types = HashMap::new();
    let opaque_types = ahash::AHashSet::default();

    let block = emit_extern_block_for_functions(
        &functions,
        &handle_returned,
        &enum_names,
        &unit_enum_names,
        &deferred_empty,
        &capsule_types,
        &opaque_types,
    )
    .expect("emit_extern_block_for_functions");

    assert!(
        block.contains("fn tag_record(record: Record) -> Record;"),
        "writeback fn must declare its DTO param type as the return type, got:\n{block}"
    );
    assert!(
        !block.contains("fn tag_record(record: Record) -> ();"),
        "the old void-returning declaration (silently drops the mutation) must be gone, got:\n{block}"
    );
}

#[test]
fn immutable_borrow_dto_param_keeps_the_unit_return() {
    let functions = vec![FunctionDef {
        name: "read_record".to_string(),
        params: vec![immutable_dto_param("record", "Record")],
        return_type: TypeRef::Unit,
        error_type: None,
        ..Default::default()
    }];

    let handle_returned = HashSet::new();
    let enum_names = HashSet::new();
    let unit_enum_names: HashSet<&str> = HashSet::new();
    let deferred_empty = BTreeSet::new();
    let capsule_types = HashMap::new();
    let opaque_types = ahash::AHashSet::default();

    let block = emit_extern_block_for_functions(
        &functions,
        &handle_returned,
        &enum_names,
        &unit_enum_names,
        &deferred_empty,
        &capsule_types,
        &opaque_types,
    )
    .expect("emit_extern_block_for_functions");

    assert!(
        block.contains("fn read_record(record: Record) -> ();"),
        "an immutable-borrow param must not gain a writeback return, got:\n{block}"
    );
}

#[test]
fn mut_opaque_param_keeps_the_unit_return() {
    let functions = vec![FunctionDef {
        name: "bump_engine".to_string(),
        params: vec![mut_dto_param("engine", "Engine")],
        return_type: TypeRef::Unit,
        error_type: None,
        ..Default::default()
    }];

    let handle_returned = HashSet::new();
    let enum_names = HashSet::new();
    let unit_enum_names: HashSet<&str> = HashSet::new();
    let deferred_empty = BTreeSet::new();
    let capsule_types = HashMap::new();
    let mut opaque_types = ahash::AHashSet::default();
    opaque_types.insert("Engine".to_string());

    let block = emit_extern_block_for_functions(
        &functions,
        &handle_returned,
        &enum_names,
        &unit_enum_names,
        &deferred_empty,
        &capsule_types,
        &opaque_types,
    )
    .expect("emit_extern_block_for_functions");

    assert!(
        block.contains("fn bump_engine(engine: Engine) -> ();"),
        "an opaque &mut param must keep declaring a unit return, got:\n{block}"
    );
}

#[test]
fn two_mut_dto_params_are_rejected_naming_the_function() {
    let functions = vec![FunctionDef {
        name: "tag_pair".to_string(),
        params: vec![mut_dto_param("first", "Record"), mut_dto_param("second", "Record")],
        return_type: TypeRef::Unit,
        error_type: None,
        ..Default::default()
    }];

    let handle_returned = HashSet::new();
    let enum_names = HashSet::new();
    let unit_enum_names: HashSet<&str> = HashSet::new();
    let deferred_empty = BTreeSet::new();
    let capsule_types = HashMap::new();
    let opaque_types = ahash::AHashSet::default();

    let error = emit_extern_block_for_functions(
        &functions,
        &handle_returned,
        &enum_names,
        &unit_enum_names,
        &deferred_empty,
        &capsule_types,
        &opaque_types,
    )
    .expect_err("two `&mut` DTO params must be rejected at generation time");

    let message = error.to_string();
    assert!(
        message.contains("tag_pair"),
        "diagnostic must name the function: {message}"
    );
}

#[test]
fn mut_dto_param_plus_a_return_value_is_rejected_naming_the_function() {
    let functions = vec![FunctionDef {
        name: "tag_and_count".to_string(),
        params: vec![mut_dto_param("record", "Record")],
        return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
        error_type: None,
        ..Default::default()
    }];

    let handle_returned = HashSet::new();
    let enum_names = HashSet::new();
    let unit_enum_names: HashSet<&str> = HashSet::new();
    let deferred_empty = BTreeSet::new();
    let capsule_types = HashMap::new();
    let opaque_types = ahash::AHashSet::default();

    let error = emit_extern_block_for_functions(
        &functions,
        &handle_returned,
        &enum_names,
        &unit_enum_names,
        &deferred_empty,
        &capsule_types,
        &opaque_types,
    )
    .expect_err("a `&mut` DTO param on a function that also returns a value must be rejected");

    let message = error.to_string();
    assert!(
        message.contains("tag_and_count"),
        "diagnostic must name the function: {message}"
    );
}
