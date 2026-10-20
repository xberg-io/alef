use super::result_presence::free_function_presence_gate;
use crate::core::ir::{FunctionDef, ParamDef, PrimitiveType, TypeRef};

use super::errors::{UNMATCHED_ERROR_SET, resolve_zig_error_type};
use super::helpers::emit_cleaned_zig_doc;
use super::types::{c_symbol_component, zig_field_type};

/// Returns true if `ty` (or its `Optional<>` inner) is a struct named in
/// `struct_names`. Struct parameters are passed across the FFI as opaque
/// handles, so the wrapper accepts a JSON `[]const u8` and converts to the
/// handle via the FFI's `<prefix>_<snake>_from_json` helper.
fn is_struct_named(ty: &TypeRef, struct_names: &std::collections::HashSet<String>) -> bool {
    match ty {
        TypeRef::Named(name) => struct_names.contains(name),
        TypeRef::Optional(inner) => is_struct_named(inner, struct_names),
        _ => false,
    }
}

/// Return the inner `Named(name)` for a struct parameter type.
fn struct_named_inner(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => struct_named_inner(inner),
        _ => None,
    }
}

/// Like `struct_named_inner` but searches for any Named type (used for opaque handle detection).
/// Returns the type name if `ty` (or its Optional inner) is a Named type.
pub(crate) fn opaque_type_name_inner(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => opaque_type_name_inner(inner),
        _ => None,
    }
}

/// Returns the opaque type name if `ty` is (or wraps in Optional) a Named type
/// that is in `opaque_creator_map`.
fn get_opaque_named<'a>(
    ty: &'a TypeRef,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
) -> Option<&'a str> {
    match ty {
        TypeRef::Named(name) if opaque_creator_map.contains_key(name.as_str()) => Some(name.as_str()),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_creator_map.contains_key(name.as_str()) => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// Returns true if generating the param-conversion boilerplate for `p` will
/// emit a `try` expression (heap allocation or fallible operation).
fn needs_alloc_param(p: &ParamDef) -> bool {
    let inner = match &p.ty {
        TypeRef::Optional(t) => t.as_ref(),
        other => other,
    };
    matches!(
        inner,
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_)
    )
}

fn needs_from_json_param(
    p: &ParamDef,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
) -> bool {
    get_opaque_named(&p.ty, opaque_creator_map).is_some() || is_struct_named(&p.ty, struct_names)
}

fn return_type_can_be_null(ty: &TypeRef, _struct_names: &std::collections::HashSet<String>) -> bool {
    match ty {
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Path
        | TypeRef::Json
        | TypeRef::Bytes
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _) => true,
        // `Bytes` is deliberately absent here: unlike String/Path/Json/Vec/Map, an
        // `Optional<Bytes>` return gets no `_len()` companion (see `returns_c_char` in the
        // FFI backend) and its `None` case never sets the FFI last-error state — so treating
        // a null as an error here would misclassify a legitimate `None` as a failure.
        // `Optional<Bytes>` instead rides the byte-buffer out-param convention (see
        // `return_uses_bytes_out_params`), where the C function returns an `i32` status and
        // absence arrives as a null `_out_ptr`; that makes `result_is_pointer` false, so this
        // predicate is never consulted for it. It stays excluded as a guard against a future
        // change routing it back through the pointer-return path. ~keep
        TypeRef::Optional(inner) => matches!(
            inner.as_ref(),
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _)
        ),
        _ => false,
    }
}

mod free_functions;
pub(crate) use free_functions::emit_function;

