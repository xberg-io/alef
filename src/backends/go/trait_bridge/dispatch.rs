use super::helpers::{capitalize, gen_param_conversion, rust_to_c_type};
use crate::core::ir::{MethodDef, TypeRef};
use heck::ToPascalCase;

/// Generate one trampoline function (implementation without //export).
/// The //export declaration is in binding.go to avoid duplicate definitions.
pub(super) fn gen_trampoline(out: &mut String, trait_name: &str, trait_pascal: &str, method: &MethodDef) {
    let export_name = format!("go{}{}", trait_pascal, method.name.to_pascal_case());

    let mut params = vec!["userData unsafe.Pointer".to_string()];
    for p in &method.params {
        let c_type = rust_to_c_type(&p.ty);
        params.push(format!("{} {}", p.name, c_type));
        // Bytes params carry a companion length so the trampoline can convert via
        // unsafe.Slice rather than C.GoString (which stops at NUL bytes).
        if matches!(p.ty, TypeRef::Bytes) {
            params.push(format!("{}Len C.size_t", p.name));
        }
    }

    // Determine the return type signature based on the method's return type.
    // Simple primitives (bool, i32, u32, usize, isize, etc.) return directly and do NOT use out-result parameter.
    // Complex types (String, Vec, struct, etc.) use out-result + out-error pattern.
    let is_simple_primitive = matches!(
        &method.return_type,
        TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::I32)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::U32)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::I64)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::U64)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::Usize)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::Isize)
    );

    if is_simple_primitive {
        // Simple primitive: return the value directly, only out_error for error context.
        // No out_result parameter needed.
        params.push("outError **C.char".to_string());
    } else if !matches!(method.return_type, TypeRef::Unit) {
        // Complex return type: use out_result for the value
        params.push("outResult **C.char".to_string());
        params.push("outError **C.char".to_string());
    } else {
        // Unit return: only out_error
        params.push("outError **C.char".to_string());
    }

    // Determine the Go C type for the return value
    let go_return_type = if is_simple_primitive {
        match &method.return_type {
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => "int32_t",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::I32) => "int32_t",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::U32) => "uint32_t",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::I64) => "int64_t",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::U64) => "uint64_t",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Usize) => "uintptr_t",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Isize) => "intptr_t",
            _ => "int32_t",
        }
    } else {
        "int32_t"
    };

    out.push_str(&crate::backends::go::template_env::render(
        "trampoline_signature.jinja",
        minijinja::context! {
            name => export_name,
            params => params,
            return_type => go_return_type,
        },
    ));
    out.push('\n');

    // Retrieve the Go object from the handle
    out.push_str("\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))\n");
    out.push_str(&crate::backends::go::template_env::render(
        "handle_type_assertion.jinja",
        minijinja::context! {
            type_name => trait_name,
        },
    ));
    out.push('\n');
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn 1  // error: invalid handle\n");
    out.push_str("\t}\n");
    out.push('\n');

    // Convert C parameters to Go
    for p in &method.params {
        gen_param_conversion(out, p);
    }

    // Call the method
    let mut call_args = Vec::new();
    for p in &method.params {
        call_args.push(format!("go{}", capitalize(&p.name)));
    }

    out.push_str("\t// Call the method\n");
    if method.error_type.is_some() {
        // Method returns (value?, error)
        match &method.return_type {
            TypeRef::Unit => {
                // Just returns error
                out.push_str(&crate::backends::go::template_env::render(
                    "impl_method_call_err.jinja",
                    minijinja::context! {
                        method => method.name.to_pascal_case(),
                        args => call_args.join(", "),
                    },
                ));
                out.push('\n');
            }
            _ => {
                // Returns (value, error)
                out.push_str(&crate::backends::go::template_env::render(
                    "impl_method_call_result_err.jinja",
                    minijinja::context! {
                        method => method.name.to_pascal_case(),
                        args => call_args.join(", "),
                    },
                ));
                out.push('\n');
            }
        }
        out.push_str("\tif err != nil {\n");
        out.push_str("\t\tcErr := C.CString(err.Error())\n");
        out.push_str("\t\t*outError = cErr\n");
        out.push_str("\t\treturn 1\n");
        out.push_str("\t}\n");

        // Encode result if not Unit
        if !matches!(&method.return_type, TypeRef::Unit) {
            gen_result_conversion(out, &method.return_type, is_simple_primitive);
        }
    } else {
        // Method returns only value (no error)
        out.push_str(&crate::backends::go::template_env::render(
            "impl_method_call_result.jinja",
            minijinja::context! {
                method => method.name.to_pascal_case(),
                args => call_args.join(", "),
            },
        ));
        out.push('\n');

        // Encode result if not Unit
        if !matches!(&method.return_type, TypeRef::Unit) {
            gen_result_conversion(out, &method.return_type, is_simple_primitive);
        }
    }

    out.push_str("\treturn 0  // success\n");
    out.push_str("}\n");
    out.push('\n');
}

