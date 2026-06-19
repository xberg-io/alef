use crate::backends::ffi::type_map::{c_return_type_with_paths, is_passthrough_return, is_void_return};
use crate::codegen::conversions::core_type_path;
use crate::core::ir::{CoreWrapper, FunctionDef, MethodDef, ReceiverKind, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use minijinja::context;

use super::super::helpers::{gen_ffi_unimplemented_body, gen_owned_value_to_c, null_return_value};
use super::params::gen_param_conversion_with_enums;
use super::return_handling::{gen_owned_c_char_to_c_with_len, return_type_needs_non_serde_named, returns_c_char};
use super::signatures::{c_symbol_component, internal_class_component};
use super::support::{ffi_doxygen_block, method_sanitized_recoverable, sanitized_recoverable};

pub(in crate::backends::ffi::gen_bindings) fn gen_streaming_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    prefix: &str,
    core_import: &str,
    body: &str,
) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let method_name = &method.name;
    let fn_name = format!("{prefix}_{type_snake}_{method_name}");
    let qualified = core_type_path(typ, core_import);
    let callback_type = format!("{}StreamCallback", internal_class_component(prefix));

    let doc_comment = ffi_doxygen_block(&method.doc);

    let body_indented = format!(" {}", body.replace('\n', "\n "));

    crate::backends::ffi::template_env::render(
        "streaming_method_wrapper.jinja",
        minijinja::context! {
            doc_comment => doc_comment.trim_end(),
            fn_name => fn_name,
            qualified => qualified,
            callback_type => callback_type,
            body_indented => body_indented,
        },
    )
}

// ---------------------------------------------------------------------------
// Method wrappers
// ---------------------------------------------------------------------------