/// Emit a Zig wrapper for a function returning a host-native capsule (Language) type.
///
/// The C symbol returns the host runtime's raw grammar pointer; the wrapper constructs the
/// host `Language` using the expression from `cap.construct_expr`.
///
/// `cap.host_type` and `cap.construct_expr` are required; missing values produce a
/// `// ALEF ERROR:` comment in the generated output rather than silently falling
/// back to a hardcoded default.
fn emit_capsule_function(
    f: &FunctionDef,
    prefix: &str,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
    cap: &crate::core::config::HostCapsuleTypeConfig,
    declared_errors: &[String],
    out: &mut String,
) {
    emit_cleaned_zig_doc(out, &f.doc, "");

    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format_param_wrapper(p, struct_names, opaque_creator_map))
        .collect();

    let body_needs_try = f.params.iter().any(needs_alloc_param);
    let host_type = match cap.required_host_type("Language", "zig") {
        Ok(t) => t.to_string(),
        Err(e) => {
            out.push_str(&format!("// ALEF ERROR: {e}\n"));
            return;
        }
    };
    let zig_error_type = f
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors));
    let return_ty = if let Some(err) = &zig_error_type {
        format!("{err}!{host_type}")
    } else if body_needs_try {
        format!("error{{OutOfMemory}}!{host_type}")
    } else {
        host_type.clone()
    };

    out.push_str(&crate::backends::zig::template_env::render(
        "function_signature.jinja",
        crate::alef_context! {
            func_name => &f.name,
            params => params.join(", "),
            return_ty => &return_ty,
        },
    ));

    for p in &f.params {
        emit_param_conversion(
            p,
            prefix,
            struct_names,
            opaque_creator_map,
            "return error.OutOfMemory;",
            out,
        );
    }

    let c_args: Vec<String> = f
        .params
        .iter()
        .flat_map(|p| c_arg_names(p, struct_names, opaque_creator_map))
        .collect();
    let c_call = format!("c.{prefix}_{}({})", f.name, c_args.join(", "));
    out.push_str(&crate::backends::zig::template_env::render(
        "function_call_result.jinja",
        crate::alef_context! {
            c_call => &c_call,
        },
    ));

    for p in &f.params {
        emit_param_free(p, prefix, struct_names, opaque_creator_map, out);
    }

    if let Some(error_type) = &zig_error_type {
        out.push_str(&crate::backends::zig::template_env::render(
            "function_error_check.jinja",
            crate::alef_context! { prefix => prefix },
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "function_error_return.jinja",
            crate::alef_context! { error_type => error_type },
        ));
        out.push_str("    }\n");
    }

    out.push_str("    if (_result == null) return null;\n");
    let construct = match cap.construct_required("_result", "Language", "zig") {
        Ok(c) => c,
        Err(e) => {
            out.push_str(&format!("    // ALEF ERROR: {e}\n"));
            out.push_str("}\n");
            return;
        }
    };
    out.push_str(&format!("    return {construct};\n"));
    out.push_str("}\n");
}

/// Return the Zig-wrapper parameter type string for a function parameter.
fn format_param_wrapper(
    p: &ParamDef,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
) -> String {
    let ty_str = zig_param_type(&p.ty, p.optional, struct_names, opaque_creator_map);
    format!("{}: {}", p.name, ty_str)
}

/// The Zig wrapper's return-type text: an error union (`<Set>!<T>`) if the function is
/// fallible in Zig, or the bare type otherwise. Fallibility is inferred exactly as
/// `emit_function`'s body infers it -- from a declared `error_type`, or from any
/// parameter/return conversion that itself needs `try` (`error{OutOfMemory}` /
/// `error{OutOfMemory,InvalidJson}`) -- so this is the single place both `emit_function`
/// and the breaking-signature-change baseline
/// (`ZigBackend::public_function_signatures`) compute it, and the two can never quietly
/// disagree about what "the emitted return type" is. ~keep
pub(crate) fn wrapper_return_type(
    f: &FunctionDef,
    declared_errors: &[String],
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
) -> String {
    let zig_error_type = f
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors));

    let body_needs_try = f.params.iter().any(needs_alloc_param)
        || return_conversion_needs_out_of_memory(&f.return_type, struct_names)
        || return_uses_bytes_out_params(&f.return_type);
    let body_needs_invalid_json = f
        .params
        .iter()
        .any(|p| needs_from_json_param(p, struct_names, opaque_creator_map));

    if let Some(error_type) = &zig_error_type {
        format!("{}!{}", error_type, zig_return_type(&f.return_type, struct_names))
    } else if infallible_result_needs_null_check(&f.return_type) {
        let err_set = if body_needs_invalid_json {
            "error{OutOfMemory,InvalidJson,UnknownFfiError,Cancelled}"
        } else {
            UNMATCHED_ERROR_SET
        };
        format!("{err_set}!{}", zig_return_type(&f.return_type, struct_names))
    } else if body_needs_try || body_needs_invalid_json {
        let err_set = if body_needs_invalid_json {
            "error{OutOfMemory,InvalidJson}"
        } else {
            "error{OutOfMemory}"
        };
        format!("{err_set}!{}", zig_return_type(&f.return_type, struct_names))
    } else {
        zig_return_type(&f.return_type, struct_names)
    }
}

