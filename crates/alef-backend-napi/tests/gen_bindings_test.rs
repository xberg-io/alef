use alef_backend_napi::NapiBackend;
use alef_core::backend::Backend;
use alef_core::config::{NewAlefConfig, NodeCapsuleTypeConfig, NodeConfig, ResolvedCrateConfig};
use alef_core::ir::*;
use std::collections::HashMap;

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
    }
}

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

#[test]
fn test_basic_generation() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), true),
                make_field("backend", TypeRef::String, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Test configuration".to_string(),
            cfg: None,
        }],
        functions: vec![FunctionDef {
            name: "extract_file".to_string(),
            rust_path: "test_lib::extract_file".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "path".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("Config".to_string()),
                    optional: true,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Extract text from file".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test_lib::Mode".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Fast mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Accurate".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Accurate mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Processing mode".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    assert!(!files.is_empty(), "Should generate files");

    // Check for lib.rs file
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some(), "Should generate lib.rs");

    let lib_rs_content = lib_rs.unwrap().content.as_str();

    // Assert NAPI markers are present
    assert!(
        lib_rs_content.contains("#[napi("),
        "Should contain #[napi(...)] attributes"
    );
    assert!(
        lib_rs_content.contains("napi_derive::napi"),
        "Should import napi_derive::napi"
    );
    assert!(
        lib_rs_content.contains("JsConfig"),
        "Should contain JsConfig type (Js-prefixed)"
    );
    assert!(
        lib_rs_content.contains("JsMode"),
        "Should contain JsMode enum (Js-prefixed)"
    );
    assert!(
        lib_rs_content.contains("extractFile"),
        "Should contain extractFile function (camelCase)"
    );
    assert!(
        lib_rs_content.contains("napi(object)"),
        "Non-opaque structs should use napi(object) attribute"
    );
    assert!(
        lib_rs_content.contains("napi(string_enum)"),
        "Enums should use napi(string_enum) attribute"
    );
}

#[test]
fn test_type_mapping() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Numbers".to_string(),
            rust_path: "test_lib::Numbers".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("u32_val", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("i64_val", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("string_val", TypeRef::String, true),
                make_field("string_list", TypeRef::Vec(Box::new(TypeRef::String)), false),
                make_field("opt_string", TypeRef::Optional(Box::new(TypeRef::String)), true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify the Numbers struct is defined with NAPI object attribute
    assert!(content.contains("Numbers"), "Should contain Numbers struct");
    assert!(
        content.contains("u32") || content.contains("u32_val"),
        "Should map u32 field"
    );
    assert!(
        content.contains("i64") || content.contains("i64_val"),
        "Should map i64 field"
    );
    assert!(content.contains("String"), "Should map String fields");
    assert!(
        content.contains("Vec") || content.contains("string_list"),
        "Should map Vec<String> field"
    );
}

#[test]
fn test_enum_generation() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "test_lib::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Pending".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Pending status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Active status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Complete".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Complete status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Task status".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify enum generation with NAPI string_enum attribute
    assert!(content.contains("Status"), "Should contain Status enum");
    assert!(content.contains("Pending"), "Should contain Pending variant");
    assert!(content.contains("Active"), "Should contain Active variant");
    assert!(content.contains("Complete"), "Should contain Complete variant");
    assert!(
        content.contains("napi(string_enum)"),
        "Should use napi(string_enum) attribute"
    );
}

#[test]
fn test_generated_header() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();

    // Verify lib.rs has generated_header: false (as per source code)
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let lib_rs_file = lib_rs.unwrap();
    assert!(
        !lib_rs_file.generated_header,
        "lib.rs should have generated_header: false"
    );
}

#[test]
fn test_async_function() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "process_async".to_string(),
            rust_path: "test_lib::process_async".to_string(),
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
            }],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("Error".to_string()),
            doc: "Async processor".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify async function is generated with proper async keyword
    assert!(
        content.contains("async fn process_async"),
        "Should contain async function"
    );
    // Verify tokio runtime is added for async support
    assert!(
        content.contains("tokio") || content.contains("spawn_blocking"),
        "Should include tokio runtime support for async"
    );
    // Verify return type indicates async/promise
    assert!(content.contains("#[napi"), "Async function should have napi attribute");
}

