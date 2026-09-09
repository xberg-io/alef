//! A bytes-returning method must declare the out-param triple, never the string shape.
//!
//! The C# predicate matched bare `Bytes` only, so an `Optional<Bytes>` return fell through to the
//! string-returning template: six native parameters declared as three, and an `int32_t` status
//! declared pointer-width. The generated call then treated the small status as a pointer,
//! dereferenced it and freed it. Go could not reproduce it because cgo type-checks against the
//! real header; C# `DllImport` restates the signature by hand and compiles clean when wrong.
//!
//! These pin the C# declaration against the FFI's own rule, `TypeRef::returns_bytes_out_params`,
//! which is what emits the header. ~keep

use crate::backends::csharp::gen_bindings::pinvoke::{gen_pinvoke_for_func, gen_pinvoke_for_method};
use crate::core::ir::{FunctionDef, MethodDef, ReceiverKind, TypeRef};
use std::collections::{HashMap, HashSet};

fn bytes_function(return_type: TypeRef, fallible: bool) -> FunctionDef {
    FunctionDef {
        name: "sample_bytes".into(),
        rust_path: "sample_core::sample_bytes".into(),
        return_type,
        error_type: fallible.then(|| "SampleError".into()),
        ..Default::default()
    }
}

fn bytes_method(return_type: TypeRef, fallible: bool) -> MethodDef {
    MethodDef {
        name: "sample_bytes".into(),
        return_type,
        error_type: fallible.then(|| "SampleError".into()),
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    }
}

fn optional_bytes() -> TypeRef {
    TypeRef::Optional(Box::new(TypeRef::Bytes))
}

/// The IR rule is indifferent to the optional wrapper and to fallibility, because the C signature
/// is. Every backend answering this question must agree with it.
#[test]
fn the_ir_rule_covers_optional_and_infallible_bytes() {
    assert!(TypeRef::Bytes.returns_bytes_out_params());
    assert!(optional_bytes().returns_bytes_out_params());
    assert!(!TypeRef::String.returns_bytes_out_params());
    assert!(!TypeRef::Unit.returns_bytes_out_params());
    assert!(!TypeRef::Optional(Box::new(TypeRef::String)).returns_bytes_out_params());
}

/// `Optional<Bytes>` is the shape that produced the undefined behaviour: it must declare the
/// out-param triple exactly as bare `Bytes` does, not a pointer return.
#[test]
fn optional_bytes_declares_the_out_param_triple_like_bare_bytes() {
    for fallible in [false, true] {
        for return_type in [TypeRef::Bytes, optional_bytes()] {
            let declaration = gen_pinvoke_for_func(
                "sample_sample_bytes",
                &bytes_function(return_type.clone(), fallible),
                &HashSet::new(),
                &HashSet::new(),
                &HashMap::new(),
                &Default::default(),
            );
            for out_param in ["outPtr", "outLen", "outCap"] {
                assert!(
                    declaration.contains(out_param),
                    "bytes return {return_type:?} (fallible={fallible}) dropped {out_param}, so the \
                     managed declaration disagrees with the header:\n{declaration}"
                );
            }
            assert!(
                !declaration.contains("LPUTF8Str"),
                "bytes return {return_type:?} (fallible={fallible}) was rendered with the string \
                 template:\n{declaration}"
            );
        }
    }
}

/// Same for an instance method, which is the shape the consumer actually hit.
#[test]
fn optional_bytes_method_declares_the_out_param_triple() {
    for return_type in [TypeRef::Bytes, optional_bytes()] {
        let declaration = gen_pinvoke_for_method(
            "sample_registry_sample_bytes",
            "SampleBytes",
            &bytes_method(return_type.clone(), true),
            &HashMap::new(),
            &Default::default(),
        );
        for out_param in ["outPtr", "outLen", "outCap"] {
            assert!(
                declaration.contains(out_param),
                "bytes method {return_type:?} dropped {out_param}:\n{declaration}"
            );
        }
    }
}
