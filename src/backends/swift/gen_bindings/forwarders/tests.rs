use super::*;
use crate::core::ir::{NewtypeWrapper, NewtypeWrapperMetadata, TypeRef};

#[test]
fn test_swift_type_name_bool_returns_bool() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::Bool)), "Bool");
}

#[test]
fn test_swift_type_name_usize_returns_uint() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::Usize)), "UInt");
}

#[test]
fn test_swift_type_name_u8_returns_uint8() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::U8)), "UInt8");
}

#[test]
fn test_swift_type_name_u32_returns_uint32() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::U32)), "UInt32");
}

#[test]
fn test_swift_type_name_u64_returns_uint64() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::U64)), "UInt64");
}

#[test]
fn test_swift_type_name_i32_returns_int32() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::I32)), "Int32");
}

#[test]
fn test_swift_type_name_f32_returns_float() {
    assert_eq!(swift_type_name(&TypeRef::Primitive(PrimitiveType::F32)), "Float");
}

fn make_function(name: &str, params: Vec<(&str, TypeRef)>, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("sample::{name}"),
        params: params
            .into_iter()
            .map(|(pname, ty)| crate::core::ir::ParamDef {
                name: pname.to_string(),
                ty,
                ..crate::core::ir::ParamDef::default()
            })
            .collect(),
        return_type,
        ..FunctionDef::default()
    }
}

#[test]
fn skips_forwarder_when_param_type_is_excluded() {
    let func = make_function(
        "extract_keywords",
        vec![("config", TypeRef::Named("KeywordConfig".to_string()))],
        TypeRef::Unit,
    );
    let mut exclude: HashSet<String> = HashSet::new();
    exclude.insert("KeywordConfig".to_string());
    assert!(function_references_excluded_type(&func, &exclude));
}

#[test]
fn skips_forwarder_when_return_type_is_excluded() {
    let func = make_function(
        "build_keyword",
        vec![("text", TypeRef::String)],
        TypeRef::Named("Keyword".to_string()),
    );
    let mut exclude: HashSet<String> = HashSet::new();
    exclude.insert("Keyword".to_string());
    assert!(function_references_excluded_type(&func, &exclude));
}

#[test]
fn keeps_forwarder_when_only_primitives_are_used() {
    let func = make_function(
        "echo_count",
        vec![("count", TypeRef::Primitive(PrimitiveType::U32))],
        TypeRef::Primitive(PrimitiveType::U32),
    );
    let mut exclude: HashSet<String> = HashSet::new();
    exclude.insert("KeywordConfig".to_string());
    exclude.insert("Keyword".to_string());
    assert!(!function_references_excluded_type(&func, &exclude));
}

#[test]
fn skips_forwarder_when_vec_named_param_is_excluded() {
    let func = make_function(
        "score_keywords",
        vec![(
            "keywords",
            TypeRef::Vec(Box::new(TypeRef::Named("Keyword".to_string()))),
        )],
        TypeRef::Unit,
    );
    let mut exclude: HashSet<String> = HashSet::new();
    exclude.insert("Keyword".to_string());
    assert!(function_references_excluded_type(&func, &exclude));
}

#[test]
fn skips_forwarder_when_optional_named_return_is_excluded() {
    let func = make_function(
        "maybe_yake",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::Named("YakeParams".to_string()))),
    );
    let mut exclude: HashSet<String> = HashSet::new();
    exclude.insert("YakeParams".to_string());
    assert!(function_references_excluded_type(&func, &exclude));
}

#[test]
fn empty_exclude_set_keeps_every_function() {
    let func = make_function(
        "extract_keywords",
        vec![("config", TypeRef::Named("KeywordConfig".to_string()))],
        TypeRef::Named("Keyword".to_string()),
    );
    let exclude: HashSet<String> = HashSet::new();
    assert!(!function_references_excluded_type(&func, &exclude));
}

#[test]
fn skips_forwarder_when_map_value_is_excluded_type() {
    let func = make_function(
        "score_map",
        vec![(
            "table",
            TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("Keyword".to_string())),
            ),
        )],
        TypeRef::Unit,
    );
    let mut exclude: HashSet<String> = HashSet::new();
    exclude.insert("Keyword".to_string());
    assert!(function_references_excluded_type(&func, &exclude));
}

