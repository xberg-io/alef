use alef_core::ir::{FunctionDef, ParamDef, PrimitiveType, TypeRef};

use super::errors::resolve_zig_error_type;
use super::helpers::emit_cleaned_zig_doc;
use super::types::zig_field_type;

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

fn snake_case(name: &str) -> String {
    heck::AsSnakeCase(name).to_string()
}

pub(crate) fn emit_function(
    f: &FunctionDef,
    prefix: &str,
    declared_errors: &[String],
    top_level_names: &std::collections::HashSet<String>,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
    out: &mut String,
) {
    emit_cleaned_zig_doc(out, &f.doc, "");

    // Rename param names that would shadow a top-level decl (Zig 0.16+ rejects
    // shadowing of file-scope identifiers by function parameters).
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

    // Build the wrapper-level parameter list (Zig-idiomatic types, not raw C types).
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format_param_wrapper(p, struct_names, opaque_creator_map))
        .collect();

    let zig_error_type = f
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors));

    let return_ty = if let Some(error_type) = &zig_error_type {
        format!(
            "({}||error{{OutOfMemory}})!{}",
            error_type,
            zig_return_type(&f.return_type, struct_names)
        )
    } else {
        zig_return_type(&f.return_type, struct_names)
    };

    out.push_str(&crate::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            func_name => &f.name,
            params => params.join(", "),
            return_ty => &return_ty,
        },
    ));

    // Emit allocation/conversion boilerplate for each parameter.
    for p in &f.params {
        emit_param_conversion(p, prefix, struct_names, opaque_creator_map, out);
    }

    // Build the C argument list.
    let c_args: Vec<String> = f
        .params
        .iter()
        .flat_map(|p| c_arg_names(p, struct_names, opaque_creator_map))
        .collect();
    let c_call = format!("c.{prefix}_{}({})", f.name, c_args.join(", "));

    if let Some(error_type) = &zig_error_type {
        // Fallible function: call C, then check last_error_code(). Zig requires `_`
        // (single underscore) to discard a value; named locals must be used.
        if matches!(f.return_type, TypeRef::Unit) {
            out.push_str(&crate::template_env::render(
                "function_call_unit.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "function_call_result.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
        }
        out.push_str(&crate::template_env::render(
            "function_error_check.jinja",
            minijinja::context! {
                prefix => prefix,
            },
        ));
        out.push_str(&crate::template_env::render(
            "function_error_return.jinja",
            minijinja::context! {
                error_type => error_type,
            },
        ));
        out.push_str("    }\n");

        // Free owned C strings after the error check.
        for p in &f.params {
            emit_param_free(p, prefix, struct_names, opaque_creator_map, out);
        }

        // Produce the Zig return value from `_result`.
        if matches!(f.return_type, TypeRef::Unit) {
            out.push_str("    return;\n");
        } else {
            let ret_expr = unwrap_return_expr("_result", &f.return_type, prefix, struct_names);
            out.push_str(&crate::template_env::render(
                "function_return.jinja",
                minijinja::context! {
                    ret_expr => ret_expr,
                },
            ));
        }
    } else {
        // Infallible function: free params, return directly.
        for p in &f.params {
            emit_param_free(p, prefix, struct_names, opaque_creator_map, out);
        }
        if matches!(f.return_type, TypeRef::Unit) {
            out.push_str(&crate::template_env::render(
                "function_call_unit.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "function_call_result.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
            let ret_expr = unwrap_return_expr("_result", &f.return_type, prefix, struct_names);
            out.push_str(&crate::template_env::render(
                "function_return.jinja",
                minijinja::context! {
                    ret_expr => ret_expr,
                },
            ));
        }
    }

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
    // Opaque handle types accept optional config JSON — always ?[]const u8.
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
    out: &mut String,
) {
    let name = &p.name;

    // Opaque handle: accept ?[]const u8 config JSON, create the handle internally.
    // The creator function is looked up from opaque_creator_map.
    if let Some(opaque_name) = get_opaque_named(&p.ty, opaque_creator_map) {
        if let Some((creator_fn, config_snake)) = opaque_creator_map.get(opaque_name) {
            out.push_str(&crate::template_env::render(
                "param_opaque_config_from_json.jinja",
                minijinja::context! {
                    name => name,
                    prefix => prefix,
                    creator_fn => creator_fn,
                    config_snake => config_snake,
                },
            ));
        }
        return;
    }

    if let Some(inner_name) = struct_named_inner(&p.ty) {
        if struct_names.contains(inner_name) {
            let snake = snake_case(inner_name);
            // Determine if the wrapper-level type is optional (either the
            // outer TypeRef is Optional, or the param itself is marked optional).
            let is_optional = p.optional || matches!(p.ty, TypeRef::Optional(_));
            if is_optional {
                // Allocate `_z` only when caller passed a value, then convert to
                // an opaque handle. When caller passed null, the C handle is null.
                out.push_str(&crate::template_env::render(
                    "param_optional_string_alloc.jinja",
                    minijinja::context! { name => name },
                ));
                out.push_str(&crate::template_env::render(
                    "param_optional_struct_handle.jinja",
                    minijinja::context! {
                        name => name,
                        prefix => prefix,
                        snake => &snake,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "param_string_line1.jinja",
                    minijinja::context! { name => name },
                ));
                out.push_str(&crate::template_env::render(
                    "param_string_line2.jinja",
                    minijinja::context! { name => name },
                ));
                out.push_str(&crate::template_env::render(
                    "param_struct_handle.jinja",
                    minijinja::context! {
                        name => name,
                        prefix => prefix,
                        snake => &snake,
                    },
                ));
            }
            return;
        }
    }
    // Optional `String`/`Path` parameters arrive as `?[]const u8` and cannot
    // be passed straight to `allocPrintSentinel("{s}", ...)` (Zig's writer
    // refuses to format an optional). Emit conditional allocation that maps
    // `null` → `null` and a value → an owned sentinel-terminated copy.
    let is_optional_string = p.optional
        || matches!(
                &p.ty,
                TypeRef::Optional(inner)
                    if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        );
    if is_optional_string && matches!(unwrap_optional(&p.ty), TypeRef::String | TypeRef::Path) {
        out.push_str(&crate::template_env::render(
            "param_optional_string_alloc.jinja",
            minijinja::context! { name => name },
        ));
        return;
    }
    match &p.ty {
        TypeRef::String | TypeRef::Path => {
            out.push_str(&crate::template_env::render(
                "param_string_line1.jinja",
                minijinja::context! {
                    name => name,
                },
            ));
            out.push_str(&crate::template_env::render(
                "param_string_line2.jinja",
                minijinja::context! {
                    name => name,
                },
            ));
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            // Caller supplies JSON bytes; we need a null-terminated C string copy.
            out.push_str("    // Vec/Map parameters are passed as JSON strings across the FFI boundary.\n");
            out.push_str(&crate::template_env::render(
                "param_string_line1.jinja",
                minijinja::context! {
                    name => name,
                },
            ));
            out.push_str(&crate::template_env::render(
                "param_string_line2.jinja",
                minijinja::context! {
                    name => name,
                },
            ));
        }
        _ => {
            // No conversion needed — Bytes uses .ptr/.len directly, primitives pass through.
        }
    }
}

/// Strip a single `Optional<>` layer if present.
fn unwrap_optional(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Optional(inner) => inner,
        other => other,
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
    prefix: &str,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
    out: &mut String,
) {
    let name = &p.name;

    // Opaque handle: free config_z, config_handle, and the handle itself.
    if let Some(opaque_name) = get_opaque_named(&p.ty, opaque_creator_map) {
        if let Some((_, config_snake)) = opaque_creator_map.get(opaque_name) {
            let opaque_snake = snake_case(opaque_name);
            let config_name = format!("{name}_config");
            out.push_str(&crate::template_env::render(
                "param_optional_free.jinja",
                minijinja::context! {
                    name => &config_name,
                },
            ));
            out.push_str(&crate::template_env::render(
                "param_struct_handle_free.jinja",
                minijinja::context! {
                    name => &config_name,
                    prefix => prefix,
                    snake => config_snake,
                },
            ));
            out.push_str(&crate::template_env::render(
                "param_struct_handle_free.jinja",
                minijinja::context! {
                    name => name,
                    prefix => prefix,
                    snake => &opaque_snake,
                },
            ));
        }
        return;
    }

    if let Some(inner_name) = struct_named_inner(&p.ty) {
        if struct_names.contains(inner_name) {
            let snake = snake_case(inner_name);
            let is_optional = p.optional || matches!(p.ty, TypeRef::Optional(_));
            if is_optional {
                // Free both the JSON sentinel copy and the opaque handle, but
                // only if the caller actually supplied a value.
                out.push_str(&crate::template_env::render(
                    "param_optional_free.jinja",
                    minijinja::context! { name => name },
                ));
                out.push_str(&crate::template_env::render(
                    "param_struct_handle_free.jinja",
                    minijinja::context! {
                        name => name,
                        prefix => prefix,
                        snake => &snake,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "param_free.jinja",
                    minijinja::context! { name => name },
                ));
                out.push_str(&crate::template_env::render(
                    "param_struct_handle_free.jinja",
                    minijinja::context! {
                        name => name,
                        prefix => prefix,
                        snake => &snake,
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
        out.push_str(&crate::template_env::render(
            "param_optional_free.jinja",
            minijinja::context! { name => name },
        ));
        return;
    }
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            out.push_str(&crate::template_env::render(
                "param_free.jinja",
                minijinja::context! {
                    name => name,
                },
            ));
        }
        _ => {}
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
    // Opaque handle: the C call gets the handle created in emit_param_conversion.
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
    // Optional integer primitive: substitute the max-value sentinel for None.
    // The Rust FFI layer uses `<Type>::MAX` as the sentinel for Option<T> on
    // numeric primitives (e.g. `u64::MAX` represents `None` for `timeout_secs`).
    // The IR may encode this as either TypeRef::Optional(Primitive) or
    // TypeRef::Primitive with p.optional = true — handle both forms.
    {
        let prim_opt = match &p.ty {
            TypeRef::Optional(inner) => {
                if let TypeRef::Primitive(prim) = inner.as_ref() { Some(prim) } else { None }
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
) -> String {
    match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            // Copy the null-terminated C string to an owned Zig allocation, then free the C copy.
            let mut s = String::new();
            s.push_str("blk: {\n");
            s.push_str(&crate::template_env::render(
                "return_unwrap_slice.jinja",
                minijinja::context! {
                    raw => raw,
                },
            ));
            s.push_str("        const owned = try std.heap.c_allocator.dupe(u8, slice);\n");
            s.push_str(&crate::template_env::render(
                "return_unwrap_free.jinja",
                minijinja::context! {
                    raw => raw,
                },
            ));
            s.push_str("        break :blk owned;\n");
            s.push_str("    }");
            s
        }
        TypeRef::Named(name) if struct_names.contains(name) => {
            // The C function returned an opaque handle (*KREUZBERGFoo). Serialize
            // it to JSON via the FFI `<prefix>_<snake>_to_json` helper, copy the
            // JSON string into a Zig-owned buffer, then free both the JSON string
            // and the opaque handle. The wrapper returns `[]u8` (JSON).
            let snake = snake_case(name);
            crate::template_env::render(
                "return_named_json_block.jinja",
                minijinja::context! {
                    prefix => prefix,
                    snake => &snake,
                    raw => raw,
                },
            )
        }
        TypeRef::Named(name) => {
            // Opaque handle type (no serde): unwrap the nullable C pointer (guaranteed
            // non-null after the error-code check above) and wrap in the Zig struct.
            format!("{name}{{ ._handle = {raw}.? }}")
        }
        _ => raw.to_string(),
    }
}

/// Build the Zig return type for a function (not for struct fields).
///
/// Owned string/JSON/collection returns are `[]u8` (allocated slice).
/// Named struct returns (opaque C handles) are also serialized to `[]u8` (JSON).
/// Everything else matches the struct-field mapping.
pub(crate) fn zig_return_type(ty: &TypeRef, struct_names: &std::collections::HashSet<String>) -> String {
    match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => "[]u8".to_string(),
        TypeRef::Named(name) if struct_names.contains(name) => "[]u8".to_string(),
        other => zig_field_type(other, false),
    }
}