/// True for a function with no Rust error type whose C result is an owned string pointer.
/// The C function returns null for an argument that is not valid UTF-8 and when the value
/// cannot be serialized, even though the Rust function cannot fail, so the wrapper must read
/// the native error instead of slicing the null pointer. ~keep
fn infallible_result_needs_null_check(ty: &TypeRef) -> bool {
    !return_uses_bytes_out_params(ty)
        && matches!(
            ty,
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _)
        )
}

/// The Zig-wrapper parameter *types* only, in call order, without the parameter labels
/// `format_param_wrapper` includes. Zig calls are positional, so a caller's build
/// compatibility depends on the type/count/order of this list, not on what the wrapper
/// happens to label a parameter -- the breaking-signature-change baseline compares this so
/// a cosmetic parameter rename does not read as a breaking change. ~keep
pub(crate) fn wrapper_param_types(
    f: &FunctionDef,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
) -> Vec<String> {
    f.params
        .iter()
        .map(|p| zig_param_type(&p.ty, p.optional, struct_names, opaque_creator_map))
        .collect()
}

/// Zig type used at the wrapper boundary for a function parameter.
///
/// - `String`, `Path` → `[]const u8`  (body allocates null-terminated copy)
/// - `Bytes`          → `[]const u8`  (body passes `.ptr` + `.len`)
/// - `Vec`, `Map`     → `[]const u8`  (caller supplies JSON; body passes as C string)
/// - `Named` struct   → `[]const u8`  (caller supplies JSON; body converts to opaque
///   handle via the FFI `<prefix>_<snake>_from_json` helper)
/// - Everything else  → same as struct-field type
pub(crate) fn zig_param_type(
    ty: &TypeRef,
    optional: bool,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
) -> String {
    if get_opaque_named(ty, opaque_creator_map).is_some() {
        return "?[]const u8".to_string();
    }
    let inner = match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Bytes | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "[]const u8".to_string()
        }
        TypeRef::Named(name) if struct_names.contains(name) => "[]const u8".to_string(),
        TypeRef::Optional(inner) => {
            let inner_str = zig_param_type(inner, false, struct_names, opaque_creator_map);
            return format!("?{inner_str}");
        }
        other => zig_field_type(other, false),
    };
    if optional { format!("?{inner}") } else { inner }
}

