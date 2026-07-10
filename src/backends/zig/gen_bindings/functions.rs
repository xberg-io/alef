use crate::core::ir::{FunctionDef, ParamDef, PrimitiveType, TypeRef};

use super::errors::resolve_zig_error_type;
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

/// Returns true if `ty` can be null. Pointer-like types (String, Path, Json, Bytes,
/// Vec, Map, Named structs, and Optional<T>) can be null. Primitives cannot be null.
fn return_type_can_be_null(ty: &TypeRef, struct_names: &std::collections::HashSet<String>) -> bool {
    match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Bytes | TypeRef::Vec(_) | TypeRef::Map(_, _) => true,
        TypeRef::Named(name) => struct_names.contains(name),
        TypeRef::Optional(_) => true,
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_function(
    f: &FunctionDef,
    prefix: &str,
    declared_errors: &[String],
    top_level_names: &std::collections::HashSet<String>,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    out: &mut String,
) {
    if let TypeRef::Named(name) = &f.return_type {
        if let Some(cap) = capsule_types.get(name.as_str()) {
            emit_capsule_function(f, prefix, struct_names, opaque_creator_map, cap, declared_errors, out);
            return;
        }
    }

    emit_cleaned_zig_doc(out, &f.doc, "");

    let renamed_params: Vec<ParamDef> = f
        .params
        .iter()
        .map(|p| {
            let mut p2 = p.clone();
            if top_level_names.contains(&p2.name) {
                p2.name = format!("{}_arg", p2.name);
            }
            p2
        })
        .collect();
    let f_local = FunctionDef {
        params: renamed_params,
        ..f.clone()
    };
    let f = &f_local;

    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format_param_wrapper(p, struct_names, opaque_creator_map))
        .collect();

    let zig_error_type = f
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors));

    let body_needs_try = f.params.iter().any(needs_alloc_param)
        || matches!(
            &f.return_type,
            TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Bytes | TypeRef::Vec(_) | TypeRef::Map(_, _)
        )
        || matches!(&f.return_type, TypeRef::Named(name) if struct_names.contains(name));
    let body_needs_invalid_json = f
        .params
        .iter()
        .any(|p| needs_from_json_param(p, struct_names, opaque_creator_map));

    let return_ty = if let Some(error_type) = &zig_error_type {
        format!("{}!{}", error_type, zig_return_type(&f.return_type, struct_names))
    } else if body_needs_try || body_needs_invalid_json {
        let err_set = if body_needs_invalid_json {
            "error{OutOfMemory,InvalidJson}"
        } else {
            "error{OutOfMemory}"
        };
        format!("{err_set}!{}", zig_return_type(&f.return_type, struct_names))
    } else {
        zig_return_type(&f.return_type, struct_names)
    };

    out.push_str(&crate::backends::zig::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            func_name => &f.name,
            params => params.join(", "),
            return_ty => &return_ty,
        },
    ));

    let json_error_return = zig_error_type
        .as_ref()
        .map_or("return error.InvalidJson;".to_string(), |err| {
            format!("return _error_with_message({err});")
        });
    for p in &f.params {
        emit_param_conversion(p, prefix, struct_names, opaque_creator_map, &json_error_return, out);
    }

    let returns_bytes = matches!(f.return_type, TypeRef::Bytes);
    if returns_bytes {
        out.push_str("    var _out_ptr: [*c]u8 = undefined;\n");
        out.push_str("    var _out_len: usize = 0;\n");
        out.push_str("    var _out_cap: usize = 0;\n");
    }

    let mut c_args: Vec<String> = f
        .params
        .iter()
        .flat_map(|p| c_arg_names(p, struct_names, opaque_creator_map))
        .collect();
    if returns_bytes {
        c_args.push("&_out_ptr".to_string());
        c_args.push("&_out_len".to_string());
        c_args.push("&_out_cap".to_string());
    }
    let c_call = format!("c.{prefix}_{}({})", f.name, c_args.join(", "));
    let returns_c_char_like = return_uses_len_companion(&f.return_type);
    let c_len_call = if returns_c_char_like {
        Some(format!("c.{prefix}_{}_len({})", f.name, c_args.join(", ")))
    } else {
        None
    };

    if let Some(error_type) = &zig_error_type {
        let result_is_pointer = !(matches!(f.return_type, TypeRef::Unit) || returns_bytes);
        let result_can_be_null = return_type_can_be_null(&f.return_type, struct_names);
        if !result_is_pointer {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_unit.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
        } else {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_result.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
        }
        if result_is_pointer {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_error_check.jinja",
                minijinja::context! {
                    prefix => prefix,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "function_error_return.jinja",
                minijinja::context! {
                    error_type => error_type,
                },
            ));
            out.push_str("    }\n");
            if result_can_be_null {
                out.push_str("    if (_result == null) {\n");
                out.push_str(&crate::backends::zig::template_env::render(
                    "function_error_return.jinja",
                    minijinja::context! {
                        error_type => error_type,
                    },
                ));
                out.push_str("    }\n");
            }
        } else {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_error_check.jinja",
                minijinja::context! {
                    prefix => prefix,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "function_error_return.jinja",
                minijinja::context! {
                    error_type => error_type,
                },
            ));
            out.push_str("    }\n");
        }
        if let Some(len_call) = &c_len_call {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_result_len.jinja",
                minijinja::context! {
                    len_call => len_call,
                },
            ));
        }

        for p in &f.params {
            emit_param_free(p, prefix, struct_names, opaque_creator_map, out);
        }

        if returns_bytes {
            out.push_str("    const _owned = try std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len]);\n");
            out.push_str(&crate::backends::zig::template_env::render(
                "function_free_bytes.jinja",
                minijinja::context! {
                    prefix => prefix,
                },
            ));
            out.push_str("    return _owned;\n");
        } else if matches!(f.return_type, TypeRef::Unit) {
            out.push_str("    return;\n");
        } else {
            let ret_expr = unwrap_return_expr(
                "_result",
                &f.return_type,
                prefix,
                struct_names,
                Some(error_type.as_str()),
            );
            out.push_str(&crate::backends::zig::template_env::render(
                "function_return.jinja",
                minijinja::context! {
                    ret_expr => ret_expr,
                },
            ));
        }
    } else {
        for p in &f.params {
            emit_param_free(p, prefix, struct_names, opaque_creator_map, out);
        }
        if returns_bytes {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_unit.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
            out.push_str("    const _owned = try std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len]);\n");
            out.push_str(&crate::backends::zig::template_env::render(
                "function_free_bytes.jinja",
                minijinja::context! {
                    prefix => prefix,
                },
            ));
            out.push_str("    return _owned;\n");
        } else if matches!(f.return_type, TypeRef::Unit) {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_unit.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
        } else {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_result.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
            if let Some(len_call) = &c_len_call {
                out.push_str(&crate::backends::zig::template_env::render(
                    "function_result_len.jinja",
                    minijinja::context! {
                        len_call => len_call,
                    },
                ));
            }
            let ret_expr = unwrap_return_expr("_result", &f.return_type, prefix, struct_names, None);
            out.push_str(&crate::backends::zig::template_env::render(
                "function_return.jinja",
                minijinja::context! {
                    ret_expr => ret_expr,
                },
            ));
        }
    }

    out.push_str("}\n");
}

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
        minijinja::context! {
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
        minijinja::context! {
            c_call => &c_call,
        },
    ));

    for p in &f.params {
        emit_param_free(p, prefix, struct_names, opaque_creator_map, out);
    }

    if let Some(error_type) = &zig_error_type {
        out.push_str(&crate::backends::zig::template_env::render(
            "function_error_check.jinja",
            minijinja::context! { prefix => prefix },
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "function_error_return.jinja",
            minijinja::context! { error_type => error_type },
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

/// Zig type used at the wrapper boundary for a function parameter.
///
/// - `String`, `Path` → `[]const u8`  (body allocates null-terminated copy)
/// - `Bytes`          → `[]const u8`  (body passes `.ptr` + `.len`)
/// - `Vec`, `Map`     → `[]const u8`  (caller supplies JSON; body passes as C string)
/// - `Named` struct   → `[]const u8`  (caller supplies JSON; body converts to opaque
///   handle via the FFI `<prefix>_<snake>_from_json` helper)
/// - Everything else  → same as struct-field type
fn zig_param_type(
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
                minijinja::context! {
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

    if let Some(inner_name) = struct_named_inner(&p.ty) {
        if struct_names.contains(inner_name) {
            let snake = c_symbol_component(inner_name);
            let is_optional = p.optional || matches!(p.ty, TypeRef::Optional(_));
            if is_optional {
                out.push_str(&crate::backends::zig::template_env::render(
                    "param_optional_string_alloc.jinja",
                    minijinja::context! { name => name },
                ));
                out.push_str(&crate::backends::zig::template_env::render(
                    "param_optional_struct_handle.jinja",
                    minijinja::context! {
                        name => name,
                        prefix => prefix,
                        snake => &snake,
                        json_error_return => json_error_return,
                    },
                ));
            } else {
                out.push_str(&crate::backends::zig::template_env::render(
                    "param_string_line1.jinja",
                    minijinja::context! { name => name },
                ));
                out.push_str(&crate::backends::zig::template_env::render(
                    "param_string_line2.jinja",
                    minijinja::context! { name => name },
                ));
                out.push_str(&crate::backends::zig::template_env::render(
                    "param_struct_handle.jinja",
                    minijinja::context! {
                        name => name,
                        prefix => prefix,
                        snake => &snake,
                        json_error_return => json_error_return,
                    },
                ));
            }
            return;
        }
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
            minijinja::context! { name => name },
        ));
        return;
    }
    match &p.ty {
        TypeRef::String | TypeRef::Path => {
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line1.jinja",
                minijinja::context! {
                    name => name,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line2.jinja",
                minijinja::context! {
                    name => name,
                },
            ));
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            out.push_str("    // Vec/Map parameters are passed as JSON strings across the FFI boundary.\n");
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line1.jinja",
                minijinja::context! {
                    name => name,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line2.jinja",
                minijinja::context! {
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

/// Returns true when a return type maps to `*mut c_char` and therefore has a
/// matching `_len()` companion in alef-backend-ffi.
///
/// Must mirror `crate::backends::ffi::gen_bindings::functions::returns_c_char`.
pub(crate) fn return_uses_len_companion(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json => true,
        TypeRef::Vec(_) | TypeRef::Map(_, _) => true,
        TypeRef::Optional(inner) => matches!(
            inner.as_ref(),
            TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _)
        ),
        _ => false,
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
fn emit_param_free(
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
                minijinja::context! {
                    name => &config_name,
                },
            ));
            let _ = config_snake;
        }
        return;
    }

    if let Some(inner_name) = struct_named_inner(&p.ty) {
        if struct_names.contains(inner_name) {
            let _ = inner_name;
        }
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
        if let Some(prim) = prim_opt {
            if let Some(sentinel) = optional_int_sentinel(prim) {
                return vec![format!("if ({name}) |v| v else {sentinel}", name = p.name)];
            }
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
/// String/Path/Json/Vec/Map: copy the C string to an owned Zig slice, then free
/// the FFI allocation via `_free_string`.
/// Named struct (has_serde): serialize to JSON via `<prefix>_<snake>_to_json`,
/// copy the JSON string to an owned Zig slice, then free both the JSON string and
/// the opaque handle.
/// Named opaque handle (not in struct_names): wrap the raw C pointer in the Zig
/// struct wrapper as `TypeName{ ._handle = raw }`.
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
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            crate::backends::zig::template_env::render(
                "return_owned_bytes_block.jinja",
                minijinja::context! {
                    raw => raw,
                    error_type => error_type,
                },
            )
        }
        TypeRef::Named(name) if struct_names.contains(name) => {
            let snake = c_symbol_component(name);
            crate::backends::zig::template_env::render(
                "return_named_json_block.jinja",
                minijinja::context! {
                    prefix => prefix,
                    snake => &snake,
                    raw => raw,
                    error_type => error_type,
                },
            )
        }
        TypeRef::Named(name) => {
            format!("{name}{{ ._handle = {raw}.? }}")
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                crate::backends::zig::template_env::render(
                    "return_optional_owned_bytes_block.jinja",
                    minijinja::context! {
                        raw => raw,
                    },
                )
            }
            TypeRef::Named(name) if struct_names.contains(name) => {
                let snake = c_symbol_component(name);
                let inner_block = crate::backends::zig::template_env::render(
                    "return_named_json_block.jinja",
                    minijinja::context! {
                        prefix => prefix,
                        snake => &snake,
                        raw => raw,
                        error_type => error_type,
                    },
                );
                format!("if ({raw} == null) null else {inner_block}")
            }
            _ => raw.to_string(),
        },
        _ => raw.to_string(),
    }
}

/// Build the Zig return type for a function (not for struct fields).
///
/// Owned string/JSON/collection returns are `[]u8` (allocated slice).
/// `Bytes` returns are `[]u8` — the FFI uses the out-param convention
/// (`uint8_t **out_ptr, uintptr_t *out_len, uintptr_t *out_cap`) and the
/// wrapper copies the bytes into a caller-owned heap allocation.
/// Named struct returns (opaque C handles) are also serialized to `[]u8` (JSON).
/// Everything else matches the struct-field mapping.
pub(crate) fn zig_return_type(ty: &TypeRef, struct_names: &std::collections::HashSet<String>) -> String {
    match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Bytes | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "[]u8".to_string()
        }
        TypeRef::Named(name) if struct_names.contains(name) => "[]u8".to_string(),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                "?[]u8".to_string()
            }
            TypeRef::Named(name) if struct_names.contains(name) => "?[]u8".to_string(),
            other => zig_field_type(other, true),
        },
        other => zig_field_type(other, false),
    }
}

#[cfg(test)]
mod capsule_tests;
