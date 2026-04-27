use alef_core::config::{AdapterConfig, AlefConfig, Language};

/// Generate a callback bridge for the given adapter and language.
///
/// Returns `(struct_code, impl_code)` where:
/// - `struct_code` is a standalone struct definition for the bridge
/// - `impl_code` is the trait impl block
pub fn generate(adapter: &AdapterConfig, language: Language, config: &AlefConfig) -> anyhow::Result<(String, String)> {
    let result = match language {
        Language::Python => gen_python_body(adapter, config),
        Language::Node => gen_node_body(adapter, config),
        Language::Ruby => gen_ruby_body(adapter, config),
        Language::Php => gen_php_body(adapter, config),
        Language::Elixir => gen_elixir_body(adapter, config),
        Language::Wasm => gen_wasm_body(adapter, config),
        Language::Ffi => gen_ffi_body(adapter, config),
        Language::Go => gen_go_body(adapter, config),
        Language::Java => gen_java_body(adapter, config),
        Language::Csharp => gen_csharp_body(adapter, config),
        Language::R => gen_r_body(adapter, config),
        Language::Rust => anyhow::bail!("Rust does not need generated binding adapters"),
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
            anyhow::bail!("Phase 1: {language} backend not yet implemented")
        }
    };
    Ok(result)
}

