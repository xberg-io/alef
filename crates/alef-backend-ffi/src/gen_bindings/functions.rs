use std::fmt::Write;

use ahash::AHashMap;
use alef_codegen::conversions::core_type_path;
use alef_core::ir::{FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use heck::ToSnakeCase;

use crate::type_map::{c_param_type_with_paths, c_return_type_with_paths, is_void_return};

use super::helpers::{gen_ffi_unimplemented_body, gen_owned_value_to_c, null_return_value};

// ---------------------------------------------------------------------------
// Streaming method wrapper (callback-based, for Streaming adapters)
// ---------------------------------------------------------------------------

/// Generate a callback-based streaming wrapper for a method decorated with the
/// `Streaming` adapter pattern.  The caller supplies a `LiterLlmStreamCallback`
/// and an opaque `user_data` pointer; the body drives the async stream and
/// invokes the callback once per chunk.
pub(super) fn gen_streaming_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    prefix: &str,
    core_import: &str,
    body: &str,
) -> String {
    let type_snake = typ.name.to_snake_case();
    let method_name = &method.name;
    let fn_name = format!("{prefix}_{type_snake}_{method_name}");
    let qualified = core_type_path(typ, core_import);

    let mut out = String::with_capacity(2048);

    if !method.doc.is_empty() {
        for line in method.doc.lines() {
            writeln!(out, "/// {}", line).ok();
        }
    }
    writeln!(out, "/// # Safety").ok();
    writeln!(
        out,
        "/// `client` and `request_json` must be non-null valid pointers. \
         `callback` must be a valid function pointer. \
         `user_data` is forwarded as-is; ownership stays with the caller."
    )
    .ok();
    writeln!(out, "#[unsafe(no_mangle)]").ok();
    writeln!(out, "pub unsafe extern \"C\" fn {fn_name}(").ok();
    writeln!(out, "    client: *const {qualified},").ok();
    writeln!(out, "    request_json: *const std::ffi::c_char,").ok();
    writeln!(out, "    callback: LiterLlmStreamCallback,").ok();
    writeln!(out, "    user_data: *mut std::ffi::c_void,").ok();
    writeln!(out, ") -> i32 {{").ok();
    writeln!(out, "    {}", body.replace('\n', "\n    ")).ok();
    write!(out, "}}").ok();
    out
}

// ---------------------------------------------------------------------------
// Method wrappers
// ---------------------------------------------------------------------------

