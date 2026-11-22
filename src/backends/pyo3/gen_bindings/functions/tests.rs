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
fn async_pyo3_functions_place_bindings_inside_async_block() {}

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

/// Regression test for issue #380: a `&mut T` DTO parameter on a unit-returning sync function
/// previously rendered as `pub fn tag_record(record: Record) -> ()`, mutating a dropped `_core`
/// intermediate and leaving the caller's value untouched with no diagnostic. The binding must
/// instead return the mutated intermediate so the caller can observe the update.
#[test]
fn pyo3_mut_dto_param_returns_the_updated_value() {
    use crate::codegen::generators::gen_function;
    use crate::core::ir::{FunctionDef, ParamDef};

    let func = FunctionDef {
        name: "tag_record".to_owned(),
        rust_path: "sample_core::tag_record".to_owned(),
        params: vec![ParamDef {
            name: "record".to_owned(),
            ty: TypeRef::Named("Record".to_owned()),
            is_ref: true,
            is_mut: true,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
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
        output.contains("-> Record"),
        "expected the binding to return the mutated DTO type instead of `()`:\n{output}"
    );
    assert!(
        !output.contains("-> ()"),
        "must not still advertise a unit return:\n{output}"
    );
    // Load-bearing round-trip: the core call must still pass `&mut record_core` (the mutation
    // actually happens) AND the tail must hand back `record_core.into()` (the mutation is
    // actually observable). Asserting only the signature changed would pass on a binding that
    // mutates nothing, or on one that returns a fresh default value instead of the mutated one.
    assert!(
        output.contains("py.detach(|| sample_core::tag_record(&mut record_core))"),
        "expected the core call to still pass `&mut record_core`:\n{output}"
    );
    assert!(
        output.contains("record_core.into()"),
        "expected the mutated intermediate to be returned:\n{output}"
    );
}

/// Negative control for issue #380: an immutable `&T` DTO param must not gain write-back
/// semantics -- the return type must stay `()`.
#[test]
fn pyo3_immutable_dto_param_keeps_unit_return() {
    use crate::codegen::generators::gen_function;
    use crate::core::ir::{FunctionDef, ParamDef};

    let func = FunctionDef {
        name: "read_record".to_owned(),
        rust_path: "sample_core::read_record".to_owned(),
        params: vec![ParamDef {
            name: "record".to_owned(),
            ty: TypeRef::Named("Record".to_owned()),
            is_ref: true,
            is_mut: false,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
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
        output.contains("-> ()"),
        "immutable borrow must keep unit return:\n{output}"
    );
    assert!(
        !output.contains("record_core.into()"),
        "immutable borrow must not gain a write-back tail:\n{output}"
    );
}

/// Negative control for issue #380: an owned `T` DTO param (no `&mut`, no write-back candidate)
/// must render byte-for-byte the same as before the fix.
#[test]
fn pyo3_owned_dto_param_unaffected_by_writeback() {
    use crate::codegen::generators::gen_function;
    use crate::core::ir::{FunctionDef, ParamDef};

    let func = FunctionDef {
        name: "consume_record".to_owned(),
        rust_path: "sample_core::consume_record".to_owned(),
        params: vec![ParamDef {
            name: "record".to_owned(),
            ty: TypeRef::Named("Record".to_owned()),
            is_ref: false,
            is_mut: false,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        error_type: None,
        ..FunctionDef::default()
    };

    let mapper = crate::backends::pyo3::type_map::Pyo3Mapper::new();
    let cfg = crate::backends::pyo3::gen_bindings::config::binding_config("sample_core", true);
    let adapter_bodies = ahash::AHashMap::new();
    let opaque_types = ahash::AHashSet::new();

    let output = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(output.contains("-> ()"), "owned param must keep unit return:\n{output}");
    assert!(
        output.contains("py.detach(|| sample_core::consume_record(record_core))"),
        "owned param call must be unaffected:\n{output}"
    );
    assert!(
        !output.contains("record_core.into()"),
        "owned param must not gain a write-back tail:\n{output}"
    );
}

/// Regression test for issue #380 (async path): a `&mut T` DTO parameter on a unit-returning
/// `async fn` previously rendered as `pub fn tag_record_async<'py>(py: Python<'py>, record:
/// Record) -> PyResult<Bound<'py, PyAny>>` whose future body mutated a dropped `_core`
/// intermediate and then resolved to `Ok(())` -- the caller's value was left untouched with no
/// diagnostic. The binding must instead resolve to the mutated intermediate.
#[test]
fn pyo3_async_mut_dto_param_returns_the_updated_value() {
    use crate::codegen::generators::gen_function;
    use crate::core::ir::{FunctionDef, ParamDef};

    let func = FunctionDef {
        name: "tag_record_async".to_owned(),
        rust_path: "sample_core::tag_record_async".to_owned(),
        params: vec![ParamDef {
            name: "record".to_owned(),
            ty: TypeRef::Named("Record".to_owned()),
            is_ref: true,
            is_mut: true,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        is_async: true,
        error_type: None,
        ..FunctionDef::default()
    };

    let mapper = crate::backends::pyo3::type_map::Pyo3Mapper::new();
    let cfg = crate::backends::pyo3::gen_bindings::config::binding_config("sample_core", true);
    let adapter_bodies = ahash::AHashMap::new();
    let opaque_types = ahash::AHashSet::new();

    let output = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    // Load-bearing round-trip: the core call must still `.await` while passing
    // `&mut record_core`, AND the future must resolve to `Ok(record_core.into())`.
    assert!(
        output.contains("sample_core::tag_record_async(&mut record_core).await"),
        "expected the core call to still be awaited with `&mut record_core`:\n{output}"
    );
    assert!(
        output.contains("Ok(record_core.into())"),
        "expected the future to resolve to the mutated intermediate:\n{output}"
    );
    assert!(
        !output.contains("Ok(())"),
        "must not resolve the future to unit and drop the mutated value:\n{output}"
    );
}

/// Negative control: an async immutable `&T` DTO param must not gain write-back semantics.
#[test]
fn pyo3_async_immutable_dto_param_unaffected_by_writeback() {
    use crate::codegen::generators::gen_function;
    use crate::core::ir::{FunctionDef, ParamDef};

    let func = FunctionDef {
        name: "read_record_async".to_owned(),
        rust_path: "sample_core::read_record_async".to_owned(),
        params: vec![ParamDef {
            name: "record".to_owned(),
            ty: TypeRef::Named("Record".to_owned()),
            is_ref: true,
            is_mut: false,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        is_async: true,
        error_type: None,
        ..FunctionDef::default()
    };

    let mapper = crate::backends::pyo3::type_map::Pyo3Mapper::new();
    let cfg = crate::backends::pyo3::gen_bindings::config::binding_config("sample_core", true);
    let adapter_bodies = ahash::AHashMap::new();
    let opaque_types = ahash::AHashSet::new();

    let output = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(
        !output.contains("record_core.into()"),
        "immutable borrow must not gain a write-back tail:\n{output}"
    );
}

/// Negative control: an async owned `T` DTO param must render unaffected by write-back.
#[test]
fn pyo3_async_owned_dto_param_unaffected_by_writeback() {
    use crate::codegen::generators::gen_function;
    use crate::core::ir::{FunctionDef, ParamDef};

    let func = FunctionDef {
        name: "consume_record_async".to_owned(),
        rust_path: "sample_core::consume_record_async".to_owned(),
        params: vec![ParamDef {
            name: "record".to_owned(),
            ty: TypeRef::Named("Record".to_owned()),
            is_ref: false,
            is_mut: false,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        is_async: true,
        error_type: None,
        ..FunctionDef::default()
    };

    let mapper = crate::backends::pyo3::type_map::Pyo3Mapper::new();
    let cfg = crate::backends::pyo3::gen_bindings::config::binding_config("sample_core", true);
    let adapter_bodies = ahash::AHashMap::new();
    let opaque_types = ahash::AHashSet::new();

    let output = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(
        !output.contains("record_core.into()"),
        "owned param must not gain a write-back tail:\n{output}"
    );
}

/// Async write-back must also work when the core function returns `Result<(), E>`: the future
/// must map the `Ok(())` to `Ok(record_core.into())` rather than discarding it.
#[test]
fn pyo3_async_mut_dto_param_with_error_returns_the_updated_value() {
    use crate::codegen::generators::gen_function;
    use crate::core::ir::{FunctionDef, ParamDef};

    let func = FunctionDef {
        name: "tag_record_async".to_owned(),
        rust_path: "sample_core::tag_record_async".to_owned(),
        params: vec![ParamDef {
            name: "record".to_owned(),
            ty: TypeRef::Named("Record".to_owned()),
            is_ref: true,
            is_mut: true,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        is_async: true,
        error_type: Some("ProbeError".to_owned()),
        ..FunctionDef::default()
    };

    let mapper = crate::backends::pyo3::type_map::Pyo3Mapper::new();
    let cfg = crate::backends::pyo3::gen_bindings::config::binding_config("sample_core", true);
    let adapter_bodies = ahash::AHashMap::new();
    let opaque_types = ahash::AHashSet::new();

    let output = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(
        output.contains("-> PyResult<Bound<'py, PyAny>>"),
        "pyo3 async functions always declare a PyResult<Bound> outer signature:\n{output}"
    );
    assert!(
        output.contains("sample_core::tag_record_async(&mut record_core).await"),
        "expected the core call to still be awaited with `&mut record_core`:\n{output}"
    );
    assert!(
        output.contains("Ok(record_core.into())"),
        "expected the fallible future to resolve to the mutated intermediate on success:\n{output}"
    );
}

/// A Named param whose type is not re-exported at the core crate root must be named by its real
/// module path in the `let <param>_core: <path>` binding, not as `{core_import}::{Name}`.
#[test]
fn free_fn_let_binding_names_param_type_by_its_module_path() {
    use crate::codegen::generators::gen_function;
    use crate::core::ir::{FunctionDef, ParamDef};

    let func = FunctionDef {
        name: "append_annotation".to_owned(),
        rust_path: "sample_core::inner::write::append_annotation".to_owned(),
        params: vec![ParamDef {
            name: "annotation".to_owned(),
            ty: TypeRef::Named("AnnotationInput".to_owned()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        ..FunctionDef::default()
    };
    let type_paths = std::collections::HashMap::from([(
        "AnnotationInput".to_owned(),
        "sample_core::inner::write::AnnotationInput".to_owned(),
    )]);

    let mapper = crate::backends::pyo3::type_map::Pyo3Mapper::new();
    let mut cfg = crate::backends::pyo3::gen_bindings::config::binding_config("sample_core", true);
    cfg.type_paths = Some(&type_paths);
    let output = gen_function(&func, &mapper, &cfg, &ahash::AHashMap::new(), &ahash::AHashSet::new());

    assert!(
        output.contains("let annotation_core: sample_core::inner::write::AnnotationInput"),
        "{output}"
    );
    assert!(!output.contains("sample_core::AnnotationInput"), "{output}");
}
