//! WASM async and wrapper function coverage, split from tests.rs.

use super::tests::*;
use super::*;
use crate::backends::wasm::type_map::WasmMapper;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
use ahash::AHashSet;
use std::collections::HashMap;

#[test]
fn to_turbofish_from_inserts_turbofish_for_generic_type() {
    assert_eq!(to_turbofish_from("Vec<WasmEntity>"), "Vec::<WasmEntity>");
    assert_eq!(to_turbofish_from("Option<WasmFoo>"), "Option::<WasmFoo>");
    assert_eq!(to_turbofish_from("WasmEntity"), "WasmEntity");
    assert_eq!(to_turbofish_from("HashMap<String, i64>"), "HashMap::<String, i64>");
}

#[test]
fn to_turbofish_from_bare_named_type_is_unchanged() {
    assert_eq!(to_turbofish_from("WasmEntity"), "WasmEntity");
    assert_eq!(to_turbofish_from("ExtractionResult"), "ExtractionResult");
}

#[test]
fn type_has_default_lookup_returns_correct_value() {
    use crate::core::ir::ApiSurface;

    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "test".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::BTreeMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        handler_contracts: vec![],
        services: vec![],
        unsupported_public_items: Vec::new(),
    };

    assert!(
        !type_has_default("NonExistentType", &api),
        "Unknown type should return false"
    );
    assert!(
        !type_has_default("AnyType", &api),
        "Empty API should return false for any type"
    );
}

#[test]
fn gen_wasm_unimplemented_body_string_return_fails_loudly() {
    let body = gen_wasm_unimplemented_body(&TypeRef::String, "extract_text", false);
    assert!(
        body.contains("compile_error!"),
        "non-fallible String return must fail the build: {body}"
    );
    assert!(
        !body.contains("[unimplemented:"),
        "must not fabricate a placeholder string: {body}"
    );
}

#[test]
fn gen_wasm_unimplemented_body_vec_return_fails_loudly() {
    let body = gen_wasm_unimplemented_body(&TypeRef::Vec(Box::new(TypeRef::String)), "list_entries", false);
    assert!(
        body.contains("compile_error!"),
        "non-fallible Vec return must fail the build: {body}"
    );
    assert!(!body.contains("Vec::new()"), "must not fabricate an empty Vec: {body}");
}

#[test]
fn gen_wasm_unimplemented_body_optional_return_fails_loudly() {
    let body = gen_wasm_unimplemented_body(&TypeRef::Optional(Box::new(TypeRef::String)), "find_entry", false);
    assert!(
        body.contains("compile_error!"),
        "non-fallible Optional return must fail the build: {body}"
    );
    assert!(!body.contains("\"None\""), "must not fabricate a None literal: {body}");
}

#[test]
fn gen_wasm_unimplemented_body_primitive_return_fails_loudly() {
    let body = gen_wasm_unimplemented_body(
        &TypeRef::Primitive(crate::core::ir::PrimitiveType::I64),
        "count_entries",
        false,
    );
    assert!(
        body.contains("compile_error!"),
        "non-fallible primitive return must fail the build: {body}"
    );
}

#[test]
fn gen_wasm_unimplemented_body_unit_return_stays_void() {
    let body = gen_wasm_unimplemented_body(&TypeRef::Unit, "run_side_effect", false);
    assert_eq!(body, "()", "Unit return must stay a legitimate void value: {body}");
}

#[test]
fn gen_wasm_unimplemented_body_with_error_type_raises_runtime_error() {
    let body = gen_wasm_unimplemented_body(&TypeRef::String, "extract_text", true);
    assert_eq!(
        body, "Err(JsValue::from_str(\"Not implemented: extract_text\"))",
        "fallible functions must keep raising a real runtime error: {body}"
    );
    assert!(
        !body.contains("compile_error!"),
        "fallible path must not also emit compile_error!: {body}"
    );
}

