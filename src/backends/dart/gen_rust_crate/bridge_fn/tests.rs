use super::*;

fn make_param(name: &str, ty: TypeRef, is_ref: bool, is_mut: bool, optional: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref,
        is_mut,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

#[test]
fn is_mut_named_opaque_emits_mut_inner() {
    // Regression: opaque handle parameter with is_mut=true must produce &mut name.inner,
    // not &name.inner (which would fail when core fn takes &mut T).
    let p = make_param(
        "result",
        TypeRef::Named("ExtractionResult".to_string()),
        false,
        true,
        false,
    );
    let mut opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
    opaque.insert("ExtractionResult".to_string());
    let needs_from: std::collections::HashSet<String> = std::collections::HashSet::new();
    let type_paths: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    let expr = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque);
    assert_eq!(expr, "&mut result.inner", "is_mut opaque param must use &mut: {expr}");
}

#[test]
fn is_mut_named_from_emits_mut_borrow() {
    // Regression: Named param with types_needing_from_conversion and is_mut=true
    // must produce &mut CoreTy::from(name).
    let p = make_param(
        "cfg",
        TypeRef::Named("TranslationConfig".to_string()),
        false,
        true,
        false,
    );
    let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut needs_from: std::collections::HashSet<String> = std::collections::HashSet::new();
    needs_from.insert("TranslationConfig".to_string());
    let type_paths: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    let expr = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque);
    assert!(
        expr.contains("&mut"),
        "is_mut From-converted Named param must emit &mut borrow: {expr}"
    );
}

#[test]
fn is_mut_named_transmute_emits_mut_transmute() {
    // Regression: Named param with transmute path and is_mut=true must produce
    // a mutable transmute, not an immutable one.
    let p = make_param("config", TypeRef::Named("MyConfig".to_string()), false, true, false);
    let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
    let needs_from: std::collections::HashSet<String> = std::collections::HashSet::new();
    let type_paths: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    let expr = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque);
    assert!(
        expr.contains("&mut"),
        "is_mut transmute Named param must emit &mut transmute: {expr}"
    );
    assert!(
        expr.contains("transmute"),
        "is_mut transmute Named param must emit transmute: {expr}"
    );
}

#[test]
fn vec_named_is_ref_emits_slice_not_raw_pointer() {
    // Regression: Vec<Named> with is_ref=true must produce a &[CoreT] slice via
    // slice::from_raw_parts, not a raw *const CoreT pointer.
    let p = make_param(
        "categories",
        TypeRef::Vec(Box::new(TypeRef::Named("PiiCategory".to_string()))),
        true,
        false,
        false,
    );
    let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
    let needs_from: std::collections::HashSet<String> = std::collections::HashSet::new();
    let type_paths: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    let expr = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque);
    assert!(
        expr.contains("from_raw_parts"),
        "Vec<Named> is_ref must use slice::from_raw_parts, got: {expr}"
    );
    // The result must be a &[T] slice, not a bare *const pointer.
    // The transmute internally uses *const for type punning, but the outer
    // expression must be a slice via from_raw_parts.
    assert!(
        expr.contains(".len()"),
        "Vec<Named> is_ref must include .len() for slice bounds, got: {expr}"
    );
}

#[test]
fn collect_in_return_transmute_vec_has_type_annotation() {
    // Regression: collect() in Vec<Named> return conversion must use collect::<Vec<_>>()
    // so Rust can infer the target type without E0282.
    let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ty = TypeRef::Vec(Box::new(TypeRef::Named("QrCode".to_string())));
    let transform = return_transform(&ty, "mylib", &std::collections::HashMap::new(), &opaque, false);
    let suffix = match &transform {
        RetTransform::Suffix(s) => s.clone(),
        other => panic!("expected Suffix, got {other:?}"),
    };
    assert!(
        suffix.contains("collect::<Vec<_>>()"),
        "Vec<Named> collect must have type annotation: {suffix}"
    );
}