fn make_capsule_fn() -> FunctionDef {
    make_function(
        "get_language",
        vec![("name", TypeRef::String)],
        TypeRef::Named("Language".to_string()),
    )
}

#[test]
fn capsule_forwarder_emits_opaque_pointer_reconstruction() {
    let func = make_capsule_fn();
    let cfg = crate::core::config::HostCapsuleTypeConfig {
        host_type: "MyLib.Language".to_string(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: "MyLib.Language({ptr})".to_string(),
        ..Default::default()
    };
    let mut out = String::new();
    capsule::emit_capsule_free_function_forwarder(&func, "GetLanguage", &cfg, &mut out);
    assert!(
        out.contains("OpaquePointer(bitPattern:"),
        "capsule forwarder must reconstruct OpaquePointer via bitPattern. Got:\n{out}"
    );
    assert!(
        out.contains("addr != 0"),
        "capsule forwarder must check for 0 sentinel. Got:\n{out}"
    );
}

#[test]
fn capsule_forwarder_errors_when_construct_expr_empty() {
    let func = make_capsule_fn();
    let cfg = crate::core::config::HostCapsuleTypeConfig {
        host_type: "MyLib.Language".to_string(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: String::new(),
        ..Default::default()
    };
    let mut out = String::new();
    capsule::emit_capsule_free_function_forwarder(&func, "GetLanguage", &cfg, &mut out);
    assert!(
        out.contains("ALEF ERROR"),
        "empty construct_expr must produce ALEF ERROR. Got:\n{out}"
    );
    assert!(
        out.contains("construct_expr"),
        "error must name the missing field. Got:\n{out}"
    );
}

fn make_async_enum_return_fn() -> FunctionDef {
    let mut func = make_function(
        "refresh_catalog",
        vec![("config", TypeRef::Named("CatalogRefreshConfig".to_string()))],
        TypeRef::Named("RefreshOutcome".to_string()),
    );
    func.is_async = true;
    func.error_type = Some("String".to_string());
    func
}

/// Regression test: a service function returning a `String`-backed enum (e.g. serde
/// `RefreshOutcome`) must not be constructed via the struct positional-init pattern
/// `EnumName(_rb_obj)` — enums only synthesize `init(from: Decoder)`, so that call fails
/// to compile. The async forwarder must decode via the enum's `RawValue` initializer
/// instead.
#[test]
fn async_forwarder_decodes_unit_enum_return_via_raw_value_not_positional_init() {
    let func = make_async_enum_return_fn();
    let mut known_dto_names: HashSet<String> = HashSet::new();
    // `known_dto_names` mirrors `compute_first_class_dto_names`, which intentionally
    // includes unit-serde enum names alongside true struct DTOs. ~keep
    known_dto_names.insert("RefreshOutcome".to_string());
    let enum_names: HashSet<String> = known_dto_names.clone();
    let unit_enum_names: HashSet<String> = known_dto_names.clone();

    let mut out = String::new();
    emit_async_free_function_forwarder(
        &func,
        "refreshCatalog",
        &known_dto_names,
        &enum_names,
        &unit_enum_names,
        AsyncForwarderContext {
            handle_returned_types: &HashSet::new(),
            error_type_name: "MyLibError",
        },
        &mut out,
    );

    assert!(
        !out.contains("RefreshOutcome(_rb_obj)"),
        "must not emit the struct-init pattern for an enum return. Got:\n{out}"
    );
    assert!(
        out.contains("RefreshOutcome(rawValue:"),
        "must decode the enum via its RawValue initializer. Got:\n{out}"
    );
    assert!(
        out.contains("MyLibError.validation(message: \"Unknown RefreshOutcome variant\""),
        "must throw a validation error naming the enum on an unrecognized raw value. Got:\n{out}"
    );
}

fn make_sync_enum_return_fn() -> FunctionDef {
    let mut func = make_function("current_outcome", vec![], TypeRef::Named("RefreshOutcome".to_string()));
    func.error_type = Some("String".to_string());
    func
}

/// Same regression as the async case, but for the synchronous free-function forwarder
/// path (`emit_single_free_function_forwarder`), which shares the same
/// `known_dto_names`-conflates-structs-and-enums root cause.
#[test]
fn sync_forwarder_decodes_unit_enum_return_via_raw_value_not_positional_init() {
    let func = make_sync_enum_return_fn();
    let mut known_dto_names: HashSet<String> = HashSet::new();
    known_dto_names.insert("RefreshOutcome".to_string());
    let unit_enum_names: HashSet<String> = known_dto_names.clone();
    let client_class_names: HashSet<String> = HashSet::new();

    let mut out = String::new();
    emit_single_free_function_forwarder(
        &func,
        "currentOutcome",
        &known_dto_names,
        &unit_enum_names,
        "MyLibError",
        &client_class_names,
        &mut out,
    );

    assert!(
        !out.contains("RefreshOutcome(_rb)"),
        "must not emit the struct-init pattern for an enum return. Got:\n{out}"
    );
    assert!(
        out.contains("RefreshOutcome(rawValue:"),
        "must decode the enum via its RawValue initializer. Got:\n{out}"
    );
    assert!(
        out.contains("MyLibError.validation(message: \"Unknown RefreshOutcome variant\""),
        "must throw a validation error naming the enum on an unrecognized raw value. Got:\n{out}"
    );
}

/// Regression test: `TypeRef::Path` used to fall through to the default match arm in
/// `forwarder_param_signature`, which returns `setup_line: None`, so a `Path` param was
/// passed unwrapped as a Swift `String` while a sibling `String` param was wrapped into
/// `RustString` -- producing "conflicting arguments to generic parameter
/// 'GenericIntoRustString' ('RustString' vs. 'String')". `Path` must take the same wrapping
/// arm as `String`. ~keep
#[test]
fn string_and_path_params_produce_identical_forwarder_arg_shape() {
    let known_dto_names: HashSet<String> = HashSet::new();
    let (string_ty, string_arg) = forwarder_param_signature(&TypeRef::String, "path", false, &known_dto_names);
    let (path_ty, path_arg) = forwarder_param_signature(&TypeRef::Path, "path", false, &known_dto_names);

    assert_eq!(
        string_ty, path_ty,
        "String and Path params must produce the same Swift parameter type"
    );
    assert_eq!(
        path_arg.setup_line, string_arg.setup_line,
        "Path must wrap into RustString exactly like String, not fall through with setup_line: None"
    );
    assert_eq!(path_arg.arg_expr, string_arg.arg_expr);
    assert_eq!(path_ty, "String");
    assert_eq!(path_arg.setup_line, Some("let _rb_path = RustString(path)".to_string()));
    assert_eq!(path_arg.arg_expr, "_rb_path");
}

#[test]
fn optional_path_param_wraps_like_optional_string() {
    let known_dto_names: HashSet<String> = HashSet::new();
    let ty = TypeRef::Optional(Box::new(TypeRef::Path));
    let (swift_ty, arg) = forwarder_param_signature(&ty, "path", true, &known_dto_names);
    assert_eq!(swift_ty, "String?");
    assert_eq!(
        arg.setup_line,
        Some("let _rb_path = path.map { RustString($0) }".to_string())
    );
    assert_eq!(arg.arg_expr, "_rb_path");
}

/// Regression test: `Vec<DTO>` returns must be recognized as throwing conversions so the
/// forwarder wrapper gets a `throws` clause and the return statement gets `try` -- otherwise
/// `try Dto(ref)` sits in a non-throwing context and fails to compile. ~keep
#[test]
fn vec_dto_return_throws_when_element_type_is_known_dto() {
    let mut known_dto_names: HashSet<String> = HashSet::new();
    known_dto_names.insert("PatternMatch".to_string());
    let ty = TypeRef::Vec(Box::new(TypeRef::Named("PatternMatch".to_string())));
    assert!(return_value_conversion_throws(&ty, &known_dto_names));
}

#[test]
fn vec_dto_return_does_not_throw_when_element_type_is_not_a_known_dto() {
    let known_dto_names: HashSet<String> = HashSet::new();
    let ty = TypeRef::Vec(Box::new(TypeRef::Named("PatternMatch".to_string())));
    assert!(!return_value_conversion_throws(&ty, &known_dto_names));
}

#[test]
fn bare_named_dto_return_throws_when_known() {
    let mut known_dto_names: HashSet<String> = HashSet::new();
    known_dto_names.insert("PatternMatch".to_string());
    let ty = TypeRef::Named("PatternMatch".to_string());
    assert!(return_value_conversion_throws(&ty, &known_dto_names));
}

#[test]
fn optional_named_dto_return_throws_when_known() {
    let mut known_dto_names: HashSet<String> = HashSet::new();
    known_dto_names.insert("PatternMatch".to_string());
    let ty = TypeRef::Optional(Box::new(TypeRef::Named("PatternMatch".to_string())));
    assert!(return_value_conversion_throws(&ty, &known_dto_names));
}

#[test]
fn vec_string_return_never_throws() {
    let known_dto_names: HashSet<String> = HashSet::new();
    let ty = TypeRef::Vec(Box::new(TypeRef::String));
    assert!(!return_value_conversion_throws(&ty, &known_dto_names));
}

/// Regression test: the `Vec<DTO>` return-conversion suffix used to embed a leading `try`
/// (`RustBridge.func(...)try .map { ... }`), which is a syntax error -- `try` belongs at
/// the call-site/statement level, not inside the trailing `.map` chain. ~keep
#[test]
fn vec_dto_return_suffix_maps_with_try_and_has_no_leading_try() {
    let mut known_dto_names: HashSet<String> = HashSet::new();
    known_dto_names.insert("PatternMatch".to_string());
    let ty = TypeRef::Vec(Box::new(TypeRef::Named("PatternMatch".to_string())));
    let suffix = forwarder_return_conversion_suffix_with_throws(&ty, &known_dto_names, true);
    assert_eq!(
        suffix, ".map { ref in try PatternMatch(ref) }",
        "Vec<DTO> suffix must map each element with try inside the closure. Got:\n{suffix}"
    );
    assert!(
        suffix.starts_with(".map"),
        "suffix must start with .map. Got:\n{suffix}"
    );
    assert!(
        !suffix.starts_with("try"),
        "suffix must not start with a leading try. Got:\n{suffix}"
    );
}

#[test]
fn vec_opaque_return_suffix_uses_ptr_based_reconstruction_and_has_no_leading_try() {
    let known_dto_names: HashSet<String> = HashSet::new();
    let ty = TypeRef::Vec(Box::new(TypeRef::Named("Thing".to_string())));
    let suffix = forwarder_return_conversion_suffix_with_throws(&ty, &known_dto_names, true);
    assert_eq!(
        suffix, ".map { ref in var item = try RustBridge.Thing(ptr: ref.ptr); item.isOwned = false; return item }",
        "Vec<opaque> suffix must reconstruct via ptr-based init. Got:\n{suffix}"
    );
    assert!(
        !suffix.starts_with("try"),
        "suffix must not start with a leading try. Got:\n{suffix}"
    );
}

#[test]
fn capsule_forwarder_errors_when_host_type_empty() {
    let func = make_capsule_fn();
    let cfg = crate::core::config::HostCapsuleTypeConfig {
        host_type: String::new(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: "MyLib.Language({ptr})".to_string(),
        ..Default::default()
    };
    let mut out = String::new();
    capsule::emit_capsule_free_function_forwarder(&func, "GetLanguage", &cfg, &mut out);
    assert!(
        out.contains("ALEF ERROR"),
        "empty host_type must produce ALEF ERROR. Got:\n{out}"
    );
    assert!(
        out.contains("host_type"),
        "error must name the missing field. Got:\n{out}"
    );
}

fn make_async_enum_param_fn() -> FunctionDef {
    let mut func = make_function(
        "set_region",
        vec![("region_kind", TypeRef::Named("RegionKind".to_string()))],
        TypeRef::Unit,
    );
    func.is_async = true;
    func.error_type = Some("String".to_string());
    func
}

/// Regression test: an async fn's enum-typed parameter must be JSON-encoded to a String,
/// then wrapped in `RustString(...)` before being passed to the bridge. The Rust bridge
/// declares the param as `String` (enums cross the FFI boundary pre-converted), so calling
/// `.intoRust()` on the high-level enum type -- which returns an opaque type, not a String --
/// fails to compile. Skipping the `RustString` wrap is also wrong: it collides with sibling
/// `RustString` args ("conflicting arguments to generic parameter 'GenericIntoRustString'"). ~keep
#[test]
fn async_forwarder_json_encodes_enum_param_and_wraps_in_rust_string() {
    let func = make_async_enum_param_fn();
    let mut known_dto_names: HashSet<String> = HashSet::new();
    // Mirrors real generator input: unit-serde enums are also first-class DTOs, so the
    // enum branch must win over the known-DTO `.intoRust()` branch, not just be untested. ~keep
    known_dto_names.insert("RegionKind".to_string());
    let mut enum_names: HashSet<String> = HashSet::new();
    enum_names.insert("RegionKind".to_string());
    let unit_enum_names: HashSet<String> = HashSet::new();

    let mut out = String::new();
    emit_async_free_function_forwarder(
        &func,
        "setRegion",
        &known_dto_names,
        &enum_names,
        &unit_enum_names,
        AsyncForwarderContext {
            handle_returned_types: &HashSet::new(),
            error_type_name: "MyLibError",
        },
        &mut out,
    );

    assert!(
        out.contains(
            "let _rb_regionKind = RustString(try String(data: JSONEncoder().encode(regionKind), encoding: .utf8) ?? \"null\")"
        ),
        "enum param must be JSON-encoded and the result wrapped in RustString(...). Got:\n{out}"
    );
    assert!(
        !out.contains("regionKind.intoRust()"),
        "enum param must not be passed via .intoRust() -- that returns an opaque type, not a String. Got:\n{out}"
    );
}

fn make_async_unit_return_fn() -> FunctionDef {
    let mut func = make_function("delete_catalog", vec![("catalog_id", TypeRef::String)], TypeRef::Unit);
    func.is_async = true;
    func.error_type = Some("String".to_string());
    func
}

/// Regression test: an async fn with a void (`TypeRef::Unit`) return must not emit a
/// `let result = ...` binding or a `return result`. Before the fix the facade emitted both,
/// so a local `result` of type `()` shadowed a same-named parameter, producing "constant
/// 'result' inferred to have type '()', which may be unexpected". The body must be the bare
/// bridge call with no binding and no return statement. ~keep
#[test]
fn async_forwarder_void_return_emits_no_result_binding_or_return() {
    let func = make_async_unit_return_fn();
    let known_dto_names: HashSet<String> = HashSet::new();
    let enum_names: HashSet<String> = HashSet::new();
    let unit_enum_names: HashSet<String> = HashSet::new();

    let mut out = String::new();
    emit_async_free_function_forwarder(
        &func,
        "deleteCatalog",
        &known_dto_names,
        &enum_names,
        &unit_enum_names,
        AsyncForwarderContext {
            handle_returned_types: &HashSet::new(),
            error_type_name: "MyLibError",
        },
        &mut out,
    );

    assert!(
        !out.contains("let result ="),
        "void async return must not bind an unused `let result`. Got:\n{out}"
    );
    assert!(
        !out.contains("return result"),
        "void async return must not emit `return result` -- there is nothing to return. Got:\n{out}"
    );
    assert!(
        out.contains("try await RustBridge.deleteCatalog(_rb_catalogId)"),
        "void async body must call the bridge directly with no binding. Got:\n{out}"
    );
}

fn make_async_string_return_fn() -> FunctionDef {
    let mut func = make_function("fetch_status", vec![("catalog_id", TypeRef::String)], TypeRef::String);
    func.is_async = true;
    func.error_type = Some("String".to_string());
    func
}

/// Regression test: an async fn returning `String` must convert the bridge's `RustString`
/// result to a native Swift `String` via `.toString()`, not return the opaque bridge type
/// directly. ~keep
#[test]
fn async_forwarder_string_return_converts_via_to_string() {
    let func = make_async_string_return_fn();
    let known_dto_names: HashSet<String> = HashSet::new();
    let enum_names: HashSet<String> = HashSet::new();
    let unit_enum_names: HashSet<String> = HashSet::new();

    let mut out = String::new();
    emit_async_free_function_forwarder(
        &func,
        "fetchStatus",
        &known_dto_names,
        &enum_names,
        &unit_enum_names,
        AsyncForwarderContext {
            handle_returned_types: &HashSet::new(),
            error_type_name: "MyLibError",
        },
        &mut out,
    );

    assert!(
        out.contains("let result = try await RustBridge.fetchStatus(_rb_catalogId)"),
        "String async return must bind the bridge's RustString result. Got:\n{out}"
    );
    assert!(
        out.contains("return result.toString()"),
        "String async return must convert the RustString result to a native Swift String via .toString(). Got:\n{out}"
    );
}

#[test]
fn infallible_async_forwarders_always_try_the_result_bridge() {
    let transparent = NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
        "sample::SecretString",
        "from",
        "into_inner",
        vec![],
    )]);
    let cases = [
        ("ordinaryString", TypeRef::String, None),
        ("transparentString", TypeRef::String, Some(transparent)),
        ("unitValue", TypeRef::Unit, None),
        ("enumValue", TypeRef::Named("Status".to_string()), None),
        ("dtoValue", TypeRef::Named("Record".to_string()), None),
    ];

    for (swift_name, return_type, wrapper) in cases {
        let mut func = make_function(swift_name, vec![], return_type);
        func.is_async = true;
        func.return_newtype_wrapper = wrapper;
        let known_dto_names = HashSet::from(["Status".to_string(), "Record".to_string()]);
        let unit_enum_names = HashSet::from(["Status".to_string()]);
        let mut out = String::new();

        emit_async_free_function_forwarder(
            &func,
            swift_name,
            &known_dto_names,
            &HashSet::new(),
            &unit_enum_names,
            AsyncForwarderContext {
                handle_returned_types: &HashSet::new(),
                error_type_name: "MyLibError",
            },
            &mut out,
        );

        assert!(
            out.contains(&format!("public func {swift_name}() async throws")),
            "every async facade must expose the bridge Result as throws, got:\n{out}"
        );
        assert!(
            !out.contains("Task.detached"),
            "the facade must await the async bridge directly, not park a pool thread, got:\n{out}"
        );
        assert!(
            out.contains(&format!("try await RustBridge.{swift_name}()")),
            "the Result-returning bridge call must be awaited with try, got:\n{out}"
        );
    }
}

