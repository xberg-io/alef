use heck::ToSnakeCase;

use crate::backends::go::type_map::go_type;
use crate::codegen::naming::go_type_name;
use crate::core::ir::TypeRef;

pub(in crate::backends::go::gen_bindings) fn cgo_type_for_primitive(
    prim: &crate::core::ir::PrimitiveType,
) -> &'static str {
    use crate::core::ir::PrimitiveType;
    match prim {
        PrimitiveType::U8 => "C.uint8_t",
        PrimitiveType::U16 => "C.uint16_t",
        PrimitiveType::U32 => "C.uint32_t",
        PrimitiveType::U64 => "C.uint64_t",
        PrimitiveType::Usize => "C.size_t",
        PrimitiveType::I8 => "C.int8_t",
        PrimitiveType::I16 => "C.int16_t",
        PrimitiveType::I32 => "C.int32_t",
        PrimitiveType::I64 => "C.int64_t",
        PrimitiveType::Isize => "C.ptrdiff_t",
        PrimitiveType::F32 => "C.float",
        PrimitiveType::F64 => "C.double",
        PrimitiveType::Bool => "C.int32_t",
    }
}

/// Return the Go expression for the maximum value of a primitive type, used as a sentinel
/// to signal "None" to FFI functions that use max-value sentinels for optional primitives.
pub(in crate::backends::go::gen_bindings) fn primitive_max_sentinel(
    prim: &crate::core::ir::PrimitiveType,
) -> &'static str {
    use crate::core::ir::PrimitiveType;
    match prim {
        PrimitiveType::U8 => "^uint8(0)",
        PrimitiveType::U16 => "^uint16(0)",
        PrimitiveType::U32 => "^uint32(0)",
        PrimitiveType::U64 => "^uint64(0)",
        PrimitiveType::Usize => "^uint(0)",
        PrimitiveType::I8 => "int8(127)",
        PrimitiveType::I16 => "int16(32767)",
        PrimitiveType::I32 => "int32(2147483647)",
        PrimitiveType::I64 => "int64(9223372036854775807)",
        PrimitiveType::Isize => "int(^uint(0) >> 1)",
        PrimitiveType::F32 => "float32(0)",
        PrimitiveType::F64 => "float64(0)",
        PrimitiveType::Bool => "false",
    }
}

/// Get a type name suitable for a function suffix (e.g., unmarshalFoo).
pub(in crate::backends::go::gen_bindings) fn type_name(ty: &TypeRef) -> String {
    match ty {
        // IR Named types are already PascalCase from Rust source. Avoid
        // ToPascalCase to preserve all-caps acronyms (GraphQL, JSON, HTTP).
        TypeRef::Named(n) => n.clone(),
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "Bytes".to_string(),
        TypeRef::Optional(inner) => type_name(inner),
        TypeRef::Vec(inner) => format!("List{}", type_name(inner)),
        TypeRef::Map(_, v) => format!("Map{}", type_name(v)),
        TypeRef::Json => "JSON".to_string(),
        TypeRef::Path => "Path".to_string(),
        TypeRef::Unit => "Void".to_string(),
        TypeRef::Duration => "U64".to_string(),
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => "Bool".to_string(),
            crate::core::ir::PrimitiveType::U8 => "U8".to_string(),
            crate::core::ir::PrimitiveType::U16 => "U16".to_string(),
            crate::core::ir::PrimitiveType::U32 => "U32".to_string(),
            crate::core::ir::PrimitiveType::U64 => "U64".to_string(),
            crate::core::ir::PrimitiveType::I8 => "I8".to_string(),
            crate::core::ir::PrimitiveType::I16 => "I16".to_string(),
            crate::core::ir::PrimitiveType::I32 => "I32".to_string(),
            crate::core::ir::PrimitiveType::I64 => "I64".to_string(),
            crate::core::ir::PrimitiveType::F32 => "F32".to_string(),
            crate::core::ir::PrimitiveType::F64 => "F64".to_string(),
            crate::core::ir::PrimitiveType::Usize => "Usize".to_string(),
            crate::core::ir::PrimitiveType::Isize => "Isize".to_string(),
        },
    }
}

/// Generate a Go expression that converts a C return value (`ptr`) to the correct Go type.
///
/// For primitives like Bool, this produces inline conversion (e.g., `func() *bool { v := ptr != 0; return &v }()`).
/// For Named types (opaque handles), this uses `_to_json` to serialize then `json.Unmarshal` in Go.
/// For strings, this calls `C.GoString`.
/// The `ffi_prefix` is used to construct C type names for Named types.
pub(in crate::backends::go::gen_bindings) fn go_return_expr(
    ty: &TypeRef,
    var_name: &str,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    value_only_types: &std::collections::HashSet<String>,
) -> String {
    go_return_expr_inner(ty, var_name, ffi_prefix, opaque_names, value_only_types)
}

