use ahash::{AHashMap, AHashSet};
use alef_codegen::conversions::core_type_path;
use alef_core::ir::{FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use heck::ToSnakeCase;
use minijinja::context;
use std::fmt::Write;

/// Returns true when a sanitized function/method can be auto-recovered via JSON-roundtrip:
/// every sanitized param is a `Vec<String>` with `original_type` set (i.e. originally a
/// `Vec<tuple>`).  In that case the FFI param is a `*const c_char` JSON array and the
/// existing `Vec` conversion path produces `let items_rs = serde_json::from_str(...)?` whose
/// element type is inferred from the core call.
///
/// The function-level signature might also have been marked `sanitized` because of the return
/// type (e.g. `Option<&'static EmbeddingPreset>` → `Option<String>` after stdlib unification).
/// We can only recover when the return is itself representable in the FFI's existing return
/// machinery — i.e. simple types, owned Named, Optional<Named>, Vec, Map.  If the return
/// type was Named but got sanitized to String (because the type isn't in the API surface),
/// the FFI would marshal a struct as a string and miscompile, so we still emit a stub there.
fn sanitized_recoverable(func: &FunctionDef) -> bool {
    let params_ok = func.params.iter().all(|p| {
        if !p.sanitized {
            return true;
        }
        p.original_type.is_some() && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String))
    });
    if !params_ok {
        return false;
    }
    // Conservative: if the function was sanitized but no param was sanitized, the trigger was
    // the return type.  Recovering that requires JSON-serializing the actual core value, which
    // requires the core type to derive Serialize — alef has no way to know that here.  Stub.
    let any_param_sanitized = func.params.iter().any(|p| p.sanitized);
    !func.sanitized || any_param_sanitized
}

/// Method-level analogue of [`sanitized_recoverable`].
fn method_sanitized_recoverable(method: &MethodDef) -> bool {
    let params_ok = method.params.iter().all(|p| {
        if !p.sanitized {
            return true;
        }
        p.original_type.is_some() && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String))
    });
    if !params_ok {
        return false;
    }
    let any_param_sanitized = method.params.iter().any(|p| p.sanitized);
    !method.sanitized || any_param_sanitized
}

