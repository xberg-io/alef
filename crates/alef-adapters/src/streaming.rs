use alef_core::config::{AdapterConfig, AlefConfig, Language};

/// Generate the method body and optionally a struct definition for a streaming adapter.
///
/// Returns `(method_body, Option<struct_definition>)`.
/// The struct definition is only produced for languages that need a separate iterator struct
/// (currently Python/PyO3).
pub fn generate_body(
    adapter: &AdapterConfig,
    language: Language,
    config: &AlefConfig,
) -> anyhow::Result<(String, Option<String>)> {
    let result = match language {
        Language::Python => gen_python_body(adapter, config),
        Language::Node => gen_node_body(adapter, config),
        Language::Ruby => gen_ruby_body(adapter, config),
        Language::Php => gen_php_body(adapter, config),
        Language::Elixir => gen_elixir_body(adapter, config),
        Language::Wasm => gen_wasm_body(adapter, config),
        Language::Ffi => gen_ffi_body(adapter),
        Language::Go => gen_go_body(adapter),
        Language::Java => gen_java_body(adapter),
        Language::Csharp => gen_csharp_body(adapter),
        Language::R => gen_r_body(adapter, config),
        Language::Rust => anyhow::bail!("Rust does not need generated binding adapters"),
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
            anyhow::bail!("Phase 1: {language} backend not yet implemented")
        }
    };
    Ok(result)
}

/// Build the call arguments referencing `_core` locals created by the method codegen.
///
/// The regular method codegen already emits `let {name}_core: CoreType = {name}.into();`
/// for each parameter, so the adapter body must use those converted locals — not call
/// `.into()` a second time (which would trigger a use-after-move error).
fn call_args(adapter: &AdapterConfig) -> Vec<String> {
    adapter.params.iter().map(|p| format!("core_{}", p.name)).collect()
}

/// Build conversion let-bindings for core types.
fn core_let_bindings(adapter: &AdapterConfig, core_import: &str) -> Vec<String> {
    adapter
        .params
        .iter()
        .map(|p| {
            if p.optional {
                format!(
                    "let core_{name} = {name}.map(|v| -> {core_import}::{ty} {{ v.into() }});",
                    name = p.name,
                    core_import = core_import,
                    ty = p.ty,
                )
            } else {
                format!(
                    "let core_{name}: {core_import}::{ty} = {name}.into();",
                    name = p.name,
                    core_import = core_import,
                    ty = p.ty,
                )
            }
        })
        .collect()
}

/// Build conversion let-bindings for core types, cloning the input first.
/// Used by PHP which passes struct params by reference.
fn core_let_bindings_cloned(adapter: &AdapterConfig, core_import: &str) -> Vec<String> {
    adapter
        .params
        .iter()
        .map(|p| {
            if p.optional {
                format!(
                    "let core_{name} = {name}.as_ref().map(|v| -> {core_import}::{ty} {{ v.clone().into() }});",
                    name = p.name,
                    core_import = core_import,
                    ty = p.ty,
                )
            } else {
                format!(
                    "let core_{name}: {core_import}::{ty} = {name}.clone().into();",
                    name = p.name,
                    core_import = core_import,
                    ty = p.ty,
                )
            }
        })
        .collect()
}

/// Get the iterator struct name from the adapter name.
fn iterator_name(adapter: &AdapterConfig) -> String {
    to_pascal_case(&adapter.name) + "Iterator"
}

// ---------------------------------------------------------------------------
// Python (PyO3)
// ---------------------------------------------------------------------------