#[test]
fn async_capsule_forwarders_are_throwing_for_infallible_and_fallible_core_functions() {
    let cfg = crate::core::config::HostCapsuleTypeConfig {
        host_type: "MyLib.Language".to_string(),
        construct_expr: "MyLib.Language({ptr})".to_string(),
        ..Default::default()
    };

    for is_fallible in [false, true] {
        let mut func = make_capsule_fn();
        func.is_async = true;
        func.error_type = is_fallible.then(|| "SampleError".to_string());
        let mut out = String::new();

        capsule::emit_async_capsule_free_function_forwarder(&func, "getLanguage", &cfg, &mut out);

        assert!(
            out.contains("public func getLanguage(name: String) async throws -> MyLib.Language"),
            "async capsule facades must always throw and return a nonoptional host value, got:\n{out}"
        );
        assert!(
            !out.contains("Task.detached"),
            "the capsule facade must await the async bridge directly, got:\n{out}"
        );
        assert!(
            out.contains("let addr = try await RustBridge.getLanguage(name)"),
            "the Result-returning capsule bridge call must be awaited with try, got:\n{out}"
        );
        assert!(
            out.contains("else { throw NSError("),
            "a zero capsule pointer must remain an error rather than become nil, got:\n{out}"
        );
    }
}

#[test]
fn infallible_async_json_result_payloads_agree_across_all_generated_surfaces() {
    let cases = [
        ("optionalValue", TypeRef::Optional(Box::new(TypeRef::String)), "String?"),
        (
            "mapValue",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            "[String: String]",
        ),
        (
            "nestedValues",
            TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
            "[[String]]",
        ),
        ("unsignedValue", TypeRef::Primitive(PrimitiveType::U64), "UInt64"),
        ("signedValue", TypeRef::Primitive(PrimitiveType::I64), "Int64"),
    ];

    for (swift_name, return_type, swift_type) in cases {
        let mut func = make_function(swift_name, vec![], return_type);
        func.is_async = true;
        let handle_returned_types: HashSet<String> = HashSet::new();
        let enum_names: HashSet<String> = HashSet::new();
        let unit_enum_names: HashSet<&str> = HashSet::new();
        let extern_block = crate::backends::swift::gen_rust_crate::extern_block::emit_extern_block_for_functions(
            std::slice::from_ref(&func),
            &handle_returned_types,
            &enum_names,
            &unit_enum_names,
            &std::collections::BTreeSet::new(),
            &std::collections::HashMap::new(),
            &ahash::AHashSet::default(),
        )
        .expect("emit async extern");
        let type_paths = std::collections::HashMap::new();
        let shim_enum_names: HashSet<&str> = HashSet::new();
        let capsule_types = std::collections::HashMap::new();
        let opaque_types = ahash::AHashSet::default();
        let shim_context = crate::backends::swift::gen_rust_crate::shims::FunctionShimContext {
            source_crate: "sample",
            type_paths: &type_paths,
            unit_enum_names: &shim_enum_names,
            tagged_enum_names: &shim_enum_names,
            no_serde_names: &shim_enum_names,
            handle_returned_types: &handle_returned_types,
            capsule_types: &capsule_types,
            opaque_types: &opaque_types,
        };
        let shim = crate::backends::swift::gen_rust_crate::shims::emit_function_shim(&func, &shim_context)
            .expect("emit async shim");
        let mut swift = String::new();
        emit_async_free_function_forwarder(
            &func,
            swift_name,
            &enum_names,
            &enum_names,
            &enum_names,
            AsyncForwarderContext {
                handle_returned_types: &handle_returned_types,
                error_type_name: "MyLibError",
            },
            &mut swift,
        );

        assert!(extern_block.contains("-> Result<String, String>"), "{extern_block}");
        assert!(
            shim.contains("serde_json::to_string"),
            "the Result Ok payload must be serialized to match the extern String, got:\n{shim}"
        );
        assert!(
            swift.contains(&format!("try await RustBridge.{swift_name}().toString()")),
            "the Swift facade must read the Result Ok String payload, got:\n{swift}"
        );
        assert!(
            swift.contains(&format!("JSONDecoder().decode({swift_type}.self")),
            "the Swift facade must decode the original return type, got:\n{swift}"
        );
    }
}

