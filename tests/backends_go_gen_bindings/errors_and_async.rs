//! Go error-type and async-function generation coverage, split from the crate root.

use super::{make_config, make_field};
use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::ir::*;

#[test]
fn test_error_types() {
    let backend = GoBackend;

    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "GoError".to_string(),
            rust_path: "test_lib::GoError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "NotFound".to_string(),
                    error_code: None,
                    fields: vec![],
                    doc: "Resource not found".to_string(),
                    message_template: Some("not found".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    error_code: None,
                    fields: vec![make_field("reason", TypeRef::String, false)],
                    doc: "Invalid input provided".to_string(),
                    message_template: Some("invalid input: {reason}".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    is_tuple: false,
                },
            ],
            doc: "Error type for library".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("errors") || content.contains("Error"),
        "Should import or reference errors package"
    );
    assert!(
        content.contains("GoError") || content.contains("lastError"),
        "Should generate error-related code"
    );
}

#[test]
fn coded_error_keeps_the_native_message_and_still_matches_its_sentinel() {
    let backend = GoBackend;

    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "GoError".to_string(),
            rust_path: "test_lib::GoError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Timeout".to_string(),
                error_code: Some(1014),
                fields: vec![
                    make_field("elapsed_ms", TypeRef::String, false),
                    make_field("limit_ms", TypeRef::String, false),
                ],
                doc: "Extraction timed out".to_string(),
                message_template: Some("Extraction timed out after {elapsed_ms}ms (limit: {limit_ms}ms)".to_string()),
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
            }],
            doc: "Error type for library".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend
        .generate_bindings(&api, &make_config())
        .expect("Generation should succeed");
    let content = &files[0].content;

    assert!(
        content.contains("e.sentinel = ErrTimeout"),
        "the coded variant must still resolve to its sentinel: {content}"
    );
    assert!(
        content.contains("e := &GoError{Code: fmt.Sprintf(\"%d\", code)}"),
        "lastError must build the typed *Error rather than a flattened fmt.Errorf: {content}"
    );
    assert!(
        content.contains("func (e *GoError) Unwrap() error { return e.sentinel }"),
        "errors.Is must keep matching the sentinel through Unwrap"
    );
    assert!(
        !content.contains("nativeError"),
        "the typed *Error subsumes the private nativeError wrapper"
    );

    let context_read = content.find("C.test_last_error_context()").expect("reads context");
    let switch_start = content.find("\tswitch code {").expect("emits the code switch");
    assert!(
        context_read < switch_start,
        "the context must be read before the switch, or coded errors return early and lose it"
    );
}

#[test]
fn test_async_function() {
    let backend = GoBackend;

    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "async_process".to_string(),
            rust_path: "test_lib::async_process".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "input".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: alef::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("Error".to_string()),
            doc: "Process data asynchronously".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    assert!(content.contains("func AsyncProcess("), "Should define async function");
    assert!(
        content.contains("AsyncProcess"),
        "Async function should be included in generated code"
    );
}