fn gen_python_body(adapter: &AdapterConfig, config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");
    let error_type = adapter.error_type.as_deref().unwrap_or("anyhow::Error");
    let core_import = config.core_import();
    let iter_name = iterator_name(adapter);

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let struct_def = format!(
        "#[pyclass]\n\
         pub struct {iter_name} {{\n    \
             inner: Arc<tokio::sync::Mutex<futures::stream::BoxStream<'static, Result<{core_import}::{item_type}, {core_import}::{error_type}>>>>,\n\
         }}\n\
         \n\
         #[pymethods]\n\
         impl {iter_name} {{\n    \
             fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {{ slf }}\n\
             \n    \
             fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {{\n        \
                 let inner = self.inner.clone();\n        \
                 pyo3_async_runtimes::tokio::future_into_py(py, async move {{\n            \
                     let mut stream = inner.lock().await;\n            \
                     match futures::StreamExt::next(&mut *stream).await {{\n                \
                         Some(Ok(chunk)) => Ok(Some({item_type}::from(chunk))),\n                \
                         Some(Err(e)) => Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string())),\n                \
                         None => Ok(None),  // StopAsyncIteration\n            \
                     }}\n        \
                 }}).map(Some)\n    \
             }}\n\
         }}"
    );

    let let_bindings = core_let_bindings(adapter, &core_import);
    let bindings_block = if let_bindings.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", let_bindings.join("\n    "))
    };

    let method_body = format!(
        "let inner = self.inner.clone();\n    \
         {bindings_block}\
         let stream = pyo3_async_runtimes::tokio::get_runtime()\n        \
             .block_on(inner.{core_path}({call_str}))\n        \
             .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;\n    \
         let iter = {iter_name} {{\n        \
             inner: Arc::new(tokio::sync::Mutex::new(stream)),\n    \
         }};\n    \
         Ok(Bound::new(py, iter)?.into_any())"
    );

    (method_body, Some(struct_def))
}

// ---------------------------------------------------------------------------
// Node (NAPI)
// ---------------------------------------------------------------------------

fn gen_node_body(adapter: &AdapterConfig, config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let prefix = config.node_type_prefix();
    let raw_item = adapter.item_type.as_deref().unwrap_or("()");
    let item_type = if raw_item == "()" {
        raw_item.to_string()
    } else {
        format!("{prefix}{raw_item}")
    };
    let core_import = config.core_import();

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let let_bindings = core_let_bindings(adapter, &core_import);
    let bindings_block = if let_bindings.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", let_bindings.join("\n    "))
    };

    let body = format!(
        "use futures_util::StreamExt;\n    \
         {bindings_block}\
         let stream = self.inner.{core_path}({call_str}).await\n        \
             .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;\n    \
         let chunks: Vec<{item_type}> = stream\n        \
             .collect::<Vec<_>>().await\n        \
             .into_iter()\n        \
             .collect::<std::result::Result<Vec<_>, _>>()\n        \
             .map(|v| v.into_iter().map({item_type}::from).collect())\n        \
             .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;\n    \
         serde_json::to_string(&chunks)\n        \
             .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// Ruby (Magnus)
// ---------------------------------------------------------------------------

fn gen_ruby_body(adapter: &AdapterConfig, config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");
    let core_import = config.core_import();

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let let_bindings = core_let_bindings(adapter, &core_import);
    let bindings_block = if let_bindings.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", let_bindings.join("\n    "))
    };

    let body = format!(
        "use futures_util::StreamExt;\n    \
         let rt = tokio::runtime::Runtime::new()\n        \
             .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n    \
         {bindings_block}\
         rt.block_on(async {{\n        \
             let stream = self.inner.{core_path}({call_str}).await\n            \
                 .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
             let chunks: Vec<{item_type}> = stream\n            \
                 .collect::<Vec<_>>().await\n            \
                 .into_iter()\n            \
                 .collect::<std::result::Result<Vec<_>, _>>()\n            \
                 .map(|v| v.into_iter().map({item_type}::from).collect())\n            \
                 .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
             serde_json::to_string(&chunks)\n            \
                 .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))\n    \
         }})"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// PHP (ext-php-rs)
// ---------------------------------------------------------------------------