/// Marshal a Go callback result into the C out-result slot, or return it directly.
///
/// For simple primitives (bool, i32, etc.), return the value directly as a C type.
/// For complex types (String, Path, Vec, struct, etc.), marshal into the out_result slot.
///
/// The Rust FFI side decodes String/Path/Char callback returns as raw UTF-8 C strings,
/// while Json/Named/Vec/Map returns are parsed as JSON payloads. Keep those contracts
/// separate so string-like returns are not accidentally JSON-quoted and raw JSON payloads
/// are not double-encoded.
fn gen_result_conversion(out: &mut String, return_type: &TypeRef, is_simple_primitive: bool) {
    if is_simple_primitive {
        // For simple primitives (bool, i32, u64, etc.), cast and return directly
        match return_type {
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => {
                out.push_str("\tif callbackResult {\n");
                out.push_str("\t\treturn 1\n");
                out.push_str("\t}\n");
                out.push_str("\treturn 0\n");
            }
            TypeRef::Primitive(p) => {
                // Map primitive type to correct C casting
                use crate::core::ir::PrimitiveType::*;
                let c_type = match p {
                    Bool => "int32_t", // Handled above
                    U8 => "uint8_t",
                    U16 => "uint16_t",
                    U32 => "uint32_t",
                    U64 => "uint64_t",
                    I8 => "int8_t",
                    I16 => "int16_t",
                    I32 => "int32_t",
                    I64 => "int64_t",
                    F32 => "float",
                    F64 => "double",
                    Usize => "uintptr_t",
                    Isize => "intptr_t",
                };
                out.push_str(&format!("\treturn C.{}(callbackResult)\n", c_type));
            }
            _ => {
                // Should not happen if is_simple_primitive is correctly set
                out.push_str("\treturn 0  // error: unexpected type for simple primitive path\n");
            }
        }
    } else {
        // Complex type: marshal into out_result
        match return_type {
            TypeRef::String | TypeRef::Char | TypeRef::Path => {
                out.push_str("\tcResult := C.CString(callbackResult)\n");
                out.push_str("\t*outResult = cResult\n");
            }
            TypeRef::Json => {
                out.push_str("\tcResult := C.CString(string(callbackResult))\n");
                out.push_str("\t*outResult = cResult\n");
            }
            _ => {
                out.push_str("\tjsonBytes, _ := json.Marshal(callbackResult)\n");
                out.push_str("\tcResult := C.CString(string(jsonBytes))\n");
                out.push_str("\t*outResult = cResult\n");
            }
        }
    }
}