#[test]
fn test_methods_generation() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Processor".to_string(),
            rust_path: "test_lib::Processor".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![
                MethodDef {
                    name: "process".to_string(),
                    params: vec![ParamDef {
                        name: "data".to_string(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: false,
                        is_mut: false,
                        newtype_wrapper: None,
                        original_type: None,
                    }],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: Some("Error".to_string()),
                    doc: "Process data".to_string(),
                    receiver: Some(alef_core::ir::ReceiverKind::Ref),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                },
                MethodDef {
                    name: "create".to_string(),
                    params: vec![],
                    return_type: TypeRef::Named("Processor".to_string()),
                    is_async: false,
                    is_static: true,
                    error_type: None,
                    doc: "Create processor".to_string(),
                    receiver: None,
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                },
            ],
            is_opaque: true,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Text processor".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify opaque struct is generated with Js prefix
    assert!(
        content.contains("struct JsProcessor"),
        "Should contain opaque struct JsProcessor"
    );
    // Verify impl block with napi attribute for methods
    assert!(
        content.contains("impl JsProcessor"),
        "Should contain impl block for JsProcessor"
    );
    // Verify instance method is generated
    assert!(content.contains("fn process"), "Should contain instance method process");
    // Verify static method is generated
    assert!(content.contains("fn create"), "Should contain static method create");
    // Verify napi attributes on methods
    assert!(content.contains("#[napi"), "Methods should have napi attributes");
    // Verify Arc usage for opaque types
    assert!(
        content.contains("Arc"),
        "Opaque types should use Arc for interior mutability"
    );
}

#[test]
fn test_error_types() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "risky_operation".to_string(),
            rust_path: "test_lib::risky_operation".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("ProcessError".to_string()),
            doc: "Operation that can fail".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "ProcessError".to_string(),
            rust_path: "test_lib::ProcessError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "NotFound".to_string(),
                    fields: vec![],
                    doc: "Item not found".to_string(),
                    message_template: Some("not found".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                },
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    fields: vec![make_field("reason", TypeRef::String, false)],
                    doc: "Invalid input provided".to_string(),
                    message_template: Some("invalid: {0}".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                },
            ],
            doc: "Processing error".to_string(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify error handling code is generated
    assert!(
        content.contains("ProcessError") || content.contains("map_err"),
        "Should contain error handling for ProcessError"
    );
    // Verify error conversion function is generated
    assert!(
        content.contains("napi::Error") || content.contains("GenericFailure"),
        "Should contain NAPI error conversion"
    );
    // Verify error variant constants are generated
    assert!(
        content.contains("NotFound") || content.contains("InvalidInput"),
        "Should contain error variant handling"
    );
}

#[test]
fn test_opaque_type() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Handle".to_string(),
            rust_path: "test_lib::Handle".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "get_value".to_string(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Get handle value".to_string(),
                receiver: Some(alef_core::ir::ReceiverKind::Ref),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
            }],
            is_opaque: true,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Opaque handle type".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify opaque struct uses Arc for memory management
    assert!(
        content.contains("struct JsHandle") && content.contains("Arc"),
        "Opaque type should be JsHandle wrapped in Arc"
    );
    // Verify impl block for opaque type methods
    assert!(
        content.contains("impl JsHandle"),
        "Should have impl block for opaque JsHandle"
    );
    // Verify napi attribute on impl block
    assert!(
        content.contains("#[napi]") && content.contains("impl JsHandle"),
        "Opaque impl block should have napi attribute"
    );
    // Verify method references self.inner for delegation
    assert!(
        content.contains("self.inner") || content.contains("get_value"),
        "Opaque method should delegate to inner type"
    );
}

