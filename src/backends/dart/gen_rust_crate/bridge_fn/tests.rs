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
    assert!(
        expr.contains(".len()"),
        "Vec<Named> is_ref must include .len() for slice bounds, got: {expr}"
    );
}

#[test]
fn collect_in_return_transmute_vec_has_type_annotation() {
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
    let mut opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ty_named = TypeRef::Named("Foo".to_string());

    let t = return_transform(&ty_named, "mylib", &std::collections::HashMap::new(), &opaque, false);
    let body = build_body("sample_crate::foo()", "", &t, false, false, false);
    assert!(
        body.contains("Foo::from(sample_crate::foo())"),
        "sync scalar Named must emit direct call, got: {body}"
    );
    assert!(!body.contains("(Foo::from)("), "must not use (path)(expr) wrap: {body}");

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
    let ty = TypeRef::Path;
    let cast = build_primitive_result_cast(&ty, false);
    assert!(cast.contains("display()"), "Path cast must use .display(): {cast}");
    assert!(cast.contains("to_string()"), "Path cast must use to_string(): {cast}");
}

#[test]
fn unit_return_no_error_emits_statement_not_expression() {
    let transform = RetTransform::None;
    let body_sync = build_body("sample_crate::clear()", "", &transform, false, false, true);
    let body_async = build_body("sample_crate::clear()", "", &transform, false, true, true);

    assert!(
        body_sync.contains("sample_crate::clear();"),
        "unit return without error must emit semicolon in sync fn: {body_sync}"
    );
    assert!(
        !body_sync.contains("sample_crate::clear()\n"),
        "unit return must NOT have semicolon-less expression: {body_sync}"
    );

    assert!(
        body_async.contains("sample_crate::clear().await;"),
        "unit return without error must emit semicolon in async fn: {body_async}"
    );
}

#[test]
fn bool_to_bool_cast_skipped_redundant() {
    let ty = TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool);
    let cast = build_primitive_result_cast(&ty, false);

    assert_eq!(cast, "", "bool->bool cast must be empty (redundant), got: '{cast}'");
}

#[test]
fn non_matching_primitive_cast_preserved() {
    let ty_i64 = TypeRef::Primitive(crate::core::ir::PrimitiveType::I64);
    let cast_i64 = build_primitive_result_cast(&ty_i64, false);

    assert_eq!(
        cast_i64, "",
        "i64->i64 cast must be empty (redundant), got: '{cast_i64}'"
    );

    let ty_f64 = TypeRef::Primitive(crate::core::ir::PrimitiveType::F64);
    let cast_f64 = build_primitive_result_cast(&ty_f64, false);
    assert_eq!(
        cast_f64, "",
        "f64->f64 cast must be empty (redundant), got: '{cast_f64}'"
    );
}
