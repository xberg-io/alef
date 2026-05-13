use crate::type_map::java_ffi_type;
use ahash::AHashSet;
use alef_core::ir::{FunctionDef, MethodDef, PrimitiveType, TypeRef};
use heck::ToSnakeCase;

/// Returns true when the function's Rust return type is `Result<Vec<u8>>` (or
/// `Result<Option<Vec<u8>>>`). The FFI layer emits these as the out-param
/// convention: `(inputs..., out_ptr: *mut *mut u8, out_len: *mut usize,
/// out_cap: *mut usize) -> i32`.
pub(crate) fn is_bytes_result(func: &FunctionDef) -> bool {
    if func.error_type.is_none() {
        return false;
    }
    match &func.return_type {
        TypeRef::Bytes => true,
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Bytes),
        _ => false,
    }
}

/// Same detection for methods on opaque types.
/// Reserved for future Java opaque-type method dispatch.
#[allow(dead_code)]
pub(crate) fn is_bytes_result_method(method: &MethodDef) -> bool {
    if method.error_type.is_none() {
        return false;
    }
    match &method.return_type {
        TypeRef::Bytes => true,
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Bytes),
        _ => false,
    }
}

/// Check if the return type is a string-like type that requires pointer-based
/// FFI return handling (allocate + free pattern). `Optional<String>` and
/// `Optional<Path>` reduce to a nullable pointer with the same handling — the
/// boxed Java type is also `String`/`Path`, so the wrapper signature is
/// unchanged from the non-optional case.
pub(crate) fn is_ffi_string_return(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => true,
        TypeRef::Optional(inner) => matches!(
            inner.as_ref(),
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
        ),
        _ => false,
    }
}

/// Return the Java cast expression for a primitive FFI return type.
pub(crate) fn java_ffi_return_cast(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Primitive(prim) => match prim {
            PrimitiveType::Bool => "boolean",
            PrimitiveType::U8 | PrimitiveType::I8 => "byte",
            PrimitiveType::U16 | PrimitiveType::I16 => "short",
            PrimitiveType::U32 | PrimitiveType::I32 => "int",
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => "long",
            PrimitiveType::F32 => "float",
            PrimitiveType::F64 => "double",
        },
        TypeRef::Duration => "long",
        _ => "MemorySegment",
    }
}

pub(crate) fn gen_ffi_layout(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(prim) => java_ffi_type(prim).to_string(),
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Bytes => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Optional(inner) => gen_ffi_layout(inner),
        TypeRef::Vec(_) => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Map(_, _) => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Named(_) => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Unit => "".to_string(),
        TypeRef::Duration => "ValueLayout.JAVA_LONG".to_string(),
    }
}