/// Build a comma-separated parameter list for function signatures.
fn params_str(adapter: &AdapterConfig) -> String {
    adapter
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, p.ty))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Build a comma-separated list of parameter names for call sites.
fn call_args(adapter: &AdapterConfig) -> String {
    adapter
        .params
        .iter()
        .map(|p| p.name.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

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

fn trait_name(adapter: &AdapterConfig) -> &str {
    adapter.trait_name.as_deref().unwrap_or("Handler")
}

fn method_name(adapter: &AdapterConfig) -> &str {
    adapter.trait_method.as_deref().unwrap_or("handle")
}

fn returns_type(adapter: &AdapterConfig) -> &str {
    adapter.returns.as_deref().unwrap_or("()")
}

fn error_type(adapter: &AdapterConfig) -> &str {
    adapter.error_type.as_deref().unwrap_or("anyhow::Error")
}

// ---------------------------------------------------------------------------
// Python (PyO3)
// ---------------------------------------------------------------------------

fn gen_python_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, String) {
    let name = &adapter.name;
    let struct_name = format!("Py{}Bridge", to_pascal_case(name));
    let trait_nm = trait_name(adapter);
    let method_nm = method_name(adapter);
    let _returns = returns_type(adapter);
    let error = error_type(adapter);
    let _params = params_str(adapter);
    let _args = call_args(adapter);

    // Import the trait and error type from core_path
    let core_path = &adapter.core_path;
    let import_base = core_path
        .rsplit("::")
        .skip(1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("::");

    let struct_code = format!(
        "use pyo3::prelude::*;\n\
         use {import_base}::{{{trait_nm}, {error}}};\n\n\
         /// Generated FFI bridge for {trait_nm} trait — Python implementation.\n\
         pub struct {struct_name} {{\n    \
             callback: Py<PyAny>,\n    \
             is_async: bool,\n\
         }}\n\
         \n\
         impl {struct_name} {{\n    \
             /// Create a new bridge from a Python callable.\n    \
             pub fn new(py: Python<'_>, callback: &Bound<'_, pyo3::PyAny>) -> PyResult<Self> {{\n        \
                 let is_async = py.import(\"inspect\")?\n            \
                     .call_method1(\"iscoroutinefunction\", (callback,))?\n            \
                     .is_truthy()\n            \
                     .unwrap_or(false);\n        \
                 Ok(Self {{\n            \
                     callback: callback.clone().unbind(),\n            \
                     is_async,\n        \
                 }})\n    \
             }}\n\
         }}"
    );

    let impl_code = format!(
        "impl {trait_nm} for {struct_name} {{\n    \
             type Input = Py<PyAny>;\n    \
             type Output = Py<PyAny>;\n\n    \
             fn prepare_request(&self, request_data: &spikard_http::handler_trait::RequestData) -> Result<Self::Input, {error}> {{\n        \
                 Err({error}::Execution(\"prepare_request not implemented for PyHandlerBridge\".into()))\n    \
             }}\n\n    \
             fn interpret_response(&self, output: Self::Output) -> Result<axum::http::Response<axum::body::Body>, {error}> {{\n        \
                 Err({error}::Execution(\"interpret_response not implemented for PyHandlerBridge\".into()))\n    \
             }}\n\n    \
             fn {method_nm}(&self, input: Self::Input) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self::Output, {error}>> + Send + '_>> {{\n        \
                 let callback = Python::attach(|py| self.callback.clone_ref(py));\n        \
                 let is_async = self.is_async;\n        \
                 Box::pin(async move {{\n            \
                     let join_result = tokio::task::spawn_blocking(move || -> Result<Py<PyAny>, {error}> {{\n                \
                         Python::attach(|py| {{\n                    \
                             let result = callback.call1(py, (input,))\n                        \
                                 .map_err(|e| {error}::Execution(e.to_string()))?;\n                    \
                             if is_async {{\n                        \
                                 let asyncio = py.import(\"asyncio\")\n                            \
                                     .map_err(|e| {error}::Execution(e.to_string()))?;\n                        \
                                 let event_loop = asyncio.call_method0(\"new_event_loop\")\n                            \
                                     .map_err(|e| {error}::Execution(e.to_string()))?;\n                        \
                                 let awaited = event_loop.call_method1(\"run_until_complete\", (result.bind(py),))\n                            \
                                     .map_err(|e| {error}::Execution(e.to_string()))?;\n                        \
                                 Ok(awaited.unbind())\n                    \
                             }} else {{\n                        \
                                 Ok(result)\n                    \
                             }}\n                \
                         }})\n            \
                     }}).await.map_err(|e| {error}::Execution(e.to_string()))?;\n            \
                     join_result\n        \
                 }})\n    \
             }}\n\
         }}"
    );

    (struct_code, impl_code)
}

// ---------------------------------------------------------------------------
// Node (NAPI)
// ---------------------------------------------------------------------------

fn gen_node_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, String) {
    let name = &adapter.name;
    let struct_name = format!("Js{}Bridge", to_pascal_case(name));
    let trait_nm = trait_name(adapter);
    let method_nm = method_name(adapter);
    let returns = returns_type(adapter);
    let error = error_type(adapter);
    let params = params_str(adapter);
    let args = call_args(adapter);

    let struct_code = format!(
        "/// Generated FFI bridge for {trait_nm} trait — Node/NAPI implementation.\n\
         pub struct {struct_name} {{\n    \
             callback: napi::threadsafe_function::ThreadsafeFunction<\n        \
                 String,\n        \
                 napi::threadsafe_function::ErrorStrategy::Fatal,\n    \
             >,\n\
         }}\n\
         \n\
         impl {struct_name} {{\n    \
             /// Create a new bridge from a JS function.\n    \
             pub fn new(env: napi::Env, callback: napi::JsFunction) -> napi::Result<Self> {{\n        \
                 let tsfn = callback.create_threadsafe_function(\n            \
                     0,\n            \
                     |ctx: napi::threadsafe_function::ThreadSafeCallContext<String>| {{\n                \
                         Ok(vec![ctx.env.create_string(&ctx.value)?])\n            \
                     }},\n        \
                 )?;\n        \
                 Ok(Self {{ callback: tsfn }})\n    \
             }}\n\
         }}"
    );

    let impl_code = format!(
        "impl {trait_nm} for {struct_name} {{\n    \
             async fn {method_nm}(&self, {params}) -> Result<{returns}, {error}> {{\n        \
                 // TODO: serialize params into JSON for JS bridge\n        \
                 let payload = serde_json::to_string(&({args}))\n            \
                     .map_err(|e| {error}::from(e.to_string()))?;\n\
         \n        \
                 let result = self.callback.call_async::<String>(payload).await\n            \
                     .map_err(|e| {error}::from(e.to_string()))?;\n\
         \n        \
                 // TODO: deserialize JS result to {returns}\n        \
                 compile_error!(\"callback bridge JS result conversion not yet implemented for {returns}\")\n    \
             }}\n\
         }}"
    );

    (struct_code, impl_code)
}

// ---------------------------------------------------------------------------
// Ruby (Magnus)
// ---------------------------------------------------------------------------

fn gen_ruby_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, String) {
    let name = &adapter.name;
    let struct_name = format!("Rb{}Bridge", to_pascal_case(name));
    let trait_nm = trait_name(adapter);
    let method_nm = method_name(adapter);
    let returns = returns_type(adapter);
    let error = error_type(adapter);
    let params = params_str(adapter);
    let args = call_args(adapter);

    let struct_code = format!(
        "/// Generated FFI bridge for {trait_nm} trait — Ruby/Magnus implementation.\n\
         pub struct {struct_name} {{\n    \
             callback: magnus::value::Opaque<magnus::Value>,\n\
         }}\n\
         \n\
         impl {struct_name} {{\n    \
             /// Create a new bridge from a Ruby callable.\n    \
             pub fn new(callback: magnus::Value) -> Self {{\n        \
                 Self {{\n            \
                     callback: magnus::value::Opaque::from(callback),\n        \
                 }}\n    \
             }}\n\
         }}"
    );

    let impl_code = format!(
        "impl {trait_nm} for {struct_name} {{\n    \
             async fn {method_nm}(&self, {params}) -> Result<{returns}, {error}> {{\n        \
                 let callback = self.callback;\n\
         \n        \
                 // Invoke the Ruby callback under the GVL\n        \
                 let result = tokio::task::spawn_blocking(move || {{\n            \
                     let ruby = unsafe {{ magnus::Ruby::get_unchecked() }};\n            \
                     let cb = ruby.get_inner(callback);\n            \
                     // TODO: convert params to Ruby values\n            \
                     let rb_result = cb.funcall::<_, _, magnus::Value>(\"call\", ({args},))\n                \
                         .map_err(|e| {error}::from(e.to_string()))?;\n            \
                     // TODO: convert Ruby result to {returns}\n            \
                     Ok::<_, {error}>(rb_result)\n        \
                 }}).await\n        \
                 .map_err(|e| {error}::from(e.to_string()))??;\n\
         \n        \
                 let _ = result;\n        \
                 compile_error!(\"callback bridge Ruby result conversion not yet implemented for {returns}\")\n    \
             }}\n\
         }}"
    );

    (struct_code, impl_code)
}