/// Emit the allocation / conversion lines needed before the C call for `p`.
///
/// String/Path: allocate a null-terminated copy via `std.heap.c_allocator`.
/// Vec/Map:     same — caller supplies a JSON `[]const u8`; we need a sentinel-
///              terminated copy to pass to `*const c_char` parameters.
/// Named struct (opt or required): caller supplies JSON `[]const u8`; we
///              allocate a sentinel-terminated copy and convert it to an
///              opaque FFI handle via `<prefix>_<snake>_from_json`. The
///              optional variant unwraps the optional first and substitutes
///              `null` for the C handle when the wrapper arg is `null`.
/// Bytes:       nothing needed; `.ptr` and `.len` are used directly in `c_arg_names`.
fn emit_param_conversion(
    p: &ParamDef,
    prefix: &str,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
    json_error_return: &str,
    out: &mut String,
) {
    let name = &p.name;

    if let Some(opaque_name) = get_opaque_named(&p.ty, opaque_creator_map) {
        if let Some((creator_fn, config_snake)) = opaque_creator_map.get(opaque_name) {
            out.push_str(&crate::backends::zig::template_env::render(
                "param_opaque_config_from_json.jinja",
                crate::alef_context! {
                    name => name,
                    prefix => prefix,
                    creator_fn => creator_fn,
                    config_snake => config_snake,
                    name_snake => &c_symbol_component(opaque_name),
                    json_error_return => json_error_return,
                },
            ));
        }
        return;
    }

    if let Some(inner_name) = struct_named_inner(&p.ty)
        && struct_names.contains(inner_name)
    {
        let snake = c_symbol_component(inner_name);
        let is_optional = p.optional || matches!(p.ty, TypeRef::Optional(_));
        if is_optional {
            out.push_str(&crate::backends::zig::template_env::render(
                "param_optional_string_alloc.jinja",
                crate::alef_context! { name => name },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "param_optional_struct_handle.jinja",
                crate::alef_context! {
                    name => name,
                    prefix => prefix,
                    snake => &snake,
                    json_error_return => json_error_return,
                },
            ));
        } else {
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line1.jinja",
                crate::alef_context! { name => name },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line2.jinja",
                crate::alef_context! { name => name },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "param_struct_handle.jinja",
                crate::alef_context! {
                    name => name,
                    prefix => prefix,
                    snake => &snake,
                    json_error_return => json_error_return,
                },
            ));
        }
        return;
    }
    let is_optional_string = p.optional
        || matches!(
                &p.ty,
                TypeRef::Optional(inner)
                    if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        );
    if is_optional_string && matches!(unwrap_optional(&p.ty), TypeRef::String | TypeRef::Path) {
        out.push_str(&crate::backends::zig::template_env::render(
            "param_optional_string_alloc.jinja",
            crate::alef_context! { name => name },
        ));
        return;
    }
    match &p.ty {
        TypeRef::String | TypeRef::Path => {
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line1.jinja",
                crate::alef_context! {
                    name => name,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line2.jinja",
                crate::alef_context! {
                    name => name,
                },
            ));
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            out.push_str("    // Vec/Map parameters are passed as JSON strings across the FFI boundary.\n");
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line1.jinja",
                crate::alef_context! {
                    name => name,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line2.jinja",
                crate::alef_context! {
                    name => name,
                },
            ));
        }
        _ => {}
    }
}

/// Strip a single `Optional<>` layer if present.
fn unwrap_optional(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Optional(inner) => inner,
        other => other,
    }
}

/// Returns true when a return type crosses the C boundary via the byte-buffer out-param
/// convention (`&_out_ptr, &_out_len, &_out_cap` trailing args, `i32` status return)
/// instead of as a direct C return value.
///
/// `Optional<Bytes>` shares the convention with bare `Bytes`; `None` arrives as a null
/// `_out_ptr` with `_out_len == _out_cap == 0`, while `Some(&[])` arrives as a non-null
/// `_out_ptr` with `_out_len == 0`. This is why `Bytes` cannot use the `_len()` companion
/// the `Char`/`String` path uses: `Bytes` carries no NUL terminator, so its length only
/// ever exists in `*_out_len`.
///
/// Must mirror `crate::backends::ffi::gen_bindings::functions::returns_bytes_out_params`.
/// ~keep
pub(crate) fn return_uses_bytes_out_params(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Bytes => true,
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Bytes),
        _ => false,
    }
}