/// Fixture for the return-conversion tests below: an async free function whose return type is a
/// `Named` the mapper may or may not back with a generated wrapper.
fn async_function_returning(name: &str) -> FunctionDef {
    FunctionDef {
        return_type: TypeRef::Named(name.to_string()),
        ..async_function(vec![])
    }
}

/// The async wrapper converts its result with `{mapped}::from(result)`, which only compiles when
/// the mapped type has a `From<CoreType>`. A `wasm.type_overrides` entry can redirect the return
/// type to the opaque `JsValue`, which implements no such conversion — the signature and the body
/// are both written from the mapper, so the body must follow it into the serde bridge.
#[test]
fn async_free_function_returning_js_value_mapped_type_uses_serde() {
    let mut overrides = HashMap::new();
    overrides.insert("Report".to_string(), "JsValue".to_string());
    let mapper = WasmMapper::new(overrides, "Wasm".to_string());

    let out = gen_function_with_emitted_dtos(
        &async_function_returning("Report"),
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &empty_surface(),
        &AHashSet::new(),
    );

    assert!(
        out.contains("serde_wasm_bindgen::to_value(&(result))"),
        "a JsValue-mapped return must be serialized:\n{out}"
    );
    assert!(
        !out.contains("JsValue::from(result)"),
        "JsValue implements no From<Report>:\n{out}"
    );
}

/// Positive control: with no override the mapper renders `WasmReport`, whose `From<core::Report>`
/// alef generates itself, so the direct conversion must be unchanged.
#[test]
fn async_free_function_returning_wrapper_mapped_type_keeps_from() {
    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());

    let out = gen_function_with_emitted_dtos(
        &async_function_returning("Report"),
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &empty_surface(),
        &AHashSet::new(),
    );

    assert!(
        out.contains("result.into()"),
        "a wrapper-mapped return must keep the direct From conversion:\n{out}"
    );
    assert!(
        !out.contains("serde_wasm_bindgen::to_value(&(result))"),
        "a wrapper-mapped return must not detour through serde:\n{out}"
    );
}

/// Regression for the `extract`/`extract_batch`/`map_url` config-marshalling defect: a single
/// `Named` async parameter backed by a generated `#[wasm_bindgen]` wrapper (i.e. the mapper does
/// not redirect it to the opaque `JsValue`) must take the typed wrapper and `.into()` it, exactly
/// like the sibling `Vec<Named>` path already does for `actions`/`inputs`. Routing it through
/// `serde_wasm_bindgen::from_value::<CoreType>` instead is the bug: that walks the *core* struct's
/// snake_case serde field names against the wasm-bindgen wrapper's camelCase getters, so every
/// multi-word field misses and silently falls back to its serde default.
#[test]
fn async_named_param_with_wrapper_takes_typed_option_and_into() {
    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let func = async_function(vec![param("config", TypeRef::Named("ExtractionConfig".to_string()))]);
    let api = crate::core::ir::ApiSurface {
        types: vec![crate::core::ir::TypeDef {
            name: "ExtractionConfig".to_string(),
            has_default: true,
            ..Default::default()
        }],
        ..empty_surface()
    };

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &api,
        &AHashSet::new(),
    );

    assert!(
        out.contains("config: Option<WasmExtractionConfig>"),
        "a wrapper-backed Named param must take the typed wrapper, not JsValue:\n{out}"
    );
    assert!(
        out.contains("let config_core: sample_fixture::ExtractionConfig = config.map(Into::into).unwrap_or_default();"),
        "omitting the param must still fall back to the core type's Default:\n{out}"
    );
    assert!(
        !out.contains("serde_wasm_bindgen::from_value::<sample_fixture::ExtractionConfig>"),
        "must not round-trip a wrapper-backed type through serde_wasm_bindgen:\n{out}"
    );
}

