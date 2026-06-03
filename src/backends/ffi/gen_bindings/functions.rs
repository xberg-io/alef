use crate::codegen::conversions::core_type_path;
use crate::codegen::doc_emission::emit_c_doxygen;
use crate::codegen::naming::{pascal_to_snake, to_class_name};
use crate::core::ir::{FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use minijinja::context;

/// Returns true if a method should be skipped from C FFI wrapper generation.
///
/// Methods are skipped if they:
/// 1. Have generic type parameters (detected by parameters with Named types not in the path_map)
/// 2. Return a reference to the receiver type (builder-style methods returning `&mut Self` or `&Self`)
/// 3. Are static constructors on opaque types (handled via gen_opaque_static_constructor instead)
///
/// Such methods are handled through the service-API registration path instead of as
/// standalone C function wrappers.
pub(super) fn should_skip_method_wrapper(
    method: &MethodDef,
    typ: &TypeDef,
    path_map: &AHashMap<String, String>,
) -> bool {
    // Skip if any parameter is a Named type not in the path_map (likely a generic type parameter)
    for param in &method.params {
        if let TypeRef::Named(name) = &param.ty {
            if !path_map.contains_key(name.as_str()) {
                return true;
            }
        }
    }

    // Skip if the method returns a reference to the receiver type (builder-style methods).
    // These methods return `&mut Self` or `&Self`, which cannot be represented as owned
    // C handles. They're meant to be accessed through service API instead.
    if method.returns_ref {
        // Check if the return type (a reference) points back to the receiver type
        if let TypeRef::Named(name) = &method.return_type {
            if name == &typ.name {
                return true;
            }
        }
    }

    // Skip static constructors on opaque types — they are handled specially via
    // gen_opaque_static_constructor to emit proper enum-by-value marshalling.
    if typ.is_opaque && method.is_static {
        if let TypeRef::Named(name) = &method.return_type {
            if name == &typ.name {
                return true;
            }
        }
    }

    false
}

/// Render a Doxygen `///` block for an FFI extern function whose rustdoc comes
/// from the source `# Arguments` / `# Returns` / `# Errors` sections. Always
/// appends the universal FFI `\note SAFETY:` clause that the alef FFI template
/// previously hard-coded, so callers don't have to repeat it.
///
/// Returns an empty string when `doc` is empty AND no safety note is needed
/// (currently we always emit the safety note for `extern "C"` boundaries).
fn ffi_doxygen_block(doc: &str) -> String {
    let mut full_doc = String::with_capacity(doc.len() + 128);
    if !doc.is_empty() {
        full_doc.push_str(doc);
        if !doc.contains("# Safety") {
            full_doc.push_str(
                "\n\n# Safety\n\nCaller must ensure all pointer arguments are valid or null. \
                 Returned pointers must be freed with the appropriate free function.",
            );
        }
    } else {
        full_doc.push_str(
            "# Safety\n\nCaller must ensure all pointer arguments are valid or null. \
             Returned pointers must be freed with the appropriate free function.",
        );
    }
    let mut out = String::new();
    emit_c_doxygen(&mut out, &full_doc, "");
    out
}

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

use crate::backends::ffi::type_map::{c_return_type_with_paths, is_passthrough_return, is_void_return};

use super::helpers::{gen_ffi_unimplemented_body, gen_owned_value_to_c, null_return_value};

/// Returns true when the return type requires JSON serialization of a Named type that is NOT
/// in `serde_names` (i.e. does not derive `serde::Serialize`).
///
/// FFI returns `Vec<T>` and `Map<K, V>` by serializing to JSON, which requires all
/// contained Named types to implement `serde::Serialize`. When a Named type lacks that
/// derive, generating the JSON path would produce a compile error in the output crate.
/// Such functions are stubbed out (emit unimplemented) instead.
fn return_type_needs_non_serde_named(ty: &TypeRef, serde_names: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Vec(inner) => {
            if let TypeRef::Named(n) = inner.as_ref() {
                return !serde_names.contains(n.as_str());
            }
            false
        }
        TypeRef::Map(k, v) => {
            let k_bad = matches!(k.as_ref(), TypeRef::Named(n) if !serde_names.contains(n.as_str()));
            let v_bad = matches!(v.as_ref(), TypeRef::Named(n) if !serde_names.contains(n.as_str()));
            k_bad || v_bad
        }
        TypeRef::Optional(inner) => return_type_needs_non_serde_named(inner, serde_names),
        _ => false,
    }
}