pub(super) fn gen_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    prefix: &str,
    core_import: &str,
    path_map: &AHashMap<String, String>,
) -> String {
    let type_snake = typ.name.to_snake_case();
    let type_name = &typ.name;
    let method_name = &method.name;
    let fn_name = format!("{prefix}_{type_snake}_{method_name}");

    let mut out = String::with_capacity(2048);

    if !method.doc.is_empty() {
        for line in method.doc.lines() {
            writeln!(out, "/// {}", line).ok();
        }
    }
    writeln!(out, "/// # Safety").ok();
    writeln!(out, "/// Caller must ensure all pointer arguments are valid or null.").ok();
    writeln!(
        out,
        "/// Returned pointers must be freed with the appropriate free function."
    )
    .ok();
    // Count total FFI params: this + params + extra _len for Bytes params
    let ffi_param_count = (if method.is_static { 0 } else { 1 })
        + method.params.len()
        + method.params.iter().filter(|p| matches!(p.ty, TypeRef::Bytes)).count();
    if ffi_param_count > 7 {
        writeln!(out, "#[allow(clippy::too_many_arguments)]").ok();
    }
    writeln!(out, "#[unsafe(no_mangle)]").ok();

    let qualified = core_type_path(typ, core_import);

    // Return type
    let has_error = method.error_type.is_some();
    let mut ret_type = if has_error && is_void_return(&method.return_type) {
        "i32".to_string() // 0 = success, nonzero = error
    } else if has_error {
        // Fallible + non-void: return nullable pointer
        match &method.return_type {
            TypeRef::Primitive(_) => c_return_type_with_paths(&method.return_type, core_import, path_map).into_owned(),
            _ => c_return_type_with_paths(&method.return_type, core_import, path_map).into_owned(),
        }
    } else {
        c_return_type_with_paths(&method.return_type, core_import, path_map).into_owned()
    };

    // Replace "Self" with the actual qualified type name in FFI signatures
    if ret_type.contains("Self") {
        ret_type = ret_type.replace("Self", &qualified);
    }

    // Check if this method will be unimplemented before building params
    let will_be_unimplemented = method.sanitized;

    // Build parameter list — prefix with _ if unimplemented
    let mut params = Vec::new();
    if !method.is_static {
        let receiver_ty = match method.receiver.as_ref().unwrap_or(&ReceiverKind::Ref) {
            ReceiverKind::Ref => format!("*const {qualified}"),
            ReceiverKind::RefMut | ReceiverKind::Owned => format!("*mut {qualified}"),
        };
        let param_name = if will_be_unimplemented { "_this" } else { "this" };
        params.push(format!("    {param_name}: {receiver_ty}"));
    }
    for p in &method.params {
        let param_name = if will_be_unimplemented {
            format!("_{}", p.name)
        } else {
            p.name.clone()
        };
        params.push(format!(
            "    {}: {}",
            param_name,
            c_param_type_with_paths(&p.ty, core_import, path_map)
        ));
        // Bytes parameters need a separate length parameter
        if matches!(p.ty, TypeRef::Bytes) {
            let len_param_name = if will_be_unimplemented {
                format!("_{}_len", p.name)
            } else {
                format!("{}_len", p.name)
            };
            params.push(format!("    {}: usize", len_param_name));
        }
    }

    if is_void_return(&method.return_type) && !has_error {
        writeln!(out, "pub unsafe extern \"C\" fn {fn_name}(").ok();
        writeln!(out, "{}", params.join(",\n")).ok();
        writeln!(out, ") {{").ok();
    } else {
        writeln!(out, "pub unsafe extern \"C\" fn {fn_name}(").ok();
        writeln!(out, "{}", params.join(",\n")).ok();
        writeln!(out, ") -> {ret_type} {{").ok();
    }

    writeln!(out, "    clear_last_error();").ok();

    // If method signature was sanitized, generate unimplemented body
    if will_be_unimplemented {
        writeln!(
            out,
            "{}",
            gen_ffi_unimplemented_body(&method.return_type, &format!("{type_name}::{method_name}"), has_error)
        )
        .ok();
        write!(out, "}}").ok();
        return out;
    }

    // Null-check self
    if !method.is_static {
        writeln!(out, "    if this.is_null() {{").ok();
        writeln!(out, "        set_last_error(1, \"Null pointer passed for self\");").ok();
        let fail_ret = if has_error && is_void_return(&method.return_type) {
            "return -1;".to_string()
        } else if is_void_return(&method.return_type) {
            "return;".to_string()
        } else {
            format!("return {};", null_return_value(&method.return_type))
        };
        writeln!(out, "        {fail_ret}").ok();
        writeln!(out, "    }}").ok();

        let deref = match method.receiver.as_ref().unwrap_or(&ReceiverKind::Ref) {
            ReceiverKind::Ref => {
                "// SAFETY: null check above guarantees this is a valid pointer.\n    let obj = unsafe { &*this };"
                    .to_string()
            }
            ReceiverKind::RefMut => {
                "// SAFETY: null check above guarantees this is a valid pointer; caller ensures exclusive access.\n    let obj = unsafe { &mut *this };"
                    .to_string()
            }
            ReceiverKind::Owned => {
                "// SAFETY: null check above guarantees this is a valid pointer originally from Box::into_raw.\n    let obj = unsafe { *Box::from_raw(this) };"
                    .to_string()
            }
        };
        writeln!(out, "    {deref}").ok();
    }

    // Null-check and convert each parameter
    for p in &method.params {
        write!(
            out,
            "{}",
            gen_param_conversion(p, has_error, &method.return_type, core_import)
        )
        .ok();
    }

    // Emit Vec<&str> intermediate bindings for Vec<String> params with is_ref=true.
    // gen_param_conversion produces a Vec<String> ({name}_rs), but the core expects &[&str].
    // A &Vec<String> cannot coerce to &[&str], so we collect a Vec<&str> first.
    for p in &method.params {
        if p.is_ref && !p.optional {
            if let TypeRef::Vec(inner) = &p.ty {
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) {
                    let rs = format!("{}_rs", p.name);
                    writeln!(
                        out,
                        "    let {rs}_refs: Vec<&str> = {rs}.iter().map(|s| s.as_str()).collect();"
                    )
                    .ok();
                }
            }
        }
    }

    // Build the call expression — pass &ref for String/Bytes params, owned for Path/Named
    let is_owned_receiver = method.receiver.as_ref() == Some(&ReceiverKind::Owned);
    let arg_names: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let rs = format!("{}_rs", p.name);
            match &p.ty {
                TypeRef::Path if !p.optional => {
                    // Pass &Path when is_ref=true, otherwise pass owned PathBuf
                    if p.is_ref { format!("{rs}.as_path()") } else { rs }
                }
                TypeRef::Named(_) if !p.optional => {
                    // Pass by value when method takes owned (Owned receiver or is_ref=false)
                    if is_owned_receiver || !p.is_ref {
                        rs
                    } else {
                        format!("&{rs}")
                    }
                }
                TypeRef::String | TypeRef::Char if !p.optional => {
                    // Pass &str when is_ref=true, otherwise pass owned String
                    if p.is_ref { format!("&{rs}") } else { rs }
                }
                TypeRef::Bytes if !p.optional => {
                    // Pass &[u8] when is_ref=true (function takes &[u8]),
                    // otherwise pass owned Vec<u8>
                    if p.is_ref { format!("&{rs}") } else { rs }
                }
                TypeRef::String | TypeRef::Char | TypeRef::Bytes if p.optional => {
                    // Only convert to &str slice when the core param is a reference (&str).
                    // When is_ref=false, the core takes Option<String> — pass owned.
                    if p.is_ref { format!("{rs}.as_deref()") } else { rs }
                }
                TypeRef::Path if p.optional => {
                    // Optional Path: rs is Option<String> (from param conversion)
                    // If is_ref=true, convert to Option<&Path>; else Option<PathBuf>
                    if p.is_ref {
                        format!("{rs}.as_ref().map(|s| std::path::Path::new(s.as_str()))")
                    } else {
                        format!("{rs}.map(std::path::PathBuf::from)")
                    }
                }
                TypeRef::Named(_) if p.optional => {
                    // Optional Named: rs is Option<T>
                    // If is_ref=true, convert to Option<&T>; else pass owned
                    if p.is_ref { format!("{rs}.as_ref()") } else { rs }
                }
                TypeRef::Json if !p.optional => {
                    // Json: rs is already serde_json::Value (from param conversion)
                    // If is_ref=true, pass &value; else pass owned
                    if p.is_ref { format!("&{rs}") } else { rs }
                }
                TypeRef::Json if p.optional => {
                    // Optional Json: rs is Option<Value>
                    // If is_ref=true, convert to Option<&Value>; else pass owned
                    if p.is_ref { format!("{rs}.as_ref()") } else { rs }
                }
                TypeRef::Vec(inner) if !p.optional => {
                    // Vec<String> with is_ref=true: core expects &[&str].
                    // Use the {rs}_refs intermediate (Vec<&str>) and pass as slice.
                    if p.is_ref && matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) {
                        format!("&{rs}_refs")
                    } else if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref {
                        format!("&{rs}")
                    } else {
                        rs
                    }
                }
                TypeRef::Map(_, _) if !p.optional => {
                    // When is_ref=true, pass &map. When is_mut=true, pass &mut map.
                    // Otherwise pass the map owned.
                    if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref {
                        format!("&{rs}")
                    } else {
                        rs
                    }
                }
                TypeRef::Vec(_) | TypeRef::Map(_, _) if p.optional => {
                    // Optional Vec/Map: rs is Option<Vec<T>> or Option<HashMap<K, V>>
                    // If is_ref=true, convert to Option<&[T]> with .as_deref()
                    // If is_mut=true, convert to Option<&mut Vec<T>> with .as_deref_mut()
                    // Otherwise pass owned Option
                    if p.is_mut {
                        format!("{rs}.as_deref_mut()")
                    } else if p.is_ref {
                        format!("{rs}.as_deref()")
                    } else {
                        rs
                    }
                }
                _ => rs,
            }
        })
        .collect();
    let call_args = arg_names.join(", ");

    if method.is_async {
        if method.is_static {
            writeln!(
                out,
                "    let result = get_ffi_runtime().block_on(async {{ {qualified}::{method_name}({call_args}).await }});"
            )
            .ok();
        } else {
            writeln!(
                out,
                "    let result = get_ffi_runtime().block_on(async {{ obj.{method_name}({call_args}).await }});"
            )
            .ok();
        }
    } else if method.is_static {
        writeln!(out, "    let result = {qualified}::{method_name}({call_args});").ok();
    } else if method_name == "drop" {
        // Special case: Rust's drop method cannot be called directly with dot notation.
        // Use std::mem::drop instead.
        writeln!(out, "    std::mem::drop(obj);").ok();
    } else {
        writeln!(out, "    let result = obj.{method_name}({call_args});").ok();
    }

    // Handle return
    // When return_newtype_wrapper is set, the core function returns a newtype (e.g. NodeIndex)
    // but the IR has already resolved it to the inner type (e.g. u32). Unwrap with `.0`.
    let result_expr = if method.return_newtype_wrapper.is_some() && matches!(method.return_type, TypeRef::Primitive(_))
    {
        "result.0"
    } else {
        "result"
    };
    // When returns_ref=true, the core returns a reference (&T or &[T]).
    // We need to convert it to an owned value for C FFI:
    // - For String/&str: clone to owned String
    // - For Named/&T: clone to owned T
    // - For Vec/&[T]: clone to owned Vec
    // This must happen before passing to gen_owned_value_to_c.
    if method.returns_ref && !has_error {
        match &method.return_type {
            TypeRef::String | TypeRef::Char => {
                writeln!(out, "    let result = result.clone();").ok();
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                writeln!(out, "    let result = result.clone();").ok();
            }
            TypeRef::Named(_) => {
                writeln!(out, "    let result = result.clone();").ok();
            }
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::Named(_) | TypeRef::String | TypeRef::Char | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                    writeln!(out, "    let result = result.cloned();").ok();
                }
                _ => {}
            },
            _ => {}
        }
    }
    // When returns_cow=true, the core returns Cow<'_, T> but FFI needs owned T.
    // Convert to owned by calling .into_owned().
    if method.returns_cow && !has_error {
        writeln!(out, "    let result = result.into_owned();").ok();
    }
    if has_error {
        writeln!(out, "    match result {{").ok();
        if is_void_return(&method.return_type) {
            writeln!(out, "        Ok(()) => 0,").ok();
        } else {
            writeln!(out, "        Ok(val) => {{").ok();
            let val_expr =
                if method.return_newtype_wrapper.is_some() && matches!(method.return_type, TypeRef::Primitive(_)) {
                    "val.0"
                } else {
                    "val"
                };
            write!(
                out,
                "{}",
                gen_owned_value_to_c(val_expr, &method.return_type, "            ")
            )
            .ok();
            writeln!(out, "        }}").ok();
        }
        writeln!(out, "        Err(e) => {{").ok();
        writeln!(out, "            set_last_error(2, &e.to_string());").ok();
        if is_void_return(&method.return_type) {
            writeln!(out, "            -1").ok();
        } else {
            writeln!(out, "            {}", null_return_value(&method.return_type)).ok();
        }
        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
    } else if is_void_return(&method.return_type) {
        // void, no error — result is already ()
    } else {
        write!(
            out,
            "{}",
            gen_owned_value_to_c(result_expr, &method.return_type, "    ")
        )
        .ok();
    }

    write!(out, "}}").ok();
    out
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