/// Same fixture, but the param is IR-optional. The Rust parameter type stays `Option<Wasm...>`
/// either way (that's what lets an omitted argument still resolve to the core Default above); what
/// changes is the binding, which keeps `None` as `None` instead of substituting a default.
#[test]
fn async_named_optional_param_with_wrapper_maps_into_option() {
    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let func = async_function(vec![ParamDef {
        optional: true,
        ..param("config", TypeRef::Named("ExtractionConfig".to_string()))
    }]);
    let api = crate::core::ir::ApiSurface {
        types: vec![crate::core::ir::TypeDef {
            name: "ExtractionConfig".to_string(),
            has_default: true,
            ..Default::default()
        }],
        ..empty_surface()
    };

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &api,
        &AHashSet::new(),
    );

    assert!(
        out.contains("config: Option<WasmExtractionConfig>"),
        "an optional wrapper-backed Named param must still take the typed wrapper:\n{out}"
    );
    assert!(
        out.contains("let config_core: Option<sample_fixture::ExtractionConfig> = config.map(Into::into);"),
        "an optional param must map into an Option without a Default substitution:\n{out}"
    );
}

/// Negative control: when a `wasm.type_overrides` entry redirects this Named type to the opaque
/// `JsValue` (no generated wrapper exists to convert from), the original serde round-trip is the
/// only route available and must be preserved.
#[test]
fn async_named_param_overridden_to_js_value_keeps_serde_round_trip() {
    let mut overrides = HashMap::new();
    overrides.insert("ExtractionConfig".to_string(), "JsValue".to_string());
    let mapper = WasmMapper::new(overrides, "Wasm".to_string());
    let func = async_function(vec![param("config", TypeRef::Named("ExtractionConfig".to_string()))]);
    let api = crate::core::ir::ApiSurface {
        types: vec![crate::core::ir::TypeDef {
            name: "ExtractionConfig".to_string(),
            has_default: true,
            ..Default::default()
        }],
        ..empty_surface()
    };

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &api,
        &AHashSet::new(),
    );

    assert!(
        out.contains("config: JsValue"),
        "a JsValue-overridden Named param must keep the JsValue signature:\n{out}"
    );
    assert!(
        out.contains("serde_wasm_bindgen::from_value::<sample_fixture::ExtractionConfig>(config)"),
        "with no generated wrapper to convert from, the serde round-trip is the only route:\n{out}"
    );
}

/// An opaque handle passed by value is consumed by wasm-bindgen's JS glue
/// (`engine.__destroy_into_raw()` nulls `__wbg_ptr`), so the JS object is dead after one call.
/// Taking it by reference makes the glue pass `engine.__wbg_ptr` instead, leaving the object
/// usable. ~keep
#[test]
fn async_opaque_handle_param_is_taken_by_reference() {
    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let mut func = async_function(vec![
        param("engine", TypeRef::Named("CrawlEngineHandle".to_string())),
        param("url", TypeRef::String),
    ]);
    func.name = "scrape".to_string();
    func.rust_path = "sample_fixture::scrape".to_string();
    func.return_type = TypeRef::Named("ScrapeResult".to_string());

    let opaque: AHashSet<String> = ["CrawlEngineHandle".to_string()].into_iter().collect();

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &opaque,
        "Wasm",
        &AHashSet::new(),
        &empty_surface(),
        &AHashSet::new(),
    );

    assert!(
        out.contains("engine: &WasmCrawlEngineHandle"),
        "opaque handle param must be by reference so repeated calls do not hit a null pointer:\n{out}"
    );
    assert!(
        !out.contains("engine: WasmCrawlEngineHandle"),
        "by-value opaque handle param leaks into the signature:\n{out}"
    );
    assert!(
        out.contains("sample_fixture::scrape(&engine.inner,"),
        "core call must still borrow the inner Arc:\n{out}"
    );
}

/// Same contract for the synchronous delegation path in `orchestration`. ~keep
#[test]
fn sync_opaque_handle_param_is_taken_by_reference() {
    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let mut func = async_function(vec![param("engine", TypeRef::Named("CrawlEngineHandle".to_string()))]);
    func.name = "close".to_string();
    func.rust_path = "sample_fixture::close".to_string();
    func.is_async = false;

    let opaque: AHashSet<String> = ["CrawlEngineHandle".to_string()].into_iter().collect();

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &opaque,
        "Wasm",
        &AHashSet::new(),
        &empty_surface(),
        &AHashSet::new(),
    );

    assert!(
        out.contains("engine: &WasmCrawlEngineHandle"),
        "opaque handle param must be by reference in the sync path too:\n{out}"
    );
}

