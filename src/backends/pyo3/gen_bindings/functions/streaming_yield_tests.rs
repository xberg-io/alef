//! The declared yield type of a generated streaming wrapper and the type its body actually
//! yields must name the same class in the same module.
//!
//! These tests assert that *relationship* rather than a fixed literal: they read the module a
//! name resolves to out of the generated `api.py` import block and compare it against the module
//! of the yielded expression's type. A wrong annotation therefore cannot pass by matching a
//! hard-coded string.

use std::collections::HashMap;

use super::gen_api_py;
use crate::core::config::{AdapterConfig, AdapterParam, AdapterPattern, DtoConfig};
use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

const MODULE_NAME: &str = "_internal_bindings";
const PACKAGE_NAME: &str = "sample_pkg";
const ADAPTER_NAME: &str = "watch_stream";
const ITEM_TYPE: &str = "StreamEvent";

/// A fully qualified Python class: the module a name resolves to, plus the class name.
#[derive(Debug, PartialEq, Eq)]
struct QualifiedClass {
    module: String,
    class: String,
}

/// Build a neutral API surface with one streaming adapter over `ITEM_TYPE`.
///
/// `item_is_return_type` decides which module the item type lives in: a return type is not
/// emitted as an `options` dataclass, so it keeps its single native identity.
fn fixture(item_is_return_type: bool) -> (ApiSurface, AdapterConfig) {
    let item = TypeDef {
        name: ITEM_TYPE.to_owned(),
        rust_path: format!("sample_core::{ITEM_TYPE}"),
        has_default: true,
        is_return_type: item_is_return_type,
        fields: vec![FieldDef {
            name: "label".to_owned(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };
    let request = TypeDef {
        name: "StreamRequest".to_owned(),
        rust_path: "sample_core::StreamRequest".to_owned(),
        has_default: true,
        fields: vec![FieldDef {
            name: "topic".to_owned(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };
    let owner = TypeDef {
        name: "Client".to_owned(),
        rust_path: "sample_core::Client".to_owned(),
        is_opaque: true,
        ..TypeDef::default()
    };
    let api = ApiSurface {
        types: vec![item, request, owner],
        ..ApiSurface::default()
    };
    let adapter = AdapterConfig {
        name: ADAPTER_NAME.to_owned(),
        pattern: AdapterPattern::Streaming,
        core_path: "sample_core::Client::watch_stream".to_owned(),
        params: vec![AdapterParam {
            name: "req".to_owned(),
            ty: "StreamRequest".to_owned(),
            optional: false,
        }],
        returns: None,
        error_type: None,
        owner_type: Some("Client".to_owned()),
        item_type: Some(ITEM_TYPE.to_owned()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: Some("sample_core::StreamRequest".to_owned()),
        skip_languages: vec![],
    };
    (api, adapter)
}

fn render(api: &ApiSurface, adapter: &AdapterConfig) -> (String, String) {
    let api_py = gen_api_py(
        api,
        MODULE_NAME,
        PACKAGE_NAME,
        &[],
        &DtoConfig::default(),
        &HashMap::new(),
        &std::collections::BTreeMap::new(),
        std::slice::from_ref(adapter),
        &[],
        &ahash::AHashSet::new(),
        &crate::core::config::ResolvedCrateConfig::default(),
    );
    let options_py =
        crate::backends::pyo3::gen_bindings::types::gen_options_py(api, MODULE_NAME, &DtoConfig::default(), &[]);
    (api_py, options_py)
}

/// Reduce a module path to one identity, so a relative import and an absolute one that name the
/// same module compare equal.
fn canonical_module(module: &str) -> String {
    module
        .trim_start_matches('.')
        .trim_start_matches(&format!("{PACKAGE_NAME}."))
        .to_owned()
}

/// The module an unqualified name in `api.py` resolves to, read out of its import block.
fn resolve_module(api_py: &str, name: &str) -> String {
    for line in api_py.lines() {
        let Some((prefix, imported)) = line.split_once(" import ") else {
            continue;
        };
        let Some(module) = prefix.strip_prefix("from ") else {
            continue;
        };
        if imported.split(", ").any(|item| item.trim() == name) {
            return canonical_module(module);
        }
    }
    panic!("`{name}` is not imported by the generated api.py:\n{api_py}");
}

/// The class named inside the streaming wrapper's `-> AsyncIterator[...]` annotation.
fn declared_yield_class(api_py: &str) -> QualifiedClass {
    let signature = api_py
        .lines()
        .find(|line| line.starts_with(&format!("async def {ADAPTER_NAME}(")))
        .unwrap_or_else(|| panic!("no `{ADAPTER_NAME}` wrapper in:\n{api_py}"));
    let annotation = signature
        .split_once("-> ")
        .and_then(|(_, rest)| rest.strip_suffix(':'))
        .unwrap_or_else(|| panic!("no return annotation on:\n{signature}"));
    let class = annotation
        .strip_prefix("AsyncIterator[")
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or_else(|| panic!("a streaming wrapper must return AsyncIterator[...], got `{annotation}`"));
    QualifiedClass {
        module: resolve_module(api_py, class),
        class: class.to_owned(),
    }
}

/// The class the streaming wrapper's body actually yields.
///
/// `yield item` re-yields the value the native engine object produced, so its type is the
/// pyclass in the native bindings module. `yield converter(item)` yields whatever the
/// converter's own return annotation in `options.py` declares. ~keep
fn yielded_class(api_py: &str, options_py: &str) -> QualifiedClass {
    let yielded = api_py
        .lines()
        .find_map(|line| line.trim().strip_prefix("yield "))
        .unwrap_or_else(|| panic!("the streaming wrapper body yields nothing:\n{api_py}"));
    let Some((converter, argument)) = yielded.split_once('(') else {
        assert_eq!(yielded, "item", "unexpected yielded expression `{yielded}`");
        return QualifiedClass {
            module: canonical_module(MODULE_NAME),
            class: ITEM_TYPE.to_owned(),
        };
    };
    assert_eq!(argument, "item)", "the converter must be applied to the streamed item");
    let signature = options_py
        .lines()
        .find(|line| line.starts_with(&format!("def {converter}(")))
        .unwrap_or_else(|| panic!("`{converter}` is not defined in options.py:\n{options_py}"));
    let return_class = signature
        .split_once("-> ")
        .and_then(|(_, rest)| rest.strip_suffix(':'))
        .unwrap_or_else(|| panic!("no return annotation on:\n{signature}"));
    QualifiedClass {
        module: resolve_module(api_py, converter),
        class: return_class.to_owned(),
    }
}

/// The `-> AsyncIterator[X]` annotation and the yielded value must agree on module and class.
///
/// Covers both item shapes: one emitted as a public `options` dataclass (the body must convert)
/// and one that keeps a single native identity (the body must yield the item unchanged).
#[test]
fn streaming_wrapper_yields_the_type_it_declares() {
    for item_is_return_type in [false, true] {
        let (api, adapter) = fixture(item_is_return_type);
        let (api_py, options_py) = render(&api, &adapter);

        let declared = declared_yield_class(&api_py);
        let yielded = yielded_class(&api_py, &options_py);

        assert_eq!(
            declared, yielded,
            "declared yield type and yielded value disagree \
             (item_is_return_type={item_is_return_type}):\n{api_py}"
        );
    }
}

/// The two item shapes must not collapse into one another: an `options` dataclass item is
/// annotated against `.options`, a native-identity item against the bindings module. Without
/// this, the agreement test above could pass while every item resolved to the same module.
#[test]
fn streaming_item_module_follows_the_options_dataclass_decision() {
    let (dataclass_api, dataclass_adapter) = fixture(false);
    let (dataclass_api_py, _) = render(&dataclass_api, &dataclass_adapter);
    assert_eq!(
        declared_yield_class(&dataclass_api_py).module,
        "options",
        "an item emitted as a public dataclass must be annotated from `.options`:\n{dataclass_api_py}"
    );

    let (native_api, native_adapter) = fixture(true);
    let (native_api_py, _) = render(&native_api, &native_adapter);
    assert_eq!(
        declared_yield_class(&native_api_py).module,
        MODULE_NAME,
        "an item with a single native identity must be annotated from the bindings module:\n{native_api_py}"
    );
}