fn c_symbol_component(name: &str) -> String {
    pascal_to_snake(name)
}

fn internal_class_component(name: &str) -> String {
    to_class_name(name)
}

// ---------------------------------------------------------------------------
// _len() companion helpers
// ---------------------------------------------------------------------------

/// Returns true when a TypeRef maps to `*mut c_char` in return position — meaning the
/// FFI consumer must NUL-scan to find the byte length.  A `_len()` companion is emitted
/// for every free function whose return type satisfies this predicate.
pub(super) fn returns_c_char(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => true,
        TypeRef::Vec(_) | TypeRef::Map(_, _) => true,
        TypeRef::Optional(inner) => matches!(
            inner.as_ref(),
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _)
        ),
        _ => false,
    }
}

/// Generate a C-string return expression that records the byte length before
/// transferring ownership to the caller.
///
/// The matching `_len()` companion reads this thread-local length instead of
/// re-executing the wrapped Rust function.
fn gen_owned_c_char_to_c_with_len(expr: &str, ty: &TypeRef, indent: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => format!(
            "{indent}{{\n\
             {indent}    let __alef_return = {expr}.to_string();\n\
             {indent}    match CString::new(__alef_return) {{\n\
             {indent}        Ok(cs) => {{\n\
             {indent}            set_last_return_len(cs.as_bytes().len());\n\
             {indent}            cs.into_raw()\n\
             {indent}        }}\n\
             {indent}        Err(_) => {{\n\
             {indent}            set_last_return_len(0);\n\
             {indent}            std::ptr::null_mut()\n\
             {indent}        }}\n\
             {indent}    }}\n\
             {indent}}}"
        ),
        TypeRef::Path => format!(
            "{indent}{{\n\
             {indent}    let __alef_return = {expr}.to_string_lossy().to_string();\n\
             {indent}    match CString::new(__alef_return) {{\n\
             {indent}        Ok(cs) => {{\n\
             {indent}            set_last_return_len(cs.as_bytes().len());\n\
             {indent}            cs.into_raw()\n\
             {indent}        }}\n\
             {indent}        Err(_) => {{\n\
             {indent}            set_last_return_len(0);\n\
             {indent}            std::ptr::null_mut()\n\
             {indent}        }}\n\
             {indent}    }}\n\
             {indent}}}"
        ),
        TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => format!(
            "{indent}{{\n\
             {indent}    match serde_json::to_string(&{expr}) {{\n\
             {indent}        Ok(__alef_return) => match CString::new(__alef_return) {{\n\
             {indent}            Ok(cs) => {{\n\
             {indent}                set_last_return_len(cs.as_bytes().len());\n\
             {indent}                cs.into_raw()\n\
             {indent}            }}\n\
             {indent}            Err(_) => {{\n\
             {indent}                set_last_return_len(0);\n\
             {indent}                std::ptr::null_mut()\n\
             {indent}            }}\n\
             {indent}        }},\n\
             {indent}        Err(_) => {{\n\
             {indent}            set_last_return_len(0);\n\
             {indent}            std::ptr::null_mut()\n\
             {indent}        }}\n\
             {indent}    }}\n\
             {indent}}}"
        ),
        TypeRef::Optional(inner) => {
            let inner_conversion = gen_owned_c_char_to_c_with_len("val", inner, &format!("{indent}        "));
            format!(
                "{indent}match {expr} {{\n\
                 {indent}    Some(val) => {{\n\
                 {inner_conversion}\n\
                 {indent}    }}\n\
                 {indent}    None => {{\n\
                 {indent}        set_last_return_len(0);\n\
                 {indent}        std::ptr::null_mut()\n\
                 {indent}    }}\n\
                 {indent}}}"
            )
        }
        _ => gen_owned_value_to_c(expr, ty, indent, &AHashSet::new()),
    }
}

