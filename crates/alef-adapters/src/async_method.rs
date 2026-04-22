use alef_core::config::{AdapterConfig, AlefConfig, Language};

/// Generate just the method body (what goes inside `{ ... }`) for an async method adapter.
pub fn generate_body(adapter: &AdapterConfig, language: Language, config: &AlefConfig) -> anyhow::Result<String> {
    let body = match language {
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
    };
    Ok(body)
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

/// Build conversion let-bindings for core types (used in Python async).
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

/// Build core call arguments (prefixed with core_).
fn core_call_args(adapter: &AdapterConfig) -> Vec<String> {
    adapter.params.iter().map(|p| format!("core_{}", p.name)).collect()
}

// ---------------------------------------------------------------------------
// Python (PyO3)
// ---------------------------------------------------------------------------

fn gen_python_body(adapter: &AdapterConfig, config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let returns = adapter.returns.as_deref().unwrap_or("()");
    let core_import = config.core_import();

    let let_bindings = core_let_bindings(adapter, &core_import);
    let core_args = core_call_args(adapter);
    let core_call_str = core_args.join(", ");

    let bindings_block = if let_bindings.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", let_bindings.join("\n    "))
    };

    format!(
        "let inner = self.inner.clone();\n    \
         {bindings_block}\
         pyo3_async_runtimes::tokio::future_into_py(py, async move {{\n        \
             let result = inner.{core_path}({core_call_str}).await\n            \
                 .map_err(|e| PyErr::new::<PyRuntimeError, _>(e.to_string()))?;\n        \
             Ok({returns}::from(result))\n    \
         }})"
    )
}

// ---------------------------------------------------------------------------
// Node (NAPI)
// ---------------------------------------------------------------------------

fn gen_node_body(adapter: &AdapterConfig, config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let prefix = config.node_type_prefix();
    let raw_returns = adapter.returns.as_deref().unwrap_or("()");
    let returns = if raw_returns == "()" {
        raw_returns.to_string()
    } else {
        format!("{prefix}{raw_returns}")
    };

    let args = call_args(adapter);

    if args.is_empty() {
        format!(
            "self.inner.{core_path}().await\n        \
             .map({returns}::from)\n        \
             .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
        )
    } else {
        let call_str = args.join(", ");
        format!(
            "let core_req = {call_str};\n    \
             self.inner.{core_path}(core_req).await\n        \
             .map({returns}::from)\n        \
             .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
        )
    }
}

// ---------------------------------------------------------------------------
// Ruby (Magnus)
// ---------------------------------------------------------------------------

fn gen_ruby_body(adapter: &AdapterConfig, _config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let returns = adapter.returns.as_deref().unwrap_or("()");

    let args = call_args(adapter);

    if args.is_empty() {
        format!(
            "let rt = tokio::runtime::Runtime::new()\n        \
                 .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n    \
             rt.block_on(async {{ self.inner.{core_path}().await }})\n        \
             .map({returns}::from)\n        \
             .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))"
        )
    } else {
        let call_str = args.join(", ");
        format!(
            "let rt = tokio::runtime::Runtime::new()\n        \
                 .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n    \
             let core_req = {call_str};\n    \
             rt.block_on(async {{ self.inner.{core_path}(core_req).await }})\n        \
             .map({returns}::from)\n        \
             .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))"
        )
    }
}

// ---------------------------------------------------------------------------
// PHP (ext-php-rs)
// ---------------------------------------------------------------------------

fn gen_php_body(adapter: &AdapterConfig, _config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let returns = adapter.returns.as_deref().unwrap_or("()");

    let args = call_args(adapter);

    let inner_call = if args.is_empty() {
        format!("self.inner.{core_path}().await")
    } else {
        let call_str = args.join(", ");
        format!("self.inner.{core_path}({call_str}.into()).await")
    };

    format!(
        "WORKER_RUNTIME.block_on(async {{\n        \
             {inner_call}\n    \
         }})\n    \
         .map({returns}::from)\n    \
         .map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()).into())"
    )
}

// ---------------------------------------------------------------------------
// Elixir (Rustler)
// ---------------------------------------------------------------------------