/// Returns true when a return type maps to `*mut c_char` and therefore has a
/// matching `_len()` companion in alef-backend-ffi.
///
/// Must mirror `crate::backends::ffi::gen_bindings::functions::returns_c_char`.
pub(crate) fn return_uses_len_companion(ty: &TypeRef) -> bool {
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

/// Returns true if converting a raw C return value of type `ty` into the Zig wrapper's
/// return type requires the body to fall back to `error.OutOfMemory` -- either a
/// caller-owned allocation (`try std.heap.c_allocator.dupe`, which Zig infers as
/// `error{OutOfMemory}` regardless of what the declared signature says) or a zero/null
/// opaque-handle check that the body maps onto `error.OutOfMemory` by hand.
///
/// This is the single place `wrapper_return_type` (this module) and `method_return_type`
/// (`opaque_handles/instance_methods.rs`) query to decide whether the declared return type
/// must carry `OutOfMemory` -- both signature computations call it instead of each
/// re-deriving its own list of "shapes that need `try`", which is what let a handle-return
/// method (`Node`, `TreeCursor`, ...) declare `error{HandleClosed}!Node` while its body,
/// emitted by `unwrap_return_expr`/`method_unwrap_return_expr`, unconditionally returns
/// `error.OutOfMemory` for exactly that shape.
///
/// A direct (non-`Optional`) `Named` return always needs it: both the JSON round-trip taken
/// by a `has_serde` struct and the null-handle check taken by a bare opaque handle perform a
/// fallible operation the body cannot avoid. An `Optional<Named>` bare-handle return is the
/// one exception -- it degrades a zero handle straight to Zig `null` with no `try` and no
/// error literal, so it must stay excluded or every method returning `?Node`-shaped values
/// would gain an error union its body never uses. ~keep
pub(crate) fn return_conversion_needs_out_of_memory(
    ty: &TypeRef,
    struct_names: &std::collections::HashSet<String>,
) -> bool {
    match ty {
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Path
        | TypeRef::Json
        | TypeRef::Bytes
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _)
        | TypeRef::Named(_) => true,
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                true
            }
            TypeRef::Named(name) => struct_names.contains(name),
            _ => false,
        },
        _ => false,
    }
}

/// Collect every distinct `error.<Variant>` literal a generated function body returns.
fn body_error_literals(body: &str) -> std::collections::HashSet<String> {
    const NEEDLE: &str = "error.";
    let mut found = std::collections::HashSet::new();
    let mut search_from = 0usize;
    while let Some(offset) = body[search_from..].find(NEEDLE) {
        let start = search_from + offset + NEEDLE.len();
        let end = body[start..]
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map_or(body.len(), |rel| start + rel);
        found.insert(body[start..end].to_string());
        search_from = end.max(start + 1);
    }
    found
}

/// Verify the mechanical invariant this module exists to enforce: every `error.<Variant>` the
/// emitted body can return must be admitted by the function's declared error set. A violation
/// means the emitted Zig cannot compile -- provably broken output, not a style question -- so
/// this panics rather than warning, mirroring `template_env::render`'s panic on a template that
/// fails to exist or render (an emitter-internal invariant, as opposed to a user-fixable config
/// gap like a missing capsule `host_type`, which instead gets an inline `// ALEF ERROR:`
/// comment because the user, not the emitter, can act on it).
///
/// `declared_errors` lets a custom Rust-declared error type satisfy `OutOfMemory`/
/// `UnknownFfiError` by name: `errors::emit_error_set` guarantees every declared set carries
/// both even when `return_ty`'s text is just the bare type name and doesn't spell them out. ~keep
pub(crate) fn assert_error_set_covers_body(context: &str, return_ty: &str, body: &str, declared_errors: &[String]) {
    // `anyerror` is Zig's universal error set: it admits every error by construction, so a
    // containment check against the declared text is meaningless there and would panic on output
    // that compiles fine. Checked before anything else because this invariant hard-fails -- an
    // over-strict check on legitimate output is worse than the divergence it hunts. ~keep
    if return_ty.contains("anyerror") {
        return;
    }
    let custom_error_declared = declared_errors.iter().any(|name| return_ty.contains(name.as_str()));
    for token in body_error_literals(body) {
        let admitted = return_ty.contains(&token)
            || (custom_error_declared && matches!(token.as_str(), "OutOfMemory" | "UnknownFfiError"));
        assert!(
            admitted,
            "zig codegen invariant violated for `{context}`: body returns `error.{token}` but \
             declared return type `{return_ty}` does not admit it -- the declared error set and \
             the emitted body's `return error.X` sites have diverged."
        );
    }
}