// ---------------------------------------------------------------------------
// PHP (ext-php-rs)
// ---------------------------------------------------------------------------

fn gen_php_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, String) {
    let name = &adapter.name;
    let struct_name = format!("Php{}Bridge", to_pascal_case(name));
    let trait_nm = trait_name(adapter);
    let method_nm = method_name(adapter);
    let returns = returns_type(adapter);
    let error = error_type(adapter);
    let params = params_str(adapter);
    let args = call_args(adapter);

    let struct_code = format!(
        "/// Generated FFI bridge for {trait_nm} trait — PHP/ext-php-rs implementation.\n\
         pub struct {struct_name} {{\n    \
             callback: ext_php_rs::types::ZendCallable,\n\
         }}\n\
         \n\
         impl {struct_name} {{\n    \
             /// Create a new bridge from a PHP callable.\n    \
             pub fn new(callback: ext_php_rs::types::ZendCallable) -> Self {{\n        \
                 Self {{ callback }}\n    \
             }}\n\
         }}"
    );

    let impl_code = format!(
        "impl {trait_nm} for {struct_name} {{\n    \
             async fn {method_nm}(&self, {params}) -> Result<{returns}, {error}> {{\n        \
                 // TODO: convert params to PHP Zval values\n        \
                 let _ = ({args});\n        \
                 let result = self.callback.try_call(vec![])\n            \
                     .map_err(|e| {error}::from(e.to_string()))?;\n\
         \n        \
                 let _ = result;\n        \
                 // TODO: convert PHP result to {returns}\n        \
                 compile_error!(\"callback bridge PHP result conversion not yet implemented for {returns}\")\n    \
             }}\n\
         }}"
    );

    (struct_code, impl_code)
}

