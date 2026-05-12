use heck::ToSnakeCase;

use alef_core::config::{AdapterConfig, Language, ResolvedCrateConfig};

/// Generate the method body and optionally a struct definition for a streaming adapter.
///
/// Returns `(method_body, Option<struct_definition>)`.
/// The struct definition is only produced for languages that need a separate iterator struct
/// (currently Python/PyO3).
pub fn generate_body(
    adapter: &AdapterConfig,
    language: Language,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<(String, Option<String>)> {
    match language {
        Language::Python => Ok(gen_python_body(adapter, config)),
        Language::Node => Ok(gen_node_body(adapter, config)),
        Language::Ruby => Ok(gen_ruby_body(adapter, config)),
        Language::Php => Ok(gen_php_body(adapter, config)),
        Language::Elixir => Ok(gen_elixir_body(adapter, config)),
        Language::Wasm => Ok(gen_wasm_body(adapter, config)),
        Language::Ffi => gen_ffi_body(adapter, config),
        Language::Go => Ok(gen_go_body(adapter)),
        Language::Java => Ok(gen_java_body(adapter)),
        Language::Csharp => Ok(gen_csharp_body(adapter)),
        Language::R => Ok(gen_r_body(adapter, config)),
        Language::Rust | Language::C => anyhow::bail!("Rust/C do not need generated binding adapters"),
        Language::Dart => Ok(gen_dart_body(adapter, config)),
        Language::Kotlin | Language::Swift | Language::Zig => {
            anyhow::bail!("Phase 1: {language} backend not yet implemented")
        }
    }
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

fn gen_python_body(adapter: &AdapterConfig, config: &ResolvedCrateConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");
    let error_type = adapter.error_type.as_deref().unwrap_or("anyhow::Error");
    let core_import = config.core_import_name();
    let iter_name = iterator_name(adapter);

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let anext_err_handler = if error_type != "anyhow::Error" {
        let simple_name = error_type.split("::").last().unwrap_or(error_type);
        let fn_name = format!("{}_to_py_err", simple_name.to_snake_case());
        format!("Err({fn_name}(e))")
    } else {
        "Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))".to_string()
    };

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
                         Some(Err(e)) => {anext_err_handler},\n                \
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

    let method_err_mapper = if error_type != "anyhow::Error" {
        let simple_name = error_type.split("::").last().unwrap_or(error_type);
        let fn_name = format!("{}_to_py_err", simple_name.to_snake_case());
        format!(".map_err({fn_name})")
    } else {
        ".map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))".to_string()
    };

    let method_body = format!(
        "let inner = self.inner.clone();\n    \
         {bindings_block}\
         let stream = pyo3_async_runtimes::tokio::get_runtime()\n        \
             .block_on(inner.{core_path}({call_str}))\n        \
             {method_err_mapper}?;\n    \
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

fn gen_node_body(adapter: &AdapterConfig, config: &ResolvedCrateConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let prefix = config.node_type_prefix();
    let raw_item = adapter.item_type.as_deref().unwrap_or("()");
    let item_type = if raw_item == "()" {
        raw_item.to_string()
    } else {
        format!("{prefix}{raw_item}")
    };
    let core_import = config.core_import_name();

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
         Ok(chunks)"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// Ruby (Magnus)
// ---------------------------------------------------------------------------

fn gen_ruby_body(adapter: &AdapterConfig, config: &ResolvedCrateConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");
    let core_import = config.core_import_name();

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

fn gen_php_body(adapter: &AdapterConfig, config: &ResolvedCrateConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");
    let core_import = config.core_import_name();

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

/// Streaming adapter for Rustler/Elixir.
///
/// Emits a pair of standalone `#[rustler::nif]` functions plus a per-adapter
/// handle resource. The pair is `{owner_lc}_{name}_start` (returns the resource)
/// and `{owner_lc}_{name}_next` (returns chunk JSON or `nil` on end-of-stream).
/// The Elixir wrapper drives the pair via `Stream.unfold/2`.
///
/// The corresponding method on the owner type is suppressed in `mod.rs` to avoid
/// double-emitting a NIF for the same method name.
fn gen_elixir_body(adapter: &AdapterConfig, config: &ResolvedCrateConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");
    let error_type = adapter.error_type.as_deref().unwrap_or("anyhow::Error");
    let core_import = config.core_import_name();
    let owner_type = adapter.owner_type.as_deref().unwrap_or("");
    let owner_lc = owner_type.to_lowercase();
    let adapter_name = &adapter.name;
    let handle_struct = format!("{}{}Handle", to_pascal_case(owner_type), to_pascal_case(adapter_name));
    let start_fn = format!("{owner_lc}_{adapter_name}_start");
    let next_fn = format!("{owner_lc}_{adapter_name}_next");
    let req_param_name = adapter
        .params
        .first()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "req".to_string());
    let req_param_type = adapter
        .params
        .first()
        .map(|p| p.ty.clone())
        .unwrap_or_else(|| "rustler::Term".to_string());

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let let_bindings = core_let_bindings(adapter, &core_import);
    let bindings_block = if let_bindings.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", let_bindings.join("\n    "))
    };

    // Stub method body — never used because the rustler backend skips streaming
    // methods. We keep something compilable in case the suppression is bypassed.
    let body = format!(
        "Err::<String, String>(\"streaming method emitted as standalone NIFs ({start_fn}/{next_fn})\".to_string())"
    );

    let struct_def = format!(
        "/// Streaming handle for `{owner_type}::{core_path}` — owns a Tokio runtime\n\
         /// plus the live `BoxStream`. Each call to `{next_fn}` blocks the dirty-CPU\n\
         /// scheduler thread on a single `stream.next()` poll.\n\
         pub struct {handle_struct} {{\n    \
             runtime: std::sync::Arc<tokio::runtime::Runtime>,\n    \
             stream: std::sync::Mutex<Option<futures_util::stream::BoxStream<'static, std::result::Result<{core_import}::{item_type}, {core_import}::{error_type}>>>>,\n\
         }}\n\
         \n\
         #[rustler::resource_impl()]\n\
         impl rustler::Resource for {handle_struct} {{}}\n\
         \n\
         /// Open a streaming `{core_path}` request. Returns an opaque iterator\n\
         /// resource which the Elixir wrapper drives via `Stream.unfold/2`.\n\
         #[rustler::nif(schedule = \"DirtyCpu\")]\n\
         pub fn {start_fn}(\n    \
             resource: rustler::ResourceArc<{owner_type}>,\n    \
             {req_param_name}: {req_param_type},\n\
         ) -> std::result::Result<rustler::ResourceArc<{handle_struct}>, String> {{\n    \
             {bindings_block}\
             let runtime = std::sync::Arc::new(\n        \
                 tokio::runtime::Builder::new_multi_thread()\n            \
                     .enable_all()\n            \
                     .build()\n            \
                     .map_err(|e| e.to_string())?,\n    \
             );\n    \
             let inner = resource.inner.clone();\n    \
             let stream = runtime\n        \
                 .block_on(async move {{ inner.{core_path}({call_str}).await }})\n        \
                 .map_err(|e| e.to_string())?;\n    \
             let handle = {handle_struct} {{\n        \
                 runtime,\n        \
                 stream: std::sync::Mutex::new(Some(stream)),\n    \
             }};\n    \
             Ok(rustler::ResourceArc::new(handle))\n\
         }}\n\
         \n\
         /// Pull the next chunk from a streaming handle. Returns the chunk JSON\n\
         /// (decoded by the Elixir wrapper via `Jason.decode!/1`) or `nil` to\n\
         /// signal end-of-stream. After end-of-stream the inner stream is dropped.\n\
         #[rustler::nif(schedule = \"DirtyCpu\")]\n\
         pub fn {next_fn}(\n    \
             handle: rustler::ResourceArc<{handle_struct}>,\n\
         ) -> std::result::Result<Option<String>, String> {{\n    \
             use futures_util::StreamExt;\n    \
             let runtime = handle.runtime.clone();\n    \
             let mut guard = handle.stream.lock().map_err(|e| e.to_string())?;\n    \
             let stream_ref = match guard.as_mut() {{\n        \
                 Some(s) => s,\n        \
                 None => return Ok(None),\n    \
             }};\n    \
             match runtime.block_on(stream_ref.next()) {{\n        \
                 Some(Ok(chunk)) => {{\n            \
                     let json = serde_json::to_string(&chunk).map_err(|e| e.to_string())?;\n            \
                     Ok(Some(json))\n        \
                 }}\n        \
                 Some(Err(e)) => {{\n            \
                     *guard = None;\n            \
                     Err(e.to_string())\n        \
                 }}\n        \
                 None => {{\n            \
                     *guard = None;\n            \
                     Ok(None)\n        \
                 }}\n    \
             }}\n\
         }}"
    );

    (body, Some(struct_def))
}

// ---------------------------------------------------------------------------
// WASM (wasm-bindgen)
// ---------------------------------------------------------------------------

fn gen_wasm_body(adapter: &AdapterConfig, config: &ResolvedCrateConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let prefix = config.wasm_type_prefix();
    let raw_item = adapter.item_type.as_deref().unwrap_or("JsValue");
    let _item_type = if raw_item == "()" || raw_item == "JsValue" {
        raw_item.to_string()
    } else {
        format!("{prefix}{raw_item}")
    };
    let core_import = config.core_import_name();

    let args = call_args(adapter);
    let call_str = args.join(", ");

    let let_bindings = core_let_bindings(adapter, &core_import);
    let bindings_block = if let_bindings.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", let_bindings.join("\n    "))
    };

    // WASM: collect stream chunks into a js_sys::Array of typed Wasm wrapper objects
    // (e.g. WasmChatCompletionChunk) so JavaScript callers access fields via the
    // `#[wasm_bindgen(getter, js_name = "...")]`-annotated camelCase getters.
    // Using serde_wasm_bindgen::to_value(&core_chunks) would emit snake_case field
    // names that don't match the generated e2e test assertions (finishReason, etc.).
    let core_item = adapter.item_type.as_deref().unwrap_or("()");
    let wasm_item = if core_item == "()" || core_item == "JsValue" {
        core_item.to_string()
    } else {
        format!("{prefix}{core_item}")
    };
    let body = format!(
        "use futures_util::StreamExt;\n    \
         {bindings_block}\
         let stream = self.inner.{core_path}({call_str}).await\n        \
             .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
         let core_chunks: Vec<{core_import}::{core_item}> = stream\n        \
             .collect::<Vec<_>>().await\n        \
             .into_iter()\n        \
             .collect::<std::result::Result<Vec<_>, _>>()\n        \
             .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
         let arr = js_sys::Array::new();\n    \
         for chunk in core_chunks {{\n        \
             arr.push(&JsValue::from({wasm_item}::from(chunk)));\n    \
         }}\n    \
         Ok(arr.into())"
    );

    (body, None)
}

// ---------------------------------------------------------------------------
// FFI (C ABI) -- Callback-based streaming
// ---------------------------------------------------------------------------

fn gen_ffi_body(adapter: &AdapterConfig, config: &ResolvedCrateConfig) -> anyhow::Result<(String, Option<String>)> {
    let core_path = &adapter.core_path;
    let prefix = config.ffi_prefix();
    let request_type = adapter.request_type.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "streaming adapter '{}': `request_type` is required for FFI body generation \
             (e.g. request_type = \"my_crate::MyRequest\")",
            adapter.name,
        )
    })?;

    let body = format!(
        "clear_last_error();\n\n    \
         if client.is_null() {{\n        \
             set_last_error(99, \"{prefix}_{name}: client must not be NULL\");\n        \
             return -1;\n    \
         }}\n    \
         if request_json.is_null() {{\n        \
             set_last_error(99, \"{prefix}_{name}: request_json must not be NULL\");\n        \
             return -1;\n    \
         }}\n\n    \
         // SAFETY: caller guarantees `client` and `request_json` are non-null and valid.\n    \
         let client_ref = unsafe {{ &(*client) }};\n\n    \
         let json_str = match unsafe {{ std::ffi::CStr::from_ptr(request_json) }}.to_str() {{\n        \
             Ok(s) => s,\n        \
             Err(e) => {{\n            \
                 set_last_error(99, &format!(\"{prefix}_{name}: request_json is not valid UTF-8: {{e}}\"));\n            \
                 return -1;\n        \
             }}\n    \
         }};\n\n    \
         let request: {request_type} = match serde_json::from_str(json_str) {{\n        \
             Ok(r) => r,\n        \
             Err(e) => {{\n            \
                 set_last_error(99, &format!(\"{prefix}_{name}: failed to parse request JSON: {{e}}\"));\n            \
                 return -1;\n        \
             }}\n    \
         }};\n\n    \
         let rt = get_ffi_runtime();\n\n    \
         let result = rt.block_on(async {{\n        \
             use futures_util::StreamExt;\n\n        \
             let mut stream = match client_ref.{core_path}(request).await {{\n            \
                 Ok(s) => s,\n            \
                 Err(e) => return Err(format!(\"{prefix}_{name}: failed to open stream: {{e}}\")),\n        \
             }};\n\n        \
             loop {{\n            \
                 match stream.next().await {{\n                \
                     None => break,\n                \
                     Some(Err(e)) => return Err(format!(\"{prefix}_{name}: stream error: {{e}}\")),\n                \
                     Some(Ok(chunk)) => {{\n                    \
                         let chunk_json = match serde_json::to_string(&chunk) {{\n                        \
                             Ok(s) => s,\n                        \
                             Err(e) => return Err(format!(\"{prefix}_{name}: failed to serialise chunk: {{e}}\")),\n                    \
                         }};\n                    \
                         match std::ffi::CString::new(chunk_json) {{\n                        \
                             Ok(c_str) => {{\n                            \
                                 // SAFETY: `callback` is a valid function pointer supplied by the caller.\n                            \
                                 // `c_str.as_ptr()` is valid for this block scope.\n                            \
                                 // `user_data` is forwarded as-is; ownership stays with the caller.\n                            \
                                 unsafe {{ callback(c_str.as_ptr(), user_data) }};\n                        \
                             }}\n                        \
                             Err(e) => return Err(format!(\"{prefix}_{name}: chunk JSON contained NUL byte: {{e}}\")),\n                    \
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
        prefix = prefix,
        core_path = core_path,
        request_type = request_type,
    );

    Ok((body, None))
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

fn gen_r_body(adapter: &AdapterConfig, config: &ResolvedCrateConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("Robj");
    let core_import = config.core_import_name();

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
// Dart (flutter_rust_bridge v2)
// ---------------------------------------------------------------------------

fn gen_dart_body(adapter: &AdapterConfig, config: &ResolvedCrateConfig) -> (String, Option<String>) {
    let core_path = &adapter.core_path;
    let item_type = adapter.item_type.as_deref().unwrap_or("()");
    let core_import = config.core_import_name();
    let args = call_args(adapter);
    let call_str = args.join(", ");
    let let_bindings = core_let_bindings(adapter, &core_import);
    let bindings_block = if let_bindings.is_empty() {
        String::new()
    } else {
        format!("{}\n        ", let_bindings.join("\n        "))
    };
    // FRB v2 StreamSink<T> pattern: the sink is passed as a parameter and the
    // function drives the core BoxStream into it from a spawned tokio task.
    let body = format!(
        "use futures_util::StreamExt;\n        \
         let inner = self.inner.clone();\n        \
         {bindings_block}\
         flutter_rust_bridge::spawn(async move {{\n            \
             match inner.{core_path}({call_str}).await {{\n                \
                 Ok(mut stream) => {{\n                    \
                     while let Some(item) = stream.next().await {{\n                        \
                         match item {{\n                            \
                             Ok(chunk) => {{ let _ = sink.add({item_type}::from(chunk)); }}\n                            \
                             Err(e) => {{ let _ = sink.add_error(e.to_string()); break; }}\n                        \
                         }}\n                    \
                     }}\n                \
                 }}\n                \
                 Err(e) => {{ let _ = sink.add_error(e.to_string()); }}\n            \
             }}\n        \
         }});"
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
