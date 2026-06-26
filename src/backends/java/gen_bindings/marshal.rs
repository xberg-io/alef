use crate::backends::java::type_map::java_ffi_type;
use crate::core::ir::{FunctionDef, MethodDef, PrimitiveType, TypeRef};
use ahash::AHashSet;
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

/// Returns the Java cast expression (including parens) to apply to `MethodHandle.invoke()`.
///
/// All integers use `JAVA_LONG` (see `java_ffi_type()`), so `invoke()` always boxes
/// as `Long`. The cast unboxes Long then narrows to the target Java type:
///
/// - bool/i64/u64: `(long)` — unbox Long directly
/// - i32/u32: `(int)(long)` — unbox Long, narrow to int
/// - i16/u16: `(short)(long)` — unbox Long, narrow to short
/// - i8/u8: `(byte)(long)` — unbox Long, narrow to byte
pub(crate) fn java_ffi_return_cast(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Primitive(prim) => match prim {
            // All integers use JAVA_LONG; bool keeps the long value (comparison narrows to bool).
            PrimitiveType::Bool => "(long)",
            // 8-bit: unbox Long, narrow to byte.
            PrimitiveType::U8 | PrimitiveType::I8 => "(byte)(long)",
            // 16-bit: unbox Long, narrow to short.
            PrimitiveType::U16 | PrimitiveType::I16 => "(short)(long)",
            // 32-bit: unbox Long, narrow to int.
            PrimitiveType::U32 | PrimitiveType::I32 => "(int)(long)",
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => "(long)",
            PrimitiveType::F32 => "(float)",
            PrimitiveType::F64 => "(double)",
        },
        TypeRef::Duration => "(long)",
        _ => "(MemorySegment)",
    }
}

pub(crate) fn java_ffi_return_expr(ty: &TypeRef, var_name: &str) -> String {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => format!("{var_name} != 0"),
        _ => var_name.to_string(),
    }
}

pub(crate) fn gen_ffi_layout_with_enums(ty: &TypeRef, enum_names: &AHashSet<String>) -> String {
    match ty {
        TypeRef::Primitive(prim) => java_ffi_type(prim).to_string(),
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Bytes => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Optional(inner) => gen_ffi_layout_with_enums(inner, enum_names),
        TypeRef::Vec(_) => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Map(_, _) => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Named(name) => {
            // Enum types (Copy-typed Named types) are passed as i32 discriminants.
            // Use JAVA_LONG for the same Win64 Panama compat reason as PrimitiveType::I32.
            if enum_names.contains(name.as_str()) {
                "ValueLayout.JAVA_LONG".to_string()
            } else {
                "ValueLayout.ADDRESS".to_string()
            }
        }
        TypeRef::Unit => "".to_string(),
        TypeRef::Duration => "ValueLayout.JAVA_LONG".to_string(),
    }
}

/// Build the Jackson `writer` expression that preserves generic element-type
/// info for a `Vec<T>` / `Map<K, V>` Java parameter. Without this, `MAPPER.
/// writeValueAsString(list)` erases T at runtime, dropping `@JsonTypeInfo`
/// discriminators on polymorphic element types like the tagged-enum DTO
/// `PageAction`. The returned expression is a `ObjectWriter`.
fn build_collection_writer_for(inner: &TypeRef, outer: &TypeRef, _opaque_types: &AHashSet<String>) -> String {
    let elem_class = java_class_literal_for(inner);
    match outer {
        TypeRef::Vec(_) => format!(
            "MAPPER.writerFor(MAPPER.getTypeFactory().constructCollectionType(java.util.List.class, {elem_class}))"
        ),
        TypeRef::Map(k, _) => {
            let key_class = java_class_literal_for(k);
            format!(
                "MAPPER.writerFor(MAPPER.getTypeFactory().constructMapType(java.util.Map.class, {key_class}, {elem_class}))"
            )
        }
        _ => "MAPPER.writer()".to_string(),
    }
}