// ---------------------------------------------------------------------------
// Elixir (Rustler)
// ---------------------------------------------------------------------------

fn gen_elixir_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, String) {
    let name = &adapter.name;
    let struct_name = format!("Ex{}Bridge", to_pascal_case(name));
    let trait_nm = trait_name(adapter);
    let method_nm = method_name(adapter);
    let returns = returns_type(adapter);
    let error = error_type(adapter);
    let params = params_str(adapter);
    let args = call_args(adapter);

    let struct_code = format!(
        "/// Generated FFI bridge for {trait_nm} trait — Elixir/Rustler implementation.\n\
         pub struct {struct_name} {{\n    \
             callback: rustler::Term<'static>,\n\
         }}\n\
         \n\
         impl {struct_name} {{\n    \
             /// Create a new bridge from an Elixir function term.\n    \
             pub fn new(callback: rustler::Term<'static>) -> Self {{\n        \
                 Self {{ callback }}\n    \
             }}\n\
         }}"
    );

    let impl_code = format!(
        "impl {trait_nm} for {struct_name} {{\n    \
             async fn {method_nm}(&self, {params}) -> Result<{returns}, {error}> {{\n        \
                 // TODO: convert params to Elixir terms\n        \
                 let _ = ({args});\n        \
                 let _ = self.callback;\n        \
                 // TODO: invoke callback via Erlang NIF scheduler\n        \
                 compile_error!(\"callback bridge Elixir result conversion not yet implemented for {returns}\")\n    \
             }}\n\
         }}"
    );

    (struct_code, impl_code)
}

// ---------------------------------------------------------------------------
// WASM (wasm-bindgen)
// ---------------------------------------------------------------------------

fn gen_wasm_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, String) {
    let name = &adapter.name;
    let struct_name = format!("Wasm{}Bridge", to_pascal_case(name));
    let trait_nm = trait_name(adapter);
    let method_nm = method_name(adapter);
    let returns = returns_type(adapter);
    let error = error_type(adapter);
    let params = params_str(adapter);
    let args = call_args(adapter);

    let struct_code = format!(
        "/// Generated FFI bridge for {trait_nm} trait — WASM/wasm-bindgen implementation.\n\
         pub struct {struct_name} {{\n    \
             callback: js_sys::Function,\n\
         }}\n\
         \n\
         impl {struct_name} {{\n    \
             /// Create a new bridge from a JS function.\n    \
             pub fn new(callback: js_sys::Function) -> Self {{\n        \
                 Self {{ callback }}\n    \
             }}\n\
         }}"
    );

    let impl_code = format!(
        "impl {trait_nm} for {struct_name} {{\n    \
             async fn {method_nm}(&self, {params}) -> Result<{returns}, {error}> {{\n        \
                 // TODO: convert params to JsValue\n        \
                 let _ = ({args});\n        \
                 let this = wasm_bindgen::JsValue::NULL;\n        \
                 let result = self.callback.call0(&this)\n            \
                     .map_err(|e| {error}::from(format!(\"{{:?}}\", e)))?;\n\
         \n        \
                 // If result is a Promise, await it\n        \
                 let future = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::from(result));\n        \
                 let resolved = future.await\n            \
                     .map_err(|e| {error}::from(format!(\"{{:?}}\", e)))?;\n\
         \n        \
                 let _ = resolved;\n        \
                 // TODO: convert JsValue result to {returns}\n        \
                 compile_error!(\"callback bridge WASM result conversion not yet implemented for {returns}\")\n    \
             }}\n\
         }}"
    );

    (struct_code, impl_code)
}

// ---------------------------------------------------------------------------
// FFI (C ABI) — function pointer callback
// ---------------------------------------------------------------------------

