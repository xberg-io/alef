//! Regression tests for issue #145: extendr free functions returning wide integers
//! must cast the core-call result so the wrapper body matches the R-facing signature.
//!
//! The extendr mapper rewrites `usize`/`u64`/`i64`/`isize` → `f64` and `u8`/`u16`/`u32`
//! → `i32`. The return signature is rendered through that mapper, so without an explicit
//! cast the body (`Vec<usize>`) disagrees with the signature (`Vec<f64>`) and the generated
//! R crate fails to compile with E0308.
//!
//! Fixtures are deliberately neutral (no real downstream names). The pyo3 regression at the
//! bottom guards that backends NOT setting the cast flags emit unchanged output.

use super::ExtendrBackend;
use crate::codegen::generators::gen_function;
use crate::core::ir::{FunctionDef, PrimitiveType, TypeRef};

fn extendr_function(return_type: TypeRef) -> String {
    let func = FunctionDef {
        name: "wide_value".to_owned(),
        rust_path: "sample_core::wide_value".to_owned(),
        params: vec![],
        return_type,
        is_async: false,
        error_type: None,
        ..FunctionDef::default()
    };
    let backend = ExtendrBackend;
    let cfg = ExtendrBackend::binding_config("sample_core", &[]);
    let adapter_bodies = ahash::AHashMap::new();
    let opaque_types = ahash::AHashSet::new();
    gen_function(&func, &backend, &cfg, &adapter_bodies, &opaque_types)
}

#[test]
fn extendr_scalar_usize_return_casts_to_f64() {
    let out = extendr_function(TypeRef::Primitive(PrimitiveType::Usize));
    assert!(out.contains("-> f64"), "signature should map usize → f64:\n{out}");
    assert!(
        out.contains(") as f64"),
        "scalar return should be cast with `as f64`:\n{out}"
    );
}

#[test]
fn extendr_vec_usize_return_casts_each_element_to_f64() {
    let out = extendr_function(TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::Usize))));
    assert!(
        out.contains("-> Vec<f64>"),
        "signature should map Vec<usize> → Vec<f64>:\n{out}"
    );
    assert!(
        out.contains(".into_iter().map(|v| v as f64).collect::<Vec<f64>>()"),
        "Vec return should cast each element to f64:\n{out}"
    );
}

#[test]
fn extendr_option_usize_return_maps_cast_to_f64() {
    let out = extendr_function(TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Usize))));
    assert!(
        out.contains("-> Option<f64>"),
        "signature should map Option<usize> → Option<f64>:\n{out}"
    );
    assert!(
        out.contains(".map(|v| v as f64)"),
        "Option return should map-cast the inner value to f64:\n{out}"
    );
}

#[test]
fn extendr_option_vec_usize_return_maps_nested_cast_to_f64() {
    let out = extendr_function(TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Primitive(
        PrimitiveType::Usize,
    ))))));
    assert!(
        out.contains("-> Option<Vec<f64>>"),
        "signature should map Option<Vec<usize>> → Option<Vec<f64>>:\n{out}"
    );
    assert!(
        out.contains(".map(|xs| xs.into_iter().map(|v| v as f64).collect::<Vec<f64>>())"),
        "Option<Vec> return should map-cast each inner element to f64:\n{out}"
    );
}

#[test]
fn extendr_u32_return_casts_to_i32() {
    let out = extendr_function(TypeRef::Primitive(PrimitiveType::U32));
    assert!(out.contains("-> i32"), "signature should map u32 → i32:\n{out}");
    assert!(
        out.contains(") as i32"),
        "small-uint return should be cast with `as i32`:\n{out}"
    );
}