fn gen_php_body(adapter: &AdapterConfig, config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");
    let core_import = config.core_import();

    let args = call_args(adapter);
    let call_str = args.join(", ");

    // PHP passes struct params by reference — clone before converting.
    let let_bindings = core_let_bindings_cloned(adapter, &core_import);
    let bindings_block = if let_bindings.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", let_bindings.join("\n    "))
    };

    let body = format!(
        "use futures_util::StreamExt;\n    \
         {bindings_block}\
         WORKER_RUNTIME.block_on(async {{\n        \
             let stream = self.inner.{core_path}({call_str}).await\n            \
                 .map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n        \
             let chunks: Vec<{item_type}> = stream\n            \
                 .collect::<Vec<_>>().await\n            \
                 .into_iter()\n            \
                 .collect::<std::result::Result<Vec<_>, _>>()\n            \
                 .map(|v| v.into_iter().map({item_type}::from).collect())\n            \
                 .map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n        \
             serde_json::to_string(&chunks)\n            \
                 .map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))\n    \
         }})"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// Elixir (Rustler)
// ---------------------------------------------------------------------------

fn gen_elixir_body(adapter: &AdapterConfig, config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");
    let core_import = config.core_import();

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let let_bindings = core_let_bindings(adapter, &core_import);
    let bindings_block = if let_bindings.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", let_bindings.join("\n    "))
    };

    let body = format!(
        "use futures_util::StreamExt;\n    \
         let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;\n    \
         {bindings_block}\
         rt.block_on(async {{\n        \
             let stream = resource.inner.{core_path}({call_str}).await\n            \
                 .map_err(|e| e.to_string())?;\n        \
             let chunks: Vec<{item_type}> = stream\n            \
                 .collect::<Vec<_>>().await\n            \
                 .into_iter()\n            \
                 .collect::<std::result::Result<Vec<_>, _>>()\n            \
                 .map(|v| v.into_iter().map({item_type}::from).collect())\n            \
                 .map_err(|e| e.to_string())?;\n        \
             serde_json::to_string(&chunks)\n            \
                 .map_err(|e| e.to_string())\n    \
         }})"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// WASM (wasm-bindgen)
// ---------------------------------------------------------------------------

fn gen_wasm_body(adapter: &AdapterConfig, config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let prefix = config.wasm_type_prefix();
    let raw_item = adapter.item_type.as_deref().unwrap_or("JsValue");
    let _item_type = if raw_item == "()" || raw_item == "JsValue" {
        raw_item.to_string()
    } else {
        format!("{prefix}{raw_item}")
    };
    let core_import = config.core_import();

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let let_bindings = core_let_bindings(adapter, &core_import);
    let bindings_block = if let_bindings.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", let_bindings.join("\n    "))
    };

    // WASM: use serde_wasm_bindgen to convert core types (which have Serialize) to JsValue.
    // Return JsValue instead of String for idiomatic WASM interop.
    let core_item = adapter.item_type.as_deref().unwrap_or("()");
    let body = format!(
        "use futures_util::StreamExt;\n    \
         {bindings_block}\
         let stream = self.inner.{core_path}({call_str}).await\n        \
             .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
         let chunks: Vec<{core_import}::{core_item}> = stream\n        \
             .collect::<Vec<_>>().await\n        \
             .into_iter()\n        \
             .collect::<std::result::Result<Vec<_>, _>>()\n        \
             .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
         serde_wasm_bindgen::to_value(&chunks)\n        \
             .map_err(|e| JsValue::from_str(&e.to_string()))"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// FFI (C ABI) -- Callback-based streaming
// ---------------------------------------------------------------------------

fn gen_ffi_body(adapter: &AdapterConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;

    let body = format!(
        "clear_last_error();\n\n    \
         if client.is_null() {{\n        \
             set_last_error(99, \"literllm_{name}: client must not be NULL\");\n        \
             return -1;\n    \
         }}\n    \
         if request_json.is_null() {{\n        \
             set_last_error(99, \"literllm_{name}: request_json must not be NULL\");\n        \
             return -1;\n    \
         }}\n\n    \
         // SAFETY: caller guarantees `client` and `request_json` are non-null and valid.\n    \
         let client_ref = unsafe {{ &(*client) }};\n\n    \
         let json_str = match unsafe {{ std::ffi::CStr::from_ptr(request_json) }}.to_str() {{\n        \
             Ok(s) => s,\n        \
             Err(e) => {{\n            \
                 set_last_error(99, &format!(\"literllm_{name}: request_json is not valid UTF-8: {{e}}\"));\n            \
                 return -1;\n        \
             }}\n    \
         }};\n\n    \
         let request: liter_llm::ChatCompletionRequest = match serde_json::from_str(json_str) {{\n        \
             Ok(r) => r,\n        \
             Err(e) => {{\n            \
                 set_last_error(99, &format!(\"literllm_{name}: failed to parse request JSON: {{e}}\"));\n            \
                 return -1;\n        \
             }}\n    \
         }};\n\n    \
         let rt = get_ffi_runtime();\n\n    \
         let result = rt.block_on(async {{\n        \
             use futures_util::StreamExt;\n\n        \
             let mut stream = match client_ref.{core_path}(request).await {{\n            \
                 Ok(s) => s,\n            \
                 Err(e) => return Err(format!(\"literllm_{name}: failed to open stream: {{e}}\")),\n        \
             }};\n\n        \
             loop {{\n            \
                 match stream.next().await {{\n                \
                     None => break,\n                \
                     Some(Err(e)) => return Err(format!(\"literllm_{name}: stream error: {{e}}\")),\n                \
                     Some(Ok(chunk)) => {{\n                    \
                         let chunk_json = match serde_json::to_string(&chunk) {{\n                        \
                             Ok(s) => s,\n                        \
                             Err(e) => return Err(format!(\"literllm_{name}: failed to serialise chunk: {{e}}\")),\n                    \
                         }};\n                    \
                         match std::ffi::CString::new(chunk_json) {{\n                        \
                             Ok(c_str) => {{\n                            \
                                 // SAFETY: `callback` is a valid function pointer supplied by the caller.\n                            \
                                 // `c_str.as_ptr()` is valid for this block scope.\n                            \
                                 // `user_data` is forwarded as-is; ownership stays with the caller.\n                            \
                                 unsafe {{ callback(c_str.as_ptr(), user_data) }};\n                        \
                             }}\n                        \
                             Err(e) => return Err(format!(\"literllm_{name}: chunk JSON contained NUL byte: {{e}}\")),\n                    \
                         }}\n                \
                     }}\n            \
                 }}\n        \
             }}\n        \
             Ok(())\n    \
         }});\n\n    \
         match result {{\n        \
             Ok(()) => 0,\n        \
             Err(e) => {{\n            \
                 set_last_error(99, &e);\n            \
                 -1\n        \
             }}\n    \
         }}",
        name = adapter.name,
        core_path = core_path,
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// Go -- Streaming not supported via FFI
// ---------------------------------------------------------------------------

fn gen_go_body(adapter: &AdapterConfig) -> (String, Option<String>) {
    let body = format!("compile_error!(\"streaming not supported via FFI: {}\")", adapter.name);
    (body, None)
}

// ---------------------------------------------------------------------------
// Java -- Streaming not supported via FFI
// ---------------------------------------------------------------------------

fn gen_java_body(adapter: &AdapterConfig) -> (String, Option<String>) {
    let body = format!("compile_error!(\"streaming not supported via FFI: {}\")", adapter.name);
    (body, None)
}

// ---------------------------------------------------------------------------
// C# -- Streaming not supported via FFI
// ---------------------------------------------------------------------------

fn gen_csharp_body(adapter: &AdapterConfig) -> (String, Option<String>) {
    let body = format!("compile_error!(\"streaming not supported via FFI: {}\")", adapter.name);
    (body, None)
}

// ---------------------------------------------------------------------------
// R (extendr) -- collect stream into Vec
// ---------------------------------------------------------------------------

fn gen_r_body(adapter: &AdapterConfig, config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("Robj");
    let core_import = config.core_import();

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let let_bindings = core_let_bindings(adapter, &core_import);
    let bindings_block = if let_bindings.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", let_bindings.join("\n    "))
    };

    let body = format!(
        "use futures_util::StreamExt;\n    \
         let rt = tokio::runtime::Runtime::new()\n        \
             .map_err(|e| extendr_api::Error::Other(e.to_string()))?;\n    \
         {bindings_block}\
         rt.block_on(async {{\n        \
             let stream = self.inner.{core_path}({call_str}).await\n            \
                 .map_err(|e| extendr_api::Error::Other(e.to_string()))?;\n        \
             let chunks: Vec<{item_type}> = stream\n            \
                 .collect::<Vec<_>>().await\n            \
                 .into_iter()\n            \
                 .collect::<std::result::Result<Vec<_>, _>>()\n            \
                 .map(|v| v.into_iter().map({item_type}::from).collect())\n            \
                 .map_err(|e| extendr_api::Error::Other(e.to_string()))?;\n        \
             serde_json::to_string(&chunks)\n            \
                 .map_err(|e| extendr_api::Error::Other(e.to_string()))\n    \
         }})"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect()
}