use crate::type_map::{c_param_type_with_paths, c_return_type_with_paths, is_passthrough_return, is_void_return};

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

    let doc_comment = if !method.doc.is_empty() {
        let lines: Vec<&str> = method.doc.lines().collect();
        crate::template_env::render("doc_comment_lines.jinja", minijinja::context! { doc_lines => lines })
    } else {
        String::new()
    };

    let body_indented = format!(" {}", body.replace('\n', "\n "));

    crate::template_env::render(
        "streaming_method_wrapper.jinja",
        minijinja::context! {
            doc_comment => doc_comment.trim_end(),
            fn_name => fn_name,
            qualified => qualified,
            body_indented => body_indented,
        },
    )
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
    enum_names: &AHashSet<String>,
) -> String {
    let type_snake = typ.name.to_snake_case();
    let type_name = &typ.name;
    let method_name = &method.name;
    let fn_name = format!("{prefix}_{type_snake}_{method_name}");

    // Generate doc comment
    let doc_comment = if !method.doc.is_empty() {
        let lines: Vec<&str> = method.doc.lines().collect();
        crate::template_env::render("doc_comment_lines.jinja", context! { doc_lines => lines })
    } else {
        String::new()
    };

    let has_error = method.error_type.is_some();

    // Detect Result<Vec<u8>> returns — these use the out-param convention instead of
    // a direct *mut u8 return, because the caller must also receive len and cap to
    // be able to call {prefix}_free_bytes later.
    let is_bytes_result = has_error && matches!(method.return_type, TypeRef::Bytes);

    // Count total FFI params: this + params + extra _len for Bytes params + 3 for bytes out-params
    let ffi_param_count = (if method.is_static { 0 } else { 1 })
        + method.params.len()
        + method.params.iter().filter(|p| matches!(p.ty, TypeRef::Bytes)).count()
        + if is_bytes_result { 3 } else { 0 };
    let allow_clippy = if ffi_param_count > 7 {
        Some("clippy::too_many_arguments".to_string())
    } else {
        None
    };

    let qualified = core_type_path(typ, core_import);

    // Return type
    let mut ret_type = if is_bytes_result {
        // Out-param convention — always returns i32 (0 = success, non-zero = error)
        "i32".to_string()
    } else if has_error && is_void_return(&method.return_type) {
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

    // Check if this method will be unimplemented before building params.
    // Sanitized methods with recoverable params (Vec<String> originally Vec<tuple>) are
    // re-routed through the standard JSON-roundtrip Vec conversion below.
    let will_be_unimplemented = method.sanitized && !method_sanitized_recoverable(method);

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
    // Result<Vec<u8>> returns use three out-params instead of a direct pointer return
    if is_bytes_result {
        let pfx = if will_be_unimplemented { "_" } else { "" };
        params.push(format!("    {pfx}out_ptr: *mut *mut u8"));
        params.push(format!("    {pfx}out_len: *mut usize"));
        params.push(format!("    {pfx}out_cap: *mut usize"));
    }

    let return_type = if is_void_return(&method.return_type) && !has_error {
        None
    } else {
        Some(ret_type.clone())
    };

    let header = crate::template_env::render(
        "method_wrapper_header.jinja",
        context! {
            doc_comment => doc_comment.trim_end(),
            allow_clippy => allow_clippy,
            fn_name => fn_name.clone(),
            params => params,
            return_type => return_type,
        },
    );

    let mut out = header;

    // If method signature was sanitized, generate unimplemented body
    if will_be_unimplemented {
        out.push_str(&gen_ffi_unimplemented_body(
            if is_bytes_result {
                &TypeRef::Unit
            } else {
                &method.return_type
            },
            &format!("{type_name}::{method_name}"),
            has_error || is_bytes_result,
        ));
        out.push_str("\n}");
        return out;
    }

    // Null-check the out-params for byte-buffer returns
    if is_bytes_result {
        out.push_str(&crate::template_env::render(
            "bytes_result_null_check.jinja",
            context! {},
        ));
    }

    // Null-check self
    if !method.is_static {
        let fail_ret = if is_bytes_result || (has_error && is_void_return(&method.return_type)) {
            "return -1;".to_string()
        } else if is_void_return(&method.return_type) {
            "return;".to_string()
        } else {
            format!("return {};", null_return_value(&method.return_type))
        };

        let null_check = match method.receiver.as_ref().unwrap_or(&ReceiverKind::Ref) {
            ReceiverKind::Ref => {
                crate::template_env::render("null_check_self_ref.jinja", context! { fail_ret => fail_ret })
            }
            ReceiverKind::RefMut => {
                crate::template_env::render("null_check_self_mut.jinja", context! { fail_ret => fail_ret })
            }
            ReceiverKind::Owned => {
                crate::template_env::render("null_check_self_owned.jinja", context! { fail_ret => fail_ret })
            }
        };
        out.push_str(&crate::template_env::render(
            "code_line.jinja",
            context! { content => null_check },
        ));
    }

    // Null-check and convert each parameter
    for p in &method.params {
        write!(
            out,
            "{}",
            gen_param_conversion(p, has_error, is_bytes_result, &method.return_type, core_import)
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
                    out.push_str(&crate::template_env::render(
                        "vec_string_refs.jinja",
                        context! { rs_name => rs.clone() },
                    ));
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
                    // Optional Path: rs is Option<String> when is_ref=true, Option<PathBuf> when is_ref=false (from param conversion)
                    // If is_ref=true, convert to Option<&Path>; else pass owned Option<PathBuf> directly
                    if p.is_ref {
                        format!("{rs}.as_ref().map(|s| std::path::Path::new(s.as_str()))")
                    } else {
                        rs
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

    // For passthrough returns (primitive non-Bool) without error/ref/cow/newtype,
    // emit the call as a tail expression directly to avoid `let_and_return`.
    let can_inline = is_passthrough_return(&method.return_type)
        && !has_error
        && !method.returns_ref
        && !method.returns_cow
        && method.return_newtype_wrapper.is_none();

    if method.is_async {
        let call = if method.is_static {
            format!("get_ffi_runtime().block_on(async {{ {qualified}::{method_name}({call_args}).await }})")
        } else {
            format!("get_ffi_runtime().block_on(async {{ obj.{method_name}({call_args}).await }})")
        };
        if can_inline {
            out.push_str(&crate::template_env::render(
                "call_inline.jinja",
                context! { call => call },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "call_with_result.jinja",
                context! { call => call },
            ));
        }
    } else if method.is_static {
        if can_inline {
            out.push_str(&crate::template_env::render("static_method_call.jinja", context! { qualified => qualified.clone(), method_name => method_name.clone(), call_args => call_args.clone() }));
        } else {
            out.push_str(&crate::template_env::render("static_method_call_result.jinja", context! { qualified => qualified.clone(), method_name => method_name.clone(), call_args => call_args.clone() }));
        }
    } else if method_name == "drop" {
        // Special case: Rust's drop method cannot be called directly with dot notation.
        // Use std::mem::drop instead.
        out.push_str("    std::mem::drop(obj);\n");
    } else if can_inline {
        out.push_str(&crate::template_env::render(
            "instance_method_call.jinja",
            context! { method_name => method_name.clone(), call_args => call_args.clone() },
        ));
    } else {
        out.push_str(&crate::template_env::render(
            "instance_method_call_result.jinja",
            context! { method_name => method_name.clone(), call_args => call_args.clone() },
        ));
    }

    // Handle return
    if is_bytes_result {
        // Result<Vec<u8>> — decompose the Vec and write to out-params.
        out.push_str(&crate::template_env::render("bytes_result_match.jinja", context! {}));
    } else {
        // When return_newtype_wrapper is set, the core function returns a newtype (e.g. NodeIndex)
        // but the IR has already resolved it to the inner type (e.g. u32). Unwrap with `.0`.
        let result_expr =
            if method.return_newtype_wrapper.is_some() && matches!(method.return_type, TypeRef::Primitive(_)) {
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
                // &str -> owned String. `.clone()` on &str is a no-op (str: !Sized
                // doesn't impl Clone) and triggers `noop_method_call`. Use to_owned.
                TypeRef::String => {
                    out.push_str("    let result = result.to_owned();\n");
                }
                TypeRef::Char => {
                    out.push_str("    let result = result.clone();\n");
                }
                TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                    out.push_str("    let result = result.clone();\n");
                }
                TypeRef::Named(_) => {
                    out.push_str("    let result = result.clone();\n");
                }
                TypeRef::Optional(inner) => match inner.as_ref() {
                    // Option<&str>::cloned() doesn't compile because `str: !Sized`. Use
                    // .map(str::to_owned) to convert Option<&str> -> Option<String>.
                    TypeRef::String => {
                        out.push_str("    let result = result.map(str::to_owned);\n");
                    }
                    TypeRef::Named(_) | TypeRef::Char | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        out.push_str("    let result = result.cloned();\n");
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        // When returns_cow=true, the core returns Cow<'_, T> but FFI needs owned T.
        // Convert to owned by calling .into_owned().
        if method.returns_cow && !has_error {
            out.push_str("    let result = result.into_owned();\n");
        }
        if has_error {
            if is_void_return(&method.return_type) {
                out.push_str(&crate::template_env::render("error_match_void.jinja", context! {}));
            } else {
                let val_expr =
                    if method.return_newtype_wrapper.is_some() && matches!(method.return_type, TypeRef::Primitive(_)) {
                        "val.0"
                    } else {
                        "val"
                    };
                let ok_body = gen_owned_value_to_c(val_expr, &method.return_type, "            ", enum_names);
                out.push_str(&crate::template_env::render(
                    "error_match_non_void.jinja",
                    context! {
                        ok_body => ok_body,
                        null_ret => null_return_value(&method.return_type),
                    },
                ));
            }
        } else if is_void_return(&method.return_type) {
            // void, no error — result is already ()
        } else if can_inline {
            // Passthrough primitive: call was already emitted as tail expression
        } else {
            write!(
                out,
                "{}",
                gen_owned_value_to_c(result_expr, &method.return_type, "    ", enum_names)
            )
            .ok();
        }
    }

    out.push_str("\n}");
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
    enum_names: &AHashSet<String>,
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

    // Generate doc comment
    let doc_comment = if !func.doc.is_empty() {
        let lines: Vec<&str> = func.doc.lines().collect();
        crate::template_env::render("doc_comment_lines.jinja", context! { doc_lines => lines })
    } else {
        String::new()
    };

    let has_error = func.error_type.is_some();

    // Detect Result<Vec<u8>> returns — these use the out-param convention instead of
    // a direct *mut u8 return, because the caller must also receive len and cap to
    // be able to call {prefix}_free_bytes later.
    let is_bytes_result = has_error && matches!(func.return_type, TypeRef::Bytes);

    // Count total FFI params: params + extra _len for Bytes params + 3 for bytes out-params
    let ffi_param_count = func.params.len()
        + func.params.iter().filter(|p| matches!(p.ty, TypeRef::Bytes)).count()
        + if is_bytes_result { 3 } else { 0 };
    let allow_clippy = if ffi_param_count > 7 {
        Some("clippy::too_many_arguments".to_string())
    } else {
        None
    };

    let ret_type = if is_bytes_result {
        // Out-param convention — always returns i32 (0 = success, non-zero = error)
        "i32".to_string()
    } else if has_error && is_void_return(&func.return_type) {
        "i32".to_string()
    } else {
        c_return_type_with_paths(&func.return_type, core_import, path_map).into_owned()
    };

    // Check if this function will be unimplemented before building params.
    // Sanitized funcs with recoverable params (Vec<String> originally Vec<tuple>) are
    // re-routed through the standard JSON-roundtrip Vec conversion below.
    let will_be_unimplemented = func.sanitized && !sanitized_recoverable(func);

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
    // Result<Vec<u8>> returns use three out-params instead of a direct pointer return
    if is_bytes_result {
        let pfx = if will_be_unimplemented { "_" } else { "" };
        params.push(format!("    {pfx}out_ptr: *mut *mut u8"));
        params.push(format!("    {pfx}out_len: *mut usize"));
        params.push(format!("    {pfx}out_cap: *mut usize"));
    }

    let return_type = if is_void_return(&func.return_type) && !has_error {
        None
    } else {
        Some(ret_type.clone())
    };

    let header = crate::template_env::render(
        "free_function_header.jinja",
        context! {
            doc_comment => doc_comment.trim_end(),
            allow_clippy => allow_clippy,
            fn_name => ffi_name.clone(),
            params => params,
            return_type => return_type,
        },
    );

    let mut out = header;

    // If function signature was sanitized or involves opaque types, generate unimplemented body
    if will_be_unimplemented {
        out.push_str(&gen_ffi_unimplemented_body(
            if is_bytes_result {
                &TypeRef::Unit
            } else {
                &func.return_type
            },
            func_name,
            has_error || is_bytes_result,
        ));
        out.push_str("\n}");
        return out;
    }

    // Null-check the out-params for byte-buffer returns
    if is_bytes_result {
        out.push_str(&crate::template_env::render(
            "bytes_result_null_check.jinja",
            context! {},
        ));
    }

    // Convert parameters
    for p in &func.params {
        write!(
            out,
            "{}",
            gen_param_conversion(p, has_error, is_bytes_result, &func.return_type, core_import)
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
                    out.push_str(&crate::template_env::render(
                        "vec_string_refs.jinja",
                        context! { rs_name => rs.clone() },
                    ));
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
                    // Optional Path: rs is Option<String> when is_ref=true, Option<PathBuf> when is_ref=false (from param conversion)
                    // If is_ref=true, convert to Option<&Path>; else pass owned Option<PathBuf> directly
                    if p.is_ref {
                        format!("{rs}.as_ref().map(|s| std::path::Path::new(s.as_str()))")
                    } else {
                        rs
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

    let can_inline_fn = is_passthrough_return(&func.return_type)
        && !has_error
        && !func.returns_ref
        && !func.returns_cow
        && func.return_newtype_wrapper.is_none();

    if func.is_async {
        let call = format!("get_ffi_runtime().block_on(async {{ {core_fn_path}({call_args}).await }})");
        if can_inline_fn {
            out.push_str(&crate::template_env::render(
                "call_inline.jinja",
                context! { call => call },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "call_with_result.jinja",
                context! { call => call },
            ));
        }
    } else if can_inline_fn {
        out.push_str(&crate::template_env::render(
            "call_inline.jinja",
            context! { call => format!("{core_fn_path}({call_args})") },
        ));
    } else {
        out.push_str(&crate::template_env::render(
            "call_with_result.jinja",
            context! { call => format!("{core_fn_path}({call_args})") },
        ));
    }

    // Handle return
    if is_bytes_result {
        // Result<Vec<u8>> — decompose the Vec and write to out-params.
        out.push_str(&crate::template_env::render("bytes_result_match.jinja", context! {}));
    } else {
        // When return_newtype_wrapper is set, the core function returns a newtype but IR has the inner type.
        let result_expr = if func.return_newtype_wrapper.is_some() && matches!(func.return_type, TypeRef::Primitive(_))
        {
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
            out.push_str("    let result = result.cloned();\n");
        }
        // When returns_cow=true, the core returns Cow<'_, T> but FFI needs owned T.
        // Convert to owned by calling .into_owned().
        if func.returns_cow && !has_error {
            out.push_str("    let result = result.into_owned();\n");
        }
        if has_error {
            if is_void_return(&func.return_type) {
                out.push_str(&crate::template_env::render("error_match_void.jinja", context! {}));
            } else {
                let val_expr =
                    if func.return_newtype_wrapper.is_some() && matches!(func.return_type, TypeRef::Primitive(_)) {
                        "val.0"
                    } else {
                        "val"
                    };
                let ok_body = gen_owned_value_to_c(val_expr, &func.return_type, "            ", enum_names);
                out.push_str(&crate::template_env::render(
                    "error_match_non_void.jinja",
                    context! {
                        ok_body => ok_body,
                        null_ret => null_return_value(&func.return_type),
                    },
                ));
            }
        } else if is_void_return(&func.return_type) {
            // nothing
        } else if can_inline_fn {
            // Passthrough primitive: call was already emitted as tail expression
        } else {
            write!(
                out,
                "{}",
                gen_owned_value_to_c(result_expr, &func.return_type, "    ", enum_names)
            )
            .ok();
        }
    }

    out.push_str("\n}");
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
fn type_ref_to_rust_type(ty: &TypeRef, core_import: &str) -> String {
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
        TypeRef::Named(name) => format!("{core_import}::{name}"),
        TypeRef::Vec(inner) => format!("Vec<{}>", type_ref_to_rust_type(inner, core_import)),
        TypeRef::Map(key, val) => format!(
            "std::collections::HashMap<{}, {}>",
            type_ref_to_rust_type(key, core_import),
            type_ref_to_rust_type(val, core_import)
        ),
        TypeRef::Optional(inner) => format!("Option<{}>", type_ref_to_rust_type(inner, core_import)),
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
    is_bytes_result: bool,
    return_type: &TypeRef,
    core_import: &str,
) -> String {
    let name = &param.name;
    let rs_name = format!("{name}_rs");
    let mut out = String::with_capacity(2048);

    let fail_ret = if is_bytes_result || (has_error && is_void_return(return_type)) {
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
                out.push_str(&crate::template_env::render(
                    "param_optional_string_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            TypeRef::Path => {
                out.push(' ');
                out.push_str(&crate::template_env::render(
                    "param_path_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        is_ref => param.is_ref,
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            TypeRef::Json => {
                out.push_str(&crate::template_env::render(
                    "param_optional_json_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        turbofish => String::new(),
                    },
                ));
            }
            TypeRef::Named(_type_name) => {
                out.push_str(&crate::template_env::render(
                    "param_optional_named_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        is_ref => param.is_ref,
                    },
                ));
            }
            TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool) => {
                out.push(' ');
                out.push_str(&crate::template_env::render(
                    "param_optional_bool_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                    },
                ));
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
                out.push(' ');
                out.push_str(&crate::template_env::render(
                    "param_optional_numeric_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        max_val => max_val,
                        is_float => is_float,
                    },
                ));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                // Optional Vec/Map: deserialize from JSON string
                let type_hint = match &param.ty {
                    TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        format!("::<{}>", type_ref_to_rust_type(&param.ty, core_import))
                    }
                    _ => String::new(),
                };
                out.push(' ');
                out.push_str(&crate::template_env::render(
                    "param_optional_vec_map_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        turbofish => type_hint,
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            _ => {
                // Fallback: treat as nullable JSON string
                out.push_str(&crate::template_env::render(
                    "param_optional_fallback.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
        }
    } else {
        match &param.ty {
            TypeRef::String | TypeRef::Char => {
                out.push_str(&crate::template_env::render(
                    "param_non_optional_string_conversion.jinja",
                    context! {
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        rs_name => rs_name.clone(),
                    },
                ));
            }
            TypeRef::Path => {
                out.push_str(&crate::template_env::render(
                    "param_non_optional_path_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            TypeRef::Json => {
                let turbofish = String::new();
                let mut_keyword = String::new();
                out.push_str(&crate::template_env::render(
                    "param_non_optional_json_conversion.jinja",
                    context! {
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        rs_name => rs_name.clone(),
                        turbofish => turbofish,
                        mut_keyword => mut_keyword,
                    },
                ));
            }
            TypeRef::Primitive(prim) => match prim {
                alef_core::ir::PrimitiveType::Bool => {
                    out.push_str(&crate::template_env::render(
                        "param_primitive_bool.jinja",
                        context! { rs_name => rs_name.clone(), name => name.clone() },
                    ));
                }
                _ => {
                    if let Some(newtype_path) = &param.newtype_wrapper {
                        // Param was resolved from a newtype (e.g. NodeIndex→u32): re-wrap for core call.
                        out.push_str(&crate::template_env::render("param_primitive_newtype.jinja", context! { rs_name => rs_name.clone(), newtype_path => newtype_path.clone(), name => name.clone() }));
                    } else {
                        out.push_str(&crate::template_env::render(
                            "param_primitive_passthrough.jinja",
                            context! { rs_name => rs_name.clone(), name => name.clone() },
                        ));
                    }
                }
            },
            TypeRef::Named(_type_name) => {
                out.push_str(&crate::template_env::render(
                    "param_non_optional_named_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        is_ref => param.is_ref,
                    },
                ));
            }
            TypeRef::Bytes => {
                // Bytes come as (*const u8, len: usize) — the len param is a separate
                // parameter named {name}_len by convention. A null pointer is allowed
                // when the corresponding length is zero (empty input is a legitimate
                // case — e.g. extracting from a 0-byte file). Reject null only when
                // the caller claims a non-zero length.
                out.push_str(&crate::template_env::render(
                    "param_non_optional_bytes_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                // Passed as JSON string
                let mut_keyword = if param.is_mut { "mut " } else { "" };
                let type_hint = match &param.ty {
                    TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        format!("::<{}>", type_ref_to_rust_type(&param.ty, core_import))
                    }
                    _ => String::new(),
                };
                out.push_str(&crate::template_env::render(
                    "param_non_optional_json_conversion.jinja",
                    context! {
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        rs_name => rs_name.clone(),
                        turbofish => type_hint,
                        mut_keyword => mut_keyword,
                    },
                ));
            }
            TypeRef::Optional(_) => {
                // Should not happen for non-optional param, but handle gracefully
                out.push_str(&crate::template_env::render(
                    "param_optional_passthrough.jinja",
                    context! { rs_name => rs_name.clone(), name => name.clone() },
                ));
            }
            TypeRef::Duration => {
                // Duration passed as u64 milliseconds
                out.push_str(&crate::template_env::render(
                    "param_duration_conversion.jinja",
                    context! { rs_name => rs_name.clone(), name => name.clone() },
                ));
            }
            TypeRef::Unit => {
                // No parameter to convert
            }
        }
    }

    out
}