fn gen_elixir_body(adapter: &AdapterConfig, _config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let returns = adapter.returns.as_deref().unwrap_or("()");

    let args = call_args(adapter);
    let call_str = args.join(", ");

    format!(
        "let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;\n    \
         rt.block_on(async {{ resource.inner.{core_path}({call_str}).await }})\n        \
         .map({returns}::from)\n        \
         .map_err(|e| e.to_string())"
    )
}

// ---------------------------------------------------------------------------
// WASM (wasm-bindgen)
// ---------------------------------------------------------------------------

fn gen_wasm_body(adapter: &AdapterConfig, _config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let returns = adapter.returns.as_deref().unwrap_or("JsValue");

    let args = call_args(adapter);
    let call_str = args.join(", ");

    format!(
        "self.inner.{core_path}({call_str}).await\n        \
         .map({returns}::from)\n        \
         .map_err(|e| JsValue::from_str(&e.to_string()))"
    )
}

// ---------------------------------------------------------------------------
// FFI (C ABI) -- async becomes sync via block_on
// ---------------------------------------------------------------------------

fn gen_ffi_body(adapter: &AdapterConfig, config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let prefix = config.ffi_prefix();
    let owner_type = adapter.owner_type.as_deref().unwrap_or("Self");
    let owner_snake = to_snake_case(owner_type);
    let _ = prefix;
    let _ = owner_snake;

    let conversions: Vec<String> = adapter
        .params
        .iter()
        .map(|p| {
            if p.ty == "String" || p.ty == "&str" {
                format!(
                    "let {name} = unsafe {{ std::ffi::CStr::from_ptr({name}) }}\n        \
                     .to_str()\n        \
                     .unwrap_or_default()\n        \
                     .to_owned();",
                    name = p.name,
                )
            } else {
                format!(
                    "let {name}_str = unsafe {{ std::ffi::CStr::from_ptr({name}_json) }}\n        \
                     .to_str()\n        \
                     .unwrap_or_default();\n    \
                     let {name}: {ty} = match serde_json::from_str({name}_str) {{\n        \
                         Ok(v) => v,\n        \
                         Err(e) => {{\n            \
                             update_last_error(e);\n            \
                             return std::ptr::null_mut();\n        \
                         }}\n    \
                     }};",
                    name = p.name,
                    ty = p.ty,
                )
            }
        })
        .collect();

    let call_args_list: Vec<String> = adapter.params.iter().map(|p| p.name.clone()).collect();
    let call_str = call_args_list.join(", ");
    let conversion_block = if conversions.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", conversions.join("\n    "))
    };

    format!(
        "let client = unsafe {{ &*client }};\n    \
         {conversion_block}\
         let rt = match tokio::runtime::Runtime::new() {{\n        \
             Ok(rt) => rt,\n        \
             Err(e) => {{\n            \
                 update_last_error(e);\n            \
                 return std::ptr::null_mut();\n        \
             }}\n    \
         }};\n    \
         match rt.block_on(async {{ client.inner.{core_path}({call_str}).await }}) {{\n        \
             Ok(result) => {{\n            \
                 let json = serde_json::to_string(&result).unwrap_or_default();\n            \
                 std::ffi::CString::new(json).unwrap_or_default().into_raw()\n        \
             }}\n        \
             Err(e) => {{\n            \
                 update_last_error(e);\n            \
                 std::ptr::null_mut()\n        \
             }}\n    \
         }}"
    )
}

// ---------------------------------------------------------------------------
// Go (wraps C FFI)
// ---------------------------------------------------------------------------

