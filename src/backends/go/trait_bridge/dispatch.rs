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
    if !matches!(method.return_type, TypeRef::Unit) {
        params.push("outResult **C.char".to_string());
    }
    params.push("outError **C.char".to_string());

    out.push_str(&crate::backends::go::template_env::render(
        "trampoline_signature.jinja",
        minijinja::context! {
            name => export_name,
            params => params,
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
            gen_result_conversion(out, &method.return_type);
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
            gen_result_conversion(out, &method.return_type);
        }
    }

    out.push_str("\treturn 0  // success\n");
    out.push_str("}\n");
    out.push('\n');
}

/// Marshal a Go callback result into the C out-result slot.
///
/// The Rust FFI side decodes String/Path/Char callback returns as raw UTF-8 C strings,
/// while Json/Named/Vec/Map returns are parsed as JSON payloads. Keep those contracts
/// separate so string-like returns are not accidentally JSON-quoted and raw JSON payloads
/// are not double-encoded.
fn gen_result_conversion(out: &mut String, return_type: &TypeRef) {
    match return_type {
        TypeRef::String | TypeRef::Char | TypeRef::Path => {
            out.push_str("\tcResult := C.CString(result)\n");
            out.push_str("\t*outResult = cResult\n");
        }
        TypeRef::Json => {
            out.push_str("\tcResult := C.CString(string(result))\n");
            out.push_str("\t*outResult = cResult\n");
        }
        _ => {
            out.push_str("\tjsonBytes, _ := json.Marshal(result)\n");
            out.push_str("\tcResult := C.CString(string(jsonBytes))\n");
            out.push_str("\t*outResult = cResult\n");
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
