//! Test suite for alef-adapters crate covering all adapter patterns and languages.

use alef::adapters::build_adapter_bodies;
use alef::core::config::{AdapterConfig, AdapterParam, AdapterPattern, Language, NewAlefConfig, ResolvedCrateConfig};

/// Helper to create a minimal ResolvedCrateConfig with the given languages.
fn make_config(languages: Vec<Language>) -> ResolvedCrateConfig {
    let lang_strs: Vec<String> = languages.iter().map(|l| format!("\"{l}\"")).collect();
    let langs_toml = lang_strs.join(", ");
    let toml_str = format!(
        r#"
[workspace]
languages = [{langs_toml}]

[[crates]]
name = "test-crate"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_str).expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

/// Test SyncFunction adapter with Python.
/// Asserts that the generated body contains PyErr conversion.
#[test]
fn test_sync_function_python() {
    let mut config = make_config(vec![Language::Python]);
    config.adapters = vec![AdapterConfig {
        name: "convert".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::convert".to_string(),
        params: vec![AdapterParam {
            name: "input".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        returns: Some("String".to_string()),
        error_type: Some("ConvertError".to_string()),
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Python).expect("build failed");

    assert!(bodies.contains_key("convert"), "Expected 'convert' adapter body");
    let body = &bodies["convert"];

    assert!(
        body.contains("PyErr"),
        "Python body should contain PyErr conversion. Got: {}",
        body
    );
    assert!(body.contains("my_crate::convert"), "Body should reference core path");
    assert!(body.contains(".into()"), "Body should convert arguments with .into()");
}

/// Test SyncFunction adapter with Node.
/// Asserts that the generated body contains napi::Error conversion.
#[test]
fn test_sync_function_node() {
    let mut config = make_config(vec![Language::Node]);
    config.adapters = vec![AdapterConfig {
        name: "validate".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::validate".to_string(),
        params: vec![AdapterParam {
            name: "data".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        returns: Some("bool".to_string()),
        error_type: Some("ValidationError".to_string()),
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Node).expect("build failed");

    assert!(bodies.contains_key("validate"), "Expected 'validate' adapter body");
    let body = &bodies["validate"];

    assert!(
        body.contains("napi::Error"),
        "Node body should contain napi::Error conversion. Got: {}",
        body
    );
    assert!(body.contains("my_crate::validate"), "Body should reference core path");
}

/// Test AsyncMethod adapter with Python.
/// Asserts that the generated body references async handling and pyo3_async_runtimes.
#[test]
fn test_async_method_python() {
    let mut config = make_config(vec![Language::Python]);
    config.adapters = vec![AdapterConfig {
        name: "process_async".to_string(),
        pattern: AdapterPattern::AsyncMethod,
        core_path: "process_async".to_string(),
        params: vec![
            AdapterParam {
                name: "request".to_string(),
                ty: "Request".to_string(),
                optional: false,
            },
            AdapterParam {
                name: "timeout".to_string(),
                ty: "u64".to_string(),
                optional: true,
            },
        ],
        returns: Some("Response".to_string()),
        error_type: Some("ProcessError".to_string()),
        owner_type: Some("MyClient".to_string()),
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Python).expect("build failed");

    assert!(
        bodies.contains_key("MyClient.process_async"),
        "Expected 'MyClient.process_async' adapter body (owner.method pattern)"
    );
    let body = &bodies["MyClient.process_async"];

    assert!(
        body.contains("pyo3_async_runtimes"),
        "Python async body should use pyo3_async_runtimes. Got: {}",
        body
    );
    assert!(body.contains(".await"), "Python async body should contain .await");
    assert!(body.contains("self.inner"), "Async method should reference self.inner");
}

/// Test AsyncMethod adapter with Node.
/// Asserts that the generated body contains proper async/await handling.
#[test]
fn test_async_method_node() {
    let mut config = make_config(vec![Language::Node]);
    config.adapters = vec![AdapterConfig {
        name: "fetch".to_string(),
        pattern: AdapterPattern::AsyncMethod,
        core_path: "fetch".to_string(),
        params: vec![AdapterParam {
            name: "url".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        returns: Some("Data".to_string()),
        error_type: Some("FetchError".to_string()),
        owner_type: Some("HttpClient".to_string()),
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Node).expect("build failed");

    let body = &bodies["HttpClient.fetch"];
    assert!(body.contains(".await"), "Node async body should contain .await");
    assert!(
        body.contains("napi::Error"),
        "Node async body should handle napi::Error"
    );
}

/// Test CallbackBridge adapter with Python.
/// Asserts that the generated bodies contain struct code and impl code with proper trait impl.
#[test]
fn test_callback_bridge_python() {
    let mut config = make_config(vec![Language::Python]);
    config.adapters = vec![AdapterConfig {
        name: "event_handler".to_string(),
        pattern: AdapterPattern::CallbackBridge,
        core_path: "my_crate::handler".to_string(),
        params: vec![AdapterParam {
            name: "event".to_string(),
            ty: "Event".to_string(),
            optional: false,
        }],
        returns: Some("Response".to_string()),
        error_type: Some("HandlerError".to_string()),
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: Some("EventHandler".to_string()),
        trait_method: Some("handle_event".to_string()),
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Python).expect("build failed");

    assert!(
        bodies.contains_key("event_handler.__bridge_struct__"),
        "Expected bridge struct key"
    );
    assert!(
        bodies.contains_key("event_handler.__bridge_impl__"),
        "Expected bridge impl key"
    );

    let struct_code = &bodies["event_handler.__bridge_struct__"];
    let impl_code = &bodies["event_handler.__bridge_impl__"];

    assert!(
        struct_code.contains("pyo3"),
        "Python bridge struct should reference pyo3. Got: {}",
        struct_code
    );
    assert!(
        struct_code.contains("Py<PyAny>"),
        "Python bridge should wrap Python callables"
    );

    assert!(
        impl_code.contains("EventHandler"),
        "Impl should implement specified trait"
    );
    assert!(impl_code.contains("handle_event"), "Impl should contain trait method");
}

/// Test CallbackBridge adapter with Node.
/// Asserts that the generated bodies contain NAPI-specific callback handling.
#[test]
fn test_callback_bridge_node() {
    let mut config = make_config(vec![Language::Node]);
    config.adapters = vec![AdapterConfig {
        name: "request_handler".to_string(),
        pattern: AdapterPattern::CallbackBridge,
        core_path: "my_crate::handler".to_string(),
        params: vec![AdapterParam {
            name: "req".to_string(),
            ty: "Request".to_string(),
            optional: false,
        }],
        returns: Some("Response".to_string()),
        error_type: Some("Error".to_string()),
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: Some("RequestHandler".to_string()),
        trait_method: Some("handle".to_string()),
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Node).expect("build failed");

    let struct_code = &bodies["request_handler.__bridge_struct__"];
    let impl_code = &bodies["request_handler.__bridge_impl__"];

    assert!(
        struct_code.contains("ThreadsafeFunction") || struct_code.contains("napi"),
        "Node bridge should use ThreadsafeFunction or NAPI API. Got: {}",
        struct_code
    );
    assert!(
        impl_code.contains("RequestHandler"),
        "Impl should implement RequestHandler trait"
    );
}

/// Test Streaming adapter with Python.
/// Asserts that the generated bodies contain async iterator handling and struct generation.
#[test]
fn test_streaming_python() {
    let mut config = make_config(vec![Language::Python]);
    config.adapters = vec![AdapterConfig {
        name: "stream_data".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "stream_data".to_string(),
        params: vec![AdapterParam {
            name: "limit".to_string(),
            ty: "u32".to_string(),
            optional: false,
        }],
        returns: None,
        error_type: None,
        owner_type: Some("DataClient".to_string()),
        item_type: Some("DataItem".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Python).expect("build failed");

    assert!(
        bodies.contains_key("DataClient.stream_data"),
        "Expected streaming method body"
    );
    assert!(
        bodies.contains_key("DataClient.stream_data.__stream_struct__"),
        "Expected streaming iterator struct under owner-qualified key. Keys: {:?}",
        bodies.keys().collect::<Vec<_>>()
    );

    let method_body = &bodies["DataClient.stream_data"];
    let struct_def = &bodies["DataClient.stream_data.__stream_struct__"];

    assert!(
        method_body.contains("Iterator") || method_body.contains("StreamData"),
        "Method body should reference iterator"
    );

    assert!(
        struct_def.contains("#[pyclass]"),
        "Streaming struct should be a pyclass"
    );
    assert!(
        struct_def.contains("__anext__"),
        "Streaming struct should implement __anext__ for async iteration"
    );
}

/// Test Streaming adapter with Node.
/// Asserts that the generated body collects stream into Vec.
#[test]
fn test_streaming_node() {
    let mut config = make_config(vec![Language::Node]);
    config.adapters = vec![AdapterConfig {
        name: "list_items".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "list_items".to_string(),
        params: vec![],
        returns: None,
        error_type: None,
        owner_type: Some("Client".to_string()),
        item_type: Some("Item".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Node).expect("build failed");

    let body = &bodies["Client.list_items"];

    assert!(
        body.contains("tokio::sync::mpsc"),
        "Node streaming should use mpsc channel. Got: {}",
        body
    );
    assert!(
        body.contains("tokio::spawn"),
        "Node streaming should spawn background task"
    );
    assert!(
        body.contains("ListItemsIterator"),
        "Node streaming should return iterator struct"
    );
}

/// Test FFI (C ABI) language with SyncFunction.
/// FFI should generate C-compatible code with error handling.
#[test]
fn test_sync_function_ffi() {
    let mut config = make_config(vec![Language::Ffi]);
    config.adapters = vec![AdapterConfig {
        name: "compute".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::compute".to_string(),
        params: vec![AdapterParam {
            name: "value".to_string(),
            ty: "i32".to_string(),
            optional: false,
        }],
        returns: Some("i32".to_string()),
        error_type: None,
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Ffi).expect("build failed");

    let body = &bodies["compute"];

    assert!(
        body.contains("match") && body.contains("Ok(result)") && body.contains("Err(e)"),
        "FFI body should match on Result. Got: {}",
        body
    );
    assert!(body.contains("CString"), "FFI should use CString for returning strings");
    assert!(
        body.contains("update_last_error"),
        "FFI should call update_last_error on error"
    );
}

/// Test Go language with SyncFunction.
/// Go should generate code that calls C FFI and deserializes JSON.
#[test]
fn test_sync_function_go() {
    let mut config = make_config(vec![Language::Go]);
    config.adapters = vec![AdapterConfig {
        name: "transform".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::transform".to_string(),
        params: vec![AdapterParam {
            name: "input".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        returns: Some("String".to_string()),
        error_type: None,
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Go).expect("build failed");

    let body = &bodies["transform"];

    assert!(
        body.contains("C.CString") || body.contains("C."),
        "Go body should call C functions. Got: {}",
        body
    );
    assert!(
        body.contains("json.Unmarshal"),
        "Go body should deserialize JSON result"
    );
    assert!(body.contains("defer C.free"), "Go body should free C-allocated memory");
}

/// Test Java (Panama FFI) language with SyncFunction.
/// Java should generate MemorySegment-based FFI code.
#[test]
fn test_sync_function_java() {
    let mut config = make_config(vec![Language::Java]);
    config.adapters = vec![AdapterConfig {
        name: "process".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::process".to_string(),
        params: vec![AdapterParam {
            name: "data".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        returns: Some("String".to_string()),
        error_type: None,
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Java).expect("build failed");

    let body = &bodies["process"];

    assert!(
        body.contains("Arena"),
        "Java body should use Arena for memory management. Got: {}",
        body
    );
    assert!(body.contains("MemorySegment"), "Java body should use MemorySegment");
    assert!(
        body.contains("invokeExact"),
        "Java body should invoke function via invokeExact"
    );
}

/// Test C# (P/Invoke) language with SyncFunction.
/// C# should generate P/Invoke code with IntPtr handling.
#[test]
fn test_sync_function_csharp() {
    let mut config = make_config(vec![Language::Csharp]);
    config.adapters = vec![AdapterConfig {
        name: "execute".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::execute".to_string(),
        params: vec![AdapterParam {
            name: "cmd".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        returns: Some("String".to_string()),
        error_type: None,
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Csharp).expect("build failed");

    let body = &bodies["execute"];

    assert!(
        body.contains("IntPtr"),
        "C# body should use IntPtr for FFI. Got: {}",
        body
    );
    assert!(
        body.contains("Marshal.PtrToStringUTF8"),
        "C# body should marshal string from pointer"
    );
    assert!(body.contains("FreeString"), "C# body should free allocated memory");
}

/// Test Ruby language with SyncFunction.
/// Ruby should generate Magnus-compatible code.
#[test]
fn test_sync_function_ruby() {
    let mut config = make_config(vec![Language::Ruby]);
    config.adapters = vec![AdapterConfig {
        name: "parse".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::parse".to_string(),
        params: vec![AdapterParam {
            name: "text".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        returns: Some("String".to_string()),
        error_type: Some("ParseError".to_string()),
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Ruby).expect("build failed");

    let body = &bodies["parse"];

    assert!(
        body.contains("magnus::Error"),
        "Ruby body should use magnus::Error. Got: {}",
        body
    );
    assert!(
        body.contains("exception_runtime_error()"),
        "Ruby body should raise runtime error"
    );
}

/// Test PHP language with SyncFunction.
/// PHP should generate ext-php-rs compatible code.
#[test]
fn test_sync_function_php() {
    let mut config = make_config(vec![Language::Php]);
    config.adapters = vec![AdapterConfig {
        name: "encode".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::encode".to_string(),
        params: vec![AdapterParam {
            name: "input".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        returns: Some("String".to_string()),
        error_type: Some("EncodeError".to_string()),
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Php).expect("build failed");

    let body = &bodies["encode"];

    assert!(
        body.contains("PhpException"),
        "PHP body should use PhpException. Got: {}",
        body
    );
}

/// Test Elixir language with AsyncMethod.
/// Elixir should use tokio runtime blocking in NIFs.
#[test]
fn test_async_method_elixir() {
    let mut config = make_config(vec![Language::Elixir]);
    config.adapters = vec![AdapterConfig {
        name: "call_async".to_string(),
        pattern: AdapterPattern::AsyncMethod,
        core_path: "call_async".to_string(),
        params: vec![AdapterParam {
            name: "arg".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        returns: Some("String".to_string()),
        error_type: Some("Error".to_string()),
        owner_type: Some("Client".to_string()),
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Elixir).expect("build failed");

    let body = &bodies["Client.call_async"];

    assert!(
        body.contains("block_on"),
        "Elixir async body should use block_on. Got: {}",
        body
    );
    assert!(
        body.contains("tokio::runtime::Runtime"),
        "Elixir should create tokio runtime"
    );
}

/// Test WASM language with SyncFunction.
/// WASM should use wasm-bindgen error handling with JsValue.
#[test]
fn test_sync_function_wasm() {
    let mut config = make_config(vec![Language::Wasm]);
    config.adapters = vec![AdapterConfig {
        name: "calc".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::calc".to_string(),
        params: vec![AdapterParam {
            name: "n".to_string(),
            ty: "i32".to_string(),
            optional: false,
        }],
        returns: Some("i32".to_string()),
        error_type: Some("Error".to_string()),
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Wasm).expect("build failed");

    let body = &bodies["calc"];

    assert!(body.contains("JsValue"), "WASM body should use JsValue. Got: {}", body);
    assert!(
        body.contains("JsValue::from_str"),
        "WASM body should convert errors to JsValue strings"
    );
}

/// Test R language with SyncFunction.
/// R should use extendr error handling.
#[test]
fn test_sync_function_r() {
    let mut config = make_config(vec![Language::R]);
    config.adapters = vec![AdapterConfig {
        name: "sum_vals".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::sum_vals".to_string(),
        params: vec![AdapterParam {
            name: "vals".to_string(),
            ty: "Vec<i32>".to_string(),
            optional: false,
        }],
        returns: Some("i32".to_string()),
        error_type: Some("Error".to_string()),
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::R).expect("build failed");

    let body = &bodies["sum_vals"];

    assert!(
        body.contains("extendr_api::Error"),
        "R body should use extendr_api::Error. Got: {}",
        body
    );
}

/// Test SyncFunction with optional parameters.
/// Arguments should be converted with .map(Into::into) for optional params.
#[test]
fn test_sync_function_optional_params() {
    let mut config = make_config(vec![Language::Python]);
    config.adapters = vec![AdapterConfig {
        name: "format_text".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::format_text".to_string(),
        params: vec![
            AdapterParam {
                name: "text".to_string(),
                ty: "String".to_string(),
                optional: false,
            },
            AdapterParam {
                name: "options".to_string(),
                ty: "FormatOptions".to_string(),
                optional: true,
            },
        ],
        returns: Some("String".to_string()),
        error_type: None,
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Python).expect("build failed");

    let body = &bodies["format_text"];

    assert!(
        body.contains("text.into()"),
        "Required param should use .into(). Got: {}",
        body
    );
    assert!(
        body.contains("options.map(Into::into)"),
        "Optional param should use .map(Into::into). Got: {}",
        body
    );
}

/// Test multiple adapters in same config.
/// Each adapter should get its own entry in the map.
#[test]
fn test_multiple_adapters() {
    let mut config = make_config(vec![Language::Node]);
    config.adapters = vec![
        AdapterConfig {
            name: "create".to_string(),
            pattern: AdapterPattern::SyncFunction,
            core_path: "my_crate::create".to_string(),
            params: vec![],
            returns: Some("Object".to_string()),
            error_type: None,
            owner_type: None,
            item_type: None,
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,

            skip_languages: vec![],
        },
        AdapterConfig {
            name: "destroy".to_string(),
            pattern: AdapterPattern::SyncFunction,
            core_path: "my_crate::destroy".to_string(),
            params: vec![AdapterParam {
                name: "obj".to_string(),
                ty: "Object".to_string(),
                optional: false,
            }],
            returns: None,
            error_type: None,
            owner_type: None,
            item_type: None,
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,

            skip_languages: vec![],
        },
    ];

    let bodies = build_adapter_bodies(&config, Language::Node).expect("build failed");

    assert_eq!(bodies.len(), 2, "Should have 2 adapter bodies");
    assert!(bodies.contains_key("create"), "Should have 'create' adapter");
    assert!(bodies.contains_key("destroy"), "Should have 'destroy' adapter");
}

/// Test async method with optional parameters.
#[test]
fn test_async_method_optional_params() {
    let mut config = make_config(vec![Language::Node]);
    config.adapters = vec![AdapterConfig {
        name: "wait".to_string(),
        pattern: AdapterPattern::AsyncMethod,
        core_path: "wait".to_string(),
        params: vec![
            AdapterParam {
                name: "duration".to_string(),
                ty: "u64".to_string(),
                optional: false,
            },
            AdapterParam {
                name: "callback".to_string(),
                ty: "String".to_string(),
                optional: true,
            },
        ],
        returns: Some("()".to_string()),
        error_type: Some("Error".to_string()),
        owner_type: Some("Timer".to_string()),
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Node).expect("build failed");

    let body = &bodies["Timer.wait"];
    assert!(
        body.contains("duration") && body.contains("callback"),
        "Body should handle both required and optional params"
    );
}

/// Test GIL release flag for Python.
/// When gil_release is true, code should be wrapped in py.allow_threads().
#[test]
fn test_sync_function_python_gil_release() {
    let mut config = make_config(vec![Language::Python]);
    config.adapters = vec![AdapterConfig {
        name: "heavy_compute".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::heavy_compute".to_string(),
        params: vec![AdapterParam {
            name: "data".to_string(),
            ty: "Vec<u8>".to_string(),
            optional: false,
        }],
        returns: Some("u64".to_string()),
        error_type: None,
        owner_type: None,
        item_type: None,
        gil_release: true,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Python).expect("build failed");

    let body = &bodies["heavy_compute"];

    assert!(
        body.contains("py.allow_threads"),
        "Python body with gil_release=true should call py.allow_threads. Got: {}",
        body
    );
}

/// Test that bodies map is built successfully with empty adapter list.
#[test]
fn test_empty_adapters() {
    let config = make_config(vec![Language::Python, Language::Node]);

    let bodies = build_adapter_bodies(&config, Language::Python).expect("build failed");

    assert!(bodies.is_empty(), "Should have no adapter bodies for empty config");
}

/// Test Python with string parameter type handling.
#[test]
fn test_python_string_params() {
    let mut config = make_config(vec![Language::Python]);
    config.adapters = vec![AdapterConfig {
        name: "concat".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::concat".to_string(),
        params: vec![
            AdapterParam {
                name: "a".to_string(),
                ty: "String".to_string(),
                optional: false,
            },
            AdapterParam {
                name: "b".to_string(),
                ty: "String".to_string(),
                optional: false,
            },
        ],
        returns: Some("String".to_string()),
        error_type: None,
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Python).expect("build failed");

    let body = &bodies["concat"];
    assert!(
        body.contains("a.into()") || body.contains("a"),
        "Should reference 'a' param"
    );
    assert!(
        body.contains("b.into()") || body.contains("b"),
        "Should reference 'b' param"
    );
}

/// Test FFI with string parameters requiring CStr conversion.
#[test]
fn test_ffi_string_conversion() {
    let mut config = make_config(vec![Language::Ffi]);
    config.adapters = vec![AdapterConfig {
        name: "echo".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::echo".to_string(),
        params: vec![AdapterParam {
            name: "msg".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        returns: Some("String".to_string()),
        error_type: None,
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Ffi).expect("build failed");

    let body = &bodies["echo"];

    assert!(
        body.contains("CStr::from_ptr") || body.contains("to_str()"),
        "FFI should convert string parameters from C pointers. Got: {}",
        body
    );
}

/// Test Go with numeric parameter conversion.
#[test]
fn test_go_numeric_params() {
    let mut config = make_config(vec![Language::Go]);
    config.adapters = vec![AdapterConfig {
        name: "multiply".to_string(),
        pattern: AdapterPattern::SyncFunction,
        core_path: "my_crate::multiply".to_string(),
        params: vec![
            AdapterParam {
                name: "x".to_string(),
                ty: "i32".to_string(),
                optional: false,
            },
            AdapterParam {
                name: "y".to_string(),
                ty: "i32".to_string(),
                optional: false,
            },
        ],
        returns: Some("i32".to_string()),
        error_type: None,
        owner_type: None,
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,

        skip_languages: vec![],
    }];

    let bodies = build_adapter_bodies(&config, Language::Go).expect("build failed");

    let body = &bodies["multiply"];

    assert!(
        body.contains("C.") || body.contains("json"),
        "Go should call C functions or use JSON serialization. Got: {}",
        body
    );
}

fn two_streaming_adapters_on_one_owner() -> Vec<AdapterConfig> {
    vec![
        AdapterConfig {
            name: "crawl_stream".to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: "crawl_stream".to_string(),
            params: vec![AdapterParam {
                name: "req".to_string(),
                ty: "CrawlStreamRequest".to_string(),
                optional: false,
            }],
            returns: None,
            error_type: Some("CrawlError".to_string()),
            owner_type: Some("CrawlEngineHandle".to_string()),
            item_type: Some("CrawlEvent".to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: Some("sample_crawler::CrawlStreamRequest".to_string()),

            skip_languages: vec![],
        },
        AdapterConfig {
            name: "batch_crawl_stream".to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: "batch_crawl_stream".to_string(),
            params: vec![AdapterParam {
                name: "req".to_string(),
                ty: "BatchCrawlStreamRequest".to_string(),
                optional: false,
            }],
            returns: None,
            error_type: Some("CrawlError".to_string()),
            owner_type: Some("CrawlEngineHandle".to_string()),
            item_type: Some("CrawlEvent".to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: Some("sample_crawler::BatchCrawlStreamRequest".to_string()),

            skip_languages: vec![],
        },
    ]
}

#[test]
fn test_two_streaming_adapters_share_owner_python_emits_distinct_iterators() {
    let mut config = make_config(vec![Language::Python]);
    config.adapters = two_streaming_adapters_on_one_owner();

    let bodies = build_adapter_bodies(&config, Language::Python).expect("build failed");

    let key_crawl = "CrawlEngineHandle.crawl_stream.__stream_struct__";
    let key_batch = "CrawlEngineHandle.batch_crawl_stream.__stream_struct__";
    assert!(
        bodies.contains_key(key_crawl),
        "missing per-adapter iterator key '{key_crawl}'. Keys: {:?}",
        bodies.keys().collect::<Vec<_>>()
    );
    assert!(
        bodies.contains_key(key_batch),
        "missing per-adapter iterator key '{key_batch}'. Keys: {:?}",
        bodies.keys().collect::<Vec<_>>()
    );

    let struct_crawl = &bodies[key_crawl];
    let struct_batch = &bodies[key_batch];

    assert!(
        struct_crawl.contains("CrawlStreamIterator"),
        "crawl_stream iterator struct should be named CrawlStreamIterator. Got: {struct_crawl}"
    );
    assert!(
        struct_batch.contains("BatchCrawlStreamIterator"),
        "batch_crawl_stream iterator struct should be named BatchCrawlStreamIterator. Got: {struct_batch}"
    );
    assert!(
        !struct_crawl.contains("BatchCrawlStreamIterator"),
        "crawl_stream struct must not collide with BatchCrawlStreamIterator. Got: {struct_crawl}"
    );
}

#[test]
fn test_two_streaming_adapters_share_owner_elixir_emits_distinct_handles() {
    let mut config = make_config(vec![Language::Elixir]);
    config.adapters = two_streaming_adapters_on_one_owner();

    let bodies = build_adapter_bodies(&config, Language::Elixir).expect("build failed");

    let key_crawl = "CrawlEngineHandle.crawl_stream.__stream_struct__";
    let key_batch = "CrawlEngineHandle.batch_crawl_stream.__stream_struct__";
    let struct_crawl = bodies
        .get(key_crawl)
        .unwrap_or_else(|| panic!("missing '{key_crawl}'. Keys: {:?}", bodies.keys().collect::<Vec<_>>()));
    let struct_batch = bodies
        .get(key_batch)
        .unwrap_or_else(|| panic!("missing '{key_batch}'. Keys: {:?}", bodies.keys().collect::<Vec<_>>()));

    assert!(
        struct_crawl.contains("crawlenginehandle_crawl_stream_start"),
        "crawl_stream body must define start NIF. Got: {struct_crawl}"
    );
    assert!(
        struct_batch.contains("crawlenginehandle_batch_crawl_stream_start"),
        "batch_crawl_stream body must define batch start NIF. Got: {struct_batch}"
    );

    assert!(
        struct_crawl.contains("CrawlStreamRequest") && !struct_crawl.contains("BatchCrawlStreamRequest"),
        "crawl_stream must use its own request type, not the second adapter's. Got: {struct_crawl}"
    );
    assert!(
        struct_batch.contains("BatchCrawlStreamRequest"),
        "batch_crawl_stream must use its own request type. Got: {struct_batch}"
    );
}

/// Regression: the Elixir streaming start NIF must read-lock and clone the inner
/// handle out of `Arc<RwLock<_>>` before calling the core stream method, mirroring
/// the non-streaming opaque path. Cloning the `Arc<RwLock<_>>` directly compiled to
/// `E0599: no method named <stream_fn> on Arc<RwLock<Handle>>` (surfaced by a downstream
/// streaming e2e).
#[test]
fn should_lock_and_clone_handle_in_elixir_streaming_start_nif() {
    let mut config = make_config(vec![Language::Elixir]);
    config.adapters = two_streaming_adapters_on_one_owner();

    let bodies = build_adapter_bodies(&config, Language::Elixir).expect("build failed");
    let struct_crawl = &bodies["CrawlEngineHandle.crawl_stream.__stream_struct__"];

    assert!(
        struct_crawl.contains("resource.inner.read().unwrap_or_else(|e| e.into_inner()).clone()"),
        "streaming start NIF must read-lock+clone the handle before the core call. Got: {struct_crawl}"
    );
    assert!(
        !struct_crawl.contains("let inner = resource.inner.clone();"),
        "streaming start NIF must not clone the Arc<RwLock<_>> directly (E0599). Got: {struct_crawl}"
    );
}

/// When an adapter has `skip_languages = ["wasm"]` the WASM backend must not
/// receive any body entries for that adapter.
#[test]
fn test_skip_languages_wasm_suppresses_wasm_body() {
    let mut config = make_config(vec![Language::Wasm]);
    config.adapters = vec![AdapterConfig {
        name: "crawl_stream".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "crawl_stream".to_string(),
        params: vec![AdapterParam {
            name: "req".to_string(),
            ty: "CrawlStreamRequest".to_string(),
            optional: false,
        }],
        returns: None,
        error_type: Some("CrawlError".to_string()),
        owner_type: Some("CrawlEngineHandle".to_string()),
        item_type: Some("CrawlEvent".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: Some("sample_crawler::CrawlStreamRequest".to_string()),
        skip_languages: vec!["wasm".to_string()],
    }];

    let bodies = build_adapter_bodies(&config, Language::Wasm).expect("build_adapter_bodies failed");

    assert!(
        !bodies.contains_key("CrawlEngineHandle.crawl_stream"),
        "WASM body must be suppressed when skip_languages contains 'wasm'. Keys: {:?}",
        bodies.keys().collect::<Vec<_>>()
    );
    assert!(
        !bodies.contains_key("CrawlEngineHandle.crawl_stream.__stream_struct__"),
        "WASM struct def must also be suppressed. Keys: {:?}",
        bodies.keys().collect::<Vec<_>>()
    );
}

/// An adapter with skip_languages not containing a given language must still
/// emit bodies for that language as normal.
#[test]
fn test_skip_languages_does_not_suppress_other_languages() {
    let mut config = make_config(vec![Language::Python]);
    config.adapters = vec![AdapterConfig {
        name: "crawl_stream".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "crawl_stream".to_string(),
        params: vec![AdapterParam {
            name: "req".to_string(),
            ty: "CrawlStreamRequest".to_string(),
            optional: false,
        }],
        returns: None,
        error_type: Some("CrawlError".to_string()),
        owner_type: Some("CrawlEngineHandle".to_string()),
        item_type: Some("CrawlEvent".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,
        skip_languages: vec!["wasm".to_string()],
    }];

    let bodies = build_adapter_bodies(&config, Language::Python).expect("build_adapter_bodies failed");

    assert!(
        bodies.contains_key("CrawlEngineHandle.crawl_stream"),
        "Python body must still be emitted when skip_languages only contains 'wasm'. Keys: {:?}",
        bodies.keys().collect::<Vec<_>>()
    );
}
