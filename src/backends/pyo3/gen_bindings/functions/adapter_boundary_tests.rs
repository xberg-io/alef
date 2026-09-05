//! An adapter wrapper's declared param/return types and the values its body actually forwards
//! to (and receives from) the native engine call must agree, exactly like
//! `streaming_yield_tests.rs` proves for the streaming `yield` case. These tests cover the two
//! other adapter shapes that cross the same public-dataclass/native-pyclass boundary:
//! `AdapterPattern::AsyncMethod`'s single return value, and an adapter param typed as a public
//! `options` dataclass on either pattern.

use std::collections::HashMap;

use super::gen_api_py;
use crate::core::config::{AdapterConfig, AdapterParam, AdapterPattern, DtoConfig};
use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

const OWNER_TYPE: &str = "Client";
const REQUEST_TYPE: &str = "RequestOptions";
const RESULT_TYPE: &str = "ResultData";

/// Two `has_default`, non-`is_return_type` structs — both emitted as public `options`
/// dataclasses, and both therefore having a `_to_rust_<snake>` / `_from_native_<snake>`
/// converter pair available.
fn dataclass_types() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: OWNER_TYPE.to_owned(),
            rust_path: format!("sample_core::{OWNER_TYPE}"),
            is_opaque: true,
            ..TypeDef::default()
        },
        TypeDef {
            name: REQUEST_TYPE.to_owned(),
            rust_path: format!("sample_core::{REQUEST_TYPE}"),
            has_default: true,
            fields: vec![FieldDef {
                name: "topic".to_owned(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: RESULT_TYPE.to_owned(),
            rust_path: format!("sample_core::{RESULT_TYPE}"),
            has_default: true,
            fields: vec![FieldDef {
                name: "label".to_owned(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ]
}

fn render(api: &ApiSurface, adapter: &AdapterConfig) -> (String, String) {
    let api_py = gen_api_py(
        api,
        "_internal_bindings",
        "sample_pkg",
        &[],
        &DtoConfig::default(),
        &HashMap::new(),
        &std::collections::BTreeMap::new(),
        std::slice::from_ref(adapter),
        &[],
        &ahash::AHashSet::new(),
        &crate::core::config::ResolvedCrateConfig::default(),
    );
    let options_py = crate::backends::pyo3::gen_bindings::types::gen_options_py(
        api,
        "_internal_bindings",
        &DtoConfig::default(),
        &[],
    );
    (api_py, options_py)
}

/// An `AsyncMethod` adapter whose return type is a public dataclass must convert the engine's
/// native return value with `_from_native_<snake>` before handing it back — the annotation says
/// `ResultData`, but `engine.fetch_data(...)` produces the native pyclass.
#[test]
fn async_method_adapter_converts_a_dataclass_return_value() {
    let api = ApiSurface {
        types: dataclass_types(),
        ..ApiSurface::default()
    };
    let adapter = AdapterConfig {
        name: "fetch_data".to_owned(),
        pattern: AdapterPattern::AsyncMethod,
        core_path: "sample_core::Client::fetch_data".to_owned(),
        params: vec![],
        returns: Some(RESULT_TYPE.to_owned()),
        error_type: None,
        owner_type: Some(OWNER_TYPE.to_owned()),
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,
        skip_languages: vec![],
    };

    let (api_py, options_py) = render(&api, &adapter);

    assert!(
        options_py.contains("def _from_native_result_data("),
        "options.py must define the return converter:\n{options_py}"
    );
    assert!(
        api_py.contains("return _from_native_result_data(await engine.fetch_data())"),
        "api.py must convert the engine's native return value before handing it back:\n{api_py}"
    );
}

/// An `AsyncMethod` adapter param typed as a public dataclass must be converted to the native
/// pyclass with `_to_rust_<snake>` before the engine call — the engine has no `FromPyObject`
/// path that accepts the dataclass directly.
#[test]
fn async_method_adapter_converts_a_dataclass_argument() {
    let api = ApiSurface {
        types: dataclass_types(),
        ..ApiSurface::default()
    };
    let adapter = AdapterConfig {
        name: "fetch_data".to_owned(),
        pattern: AdapterPattern::AsyncMethod,
        core_path: "sample_core::Client::fetch_data".to_owned(),
        params: vec![AdapterParam {
            name: "req".to_owned(),
            ty: REQUEST_TYPE.to_owned(),
            optional: false,
        }],
        returns: None,
        error_type: None,
        owner_type: Some(OWNER_TYPE.to_owned()),
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,
        skip_languages: vec![],
    };

    let (api_py, _options_py) = render(&api, &adapter);

    assert!(
        api_py.contains("def _to_rust_request_options("),
        "api.py must define the param converter for a dataclass-typed adapter param:\n{api_py}"
    );
    assert!(
        api_py.contains("_rust_req = _to_rust_request_options(req)"),
        "api.py must convert `req` to the native pyclass before the engine call:\n{api_py}"
    );
    assert!(
        api_py.contains("return await engine.fetch_data(_rust_req)"),
        "api.py must forward the converted value, not the raw dataclass, to the engine call:\n{api_py}"
    );
}

/// A streaming adapter with more than one param (or no `request_type`) takes the general
/// param-forwarding path rather than the single-field request-construction shortcut. A
/// dataclass-typed param on that path must convert exactly like the `AsyncMethod` case.
#[test]
fn streaming_adapter_converts_a_dataclass_argument_on_the_general_param_path() {
    let api = ApiSurface {
        types: dataclass_types(),
        ..ApiSurface::default()
    };
    let adapter = AdapterConfig {
        name: "watch".to_owned(),
        pattern: AdapterPattern::Streaming,
        core_path: "sample_core::Client::watch".to_owned(),
        params: vec![AdapterParam {
            name: "req".to_owned(),
            ty: REQUEST_TYPE.to_owned(),
            optional: false,
        }],
        returns: None,
        error_type: None,
        owner_type: Some(OWNER_TYPE.to_owned()),
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        // No `request_type`: the single-field request-construction shortcut never triggers, so
        // this exercises the general param-forwarding path used by every other adapter shape.
        request_type: None,
        skip_languages: vec![],
    };

    let (api_py, _options_py) = render(&api, &adapter);

    assert!(
        api_py.contains("_rust_req = _to_rust_request_options(req)"),
        "api.py must convert `req` to the native pyclass before the engine call:\n{api_py}"
    );
    assert!(
        api_py.contains("async for item in engine.watch(_rust_req):"),
        "api.py must forward the converted value, not the raw dataclass, to the engine call:\n{api_py}"
    );
}

/// A param whose type is not a public dataclass (an opaque handle, in this fixture) must pass
/// through unchanged — the conversion only applies to the exact set of types that have a
/// `_to_rust_<snake>` converter to call.
#[test]
fn adapter_param_of_a_non_dataclass_type_is_not_converted() {
    let api = ApiSurface {
        types: dataclass_types(),
        ..ApiSurface::default()
    };
    let adapter = AdapterConfig {
        name: "fetch_data".to_owned(),
        pattern: AdapterPattern::AsyncMethod,
        core_path: "sample_core::Client::fetch_data".to_owned(),
        params: vec![AdapterParam {
            name: "key".to_owned(),
            ty: "String".to_owned(),
            optional: false,
        }],
        returns: None,
        error_type: None,
        owner_type: Some(OWNER_TYPE.to_owned()),
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,
        skip_languages: vec![],
    };

    let (api_py, _options_py) = render(&api, &adapter);

    assert!(
        api_py.contains("return await engine.fetch_data(key)"),
        "a non-dataclass param must be forwarded unchanged:\n{api_py}"
    );
    assert!(
        !api_py.contains("_to_rust_"),
        "no converter should be emitted when no adapter param is a dataclass type:\n{api_py}"
    );
}
