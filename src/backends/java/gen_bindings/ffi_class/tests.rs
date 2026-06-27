use crate::core::config::{
    BridgeBinding, GenerateConfig, HostCapsuleTypeConfig, ResolvedCrateConfig, TraitBridgeConfig,
};
use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use std::collections::HashMap;
use std::collections::HashSet;

use super::*;

fn create_test_opaque_types() -> AHashSet<String> {
    AHashSet::new()
}

fn create_test_bridge_sets() -> (HashSet<String>, HashSet<String>) {
    (HashSet::new(), HashSet::new())
}

fn create_test_capsule_types() -> HashMap<String, HostCapsuleTypeConfig> {
    HashMap::new()
}

fn create_test_function(name: &str, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test::{}", name),
        original_rust_path: String::new(),
        params: vec![],
        return_type,
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[test]
fn test_optional_string_return_emits_optional_empty() {
    let func = create_test_function("get_name", TypeRef::Optional(Box::new(TypeRef::String)));

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "test",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &AHashMap::new(),
        &create_test_capsule_types(),
    );

    assert!(out.contains("return Optional.empty();"));
    assert!(out.contains("return Optional.of(str);"));
}

#[test]
fn test_optional_named_return_emits_optional_wrappers() {
    let func = create_test_function(
        "get_preset",
        TypeRef::Optional(Box::new(TypeRef::Named("EmbeddingPreset".to_string()))),
    );

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "test",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &AHashMap::new(),
        &create_test_capsule_types(),
    );

    assert!(out.contains("return Optional.empty();"));
    assert!(out.contains("return Optional.of(MAPPER.readValue(json, EmbeddingPreset.class));"));
}

#[test]
fn test_optional_vec_return_emits_optional_list() {
    let func = create_test_function(
        "list_items",
        TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
    );

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "test",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &AHashMap::new(),
        &create_test_capsule_types(),
    );

    // Vec returns now go through the readJsonList helper to deduplicate
    // the JSON-deserialize boilerplate (CPD was flagging multiple inline
    // copies). The empty-list-on-null path lives inside the helper.
    assert!(out.contains(
            "return Optional.of(readJsonList(resultPtr, new com.fasterxml.jackson.core.type.TypeReference<java.util.List<String>>()"
        ));
}

#[test]
fn test_optional_bytes_result_emits_out_param_pattern() {
    // Bytes with error_type → out-param convention: i32 return + 3 trailing out-params.
    let func = FunctionDef {
        name: "get_data".to_string(),
        rust_path: "test::get_data".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::Optional(Box::new(TypeRef::Bytes)),
        is_async: false,
        error_type: Some("SampleCrateError".to_string()),
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "test",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &AHashMap::new(),
        &create_test_capsule_types(),
    );

    assert!(out.contains("outPtrHolder"), "should allocate outPtrHolder");
    assert!(out.contains("outLenHolder"), "should allocate outLenHolder");
    assert!(out.contains("outCapHolder"), "should allocate outCapHolder");
    assert!(out.contains("int rc = (int)"), "should have int rc");
    assert!(out.contains("TEST_FREE_BYTES"), "should call FREE_BYTES");
    assert!(out.contains("return Optional.empty();"));
    assert!(out.contains("return Optional.of(result);"));
}

#[test]
fn test_options_field_visitor_setter_uses_configured_renderer_field() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            name: "parse".to_string(),
            rust_path: "syntax::parse".to_string(),
            params: vec![
                ParamDef {
                    name: "source".to_string(),
                    ty: TypeRef::String,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Named("ParseOptions".to_string()),
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Named("WalkOutcome".to_string()),
            error_type: Some("ParseError".to_string()),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };
    let config = ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "SyntaxWalker".to_string(),
            type_alias: Some("SyntaxWalkerHandle".to_string()),
            param_name: Some("renderer".to_string()),
            bind_via: BridgeBinding::OptionsField,
            options_type: Some("ParseOptions".to_string()),
            options_field: Some("renderer".to_string()),
            context_type: Some("SyntaxContext".to_string()),
            result_type: Some("WalkOutcome".to_string()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let out = gen_main_class(
        &api,
        &config,
        "dev.syntax",
        "Syntax",
        "syn",
        &HashSet::new(),
        &HashSet::new(),
        true,
        &create_test_capsule_types(),
    );

    assert!(
        out.contains("NativeLib.SYN_OPTIONS_SET_RENDERER.invoke("),
        "Java options-field bridge must invoke the renderer-derived setter"
    );
    assert!(
        !out.contains("SYN_OPTIONS_SET_VISITOR_HANDLE") && !out.contains("options_set_visitor_handle"),
        "Java options-field bridge must not bind the legacy visitor_handle setter"
    );
}

#[test]
fn test_java_async_wrappers_respect_generate_override() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            name: "extract".to_string(),
            rust_path: "demo::extract".to_string(),
            return_type: TypeRef::Named("ExtractionResult".to_string()),
            is_async: true,
            error_type: Some("DemoError".to_string()),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };
    let mut generate_overrides = HashMap::new();
    generate_overrides.insert(
        "java".to_string(),
        GenerateConfig {
            async_wrappers: false,
            ..GenerateConfig::default()
        },
    );
    let config = ResolvedCrateConfig {
        generate_overrides,
        ..ResolvedCrateConfig::default()
    };
    let out = gen_main_class(
        &api,
        &config,
        "com.example",
        "ExampleRs",
        "example",
        &HashSet::new(),
        &HashSet::new(),
        false,
        &create_test_capsule_types(),
    );

    assert!(out.contains("ExtractionResult extract("));
    assert!(!out.contains("extractAsync("));
    assert!(!out.contains("CompletableFuture"));
}

