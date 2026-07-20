use super::*;
use crate::core::ir::TypeRef;

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
    };
    let mut out = String::new();
    emit_capsule_free_function_forwarder(&func, "GetLanguage", &cfg, &mut out);
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
    };
    let mut out = String::new();
    emit_capsule_free_function_forwarder(&func, "GetLanguage", &cfg, &mut out);
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
    // `known_dto_names` mirrors `compute_first_class_dto_names`, which intentionally ~keep
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
        "LiterLlmError",
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
        out.contains("LiterLlmError.validation(message: \"Unknown RefreshOutcome variant\""),
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
        "LiterLlmError",
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
        out.contains("LiterLlmError.validation(message: \"Unknown RefreshOutcome variant\""),
        "must throw a validation error naming the enum on an unrecognized raw value. Got:\n{out}"
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
    };
    let mut out = String::new();
    emit_capsule_free_function_forwarder(&func, "GetLanguage", &cfg, &mut out);
    assert!(
        out.contains("ALEF ERROR"),
        "empty host_type must produce ALEF ERROR. Got:\n{out}"
    );
    assert!(
        out.contains("host_type"),
        "error must name the missing field. Got:\n{out}"
    );
}
