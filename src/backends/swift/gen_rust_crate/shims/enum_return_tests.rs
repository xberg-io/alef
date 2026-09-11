//! Regression coverage for a data-carrying enum in RETURN position.
//!
//! `emit_function_shim` decided how to wrap a `Named` return by asking whether the name was a
//! UNIT enum, and fell back to the tuple-struct constructor call `{t}(value)` when it was not.
//! For a data-carrying enum that call names an enum type as a function — `E0423` — because the
//! mirror type `enums::emit_enum_wrapper` declares for it is an `enum`, not the `pub struct
//! T(pub SourceT)` newtype `wrappers::constructors` declares for a struct. The discriminator has
//! to be enum-vs-struct: `impl From<core> for {t}` is emitted for every enum, unit or tagged.

use super::*;
use crate::core::ir::{FunctionDef, TypeRef};

fn shim_context<'a>(
    type_paths: &'a HashMap<String, String>,
    unit_enum_names: &'a HashSet<&'a str>,
    tagged_enum_names: &'a HashSet<&'a str>,
    empty_names: &'a HashSet<&'a str>,
    handle_returned_types: &'a HashSet<String>,
    capsule_types: &'a HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    opaque_types: &'a ahash::AHashSet<String>,
) -> FunctionShimContext<'a> {
    FunctionShimContext {
        source_crate: "sample_crate",
        type_paths,
        unit_enum_names,
        tagged_enum_names,
        no_serde_names: empty_names,
        handle_returned_types,
        capsule_types,
        opaque_types,
    }
}

fn function_returning(return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: "pick_routing".to_string(),
        rust_path: "sample_crate::pick_routing".to_string(),
        return_type,
        ..Default::default()
    }
}

#[test]
fn data_carrying_enum_return_is_converted_with_from_not_a_tuple_struct_call() {
    let f = function_returning(TypeRef::Named("Routing".to_string()));
    let type_paths = HashMap::from([("Routing".to_string(), "sample_crate::Routing".to_string())]);
    let unit_enum_names: HashSet<&str> = HashSet::new();
    let tagged_enum_names = HashSet::from(["Routing"]);
    let empty_names: HashSet<&str> = HashSet::new();
    let handle_returned_types = HashSet::from(["Routing".to_string()]);
    let capsule_types = HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &unit_enum_names,
        &tagged_enum_names,
        &empty_names,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");

    assert!(
        shim.contains("Routing::from("),
        "a data-carrying enum return must go through the mirror type's From impl, got:\n{shim}"
    );
    assert!(
        !shim.contains("Routing(sample_crate::pick_routing()"),
        "must not call the enum type as a tuple-struct constructor, got:\n{shim}"
    );
}

#[test]
fn unit_enum_return_keeps_using_from() {
    let f = function_returning(TypeRef::Named("Mode".to_string()));
    let type_paths = HashMap::from([("Mode".to_string(), "sample_crate::Mode".to_string())]);
    let unit_enum_names = HashSet::from(["Mode"]);
    let tagged_enum_names: HashSet<&str> = HashSet::new();
    let empty_names: HashSet<&str> = HashSet::new();
    let handle_returned_types = HashSet::from(["Mode".to_string()]);
    let capsule_types = HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &unit_enum_names,
        &tagged_enum_names,
        &empty_names,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");

    assert!(
        shim.contains("Mode::from("),
        "a unit enum return must still go through the mirror type's From impl, got:\n{shim}"
    );
}

/// The other half of the same decision: a non-enum `Named` return really is the tuple-struct
/// newtype `pub struct T(pub SourceT)`, which has no `From` impl, so it must keep the
/// constructor call. Making the wrap unconditional would break this case with `E0599`.
#[test]
fn struct_return_keeps_the_tuple_struct_constructor_call() {
    let f = function_returning(TypeRef::Named("Report".to_string()));
    let type_paths = HashMap::from([("Report".to_string(), "sample_crate::Report".to_string())]);
    let unit_enum_names: HashSet<&str> = HashSet::new();
    let tagged_enum_names: HashSet<&str> = HashSet::new();
    let empty_names: HashSet<&str> = HashSet::new();
    let handle_returned_types = HashSet::from(["Report".to_string()]);
    let capsule_types = HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &unit_enum_names,
        &tagged_enum_names,
        &empty_names,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");

    assert!(
        shim.contains("Report(sample_crate::pick_routing())"),
        "a struct newtype return must keep the tuple-struct constructor call, got:\n{shim}"
    );
    assert!(
        !shim.contains("Report::from("),
        "the struct newtype has no From impl to call, got:\n{shim}"
    );
}

/// The `Vec<Enum>` return shape bypassed both wrap closures entirely and inlined `{t}(x.clone())`,
/// so it emitted the tuple-struct call against an enum even for a UNIT enum.
#[test]
fn returned_ref_vec_of_enums_is_converted_with_from_for_each_element() {
    let mut f = function_returning(TypeRef::Vec(Box::new(TypeRef::Named("Routing".to_string()))));
    f.returns_ref = true;
    let type_paths = HashMap::from([("Routing".to_string(), "sample_crate::Routing".to_string())]);
    let unit_enum_names: HashSet<&str> = HashSet::new();
    let tagged_enum_names = HashSet::from(["Routing"]);
    let empty_names: HashSet<&str> = HashSet::new();
    let handle_returned_types = HashSet::from(["Routing".to_string()]);
    let capsule_types = HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let context = shim_context(
        &type_paths,
        &unit_enum_names,
        &tagged_enum_names,
        &empty_names,
        &handle_returned_types,
        &capsule_types,
        &opaque_types,
    );

    let shim = emit_function_shim(&f, &context).expect("emit_function_shim");

    assert!(
        shim.contains("Routing::from(x.clone())"),
        "each element of a by-reference Vec<Enum> return must go through From, got:\n{shim}"
    );
    assert!(
        !shim.contains("Routing(x.clone())"),
        "must not call the enum type as a tuple-struct constructor per element, got:\n{shim}"
    );
}