pub(super) fn gen_free_function(
    func: &FunctionDef,
    prefix: &str,
    core_import: &str,
    path_map: &AHashMap<String, String>,
) -> String {
    let fn_name_snake = func.name.to_snake_case();
    let ffi_name = format!("{prefix}_{fn_name_snake}");
    // Use the full rust_path for correct module path resolution
    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };
    let func_name = &func.name;

    let mut out = String::with_capacity(2048);

    if !func.doc.is_empty() {
        for line in func.doc.lines() {
            writeln!(out, "/// {}", line).ok();
        }
    }
    writeln!(out, "/// # Safety").ok();
    writeln!(out, "/// Caller must ensure all pointer arguments are valid or null.").ok();
    writeln!(
        out,
        "/// Returned pointers must be freed with the appropriate free function."
    )
    .ok();
    // Count total FFI params: params + extra _len for Bytes params
    let ffi_param_count = func.params.len() + func.params.iter().filter(|p| matches!(p.ty, TypeRef::Bytes)).count();
    if ffi_param_count > 7 {
        writeln!(out, "#[allow(clippy::too_many_arguments)]").ok();
    }
    writeln!(out, "#[unsafe(no_mangle)]").ok();

    let has_error = func.error_type.is_some();
    let ret_type = if has_error && is_void_return(&func.return_type) {
        "i32".to_string()
    } else {
        c_return_type_with_paths(&func.return_type, core_import, path_map).into_owned()
    };

    // Check if this function will be unimplemented before building params
    let will_be_unimplemented = func.sanitized;

    // Build parameter list — prefix with _ if unimplemented
    let mut params = Vec::new();
    for p in &func.params {
        let param_name = if will_be_unimplemented {
            format!("_{}", p.name)
        } else {
            p.name.clone()
        };
        params.push(format!(
            "    {}: {}",
            param_name,
            c_param_type_with_paths(&p.ty, core_import, path_map)
        ));
        // Bytes parameters need a separate length parameter
        if matches!(p.ty, TypeRef::Bytes) {
            let len_param_name = if will_be_unimplemented {
                format!("_{}_len", p.name)
            } else {
                format!("{}_len", p.name)
            };
            params.push(format!("    {}: usize", len_param_name));
        }
    }

    if is_void_return(&func.return_type) && !has_error {
        writeln!(out, "pub unsafe extern \"C\" fn {ffi_name}(").ok();
        writeln!(out, "{}", params.join(",\n")).ok();
        writeln!(out, ") {{").ok();
    } else {
        writeln!(out, "pub unsafe extern \"C\" fn {ffi_name}(").ok();
        writeln!(out, "{}", params.join(",\n")).ok();
        writeln!(out, ") -> {ret_type} {{").ok();
    }

    writeln!(out, "    clear_last_error();").ok();

    // If function signature was sanitized or involves opaque types, generate unimplemented body
    if will_be_unimplemented {
        writeln!(
            out,
            "{}",
            gen_ffi_unimplemented_body(&func.return_type, func_name, has_error)
        )
        .ok();
        write!(out, "}}").ok();
        return out;
    }

    // Convert parameters
    for p in &func.params {
        write!(
            out,
            "{}",
            gen_param_conversion(p, has_error, &func.return_type, core_import)
        )
        .ok();
    }

    // Emit Vec<&str> intermediate bindings for Vec<String> params with is_ref=true.
    // gen_param_conversion produces a Vec<String> ({name}_rs), but the core expects &[&str].
    // A &Vec<String> cannot coerce to &[&str], so we collect a Vec<&str> first.
    for p in &func.params {
        if p.is_ref && !p.optional {
            if let TypeRef::Vec(inner) = &p.ty {
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) {
                    let rs = format!("{}_rs", p.name);
                    writeln!(
                        out,
                        "    let {rs}_refs: Vec<&str> = {rs}.iter().map(|s| s.as_str()).collect();"
                    )
                    .ok();
                }
            }
        }
    }

    // Call — pass &ref for String/Bytes/Named params, owned for Path
    let arg_names: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let rs = format!("{}_rs", p.name);
            match &p.ty {
                TypeRef::Path if !p.optional => {
                    // Pass &Path when is_ref=true, otherwise pass owned PathBuf
                    if p.is_ref { format!("{rs}.as_path()") } else { rs }
                }
                TypeRef::String | TypeRef::Char if !p.optional => {
                    // Pass &str when is_ref=true, otherwise pass owned String
                    if p.is_ref { format!("&{rs}") } else { rs }
                }
                TypeRef::Bytes if !p.optional => {
                    // Pass &[u8] when is_ref=true (function takes &[u8]),
                    // otherwise pass owned Vec<u8>
                    if p.is_ref { format!("&{rs}") } else { rs }
                }
                TypeRef::Named(_) if !p.optional => {
                    // Pass by value when function takes owned (is_ref=false)
                    if p.is_ref { format!("&{rs}") } else { rs }
                }
                TypeRef::String | TypeRef::Char | TypeRef::Bytes if p.optional => {
                    // Only convert to &str slice when the core param is a reference (&str).
                    // When is_ref=false, the core takes Option<String> — pass owned.
                    if p.is_ref { format!("{rs}.as_deref()") } else { rs }
                }
                TypeRef::Path if p.optional => {
                    // Optional Path: rs is Option<String> (from param conversion)
                    // If is_ref=true, convert to Option<&Path>; else Option<PathBuf>
                    if p.is_ref {
                        format!("{rs}.as_ref().map(|s| std::path::Path::new(s.as_str()))")
                    } else {
                        format!("{rs}.map(std::path::PathBuf::from)")
                    }
                }
                TypeRef::Named(_) if p.optional => {
                    // Optional Named: rs is Option<T>
                    // If is_ref=true, convert to Option<&T>; else pass owned
                    if p.is_ref { format!("{rs}.as_ref()") } else { rs }
                }
                TypeRef::Json if !p.optional => {
                    // Json: rs is already serde_json::Value (from param conversion)
                    // If is_ref=true, pass &value; else pass owned
                    if p.is_ref { format!("&{rs}") } else { rs }
                }
                TypeRef::Json if p.optional => {
                    // Optional Json: rs is Option<Value>
                    // If is_ref=true, convert to Option<&Value>; else pass owned
                    if p.is_ref { format!("{rs}.as_ref()") } else { rs }
                }
                TypeRef::Vec(inner) if !p.optional => {
                    // Vec<String> with is_ref=true: core expects &[&str].
                    // Use the {rs}_refs intermediate (Vec<&str>) and pass as slice.
                    if p.is_ref && matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) {
                        format!("&{rs}_refs")
                    } else if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref {
                        format!("&{rs}")
                    } else {
                        rs
                    }
                }
                TypeRef::Map(_, _) if !p.optional => {
                    // When is_ref=true, pass &map. When is_mut=true, pass &mut map.
                    // Otherwise pass the map owned.
                    if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref {
                        format!("&{rs}")
                    } else {
                        rs
                    }
                }
                TypeRef::Vec(_) | TypeRef::Map(_, _) if p.optional => {
                    // Optional Vec/Map: rs is Option<Vec<T>> or Option<HashMap<K, V>>
                    // If is_ref=true, convert to Option<&[T]> with .as_deref()
                    // If is_mut=true, convert to Option<&mut Vec<T>> with .as_deref_mut()
                    // Otherwise pass owned Option
                    if p.is_mut {
                        format!("{rs}.as_deref_mut()")
                    } else if p.is_ref {
                        format!("{rs}.as_deref()")
                    } else {
                        rs
                    }
                }
                _ => rs,
            }
        })
        .collect();
    let call_args = arg_names.join(", ");

    if func.is_async {
        writeln!(
            out,
            "    let result = get_ffi_runtime().block_on(async {{ {core_fn_path}({call_args}).await }});"
        )
        .ok();
    } else {
        writeln!(out, "    let result = {core_fn_path}({call_args});").ok();
    }

    // Handle return
    // When return_newtype_wrapper is set, the core function returns a newtype but IR has the inner type.
    let result_expr = if func.return_newtype_wrapper.is_some() && matches!(func.return_type, TypeRef::Primitive(_)) {
        "result.0"
    } else {
        "result"
    };
    // When returns_ref=true and return type is Option<NamedType>, the core returns Option<&T>.
    // Clone to get owned Option<T> before boxing.
    if func.returns_ref
        && !has_error
        && matches!(&func.return_type, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)))
    {
        writeln!(out, "    let result = result.cloned();").ok();
    }
    // When returns_cow=true, the core returns Cow<'_, T> but FFI needs owned T.
    // Convert to owned by calling .into_owned().
    if func.returns_cow && !has_error {
        writeln!(out, "    let result = result.into_owned();").ok();
    }
    if has_error {
        writeln!(out, "    match result {{").ok();
        if is_void_return(&func.return_type) {
            writeln!(out, "        Ok(()) => 0,").ok();
        } else {
            writeln!(out, "        Ok(val) => {{").ok();
            let val_expr = if func.return_newtype_wrapper.is_some() && matches!(func.return_type, TypeRef::Primitive(_))
            {
                "val.0"
            } else {
                "val"
            };
            write!(
                out,
                "{}",
                gen_owned_value_to_c(val_expr, &func.return_type, "            ")
            )
            .ok();
            writeln!(out, "        }}").ok();
        }
        writeln!(out, "        Err(e) => {{").ok();
        writeln!(out, "            set_last_error(2, &e.to_string());").ok();
        if is_void_return(&func.return_type) {
            writeln!(out, "            -1").ok();
        } else {
            writeln!(out, "            {}", null_return_value(&func.return_type)).ok();
        }
        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
    } else if is_void_return(&func.return_type) {
        // nothing
    } else {
        write!(out, "{}", gen_owned_value_to_c(result_expr, &func.return_type, "    ")).ok();
    }

    write!(out, "}}").ok();
    out
}

