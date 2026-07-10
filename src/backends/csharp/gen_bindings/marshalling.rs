use crate::codegen::naming::csharp_type_name;
use crate::core::ir::{PrimitiveType, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::HashSet;

/// Returns the C# type to use in a `[DllImport]` declaration for the given return type.
///
/// Key differences from the high-level `csharp_type`:
/// - Bool is marshalled as `int` (C FFI convention) — the wrapper compares != 0.
/// - String / Named / Vec / Map / Path / Json / Bytes all come back as `IntPtr`.
/// - Numeric primitives use their natural C# types (`nuint`, `int`, etc.).
pub(super) fn pinvoke_return_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Unit => "void",
        TypeRef::Primitive(PrimitiveType::Bool) => "int",
        TypeRef::Primitive(PrimitiveType::U8) => "byte",
        TypeRef::Primitive(PrimitiveType::U16) => "ushort",
        TypeRef::Primitive(PrimitiveType::U32) => "uint",
        TypeRef::Primitive(PrimitiveType::U64) => "ulong",
        TypeRef::Primitive(PrimitiveType::I8) => "sbyte",
        TypeRef::Primitive(PrimitiveType::I16) => "short",
        TypeRef::Primitive(PrimitiveType::I32) => "int",
        TypeRef::Primitive(PrimitiveType::I64) => "long",
        TypeRef::Primitive(PrimitiveType::F32) => "float",
        TypeRef::Primitive(PrimitiveType::F64) => "double",
        TypeRef::Primitive(PrimitiveType::Usize) => "ulong",
        TypeRef::Primitive(PrimitiveType::Isize) => "long",
        TypeRef::Duration => "ulong",
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Optional(_)
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _)
        | TypeRef::Named(_)
        | TypeRef::Path
        | TypeRef::Json => "IntPtr",
    }
}

/// Returns the C# type to use for a parameter in a `[DllImport]` declaration.
///
/// Managed reference types (Named structs, Vec, Map, Bytes, Optional of Named, etc.)
/// cannot be directly marshalled by P/Invoke.  They must be passed as `IntPtr` (opaque
/// handle or JSON-string pointer).  Primitive types and plain strings use their natural
/// types.
pub(super) fn pinvoke_param_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "string",
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes | TypeRef::Optional(_) => "IntPtr",
        TypeRef::Unit => "void",
        TypeRef::Primitive(PrimitiveType::Bool) => "int",
        TypeRef::Primitive(PrimitiveType::U8) => "byte",
        TypeRef::Primitive(PrimitiveType::U16) => "ushort",
        TypeRef::Primitive(PrimitiveType::U32) => "uint",
        TypeRef::Primitive(PrimitiveType::U64) => "ulong",
        TypeRef::Primitive(PrimitiveType::I8) => "sbyte",
        TypeRef::Primitive(PrimitiveType::I16) => "short",
        TypeRef::Primitive(PrimitiveType::I32) => "int",
        TypeRef::Primitive(PrimitiveType::I64) => "long",
        TypeRef::Primitive(PrimitiveType::F32) => "float",
        TypeRef::Primitive(PrimitiveType::F64) => "double",
        TypeRef::Primitive(PrimitiveType::Usize) => "ulong",
        TypeRef::Primitive(PrimitiveType::Isize) => "long",
        TypeRef::Duration => "ulong",
    }
}

/// Returns true if a parameter should be hidden from the public API because it is a
/// trait-bridge param (e.g. the FFI visitor handle).
pub(super) fn is_bridge_param(
    param: &crate::core::ir::ParamDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> bool {
    bridge_param_names.contains(&param.name)
        || matches!(&param.ty, crate::core::ir::TypeRef::Named(n) if bridge_type_aliases.contains(n))
}

/// Does the return type need IntPtr→string marshalling in the wrapper?
pub(super) fn returns_string(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json)
}

/// Does the return type come back as a C int that should be converted to bool?
pub(super) fn returns_bool_via_int(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Primitive(PrimitiveType::Bool))
}

/// Does the return type need JSON deserialization from an IntPtr string?
pub(super) fn returns_json_object(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_) | TypeRef::Bytes | TypeRef::Optional(_)
    )
}

/// Returns true if the FFI return type is a pointer (IntPtr), as opposed to a numeric value.
/// Only pointer-returning functions use `IntPtr.Zero` as an error sentinel.
pub(super) fn returns_ptr(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::String
            | TypeRef::Char
            | TypeRef::Path
            | TypeRef::Json
            | TypeRef::Named(_)
            | TypeRef::Vec(_)
            | TypeRef::Map(_, _)
            | TypeRef::Bytes
            | TypeRef::Optional(_)
    )
}