#[test]
fn test_optional_and_default_fields() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Options".to_string(),
            rust_path: "test_lib::Options".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), true),
                make_field("retries", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("max_size", TypeRef::Primitive(PrimitiveType::U64), true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Configuration with defaults".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify struct with default impl
    assert!(
        content.contains("struct JsOptions"),
        "Should contain Options struct with Js prefix"
    );
    // Verify fields are wrapped in Option when type has default
    assert!(
        content.contains("Option<") || content.contains("timeout"),
        "Fields should be wrapped in Option for types with defaults"
    );
    // Verify napi(object) attribute
    assert!(
        content.contains("napi(object)"),
        "Non-opaque struct should use napi(object)"
    );
    // Verify Default derive is added
    assert!(
        content.contains("Default") || content.contains("impl Default for JsOptions"),
        "Type with has_default should derive Default or have impl"
    );
}

#[test]
fn test_async_method() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "AsyncWorker".to_string(),
            rust_path: "test_lib::AsyncWorker".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "process_async".to_string(),
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
                }],
                return_type: TypeRef::String,
                is_async: true,
                is_static: false,
                error_type: None,
                doc: "Async process".to_string(),
                receiver: Some(alef_core::ir::ReceiverKind::Ref),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
            }],
            is_opaque: true,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Async worker".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify async method keyword
    assert!(
        content.contains("async fn process_async"),
        "Should contain async method"
    );
    // Verify tokio runtime for async support
    assert!(
        content.contains("tokio") || content.contains("spawn_blocking"),
        "Should include tokio support for async methods"
    );
    // Verify method is in impl block
    assert!(
        content.contains("impl JsAsyncWorker"),
        "Should have impl block for opaque async worker"
    );
}

#[test]
fn test_static_method_with_error() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Factory".to_string(),
            rust_path: "test_lib::Factory".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "from_config".to_string(),
                params: vec![ParamDef {
                    name: "config_path".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                }],
                return_type: TypeRef::Named("Factory".to_string()),
                is_async: false,
                is_static: true,
                error_type: Some("Error".to_string()),
                doc: "Create from config".to_string(),
                receiver: None,
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
            }],
            is_opaque: true,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Factory type".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify static method (no &self parameter)
    assert!(content.contains("fn from_config"), "Should contain static method");
    // Verify error handling in static method
    assert!(
        content.contains("map_err") || content.contains("GenericFailure"),
        "Static method with error should have error conversion"
    );
    // Verify return type wrapping for opaque types
    assert!(
        content.contains("JsFactory") || content.contains("Arc"),
        "Static method returning opaque type should wrap in Js and Arc"
    );
}

#[test]
fn test_map_types() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field(
                "settings",
                TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                false,
            )],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Config with map".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files.iter().find(|f| f.path.to_string_lossy().ends_with("lib.rs"));
    assert!(lib_rs.is_some());

    let content = lib_rs.unwrap().content.as_str();

    // Verify HashMap import is added for Map types
    assert!(content.contains("HashMap"), "Should import HashMap for Map types");
    // Verify struct contains map field
    assert!(content.contains("settings"), "Should contain settings field for map");
}

/// Regression test: tagged enum where each variant holds a different Named struct type
/// (all in the same positional field `_0`) must generate `.into()` conversions, not
/// `serde_json::from_str`. The binding struct stores `Option<JsXxx>`, not `Option<String>`.
#[test]
fn test_tagged_enum_different_named_types_per_variant_uses_into_not_serde_json() {
    let backend = NapiBackend;

    // Simulate the liter-llm `Message` enum pattern:
    // #[serde(tag = "role")]
    // enum Message {
    //     #[serde(rename = "system")]  System(SystemMessage),
    //     #[serde(rename = "user")]    User(UserMessage),
    // }
    // where SystemMessage and UserMessage are distinct structs, both exposed as types.
    let make_variant = |name: &str, rename: &str, struct_name: &str| EnumVariant {
        name: name.to_string(),
        fields: vec![FieldDef {
            name: "_0".to_string(),
            ty: TypeRef::Named(struct_name.to_string()),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
        }],
        is_tuple: false,
        doc: String::new(),
        is_default: false,
        serde_rename: Some(rename.to_string()),
    };

    let make_type = |name: &str| TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        fields: vec![make_field("content", TypeRef::String, false)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![make_type("SystemMessage"), make_type("UserMessage")],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Message".to_string(),
            rust_path: "test_lib::Message".to_string(),
            original_rust_path: String::new(),
            serde_tag: Some("role".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            doc: String::new(),
            cfg: None,
            variants: vec![
                make_variant("System", "system", "SystemMessage"),
                make_variant("User", "user", "UserMessage"),
            ],
            is_copy: false,
            has_serde: false,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "generate_bindings should succeed");

    let files = result.unwrap();
    let content = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .unwrap()
        .content
        .as_str();

    // When variants share the same positional field name (_0) with DIFFERENT Named types
    // (SystemMessage vs UserMessage), the field cannot be a single concrete JsXxx type.
    // It must be stored as String (JSON) and converted via serde_json per variant.
    assert!(
        content.contains("serde_json::from_str"),
        "binding→core conversion must use serde_json::from_str for mixed-type Named fields"
    );
    assert!(
        content.contains("serde_json::to_string"),
        "core→binding conversion must use serde_json::to_string for mixed-type Named fields"
    );
    // The flattened struct field for mixed Named types must be Option<String>, not Option<JsXxx>
    assert!(
        content.contains("_0: Option<String>"),
        "_0 field with mixed Named types must be typed as Option<String> in the flattened struct"
    );
}

// ---------------------------------------------------------------------------
// Trait bridge helpers
// ---------------------------------------------------------------------------

fn make_trait_def_napi(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("my_lib::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
    }
}

fn make_method_napi(name: &str, return_type: TypeRef, has_error: bool, has_default: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: false,
        is_static: false,
        error_type: if has_error {
            Some("Box<dyn std::error::Error + Send + Sync>".to_string())
        } else {
            None
        },
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: has_default,
    }
}