/// Return the max-value sentinel literal for a primitive integer type, if one
/// is used by the C FFI to represent `None`.  The Rust FFI layer uses
/// `<Type>::MAX` as the sentinel for optional numeric primitives.
pub(super) fn optional_int_sentinel(prim: &PrimitiveType) -> Option<&'static str> {
    match prim {
        PrimitiveType::U8 => Some("std.math.maxInt(u8)"),
        PrimitiveType::U16 => Some("std.math.maxInt(u16)"),
        PrimitiveType::U32 => Some("std.math.maxInt(u32)"),
        PrimitiveType::U64 | PrimitiveType::Usize => Some("std.math.maxInt(u64)"),
        PrimitiveType::I8 => Some("std.math.maxInt(i8)"),
        PrimitiveType::I16 => Some("std.math.maxInt(i16)"),
        PrimitiveType::I32 => Some("std.math.maxInt(i32)"),
        PrimitiveType::I64 | PrimitiveType::Isize => Some("std.math.maxInt(i64)"),
        _ => None,
    }
}

/// Emit the deallocation lines for allocations made in `emit_param_conversion`.
///
/// These are emitted after the C call (and after the error check) so the
/// allocations are always freed even when an error is returned.
pub(super) fn emit_param_free(
    p: &ParamDef,
    _prefix: &str,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
    out: &mut String,
) {
    let name = &p.name;

    if let Some(opaque_name) = get_opaque_named(&p.ty, opaque_creator_map) {
        if let Some((_, config_snake)) = opaque_creator_map.get(opaque_name) {
            let config_name = format!("{name}_config");
            out.push_str(&crate::backends::zig::template_env::render(
                "param_optional_free.jinja",
                crate::alef_context! {
                    name => &config_name,
                },
            ));
            let _ = config_snake;
        }
        return;
    }

    if let Some(inner_name) = struct_named_inner(&p.ty)
        && struct_names.contains(inner_name)
    {
        let _ = inner_name;
    }
}

/// The C argument name(s) to use for a given wrapper parameter.
///
/// Bytes expand to two arguments: `.ptr` and `.len`.
/// String/Path/Vec/Map expand to the `_z` null-terminated copy.
/// Optional String/Path expand to a conditional unwrap of the optional slice
/// to its `.ptr`, substituting `null` when the wrapper arg was null — Zig
/// does not auto-coerce `?[:0]u8` into `?[*:0]const u8`.
/// Named structs expand to the `_handle` opaque pointer produced by the
/// JSON-to-handle helper in `emit_param_conversion`.
/// Everything else passes the parameter directly.
fn c_arg_names(
    p: &ParamDef,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
) -> Vec<String> {
    if get_opaque_named(&p.ty, opaque_creator_map).is_some() {
        return vec![format!("{}_handle", p.name)];
    }
    if is_struct_named(&p.ty, struct_names) {
        return vec![format!("{}_handle", p.name)];
    }
    let is_optional_string = p.optional
        || matches!(
            &p.ty,
            TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        );
    if is_optional_string && matches!(unwrap_optional(&p.ty), TypeRef::String | TypeRef::Path) {
        return vec![format!("if ({0}_z) |z| z.ptr else null", p.name)];
    }
    {
        let prim_opt = match &p.ty {
            TypeRef::Optional(inner) => {
                if let TypeRef::Primitive(prim) = inner.as_ref() {
                    Some(prim)
                } else {
                    None
                }
            }
            TypeRef::Primitive(prim) if p.optional => Some(prim),
            _ => None,
        };
        if let Some(prim) = prim_opt
            && let Some(sentinel) = optional_int_sentinel(prim)
        {
            return vec![format!("if ({name}) |v| v else {sentinel}", name = p.name)];
        }
    }
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            vec![format!("{}_z", p.name)]
        }
        TypeRef::Bytes => {
            vec![format!("{}.ptr", p.name), format!("{}.len", p.name)]
        }
        _ => vec![p.name.clone()],
    }
}