#[test]
fn test_bytes_result_emits_out_param_pattern_non_optional() {
    // Non-optional Bytes with error_type → out-param convention.
    let func = FunctionDef {
        name: "render_png".to_string(),
        rust_path: "test::render_png".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::Bytes,
        is_async: false,
        error_type: Some("SampleCrateError".to_string()),
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "test",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &AHashMap::new(),
        &create_test_capsule_types(),
    );

    assert!(out.contains("outPtrHolder"), "should allocate outPtrHolder");
    assert!(out.contains("TEST_FREE_BYTES"), "should call FREE_BYTES");
    assert!(!out.contains("FREE_STRING"), "must not use FREE_STRING");
    assert!(out.contains("return result;"), "non-optional plain return");
    assert!(!out.contains("Optional.of(result)"), "must not wrap in Optional");
}

#[test]
fn test_non_optional_string_return_no_optional_wrapper() {
    let func = create_test_function("get_name", TypeRef::String);

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "test",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &AHashMap::new(),
        &create_test_capsule_types(),
    );

    assert!(out.contains("return null;"));
    assert!(out.contains("return str;"));
    assert!(!out.contains("Optional.empty()"));
    assert!(!out.contains("Optional.of(str)"));
}

#[test]
fn test_path_return_wraps_with_path_of() {
    let func = create_test_function("cache_dir", TypeRef::Path);

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "test",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &AHashMap::new(),
        &create_test_capsule_types(),
    );

    assert!(out.contains("return java.nio.file.Path.of(str);"));
    assert!(!out.contains("return str;"));
}

#[test]
fn test_optional_path_return_wraps_with_path_of() {
    let func = create_test_function("maybe_cache_dir", TypeRef::Optional(Box::new(TypeRef::Path)));

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "test",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &AHashMap::new(),
        &create_test_capsule_types(),
    );

    assert!(out.contains("return Optional.of(java.nio.file.Path.of(str));"));
}

#[test]
fn test_non_optional_vec_return_no_optional_wrapper() {
    let func = create_test_function("list_items", TypeRef::Vec(Box::new(TypeRef::String)));

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "test",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &AHashMap::new(),
        &create_test_capsule_types(),
    );

    // The Vec dispatch path now delegates to the readJsonList helper.
    // Optional<List<T>> wrapping is added by the caller; non-optional
    // is a bare call.
    assert!(out.contains(
        "return readJsonList(resultPtr, new com.fasterxml.jackson.core.type.TypeReference<java.util.List<String>>()"
    ));
    assert!(!out.contains("Optional.of(readJsonList"));
}

#[test]
fn vec_return_uses_helper_not_inline_json_deserialize() {
    // CPD regression: every Vec-returning method previously inlined a
    // ~15-line null-check + reinterpret + free + readValue block, which
    // CPD (rightly) flagged as duplication. The helper extraction means
    // the call site is one line and `readJsonList` appears exactly once
    // in the helper section.
    let func = create_test_function("list_items", TypeRef::Vec(Box::new(TypeRef::String)));

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "test",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &AHashMap::new(),
        &create_test_capsule_types(),
    );

    // The previously-duplicated JSON-deserialize line must NOT appear at
    // the call site any more (it now lives only in the helper, which is
    // emitted by gen_helper_methods at the bottom of the class).
    assert!(!out.contains(
        "createObjectMapper().readValue(json, new com.fasterxml.jackson.core.type.TypeReference<java.util.List<"
    ));
}

