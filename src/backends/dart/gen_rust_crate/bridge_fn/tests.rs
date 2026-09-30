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

    let expr = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque, false);
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

    let expr = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque, false);
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

    let expr = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque, false);
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

    let expr = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque, false);
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

/// Build a sanitized-return function: the extractor collapsed an unmirrorable Named
/// return type to `String`, so `return_sanitized` is set (see
/// `cli::pipeline::extract::sanitizer`).
fn sanitized_fn(return_type: TypeRef, error_type: Option<&str>) -> FunctionDef {
    FunctionDef {
        name: "analyze_document".to_string(),
        rust_path: "sample_crate::analyze_document".to_string(),
        params: vec![make_param("path", TypeRef::String, false, false, false)],
        return_type,
        error_type: error_type.map(ToString::to_string),
        sanitized: true,
        return_sanitized: true,
        ..FunctionDef::default()
    }
}

fn emit(f: &FunctionDef) -> String {
    let mut out = String::new();
    emit_bridge_fn(
        &mut out,
        f,
        "sample_crate",
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
        &[],
    )
    .expect("emit_bridge_fn");
    out
}

#[test]
fn optional_u32_is_checked_in_fallible_bridge_and_preserves_zero() {
    let f = FunctionDef {
        name: "bounded".to_string(),
        rust_path: "sample_crate::bounded".to_string(),
        params: vec![make_param(
            "max",
            TypeRef::Primitive(PrimitiveType::U32),
            false,
            false,
            true,
        )],
        return_type: TypeRef::Unit,
        error_type: Some("Error".to_string()),
        ..FunctionDef::default()
    };

    let generated = emit(&f);

    assert!(generated.contains("-> Result<(), String>"), "{generated}");
    assert!(generated.contains("max.map(|v| u32::try_from(v)"), "{generated}");
    assert!(generated.contains(".transpose()?"), "{generated}");
    assert!(
        !generated.contains("filter(|"),
        "Some(0) must not be treated as absent: {generated}"
    );
    assert!(generated.contains(".to_string()"), "{generated}");
    assert!(!generated.contains("format!("), "{generated}");
}