/// Render the Java `Class<?>` literal (e.g., `String.class`, `PageAction.class`,
/// `Integer.class`) for a Rust IR type. Used to construct typed Jackson
/// CollectionType / MapType so polymorphic element types serialize correctly.
fn java_class_literal_for(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String.class".to_string(),
        TypeRef::Bytes => "byte[].class".to_string(),
        TypeRef::Path => "java.nio.file.Path.class".to_string(),
        TypeRef::Json => "Object.class".to_string(),
        TypeRef::Unit => "Void.class".to_string(),
        TypeRef::Duration => "Long.class".to_string(),
        TypeRef::Primitive(prim) => match prim {
            PrimitiveType::Bool => "Boolean.class".to_string(),
            PrimitiveType::U8 | PrimitiveType::I8 => "Byte.class".to_string(),
            PrimitiveType::U16 | PrimitiveType::I16 => "Short.class".to_string(),
            PrimitiveType::U32 | PrimitiveType::I32 => "Integer.class".to_string(),
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => {
                "Long.class".to_string()
            }
            PrimitiveType::F32 => "Float.class".to_string(),
            PrimitiveType::F64 => "Double.class".to_string(),
        },
        TypeRef::Named(name) => format!("{name}.class"),
        TypeRef::Optional(inner) => java_class_literal_for(inner),
        TypeRef::Vec(_) => "java.util.List.class".to_string(),
        TypeRef::Map(_, _) => "java.util.Map.class".to_string(),
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
        TypeRef::String | TypeRef::Char => {
            let cname = "c".to_string() + name;
            out.push_str(&crate::backends::java::template_env::render(
                "marshal_string.jinja",
                minijinja::context! {
                    cname => &cname,
                    name => name,
                },
            ));
        }
        TypeRef::Json => {
            // Object (polymorphic JSON) does not require marshalling.
            // Pass through directly.
        }
        TypeRef::Path => {
            let cname = "c".to_string() + name;
            out.push_str(&crate::backends::java::template_env::render(
                "marshal_path.jinja",
                minijinja::context! {
                    cname => &cname,
                    name => name,
                },
            ));
        }
        TypeRef::Bytes => {
            let cname = "c".to_string() + name;
            out.push_str(&crate::backends::java::template_env::render(
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
                out.push_str(&crate::backends::java::template_env::render(
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
                out.push_str(&crate::backends::java::template_env::render(
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
                TypeRef::String | TypeRef::Char => {
                    let cname = "c".to_string() + name;
                    out.push_str(&crate::backends::java::template_env::render(
                        "marshal_optional_string.jinja",
                        minijinja::context! {
                            cname => &cname,
                            name => name,
                        },
                    ));
                }
                TypeRef::Json => {
                    // Optional<Object> (polymorphic JSON) does not require marshalling.
                    // Pass through directly.
                }
                TypeRef::Path => {
                    let cname = "c".to_string() + name;
                    out.push_str(&crate::backends::java::template_env::render(
                        "marshal_optional_path.jinja",
                        minijinja::context! {
                            cname => &cname,
                            name => name,
                        },
                    ));
                }
                TypeRef::Bytes => {
                    let cname = "c".to_string() + name;
                    out.push_str(&crate::backends::java::template_env::render(
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
                        out.push_str(&crate::backends::java::template_env::render(
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
                        out.push_str(&crate::backends::java::template_env::render(
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
                    use crate::core::ir::PrimitiveType;
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
                        PrimitiveType::Bool => ("int", "0"),
                    };
                    out.push_str(&crate::backends::java::template_env::render(
                        "marshal_optional_primitive.jinja",
                        minijinja::context! {
                            cname => &cname,
                            name => name,
                            prim_kw => prim_kw,
                            none_lit => none_lit,
                            value_expr => if matches!(prim, PrimitiveType::Bool) {
                                format!("({name} ? 1 : 0)")
                            } else {
                                name.to_string()
                            },
                        },
                    ));
                }
                _ => {
                    // Other optional types pass through
                }
            }
        }
        TypeRef::Vec(inner) | TypeRef::Map(_, inner) => {
            let cname = "c".to_string() + name;
            // Jackson loses generic element-type info on a bare List<T>/Map<K,V> serialization
            // (java type erasure: at runtime the value is `ArrayList<Click>`, the declared
            // T is unrecoverable). For polymorphic element types annotated with
            // `@JsonTypeInfo` (tagged-enum DTOs like `PageAction`), this drops the
            // discriminator (`{"selector":"#x"}` instead of `{"type":"click","selector":"#x"}`)
            // and the Rust side fails to deserialize with `missing field "type"`.
            // Use `writerFor(TypeFactory.constructCollectionType(...))` so Jackson sees
            // the declared element type and re-applies polymorphic typing.
            let java_writer = build_collection_writer_for(inner, ty, opaque_types);
            out.push_str(&crate::backends::java::template_env::render(
                "marshal_vec_map.jinja",
                minijinja::context! {
                    cname => &cname,
                    name => name,
                    java_writer => &java_writer,
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
        TypeRef::String | TypeRef::Char | TypeRef::Path => vec!["c".to_string() + name],
        TypeRef::Json => {
            // Object (polymorphic JSON) passed directly without marshalling.
            vec![name.to_string()]
        }
        TypeRef::Named(_) => vec!["c".to_string() + name],
        TypeRef::Vec(_) | TypeRef::Map(_, _) => vec!["c".to_string() + name],
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Named(_) => {
                vec!["c".to_string() + name]
            }
            TypeRef::Json => {
                // Optional<Object> passed directly without marshalling.
                vec![name.to_string()]
            }
            // Optional primitives are unwrapped via a `c<Name>` local that coerces null → 0/false
            // (see marshal_param_to_ffi). Reference that local instead of the raw boxed parameter
            // so MethodHandle.invoke doesn't auto-unbox a null Long/Integer and throw NPE.
            TypeRef::Primitive(_) => vec!["c".to_string() + name],
            _ => vec![name.to_string()],
        },
        TypeRef::Primitive(PrimitiveType::Bool) => vec![format!("({name} ? 1 : 0)")],
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

    out.push_str(&crate::backends::java::template_env::render(
        "gen_helper_methods_header.jinja",
        minijinja::context! {},
    ));
    out.push('\n');

    if needs_check_last_error {
        // Reads the last FFI error code and, if non-zero, reads the error message and throws.
        // Called immediately after a null-pointer return from an FFI call.
        out.push_str(&crate::backends::java::template_env::render(
            "helper_check_last_error.jinja",
            minijinja::context! {
                prefix_upper => prefix.to_uppercase(),
                class_name => class_name,
            },
        ));
    }

    if needs_create_object_mapper {
        out.push_str(&crate::backends::java::template_env::render(
            "helper_object_mapper.jinja",
            minijinja::context! {},
        ));
    }

    if needs_read_cstring {
        out.push_str(&crate::backends::java::template_env::render(
            "helper_read_cstring.jinja",
            minijinja::context! {},
        ));
    }

    if needs_read_bytes {
        out.push_str(&crate::backends::java::template_env::render(
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
        out.push_str(&crate::backends::java::template_env::render(
            "helper_read_json_list.jinja",
            minijinja::context! {
                class_name => class_name,
                free_handle => free_handle,
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashSet;

    #[test]
    fn java_ffi_return_cast_i8_uses_long_chain() {
        assert_eq!(
            java_ffi_return_cast(&TypeRef::Primitive(PrimitiveType::I8)),
            "(byte)(long)"
        );
        assert_eq!(
            java_ffi_return_cast(&TypeRef::Primitive(PrimitiveType::U8)),
            "(byte)(long)"
        );
    }

    #[test]
    fn java_ffi_return_cast_i16_uses_long_chain() {
        assert_eq!(
            java_ffi_return_cast(&TypeRef::Primitive(PrimitiveType::I16)),
            "(short)(long)"
        );
        assert_eq!(
            java_ffi_return_cast(&TypeRef::Primitive(PrimitiveType::U16)),
            "(short)(long)"
        );
    }

    #[test]
    fn gen_ffi_layout_with_enums_enum_uses_java_long() {
        let mut enum_names = AHashSet::new();
        enum_names.insert("Status".to_string());
        let ty = TypeRef::Named("Status".to_string());
        assert_eq!(gen_ffi_layout_with_enums(&ty, &enum_names), "ValueLayout.JAVA_LONG");
    }

    #[test]
    fn gen_ffi_layout_with_enums_non_enum_named_uses_address() {
        let mut enum_names = AHashSet::new();
        enum_names.insert("Status".to_string());
        let ty = TypeRef::Named("SomeStruct".to_string());
        assert_eq!(gen_ffi_layout_with_enums(&ty, &enum_names), "ValueLayout.ADDRESS");
    }
}