fn go_return_expr_inner(
    ty: &TypeRef,
    var_name: &str,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    // value_only_types was previously used to skip _to_json for all-primitive structs.
    // The FFI backend now emits _to_json for all non-opaque non-Update types, so this
    // set is no longer consulted. The parameter is kept for API compatibility.
    _value_only_types: &std::collections::HashSet<String>,
) -> String {
    match ty {
        TypeRef::Primitive(prim) => match prim {
            crate::core::ir::PrimitiveType::Bool => format!("{var_name} != 0"),
            _ => {
                // Numeric primitives: plain cast. Non-Optional fallible methods use this
                // expression in the value position of `return value, nil`; Optional(Primitive)
                // returns wrap this in a `func() *T { v := <expr>; return &v }()` closure
                // (handled in the Optional branch below).
                let go_ty = go_type(ty);
                format!("{go_ty}({var_name})")
            }
        },
        TypeRef::Named(name) => {
            if opaque_names.contains(name.as_str()) {
                // Opaque types: wrap the raw C pointer in the Go handle struct.
                // IR name is already PascalCase from Rust; preserve all-caps
                // acronyms (GraphQLError stays GraphQLError, not GraphQlError).
                format!(
                    "&{go_type}{{ptr: unsafe.Pointer({var_name})}}",
                    go_type = name,
                    var_name = var_name,
                )
            } else {
                // Full conversion: serialize C handle to JSON, then unmarshal into Go struct.
                // The FFI backend emits _to_json for all non-opaque types (including those whose
                // fields are all primitives/strings), so we always use the JSON path here.
                let type_snake = name.to_snake_case();
                let go_type = go_type_name(name);
                format!(
                    "func() *{go_type} {{\n\
                     \tjsonPtr := C.{ffi_prefix}_{type_snake}_to_json({var_name})\n\
                     \tif jsonPtr == nil {{ return nil }}\n\
                     \tdefer C.{ffi_prefix}_free_string(jsonPtr)\n\
                     \tvar result {go_type}\n\
                     \tif err := json.Unmarshal([]byte(C.GoString(jsonPtr)), &result); err != nil {{ return nil }}\n\
                     \treturn &result\n\
                     }}()",
                    go_type = go_type,
                    ffi_prefix = ffi_prefix,
                    type_snake = type_snake,
                    var_name = var_name,
                )
            }
        }
        TypeRef::String | TypeRef::Char | TypeRef::Path => {
            // Non-optional String/Char/Path: return bare string value (C pointer conversion)
            format!("C.GoString({})", var_name)
        }
        TypeRef::Json => {
            format!(
                "func() *json.RawMessage {{ if {var_name} == nil {{ return nil }}; v := json.RawMessage(C.GoString({var_name})); return &v }}()"
            )
        }
        TypeRef::Bytes => {
            format!("unmarshalBytes({})", var_name)
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            // Optional(Bytes): the bare Bytes arm yields `unmarshalBytes(ptr)` which is `[]byte`,
            // but an `Option<&[u8]>` return has signature `*[]byte`. Null-check the pointer and
            // box the copied slice so a null FFI pointer maps to a nil `*[]byte`.
            TypeRef::Bytes => {
                format!(
                    "func() *[]byte {{\n\
                     \tif {var_name} == nil {{ return nil }}\n\
                     \tb := unmarshalBytes({var_name})\n\
                     \treturn &b\n\
                     }}()",
                    var_name = var_name,
                )
            }
            // Optional(Primitive): wrap the plain primitive expression in a closure that
            // takes its address. The Primitive branch above emits plain `T(var)` / `var != 0`,
            // which would be a value (not a pointer); Optional callers want `*T`.
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => {
                format!("func() *bool {{ v := {var_name} != 0; return &v }}()")
            }
            TypeRef::Primitive(_) => {
                let go_ty = go_type(inner);
                format!("func() *{go_ty} {{ v := {go_ty}({var_name}); return &v }}()")
            }
            // Optional(String/Char/Path): null-check and box the converted Go string.
            // The C function returns an allocated string pointer; free it after conversion.
            TypeRef::String | TypeRef::Char | TypeRef::Path => {
                format!(
                    "func() *string {{\n\
                     \tif {var_name} == nil {{ return nil }}\n\
                     \tdefer C.{ffi_prefix}_free_string({var_name})\n\
                     \ts := C.GoString({var_name})\n\
                     \treturn &s\n\
                     }}()",
                    var_name = var_name,
                    ffi_prefix = ffi_prefix,
                )
            }
            _ => go_return_expr_inner(inner, var_name, ffi_prefix, opaque_names, _value_only_types),
        },
        TypeRef::Vec(inner) => {
            // Vec types are returned as JSON strings from FFI. Deserialize inline.
            // Return []T (not *[]T) — slices are already reference types in Go.
            let go_elem = go_type(inner);
            format!(
                "func() []{go_elem} {{\n\
                 \tif {var_name} == nil {{ return nil }}\n\
                 \tdefer C.{ffi_prefix}_free_string({var_name})\n\
                 \tvar result []{go_elem}\n\
                 \tif err := json.Unmarshal([]byte(C.GoString({var_name})), &result); err != nil {{ return nil }}\n\
                 \treturn result\n\
                 }}()",
                go_elem = go_elem,
                var_name = var_name,
                ffi_prefix = ffi_prefix,
            )
        }
        TypeRef::Map(k, v) => {
            // Map types are returned as JSON strings from FFI. Deserialize inline.
            // Return map[K]V (not *map[K]V) — maps are already reference types in Go.
            let go_k = go_type(k);
            let go_v = go_type(v);
            format!(
                "func() map[{go_k}]{go_v} {{\n\
                 \tif {var_name} == nil {{ return nil }}\n\
                 \tdefer C.{ffi_prefix}_free_string({var_name})\n\
                 \tvar result map[{go_k}]{go_v}\n\
                 \tif err := json.Unmarshal([]byte(C.GoString({var_name})), &result); err != nil {{ return nil }}\n\
                 \treturn result\n\
                 }}()",
                go_k = go_k,
                go_v = go_v,
                var_name = var_name,
                ffi_prefix = ffi_prefix,
            )
        }
        _ => format!("unmarshal{}({})", type_name(ty), var_name),
    }
}