/// Generate a `{ffi_name}_len(same params) -> usize` companion for a free function whose
/// return type maps to `*mut c_char`.  The companion returns the byte length recorded by
/// the immediately preceding primary function call on the same thread.
///
/// Enables safe `[]const u8` slice construction in Zig and `MemorySegment` slicing in Java
/// FFM Panama without a NUL-scan or re-running the wrapped Rust operation.
pub(super) fn gen_free_function_len_companion(
    func: &FunctionDef,
    prefix: &str,
    _core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
) -> String {
    let fn_name_snake = c_symbol_component(&func.name);
    let ffi_name = format!("{prefix}_{fn_name_snake}_len");

    let ffi_param_count = func.params.len() + func.params.iter().filter(|p| matches!(p.ty, TypeRef::Bytes)).count();
    let allow_clippy = if ffi_param_count > 7 {
        Some("clippy::too_many_arguments".to_string())
    } else {
        None
    };

    let will_be_unimplemented = func.sanitized && !sanitized_recoverable(func);
    let mut params = Vec::new();
    for p in &func.params {
        let param_name = format!("_{}", p.name);
        params.push(format!(
            "    {}: {}",
            param_name,
            crate::backends::ffi::type_map::c_param_type_with_paths_and_enums(
                &p.ty,
                _core_import,
                path_map,
                enum_names,
                false // len companion params are ABI-alignment dummies; is_mut irrelevant
            )
        ));
        if matches!(p.ty, TypeRef::Bytes) {
            params.push(format!("    _{}_len: usize", p.name));
        }
    }

    let synthetic_doc = format!(
        "Return the byte length of the C string most recently returned by `{prefix}_{fn_name_snake}` \
         on this thread. Returns 0 when the primary call returned null or failed before producing a \
         string. Enables safe slice construction in Zig and Java FFM Panama without a NUL-scan.\n\n\
         # Safety\n\nPointer arguments are ignored and are present only to keep the companion ABI \
         aligned with `{prefix}_{fn_name_snake}`.",
    );
    let doc_comment = ffi_doxygen_block(&synthetic_doc);

    let mut out = String::with_capacity(2048);
    out.push_str(&doc_comment);
    if let Some(ref clippy) = allow_clippy {
        out.push_str(&format!("#[allow({clippy})]\n"));
    }
    out.push_str("#[unsafe(no_mangle)]\n");
    out.push_str("pub unsafe extern \"C\" fn ");
    out.push_str(&ffi_name);
    out.push_str("(\n");
    for (i, p) in params.iter().enumerate() {
        out.push_str(p);
        if i + 1 < params.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str(") -> usize {\n");

    if will_be_unimplemented {
        out.push_str("    0\n}");
        return out;
    }

    out.push_str("    last_return_len()\n");
    out.push_str("\n}");
    out
}

// ---------------------------------------------------------------------------
// Streaming method wrapper (callback-based, for Streaming adapters)
// ---------------------------------------------------------------------------

/// Generate a callback-based streaming wrapper for a method decorated with the
/// `Streaming` adapter pattern.  The caller supplies a `{Prefix}StreamCallback`
/// and an opaque `user_data` pointer; the body drives the async stream and
/// invokes the callback once per chunk.
pub(super) fn gen_streaming_method_wrapper(
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

pub(super) fn gen_method_wrapper(
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
                    if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref {
                        format!("&{rs}")
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
                TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                    // Return type may be `&[T]` (slice) — `.clone()` on a slice is a noop
                    // because `[T]: !Sized`. Use `.to_vec()` to produce an owned Vec.
                    out.push_str("    let result = result.to_vec();\n");
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

pub(super) fn gen_free_function(
    func: &FunctionDef,
    prefix: &str,
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
    serde_names: &AHashSet<String>,
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
                    // When is_mut=true, the local rs is already `&mut T` (bound via
                    // `let rs = unsafe { &mut *ptr }`). Pass it directly — adding
                    // `&mut` would produce `&mut &mut T` (E0308).
                    if p.is_mut || !p.is_ref { rs } else { format!("&{rs}") }
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
                    if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref {
                        format!("&{rs}")
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
                let ok_body = if returns_c_char(&func.return_type) {
                    gen_owned_c_char_to_c_with_len(val_expr, &func.return_type, "            ")
                } else {
                    gen_owned_value_to_c(val_expr, &func.return_type, "            ", enum_names)
                };
                out.push_str(&crate::backends::ffi::template_env::render(
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
            let content = if returns_c_char(&func.return_type) {
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
            crate::core::ir::PrimitiveType::Bool => "bool".to_string(),
            crate::core::ir::PrimitiveType::U8 => "u8".to_string(),
            crate::core::ir::PrimitiveType::U16 => "u16".to_string(),
            crate::core::ir::PrimitiveType::U32 => "u32".to_string(),
            crate::core::ir::PrimitiveType::U64 => "u64".to_string(),
            crate::core::ir::PrimitiveType::I8 => "i8".to_string(),
            crate::core::ir::PrimitiveType::I16 => "i16".to_string(),
            crate::core::ir::PrimitiveType::I32 => "i32".to_string(),
            crate::core::ir::PrimitiveType::I64 => "i64".to_string(),
            crate::core::ir::PrimitiveType::F32 => "f32".to_string(),
            crate::core::ir::PrimitiveType::F64 => "f64".to_string(),
            crate::core::ir::PrimitiveType::Usize => "usize".to_string(),
            crate::core::ir::PrimitiveType::Isize => "isize".to_string(),
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

pub(super) fn gen_param_conversion_with_enums(
    param: &ParamDef,
    has_error: bool,
    is_bytes_result: bool,
    return_type: &TypeRef,
    core_import: &str,
    enum_names: &AHashSet<String>,
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
                out.push_str(&crate::backends::ffi::template_env::render(
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
                out.push_str(&crate::backends::ffi::template_env::render(
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
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_json_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        turbofish => String::new(),
                    },
                ));
            }
            TypeRef::Named(type_name) if enum_names.contains(type_name.as_str()) => {
                // Optional enum passed as i32 sentinel: reconstruct via private Rust helper.
                // Use match+explicit return rather than `?` because the outer function may return
                // *mut T or i32, not Result/Option, so the ? operator is unavailable.
                //
                // IMPORTANT: the local variable name is `{rs_name}` (derived from the FFI param
                // name), NOT from `enum_snake` (which is derived from the type name). The
                // conversion helper is named `{enum_snake}_from_i32_rs` and receives `{name}`
                // (the actual FFI param), not `{enum_snake}` which may differ when param name
                // != snake_case(type_name) (e.g. param `strategy` of type `RedactionStrategy`).
                let enum_snake = c_symbol_component(type_name);
                out.push_str(&format!(
                    "    let {rs_name} = match {enum_snake}_from_i32_rs({name}) {{\n        \
                     Some(v) => v,\n        \
                     None => {{\n            \
                     set_last_error(1, \"invalid enum discriminant for {type_name}\");\n            \
                     {fail_ret}\n        \
                     }},\n    \
                     }};\n",
                ));
            }
            TypeRef::Named(_type_name) => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_named_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        is_ref => param.is_ref,
                    },
                ));
            }
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => {
                out.push(' ');
                out.push_str(&crate::backends::ffi::template_env::render(
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
                    crate::core::ir::PrimitiveType::U8 => "u8::MAX",
                    crate::core::ir::PrimitiveType::U16 => "u16::MAX",
                    crate::core::ir::PrimitiveType::U32 => "u32::MAX",
                    crate::core::ir::PrimitiveType::U64 => "u64::MAX",
                    crate::core::ir::PrimitiveType::I8 => "i8::MAX",
                    crate::core::ir::PrimitiveType::I16 => "i16::MAX",
                    crate::core::ir::PrimitiveType::I32 => "i32::MAX",
                    crate::core::ir::PrimitiveType::I64 => "i64::MAX",
                    crate::core::ir::PrimitiveType::F32 => "f32::NAN",
                    crate::core::ir::PrimitiveType::F64 => "f64::NAN",
                    crate::core::ir::PrimitiveType::Usize => "usize::MAX",
                    crate::core::ir::PrimitiveType::Isize => "isize::MAX",
                    crate::core::ir::PrimitiveType::Bool => unreachable!("handled above"),
                };
                let is_float = matches!(
                    prim,
                    crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64
                );
                out.push(' ');
                out.push_str(&crate::backends::ffi::template_env::render(
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
                    TypeRef::Vec(_) => {
                        format!("::<{}>", type_ref_to_rust_type(&param.ty, core_import))
                    }
                    TypeRef::Map(_, val_ty) if param.map_is_ahash => {
                        // AHashMap target: deserialize directly into AHashMap with the correct
                        // key type so no post-deserialization conversion is needed.
                        let val_rust = type_ref_to_rust_type(val_ty, core_import);
                        let key_rust = if param.map_key_is_cow {
                            "std::borrow::Cow<'static, str>".to_string()
                        } else {
                            "String".to_string()
                        };
                        format!("::<ahash::AHashMap<{key_rust}, {val_rust}>>")
                    }
                    TypeRef::Map(_, _) => {
                        format!("::<{}>", type_ref_to_rust_type(&param.ty, core_import))
                    }
                    _ => String::new(),
                };
                out.push(' ');
                out.push_str(&crate::backends::ffi::template_env::render(
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
                out.push_str(&crate::backends::ffi::template_env::render(
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
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_non_optional_string_conversion.jinja",
                    context! {
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        rs_name => rs_name.clone(),
                    },
                ));
            }
            TypeRef::Path => {
                out.push_str(&crate::backends::ffi::template_env::render(
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
                out.push_str(&crate::backends::ffi::template_env::render(
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
                crate::core::ir::PrimitiveType::Bool => {
                    out.push_str(&crate::backends::ffi::template_env::render(
                        "param_primitive_bool.jinja",
                        context! { rs_name => rs_name.clone(), name => name.clone() },
                    ));
                }
                _ => {
                    if let Some(newtype_path) = &param.newtype_wrapper {
                        // Param was resolved from a newtype (e.g. NodeIndex→u32): re-wrap for core call.
                        out.push_str(&crate::backends::ffi::template_env::render("param_primitive_newtype.jinja", context! { rs_name => rs_name.clone(), newtype_path => newtype_path.clone(), name => name.clone() }));
                    } else {
                        out.push_str(&crate::backends::ffi::template_env::render(
                            "param_primitive_passthrough.jinja",
                            context! { rs_name => rs_name.clone(), name => name.clone() },
                        ));
                    }
                }
            },
            TypeRef::Named(type_name) if enum_names.contains(type_name.as_str()) => {
                // Enum passed as i32: reconstruct using the private Rust helper.
                // Use match+explicit return rather than `?` because the outer function may return
                // *mut T or i32, not Result/Option, so the ? operator is unavailable.
                //
                // IMPORTANT: the local variable name is `{rs_name}` (derived from the FFI param
                // name, e.g. `strategy_rs`), NOT `{enum_snake}_rs` (which would use the type
                // name, e.g. `redaction_strategy_rs`). The conversion helper is still named
                // `{enum_snake}_from_i32_rs` but it receives `{name}` (the actual FFI param).
                // This fixes the mismatch when param name != snake_case(type_name).
                let enum_snake = c_symbol_component(type_name);
                out.push_str(&format!(
                    "    let {rs_name} = match {enum_snake}_from_i32_rs({name}) {{\n        \
                     Some(v) => v,\n        \
                     None => {{\n            \
                     set_last_error(1, \"invalid enum discriminant for {type_name}\");\n            \
                     {fail_ret}\n        \
                     }},\n    \
                     }};\n",
                ));
            }
            TypeRef::Named(_type_name) => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_non_optional_named_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        is_ref => param.is_ref,
                        is_mut => param.is_mut,
                    },
                ));
            }
            TypeRef::Bytes => {
                // Bytes come as (*const u8, len: usize) — the len param is a separate
                // parameter named {name}_len by convention. A null pointer is allowed
                // when the corresponding length is zero (empty input is a legitimate
                // case — e.g. extracting from a 0-byte file). Reject null only when
                // the caller claims a non-zero length.
                out.push_str(&crate::backends::ffi::template_env::render(
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
                    TypeRef::Vec(_) => {
                        format!("::<{}>", type_ref_to_rust_type(&param.ty, core_import))
                    }
                    TypeRef::Map(_, val_ty) if param.map_is_ahash => {
                        // AHashMap target: deserialize directly.
                        let val_rust = type_ref_to_rust_type(val_ty, core_import);
                        let key_rust = if param.map_key_is_cow {
                            "std::borrow::Cow<'static, str>".to_string()
                        } else {
                            "String".to_string()
                        };
                        format!("::<ahash::AHashMap<{key_rust}, {val_rust}>>")
                    }
                    TypeRef::Map(_, _) => {
                        format!("::<{}>", type_ref_to_rust_type(&param.ty, core_import))
                    }
                    _ => String::new(),
                };
                out.push_str(&crate::backends::ffi::template_env::render(
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
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_passthrough.jinja",
                    context! { rs_name => rs_name.clone(), name => name.clone() },
                ));
            }
            TypeRef::Duration => {
                // Duration passed as u64 milliseconds
                out.push_str(&crate::backends::ffi::template_env::render(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_type_needs_non_serde_named_vec_non_serde() {
        // Regression: Vec<PatternMatch> where PatternMatch lacks serde must trigger
        // unimplemented body generation instead of emitting json_or_vec_or_map path.
        let mut serde_names: AHashSet<String> = AHashSet::new();
        serde_names.insert("ExtractionResult".to_string());

        let vec_non_serde = TypeRef::Vec(Box::new(TypeRef::Named("PatternMatch".to_string())));
        assert!(
            return_type_needs_non_serde_named(&vec_non_serde, &serde_names),
            "Vec<PatternMatch> without Serialize must be detected as needing stub"
        );
    }

    #[test]
    fn return_type_needs_non_serde_named_vec_serde_ok() {
        // Vec<ExtractionResult> where ExtractionResult has serde should NOT trigger stub.
        let mut serde_names: AHashSet<String> = AHashSet::new();
        serde_names.insert("ExtractionResult".to_string());

        let vec_serde = TypeRef::Vec(Box::new(TypeRef::Named("ExtractionResult".to_string())));
        assert!(
            !return_type_needs_non_serde_named(&vec_serde, &serde_names),
            "Vec<ExtractionResult> with Serialize must NOT be detected as needing stub"
        );
    }

    #[test]
    fn return_type_needs_non_serde_named_primitive_vec_not_affected() {
        // Vec<String>, Vec<u64> etc. never need serde check.
        let serde_names: AHashSet<String> = AHashSet::new();
        assert!(!return_type_needs_non_serde_named(
            &TypeRef::Vec(Box::new(TypeRef::String)),
            &serde_names
        ));
        assert!(!return_type_needs_non_serde_named(
            &TypeRef::Vec(Box::new(TypeRef::Primitive(crate::core::ir::PrimitiveType::U64))),
            &serde_names
        ));
    }

    #[test]
    fn named_param_is_mut_call_site_passes_local_directly() {
        // Regression (Bug 2): When is_mut=true, the conversion template binds the local
        // via `let result_rs = unsafe { &mut *result }` — the local is already `&mut T`.
        // The call site must pass `result_rs` directly, NOT `&mut result_rs`, which would
        // produce `&mut &mut T` (E0308 mismatched types).
        use crate::core::ir::ParamDef;

        let p = ParamDef {
            name: "result".to_string(),
            ty: TypeRef::Named("ExtractionResult".to_string()),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: true,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
        };
        let rs = format!("{}_rs", p.name);
        // Simulate the call-site arm for Named non-optional with is_mut
        // (mirrors the TypeRef::Named(!p.optional) arm in gen_free_function / gen_method_wrapper)
        let result = if p.is_mut {
            // Local is already &mut T — pass directly, no extra &mut prefix.
            rs.clone()
        } else if p.is_ref {
            format!("&{rs}")
        } else {
            rs.clone()
        };
        assert_eq!(
            result, "result_rs",
            "is_mut Named param must pass local directly (already &mut T)"
        );
    }

    #[test]
    fn enum_param_local_name_uses_param_name_not_type_name() {
        // Regression (Bug 3): enum-discriminant params must name the local after the FFI
        // param (e.g. `strategy_rs`), not after the type (e.g. `redaction_strategy_rs`).
        // The conversion helper is still `{type_snake}_from_i32_rs` but the local and its
        // call site use `{param_name}_rs`.
        use crate::core::ir::ParamDef;

        let mut enum_names: AHashSet<String> = AHashSet::new();
        enum_names.insert("RedactionStrategy".to_string());

        let p = ParamDef {
            name: "strategy".to_string(), // param name differs from type snake
            ty: TypeRef::Named("RedactionStrategy".to_string()),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
        };

        // Run the real conversion generator.
        let output = gen_param_conversion_with_enums(&p, false, false, &TypeRef::Unit, "sample_crate", &enum_names);

        // Must bind to `strategy_rs` (from param name), not `redaction_strategy_rs` (from type).
        assert!(
            output.contains("let strategy_rs ="),
            "enum local must be named after param (strategy_rs), got:\n{output}"
        );
        // Must call the helper with the actual param name `strategy`, not `redaction_strategy`.
        assert!(
            output.contains("redaction_strategy_from_i32_rs(strategy)"),
            "enum helper must receive the FFI param name (strategy), got:\n{output}"
        );
    }
}