#[test]
fn dto_unsigned_offsets_are_checked_before_fallible_core_call() {
    let f = FunctionDef {
        name: "redact".to_string(),
        rust_path: "sample_crate::redact".to_string(),
        params: vec![make_param(
            "findings",
            TypeRef::Vec(Box::new(TypeRef::Named("Finding".to_string()))),
            false,
            false,
            false,
        )],
        return_type: TypeRef::Unit,
        error_type: Some("Error".to_string()),
        ..FunctionDef::default()
    };
    let finding = TypeDef {
        name: "Finding".to_string(),
        fields: vec![crate::core::ir::FieldDef {
            name: "start".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::Usize),
            optional: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut generated = String::new();
    emit_bridge_fn(
        &mut generated,
        &f,
        "sample_crate",
        &std::collections::HashMap::new(),
        &std::collections::HashSet::from(["Finding".to_string()]),
        &std::collections::HashSet::new(),
        &[],
        &[finding],
    )
    .expect("emit bridge");

    assert!(generated.contains("-> Result<(), String>"), "{generated}");
    assert!(generated.contains("if let Some(value) = item.start"), "{generated}");
    assert!(generated.contains("usize::try_from(value)"), "{generated}");
    assert!(
        generated.contains("Finding.start is outside the valid usize range"),
        "{generated}"
    );
    assert!(generated.contains(".to_string())?"), "{generated}");
    assert!(!generated.contains("format!("), "{generated}");
}

#[test]
fn unsigned_dto_does_not_change_an_infallible_public_signature() {
    let f = FunctionDef {
        name: "rank".to_string(),
        rust_path: "sample_crate::rank".to_string(),
        params: vec![make_param(
            "matches",
            TypeRef::Vec(Box::new(TypeRef::Named("Match".to_string()))),
            false,
            false,
            false,
        )],
        return_type: TypeRef::Unit,
        ..FunctionDef::default()
    };
    let match_type = TypeDef {
        name: "Match".to_string(),
        fields: vec![crate::core::ir::FieldDef {
            name: "offset".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::Usize),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut generated = String::new();
    emit_bridge_fn(
        &mut generated,
        &f,
        "sample_crate",
        &std::collections::HashMap::new(),
        &std::collections::HashSet::from(["Match".to_string()]),
        &std::collections::HashSet::new(),
        &[],
        &[match_type],
    )
    .expect("emit bridge");

    assert!(!generated.contains("Result<"), "{generated}");
    assert!(!generated.contains("usize::try_from"), "{generated}");
}

#[test]
fn sanitized_string_return_emits_compile_error_naming_the_function() {
    let body = sanitized_return_body(
        &TypeRef::String,
        "analyze_document",
        false,
        &[make_param("path", TypeRef::String, false, false, false)],
    );

    assert!(
        body.contains("compile_error!"),
        "sanitized infallible return must emit compile_error!, got: {body}"
    );
    assert!(
        body.contains("analyze_document"),
        "compile_error! must name the offending function, got: {body}"
    );
    assert!(
        body.contains("dart.exclude_functions"),
        "compile_error! must name the escape hatch, got: {body}"
    );
    assert!(
        !body.contains("String::new()"),
        "fabricated empty-string default must not survive, got: {body}"
    );
}

/// The vacuous-pass case this fix targets: a sanitized `Option<T>` used to return
/// `None`, so a Dart fixture asserting `result == null` passed while the core function
/// was never called.
#[test]
fn sanitized_optional_return_no_longer_fabricates_none() {
    let body = sanitized_return_body(
        &TypeRef::Optional(Box::new(TypeRef::Named("Report".to_string()))),
        "analyze_document",
        false,
        &[],
    );

    assert!(
        body.contains("compile_error!"),
        "sanitized Option return must emit compile_error!, got: {body}"
    );
    assert!(
        !body.contains("None"),
        "fabricated `None` default must not survive, got: {body}"
    );
}

/// A sanitized `Vec<T>` used to return `Vec::new()`, so a fixture asserting "no results"
/// passed vacuously.
#[test]
fn sanitized_vec_return_no_longer_fabricates_empty_vec() {
    let body = sanitized_return_body(
        &TypeRef::Vec(Box::new(TypeRef::Named("Report".to_string()))),
        "analyze_document",
        false,
        &[],
    );

    assert!(
        body.contains("compile_error!"),
        "sanitized Vec return must emit compile_error!, got: {body}"
    );
    assert!(
        !body.contains("Vec::new()"),
        "fabricated empty-vec default must not survive, got: {body}"
    );
}

/// A fallible function keeps the generated crate compiling — the failure has a place to
/// live at runtime — but must be a real `Err` naming the function, never `Ok(default)`.
#[test]
fn sanitized_fallible_return_emits_err_not_ok_default() {
    let body = sanitized_return_body(
        &TypeRef::String,
        "analyze_document",
        true,
        &[make_param("path", TypeRef::String, false, false, false)],
    );

    assert!(
        body.contains("Err("),
        "sanitized fallible return must emit Err(..), got: {body}"
    );
    assert!(
        body.contains("analyze_document"),
        "Err message must name the offending function, got: {body}"
    );
    assert!(
        !body.contains("Ok("),
        "sanitized fallible return must not report success, got: {body}"
    );
    assert!(
        !body.contains("compile_error!"),
        "fallible branch must stay buildable rather than break the crate, got: {body}"
    );
}

/// A `()` return has no value to fabricate, so the void body stays legitimate.
#[test]
fn sanitized_unit_return_stays_void() {
    let body = sanitized_return_body(&TypeRef::Unit, "clear_cache", false, &[]);

    assert_eq!(body, "    ()\n", "unit return must stay void, got: {body}");
}

#[test]
fn emit_bridge_fn_sanitized_return_never_calls_the_core_function() {
    let rendered = emit(&sanitized_fn(TypeRef::String, None));

    assert!(
        rendered.contains("compile_error!"),
        "sanitized bridge fn must fail loudly, got:\n{rendered}"
    );
    assert!(
        rendered.contains("analyze_document"),
        "failure must name the offending function, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("String::new()"),
        "fabricated default must not reach the generated crate, got:\n{rendered}"
    );
}

/// The fallible bridge fn returns `Result<T, String>`, so the emitted `Err` must be a
/// `String` — the failure has to type-check against the signature this same function
/// emitted, not just read well.
#[test]
fn emit_bridge_fn_sanitized_fallible_return_emits_err_matching_its_signature() {
    let rendered = emit(&sanitized_fn(TypeRef::String, Some("SampleError")));

    assert!(
        rendered.contains("-> Result<String, String>"),
        "fallible sanitized fn must keep its Result signature, got:\n{rendered}"
    );
    assert!(
        rendered.contains(".to_string())"),
        "Err payload must be a String to match the signature, got:\n{rendered}"
    );
    assert!(
        rendered.contains("analyze_document"),
        "Err message must name the offending function, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("Ok("),
        "fallible sanitized fn must not report success, got:\n{rendered}"
    );
}

/// Positive control: a function whose return type was NOT sanitized still emits the real
/// call into the core crate.
#[test]
fn emit_bridge_fn_unsanitized_return_still_calls_the_core_function() {
    let mut f = sanitized_fn(TypeRef::String, None);
    f.sanitized = false;
    f.return_sanitized = false;
    let rendered = emit(&f);

    assert!(
        rendered.contains("sample_crate::analyze_document"),
        "unsanitized fn must call the core function, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("compile_error!"),
        "unsanitized fn must not fail, got:\n{rendered}"
    );
}

/// Positive control: the explicit, opt-in `dart.stub_methods` placeholder is untouched —
/// a user who genuinely wants a no-op function still has that switch, and it is checked
/// before the sanitized-return branch.
#[test]
fn emit_bridge_fn_configured_stub_method_still_emits_unimplemented() {
    let f = sanitized_fn(TypeRef::String, None);
    let mut out = String::new();
    emit_bridge_fn(
        &mut out,
        &f,
        "sample_crate",
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &["analyze_document".to_string()],
        &[],
    )
    .expect("emit_bridge_fn");

    assert!(
        out.contains("unimplemented!"),
        "configured stub method must keep its explicit placeholder, got:\n{out}"
    );
    assert!(
        !out.contains("compile_error!"),
        "configured stub method must not be treated as a sanitized-return failure, got:\n{out}"
    );
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

// --- `&mut T` DTO writeback coverage (alef issue #380) -------------------------------------
//
// `fn tag_record(record: &mut Record)` used to render as
// `pub fn tag_record(record: Record) { probe_lib::tag_record(&mut probe_lib::Record::from(record)); }`
// -- mutating a temporary rvalue that is dropped, so the caller's value never changes and
// nothing signals the failure. See `crate::codegen::mut_writeback`.

fn function_named(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("sample_crate::{name}"),
        params,
        return_type,
        ..FunctionDef::default()
    }
}

fn mut_record_param() -> ParamDef {
    make_param("record", TypeRef::Named("Record".to_string()), true, true, false)
}

#[test]
fn mut_dto_param_binds_a_core_intermediate_mutates_it_and_returns_the_mirror() {
    let f = function_named("tag_record", vec![mut_record_param()], TypeRef::Unit);
    let rendered = emit(&f);

    assert!(
        rendered.contains("pub fn tag_record(record: Record) -> Record {"),
        "writeback fn must return the mirror type instead of (), got:\n{rendered}"
    );
    assert!(
        rendered.contains("let mut record_core"),
        "writeback fn must bind a core-typed intermediate before mutating it, got:\n{rendered}"
    );
    assert!(
        rendered.contains("&mut record_core"),
        "writeback fn must pass the bound intermediate by &mut to the core call, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Record::from(record_core)") || rendered.contains("record_core"),
        "writeback fn must hand the mutated intermediate back as the mirror type, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("&mut probe_lib::Record::from(record)")
            && !rendered.contains("&mut sample_crate::Record::from(record)"),
        "the old rvalue-mutating form (mutates a temporary and drops it) must be gone, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("pub fn tag_record(record: Record) {\n    sample_crate::tag_record(&mut"),
        "must not still call the core fn with an inline &mut rvalue in a void-returning wrapper, got:\n{rendered}"
    );
}

#[test]
fn mut_dto_param_via_from_conversion_round_trips_through_the_core_type() {
    // Force the `types_needing_from_conversion` path (Dart's "sanitized field" conversion
    // branch) rather than the transmute path, matching a mirror type whose field types
    // differ from the core (e.g. u32 -> i64), which is the shape that actually reproduced
    // the original bug (the emitted rvalue-mutation form only appears on this path).
    let f = function_named("tag_record", vec![mut_record_param()], TypeRef::Unit);
    let mut out = String::new();
    let mut needs_from: std::collections::HashSet<String> = std::collections::HashSet::new();
    needs_from.insert("Record".to_string());
    emit_bridge_fn(
        &mut out,
        &f,
        "sample_crate",
        &std::collections::HashMap::new(),
        &needs_from,
        &std::collections::HashSet::new(),
        &[],
        &[],
    )
    .expect("emit_bridge_fn");

    assert!(
        out.contains("let mut record_core = sample_crate::Record::from(record);"),
        "From-path writeback must bind the core value via the mirror's From impl, got:\n{out}"
    );
    assert!(
        out.contains("sample_crate::tag_record(&mut record_core);"),
        "From-path writeback must call the core fn with &mut on the bound intermediate, got:\n{out}"
    );
    assert!(
        out.contains("Record::from(record_core)"),
        "From-path writeback must hand back Record::from(record_core), got:\n{out}"
    );
}

#[test]
fn immutable_borrow_dto_param_is_not_rewritten_as_writeback() {
    let p = make_param("record", TypeRef::Named("Record".to_string()), true, false, false);
    let f = function_named(
        "read_record",
        vec![p],
        TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
    );
    let rendered = emit(&f);

    assert!(
        !rendered.contains("-> Record"),
        "an immutable-borrow param must not gain a mirror-type return, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("record_core"),
        "an immutable-borrow param must not get a writeback core intermediate, got:\n{rendered}"
    );
}

#[test]
fn owned_dto_param_is_byte_for_byte_unchanged() {
    let p = make_param("record", TypeRef::Named("Record".to_string()), false, false, false);
    let f = function_named(
        "consume_record",
        vec![p],
        TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
    );
    let rendered = emit(&f);

    let expected = "pub fn consume_record(record: Record) -> i64 {\n    sample_crate::consume_record(unsafe { std::mem::transmute::<Record, sample_crate::Record>(record) }) as i64\n}\n";
    assert_eq!(
        rendered, expected,
        "an owned (by-value) DTO param must render exactly as before, got:\n{rendered}"
    );
}

#[test]
fn two_mut_dto_params_are_rejected_at_generation_time_naming_the_function() {
    let f = function_named(
        "tag_pair",
        vec![
            make_param("first", TypeRef::Named("Record".to_string()), true, true, false),
            make_param("second", TypeRef::Named("Record".to_string()), true, true, false),
        ],
        TypeRef::Unit,
    );
    let mut out = String::new();
    let error = emit_bridge_fn(
        &mut out,
        &f,
        "sample_crate",
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
        &[],
    )
    .expect_err("two `&mut` DTO params must be rejected at generation time");

    let message = error.to_string();
    assert!(
        message.contains("tag_pair"),
        "diagnostic must name the function: {message}"
    );
    assert!(
        out.is_empty(),
        "no partial output should be written on rejection, got:\n{out}"
    );
}

#[test]
fn mut_dto_param_plus_a_return_value_is_rejected_naming_the_function() {
    let f = function_named(
        "tag_and_count",
        vec![mut_record_param()],
        TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
    );
    let mut out = String::new();
    let error = emit_bridge_fn(
        &mut out,
        &f,
        "sample_crate",
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
        &[],
    )
    .expect_err("a `&mut` DTO param on a function that also returns a value must be rejected");

    let message = error.to_string();
    assert!(
        message.contains("tag_and_count"),
        "diagnostic must name the function: {message}"
    );
}

#[test]
fn mut_opaque_param_is_not_treated_as_writeback() {
    // Opaque types are excluded by `mut_writeback::is_writeback_param` -- the host holds a
    // live handle for those, so mutating through it (the pre-existing `&mut engine.inner`
    // behavior) is already correct and must be left alone.
    let p = make_param("engine", TypeRef::Named("Engine".to_string()), true, true, false);
    let f = function_named("bump_engine", vec![p], TypeRef::Unit);
    let mut out = String::new();
    let mut opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
    opaque.insert("Engine".to_string());
    emit_bridge_fn(
        &mut out,
        &f,
        "sample_crate",
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &opaque,
        &[],
        &[],
    )
    .expect("emit_bridge_fn");

    assert!(
        !out.contains("-> Engine"),
        "an opaque &mut param must not gain a writeback return, got:\n{out}"
    );
    assert!(
        out.contains("&mut engine.inner"),
        "an opaque &mut param must keep mutating through its live handle, got:\n{out}"
    );
}
