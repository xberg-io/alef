//! Regression tests for NAPI auto-delegation of common param/return shapes that previously
//! produced wrong code (E0277/E0308/E0599) or `compile_error!` stubs:
//!   - Case 1: a `&T` (non-opaque Named) reference param on a static DTO factory method.
//!   - Case 3: a free function mixing `&Named`, `Option<Json>` and `&Map` params returning Result.
//!   - Case 4: an `Option<bytes>` slice param emitted as `Option<Buffer>` in the binding.

use alef::backends::napi::NapiBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::*;
use std::collections::HashMap;

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.node]
package_name = "test-lib"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn param(name: &str, ty: TypeRef, optional: bool, is_ref: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: CoreWrapper::None,
    }
}

fn dto(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn func(name: &str, params: Vec<ParamDef>, return_type: TypeRef, error_type: Option<&str>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async: false,
        error_type: error_type.map(str::to_string),
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

fn api(types: Vec<TypeDef>, functions: Vec<FunctionDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types,
        functions,
        enums: vec![],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn lib_rs(api: &ApiSurface) -> String {
    let files = NapiBackend.generate_bindings(api, &make_config()).unwrap();
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .map(|f| f.content.clone())
        .expect("lib.rs should be generated")
}

#[test]
fn static_dto_method_with_named_ref_param_binds_owned_core_and_borrows() {
    // `ConfidenceSignals::from_extraction_result(result: &ExtractionResult, ...) -> Self`.
    let from = MethodDef {
        name: "from_extraction_result".to_string(),
        params: vec![
            param("result", TypeRef::Named("ExtractionResult".to_string()), false, true),
            param("text_coverage", TypeRef::Primitive(PrimitiveType::F32), false, false),
        ],
        return_type: TypeRef::Named("ConfidenceSignals".to_string()),
        is_async: false,
        is_static: true,
        error_type: None,
        doc: String::new(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let extraction_result = dto("ExtractionResult", vec![]);
    let signals = dto("ConfidenceSignals", vec![from]);
    let content = lib_rs(&api(vec![extraction_result, signals], vec![]));

    assert!(
        !content.contains("compile_error!"),
        "static DTO factory with &T param must auto-delegate, not bail:\n{content}"
    );
    // The owned core temporary is bound, then borrowed at the call site.
    assert!(
        content.contains("let result_core: test_lib::ExtractionResult = result.into();"),
        "should bind an owned core temporary for the &T param:\n{content}"
    );
    assert!(
        content.contains("test_lib::ConfidenceSignals::from_extraction_result(&result_core, text_coverage"),
        "should pass a borrow of the owned core temporary:\n{content}"
    );
    // It must NOT pass `result.into()` directly where `&ExtractionResult` is expected.
    assert!(
        !content.contains("from_extraction_result(result.into()"),
        "must not pass an owned value where &T is expected:\n{content}"
    );
}

#[test]
fn free_fn_mixing_named_ref_optional_json_and_map_delegates_correctly() {
    // `resolve(preset: &Preset, custom_schema: Option<Json>, context: &BTreeMap<String,String>) -> Result`.
    let resolve = func(
        "resolve",
        vec![
            param("preset", TypeRef::Named("Preset".to_string()), false, true),
            param("custom_schema", TypeRef::Json, true, false),
            {
                let mut p = param(
                    "context",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    false,
                    true,
                );
                p.map_is_btree = true;
                p
            },
        ],
        TypeRef::Named("ResolvedPreset".to_string()),
        Some("ResolveError"),
    );
    let content = lib_rs(&api(
        vec![dto("Preset", vec![]), dto("ResolvedPreset", vec![])],
        vec![resolve],
    ));

    assert!(
        !content.contains("compile_error!"),
        "mixed-shape free fn must auto-delegate:\n{content}"
    );
    assert!(
        content.contains("let preset_core: test_lib::Preset = preset.into();"),
        "should bind an owned core temporary for the &Preset param:\n{content}"
    );
    // NAPI maps Json to serde_json::Value, so the Json param is passed through unchanged
    // (no serde_json::from_str at the call site).
    assert!(
        !content.contains("serde_json::from_str(&custom_schema"),
        "NAPI Json params are already serde_json::Value and must not be re-parsed:\n{content}"
    );
    // The promoted &BTreeMap param is materialised and collected before borrowing.
    assert!(
        content.contains("&context.unwrap_or_default().into_iter().collect::<std::collections::BTreeMap<_, _>>()"),
        "promoted &BTreeMap param should be collected and borrowed:\n{content}"
    );
}

#[test]
fn free_fn_optional_bytes_param_converts_option_buffer_not_option_to_vec() {
    // `analyze(document_bytes: Option<&[u8]>) -> u32` — binding receives Option<Buffer>.
    let analyze = func(
        "analyze",
        vec![param("document_bytes", TypeRef::Bytes, true, true)],
        TypeRef::Primitive(PrimitiveType::U32),
        None,
    );
    let content = lib_rs(&api(vec![], vec![analyze]));

    // Must convert Option<Buffer> -> Option<Vec<u8>> via .map(...), never `Option.to_vec()`.
    assert!(
        content.contains("let document_bytes: Option<Vec<u8>> = document_bytes.map(|b| b.to_vec());"),
        "optional bytes param must convert via .map(|b| b.to_vec()):\n{content}"
    );
    assert!(
        !content.contains("let document_bytes: Vec<u8> = document_bytes.to_vec();"),
        "must not call .to_vec() directly on Option<Buffer>:\n{content}"
    );
    // The call site uses as_deref() to reach Option<&[u8]>.
    assert!(
        content.contains("document_bytes.as_deref()"),
        "call site should pass Option<&[u8]> via as_deref():\n{content}"
    );
}