/// Returns the argument expression to pass to the native method for a given parameter.
///
/// For truly opaque types (is_opaque = true), the C# class wraps an IntPtr; pass `.Handle`.
/// For data-struct `Named` types this is the handle variable (e.g. `optionsHandle`).
/// For everything else it is the parameter name (with `!` for optional).
pub(super) fn native_call_arg(
    ty: &TypeRef,
    param_name: &str,
    optional: bool,
    true_opaque_types: &HashSet<String>,
) -> String {
    match ty {
        TypeRef::Named(type_name) if true_opaque_types.contains(type_name) => {
            let bang = if optional { "!" } else { "" };
            format!("{param_name}{bang}.Handle")
        }
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            format!("{param_name}Handle")
        }
        TypeRef::Bytes => {
            format!("{param_name}Handle.AddrOfPinnedObject()")
        }
        TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => {
            if optional {
                format!("({param_name} ?? false)")
            } else {
                param_name.to_string()
            }
        }
        ty => {
            if optional {
                if let TypeRef::Primitive(prim) = ty {
                    use crate::core::ir::PrimitiveType;
                    let sentinel = match prim {
                        PrimitiveType::U8 => "byte.MaxValue",
                        PrimitiveType::U16 => "ushort.MaxValue",
                        PrimitiveType::U32 => "uint.MaxValue",
                        PrimitiveType::U64 | PrimitiveType::Usize => "ulong.MaxValue",
                        PrimitiveType::I8 => "sbyte.MaxValue",
                        PrimitiveType::I16 => "short.MaxValue",
                        PrimitiveType::I32 => "int.MaxValue",
                        PrimitiveType::I64 | PrimitiveType::Isize => "long.MaxValue",
                        PrimitiveType::F32 => "float.NaN",
                        PrimitiveType::F64 => "double.NaN",
                        PrimitiveType::Bool => unreachable!("handled above"),
                    };
                    format!("{param_name} ?? {sentinel}")
                } else if matches!(ty, TypeRef::Duration) {
                    format!("{param_name}.GetValueOrDefault()")
                } else {
                    format!("{param_name}!")
                }
            } else {
                param_name.to_string()
            }
        }
    }
}

/// Build the byte-slice length argument passed to a native call.
///
/// `cast` is the C# cast prefix the P/Invoke length parameter expects (e.g. `"(UIntPtr)"`
/// or `"(nuint)"`). For optional `byte[]?` parameters the array may be null — pinning a
/// null array yields `IntPtr.Zero` and a zero length, so we null-coalesce the length to
/// `0` rather than dereferencing `.Length` (which would trip CS8602 under
/// `<TreatWarningsAsErrors>` / nullable-reference analysis).
pub(super) fn bytes_len_arg(cast: &str, param_name: &str, optional: bool) -> String {
    if optional {
        format!("{cast}({param_name}?.Length ?? 0)")
    } else {
        format!("{cast}{param_name}.Length")
    }
}

/// Returns true when wrapper setup allocates a temporary handle that must be
/// released after the native call.
pub(super) fn needs_param_teardown(
    params: &[crate::core::ir::ParamDef],
    true_opaque_types: &HashSet<String>,
    enum_names: &HashSet<String>,
) -> bool {
    params.iter().any(|param| match &param.ty {
        TypeRef::Named(type_name) => !true_opaque_types.contains(type_name) && !enum_names.contains(type_name),
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes => true,
        _ => false,
    })
}