pub(crate) fn marshal_param_to_ffi(
    out: &mut String,
    name: &str,
    ty: &TypeRef,
    opaque_types: &AHashSet<String>,
    prefix: &str,
) {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Json => {
            let cname = "c".to_string() + name;
            out.push_str(&crate::template_env::render(
                "marshal_string.jinja",
                minijinja::context! {
                    cname => &cname,
                    name => name,
                },
            ));
        }
        TypeRef::Path => {
            let cname = "c".to_string() + name;
            out.push_str(&crate::template_env::render(
                "marshal_path.jinja",
                minijinja::context! {
                    cname => &cname,
                    name => name,
                },
            ));
        }
        TypeRef::Bytes => {
            let cname = "c".to_string() + name;
            out.push_str(&crate::template_env::render(
                "marshal_bytes.jinja",
                minijinja::context! {
                    cname => &cname,
                    name => name,
                },
            ));
        }
        TypeRef::Named(type_name) => {
            let cname = "c".to_string() + name;
            if opaque_types.contains(type_name.as_str()) {
                out.push_str(&crate::template_env::render(
                    "marshal_opaque_handle.jinja",
                    minijinja::context! {
                        cname => &cname,
                        name => name,
                    },
                ));
            } else {
                let type_snake = type_name.to_snake_case();
                let from_json_handle = format!(
                    "NativeLib.{}_{}_FROM_JSON",
                    prefix.to_uppercase(),
                    type_snake.to_uppercase()
                );
                out.push_str(&crate::template_env::render(
                    "marshal_named_type.jinja",
                    minijinja::context! {
                        cname => &cname,
                        name => name,
                        from_json_handle => &from_json_handle,
                    },
                ));
            }
        }
        TypeRef::Optional(inner) => {
            match inner.as_ref() {
                TypeRef::String | TypeRef::Char | TypeRef::Json => {
                    let cname = "c".to_string() + name;
                    out.push_str(&crate::template_env::render(
                        "marshal_optional_string.jinja",
                        minijinja::context! {
                            cname => &cname,
                            name => name,
                        },
                    ));
                }
                TypeRef::Path => {
                    let cname = "c".to_string() + name;
                    out.push_str(&crate::template_env::render(
                        "marshal_optional_path.jinja",
                        minijinja::context! {
                            cname => &cname,
                            name => name,
                        },
                    ));
                }
                TypeRef::Bytes => {
                    let cname = "c".to_string() + name;
                    out.push_str(&crate::template_env::render(
                        "marshal_optional_bytes.jinja",
                        minijinja::context! {
                            cname => &cname,
                            name => name,
                        },
                    ));
                }
                TypeRef::Named(type_name) => {
                    let cname = "c".to_string() + name;
                    if opaque_types.contains(type_name.as_str()) {
                        out.push_str(&crate::template_env::render(
                            "marshal_optional_opaque_handle.jinja",
                            minijinja::context! {
                                cname => &cname,
                                name => name,
                            },
                        ));
                    } else {
                        let type_snake = type_name.to_snake_case();
                        let from_json_handle = format!(
                            "NativeLib.{}_{}_FROM_JSON",
                            prefix.to_uppercase(),
                            type_snake.to_uppercase()
                        );
                        out.push_str(&crate::template_env::render(
                            "marshal_optional_named_type.jinja",
                            minijinja::context! {
                                cname => &cname,
                                name => name,
                                from_json_handle => &from_json_handle,
                            },
                        ));
                    }
                }
                // Optional primitive numeric types: Java auto-unboxes null Long/Integer/etc.
                // via `.intValue()`/`.longValue()` when passed to MethodHandle.invoke(...),
                // which throws NullPointerException. Emit an explicit null → sentinel
                // coercion local so the FFI call always receives a valid primitive AND the
                // FFI shim's `if x == {prim}::MAX { None }` decoder recognises a null caller.
                // Sentinel choice mirrors `alef-backend-ffi` `param_optional_numeric_conversion`:
                // unsigned ints use bitwise -1 (truncates to all-bits-set = u{N}::MAX);
                // signed ints use the Java boxed type's MAX_VALUE; floats use NaN.
                TypeRef::Primitive(prim) => {
                    use alef_core::ir::PrimitiveType;
                    let cname = "c".to_string() + name;
                    let (prim_kw, none_lit) = match prim {
                        PrimitiveType::U64 | PrimitiveType::Usize => ("long", "-1L"),
                        PrimitiveType::I64 | PrimitiveType::Isize => ("long", "Long.MAX_VALUE"),
                        PrimitiveType::U32 => ("int", "-1"),
                        PrimitiveType::I32 => ("int", "Integer.MAX_VALUE"),
                        PrimitiveType::U16 => ("short", "(short) -1"),
                        PrimitiveType::I16 => ("short", "Short.MAX_VALUE"),
                        PrimitiveType::U8 => ("byte", "(byte) -1"),
                        PrimitiveType::I8 => ("byte", "Byte.MAX_VALUE"),
                        PrimitiveType::F32 => ("float", "Float.NaN"),
                        PrimitiveType::F64 => ("double", "Double.NaN"),
                        PrimitiveType::Bool => ("boolean", "false"),
                    };
                    out.push_str(&crate::template_env::render(
                        "marshal_optional_primitive.jinja",
                        minijinja::context! {
                            cname => &cname,
                            name => name,
                            prim_kw => prim_kw,
                            none_lit => none_lit,
                        },
                    ));
                }
                _ => {
                    // Other optional types pass through
                }
            }
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            let cname = "c".to_string() + name;
            out.push_str(&crate::template_env::render(
                "marshal_vec_map.jinja",
                minijinja::context! {
                    cname => &cname,
                    name => name,
                },
            ));
        }
        _ => {
            // Primitives and others pass through directly
        }
    }
}