/// Produce the Zig expression that converts a raw C return value (`raw`) to the
/// wrapper return type.
///
/// Bool: the C ABI represents `bool` as `i32`; Zig rejects an implicit `i32→bool`
/// coercion, so emit `_result != 0`.
/// String/Char/Path/Json/Vec/Map: copy the C string to an owned Zig slice, then free
/// the FFI allocation via `_free_string`. `Char` takes the same `*mut c_char` +
/// `_len()` convention as `String` on the FFI side, so it shares this arm.
/// Named struct (has_serde): serialize to JSON via `<prefix>_<snake>_to_json`,
/// copy the JSON string to an owned Zig slice, then free both the JSON string and
/// the opaque handle.
/// Everything else: pass through unchanged.
fn unwrap_return_expr(
    raw: &str,
    ty: &TypeRef,
    prefix: &str,
    struct_names: &std::collections::HashSet<String>,
    error_type: Option<&str>,
) -> String {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => format!("{raw} != 0"),
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            crate::backends::zig::template_env::render(
                "return_owned_bytes_block.jinja",
                crate::alef_context! {
                    raw => raw,
                    error_type => error_type,
                },
            )
        }
        TypeRef::Named(name) if struct_names.contains(name) => {
            let snake = c_symbol_component(name);
            crate::backends::zig::template_env::render(
                "return_named_json_block.jinja",
                crate::alef_context! {
                    prefix => prefix,
                    snake => &snake,
                    raw => raw,
                    error_type => error_type,
                },
            )
        }
        TypeRef::Named(name) => {
            let fallback = if error_type.is_some() {
                "error.UnknownFfiError"
            } else {
                "error.OutOfMemory"
            };
            format!("blk: {{ if ({raw} == 0) return {fallback}; break :blk {name}{{ ._handle = {raw} }}; }}")
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                crate::backends::zig::template_env::render(
                    "return_optional_owned_bytes_block.jinja",
                    crate::alef_context! {
                        raw => raw,
                    },
                )
            }
            TypeRef::Named(name) if struct_names.contains(name) => {
                let snake = c_symbol_component(name);
                let inner_block = crate::backends::zig::template_env::render(
                    "return_named_json_block.jinja",
                    crate::alef_context! {
                        prefix => prefix,
                        snake => &snake,
                        raw => raw,
                        error_type => error_type,
                    },
                );
                format!("if ({raw} == 0) null else {inner_block}")
            }
            TypeRef::Named(name) => {
                format!("if ({raw} == 0) null else {name}{{ ._handle = {raw} }}")
            }
            _ => raw.to_string(),
        },
        _ => raw.to_string(),
    }
}

/// Build the Zig return type for a function (not for struct fields).
///
/// Owned string/char/JSON/collection returns are `[]u8` (allocated slice) — `Char`
/// crosses the FFI exactly like `String` (see `returns_c_char` in the FFI backend),
/// so it takes the same owned-slice shape rather than the `[]const u8` a struct
/// field would get.
/// `Bytes` returns are `[]u8` — the FFI uses the out-param convention
/// (`uint8_t **out_ptr, uintptr_t *out_len, uintptr_t *out_cap`) and the
/// wrapper copies the bytes into a caller-owned heap allocation. `Optional<Bytes>`
/// is `?[]u8` (owned, not the `[]const u8` a struct field would get) and rides the
/// same out-param convention with a null `out_ptr` standing for `None`.
/// Named struct returns (opaque C handles) are also serialized to `[]u8` (JSON).
/// Everything else matches the struct-field mapping.
pub(crate) fn zig_return_type(ty: &TypeRef, struct_names: &std::collections::HashSet<String>) -> String {
    match ty {
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Path
        | TypeRef::Json
        | TypeRef::Bytes
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _) => "[]u8".to_string(),
        TypeRef::Named(name) if struct_names.contains(name) => "[]u8".to_string(),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String
            | TypeRef::Char
            | TypeRef::Path
            | TypeRef::Json
            | TypeRef::Bytes
            | TypeRef::Vec(_)
            | TypeRef::Map(_, _) => "?[]u8".to_string(),
            TypeRef::Named(name) if struct_names.contains(name) => "?[]u8".to_string(),
            other => zig_field_type(other, true),
        },
        other => zig_field_type(other, false),
    }
}

#[cfg(test)]
mod capsule_tests;

#[cfg(test)]
mod tests;