/// Generate trampolines for plugin methods: Name, Version, Initialize, Shutdown.
pub(super) fn gen_plugin_trampolines(out: &mut String, trait_name: &str, trait_pascal: &str) {
    // Name trampoline
    out.push_str(&crate::backends::go::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => format!("go{trait_pascal}Name"),
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "plugin_method_trampoline_header.jinja",
        minijinja::context! {
            pascal => &trait_pascal,
            method => "Name",
            params => "userData unsafe.Pointer, outResult **C.char, outError **C.char",
        },
    ));
    out.push('\n');
    out.push_str("\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))\n");
    out.push_str(&crate::backends::go::template_env::render(
        "handle_type_assertion.jinja",
        minijinja::context! {
            type_name => trait_name,
        },
    ));
    out.push('\n');
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn 1\n");
    out.push_str("\t}\n");
    out.push_str("\tname := impl.Name()\n");
    out.push_str("\tcName := C.CString(name)\n");
    out.push_str("\t*outResult = cName\n");
    out.push_str("\treturn 0\n");
    out.push_str("}\n");
    out.push('\n');

    // Version trampoline
    out.push_str(&crate::backends::go::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => format!("go{trait_pascal}Version"),
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "plugin_method_trampoline_header.jinja",
        minijinja::context! {
            pascal => &trait_pascal,
            method => "Version",
            params => "userData unsafe.Pointer, outResult **C.char, outError **C.char",
        },
    ));
    out.push('\n');
    out.push_str("\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))\n");
    out.push_str(&crate::backends::go::template_env::render(
        "handle_type_assertion.jinja",
        minijinja::context! {
            type_name => trait_name,
        },
    ));
    out.push('\n');
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn 1\n");
    out.push_str("\t}\n");
    out.push_str("\tversion := impl.Version()\n");
    out.push_str("\tcVersion := C.CString(version)\n");
    out.push_str("\t*outResult = cVersion\n");
    out.push_str("\treturn 0\n");
    out.push_str("}\n");
    out.push('\n');

    // Initialize trampoline
    out.push_str(&crate::backends::go::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => format!("go{trait_pascal}Initialize"),
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "plugin_method_trampoline_header.jinja",
        minijinja::context! {
            pascal => &trait_pascal,
            method => "Initialize",
            params => "userData unsafe.Pointer, outError **C.char",
        },
    ));
    out.push('\n');
    out.push_str("\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))\n");
    out.push_str(&crate::backends::go::template_env::render(
        "handle_type_assertion.jinja",
        minijinja::context! {
            type_name => trait_name,
        },
    ));
    out.push('\n');
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn 1\n");
    out.push_str("\t}\n");
    out.push_str("\terr := impl.Initialize()\n");
    out.push_str("\tif err != nil {\n");
    out.push_str("\t\tcErr := C.CString(err.Error())\n");
    out.push_str("\t\t*outError = cErr\n");
    out.push_str("\t\treturn 1\n");
    out.push_str("\t}\n");
    out.push_str("\treturn 0\n");
    out.push_str("}\n");
    out.push('\n');

    // Shutdown trampoline
    out.push_str(&crate::backends::go::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => format!("go{trait_pascal}Shutdown"),
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "plugin_method_trampoline_header.jinja",
        minijinja::context! {
            pascal => &trait_pascal,
            method => "Shutdown",
            params => "userData unsafe.Pointer, outError **C.char",
        },
    ));
    out.push('\n');
    out.push_str("\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))\n");
    out.push_str(&crate::backends::go::template_env::render(
        "handle_type_assertion.jinja",
        minijinja::context! {
            type_name => trait_name,
        },
    ));
    out.push('\n');
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn 1\n");
    out.push_str("\t}\n");
    out.push_str("\terr := impl.Shutdown()\n");
    out.push_str("\tif err != nil {\n");
    out.push_str("\t\tcErr := C.CString(err.Error())\n");
    out.push_str("\t\t*outError = cErr\n");
    out.push_str("\t\treturn 1\n");
    out.push_str("\t}\n");
    out.push_str("\treturn 0\n");
    out.push_str("}\n");
    out.push('\n');

    // FreeUserData trampoline — called by Rust Drop (Go 1.26+ cleanup-queue).
    // DO NOT call cgo.Handle.Delete() here: Go's cleanup-queue runs finalizers in a
    // context where they may panic if the handle is invalid or already deleted.
    // Instead, rely on explicit Unregister() calls for proper cleanup.
    out.push_str(&crate::backends::go::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => format!("go{trait_pascal}FreeUserData"),
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "plugin_free_user_data_func.jinja",
        minijinja::context! {
            pascal => &trait_pascal,
        },
    ));
    out.push('\n');
    out.push_str("\t// No-op to avoid cleanup-queue panics. Handles cleaned in Unregister().\n");
    out.push_str("}\n");
    out.push('\n');

    out.push_str(&crate::backends::go::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => format!("go{trait_pascal}FreeString"),
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "trait_free_string_func.jinja",
        minijinja::context! {
            trait_pascal => &trait_pascal,
        },
    ));
}