/// For each `Named` parameter, emit code to serialise it to JSON and obtain a native handle.
///
/// For truly opaque types (is_opaque = true), the C# class already wraps the native handle, so
/// we pass `param.Handle` directly without any JSON serialisation.
pub(super) fn emit_named_param_setup(
    out: &mut String,
    params: &[crate::core::ir::ParamDef],
    indent: &str,
    true_opaque_types: &HashSet<String>,
    exception_name: &str,
    _types: &[TypeDef],
    enum_names: &HashSet<String>,
) {
    use crate::backends::csharp::template_env::render;

    for param in params {
        let param_name = param.name.to_lower_camel_case();
        let json_var = format!("{param_name}Json");
        let handle_var = format!("{param_name}Handle");

        match &param.ty {
            TypeRef::Named(type_name) => {
                if true_opaque_types.contains(type_name) {
                    continue;
                }
                if enum_names.contains(type_name) {
                    if param.optional {
                        out.push_str(&render(
                            "named_param_enum_optional.jinja",
                            minijinja::context! { indent, handle_var, param_name },
                        ));
                    } else {
                        out.push_str(&render(
                            "named_param_enum_required.jinja",
                            minijinja::context! { indent, handle_var, param_name },
                        ));
                    }
                    continue;
                }
                let from_json_method = format!("{}FromJson", csharp_type_name(type_name));

                if param.optional {
                    out.push_str(&crate::backends::csharp::template_env::render(
                        "named_param_handle_from_json_optional.jinja",
                        minijinja::context! {
                            indent,
                            handle_var => &handle_var,
                            from_json_method => &from_json_method,
                            json_var => &json_var,
                            param_name => &param_name,
                            exception_name => exception_name,
                        },
                    ));
                } else {
                    out.push_str(&crate::backends::csharp::template_env::render(
                        "named_param_json_serialize.jinja",
                        minijinja::context! { indent, json_var => &json_var, param_name => &param_name },
                    ));
                    out.push_str(&crate::backends::csharp::template_env::render(
                        "named_param_handle_from_json.jinja",
                        minijinja::context! {
                            indent,
                            handle_var => &handle_var,
                            from_json_method => &from_json_method,
                            json_var => &json_var,
                            exception_name => exception_name,
                        },
                    ));
                }
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_json_serialize.jinja",
                    minijinja::context! { indent, json_var => &json_var, param_name => &param_name },
                ));
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_handle_string.jinja",
                    minijinja::context! { indent, handle_var => &handle_var, json_var => &json_var },
                ));
            }
            TypeRef::Bytes => {
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_handle_pin.jinja",
                    minijinja::context! { indent, handle_var => &handle_var, param_name => &param_name },
                ));
            }
            _ => {}
        }
    }
}

/// Emit cleanup code to free native handles allocated for `Named` parameters.
///
/// Truly opaque handles (is_opaque = true) are NOT freed here — their lifetime is managed by
/// the C# wrapper class (IDisposable). Only data-struct handles (from_json-allocated) are freed.
/// Enums are not freed (they are stack values, not heap-allocated).
pub(super) fn emit_named_param_teardown(
    out: &mut String,
    params: &[crate::core::ir::ParamDef],
    true_opaque_types: &HashSet<String>,
    enum_names: &HashSet<String>,
) {
    for param in params {
        let param_name = param.name.to_lower_camel_case();
        let handle_var = format!("{param_name}Handle");
        match &param.ty {
            TypeRef::Named(type_name) => {
                if true_opaque_types.contains(type_name) {
                    continue;
                }
                if enum_names.contains(type_name) {
                    continue;
                }
                let free_method = format!("{}Free", csharp_type_name(type_name));
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_teardown_free.jinja",
                    minijinja::context! { indent => "        ", free_method => &free_method, handle_var => &handle_var },
                ));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_teardown_hglobal.jinja",
                    minijinja::context! { indent => "        ", handle_var => &handle_var },
                ));
            }
            TypeRef::Bytes => {
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_teardown_gchandle.jinja",
                    minijinja::context! { indent => "        ", handle_var => &handle_var },
                ));
            }
            _ => {}
        }
    }
}

/// Emit cleanup code with configurable indentation (used inside `Task.Run` lambdas).
pub(super) fn emit_named_param_teardown_indented(
    out: &mut String,
    params: &[crate::core::ir::ParamDef],
    indent: &str,
    true_opaque_types: &HashSet<String>,
    enum_names: &HashSet<String>,
) {
    for param in params {
        let param_name = param.name.to_lower_camel_case();
        let handle_var = format!("{param_name}Handle");
        match &param.ty {
            TypeRef::Named(type_name) => {
                if true_opaque_types.contains(type_name) {
                    continue;
                }
                if enum_names.contains(type_name) {
                    continue;
                }
                let free_method = format!("{}Free", csharp_type_name(type_name));
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_teardown_free.jinja",
                    minijinja::context! { indent, free_method => &free_method, handle_var => &handle_var },
                ));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_teardown_hglobal.jinja",
                    minijinja::context! { indent, handle_var => &handle_var },
                ));
            }
            TypeRef::Bytes => {
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_teardown_gchandle.jinja",
                    minijinja::context! { indent, handle_var => &handle_var },
                ));
            }
            _ => {}
        }
    }
}