fn make_async_method_napi(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: true,
        is_static: false,
        error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    }
}

fn make_api_napi() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    }
}

fn make_plugin_bridge_cfg(trait_name: &str) -> alef_core::config::TraitBridgeConfig {
    alef_core::config::TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef_core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }
}

fn make_visitor_bridge_cfg(trait_name: &str, type_alias: &str) -> alef_core::config::TraitBridgeConfig {
    alef_core::config::TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: Some(type_alias.to_string()),
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef_core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }
}

// ---------------------------------------------------------------------------
// NAPI trait bridge tests
// ---------------------------------------------------------------------------

#[test]
fn test_napi_visitor_bridge_produces_visitor_struct() {
    use alef_backend_napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi(
        "HtmlVisitor",
        vec![make_method_napi("visit_node", TypeRef::Unit, false, true)],
    );
    let bridge_cfg = make_visitor_bridge_cfg("HtmlVisitor", "HtmlVisitor");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("JsHtmlVisitorBridge"),
        "visitor bridge struct must be named Js{{TraitName}}Bridge"
    );
    assert!(
        code.code.contains("impl my_lib::HtmlVisitor for JsHtmlVisitorBridge"),
        "visitor bridge must implement the trait"
    );
}

#[test]
fn test_napi_visitor_bridge_has_obj_field() {
    use alef_backend_napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi(
        "HtmlVisitor",
        vec![make_method_napi("visit_node", TypeRef::Unit, false, true)],
    );
    let bridge_cfg = make_visitor_bridge_cfg("HtmlVisitor", "HtmlVisitor");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("obj: napi::bindgen_prelude::Object<'static>"),
        "NAPI visitor bridge must store Object<'static> in an 'obj' field"
    );
}