// ---------------------------------------------------------------------------
// Type helpers
// ---------------------------------------------------------------------------

/// Returns a concrete Rust type string for a [`TypeRef`], used to build turbofish
/// annotations in `serde_json::from_str::<T>()` calls.
///
/// Using `_` in these positions causes type-inference failures when the deserialized
/// value is immediately coerced (e.g. `Vec<String>` converted to `Vec<&str>`).
/// Concrete types let the compiler resolve the full chain without ambiguity.
fn type_ref_to_rust_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Primitive(prim) => match prim {
            alef_core::ir::PrimitiveType::Bool => "bool".to_string(),
            alef_core::ir::PrimitiveType::U8 => "u8".to_string(),
            alef_core::ir::PrimitiveType::U16 => "u16".to_string(),
            alef_core::ir::PrimitiveType::U32 => "u32".to_string(),
            alef_core::ir::PrimitiveType::U64 => "u64".to_string(),
            alef_core::ir::PrimitiveType::I8 => "i8".to_string(),
            alef_core::ir::PrimitiveType::I16 => "i16".to_string(),
            alef_core::ir::PrimitiveType::I32 => "i32".to_string(),
            alef_core::ir::PrimitiveType::I64 => "i64".to_string(),
            alef_core::ir::PrimitiveType::F32 => "f32".to_string(),
            alef_core::ir::PrimitiveType::F64 => "f64".to_string(),
            alef_core::ir::PrimitiveType::Usize => "usize".to_string(),
            alef_core::ir::PrimitiveType::Isize => "isize".to_string(),
        },
        TypeRef::Named(name) => name.clone(),
        TypeRef::Vec(inner) => format!("Vec<{}>", type_ref_to_rust_type(inner)),
        TypeRef::Map(key, val) => format!(
            "std::collections::HashMap<{}, {}>",
            type_ref_to_rust_type(key),
            type_ref_to_rust_type(val)
        ),
        TypeRef::Optional(inner) => format!("Option<{}>", type_ref_to_rust_type(inner)),
        TypeRef::Path => "std::path::PathBuf".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Duration => "std::time::Duration".to_string(),
        TypeRef::Unit => "()".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Parameter conversion (C types -> Rust)