#[test]
fn async_optional_handle_result_remains_native_in_all_generated_surfaces() {
    let mut func = make_function(
        "optionalHandle",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::Named("OpaqueThing".to_string()))),
    );
    func.is_async = true;
    let handle_returned_types = HashSet::from(["OpaqueThing".to_string()]);
    let enum_names: HashSet<String> = HashSet::new();
    let unit_enum_names: HashSet<&str> = HashSet::new();
    let extern_block = crate::backends::swift::gen_rust_crate::extern_block::emit_extern_block_for_functions(
        std::slice::from_ref(&func),
        &handle_returned_types,
        &enum_names,
        &unit_enum_names,
        &std::collections::BTreeSet::new(),
        &std::collections::HashMap::new(),
        &ahash::AHashSet::default(),
    )
    .expect("emit async handle extern");
    let type_paths = std::collections::HashMap::new();
    let shim_enum_names: HashSet<&str> = HashSet::new();
    let capsule_types = std::collections::HashMap::new();
    let opaque_types = ahash::AHashSet::default();
    let shim_context = crate::backends::swift::gen_rust_crate::shims::FunctionShimContext {
        source_crate: "sample",
        type_paths: &type_paths,
        unit_enum_names: &shim_enum_names,
        tagged_enum_names: &shim_enum_names,
        no_serde_names: &shim_enum_names,
        handle_returned_types: &handle_returned_types,
        capsule_types: &capsule_types,
        opaque_types: &opaque_types,
    };
    let shim = crate::backends::swift::gen_rust_crate::shims::emit_function_shim(&func, &shim_context)
        .expect("emit async handle shim");
    let mut swift = String::new();

    emit_async_free_function_forwarder(
        &func,
        "optionalHandle",
        &enum_names,
        &enum_names,
        &enum_names,
        AsyncForwarderContext {
            handle_returned_types: &handle_returned_types,
            error_type_name: "MyLibError",
        },
        &mut swift,
    );

    assert!(
        extern_block.contains("-> Result<Option<OpaqueThing>, String>"),
        "{extern_block}"
    );
    assert!(!shim.contains("serde_json::to_string"), "{shim}");
    assert!(swift.contains("try await RustBridge.optionalHandle()"), "{swift}");
    assert!(!swift.contains("JSONDecoder"), "{swift}");
    assert!(!swift.contains(".toString()"), "{swift}");
}