fn gen_ffi_body(adapter: &AdapterConfig, config: &AlefConfig) -> (String, String) {
    let name = &adapter.name;
    let prefix = config.ffi_prefix();
    let struct_name = format!("Ffi{}Bridge", to_pascal_case(name));
    let trait_nm = trait_name(adapter);
    let method_nm = method_name(adapter);
    let returns = returns_type(adapter);
    let error = error_type(adapter);
    let params = params_str(adapter);
    let _ = prefix;

    let struct_code = format!(
        "/// Generated FFI bridge for {trait_nm} trait — C ABI function pointer implementation.\n\
         pub struct {struct_name} {{\n    \
             callback: extern \"C\" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char,\n\
         }}\n\
         \n\
         unsafe impl Send for {struct_name} {{}}\n\
         unsafe impl Sync for {struct_name} {{}}\n\
         \n\
         impl {struct_name} {{\n    \
             /// Create a new bridge from a C function pointer.\n    \
             pub fn new(callback: extern \"C\" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char) -> Self {{\n        \
                 Self {{ callback }}\n    \
             }}\n\
         }}"
    );

    let impl_code = format!(
        "impl {trait_nm} for {struct_name} {{\n    \
             async fn {method_nm}(&self, {params}) -> Result<{returns}, {error}> {{\n        \
                 let callback = self.callback;\n\
         \n        \
                 // Serialize request to JSON and call through C ABI\n        \
                 let result = tokio::task::spawn_blocking(move || {{\n            \
                     // TODO: serialize params to JSON CString\n            \
                     let input = std::ffi::CString::new(\"{{}}\")\n                \
                         .map_err(|e| {error}::from(e.to_string()))?;\n            \
                     let result_ptr = callback(input.as_ptr());\n            \
                     if result_ptr.is_null() {{\n                \
                         return Err({error}::from(\"callback returned null\".to_string()));\n            \
                     }}\n            \
                     let result_cstr = unsafe {{ std::ffi::CStr::from_ptr(result_ptr) }};\n            \
                     let result_str = result_cstr.to_str()\n                \
                         .map_err(|e| {error}::from(e.to_string()))?\n                \
                         .to_owned();\n            \
                     // Free the returned pointer (caller is responsible)\n            \
                     unsafe {{ libc::free(result_ptr as *mut std::ffi::c_void) }};\n            \
                     Ok::<_, {error}>(result_str)\n        \
                 }}).await\n        \
                 .map_err(|e| {error}::from(e.to_string()))??;\n\
         \n        \
                 let _ = result;\n        \
                 // TODO: deserialize JSON result to {returns}\n        \
                 compile_error!(\"callback bridge FFI result conversion not yet implemented for {returns}\")\n    \
             }}\n\
         }}"
    );

    (struct_code, impl_code)
}

// ---------------------------------------------------------------------------
// Go (wraps C FFI) — same function pointer pattern as FFI
// ---------------------------------------------------------------------------

fn gen_go_body(adapter: &AdapterConfig, config: &AlefConfig) -> (String, String) {
    let name = &adapter.name;
    let prefix = config.ffi_prefix();
    let struct_name = format!("Go{}Bridge", to_pascal_case(name));
    let trait_nm = trait_name(adapter);
    let method_nm = method_name(adapter);
    let returns = returns_type(adapter);
    let error = error_type(adapter);
    let params = params_str(adapter);
    let _ = prefix;

    let struct_code = format!(
        "/// Generated FFI bridge for {trait_nm} trait — Go/CGo function pointer implementation.\n\
         pub struct {struct_name} {{\n    \
             callback: extern \"C\" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char,\n\
         }}\n\
         \n\
         unsafe impl Send for {struct_name} {{}}\n\
         unsafe impl Sync for {struct_name} {{}}\n\
         \n\
         impl {struct_name} {{\n    \
             /// Create a new bridge from a CGo function pointer.\n    \
             pub fn new(callback: extern \"C\" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char) -> Self {{\n        \
                 Self {{ callback }}\n    \
             }}\n\
         }}"
    );

    let impl_code = format!(
        "impl {trait_nm} for {struct_name} {{\n    \
             async fn {method_nm}(&self, {params}) -> Result<{returns}, {error}> {{\n        \
                 let callback = self.callback;\n\
         \n        \
                 let result = tokio::task::spawn_blocking(move || {{\n            \
                     let input = std::ffi::CString::new(\"{{}}\")\n                \
                         .map_err(|e| {error}::from(e.to_string()))?;\n            \
                     let result_ptr = callback(input.as_ptr());\n            \
                     if result_ptr.is_null() {{\n                \
                         return Err({error}::from(\"callback returned null\".to_string()));\n            \
                     }}\n            \
                     let result_cstr = unsafe {{ std::ffi::CStr::from_ptr(result_ptr) }};\n            \
                     let result_str = result_cstr.to_str()\n                \
                         .map_err(|e| {error}::from(e.to_string()))?\n                \
                         .to_owned();\n            \
                     unsafe {{ libc::free(result_ptr as *mut std::ffi::c_void) }};\n            \
                     Ok::<_, {error}>(result_str)\n        \
                 }}).await\n        \
                 .map_err(|e| {error}::from(e.to_string()))??;\n\
         \n        \
                 let _ = result;\n        \
                 compile_error!(\"callback bridge Go FFI result conversion not yet implemented for {returns}\")\n    \
             }}\n\
         }}"
    );

    (struct_code, impl_code)
}