/// Generate the FFI argument(s) for a parameter.
///
/// Most parameters map to a single FFI argument. However, some expand to multiple:
/// - `Bytes` expands to (pointer, length)
/// - Bytes input parameters in bytes-result functions similarly expand
///
/// Returns a vector of argument expressions to be passed to MethodHandle.invoke().
pub(crate) fn ffi_param_args(name: &str, ty: &TypeRef, _opaque_types: &AHashSet<String>) -> Vec<String> {
    match ty {
        TypeRef::Bytes => {
            // Bytes expands to pointer + length pair
            let cname = "c".to_string() + name;
            vec![cname.clone(), format!("{}Len", cname)]
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes) => {
            // Optional<Bytes> also expands to pointer + length
            let cname = "c".to_string() + name;
            vec![cname.clone(), format!("{}Len", cname)]
        }
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => vec!["c".to_string() + name],
        TypeRef::Named(_) => vec!["c".to_string() + name],
        TypeRef::Vec(_) | TypeRef::Map(_, _) => vec!["c".to_string() + name],
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Named(_) => {
                vec!["c".to_string() + name]
            }
            // Optional primitives are unwrapped via a `c<Name>` local that coerces null → 0/false
            // (see marshal_param_to_ffi). Reference that local instead of the raw boxed parameter
            // so MethodHandle.invoke doesn't auto-unbox a null Long/Integer and throw NPE.
            TypeRef::Primitive(_) => vec!["c".to_string() + name],
            _ => vec![name.to_string()],
        },
        _ => vec![name.to_string()],
    }
}

pub(crate) fn gen_function_descriptor(return_layout: &str, param_layouts: &[String]) -> String {
    if return_layout.is_empty() {
        // Void return
        if param_layouts.is_empty() {
            "FunctionDescriptor.ofVoid()".to_string()
        } else {
            format!("FunctionDescriptor.ofVoid({})", param_layouts.join(", "))
        }
    } else {
        // Non-void return
        if param_layouts.is_empty() {
            format!("FunctionDescriptor.of({})", return_layout)
        } else {
            format!("FunctionDescriptor.of({}, {})", return_layout, param_layouts.join(", "))
        }
    }
}

pub(crate) fn gen_helper_methods(out: &mut String, prefix: &str, class_name: &str) {
    // Only emit helper methods that are actually called in the generated body.
    let needs_check_last_error = out.contains("checkLastError()");
    let needs_read_cstring = out.contains("readCString(");
    let needs_read_bytes = out.contains("readBytes(");
    let needs_read_json_list = out.contains("readJsonList(");
    let needs_create_object_mapper = out.contains("MAPPER.") || needs_read_json_list;

    if !needs_check_last_error
        && !needs_read_cstring
        && !needs_read_bytes
        && !needs_read_json_list
        && !needs_create_object_mapper
    {
        return;
    }

    out.push_str(&crate::template_env::render(
        "gen_helper_methods_header.jinja",
        minijinja::context! {},
    ));
    out.push('\n');

    if needs_check_last_error {
        // Reads the last FFI error code and, if non-zero, reads the error message and throws.
        // Called immediately after a null-pointer return from an FFI call.
        out.push_str(&crate::template_env::render(
            "helper_check_last_error.jinja",
            minijinja::context! {
                prefix_upper => prefix.to_uppercase(),
                class_name => class_name,
            },
        ));
    }

    if needs_create_object_mapper {
        out.push_str(&crate::template_env::render(
            "helper_object_mapper.jinja",
            minijinja::context! {},
        ));
    }

    if needs_read_cstring {
        out.push_str(&crate::template_env::render(
            "helper_read_cstring.jinja",
            minijinja::context! {},
        ));
    }

    if needs_read_bytes {
        out.push_str(&crate::template_env::render(
            "helper_read_bytes.jinja",
            minijinja::context! {},
        ));
    }

    if needs_read_json_list {
        // Single shared helper for the FFI Vec-return path. Consolidates the
        // null-check → reinterpret → free → JSON-deserialize boilerplate that
        // was previously inlined at every Vec-returning call site (and which
        // CPD correctly flagged as duplication). Returns an empty list on a
        // null pointer to mirror the previous inline behavior.
        let free_handle = format!("NativeLib.{}_FREE_STRING", prefix.to_uppercase());
        out.push_str(&crate::template_env::render(
            "helper_read_json_list.jinja",
            minijinja::context! {
                class_name => class_name,
                free_handle => free_handle,
            },
        ));
    }
}
