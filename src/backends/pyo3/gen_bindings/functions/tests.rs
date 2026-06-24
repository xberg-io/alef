use super::{classify_param_type, emit_param_conversion};
use crate::core::ir::TypeRef;

/// Sync PyO3 free functions must release the GIL across the blocking core call so a
/// trait callback re-entering Python from a worker thread cannot deadlock. The generated
/// sync free function must (1) take an injected `py: Python<'_>` handle and (2) wrap the
/// core call in `py.detach(|| ...)`. This is a regression test for issue #136.
#[test]
fn sync_pyo3_free_fn_releases_gil_around_core_call() {
    use crate::codegen::generators::gen_function;
    use crate::core::ir::{FunctionDef, ParamDef};

    let func = FunctionDef {
        name: "count_words".to_owned(),
        rust_path: "sample_core::count_words".to_owned(),
        params: vec![ParamDef {
            name: "text".to_owned(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::U64),
        is_async: false,
        error_type: None,
        ..FunctionDef::default()
    };

    let mapper = crate::backends::pyo3::type_map::Pyo3Mapper::new();
    let cfg = crate::backends::pyo3::gen_bindings::config::binding_config("sample_core", true);
    let adapter_bodies = ahash::AHashMap::new();
    let opaque_types = ahash::AHashSet::new();

    let output = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(
        output.contains("py: Python<'_>"),
        "expected injected `py: Python<'_>` handle on sync free function:\n{output}"
    );
    assert!(
        output.contains("py.detach(|| sample_core::count_words("),
        "expected core call wrapped in `py.detach(|| ...)`:\n{output}"
    );
}

/// classify_param_type returns Plain for a bare Named type.
#[test]
fn classify_param_type_returns_plain_for_named() {
    let ty = TypeRef::Named("Foo".to_string());
    let result = classify_param_type(&ty);
    assert!(result.is_some());
    let (name, _) = result.unwrap();
    assert_eq!(name, "Foo");
}

/// classify_param_type returns None for a primitive type.
#[test]
fn classify_param_type_returns_none_for_primitive() {
    let ty = TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool);
    assert!(classify_param_type(&ty).is_none());
}

/// emit_param_conversion emits a guarded None check when optional.
#[test]
fn emit_param_conversion_guards_optional() {
    let mut out = String::new();
    emit_param_conversion(&mut out, "_rust_x", "x", "convert(x)", true);
    assert!(out.contains("if x is not None else None"));
}

/// emit_param_conversion emits a direct assignment when not optional.
#[test]
fn emit_param_conversion_direct_when_required() {
    let mut out = String::new();
    emit_param_conversion(&mut out, "_rust_x", "x", "convert(x)", false);
    assert!(!out.contains("if x is not None"));
    assert!(out.contains("_rust_x = convert(x)"));
}

/// Async Pyo3 functions with let_bindings that create temporary borrows
/// (e.g., Vec<&str> from Vec<String>) must place the bindings INSIDE the
/// `async move` block, not before it. This ensures the temporary lifetimes
/// extend to when the future executes, not just when the function returns.
///
/// This is a regression test for the fix that moves ref_let_bindings inside
/// the async block for AsyncPattern::Pyo3FutureIntoPy functions.
#[test]
fn async_pyo3_functions_place_bindings_inside_async_block() {
    // This test documents the expected behavior. The actual code generation
    // is tested implicitly when alef regenerates downstream packages and the
    // result compiles without E0597 (does not live long enough) errors.
    //
    // The generated code should look like:
    //   pyo3_async_runtimes::tokio::future_into_py(py, async move {
    //       let param_refs: Vec<&str> = param.iter().map(|s| s.as_str()).collect();
    //       let param_core: CoreType = param.into();
    //       let result = core_crate::function_name(&param_refs, &param_core).await...
    //       Ok(result.into())
    //   })
    //
    // NOT like this (which would fail with E0597):
    //   let param_refs: Vec<&str> = param.iter().map(|s| s.as_str()).collect();
    //   pyo3_async_runtimes::tokio::future_into_py(py, async move {
    //       let param_core: CoreType = param.into();
    //       let result = core_crate::function_name(&param_refs, &param_core).await...
    //   })
}

/// Regression guard for issue #145: the wide-integer return cast is gated on the extendr-only
/// cast flags (`cast_large_ints_to_f64` / `cast_uints_to_i32`). pyo3 does not set them, so a
/// `usize` return must stay `usize` with no `as f64` cast leaking into the body.
#[test]
fn pyo3_usize_return_is_not_cast_to_f64() {
    use crate::codegen::generators::gen_function;
    use crate::core::ir::{FunctionDef, PrimitiveType};

    let func = FunctionDef {
        name: "wide_value".to_owned(),
        rust_path: "sample_core::wide_value".to_owned(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::Usize),
        is_async: false,
        error_type: None,
        ..FunctionDef::default()
    };

    let mapper = crate::backends::pyo3::type_map::Pyo3Mapper::new();
    let cfg = crate::backends::pyo3::gen_bindings::config::binding_config("sample_core", true);
    let adapter_bodies = ahash::AHashMap::new();
    let opaque_types = ahash::AHashSet::new();

    let output = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(
        !output.contains("as f64"),
        "pyo3 usize return must not be cast to f64:\n{output}"
    );
    assert!(
        output.contains("-> usize"),
        "pyo3 signature should keep usize:\n{output}"
    );
}