#[test]
fn clear_fn_body_references_singular_native_lib_handle() {
    // Regression: a trait-bridge `clear_fn` is the plural core Rust function
    // name, but the FFI export and the `NativeLib`
    // handle constant are the singular trait-derived form
    // (`KRZ_CLEAR_OCR_BACKEND`). The facade body must reference that exact
    // constant and invoke it with an out-error parameter and check the result
    // code, just like other fallible FFI functions.
    let func = create_test_function("clear_ocr_backends", TypeRef::Unit);

    let mut clear_fn_handles = AHashMap::new();
    clear_fn_handles.insert("clear_ocr_backends".to_string(), "KRZ_CLEAR_OCR_BACKEND".to_string());

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "krz",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &clear_fn_handles,
        &create_test_capsule_types(),
    );

    // Must use the singular trait-derived handle constant
    assert!(
        out.contains("NativeLib.KRZ_CLEAR_OCR_BACKEND.invoke"),
        "clear_fn body must reference the singular trait-derived handle, got:\n{out}"
    );
    assert!(
        !out.contains("KRZ_CLEAR_OCR_BACKENDS"),
        "clear_fn body must not reference the plural core-function-derived handle, got:\n{out}"
    );
    // Must allocate out-error buffer
    assert!(
        out.contains("var outErr = arena.allocate(ValueLayout.ADDRESS)"),
        "clear_fn body must allocate outErr, got:\n{out}"
    );
    // Must pass outErr to the FFI invocation
    assert!(
        out.contains("outErr)"),
        "clear_fn body must pass outErr to FFI invocation, got:\n{out}"
    );
    // Must check the return code for error
    assert!(
        out.contains("if (primitiveResult != 0)"),
        "clear_fn body must check primitiveResult != 0, got:\n{out}"
    );
}

#[test]
fn non_clear_fn_body_derives_handle_from_function_name() {
    // Functions not registered as trait-bridge `clear_fn`s keep deriving the
    // handle constant from `func.name` (1:1 with their FFI export).
    let func = create_test_function("list_ocr_backends", TypeRef::Vec(Box::new(TypeRef::String)));

    let mut clear_fn_handles = AHashMap::new();
    clear_fn_handles.insert("clear_ocr_backends".to_string(), "KRZ_CLEAR_OCR_BACKEND".to_string());

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "krz",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &clear_fn_handles,
        &create_test_capsule_types(),
    );

    assert!(
        out.contains("NativeLib.KRZ_LIST_OCR_BACKENDS"),
        "non-clear_fn body must derive the handle from func.name, got:\n{out}"
    );
}

#[test]
fn clear_fn_error_throws_exception_with_code_and_message() {
    // Regression: clear_fn error path must construct SampleCrateRsException
    // with (int code, String message) constructor, not (String) constructor.
    // The error throw must be `new TestClassException(primitiveResult, msg)`
    // matching the SampleCrateRsException(int, String) constructor signature.
    let func = create_test_function("clear_ocr_backends", TypeRef::Unit);

    let mut clear_fn_handles = AHashMap::new();
    clear_fn_handles.insert("clear_ocr_backends".to_string(), "KRZ_CLEAR_OCR_BACKEND".to_string());

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "krz",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &clear_fn_handles,
        &create_test_capsule_types(),
    );

    // Must throw with (int code, String msg) two-argument constructor in the error path
    assert!(
        out.contains("throw new TestClassException(primitiveResult, msg)"),
        "clear_fn error path must throw TestClassException(primitiveResult, msg), got:\n{out}"
    );
}

#[test]
fn capsule_function_returns_host_type() {
    // Host-native capsule (Language) passthrough: construct the host runtime's
    // `Language` from the raw C grammar pointer instead of an opaque handle.
    let func = create_test_function("get_language", TypeRef::Named("Language".to_string()));

    let mut capsule_types = HashMap::new();
    capsule_types.insert(
        "Language".to_string(),
        HostCapsuleTypeConfig {
            host_type: "io.github.tree_sitter.Language".to_string(),
            package: "io.github.tree-sitter:jtreesitter".to_string(),
            package_version: "0.25.0".to_string(),
            construct_expr: "new Language({ptr})".to_string(),
        },
    );

    let mut out = String::new();
    let opaque_types = create_test_opaque_types();
    let (bridge_param_names, bridge_type_aliases) = create_test_bridge_sets();

    gen_sync_function_method(
        &mut out,
        &func,
        "krz",
        "TestClass",
        &opaque_types,
        &bridge_param_names,
        &bridge_type_aliases,
        false,
        &AHashMap::new(),
        &capsule_types,
    );

    // Must return the host type, not an opaque handle
    assert!(
        out.contains("io.github.tree_sitter.Language"),
        "capsule function must return host type, got:\n{out}"
    );
    // Must construct the Language from the raw pointer
    assert!(
        out.contains("new Language(resultPtr)"),
        "capsule function must construct Language from pointer, got:\n{out}"
    );
}
