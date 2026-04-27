use alef_core::config::{AdapterConfig, AlefConfig, Language};

/// Generate just the function body (what goes inside `{ ... }`) for a sync function adapter.
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
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
            anyhow::bail!("Phase 1: {language} backend not yet implemented")
        }
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

// ---------------------------------------------------------------------------
// Python (PyO3)
// ---------------------------------------------------------------------------

fn gen_python_body(adapter: &AdapterConfig, _config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let returns = adapter.returns.as_deref().unwrap_or("()");
    let gil_release = adapter.gil_release;

    let args = call_args(adapter);
    let call_str = args.join(", ");

    if gil_release {
        format!(
            "py.allow_threads(|| {{\n        \
                 {core_path}({call_str})\n            \
                 .map({returns}::from)\n            \
                 .map_err(|e| PyErr::new::<PyRuntimeError, _>(e.to_string()))\n    \
             }})"
        )
    } else {
        format!(
            "{core_path}({call_str})\n        \
             .map({returns}::from)\n        \
             .map_err(|e| PyErr::new::<PyRuntimeError, _>(e.to_string()))"
        )
    }
}

// ---------------------------------------------------------------------------
// Node (NAPI)
// ---------------------------------------------------------------------------

fn gen_node_body(adapter: &AdapterConfig, _config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let returns = adapter.returns.as_deref().unwrap_or("()");

    let args = call_args(adapter);
    let call_str = args.join(", ");

    format!(
        "{core_path}({call_str})\n        \
         .map({returns}::from)\n        \
         .map_err(|e| napi::Error::from_reason(e.to_string()))"
    )
}

// ---------------------------------------------------------------------------
// Ruby (Magnus)
// ---------------------------------------------------------------------------

fn gen_ruby_body(adapter: &AdapterConfig, _config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let returns = adapter.returns.as_deref().unwrap_or("()");

    let args = call_args(adapter);
    let call_str = args.join(", ");

    format!(
        "{core_path}({call_str})\n        \
         .map({returns}::from)\n        \
         .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))"
    )
}

// ---------------------------------------------------------------------------
// PHP (ext-php-rs)
// ---------------------------------------------------------------------------

fn gen_php_body(adapter: &AdapterConfig, _config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let returns = adapter.returns.as_deref().unwrap_or("()");

    let args = call_args(adapter);
    let call_str = args.join(", ");

    format!(
        "{core_path}({call_str})\n        \
         .map({returns}::from)\n        \
         .map_err(|e| PhpException::default(e.to_string()))"
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
        "{core_path}({call_str})\n        \
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
        "{core_path}({call_str})\n        \
         .map({returns}::from)\n        \
         .map_err(|e| JsValue::from_str(&e.to_string()))"
    )
}

// ---------------------------------------------------------------------------
// FFI (C ABI)
// ---------------------------------------------------------------------------

fn gen_ffi_body(adapter: &AdapterConfig, config: &AlefConfig) -> String {
    let core_path = &adapter.core_path;
    let prefix = config.ffi_prefix();
    let name = &adapter.name;
    let _ = prefix;
    let _ = name;

    let conversions: Vec<String> = adapter
        .params
        .iter()
        .filter_map(|p| {
            if p.ty == "String" || p.ty == "&str" {
                Some(format!(
                    "let {name} = unsafe {{ std::ffi::CStr::from_ptr({name}) }}\n        \
                     .to_str()\n        \
                     .unwrap_or_default()\n        \
                     .to_owned();",
                    name = p.name
                ))
            } else {
                None
            }
        })
        .collect();

    let call_args_list: Vec<String> = adapter.params.iter().map(|p| p.name.clone()).collect();
    let call_str = call_args_list.join(", ");
    let conversion_block = if conversions.is_empty() {
        String::new()
    } else {
        format!("{}\n", conversions.join("\n"))
    };

    format!(
        "{conversion_block}\
         match {core_path}({call_str}) {{\n        \
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

    let go_name = to_pascal_case(name);

    let c_call_args: Vec<String> = adapter
        .params
        .iter()
        .map(|p| {
            if p.ty == "String" || p.ty == "&str" {
                format!("C.CString({})", p.name)
            } else {
                format!("C.{}({})", rust_type_to_c_go(&p.ty), p.name)
            }
        })
        .collect();

    let call_str = c_call_args.join(", ");
    let _ = go_name;

    format!(
        "result := C.{prefix}_{name}({call_str})\n    \
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
         \x20           var result = (MemorySegment) {prefix}_{name}.invokeExact(arena{arg_pass});\n\
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

    let call_args_list: Vec<String> = adapter.params.iter().map(|p| p.name.clone()).collect();
    let call_str = call_args_list.join(", ");

    format!(
        "var ptr = {prefix}_{name}_native({call_str});\n\
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
        "{core_path}({call_str})\n        \
         .map({returns}::from)\n        \
         .map_err(|e| extendr_api::Error::Other(e.to_string()))"
    )
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

fn rust_type_to_c_go(ty: &str) -> &str {
    match ty {
        "bool" => "int",
        "i32" => "int",
        "i64" => "longlong",
        "u32" => "uint",
        "u64" => "ulonglong",
        "f32" => "float",
        "f64" => "double",
        _ => "int",
    }
}