#[test]
fn vec_named_return_transform_is_suffix_not_closure_call() {
    // Regression (clippy 1.95 redundant_closure_call): Vec<Named> return MUST be
    // emitted as a suffix expression (`.into_iter().map(X::from).collect::<Vec<_>>()`)
    // applied directly to the call, NOT a closure literal wrapping the call site
    // (e.g. `(|v: Vec<_>| v.into_iter().map(X::from).collect::<Vec<_>>())(call)`).
    let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ty = TypeRef::Vec(Box::new(TypeRef::Named("QrCode".to_string())));
    let transform = return_transform(&ty, "mylib", &std::collections::HashMap::new(), &opaque, false);
    match &transform {
        RetTransform::Suffix(s) => {
            assert!(
                s.starts_with(".into_iter()"),
                "expected suffix starting with .into_iter(), got {s}"
            );
            assert!(s.contains("QrCode::from"), "expected QrCode::from in suffix, got {s}");
            assert!(!s.contains("|v"), "suffix must not contain a closure literal, got {s}");
        }
        other => panic!("expected Suffix, got {other:?}"),
    }
    let body = build_body("sample_crate::detect(&x)", "", &transform, false, false, false);
    assert!(
        !body.contains("|v: Vec<_>|"),
        "body must not emit closure-literal wrap: {body}"
    );
    assert!(
        body.contains("sample_crate::detect(&x).into_iter().map(QrCode::from).collect::<Vec<_>>()"),
        "body must apply suffix directly to call: {body}"
    );
}

#[test]
fn vec_named_returns_ref_emits_iter_not_into_iter() {
    // Regression (clippy 1.95 into_iter_on_ref): when the core fn returns &[T]
    // (e.g. `&'static [&'static str]`), the bridge must use `.iter()`, NOT
    // `.into_iter()` (which on &[T] is the same but triggers the lint).
    let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ty = TypeRef::Vec(Box::new(TypeRef::Named("Foo".to_string())));
    let transform = return_transform(&ty, "mylib", &std::collections::HashMap::new(), &opaque, true);
    match transform {
        RetTransform::Suffix(s) => {
            assert!(
                s.starts_with(".iter()"),
                "ref-return Vec<Named> must start with .iter(): {s}"
            );
            assert!(!s.contains(".into_iter()"), "ref-return must not use .into_iter(): {s}");
        }
        other => panic!("expected Suffix, got {other:?}"),
    }
}

#[test]
fn option_named_return_transform_is_suffix_not_closure_call() {
    // Regression: Option<Named> return must be emitted as `.map(X::from)` suffix,
    // not `(|v: Option<_>| v.map(X::from))(call)`.
    let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ty = TypeRef::Optional(Box::new(TypeRef::Named("EmbeddingPreset".to_string())));
    let transform = return_transform(&ty, "mylib", &std::collections::HashMap::new(), &opaque, false);
    match &transform {
        RetTransform::Suffix(s) => {
            assert_eq!(s, ".map(EmbeddingPreset::from)");
        }
        other => panic!("expected Suffix, got {other:?}"),
    }
    let body = build_body("sample_crate::get(&n)", "", &transform, false, false, false);
    assert!(
        !body.contains("|v: Option<_>|"),
        "body must not emit closure-literal wrap: {body}"
    );
    assert!(
        body.contains("sample_crate::get(&n).map(EmbeddingPreset::from)"),
        "body must apply suffix directly to call: {body}"
    );
}

#[test]
fn scalar_named_return_transform_does_not_emit_closure_call() {
    // Sync, no-error, scalar Named return: must NOT emit `(closure)(call)`.
    // Bare-path form: `MirrorName::from(call)`. Opaque form: `MirrorName { inner: call }`.
    let mut opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ty_named = TypeRef::Named("Foo".to_string());

    // From-conversion path.
    let t = return_transform(&ty_named, "mylib", &std::collections::HashMap::new(), &opaque, false);
    let body = build_body("sample_crate::foo()", "", &t, false, false, false);
    assert!(
        body.contains("Foo::from(sample_crate::foo())"),
        "sync scalar Named must emit direct call, got: {body}"
    );
    assert!(!body.contains("(Foo::from)("), "must not use (path)(expr) wrap: {body}");

    // Opaque-wrap path.
    opaque.insert("Foo".to_string());
    let t = return_transform(&ty_named, "mylib", &std::collections::HashMap::new(), &opaque, false);
    let body = build_body("sample_crate::foo()", "", &t, false, false, false);
    assert!(
        body.contains("Foo { inner: sample_crate::foo() }"),
        "sync scalar opaque Named must emit struct literal, got: {body}"
    );
    assert!(!body.contains("|inner|"), "must not emit closure: {body}");
}