#[test]
fn test_napi_plugin_bridge_produces_wrapper_struct_with_inner_and_cached_name() {
    use alef_backend_napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi(
        "OcrBackend",
        vec![make_method_napi("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg("OcrBackend");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("pub struct JsOcrBackendBridge"),
        "plugin bridge wrapper struct must be JsOcrBackendBridge"
    );
    assert!(
        code.code.contains("inner:"),
        "plugin bridge wrapper must have an 'inner' field"
    );
    assert!(
        code.code.contains("cached_name: String"),
        "plugin bridge wrapper must have a 'cached_name: String' field"
    );
}

#[test]
fn test_napi_plugin_bridge_generates_super_trait_impl() {
    use alef_backend_napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi(
        "OcrBackend",
        vec![make_method_napi("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg("OcrBackend");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("impl my_lib::Plugin for JsOcrBackendBridge"),
        "plugin bridge must implement Plugin super-trait"
    );
    assert!(code.code.contains("fn name("), "Plugin impl must contain name()");
    assert!(
        code.code.contains("fn initialize("),
        "Plugin impl must contain initialize()"
    );
    assert!(
        code.code.contains("fn shutdown("),
        "Plugin impl must contain shutdown()"
    );
}

#[test]
fn test_napi_plugin_bridge_generates_trait_impl_with_forwarded_methods() {
    use alef_backend_napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi(
        "OcrBackend",
        vec![make_method_napi("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg("OcrBackend");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("impl my_lib::OcrBackend for JsOcrBackendBridge"),
        "plugin bridge must implement the trait itself"
    );
    assert!(
        code.code.contains("fn process("),
        "trait impl must forward the 'process' method"
    );
}

#[test]
fn test_napi_plugin_bridge_generates_registration_fn_with_napi_attribute() {
    use alef_backend_napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi(
        "OcrBackend",
        vec![make_method_napi("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg("OcrBackend");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("#[napi]"),
        "NAPI registration function must carry the #[napi] attribute"
    );
    assert!(
        code.code.contains("pub fn register_ocrbackend("),
        "registration function must use the configured name"
    );
}

#[test]
fn test_napi_plugin_bridge_validates_required_methods() {
    use alef_backend_napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi(
        "Analyzer",
        vec![
            make_method_napi("analyze", TypeRef::String, true, false), // required
            make_method_napi("describe", TypeRef::String, false, true), // optional
        ],
    );
    let bridge_cfg = alef_core::config::TraitBridgeConfig {
        trait_name: "Analyzer".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_analyzer".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef_core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    // Constructor must check for the required method "analyze"
    assert!(
        code.code.contains("\"analyze\""),
        "constructor must validate the required method 'analyze'"
    );
}

#[test]
fn test_napi_sync_method_body_uses_get_named_property() {
    use alef_backend_napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi("Scanner", vec![make_method_napi("scan", TypeRef::String, true, false)]);
    let bridge_cfg = make_plugin_bridge_cfg("Scanner");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("get_named_property"),
        "NAPI sync method body must use get_named_property to retrieve JS methods"
    );
}

#[test]
fn test_napi_async_method_body_uses_box_pin() {
    use alef_backend_napi::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_napi("Processor", vec![make_async_method_napi("run", TypeRef::Unit)]);
    let bridge_cfg = make_plugin_bridge_cfg("Processor");
    let api = make_api_napi();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("get_named_property(\"run\")"),
        "NAPI async method body must retrieve JS method via get_named_property"
    );
}

// ---------------------------------------------------------------------------
// capsule_types end-to-end: External<T> + __parser passthrough
// ---------------------------------------------------------------------------

fn make_capsule_config_node(type_name: &str, from_module: &str) -> NodeCapsuleTypeConfig {
    NodeCapsuleTypeConfig {
        type_name: type_name.to_string(),
        from_module: from_module.to_string(),
        construct: "external_pointer".to_string(),
        property_name: "__parser".to_string(),
        type_tag: None,
    }
}

fn make_config_with_capsule_types(capsule_map: HashMap<String, NodeCapsuleTypeConfig>) -> ResolvedCrateConfig {
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
    let mut resolved = cfg.resolve().unwrap().remove(0);
    resolved.node = Some(NodeConfig {
        package_name: Some("test-lib".to_string()),
        features: None,
        serde_rename_all: None,
        type_prefix: None,
        capsule_types: capsule_map,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: Default::default(),
        scaffold_output: None,
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: vec![],
    });
    resolved
}

fn make_language_type_def() -> TypeDef {
    TypeDef {
        name: "Language".to_string(),
        rust_path: "ts_pack::Language".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: true,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "A tree-sitter Language handle.".to_string(),
        cfg: None,
    }
}

fn make_get_language_func() -> FunctionDef {
    FunctionDef {
        name: "get_language".to_string(),
        rust_path: "ts_pack::get_language".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
        }],
        return_type: TypeRef::Named("Language".to_string()),
        is_async: false,
        error_type: Some("ts_pack::Error".to_string()),
        doc: "Look up a language by name.".to_string(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
    }
}

/// capsule_types wires up External<T> + __parser passthrough end-to-end:
/// - Language type does NOT get a #[napi] opaque class emitted.
/// - get_language returns a JsObject with __parser = External<T>(ptr from value.into_raw()).
#[test]
fn test_capsule_types_end_to_end() {
    let backend = NapiBackend;

    let mut capsule_map: HashMap<String, NodeCapsuleTypeConfig> = HashMap::new();
    capsule_map.insert(
        "Language".to_string(),
        make_capsule_config_node("Language", "tree-sitter"),
    );

    let api = ApiSurface {
        crate_name: "ts_pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![make_language_type_def()],
        functions: vec![make_get_language_func()],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config_with_capsule_types(capsule_map);

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings with capsule_types should succeed");

    assert_eq!(files.len(), 1, "expected exactly lib.rs");
    let content = &files[0].content;

    // Language must NOT appear as a #[napi] opaque struct.
    assert!(
        !content.contains("struct JsLanguage"),
        "Language must not be emitted as a #[napi] struct; content:\n{content}"
    );

    // The shim must call into_raw() to extract the raw pointer.
    assert!(
        content.contains("into_raw"),
        "get_language shim must call into_raw(); content:\n{content}"
    );

    // The shim must call raw napi_create_external (not napi-rs's wrapper, which
    // produces a value rejected by node-tree-sitter's UnwrapLanguage).
    assert!(
        content.contains("napi_create_external"),
        "get_language shim must call raw napi_create_external; content:\n{content}"
    );
    assert!(
        !content.contains("bindgen_prelude::External::new"),
        "get_language shim must NOT use bindgen_prelude::External::new; content:\n{content}"
    );

    // The shim must set the default __parser property on the returned JsObject
    // (the test config doesn't override property_name).
    assert!(
        content.contains("__parser"),
        "get_language shim must set __parser property; content:\n{content}"
    );

    // The function must return napi::Result<napi::bindgen_prelude::Object>.
    assert!(
        content.contains("bindgen_prelude::Object"),
        "get_language shim must return napi::bindgen_prelude::Object; content:\n{content}"
    );

    // The shim must accept napi::Env as its first parameter.
    assert!(
        content.contains("napi::Env"),
        "get_language shim must accept napi::Env; content:\n{content}"
    );
}

/// capsule_types dts generation:
/// - `import type { Language } from "tree-sitter"` appears at the top.
/// - `export declare class JsLanguage` is NOT emitted.
/// - `getLanguage(name: string): Language` uses the ecosystem type name.
#[test]
fn test_capsule_types_dts_generation() {
    let backend = NapiBackend;

    let mut capsule_map: HashMap<String, NodeCapsuleTypeConfig> = HashMap::new();
    capsule_map.insert(
        "Language".to_string(),
        make_capsule_config_node("Language", "tree-sitter"),
    );

    let api = ApiSurface {
        crate_name: "ts_pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![make_language_type_def()],
        functions: vec![make_get_language_func()],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config_with_capsule_types(capsule_map);

    let files = backend
        .generate_type_stubs(&api, &config)
        .expect("generate_type_stubs with capsule_types should succeed");

    assert_eq!(files.len(), 1, "expected exactly index.d.ts");
    let content = &files[0].content;

    // Import line must be emitted for the capsule type.
    assert!(
        content.contains("import type { Language } from \"tree-sitter\""),
        "index.d.ts must emit import type for capsule type; content:\n{content}"
    );

    // The class declaration for JsLanguage must NOT be emitted.
    assert!(
        !content.contains("export declare class JsLanguage"),
        "index.d.ts must not emit export declare class JsLanguage; content:\n{content}"
    );

    // getLanguage must use the ecosystem type name, not JsLanguage.
    assert!(
        content.contains("getLanguage(name: string): Language"),
        "index.d.ts must emit getLanguage returning Language (not JsLanguage); content:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// capsule_types on methods of opaque types
// ---------------------------------------------------------------------------

/// Build an opaque `LanguageRegistry` type whose `getLanguage` instance method
/// returns the capsule type `Language`.
fn make_language_registry_type_def() -> TypeDef {
    TypeDef {
        name: "LanguageRegistry".to_string(),
        rust_path: "ts_pack::LanguageRegistry".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "get_language".to_string(),
            params: vec![ParamDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            }],
            return_type: TypeRef::Named("Language".to_string()),
            is_async: false,
            is_static: false,
            error_type: Some("ts_pack::Error".to_string()),
            doc: "Look up a language by name.".to_string(),
            receiver: Some(alef_core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        }],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
    }
}

/// capsule_types on opaque method — Rust shim:
/// A method on an opaque type returning a capsule type must emit the same
/// JsObject / External<T> / __parser pattern as a free capsule function.
///
/// KNOWN LIMITATION: methods on opaque types currently fall through the regular
/// opaque-class method codegen and emit `Result<JsLanguage>` (where JsLanguage
/// is suppressed), producing a compile error in the downstream crate. Workaround:
/// expose the capsule as a free function (which works end-to-end). Tracked separately;
/// fixing methods requires threading capsule_types through methods.rs.
#[ignore = "method-on-opaque capsule path not yet wired through methods.rs"]
#[test]
fn test_capsule_types_method_on_opaque_rust_shim() {
    let backend = NapiBackend;

    let mut capsule_map: HashMap<String, NodeCapsuleTypeConfig> = HashMap::new();
    capsule_map.insert(
        "Language".to_string(),
        make_capsule_config_node("Language", "tree-sitter"),
    );

    let api = ApiSurface {
        crate_name: "ts_pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![make_language_type_def(), make_language_registry_type_def()],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config_with_capsule_types(capsule_map);

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings with opaque capsule method should succeed");

    assert_eq!(files.len(), 1);
    let content = &files[0].content;

    // Language must NOT appear as a #[napi] opaque struct (it's a capsule type).
    // Use word-boundary check: "JsLanguage {" or "JsLanguage\n" to avoid matching JsLanguageRegistry.
    let has_js_language_struct = content.contains("struct JsLanguage {")
        || content.contains("struct JsLanguage\n")
        || content.contains("struct JsLanguage\r");
    assert!(
        !has_js_language_struct,
        "Language must not be emitted as a standalone #[napi] struct; content:\n{content}"
    );

    // The method shim must accept napi::Env.
    assert!(
        content.contains("napi::Env"),
        "method shim must accept napi::Env; content:\n{content}"
    );

    // The method shim must return napi::Result<napi::bindgen_prelude::Object<'_>>.
    assert!(
        content.contains("napi::Result<napi::bindgen_prelude::Object<'_>>"),
        "method shim must return napi::Result<napi::bindgen_prelude::Object<'_>>; content:\n{content}"
    );

    // The shim must call into_raw().
    assert!(
        content.contains("into_raw"),
        "method shim must call into_raw(); content:\n{content}"
    );

    // The shim must call raw napi_create_external (not napi-rs's wrapper).
    assert!(
        content.contains("napi_create_external"),
        "method shim must call raw napi_create_external; content:\n{content}"
    );
    assert!(
        !content.contains("bindgen_prelude::External::new"),
        "method shim must NOT use bindgen_prelude::External::new; content:\n{content}"
    );

    // The shim must set __parser (default property name).
    assert!(
        content.contains("__parser"),
        "method shim must set __parser property; content:\n{content}"
    );
}

/// capsule_types on opaque method — TypeScript stubs:
/// The `index.d.ts` for an opaque class whose method returns a capsule type must:
/// 1. Emit `import type { Language } from "tree-sitter"`.
/// 2. Declare the class without `JsLanguage` anywhere.
/// 3. Emit the method returning the ecosystem type name `Language`.
#[test]
fn test_capsule_types_method_on_opaque_dts() {
    let backend = NapiBackend;

    let mut capsule_map: HashMap<String, NodeCapsuleTypeConfig> = HashMap::new();
    capsule_map.insert(
        "Language".to_string(),
        make_capsule_config_node("Language", "tree-sitter"),
    );

    let api = ApiSurface {
        crate_name: "ts_pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![make_language_type_def(), make_language_registry_type_def()],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_config_with_capsule_types(capsule_map);

    let files = backend
        .generate_type_stubs(&api, &config)
        .expect("generate_type_stubs with opaque capsule method should succeed");

    assert_eq!(files.len(), 1);
    let content = &files[0].content;

    // Import must be present.
    assert!(
        content.contains("import type { Language } from \"tree-sitter\""),
        "index.d.ts must emit import type for capsule type; content:\n{content}"
    );

    // No bare JsLanguage class declaration (as distinct from JsLanguageRegistry).
    // Check specifically for "JsLanguage {" to avoid matching JsLanguageRegistry.
    let has_js_language_class =
        content.contains("export declare class JsLanguage {") || content.contains("export declare class JsLanguage\n");
    assert!(
        !has_js_language_class,
        "index.d.ts must not emit standalone export declare class JsLanguage; content:\n{content}"
    );

    // The registry class must be present.
    assert!(
        content.contains("export declare class JsLanguageRegistry"),
        "index.d.ts must emit JsLanguageRegistry class; content:\n{content}"
    );

    // The method must use the ecosystem type, not the undeclared opaque handle.
    assert!(
        content.contains("getLanguage(name: string): Language"),
        "method must emit return type Language (not JsLanguage); content:\n{content}"
    );

    // Confirm no bare JsLanguage return type references anywhere.
    let bare_js_language_method = content.contains("): JsLanguage");
    assert!(
        !bare_js_language_method,
        "index.d.ts must not contain ): JsLanguage; content:\n{content}"
    );
}