// ---------------------------------------------------------------------------
// Java (Panama FFI) — function pointer pattern
// ---------------------------------------------------------------------------

fn gen_java_body(adapter: &AdapterConfig, config: &AlefConfig) -> (String, String) {
    let name = &adapter.name;
    let prefix = config.ffi_prefix();
    let struct_name = format!("Java{}Bridge", to_pascal_case(name));
    let trait_nm = trait_name(adapter);
    let method_nm = method_name(adapter);
    let returns = returns_type(adapter);
    let error = error_type(adapter);
    let params = params_str(adapter);
    let _ = prefix;

    let struct_code = format!(
        "/// Generated FFI bridge for {trait_nm} trait — Java/Panama function pointer implementation.\n\
         pub struct {struct_name} {{\n    \
             callback: extern \"C\" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char,\n\
         }}\n\
         \n\
         unsafe impl Send for {struct_name} {{}}\n\
         unsafe impl Sync for {struct_name} {{}}\n\
         \n\
         impl {struct_name} {{\n    \
             /// Create a new bridge from a Java/Panama function pointer.\n    \
             pub fn new(callback: extern \"C\" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char) -> Self {{\n        \
                 Self {{ callback }}\n    \
             }}\n\
         }}"
    );

    let impl_code = format!(
        "impl {trait_nm} for {struct_name} {{\n    \
             async fn {method_nm}(&self, {params}) -> Result<{returns}, {error}> {{\n        \
                 let callback = self.callback;\n\
         \n        \
                 let result = tokio::task::spawn_blocking(move || {{\n            \
                     let input = std::ffi::CString::new(\"{{}}\")\n                \
                         .map_err(|e| {error}::from(e.to_string()))?;\n            \
                     let result_ptr = callback(input.as_ptr());\n            \
                     if result_ptr.is_null() {{\n                \
                         return Err({error}::from(\"callback returned null\".to_string()));\n            \
                     }}\n            \
                     let result_cstr = unsafe {{ std::ffi::CStr::from_ptr(result_ptr) }};\n            \
                     let result_str = result_cstr.to_str()\n                \
                         .map_err(|e| {error}::from(e.to_string()))?\n                \
                         .to_owned();\n            \
                     unsafe {{ libc::free(result_ptr as *mut std::ffi::c_void) }};\n            \
                     Ok::<_, {error}>(result_str)\n        \
                 }}).await\n        \
                 .map_err(|e| {error}::from(e.to_string()))??;\n\
         \n        \
                 let _ = result;\n        \
                 compile_error!(\"callback bridge Java FFI result conversion not yet implemented for {returns}\")\n    \
             }}\n\
         }}"
    );

    (struct_code, impl_code)
}

// ---------------------------------------------------------------------------
// C# (P/Invoke) — function pointer pattern
// ---------------------------------------------------------------------------

