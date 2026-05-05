use super::functions::params_require_marshal;
use super::types::{cgo_type_for_primitive, emit_type_doc, go_return_expr, primitive_max_sentinel};
use crate::type_map::{go_optional_type, go_type};
use alef_codegen::naming::{go_param_name, go_type_name, to_go_name};
use alef_core::ir::{MethodDef, ParamDef, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::fmt::Write;

/// Generate a wrapper method for a struct method.
pub(super) fn gen_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::with_capacity(2048);

    let method_go_name = to_go_name(&method.name);

    emit_type_doc(&mut out, &method_go_name, &method.doc, "is a method.");

    // A non-opaque, non-static method marshals its receiver to JSON — that is fallible.
    // Also include params that require marshaling.
    let receiver_requires_marshal = !method.is_static && !typ.is_opaque;
    let method_marshals = receiver_requires_marshal || params_require_marshal(&method.params, opaque_names);
    let method_can_return_error = method.error_type.is_some() || method_marshals;

    let return_type = if method_can_return_error {
        if matches!(method.return_type, TypeRef::Unit) {
            "error".to_string()
        } else {
            format!("({}, error)", go_optional_type(&method.return_type))
        }
    } else if matches!(method.return_type, TypeRef::Unit) {
        "".to_string()
    } else {
        go_optional_type(&method.return_type).into_owned()
    };

    // Opaque types use "h" (for "handle") to match the receiver name in Free().
    // Non-opaque types use "r" (for "receiver").
    let receiver_name = if typ.is_opaque { "h" } else { "r" };
    let go_receiver_type = go_type_name(&typ.name);

    // Static methods become package-level functions (no receiver in Go)
    if method.is_static {
        write!(out, "func {}{}(", go_receiver_type, method_go_name).ok();
    } else {
        write!(
            out,
            "func ({} *{}) {}(",
            receiver_name, go_receiver_type, method_go_name
        )
        .ok();
    }

    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let param_type: String = if p.optional {
                go_optional_type(&p.ty).into_owned()
            } else if let TypeRef::Named(name) = &p.ty {
                if opaque_names.contains(name.as_str()) {
                    format!("*{}", go_type(&p.ty))
                } else {
                    go_type(&p.ty).into_owned()
                }
            } else {
                go_type(&p.ty).into_owned()
            };
            format!("{} {}", go_param_name(&p.name), param_type)
        })
        .collect();
    write!(out, "{}", params.join(", ")).ok();

    if return_type.is_empty() {
        writeln!(out, ") {{").ok();
    } else {
        writeln!(out, ") {} {{", return_type).ok();
    }

    {
        // Synchronous method - just convert params and call FFI
        // Note: method_can_return_error is set above (includes synthesized error for marshal-requiring methods).
        let returns_value_and_error = method_can_return_error && !matches!(method.return_type, TypeRef::Unit);
        for param in &method.params {
            write!(
                out,
                "{}",
                gen_param_to_c(
                    param,
                    returns_value_and_error,
                    method_can_return_error,
                    ffi_prefix,
                    opaque_names
                )
            )
            .ok();
        }

        // Bytes params expand to two C arguments: the pointer and the length.
        let c_params: Vec<String> = method
            .params
            .iter()
            .flat_map(|p| -> Vec<String> {
                let c_name = go_param_name(&format!("c_{}", p.name));
                if matches!(p.ty, TypeRef::Bytes) {
                    vec![c_name.clone(), format!("{}Len", c_name)]
                } else {
                    vec![c_name]
                }
            })
            .collect();

        let type_snake = typ.name.to_snake_case();
        let method_snake = method.name.to_snake_case();
        let c_call = if method.is_static {
            // Static methods don't pass a receiver
            if c_params.is_empty() {
                format!("C.{}_{}_{}()", ffi_prefix, type_snake, method_snake)
            } else {
                format!(
                    "C.{}_{}_{}({})",
                    ffi_prefix,
                    type_snake,
                    method_snake,
                    c_params.join(", ")
                )
            }
        } else if typ.is_opaque {
            // Opaque types have a ptr field — cast it directly.
            let c_receiver = format!(
                "(*C.{}{})(unsafe.Pointer({}.ptr))",
                ffi_prefix.to_uppercase(),
                typ.name,
                receiver_name
            );
            if c_params.is_empty() {
                format!("C.{}_{}_{}({})", ffi_prefix, type_snake, method_snake, c_receiver)
            } else {
                format!(
                    "C.{}_{}_{}({}, {})",
                    ffi_prefix,
                    type_snake,
                    method_snake,
                    c_receiver,
                    c_params.join(", ")
                )
            }
        } else {
            // Non-opaque structs: marshal to JSON, create a temporary handle, use it, and free it.
            let err_prefix = if returns_value_and_error { "nil, " } else { "" };
            // method_can_return_error is always true here (receiver_requires_marshal is true for
            // non-opaque non-static methods), so we always emit fmt.Errorf, never panic.
            let err_action = format!("return {err_prefix}fmt.Errorf(\"failed to marshal receiver: %w\", err)");
            let from_json_err_action = format!(
                "return {err_prefix}fmt.Errorf(\"failed to create receiver: %s\", C.GoString(C.kreuzberg_last_error_context()))"
            );
            writeln!(
                out,
                "\tjsonBytesRecv, err := json.Marshal({recv})\n\t\
                 if err != nil {{\n\t\t\
                 {err_action}\n\t\
                 }}\n\t\
                 tmpStrRecv := C.CString(string(jsonBytesRecv))\n\t\
                 cRecv := C.{ffi_prefix}_{type_snake}_from_json(tmpStrRecv)\n\t\
                 C.free(unsafe.Pointer(tmpStrRecv))\n\t\
                 if cRecv == nil {{\n\t\t\
                 {from_json_err_action}\n\t\
                 }}\n\t\
                 defer C.{ffi_prefix}_{type_snake}_free(cRecv)",
                recv = receiver_name,
                err_action = err_action,
                from_json_err_action = from_json_err_action,
                ffi_prefix = ffi_prefix,
                type_snake = type_snake,
            )
            .ok();
            if c_params.is_empty() {
                format!("C.{}_{}_{}(cRecv)", ffi_prefix, type_snake, method_snake)
            } else {
                format!(
                    "C.{}_{}_{}(cRecv, {})",
                    ffi_prefix,
                    type_snake,
                    method_snake,
                    c_params.join(", ")
                )
            }
        };

        // Detect builder pattern: opaque type method that returns the same opaque type.
        // The C function consumes (Box::from_raw) the input pointer and returns a new pointer.
        // Instead of creating a new Go struct, update r.ptr so the caller's handle stays valid.
        let is_builder_return =
            typ.is_opaque && matches!(&method.return_type, TypeRef::Named(n) if n.as_str() == typ.name.as_str());

        if method_can_return_error {
            if matches!(method.return_type, TypeRef::Unit) {
                writeln!(out, "\t{}", c_call).ok();
                // For non-opaque, non-static methods with Unit return, the C function may have
                // mutated cRecv in place (e.g. apply_update).  Write the updated state back to
                // the Go receiver so the mutation is visible to the caller.
                if !method.is_static && !typ.is_opaque {
                    writeln!(
                        out,
                        "\tjsonPtrUpdated := C.{ffi_prefix}_{type_snake}_to_json(cRecv)\n\t\
                         if jsonPtrUpdated != nil {{\n\t\t\
                         _ = json.Unmarshal([]byte(C.GoString(jsonPtrUpdated)), {recv})\n\t\t\
                         C.{ffi_prefix}_free_string(jsonPtrUpdated)\n\t\
                         }}",
                        ffi_prefix = ffi_prefix,
                        type_snake = type_snake,
                        recv = receiver_name,
                    )
                    .ok();
                }
                if method.error_type.is_some() {
                    writeln!(out, "\treturn lastError()").ok();
                } else {
                    writeln!(out, "\treturn nil").ok();
                }
            } else {
                writeln!(out, "\tptr := {}", c_call).ok();
                if method.error_type.is_some() {
                    writeln!(out, "\tif err := lastError(); err != nil {{").ok();
                    // Free the pointer if non-nil even on error, to avoid leaks.
                    // Bytes pointers are NOT freed — they alias internal storage.
                    if matches!(
                        method.return_type,
                        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
                    ) {
                        writeln!(out, "\t\tif ptr != nil {{").ok();
                        writeln!(out, "\t\t\tC.{}_free_string(ptr)", ffi_prefix).ok();
                        writeln!(out, "\t\t}}").ok();
                    }
                    writeln!(out, "\t\treturn nil, err").ok();
                    writeln!(out, "\t}}").ok();
                }
                // Free the FFI-allocated string after unmarshaling.
                // Bytes pointers are NOT freed — they alias internal storage.
                if matches!(
                    method.return_type,
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
                ) {
                    writeln!(out, "\tdefer C.{}_free_string(ptr)", ffi_prefix).ok();
                }
                // For non-opaque Named return types, free the handle after JSON extraction.
                // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
                if let TypeRef::Named(name) = &method.return_type {
                    if !opaque_names.contains(name.as_str()) {
                        let type_snake = name.to_snake_case();
                        writeln!(out, "\tdefer C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
                    }
                }
                if is_builder_return {
                    // Builder pattern: C consumed the old pointer and returned a new one.
                    // Update r.ptr in-place so the caller's handle remains valid.
                    writeln!(out, "\t{}.ptr = unsafe.Pointer(ptr)", receiver_name).ok();
                    writeln!(out, "\treturn {}, nil", receiver_name).ok();
                } else {
                    writeln!(
                        out,
                        "\treturn {}, nil",
                        go_return_expr(&method.return_type, "ptr", ffi_prefix, opaque_names)
                    )
                    .ok();
                }
            }
        } else if matches!(method.return_type, TypeRef::Unit) {
            writeln!(out, "\t{}", c_call).ok();
        } else {
            writeln!(out, "\tptr := {}", c_call).ok();
            // Add defer free for C string returns.
            // Bytes pointers are NOT freed — they alias internal storage.
            if matches!(
                method.return_type,
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
            ) {
                writeln!(out, "\tdefer C.{}_free_string(ptr)", ffi_prefix).ok();
            }
            // For non-opaque Named return types, free the handle after JSON extraction.
            // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
            if let TypeRef::Named(name) = &method.return_type {
                if !opaque_names.contains(name.as_str()) {
                    let type_snake = name.to_snake_case();
                    writeln!(out, "\tdefer C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
                }
            }
            if is_builder_return {
                // Builder pattern: C consumed the old pointer and returned a new one.
                // Update r.ptr in-place so the caller's handle remains valid.
                writeln!(out, "\t{}.ptr = unsafe.Pointer(ptr)", receiver_name).ok();
                writeln!(out, "\treturn {}", receiver_name).ok();
            } else {
                writeln!(
                    out,
                    "\treturn {}",
                    go_return_expr(&method.return_type, "ptr", ffi_prefix, opaque_names)
                )
                .ok();
            }
        }
    }

    writeln!(out, "}}").ok();
    out
}

/// Generate parameter conversion code from Go to C.
/// `returns_value_and_error` should be true when the enclosing function returns `(*T, error)`,
/// so that error paths emit `return nil, fmt.Errorf(...)` instead of `return fmt.Errorf(...)`.
/// `can_return_error` should be true when the enclosing function has `error` in its return type.
/// When false, marshal failures are handled with `panic` since the function signature has no error return.
pub(super) fn gen_param_to_c(
    param: &ParamDef,
    returns_value_and_error: bool,
    can_return_error: bool,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::with_capacity(512);
    // Go param names must be lowerCamelCase (no underscores), and internal C-side
    // temporaries use the same stem with acronym uppercasing applied.
    let go_param = go_param_name(&param.name);
    let c_name = go_param_name(&format!("c_{}", param.name));
    let err_return_prefix = if returns_value_and_error { "nil, " } else { "" };

    match &param.ty {
        TypeRef::String | TypeRef::Char => {
            if param.optional {
                // Optional string param (ty=String, optional=true): the Go variable holds *string.
                writeln!(
                    out,
                    "\tvar {c_name} *C.char\n\tif {param} != nil {{\n\t\t\
                     {c_name} = C.CString(*{param})\n\t\tdefer C.free(unsafe.Pointer({c_name}))\n\t\
                     }}",
                    c_name = c_name,
                    param = go_param,
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "\t{} := C.CString({})\n\tdefer C.free(unsafe.Pointer({}))",
                    c_name, go_param, c_name
                )
                .ok();
            }
        }
        TypeRef::Path => {
            if param.optional {
                writeln!(
                    out,
                    "\tvar {c_name} *C.char\n\tif {param} != nil {{\n\t\t\
                     {c_name} = C.CString(*{param})\n\t\tdefer C.free(unsafe.Pointer({c_name}))\n\t\
                     }}",
                    c_name = c_name,
                    param = go_param,
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "\t{} := C.CString({})\n\tdefer C.free(unsafe.Pointer({}))",
                    c_name, go_param, c_name
                )
                .ok();
            }
        }
        TypeRef::Bytes => {
            writeln!(out, "\t{} := (*C.uint8_t)(unsafe.Pointer(&{}[0]))", c_name, go_param).ok();
            writeln!(out, "\t{}Len := C.uintptr_t(len({}))", c_name, go_param).ok();
        }
        TypeRef::Named(name) => {
            if opaque_names.contains(name.as_str()) {
                // Opaque types are pointer wrappers — cast the raw pointer to the C type.
                let c_type = format!("{}{}", ffi_prefix.to_uppercase(), name);
                writeln!(
                    out,
                    "\t{c_name} := (*C.{c_type})(unsafe.Pointer({param}.ptr))",
                    param = go_param,
                )
                .ok();
            } else {
                // Non-opaque Named types: marshal to JSON, create a handle via _from_json,
                // and pass that to the C function.
                let type_snake = name.to_snake_case();
                let err_action = if can_return_error {
                    format!("return {err_return_prefix}fmt.Errorf(\"failed to marshal: %w\", err)")
                } else {
                    "panic(fmt.Sprintf(\"failed to marshal: %v\", err))".to_string()
                };
                let from_json_err_action = if can_return_error {
                    format!(
                        "return {err_return_prefix}fmt.Errorf(\"failed to create {}: %s\", C.GoString(C.kreuzberg_last_error_context()))",
                        type_snake
                    )
                } else {
                    format!(
                        "panic(\"failed to create {}: \" + C.GoString(C.kreuzberg_last_error_context()))",
                        type_snake
                    )
                };
                writeln!(
                    out,
                    "\tjsonBytes{c_name}, err := json.Marshal({param})\n\t\
                     if err != nil {{\n\t\t\
                     {err_action}\n\t\
                     }}\n\t\
                     tmpStr{c_name} := C.CString(string(jsonBytes{c_name}))\n\t\
                     {c_name} := C.{ffi_prefix}_{type_snake}_from_json(tmpStr{c_name})\n\t\
                     C.free(unsafe.Pointer(tmpStr{c_name}))\n\t\
                     if {c_name} == nil {{\n\t\t\
                     {from_json_err_action}\n\t\
                     }}\n\t\
                     defer C.{ffi_prefix}_{type_snake}_free({c_name})",
                    c_name = c_name,
                    param = go_param,
                    err_action = err_action,
                    from_json_err_action = from_json_err_action,
                    ffi_prefix = ffi_prefix,
                    type_snake = type_snake,
                )
                .ok();
            }
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            // Vec and Map types are serialized as JSON strings across the FFI boundary.
            let err_action = if can_return_error {
                format!("return {err_return_prefix}fmt.Errorf(\"failed to marshal: %w\", err)")
            } else {
                "panic(fmt.Sprintf(\"failed to marshal: %v\", err))".to_string()
            };
            writeln!(
                out,
                "\tjsonBytes{c_name}, err := json.Marshal({param})\n\t\
                 if err != nil {{\n\t\t\
                 {err_action}\n\t\
                 }}\n\t\
                 {c_name} := C.CString(string(jsonBytes{c_name}))\n\t\
                 defer C.free(unsafe.Pointer({c_name}))",
                c_name = c_name,
                param = go_param,
                err_action = err_action,
            )
            .ok();
        }
        TypeRef::Optional(inner) => {
            match inner.as_ref() {
                TypeRef::String | TypeRef::Char | TypeRef::Path => {
                    writeln!(
                        out,
                        "\tvar {} *C.char\n\tif {} != nil {{\n\t\t\
                         {} = C.CString(*{})\n\t\tdefer C.free(unsafe.Pointer({}))\n\t\
                         }}",
                        c_name, go_param, c_name, go_param, c_name
                    )
                    .ok();
                }
                TypeRef::Named(name) if opaque_names.contains(name.as_str()) => {
                    // Optional opaque type: cast the raw pointer to the C type or pass nil.
                    let c_type = format!("{}{}", ffi_prefix.to_uppercase(), name);
                    writeln!(
                        out,
                        "\tvar {c_name} *C.{c_type}\n\tif {param} != nil {{\n\t\t\
                         {c_name} = (*C.{c_type})(unsafe.Pointer({param}.ptr))\n\t\
                         }}",
                        c_name = c_name,
                        c_type = c_type,
                        param = go_param,
                    )
                    .ok();
                }
                TypeRef::Named(_) => {
                    writeln!(
                        out,
                        "\tvar {} *C.char\n\tif {} != nil {{\n\t\t\
                         jsonBytes, _ := json.Marshal({})\n\t\t\
                         {} = C.CString(string(jsonBytes))\n\t\t\
                         defer C.free(unsafe.Pointer({}))\n\t\
                         }}",
                        c_name, go_param, go_param, c_name, c_name
                    )
                    .ok();
                }
                _ => {
                    // For other optional types, just pass nil or default
                    writeln!(out, "\tvar {} *C.char", c_name).ok();
                }
            }
        }
        TypeRef::Primitive(prim) if !param.optional => {
            // Non-optional primitive: cast to the CGo type so the value can be passed directly
            // to C functions that expect C types (e.g., uintptr_t, uint32_t).
            let cgo_ty = cgo_type_for_primitive(prim);
            let go_ty = go_type(&TypeRef::Primitive(prim.clone()));
            // Special case for bool: Go bool cannot be directly cast to C.uchar.
            // Convert via conditional: if true, 1; else 0.
            if matches!(prim, alef_core::ir::PrimitiveType::Bool) {
                writeln!(
                    out,
                    "\tvar {c_name} {cgo_ty}\n\tif {param} {{\n\t\t{c_name} = 1\n\t}} else {{\n\t\t{c_name} = 0\n\t}}",
                    c_name = c_name,
                    cgo_ty = cgo_ty,
                    param = go_param,
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "\t{c_name} := {cgo_ty}({go_ty}({param}))",
                    c_name = c_name,
                    cgo_ty = cgo_ty,
                    go_ty = go_ty,
                    param = go_param,
                )
                .ok();
            }
        }
        TypeRef::Primitive(prim) if param.optional => {
            // Optional primitive: the Go param is a pointer (*T). Dereference it if non-nil,
            // otherwise pass the max-value sentinel (e.g. u64::MAX) so the FFI layer knows
            // the parameter was omitted.
            //
            // Declare the variable using the CGo type (e.g. C.uint64_t) so that CGo does
            // not reject the value when it is passed directly to the C function. Go's native
            // numeric types (uint64, uint32, …) are distinct from CGo types and cannot be
            // passed without an explicit cast — using the CGo type at declaration avoids a
            // second cast at every call-site.
            let cgo_ty = cgo_type_for_primitive(prim);
            let go_ty = go_type(&TypeRef::Primitive(prim.clone()));
            let sentinel = primitive_max_sentinel(prim);

            // Special case for bool: Go bool cannot be directly cast to C.uchar.
            if matches!(prim, alef_core::ir::PrimitiveType::Bool) {
                writeln!(
                    out,
                    "\tvar {c_name} {cgo_ty} = 255\n\tif {param} != nil {{\n\t\t\
                     if *{param} {{\n\t\t\t{c_name} = 1\n\t\t}} else {{\n\t\t\t{c_name} = 0\n\t\t}}\n\t}}",
                    c_name = c_name,
                    cgo_ty = cgo_ty,
                    param = go_param,
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "\tvar {c_name} {cgo_ty} = {cgo_ty}({sentinel})\n\tif {param} != nil {{\n\t\t\
                     {c_name} = {cgo_ty}({go_ty}(*{param}))\n\t}}",
                    c_name = c_name,
                    cgo_ty = cgo_ty,
                    go_ty = go_ty,
                    sentinel = sentinel,
                    param = go_param,
                )
                .ok();
            }
        }
        _ => {
            // Primitives and other types pass through directly
        }
    }

    if !out.is_empty() {
        writeln!(out).ok();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{CoreWrapper, FieldDef, MethodDef, PrimitiveType, TypeDef, TypeRef};

    fn opaque_type(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            doc: String::new(),
            cfg: None,
            fields: vec![],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            methods: vec![],
        }
    }

    fn simple_method(name: &str, return_type: TypeRef, is_static: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            doc: String::new(),
            params: vec![],
            return_type,
            is_static,
            is_async: false,
            error_type: None,
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        }
    }

    fn simple_param(name: &str, ty: TypeRef) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
        }
    }

    fn simple_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        }
    }

    #[test]
    fn test_gen_method_wrapper_opaque_free_method_emits_ptr_cast() {
        let typ = opaque_type("Client");
        let method = simple_method("close", TypeRef::Unit, false);
        let opaque: std::collections::HashSet<&str> = ["Client"].into();
        let out = gen_method_wrapper(&typ, &method, "krz", &opaque);
        assert!(out.contains("func (h *Client) Close()"));
        assert!(out.contains("unsafe.Pointer(h.ptr)"));
    }

    #[test]
    fn test_gen_param_to_c_string_param_emits_cstring() {
        let param = simple_param("name", TypeRef::String);
        let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let out = gen_param_to_c(&param, false, false, "krz", &opaque);
        assert!(out.contains("C.CString("));
        assert!(out.contains("defer C.free("));
    }

    #[test]
    fn test_gen_param_to_c_primitive_u64_emits_cgo_cast() {
        let param = simple_param("count", TypeRef::Primitive(PrimitiveType::U64));
        let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let out = gen_param_to_c(&param, false, false, "krz", &opaque);
        assert!(out.contains("C.uint64_t("));
    }

    #[test]
    fn test_gen_method_wrapper_non_opaque_static_emits_package_func() {
        let mut typ = opaque_type("Config");
        typ.is_opaque = false;
        typ.fields = vec![simple_field("value", TypeRef::String)];
        let method = simple_method("default_value", TypeRef::String, true);
        let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let out = gen_method_wrapper(&typ, &method, "krz", &opaque);
        // Static methods become package-level functions (no receiver)
        assert!(out.contains("func Config"));
    }
}
