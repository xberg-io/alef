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
    };
    Ok(result)
}

/// Build the call arguments with `.into()` conversion.
fn call_args(adapter: &AdapterConfig) -> Vec<String> {
    adapter
        .params
        .iter()
        .map(|p| {
            if p.optional {
                format!("{}.map(Into::into)", p.name)
            } else {
                format!("{}.into()", p.name)
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
                         Some(Err(e)) => Err(PyErr::new::<PyRuntimeError, _>(e.to_string())),\n                \
                         None => Ok(None),  // StopAsyncIteration\n            \
                     }}\n        \
                 }})\n    \
             }}\n\
         }}"
    );

    let method_body = format!(
        "let inner = self.inner.clone();\n    \
         let stream = inner.{core_path}({call_str});\n    \
         Ok({iter_name} {{\n        \
             inner: Arc::new(tokio::sync::Mutex::new(stream)),\n    \
         }})"
    );

    (method_body, Some(struct_def))
}

// ---------------------------------------------------------------------------
// Node (NAPI)
// ---------------------------------------------------------------------------

fn gen_node_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let body = format!(
        "use futures::StreamExt;\n    \
         let stream = self.inner.{core_path}({call_str});\n    \
         let chunks: Vec<_> = stream\n        \
             .map(|r| r.map({item_type}::from))\n        \
             .collect::<Vec<_>>().await\n        \
             .into_iter()\n        \
             .collect::<Result<Vec<_>, _>>()\n        \
             .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;\n    \
         Ok(chunks)"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// Ruby (Magnus)
// ---------------------------------------------------------------------------

fn gen_ruby_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let body = format!(
        "use futures::StreamExt;\n    \
         let rt = tokio::runtime::Runtime::new()\n        \
             .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n    \
         let stream = self.inner.{core_path}({call_str});\n    \
         rt.block_on(async {{\n        \
             stream\n            \
                 .map(|r| r.map({item_type}::from))\n            \
                 .collect::<Vec<_>>().await\n            \
                 .into_iter()\n            \
                 .collect::<Result<Vec<_>, _>>()\n    \
         }})\n    \
         .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// PHP (ext-php-rs)
// ---------------------------------------------------------------------------

fn gen_php_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let body = format!(
        "use futures::StreamExt;\n    \
         WORKER_RUNTIME.block_on(async {{\n        \
             let stream = self.inner.{core_path}({call_str});\n        \
             stream\n            \
                 .map(|r| r.map({item_type}::from))\n            \
                 .collect::<Vec<_>>().await\n            \
                 .into_iter()\n            \
                 .collect::<Result<Vec<_>, _>>()\n    \
         }})\n    \
         .map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()).into())"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// Elixir (Rustler)
// ---------------------------------------------------------------------------

fn gen_elixir_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let body = format!(
        "use futures::StreamExt;\n    \
         let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;\n    \
         let stream = client.inner.{core_path}({call_str});\n    \
         rt.block_on(async {{\n        \
             stream\n            \
                 .map(|r| r.map({item_type}::from))\n            \
                 .collect::<Vec<_>>().await\n            \
                 .into_iter()\n            \
                 .collect::<Result<Vec<_>, _>>()\n    \
         }})\n    \
         .map_err(|e| e.to_string())"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// WASM (wasm-bindgen)
// ---------------------------------------------------------------------------

fn gen_wasm_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("JsValue");

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let body = format!(
        "use futures::StreamExt;\n    \
         let stream = self.inner.{core_path}({call_str});\n    \
         let chunks: Vec<_> = stream\n        \
             .map(|r| r.map({item_type}::from))\n        \
             .collect::<Vec<_>>().await\n        \
             .into_iter()\n        \
             .collect::<Result<Vec<_>, _>>()\n        \
             .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
         Ok(chunks)"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// FFI (C ABI) -- Streaming not supported
// ---------------------------------------------------------------------------

fn gen_ffi_body(adapter: &AdapterConfig) -> (String, Option<String>) {
    let body = format!("compile_error!(\"streaming not supported via FFI: {}\")", adapter.name);
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

fn gen_r_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("Robj");

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let body = format!(
        "use futures::StreamExt;\n    \
         let rt = tokio::runtime::Runtime::new()\n        \
             .map_err(|e| extendr_api::Error::Other(e.to_string()))?;\n    \
         let stream = self.inner.{core_path}({call_str});\n    \
         rt.block_on(async {{\n        \
             stream\n            \
                 .map(|r| r.map({item_type}::from))\n            \
                 .collect::<Vec<_>>().await\n            \
                 .into_iter()\n            \
                 .collect::<Result<Vec<_>, _>>()\n    \
         }})\n    \
         .map_err(|e| extendr_api::Error::Other(e.to_string()))"
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