fn gen_go_body(adapter: &AdapterConfig, config: &AlefConfig) -> String {
    let name = &adapter.name;
    let prefix = config.ffi_prefix();
    let returns = adapter.returns.as_deref().unwrap_or("string");
    let owner_type = adapter.owner_type.as_deref().unwrap_or("Client");
    let owner_snake = to_snake_case(owner_type);

    let marshal_block: Vec<String> = adapter
        .params
        .iter()
        .filter(|p| p.ty != "String" && p.ty != "&str")
        .map(|p| {
            format!(
                "{name}JSON, err := json.Marshal({name})\n    \
                 if err != nil {{\n        \
                     return nil, err\n    \
                 }}",
                name = p.name,
            )
        })
        .collect();

    let c_call_args: Vec<String> = adapter
        .params
        .iter()
        .map(|p| {
            if p.ty == "String" || p.ty == "&str" {
                format!("C.CString({})", p.name)
            } else {
                format!("C.CString(string({name}JSON))", name = p.name)
            }
        })
        .collect();

    let call_str = c_call_args.join(", ");
    let marshal_str = if marshal_block.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", marshal_block.join("\n    "))
    };

    format!(
        "{marshal_str}\
         result := C.{prefix}_{owner_snake}_{name}(c.ptr, {call_str})\n    \
         if result == nil {{\n        \
             return nil, fmt.Errorf(\"%s\", lastError())\n    \
         }}\n    \
         defer C.free(unsafe.Pointer(result))\n    \
         var out {returns}\n    \
         if err := json.Unmarshal([]byte(C.GoString(result)), &out); err != nil {{\n        \
             return nil, err\n    \
         }}\n    \
         return &out, nil"
    )
}

// ---------------------------------------------------------------------------
// Java (Panama FFI)
// ---------------------------------------------------------------------------

fn gen_java_body(adapter: &AdapterConfig, config: &AlefConfig) -> String {
    let name = &adapter.name;
    let prefix = config.ffi_prefix();
    let owner_type = adapter.owner_type.as_deref().unwrap_or("Client");
    let owner_snake = to_snake_case(owner_type);

    let arg_pass = if adapter.params.is_empty() {
        String::new()
    } else {
        format!(
            ", {}",
            adapter
                .params
                .iter()
                .map(|p| {
                    if p.ty == "String" || p.ty == "&str" {
                        format!("arena.allocateFrom({})", p.name)
                    } else {
                        p.name.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    format!(
        "try (var arena = Arena.ofConfined()) {{\n\
         \x20           var result = (MemorySegment) {prefix}_{owner_snake}_{name}.invokeExact(this.handle, arena{arg_pass});\n\
         \x20           if (result.equals(MemorySegment.NULL)) {{\n\
         \x20               throw new RuntimeException(lastError());\n\
         \x20           }}\n\
         \x20           return result.getString(0);\n\
         \x20       }}"
    )
}

// ---------------------------------------------------------------------------
// C# (P/Invoke)
// ---------------------------------------------------------------------------

fn gen_csharp_body(adapter: &AdapterConfig, config: &AlefConfig) -> String {
    let name = &adapter.name;
    let prefix = config.ffi_prefix();
    let owner_type = adapter.owner_type.as_deref().unwrap_or("Client");
    let owner_snake = to_snake_case(owner_type);

    let call_args_list: Vec<String> = adapter.params.iter().map(|p| p.name.clone()).collect();
    let call_str = call_args_list.join(", ");
    let call_pass = if call_str.is_empty() {
        String::new()
    } else {
        format!(", {}", call_str)
    };

    format!(
        "var ptr = {prefix}_{owner_snake}_{name}_native(this.handle{call_pass});\n\
         \x20       if (ptr == IntPtr.Zero)\n\
         \x20           throw new InvalidOperationException(GetLastError());\n\
         \x20       try {{ return Marshal.PtrToStringUTF8(ptr)!; }}\n\
         \x20       finally {{ FreeString(ptr); }}"
    )
}

// ---------------------------------------------------------------------------
// R (extendr)
// ---------------------------------------------------------------------------

fn gen_r_body(adapter: &AdapterConfig, _config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let returns = adapter.returns.as_deref().unwrap_or("Robj");

    let args = call_args(adapter);
    let call_str = args.join(", ");

    format!(
        "let rt = tokio::runtime::Runtime::new()\n        \
             .map_err(|e| extendr_api::Error::Other(e.to_string()))?;\n    \
         rt.block_on(async {{ self.inner.{core_path}({call_str}).await }})\n        \
         .map({returns}::from)\n        \
         .map_err(|e| extendr_api::Error::Other(e.to_string()))"
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.extend(ch.to_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}
