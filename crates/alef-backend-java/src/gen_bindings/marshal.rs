use crate::type_map::java_ffi_type;
use ahash::AHashSet;
use alef_core::ir::{PrimitiveType, TypeRef};
use heck::ToSnakeCase;
use std::fmt::Write;

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
            writeln!(out, "            var {} = arena.allocateFrom({});", cname, name).ok();
        }
        TypeRef::Path => {
            // Arena.allocateFrom takes a CharSequence; java.nio.file.Path is not one.
            let cname = "c".to_string() + name;
            writeln!(
                out,
                "            var {} = arena.allocateFrom({}.toString());",
                cname, name
            )
            .ok();
        }
        TypeRef::Named(type_name) => {
            let cname = "c".to_string() + name;
            if opaque_types.contains(type_name.as_str()) {
                // Opaque handles: pass the inner MemorySegment via .handle()
                writeln!(out, "            var {} = {}.handle();", cname, name).ok();
            } else {
                // Non-opaque named types: serialize to JSON, call _from_json to get FFI pointer.
                // The pointer must be freed after the FFI call with _free.
                let type_snake = type_name.to_snake_case();
                let from_json_handle = format!(
                    "NativeLib.{}_{}_FROM_JSON",
                    prefix.to_uppercase(),
                    type_snake.to_uppercase()
                );
                let _free_handle = format!("NativeLib.{}_{}_FREE", prefix.to_uppercase(), type_snake.to_uppercase());
                writeln!(
                    out,
                    "            var {}Json = {} != null ? MAPPER.writeValueAsString({}) : null;",
                    cname, name, name
                )
                .ok();
                writeln!(
                    out,
                    "            var {}JsonSeg = {}Json != null ? arena.allocateFrom({}Json) : MemorySegment.NULL;",
                    cname, cname, cname
                )
                .ok();
                writeln!(out, "            var {} = {}Json != null", cname, cname).ok();
                writeln!(
                    out,
                    "                ? (MemorySegment) {}.invoke({}JsonSeg)",
                    from_json_handle, cname
                )
                .ok();
                writeln!(out, "                : MemorySegment.NULL;").ok();
            }
        }
        TypeRef::Optional(inner) => {
            // For optional types, marshal the inner type if not null
            match inner.as_ref() {
                TypeRef::String | TypeRef::Char | TypeRef::Json => {
                    let cname = "c".to_string() + name;
                    writeln!(
                        out,
                        "            var {} = {} != null ? arena.allocateFrom({}) : MemorySegment.NULL;",
                        cname, name, name
                    )
                    .ok();
                }
                TypeRef::Path => {
                    let cname = "c".to_string() + name;
                    writeln!(
                        out,
                        "            var {} = {} != null ? arena.allocateFrom({}.toString()) : MemorySegment.NULL;",
                        cname, name, name
                    )
                    .ok();
                }
                TypeRef::Named(type_name) => {
                    let cname = "c".to_string() + name;
                    if opaque_types.contains(type_name.as_str()) {
                        writeln!(
                            out,
                            "            var {} = {} != null ? {}.handle() : MemorySegment.NULL;",
                            cname, name, name
                        )
                        .ok();
                    } else {
                        // Non-opaque named type in Optional: serialize to JSON and call _from_json
                        let type_snake = type_name.to_snake_case();
                        let from_json_handle = format!(
                            "NativeLib.{}_{}_FROM_JSON",
                            prefix.to_uppercase(),
                            type_snake.to_uppercase()
                        );
                        writeln!(
                            out,
                            "            var {}Json = {} != null ? MAPPER.writeValueAsString({}) : null;",
                            cname, name, name
                        )
                        .ok();
                        writeln!(out, "            var {}JsonSeg = {}Json != null ? arena.allocateFrom({}Json) : MemorySegment.NULL;", cname, cname, cname).ok();
                        writeln!(out, "            var {} = {}Json != null", cname, cname).ok();
                        writeln!(
                            out,
                            "                ? (MemorySegment) {}.invoke({}JsonSeg)",
                            from_json_handle, cname
                        )
                        .ok();
                        writeln!(out, "                : MemorySegment.NULL;").ok();
                    }
                }
                _ => {
                    // Other optional types (primitives) pass through
                }
            }
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            // Vec/Map types: serialize to JSON string, then pass as a C string via arena.
            let cname = "c".to_string() + name;
            writeln!(
                out,
                "            var {}Json = MAPPER.writeValueAsString({});",
                cname, name
            )
            .ok();
            writeln!(out, "            var {} = arena.allocateFrom({}Json);", cname, cname).ok();
        }
        _ => {
            // Primitives and others pass through directly
        }
    }
}

pub(crate) fn ffi_param_name(name: &str, ty: &TypeRef, _opaque_types: &AHashSet<String>) -> String {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "c".to_string() + name,
        TypeRef::Named(_) => "c".to_string() + name,
        TypeRef::Vec(_) | TypeRef::Map(_, _) => "c".to_string() + name,
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Named(_) => {
                "c".to_string() + name
            }
            _ => name.to_string(),
        },
        _ => name.to_string(),
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

    writeln!(out, "    // Helper methods for FFI marshalling").ok();
    writeln!(out).ok();

    if needs_check_last_error {
        // Reads the last FFI error code and, if non-zero, reads the error message and throws.
        // Called immediately after a null-pointer return from an FFI call.
        out.push_str(&crate::template_env::render("helper_check_last_error.jinja", minijinja::context! {
            prefix_upper => prefix.to_uppercase(),
            class_name => class_name,
        }));
    }

    if needs_create_object_mapper {
        out.push_str(&crate::template_env::render("helper_object_mapper.jinja", minijinja::context! {}));
    }

    if needs_read_cstring {
        out.push_str(&crate::template_env::render("helper_read_cstring.jinja", minijinja::context! {}));
    }

    if needs_read_bytes {
        out.push_str(&crate::template_env::render("helper_read_bytes.jinja", minijinja::context! {}));
    }

    if needs_read_json_list {
        // Single shared helper for the FFI Vec-return path. Consolidates the
        // null-check → reinterpret → free → JSON-deserialize boilerplate that
        // was previously inlined at every Vec-returning call site (and which
        // CPD correctly flagged as duplication). Returns an empty list on a
        // null pointer to mirror the previous inline behavior.
        let free_handle = format!("NativeLib.{}_FREE_STRING", prefix.to_uppercase());
        out.push_str(&crate::template_env::render("helper_read_json_list.jinja", minijinja::context! {
            class_name => class_name,
            free_handle => free_handle,
        }));
    }
}