pub(in crate::backends::ffi::gen_bindings) fn gen_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    prefix: &str,
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
    serde_names: &AHashSet<String>,
) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let method_name = &method.name;
    let fn_name = format!("{prefix}_{type_snake}_{method_name}");

    // Generate doc comment
    let doc_comment = ffi_doxygen_block(&method.doc);

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

    // Types with lifetime parameters (e.g. NodeContext<'a>) require an explicit `<'static>`
    // lifetime in return-position `*mut T` / `*const T` pointers. Append it when the return
    // type names the enclosing type and that type has lifetime params.
    if typ.has_lifetime_params {
        if let TypeRef::Named(n) = &method.return_type {
            if n == type_name {
                let bare = format!("*mut {qualified}");
                if ret_type == bare {
                    ret_type = format!("*mut {qualified}<'static>");
                }
            }
        }
    }

    // Check if this method will be unimplemented before building params.
    // Sanitized methods with recoverable params (Vec<String> originally Vec<tuple>) are
    // re-routed through the standard JSON-roundtrip Vec conversion below.
    // Also stub out methods returning Vec<Named> / Map where Named lacks serde::Serialize.
    let return_needs_non_serde_named_method = return_type_needs_non_serde_named(&method.return_type, serde_names);
    let will_be_unimplemented =
        (method.sanitized && !method_sanitized_recoverable(method)) || return_needs_non_serde_named_method;

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
            crate::backends::ffi::type_map::c_param_type_with_paths_and_enums(
                &p.ty,
                core_import,
                path_map,
                enum_names,
                p.is_mut,
            )
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

    let header = crate::backends::ffi::template_env::render(
        "method_wrapper_header.jinja",
        context! {
            doc_comment => doc_comment.trim_end(),
            allow_clippy => allow_clippy,
            fn_name => fn_name.clone(),
            params => params,
            return_type => return_type,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
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
        out.push_str(&crate::backends::ffi::template_env::render(
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
            ReceiverKind::Ref => crate::backends::ffi::template_env::render(
                "null_check_self_ref.jinja",
                context! { fail_ret => fail_ret },
            ),
            ReceiverKind::RefMut => crate::backends::ffi::template_env::render(
                "null_check_self_mut.jinja",
                context! { fail_ret => fail_ret },
            ),
            ReceiverKind::Owned => crate::backends::ffi::template_env::render(
                "null_check_self_owned.jinja",
                context! { fail_ret => fail_ret },
            ),
        };
        out.push_str(&crate::backends::ffi::template_env::render(
            "code_line.jinja",
            context! { content => null_check },
        ));
    }

    // Null-check and convert each parameter
    for p in &method.params {
        out.push_str(&crate::backends::ffi::template_env::render(
            "emitted_code_block.jinja",
            context! {
                content => gen_param_conversion_with_enums(p, has_error, is_bytes_result, &method.return_type, core_import, enum_names),
            },
        ));
    }

    // For is_ref BTreeMap params, emit a named let binding so the temporary BTreeMap is not
    // dropped before the function call. An inline `&collect(...)` would produce a reference to a
    // temporary that is dropped at end-of-statement — rejected when the callee returns a
    // lifetime-parameterized type (e.g. NodeContext<'a>) that borrows from the map.
    for p in &method.params {
        if matches!(p.ty, TypeRef::Map(_, _)) && !p.optional && p.is_ref && p.map_is_btree {
            let rs = format!("{}_rs", p.name);
            let btree = format!("{}_btree", p.name);
            out.push_str(&crate::backends::ffi::template_env::render(
                "ffi_btree_binding.jinja",
                context! {
                    btree => btree,
                    rs => rs,
                },
            ));
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
                    // When is_mut=true, the local rs is already `&mut T` (bound via
                    // `let rs = unsafe { &mut *ptr }`). Pass it directly — adding
                    // `&mut` would produce `&mut &mut T` (E0308).
                    if p.is_mut || is_owned_receiver || !p.is_ref {
                        rs
                    } else {
                        format!("&{rs}")
                    }
                }
                TypeRef::String | TypeRef::Char if !p.optional => {
                    // Pass &str when is_ref=true, otherwise pass owned String.
                    // When core_wrapper=Cow, the core function expects `Cow<'_, str>`:
                    // String implements Into<Cow<str>>, so `.into()` performs the coercion.
                    if p.is_ref {
                        format!("&{rs}")
                    } else if p.core_wrapper == CoreWrapper::Cow {
                        format!("{rs}.into()")
                    } else {
                        rs
                    }
                }
                TypeRef::Bytes if !p.optional => {
                    // Pass &[u8] when is_ref=true (function takes &[u8]),
                    // otherwise pass owned Vec<u8>
                    if p.is_ref { format!("&{rs}") } else { rs }
                }
                TypeRef::String | TypeRef::Char | TypeRef::Bytes if p.optional => {
                    // Only convert to &str slice when the core param is a reference (&str).
                    // When is_ref=false and core_wrapper=Cow, the core takes Option<Cow<str>>:
                    // convert via `.map(std::borrow::Cow::Owned)`.
                    // Otherwise when is_ref=false, the core takes Option<String> — pass owned.
                    if p.is_ref {
                        format!("{rs}.as_deref()")
                    } else if p.core_wrapper == CoreWrapper::Cow {
                        format!("{rs}.map(std::borrow::Cow::Owned)")
                    } else {
                        rs
                    }
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
                TypeRef::Vec(_inner) if !p.optional => {
                    // When is_ref=true, pass &rs — &Vec<T> coerces to &[T].
                    // However, when vec_inner_is_ref=true (e.g. &[&str] params),
                    // &Vec<String> does NOT coerce to &[&str]. Build a temporary Vec<&str>
                    // and pass &_refs instead.
                    if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref && p.vec_inner_is_ref {
                        // Source: &[&T] (or Vec<&T>). The local `rs` is `Vec<T_owned>`
                        // after JSON deserialization. Materialize a temporary `Vec<&T>`
                        // inline so Rust extends the temporary to the enclosing
                        // statement; the call site then receives `&[&T]`. A `let`
                        // binding inside a block would drop the Vec before the call.
                        format!("&{rs}.iter().map(|s| s.as_str()).collect::<Vec<&str>>()")
                    } else if p.is_ref {
                        format!("&{rs}")
                    } else {
                        rs
                    }
                }
                TypeRef::Map(_, _) if !p.optional => {
                    // When is_ref=true, pass &map. When is_mut=true, pass &mut map.
                    // Otherwise pass the map owned.
                    // When map_is_btree=true with is_ref=true, a named let binding was emitted
                    // above (`let {name}_btree = ...`) so reference it here instead of using
                    // an inline &collect() temporary (which would be dropped before the call
                    // when the callee returns a lifetime-parameterized type).
                    if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref && p.map_is_btree {
                        format!("&{}_btree", p.name)
                    } else if p.is_ref {
                        format!("&{rs}")
                    } else if p.map_is_btree {
                        format!("{rs}.into_iter().collect::<std::collections::BTreeMap<_, _>>()")
                    } else {
                        rs
                    }
                }
                TypeRef::Vec(_) if p.optional => {
                    // Optional Vec: rs is Option<Vec<T>>.
                    // Vec<T>: Deref<Target=[T]>, so .as_deref() gives Option<&[T]>.
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
                TypeRef::Map(_, _) if p.optional => {
                    // Optional Map: rs is Option<HashMap<K, V>> (or AHashMap if map_is_ahash).
                    // HashMap/AHashMap does NOT implement Deref, so .as_deref() would fail.
                    // Use .as_ref() to get Option<&Map<K, V>>.
                    if p.is_mut {
                        format!("{rs}.as_deref_mut()")
                    } else if p.is_ref {
                        format!("{rs}.as_ref()")
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
            out.push_str(&crate::backends::ffi::template_env::render(
                "call_inline.jinja",
                context! { call => call },
            ));
        } else {
            out.push_str(&crate::backends::ffi::template_env::render(
                "call_with_result.jinja",
                context! { call => call },
            ));
        }
    } else if method.is_static {
        if can_inline {
            out.push_str(&crate::backends::ffi::template_env::render("static_method_call.jinja", context! { qualified => qualified.clone(), method_name => method_name.clone(), call_args => call_args.clone() }));
        } else {
            out.push_str(&crate::backends::ffi::template_env::render("static_method_call_result.jinja", context! { qualified => qualified.clone(), method_name => method_name.clone(), call_args => call_args.clone() }));
        }
    } else if method_name == "drop" {
        // Special case: Rust's drop method cannot be called directly with dot notation.
        // Use std::mem::drop instead.
        out.push_str("    std::mem::drop(obj);\n");
    } else if can_inline {
        out.push_str(&crate::backends::ffi::template_env::render(
            "instance_method_call.jinja",
            context! { method_name => method_name.clone(), call_args => call_args.clone() },
        ));
    } else {
        out.push_str(&crate::backends::ffi::template_env::render(
            "instance_method_call_result.jinja",
            context! { method_name => method_name.clone(), call_args => call_args.clone() },
        ));
    }

    // Handle return
    if is_bytes_result {
        // Result<Vec<u8>> — decompose the Vec and write to out-params.
        out.push_str(&crate::backends::ffi::template_env::render(
            "bytes_result_match.jinja",
            context! {},
        ));
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
                    // `char: Copy` — `.clone()` on `&char` triggers clippy::noop_method_call.
                    out.push_str("    let result = *result;\n");
                }
                TypeRef::Vec(_) => {
                    // Return type may be `&[T]` (slice) — `.clone()` on a slice is a noop
                    // because `[T]: !Sized`. Use `.to_vec()` to produce an owned Vec.
                    out.push_str("    let result = result.to_vec();\n");
                }
                TypeRef::Map(_, _) => {
                    // Return type is `&BTreeMap<K, V>` — `.to_vec()` does not exist on maps.
                    // Use `.clone()` to get an owned `BTreeMap`.
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
                out.push_str(&crate::backends::ffi::template_env::render(
                    "error_match_void.jinja",
                    context! {},
                ));
            } else {
                let val_expr =
                    if method.return_newtype_wrapper.is_some() && matches!(method.return_type, TypeRef::Primitive(_)) {
                        "val.0"
                    } else {
                        "val"
                    };
                let ok_body = gen_owned_value_to_c(val_expr, &method.return_type, "            ", enum_names);
                out.push_str(&crate::backends::ffi::template_env::render(
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
            out.push_str(&crate::backends::ffi::template_env::render(
                "emitted_code_block.jinja",
                context! {
                    content => gen_owned_value_to_c(result_expr, &method.return_type, "    ", enum_names),
                },
            ));
        }
    }

    out.push_str("\n}");
    out
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(in crate::backends::ffi::gen_bindings) fn gen_free_function(
    func: &FunctionDef,
    prefix: &str,
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
    serde_names: &AHashSet<String>,
    capsule_cfg: Option<&crate::core::config::FfiCapsuleTypeConfig>,
) -> String {
    let fn_name_snake = c_symbol_component(&func.name);
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
    let doc_comment = ffi_doxygen_block(&func.doc);

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
    } else if let Some(cfg) = capsule_cfg {
        // Capsule passthrough: return the host runtime's grammar pointer directly
        // (e.g. `*const tree_sitter::ffi::TSLanguage`) instead of boxing an opaque handle.
        super::super::capsule::capsule_c_return_type(cfg)
    } else {
        c_return_type_with_paths(&func.return_type, core_import, path_map).into_owned()
    };

    // Check if this function will be unimplemented before building params.
    // Sanitized funcs with recoverable params (Vec<String> originally Vec<tuple>) are
    // re-routed through the standard JSON-roundtrip Vec conversion below.
    // Additionally, functions returning Vec<Named> or Map where the Named type does not
    // derive serde::Serialize cannot be JSON-serialized and must be stubbed.
    let return_needs_non_serde_named = return_type_needs_non_serde_named(&func.return_type, serde_names);
    let will_be_unimplemented = (func.sanitized && !sanitized_recoverable(func)) || return_needs_non_serde_named;

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
            crate::backends::ffi::type_map::c_param_type_with_paths_and_enums(
                &p.ty,
                core_import,
                path_map,
                enum_names,
                p.is_mut,
            )
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

    let header = crate::backends::ffi::template_env::render(
        "free_function_header.jinja",
        context! {
            doc_comment => doc_comment.trim_end(),
            allow_clippy => allow_clippy,
            fn_name => ffi_name.clone(),
            params => params,
            return_type => return_type,
            source_cfg => func.cfg.as_deref().unwrap_or(""),
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
        out.push_str(&crate::backends::ffi::template_env::render(
            "bytes_result_null_check.jinja",
            context! {},
        ));
    }

    // Convert parameters
    for p in &func.params {
        out.push_str(&crate::backends::ffi::template_env::render(
            "emitted_code_block.jinja",
            context! {
                content => gen_param_conversion_with_enums(p, has_error, is_bytes_result, &func.return_type, core_import, enum_names),
            },
        ));
    }

    // For is_ref BTreeMap params, emit a named let binding so the temporary BTreeMap is not
    // dropped before the function call. An inline `&collect(...)` would produce a reference to a
    // temporary that is dropped at end-of-statement — rejected when the callee returns a
    // lifetime-parameterized type (e.g. NodeContext<'a>) that borrows from the map.
    for p in &func.params {
        if matches!(p.ty, TypeRef::Map(_, _)) && !p.optional && p.is_ref && p.map_is_btree {
            let rs = format!("{}_rs", p.name);
            let btree = format!("{}_btree", p.name);
            out.push_str(&crate::backends::ffi::template_env::render(
                "ffi_btree_binding.jinja",
                context! {
                    btree => btree,
                    rs => rs,
                },
            ));
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
                    // Pass &str when is_ref=true, otherwise pass owned String.
                    // When core_wrapper=Cow, the core function expects `Cow<'_, str>`:
                    // String implements Into<Cow<str>>, so `.into()` performs the coercion.
                    if p.is_ref {
                        format!("&{rs}")
                    } else if p.core_wrapper == CoreWrapper::Cow {
                        format!("{rs}.into()")
                    } else {
                        rs
                    }
                }
                TypeRef::Bytes if !p.optional => {
                    // Pass &[u8] when is_ref=true (function takes &[u8]),
                    // otherwise pass owned Vec<u8>
                    if p.is_ref { format!("&{rs}") } else { rs }
                }
                TypeRef::Named(_) if !p.optional => {
                    // When is_mut=true, the local rs is already `&mut T` (bound via
                    // `let rs = unsafe { &mut *ptr }`). Pass it directly — adding
                    // `&mut` would produce `&mut &mut T` (E0308).
                    if p.is_mut || !p.is_ref { rs } else { format!("&{rs}") }
                }
                TypeRef::String | TypeRef::Char | TypeRef::Bytes if p.optional => {
                    // Only convert to &str slice when the core param is a reference (&str).
                    // When is_ref=false and core_wrapper=Cow, the core takes Option<Cow<str>>:
                    // convert via `.map(std::borrow::Cow::Owned)`.
                    // Otherwise when is_ref=false, the core takes Option<String> — pass owned.
                    if p.is_ref {
                        format!("{rs}.as_deref()")
                    } else if p.core_wrapper == CoreWrapper::Cow {
                        format!("{rs}.map(std::borrow::Cow::Owned)")
                    } else {
                        rs
                    }
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
                TypeRef::Vec(_inner) if !p.optional => {
                    // When is_ref=true, pass &rs — &Vec<T> coerces to &[T].
                    // However, when vec_inner_is_ref=true (e.g. &[&str] params),
                    // &Vec<String> does NOT coerce to &[&str]. Build a temporary Vec<&str>
                    // and pass &_refs instead.
                    if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref && p.vec_inner_is_ref {
                        // Source: &[&T] (or Vec<&T>). The local `rs` is `Vec<T_owned>`
                        // after JSON deserialization. Materialize a temporary `Vec<&T>`
                        // inline so Rust extends the temporary to the enclosing
                        // statement; the call site then receives `&[&T]`. A `let`
                        // binding inside a block would drop the Vec before the call.
                        format!("&{rs}.iter().map(|s| s.as_str()).collect::<Vec<&str>>()")
                    } else if p.is_ref {
                        format!("&{rs}")
                    } else {
                        rs
                    }
                }
                TypeRef::Map(_, _) if !p.optional => {
                    // When is_ref=true, pass &map. When is_mut=true, pass &mut map.
                    // Otherwise pass the map owned.
                    // When map_is_btree=true with is_ref=true, a named let binding was emitted
                    // above (`let {name}_btree = ...`) so reference it here instead of using
                    // an inline &collect() temporary (which would be dropped before the call
                    // when the callee returns a lifetime-parameterized type).
                    if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref && p.map_is_btree {
                        format!("&{}_btree", p.name)
                    } else if p.is_ref {
                        format!("&{rs}")
                    } else if p.map_is_btree {
                        format!("{rs}.into_iter().collect::<std::collections::BTreeMap<_, _>>()")
                    } else {
                        rs
                    }
                }
                TypeRef::Vec(_) if p.optional => {
                    // Optional Vec: rs is Option<Vec<T>>.
                    // Vec<T>: Deref<Target=[T]>, so .as_deref() gives Option<&[T]>.
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
                TypeRef::Map(_, _) if p.optional => {
                    // Optional Map: rs is Option<HashMap<K, V>> (or AHashMap if map_is_ahash).
                    // HashMap/AHashMap does NOT implement Deref, so .as_deref() would fail.
                    // Use .as_ref() to get Option<&Map<K, V>>.
                    if p.is_mut {
                        format!("{rs}.as_deref_mut()")
                    } else if p.is_ref {
                        format!("{rs}.as_ref()")
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
            out.push_str(&crate::backends::ffi::template_env::render(
                "call_inline.jinja",
                context! { call => call },
            ));
        } else {
            out.push_str(&crate::backends::ffi::template_env::render(
                "call_with_result.jinja",
                context! { call => call },
            ));
        }
    } else if can_inline_fn {
        out.push_str(&crate::backends::ffi::template_env::render(
            "call_inline.jinja",
            context! { call => format!("{core_fn_path}({call_args})") },
        ));
    } else {
        out.push_str(&crate::backends::ffi::template_env::render(
            "call_with_result.jinja",
            context! { call => format!("{core_fn_path}({call_args})") },
        ));
    }

    // Handle return
    if is_bytes_result {
        // Result<Vec<u8>> — decompose the Vec and write to out-params.
        out.push_str(&crate::backends::ffi::template_env::render(
            "bytes_result_match.jinja",
            context! {},
        ));
    } else {
        let result_expr = if func.return_newtype_wrapper.is_some() && matches!(func.return_type, TypeRef::Primitive(_))
        {
            "result.0"
        } else {
            "result"
        };
        if func.returns_ref
            && !has_error
            && matches!(&func.return_type, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)))
        {
            out.push_str("    let result = result.cloned();\n");
        }
        if func.returns_cow && !has_error {
            out.push_str("    let result = result.into_owned();\n");
        }
        if has_error {
            if is_void_return(&func.return_type) {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "error_match_void.jinja",
                    context! {},
                ));
            } else {
                let val_expr =
                    if func.return_newtype_wrapper.is_some() && matches!(func.return_type, TypeRef::Primitive(_)) {
                        "val.0"
                    } else {
                        "val"
                    };
                let ok_body = if let Some(cfg) = capsule_cfg {
                    format!(
                        "            {}",
                        super::super::capsule::capsule_into_raw_expr(val_expr, cfg)
                    )
                } else if returns_c_char(&func.return_type) {
                    gen_owned_c_char_to_c_with_len(val_expr, &func.return_type, "            ")
                } else {
                    gen_owned_value_to_c(val_expr, &func.return_type, "            ", enum_names)
                };
                let null_ret = if capsule_cfg.is_some() {
                    "std::ptr::null()"
                } else {
                    null_return_value(&func.return_type)
                };
                out.push_str(&crate::backends::ffi::template_env::render(
                    "error_match_non_void.jinja",
                    context! {
                        ok_body => ok_body,
                        null_ret => null_ret,
                    },
                ));
            }
        } else if is_void_return(&func.return_type) {
            // nothing
        } else if can_inline_fn {
            // Passthrough primitive: call was already emitted as tail expression
        } else {
            let content = if let Some(cfg) = capsule_cfg {
                format!("    {}", super::super::capsule::capsule_into_raw_expr(result_expr, cfg))
            } else if returns_c_char(&func.return_type) {
                gen_owned_c_char_to_c_with_len(result_expr, &func.return_type, "    ")
            } else {
                gen_owned_value_to_c(result_expr, &func.return_type, "    ", enum_names)
            };
            out.push_str(&crate::backends::ffi::template_env::render(
                "emitted_code_block.jinja",
                context! {
                    content => content,
                },
            ));
        }
    }

    out.push_str("\n}");
    out
}