#[test]
fn ordinary_vec_of_optional_strings_recovers_jsvalue_before_core_call() {
    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let mut func = async_function(vec![param(
        "values",
        TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(TypeRef::String)))),
    )]);
    func.is_async = false;
    func.error_type = None;

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &empty_surface(),
        &AHashSet::new(),
    );

    assert!(out.contains("values: JsValue"), "{out}");
    assert!(
        out.contains("let values: Vec<Option<String>> = serde_wasm_bindgen::from_value(values)"),
        "{out}"
    );
    assert!(out.contains("sample_fixture::interact(values)"), "{out}");
}

#[test]
fn sanitized_fallible_path_recovers_nested_wrapper_jsvalue_once() {
    use crate::core::ir::NewtypeContainer::{Optional, Vec as VecContainer};

    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let mut values = param(
        "values",
        TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(TypeRef::String)))),
    );
    values.newtype_wrapper = Some(transparent_string_wrapper(vec![VecContainer, Optional]));
    let mut marker = param("marker", TypeRef::String);
    marker.sanitized = true;
    let mut func = async_function(vec![values, marker]);
    func.is_async = false;
    func.sanitized = true;
    func.return_type = TypeRef::String;

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &empty_surface(),
        &AHashSet::new(),
    );

    assert_eq!(
        out.matches("serde_wasm_bindgen::from_value(values)").count(),
        1,
        "{out}"
    );
    assert!(
        out.contains("sample_fixture::SecretString::from(value)"),
        "nested wrapper leaves must be reconstructed after JsValue recovery: {out}"
    );
}

#[test]
fn root_optional_transparent_string_function_args_use_configured_constructor_items() {
    use crate::core::ir::NewtypeContainer;

    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let mut owned = param("owned", TypeRef::String);
    owned.optional = true;
    owned.newtype_wrapper = Some(transparent_string_wrapper_with_constructor(
        "new_secret",
        vec![NewtypeContainer::Optional],
    ));
    let mut borrowed = param("borrowed", TypeRef::String);
    borrowed.optional = true;
    borrowed.is_ref = true;
    borrowed.newtype_wrapper = Some(transparent_string_wrapper_with_constructor(
        "new_secret",
        vec![NewtypeContainer::Optional],
    ));
    let mut func = async_function(vec![owned, borrowed]);
    func.is_async = false;
    func.error_type = None;

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &empty_surface(),
        &AHashSet::new(),
    );

    assert!(
        out.contains("let borrowed_newtype = (borrowed).map(sample_fixture::SecretString::new_secret);"),
        "borrowed input must keep the converted option alive: {out}"
    );
    assert!(
        out.contains(
            "sample_fixture::interact((owned).map(sample_fixture::SecretString::new_secret), borrowed_newtype.as_ref())"
        ),
        "ordinary and borrowed call arguments must use the configured constructor: {out}"
    );
    assert!(
        !out.contains("map(|value| sample_fixture::SecretString::new_secret(value))"),
        "{out}"
    );
}

#[test]
fn externally_optional_deep_wrapper_param_uses_single_jsvalue_without_losing_depth() {
    use crate::core::ir::NewtypeContainer::Optional;

    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let mut value = param("value", TypeRef::Optional(Box::new(TypeRef::String)));
    value.optional = true;
    value.newtype_wrapper = Some(transparent_string_wrapper(vec![Optional, Optional]));
    let mut func = async_function(vec![value]);
    func.is_async = false;
    func.error_type = None;

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &empty_surface(),
        &AHashSet::new(),
    );

    assert!(out.contains("value: Option<JsValue>"), "{out}");
    assert!(out.contains("let value: Option<Option<String>>"), "{out}");
    assert!(
        out.contains("value.map(|value| serde_wasm_bindgen::from_value(value)"),
        "{out}"
    );
}