// ---------------------------------------------------------------------------

pub(super) fn gen_param_conversion(
    param: &ParamDef,
    has_error: bool,
    return_type: &TypeRef,
    _core_import: &str,
) -> String {
    let name = &param.name;
    let rs_name = format!("{name}_rs");
    let mut out = String::with_capacity(2048);

    let fail_ret = if has_error && is_void_return(return_type) {
        "return -1;"
    } else if is_void_return(return_type) {
        "return;"
    } else {
        // Use null_return_value to get the correct default for the return type
        // (handles primitives, floats, Optional, Duration, pointers)
        match null_return_value(return_type) {
            "()" => "return;",
            v => {
                // Leak: we need a 'static str but null_return_value returns &'static str
                // The values are all string literals so this is fine
                let ret = format!("return {};", v);
                // Use a leaked string since fail_ret needs 'static lifetime
                // This is called once per function generation, not in a hot loop
                Box::leak(ret.into_boxed_str()) as &str
            }
        }
    };

    if param.optional {
        // Optional parameter — null means None
        match &param.ty {
            TypeRef::String | TypeRef::Char => {
                writeln!(out, "    let {rs_name} = if {name}.is_null() {{").ok();
                writeln!(out, "        None").ok();
                writeln!(out, "    }} else {{").ok();
                writeln!(out, "        match unsafe {{ CStr::from_ptr({name}) }}.to_str() {{").ok();
                writeln!(out, "            Ok(s) => Some(s.to_string()),").ok();
                writeln!(out, "            Err(_) => {{").ok();
                writeln!(
                    out,
                    "                set_last_error(1, \"Invalid UTF-8 in parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "                {fail_ret}").ok();
                writeln!(out, "            }}").ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }};").ok();
            }
            TypeRef::Path => {
                writeln!(out, "    let {rs_name} = if {name}.is_null() {{").ok();
                writeln!(out, "        None").ok();
                writeln!(out, "    }} else {{").ok();
                writeln!(out, "        match unsafe {{ CStr::from_ptr({name}) }}.to_str() {{").ok();
                if param.is_ref {
                    // Option<&Path>: defer Path creation until use
                    writeln!(out, "            Ok(s) => Some(s.to_string()),").ok();
                } else {
                    writeln!(out, "            Ok(s) => Some(std::path::PathBuf::from(s)),").ok();
                }
                writeln!(out, "            Err(_) => {{").ok();
                writeln!(
                    out,
                    "                set_last_error(1, \"Invalid UTF-8 in parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "                {fail_ret}").ok();
                writeln!(out, "            }}").ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }};").ok();
            }
            TypeRef::Json => {
                writeln!(out, "    let {rs_name} = if {name}.is_null() {{").ok();
                writeln!(out, "        None").ok();
                writeln!(out, "    }} else {{").ok();
                writeln!(out, "        match unsafe {{ CStr::from_ptr({name}) }}.to_str() {{").ok();
                writeln!(out, "            Ok(s) => Some(s.to_string()),").ok();
                writeln!(out, "            Err(_) => {{").ok();
                writeln!(
                    out,
                    "                set_last_error(1, \"Invalid UTF-8 in parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "                {fail_ret}").ok();
                writeln!(out, "            }}").ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }};").ok();
            }
            TypeRef::Named(_type_name) => {
                writeln!(out, "    let {rs_name} = if {name}.is_null() {{").ok();
                writeln!(out, "        None").ok();
                writeln!(out, "    }} else {{").ok();
                if param.is_ref {
                    // Core function takes Option<&T> — pass a reference, no clone needed.
                    writeln!(out, "        Some(unsafe {{ &*{name} }})").ok();
                } else {
                    // Core function takes Option<T> — clone out of the pointer.
                    writeln!(out, "        Some(unsafe {{ &*{name} }}.clone())").ok();
                }
                writeln!(out, "    }};").ok();
            }
            TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool) => {
                // Optional bool: -1 = None, 0 = false, 1 = true
                writeln!(out, "    let {rs_name} = if {name} < 0 {{").ok();
                writeln!(out, "        None").ok();
                writeln!(out, "    }} else {{").ok();
                writeln!(out, "        Some({name} != 0)").ok();
                writeln!(out, "    }};").ok();
            }
            TypeRef::Primitive(prim) => {
                // Optional numeric primitive: max value of type = None
                let max_val = match prim {
                    alef_core::ir::PrimitiveType::U8 => "u8::MAX",
                    alef_core::ir::PrimitiveType::U16 => "u16::MAX",
                    alef_core::ir::PrimitiveType::U32 => "u32::MAX",
                    alef_core::ir::PrimitiveType::U64 => "u64::MAX",
                    alef_core::ir::PrimitiveType::I8 => "i8::MAX",
                    alef_core::ir::PrimitiveType::I16 => "i16::MAX",
                    alef_core::ir::PrimitiveType::I32 => "i32::MAX",
                    alef_core::ir::PrimitiveType::I64 => "i64::MAX",
                    alef_core::ir::PrimitiveType::F32 => "f32::NAN",
                    alef_core::ir::PrimitiveType::F64 => "f64::NAN",
                    alef_core::ir::PrimitiveType::Usize => "usize::MAX",
                    alef_core::ir::PrimitiveType::Isize => "isize::MAX",
                    alef_core::ir::PrimitiveType::Bool => unreachable!("handled above"),
                };
                let is_float = matches!(
                    prim,
                    alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64
                );
                if is_float {
                    writeln!(out, "    let {rs_name} = if {name}.is_nan() {{").ok();
                } else {
                    writeln!(out, "    let {rs_name} = if {name} == {max_val} {{").ok();
                }
                writeln!(out, "        None").ok();
                writeln!(out, "    }} else {{").ok();
                writeln!(out, "        Some({name})").ok();
                writeln!(out, "    }};").ok();
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                // Optional Vec/Map: deserialize from JSON string
                writeln!(out, "    let {rs_name} = if {name}.is_null() {{").ok();
                writeln!(out, "        None").ok();
                writeln!(out, "    }} else {{").ok();
                writeln!(out, "        match unsafe {{ CStr::from_ptr({name}) }}.to_str() {{").ok();
                writeln!(out, "            Ok(s) => {{").ok();
                let type_hint = match &param.ty {
                    TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        format!("::<{}>", type_ref_to_rust_type(&param.ty))
                    }
                    _ => String::new(),
                };
                writeln!(out, "                match serde_json::from_str{type_hint}(s) {{").ok();
                writeln!(out, "                    Ok(v) => Some(v),").ok();
                writeln!(out, "                    Err(e) => {{").ok();
                writeln!(
                    out,
                    "                        set_last_error(2, &format!(\"Invalid JSON in parameter '{{}}': {{}}\", \"{name}\", e));"
                )
                .ok();
                writeln!(out, "                        {fail_ret}").ok();
                writeln!(out, "                    }}").ok();
                writeln!(out, "                }}").ok();
                writeln!(out, "            }}").ok();
                writeln!(out, "            Err(_) => {{").ok();
                writeln!(
                    out,
                    "                set_last_error(1, \"Invalid UTF-8 in parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "                {fail_ret}").ok();
                writeln!(out, "            }}").ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }};").ok();
            }
            _ => {
                // Fallback: treat as nullable JSON string
                writeln!(out, "    let {rs_name} = if {name}.is_null() {{").ok();
                writeln!(out, "        None").ok();
                writeln!(out, "    }} else {{").ok();
                writeln!(out, "        match unsafe {{ CStr::from_ptr({name}) }}.to_str() {{").ok();
                writeln!(out, "            Ok(s) => Some(s.to_string()),").ok();
                writeln!(out, "            Err(_) => {{").ok();
                writeln!(
                    out,
                    "                set_last_error(1, \"Invalid UTF-8 in parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "                {fail_ret}").ok();
                writeln!(out, "            }}").ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }};").ok();
            }
        }
    } else {
        match &param.ty {
            TypeRef::String | TypeRef::Char => {
                writeln!(out, "    if {name}.is_null() {{").ok();
                writeln!(
                    out,
                    "        set_last_error(1, \"Null pointer passed for parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "        {fail_ret}").ok();
                writeln!(out, "    }}").ok();
                writeln!(
                    out,
                    "    let {rs_name} = match unsafe {{ CStr::from_ptr({name}) }}.to_str() {{"
                )
                .ok();
                writeln!(out, "        Ok(s) => s.to_string(),").ok();
                writeln!(out, "        Err(_) => {{").ok();
                writeln!(
                    out,
                    "            set_last_error(1, \"Invalid UTF-8 in parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "            {fail_ret}").ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }};").ok();
            }
            TypeRef::Path => {
                writeln!(out, "    if {name}.is_null() {{").ok();
                writeln!(
                    out,
                    "        set_last_error(1, \"Null pointer passed for parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "        {fail_ret}").ok();
                writeln!(out, "    }}").ok();
                writeln!(
                    out,
                    "    let {rs_name} = match unsafe {{ CStr::from_ptr({name}) }}.to_str() {{"
                )
                .ok();
                writeln!(out, "        Ok(s) => std::path::PathBuf::from(s),").ok();
                writeln!(out, "        Err(_) => {{").ok();
                writeln!(
                    out,
                    "            set_last_error(1, \"Invalid UTF-8 in parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "            {fail_ret}").ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }};").ok();
            }
            TypeRef::Json => {
                writeln!(out, "    if {name}.is_null() {{").ok();
                writeln!(
                    out,
                    "        set_last_error(1, \"Null pointer passed for parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "        {fail_ret}").ok();
                writeln!(out, "    }}").ok();
                writeln!(
                    out,
                    "    let {name}_str = match unsafe {{ CStr::from_ptr({name}) }}.to_str() {{"
                )
                .ok();
                writeln!(out, "        Ok(s) => s,").ok();
                writeln!(out, "        Err(_) => {{").ok();
                writeln!(
                    out,
                    "            set_last_error(1, \"Invalid UTF-8 in parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "            {fail_ret}").ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }};").ok();
                writeln!(out, "    let {rs_name} = match serde_json::from_str({name}_str) {{").ok();
                writeln!(out, "        Ok(v) => v,").ok();
                writeln!(out, "        Err(_) => {{").ok();
                writeln!(
                    out,
                    "            set_last_error(1, \"Invalid JSON in parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "            {fail_ret}").ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }};").ok();
            }
            TypeRef::Primitive(prim) => match prim {
                alef_core::ir::PrimitiveType::Bool => {
                    writeln!(out, "    let {rs_name} = {name} != 0;").ok();
                }
                _ => {
                    if let Some(newtype_path) = &param.newtype_wrapper {
                        // Param was resolved from a newtype (e.g. NodeIndex→u32): re-wrap for core call.
                        writeln!(out, "    let {rs_name} = {newtype_path}({name});").ok();
                    } else {
                        writeln!(out, "    let {rs_name} = {name};").ok();
                    }
                }
            },
            TypeRef::Named(_type_name) => {
                writeln!(out, "    if {name}.is_null() {{").ok();
                writeln!(
                    out,
                    "        set_last_error(1, \"Null pointer passed for parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "        {fail_ret}").ok();
                writeln!(out, "    }}").ok();
                if param.is_ref {
                    // Core function takes &T — pass a reference, no clone needed.
                    writeln!(out, "    let {rs_name} = unsafe {{ &*{name} }};").ok();
                } else {
                    // Core function takes owned T — clone out of the pointer.
                    writeln!(out, "    let {rs_name} = unsafe {{ &*{name} }}.clone();").ok();
                }
            }
            TypeRef::Bytes => {
                // Bytes come as (*const u8, len: usize) — the len param is a separate
                // parameter named {name}_len by convention.
                writeln!(out, "    if {name}.is_null() {{").ok();
                writeln!(
                    out,
                    "        set_last_error(1, \"Null pointer passed for parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "        {fail_ret}").ok();
                writeln!(out, "    }}").ok();
                writeln!(
                    out,
                    "    let {rs_name} = unsafe {{ std::slice::from_raw_parts({name}, {name}_len) }}.to_vec();"
                )
                .ok();
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                // Passed as JSON string
                writeln!(out, "    if {name}.is_null() {{").ok();
                writeln!(
                    out,
                    "        set_last_error(1, \"Null pointer passed for parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "        {fail_ret}").ok();
                writeln!(out, "    }}").ok();
                writeln!(
                    out,
                    "    let {rs_name}_str = match unsafe {{ CStr::from_ptr({name}) }}.to_str() {{"
                )
                .ok();
                writeln!(out, "        Ok(s) => s,").ok();
                writeln!(out, "        Err(_) => {{").ok();
                writeln!(
                    out,
                    "            set_last_error(1, \"Invalid UTF-8 in parameter '{name}'\");"
                )
                .ok();
                writeln!(out, "            {fail_ret}").ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }};").ok();
                // Add 'mut' if the parameter needs to be mutably borrowed.
                // Add explicit type annotation to avoid inference issues when the result is
                // only used through a reference (e.g. &mut vec -> Rust might infer [T] instead of Vec<T>).
                let mut_keyword = if param.is_mut { "mut " } else { "" };
                let type_hint = if param.is_ref || param.is_mut {
                    match &param.ty {
                        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                            format!("::<{}>", type_ref_to_rust_type(&param.ty))
                        }
                        _ => String::new(),
                    }
                } else {
                    String::new()
                };
                writeln!(
                    out,
                    "    let {mut_keyword}{rs_name} = match serde_json::from_str{type_hint}({rs_name}_str) {{"
                )
                .ok();
                writeln!(out, "        Ok(v) => v,").ok();
                writeln!(out, "        Err(e) => {{").ok();
                writeln!(out, "            set_last_error(2, &e.to_string());").ok();
                writeln!(out, "            {fail_ret}").ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }};").ok();
            }
            TypeRef::Optional(_) => {
                // Should not happen for non-optional param, but handle gracefully
                writeln!(out, "    let {rs_name} = {name};").ok();
            }
            TypeRef::Duration => {
                // Duration passed as u64 milliseconds
                writeln!(out, "    let {rs_name} = std::time::Duration::from_millis({name});").ok();
            }
            TypeRef::Unit => {
                // No parameter to convert
            }
        }
    }

    out
}