#[test]
fn path_vec_result_cast_uses_to_string_lossy() {
    // Regression: Vec<Path> result cast must use .to_string_lossy().into_owned()
    // because PathBuf does not implement Display.
    let ty = TypeRef::Vec(Box::new(TypeRef::Path));
    let cast = build_primitive_result_cast(&ty, false);
    assert!(
        cast.contains("to_string_lossy"),
        "Vec<Path> cast must use to_string_lossy: {cast}"
    );
    assert!(
        !cast.contains(".to_string()"),
        "Vec<Path> must NOT use .to_string(): {cast}"
    );
}

#[test]
fn vec_string_returns_ref_result_cast_uses_iter_not_into_iter() {
    // Regression (clippy 1.95 into_iter_on_ref): when the core fn returns
    // `&'static [&'static str]` returned by a generated bridge function,
    // the result cast must use `.iter()` not `.into_iter()`. This is the
    // suffix path (no mirror transform), so `build_primitive_result_cast`
    // is responsible.
    let ty = TypeRef::Vec(Box::new(TypeRef::String));
    let cast_owned = build_primitive_result_cast(&ty, false);
    let cast_ref = build_primitive_result_cast(&ty, true);
    assert!(
        cast_owned.starts_with(".into_iter()"),
        "owned must use .into_iter(): {cast_owned}"
    );
    assert!(cast_ref.starts_with(".iter()"), "ref must use .iter(): {cast_ref}");
    assert!(
        !cast_ref.contains(".into_iter()"),
        "ref must not use .into_iter(): {cast_ref}"
    );
}

#[test]
fn scalar_path_result_cast_uses_display_not_to_string() {
    // Regression: scalar Path return cast must use .display().to_string()
    // not .to_string() because PathBuf does not implement Display.
    let ty = TypeRef::Path;
    let cast = build_primitive_result_cast(&ty, false);
    assert!(cast.contains("display()"), "Path cast must use .display(): {cast}");
    assert!(cast.contains("to_string()"), "Path cast must use to_string(): {cast}");
}

#[test]
fn unit_return_no_error_emits_statement_not_expression() {
    // Bug 1 fix: Unit return without error_type must emit statement (with semicolon),
    // not expression, to avoid implicit () return. This prevents clippy's `unused_unit` warning.
    let transform = RetTransform::None;
    let body_sync = build_body("sample_crate::clear()", "", &transform, false, false, true);
    let body_async = build_body("sample_crate::clear()", "", &transform, false, true, true);

    // Sync version should have semicolon.
    assert!(
        body_sync.contains("sample_crate::clear();"),
        "unit return without error must emit semicolon in sync fn: {body_sync}"
    );
    assert!(
        !body_sync.contains("sample_crate::clear()\n"),
        "unit return must NOT have semicolon-less expression: {body_sync}"
    );

    // Async version should have semicolon after await.
    assert!(
        body_async.contains("sample_crate::clear().await;"),
        "unit return without error must emit semicolon in async fn: {body_async}"
    );
}

#[test]
fn bool_to_bool_cast_skipped_redundant() {
    // Bug 2 fix: bool -> bool cast should be skipped (redundant cast warning).
    // build_primitive_result_cast must check if source and target types are identical.
    let ty = TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool);
    let cast = build_primitive_result_cast(&ty, false);

    // The cast should be empty because bool -> bool is redundant.
    assert_eq!(cast, "", "bool->bool cast must be empty (redundant), got: '{cast}'");
}

#[test]
fn non_matching_primitive_cast_preserved() {
    // Verify that casts for non-bool primitives are still emitted when needed.
    // In FRB, all integers map to i64 and floats map to f64.
    // A cast is redundant only when source and target types are identical.
    let ty_i64 = TypeRef::Primitive(crate::core::ir::PrimitiveType::I64);
    let cast_i64 = build_primitive_result_cast(&ty_i64, false);

    // i64 → i64 is redundant and should be empty.
    assert_eq!(
        cast_i64, "",
        "i64->i64 cast must be empty (redundant), got: '{cast_i64}'"
    );

    // For F64, FRB maps it to f64, so f64 -> f64 is redundant.
    let ty_f64 = TypeRef::Primitive(crate::core::ir::PrimitiveType::F64);
    let cast_f64 = build_primitive_result_cast(&ty_f64, false);
    assert_eq!(
        cast_f64, "",
        "f64->f64 cast must be empty (redundant), got: '{cast_f64}'"
    );
}
