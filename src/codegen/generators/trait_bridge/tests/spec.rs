use super::helpers::*;
use crate::codegen::generators::trait_bridge::*;
use crate::core::ir::{PrimitiveType, TypeRef};
use std::collections::HashMap;

#[test]
fn test_wrapper_name() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    assert_eq!(spec.wrapper_name(), "PyOcrBackendBridge");
}

#[test]
fn test_trait_snake() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    assert_eq!(spec.trait_snake(), "ocr_backend");
}

#[test]
fn test_trait_path_replaces_hyphens() {
    let trait_def = make_type_def("OcrBackend", "my-lib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    assert_eq!(spec.trait_path(), "my_lib::OcrBackend");
}

#[test]
fn test_required_methods_filters_no_default_impl() {
    let methods = vec![
        make_method("process", vec![], TypeRef::String, false, false, None, None),
        make_method("initialize", vec![], TypeRef::Unit, false, true, None, None),
        make_method("detect", vec![], TypeRef::String, false, false, None, None),
    ];
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let required = spec.required_methods();
    assert_eq!(required.len(), 2);
    assert!(required.iter().any(|m| m.name == "process"));
    assert!(required.iter().any(|m| m.name == "detect"));
}

#[test]
fn test_optional_methods_filters_has_default_impl() {
    let methods = vec![
        make_method("process", vec![], TypeRef::String, false, false, None, None),
        make_method("initialize", vec![], TypeRef::Unit, false, true, None, None),
        make_method("shutdown", vec![], TypeRef::Unit, false, true, None, None),
    ];
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let optional = spec.optional_methods();
    assert_eq!(optional.len(), 2);
    assert!(optional.iter().any(|m| m.name == "initialize"));
    assert!(optional.iter().any(|m| m.name == "shutdown"));
}

#[test]
fn test_error_path() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    assert_eq!(spec.error_path(), "mylib::MyError");
}