fn gen_csharp_body(adapter: &AdapterConfig, config: &AlefConfig) -> (String, String) {
    let name = &adapter.name;
    let prefix = config.ffi_prefix();
    let struct_name = format!("Cs{}Bridge", to_pascal_case(name));
    let trait_nm = trait_name(adapter);
    let method_nm = method_name(adapter);
    let returns = returns_type(adapter);
    let error = error_type(adapter);
    let params = params_str(adapter);
    let _ = prefix;

    let struct_code = format!(
        "/// Generated FFI bridge for {trait_nm} trait — C#/P/Invoke function pointer implementation.\n\
         pub struct {struct_name} {{\n    \
             callback: extern \"C\" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char,\n\
         }}\n\
         \n\
         unsafe impl Send for {struct_name} {{}}\n\
         unsafe impl Sync for {struct_name} {{}}\n\
         \n\
         impl {struct_name} {{\n    \
             /// Create a new bridge from a C#/P/Invoke function pointer.\n    \
             pub fn new(callback: extern \"C\" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char) -> Self {{\n        \
                 Self {{ callback }}\n    \
             }}\n\
         }}"
    );

    let impl_code = format!(
        "impl {trait_nm} for {struct_name} {{\n    \
             async fn {method_nm}(&self, {params}) -> Result<{returns}, {error}> {{\n        \
                 let callback = self.callback;\n\
         \n        \
                 let result = tokio::task::spawn_blocking(move || {{\n            \
                     let input = std::ffi::CString::new(\"{{}}\")\n                \
                         .map_err(|e| {error}::from(e.to_string()))?;\n            \
                     let result_ptr = callback(input.as_ptr());\n            \
                     if result_ptr.is_null() {{\n                \
                         return Err({error}::from(\"callback returned null\".to_string()));\n            \
                     }}\n            \
                     let result_cstr = unsafe {{ std::ffi::CStr::from_ptr(result_ptr) }};\n            \
                     let result_str = result_cstr.to_str()\n                \
                         .map_err(|e| {error}::from(e.to_string()))?\n                \
                         .to_owned();\n            \
                     unsafe {{ libc::free(result_ptr as *mut std::ffi::c_void) }};\n            \
                     Ok::<_, {error}>(result_str)\n        \
                 }}).await\n        \
                 .map_err(|e| {error}::from(e.to_string()))??;\n\
         \n        \
                 let _ = result;\n        \
                 compile_error!(\"callback bridge C# FFI result conversion not yet implemented for {returns}\")\n    \
             }}\n\
         }}"
    );

    (struct_code, impl_code)
}

// ---------------------------------------------------------------------------
// R (extendr)
// ---------------------------------------------------------------------------

fn gen_r_body(adapter: &AdapterConfig, _config: &AlefConfig) -> (String, String) {
    let name = &adapter.name;
    let struct_name = format!("R{}Bridge", to_pascal_case(name));
    let trait_nm = trait_name(adapter);
    let method_nm = method_name(adapter);
    let returns = returns_type(adapter);
    let error = error_type(adapter);
    let params = params_str(adapter);
    let args = call_args(adapter);

    let struct_code = format!(
        "/// Generated FFI bridge for {trait_nm} trait — R/extendr implementation.\n\
         pub struct {struct_name} {{\n    \
             callback: extendr_api::Robj,\n\
         }}\n\
         \n\
         impl {struct_name} {{\n    \
             /// Create a new bridge from an R function.\n    \
             pub fn new(callback: extendr_api::Robj) -> Self {{\n        \
                 Self {{ callback }}\n    \
             }}\n\
         }}"
    );

    let impl_code = format!(
        "impl {trait_nm} for {struct_name} {{\n    \
             async fn {method_nm}(&self, {params}) -> Result<{returns}, {error}> {{\n        \
                 // TODO: convert params to R values\n        \
                 let _ = ({args});\n        \
                 let result = self.callback.call(pairlist!())\n            \
                     .map_err(|e| {error}::from(e.to_string()))?;\n\
         \n        \
                 let _ = result;\n        \
                 // TODO: convert R result to {returns}\n        \
                 compile_error!(\"callback bridge R result conversion not yet implemented for {returns}\")\n    \
             }}\n\
         }}"
    );

    (struct_code, impl_code)
}
