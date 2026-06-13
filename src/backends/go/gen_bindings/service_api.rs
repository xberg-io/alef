//! Service-API codegen for the Go backend.
//!
//! Generates one output per `ApiSurface` with non-empty services:
//!
//! **`service.go`** — Go service wrapper that:
//! - Declares C FFI function imports (service `_new`/`_free`, registration, entrypoint functions).
//! - Defines Go handler registry (a map of opaque context indices → Go handler functions).
//! - Exports a cgo trampoline that looks up handlers in the registry and invokes them.
//! - Provides Go service struct with:
//!   - Constructor
//!   - Registration methods that store Go handlers in the registry and call C registration
//!   - `Run`/`Finalize` entrypoint methods that call C entrypoints
//!
//! All names and signatures are derived from the `ApiSurface` IR — alef is generic.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef, WrapperConstructorArg};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Find the `HandlerContractDef` by trait name in the surface.
#[allow(dead_code)]
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

/// Convert a TypeRef to a Go type string for signatures.
fn typeref_to_go_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => "string".to_owned(),
        TypeRef::Char => "byte".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::U8 => "uint8".to_owned(),
                PrimitiveType::U16 => "uint16".to_owned(),
                PrimitiveType::U32 => "uint32".to_owned(),
                PrimitiveType::U64 => "uint64".to_owned(),
                PrimitiveType::I8 => "int8".to_owned(),
                PrimitiveType::I16 => "int16".to_owned(),
                PrimitiveType::I32 => "int32".to_owned(),
                PrimitiveType::I64 => "int64".to_owned(),
                PrimitiveType::F32 => "float32".to_owned(),
                PrimitiveType::F64 => "float64".to_owned(),
                PrimitiveType::Usize => "uintptr".to_owned(),
                PrimitiveType::Isize => "intptr".to_owned(),
            }
        }
        TypeRef::Bytes => "[]byte".to_owned(),
        TypeRef::Unit => "".to_owned(), // void in Go is implicit (no return)
        TypeRef::Named(n) => n.clone(),
        TypeRef::Optional(inner) => format!("*{}", typeref_to_go_type(inner)),
        TypeRef::Vec(inner) => format!("[]{}", typeref_to_go_type(inner)),
        TypeRef::Map(k, v) => format!("map[{}]{}", typeref_to_go_type(k), typeref_to_go_type(v)),
        TypeRef::Json => "interface{}".to_owned(),
        TypeRef::Path => "string".to_owned(),
        TypeRef::Duration => "time.Duration".to_owned(),
    }
}

/// Whether an entrypoint's return type can be represented in Go.
///
/// Unit/primitives/strings map to zero values or are implicit; a `Named` type is representable only
/// when this surface wraps it (so it can cross as an opaque pointer wrapper). Anything else
/// is not representable and the entrypoint should be skipped.
fn entrypoint_return_representable(ep: &crate::core::ir::EntrypointDef, api: &ApiSurface) -> bool {
    match &ep.return_type {
        TypeRef::Unit | TypeRef::String | TypeRef::Char | TypeRef::Primitive(_) | TypeRef::Bytes => true,
        TypeRef::Named(n) => api.types.iter().any(|t| t.name == *n),
        _ => false,
    }
}

// ──────────────────────────────────────────────────────────────── Go output ──

/// Generate the Go service module (`service.go`).
///
/// For each service this emits:
/// - C FFI imports for service `_new`/`_free`, registration, and entrypoint functions.
/// - A handler registry map keyed by context index.
/// - A cgo trampoline function matching the C callback typedef signature.
/// - A Go struct mirroring the service (constructor, registration methods, entrypoints).
fn gen_service_go(api: &ApiSurface, config: &ResolvedCrateConfig, pkg_name: &str, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let ffi_header = config.ffi_header_name();

    out.push_str(&crate::backends::go::template_env::render(
        "service_file_preamble.jinja",
        minijinja::context! {
            pkg_name => pkg_name,
            ffi_header => ffi_header,
        },
    ));

    // Generate C function references (now more of a comment section, not real FFI decls)
    out.push_str("// ──────────────────────────────────────────── Service Definitions ──\n\n");
    for service in &api.services {
        gen_service_c_imports_comment(&mut out, service, api, ffi_prefix);
    }

    // Generate the handler registry and trampoline
    out.push_str("// ──────────────────────────────────────────── Handler Registry ──\n\n");
    gen_handler_registry(&mut out);

    // Generate Go service structs and methods
    out.push_str("// ──────────────────────────────────────────── Go Service API ──\n\n");
    for service in &api.services {
        gen_service_struct(&mut out, service, api, ffi_prefix, api);
    }

    out
}

/// Generate documentation comments for one service's C FFI functions.
///
/// This is purely informational — the actual C declarations come from the header
/// included in the cgo preamble. We emit a comment block for readability.
fn gen_service_c_imports_comment(out: &mut String, service: &ServiceDef, _api: &ApiSurface, ffi_prefix: &str) {
    let service_snake = service.name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();
    let registrations = service
        .registrations
        .iter()
        .map(|reg| {
            let reg_method_snake = reg.method.to_snake_case();
            let params = reg
                .metadata_params
                .iter()
                .map(|meta_param| {
                    minijinja::context! {
                        c_type => typeref_to_c_type(&meta_param.ty),
                        name => &meta_param.name,
                    }
                })
                .collect::<Vec<_>>();
            minijinja::context! {
                symbol => format!("{service_lower}_{service_snake}_register_{reg_method_snake}"),
                params => params,
            }
        })
        .collect::<Vec<_>>();
    let entrypoints = service
        .entrypoints
        .iter()
        .map(|ep| {
            let ep_name_snake = ep.method.to_snake_case();
            let params = ep
                .params
                .iter()
                .map(|ep_param| {
                    minijinja::context! {
                        c_type => typeref_to_c_type(&ep_param.ty),
                        name => &ep_param.name,
                    }
                })
                .collect::<Vec<_>>();
            minijinja::context! {
                return_c_type => typeref_to_c_type(&ep.return_type),
                symbol => format!("{service_lower}_{service_snake}_ep_{ep_name_snake}"),
                params => params,
            }
        })
        .collect::<Vec<_>>();

    out.push_str(&crate::backends::go::template_env::render(
        "service_c_imports_comment.jinja",
        minijinja::context! {
            service_name => &service.name,
            service_lower => &service_lower,
            service_snake => &service_snake,
            registrations => registrations,
            entrypoints => entrypoints,
        },
    ));
}

/// Map TypeRef to C type for function signatures in comments.
fn typeref_to_c_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => "const char*".to_owned(),
        TypeRef::Char => "char".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::U8 => "uint8_t".to_owned(),
                PrimitiveType::U16 => "uint16_t".to_owned(),
                PrimitiveType::U32 => "uint32_t".to_owned(),
                PrimitiveType::U64 => "uint64_t".to_owned(),
                PrimitiveType::I8 => "int8_t".to_owned(),
                PrimitiveType::I16 => "int16_t".to_owned(),
                PrimitiveType::I32 => "int32_t".to_owned(),
                PrimitiveType::I64 => "int64_t".to_owned(),
                PrimitiveType::F32 => "float".to_owned(),
                PrimitiveType::F64 => "double".to_owned(),
                PrimitiveType::Usize => "uintptr_t".to_owned(),
                PrimitiveType::Isize => "intptr_t".to_owned(),
            }
        }
        TypeRef::Bytes => "const uint8_t*".to_owned(),
        TypeRef::Unit => "void".to_owned(),
        _ => "void*".to_owned(),
    }
}

/// Generate a C argument expression for a parameter.
/// For opaque types and primitives, returns just the expression.
/// For DTOs in configurators, use service_c_arg_expr_with_marshal instead.
fn service_c_arg_expr(param_name: &str, ty: &TypeRef, api: &ApiSurface, upper_prefix: &str) -> String {
    match ty {
        TypeRef::String => format!("C.CString({param_name})"),
        TypeRef::Named(type_name) if api.types.iter().any(|t| t.name == *type_name) => {
            format!("(*C.{upper_prefix}{type_name})(unsafe.Pointer({param_name}.ptr))")
        }
        _ => {
            let c_type = typeref_to_c_type(ty);
            format!("{c_type}({param_name})")
        }
    }
}

/// Generate a C argument expression for a parameter with preprocessing support for DTOs.
///
/// Returns a tuple of (preprocessing_code, argument_expr) where preprocessing_code
/// is any setup needed before the call (e.g., JSON marshaling), and argument_expr
/// is the expression to pass to the C function.
fn service_c_arg_expr_with_marshal(
    param_name: &str,
    ty: &TypeRef,
    api: &ApiSurface,
    upper_prefix: &str,
    ffi_prefix: &str,
) -> (String, String) {
    match ty {
        TypeRef::String => (String::new(), format!("C.CString({param_name})")),
        TypeRef::Named(type_name) => {
            if let Some(typedef) = api.types.iter().find(|t| t.name == *type_name) {
                if typedef.is_opaque {
                    // Opaque type: access the .ptr field directly
                    (
                        String::new(),
                        format!("(*C.{upper_prefix}{type_name})(unsafe.Pointer({param_name}.ptr))"),
                    )
                } else {
                    // DTO: marshal to JSON, call _from_json, store in intermediate var
                    let var_name = format!("c_{param_name}");
                    let type_name_snake = type_name.to_snake_case();
                    let mut preprocessing = format!(
                        "\t{var_name}JSON, err := json.Marshal({param_name})\n\
                        \tif err != nil {{\n\
                        \t\treturn err\n\
                        \t}}\n\
                        \t{var_name} := C.{ffi_prefix}_{type_name_snake}_from_json(C.CString(string({var_name}JSON)))\n\
                        \tif {var_name} == nil {{\n\
                        \t\treturn errors.New(\"{type_name} config failed\")\n\
                        \t}}\n\
                        \tdefer C.{ffi_prefix}_{type_name_snake}_free({var_name})\n"
                    );
                    preprocessing = preprocessing
                        .replace("{var_name}", &var_name)
                        .replace("{param_name}", param_name)
                        .replace("{ffi_prefix}", ffi_prefix)
                        .replace("{type_name_snake}", &type_name_snake)
                        .replace("{type_name}", type_name);
                    let arg_expr = var_name.to_string();
                    (preprocessing, arg_expr)
                }
            } else {
                // Unknown named type - try to pass as is
                (
                    String::new(),
                    format!("(*C.{upper_prefix}{type_name})(unsafe.Pointer({param_name}.ptr))"),
                )
            }
        }
        _ => {
            let c_type = typeref_to_c_type(ty);
            (String::new(), format!("{c_type}({param_name})"))
        }
    }
}

fn emit_service_call_arg(out: &mut String, expr: &str) {
    out.push_str(&crate::backends::go::template_env::render(
        "service_call_arg_line.jinja",
        minijinja::context! {
            expr => expr,
        },
    ));
}

/// Generate the handler registry and cgo trampoline.
fn gen_handler_registry(out: &mut String) {
    out.push_str(&crate::backends::go::template_env::render(
        "service_handler_registry.jinja",
        minijinja::context! {},
    ));
}

/// Render a (possibly multi-line) doc string as a Go doc comment block.
///
/// Every line is prefixed with `//` so multi-paragraph docs (e.g. a Markdown `# Errors`
/// section) cannot leak un-commented source into the generated Go file.
fn go_doc_block(doc: &str) -> String {
    let mut out = String::from("//\n");
    for line in doc.trim_end().lines() {
        let line = line.trim_end();
        if line.is_empty() {
            out.push_str("//\n");
        } else {
            out.push_str(&crate::backends::go::template_env::render(
                "go_doc_block_line.jinja",
                minijinja::context! {
                    line => line,
                },
            ));
        }
    }
    out
}

/// Generate a Go service struct with constructor, registration, and entrypoint methods.
fn gen_service_struct(
    out: &mut String,
    service: &ServiceDef,
    api: &ApiSurface,
    ffi_prefix: &str,
    api_surface: &ApiSurface,
) {
    let service_name = &service.name;
    let service_snake = service_name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();
    let upper_prefix = ffi_prefix.to_uppercase();

    let doc_block = if service.doc.is_empty() {
        String::new()
    } else {
        go_doc_block(&service.doc)
    };
    out.push_str(&crate::backends::go::template_env::render(
        "service_struct.jinja",
        minijinja::context! {
            service_name => service_name,
            upper_prefix => upper_prefix,
            doc_block => doc_block,
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "service_constructor.jinja",
        minijinja::context! {
            service_name => service_name,
            service_lower => service_lower,
            service_snake => service_snake,
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "service_close_method.jinja",
        minijinja::context! {
            service_name => service_name,
            service_lower => service_lower,
            service_snake => service_snake,
            upper_prefix => upper_prefix,
        },
    ));

    // Registration methods
    for reg in &service.registrations {
        gen_registration_method(out, service, reg, api, ffi_prefix);
    }

    // Registration variant methods (e.g., Get, Post shortcuts)
    for reg in &service.registrations {
        for variant in &reg.variants {
            gen_registration_variant(out, service, reg, variant, api, ffi_prefix);
        }
    }

    // Configurator methods
    for cfg in &service.configurators {
        gen_configurator_method(out, service, cfg, api, ffi_prefix);
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        gen_entrypoint_method(out, service, ep, api_surface, ffi_prefix);
    }

    // StartBackground convenience method for non-blocking server spawn
    gen_start_background_method(out, service, ffi_prefix);
}

/// Generate a registration method for one registration.
fn gen_registration_method(
    out: &mut String,
    service: &ServiceDef,
    reg: &RegistrationDef,
    api: &ApiSurface,
    ffi_prefix: &str,
) {
    let service_name = &service.name;
    let service_snake = service_name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();
    let method_name = &reg.method;
    let method_name_pascal = method_name.to_upper_camel_case();
    let reg_method_snake = method_name.to_snake_case();

    // Build method signature with metadata params
    let mut params = vec!["handler HandlerFunc".to_owned()];
    for meta_param in &reg.metadata_params {
        let mut go_type = typeref_to_go_type(&meta_param.ty);
        // Opaque types (Named types that wrap FFI pointers) must be passed by pointer
        if let TypeRef::Named(type_name) = &meta_param.ty {
            if api.types.iter().any(|t| t.name == *type_name) {
                go_type = format!("*{}", go_type);
            }
        }
        params.push(format!("{} {}", meta_param.name, go_type));
    }
    let param_sig = params.join(", ");

    out.push_str(&crate::backends::go::template_env::render(
        "service_register_comment.jinja",
        minijinja::context! {
            method_name_pascal => &method_name_pascal,
            method_name => method_name,
        },
    ));
    if !reg.doc.is_empty() {
        out.push_str(&go_doc_block(&reg.doc));
    }

    let return_type = if reg.error_type.is_some() {
        "error".to_owned()
    } else {
        "".to_owned()
    };
    let return_sig = if !return_type.is_empty() {
        format!(" {}", return_type)
    } else {
        String::new()
    };

    let closed_return = if reg.error_type.is_some() {
        "\t\treturn errors.New(\"service is closed\")\n"
    } else {
        "\t\tpanic(\"service is closed\")\n"
    };
    out.push_str(&crate::backends::go::template_env::render(
        "service_method_header.jinja",
        minijinja::context! {
            service_name => service_name,
            method_name => format!("Register{method_name_pascal}"),
            params => &param_sig,
            return_sig => &return_sig,
            closed_return => closed_return,
        },
    ));

    // Register the handler in Go's registry
    out.push_str("\tctxID := registerHandler(handler)\n");

    // Call C registration function.
    // Pass the exported Go callback function's address as an opaque void* pointer.
    // The FFI function will transmute it back to the proper function pointer type.
    let upper_prefix = ffi_prefix.to_uppercase();
    out.push_str(&crate::backends::go::template_env::render(
        "service_registration_call_header.jinja",
        minijinja::context! {
            service_lower => &service_lower,
            service_snake => &service_snake,
            reg_method_snake => &reg_method_snake,
            upper_prefix => &upper_prefix,
            service_name => service_name,
        },
    ));

    // Add metadata params as arguments, marshaling opaque types correctly. Go requires a trailing
    // comma on every argument when the closing paren sits on its own line, so each line ends with `,`.
    for meta_param in &reg.metadata_params {
        let expr = service_c_arg_expr(&meta_param.name, &meta_param.ty, api, &upper_prefix);
        emit_service_call_arg(out, &expr);
    }
    out.push_str("\t)\n\n");

    out.push_str(&crate::backends::go::template_env::render(
        "service_registration_return.jinja",
        minijinja::context! {
            returns_error => reg.error_type.is_some(),
        },
    ));
}

/// Generate a registration variant method for one variant shortcut (e.g., Get, Post).
fn gen_registration_variant(
    out: &mut String,
    service: &ServiceDef,
    reg: &RegistrationDef,
    variant: &crate::core::ir::RegistrationVariant,
    api: &ApiSurface,
    ffi_prefix: &str,
) {
    let service_name = &service.name;
    let service_snake = service_name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();
    let variant_name_pascal = variant.name.to_upper_camel_case();
    let variant_name_snake = variant.name.to_snake_case();

    // Build method signature with variant's signature_params + handler
    let mut params = vec!["handler HandlerFunc".to_owned()];
    for sig_param in &variant.signature_params {
        let go_type = typeref_to_go_type(&sig_param.ty);
        params.push(format!("{} {}", sig_param.name, go_type));
    }
    let param_sig = params.join(", ");

    out.push_str(&crate::backends::go::template_env::render(
        "service_variant_comment.jinja",
        minijinja::context! {
            variant_name_pascal => &variant_name_pascal,
            variant_name => &variant.name,
        },
    ));
    if let Some(doc) = &variant.doc {
        out.push_str(&go_doc_block(doc));
    }

    let return_type = if reg.error_type.is_some() {
        "error".to_owned()
    } else {
        "".to_owned()
    };
    let return_sig = if !return_type.is_empty() {
        format!(" {}", return_type)
    } else {
        String::new()
    };

    let closed_return = if reg.error_type.is_some() {
        "\t\treturn errors.New(\"service is closed\")\n"
    } else {
        "\t\tpanic(\"service is closed\")\n"
    };
    out.push_str(&crate::backends::go::template_env::render(
        "service_method_header.jinja",
        minijinja::context! {
            service_name => service_name,
            method_name => &variant_name_pascal,
            params => &param_sig,
            return_sig => &return_sig,
            closed_return => closed_return,
        },
    ));

    // Register the handler in Go's registry
    out.push_str("\tctxID := registerHandler(handler)\n");

    // Call the C variant function with fixed overrides + free args
    let upper_prefix = ffi_prefix.to_uppercase();
    // The FFI exports the variant symbol as `{prefix}_{service}_{variant}` —
    // the registration method name is NOT included in the variant symbol name.
    // Get the function pointer from a public C helper function.
    out.push_str(&crate::backends::go::template_env::render(
        "service_variant_call_header.jinja",
        minijinja::context! {
            service_lower => &service_lower,
            service_snake => &service_snake,
            variant_name_snake => &variant_name_snake,
            upper_prefix => &upper_prefix,
            service_name => service_name,
        },
    ));

    // Emit the free args that follow the fixed trampoline args.
    //
    // When a wrapper_call is present the FFI function's extra args are the free
    // constructor params in declaration order (fixed params are baked in).
    // When there is no wrapper_call the extra args are the non-overridden base
    // metadata params.
    if let Some(wc) = &variant.wrapper_call {
        for arg in &wc.args {
            if let WrapperConstructorArg::Free { param } = arg {
                let expr = service_c_arg_expr(&param.name, &param.ty, api, &upper_prefix);
                emit_service_call_arg(out, &expr);
            }
        }
    } else {
        for base_param in &reg.metadata_params {
            if variant.overrides.iter().any(|o| o.param_name == base_param.name) {
                // Fixed override — baked into the FFI function; do not re-emit.
            } else if let Some(sig_param) = variant.signature_params.iter().find(|s| s.name == base_param.name) {
                let expr = service_c_arg_expr(&sig_param.name, &sig_param.ty, api, &upper_prefix);
                emit_service_call_arg(out, &expr);
            }
        }
    }
    out.push_str("\t)\n\n");

    out.push_str(&crate::backends::go::template_env::render(
        "service_registration_return.jinja",
        minijinja::context! {
            returns_error => reg.error_type.is_some(),
        },
    ));
}

/// Generate a configurator method for one configurator method.
fn gen_configurator_method(
    out: &mut String,
    service: &ServiceDef,
    cfg: &crate::core::ir::MethodDef,
    api: &ApiSurface,
    ffi_prefix: &str,
) {
    let service_name = &service.name;
    let service_snake = service_name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();
    let cfg_method_pascal = cfg.name.to_upper_camel_case();
    let cfg_method_snake = cfg.name.to_snake_case();

    // Build method signature with configurator's params
    let mut params = Vec::new();
    for cfg_param in &cfg.params {
        let go_type = typeref_to_go_type(&cfg_param.ty);
        // Opaque types must be passed by pointer
        let final_type = if let TypeRef::Named(type_name) = &cfg_param.ty {
            if api.types.iter().any(|t| t.name == *type_name) {
                format!("*{}", go_type)
            } else {
                go_type
            }
        } else {
            go_type
        };
        params.push(format!("{} {}", cfg_param.name, final_type));
    }
    let param_sig = if params.is_empty() {
        String::new()
    } else {
        params.join(", ")
    };

    out.push_str(&crate::backends::go::template_env::render(
        "service_configurator_comment.jinja",
        minijinja::context! {
            cfg_method_pascal => &cfg_method_pascal,
            cfg_name => &cfg.name,
        },
    ));
    if !cfg.doc.is_empty() {
        out.push_str(&go_doc_block(&cfg.doc));
    }

    out.push_str(&crate::backends::go::template_env::render(
        "service_method_header.jinja",
        minijinja::context! {
            service_name => service_name,
            method_name => &cfg_method_pascal,
            params => &param_sig,
            return_sig => " error",
            closed_return => "\t\treturn errors.New(\"service is closed\")\n",
        },
    ));

    let upper_prefix = ffi_prefix.to_uppercase();

    // Build configurator call with all arguments (owner + config params) as a Jinja array
    // to ensure commas are on the same line as their arguments, not orphaned.
    let mut cfg_args = Vec::new();
    let mut preprocessing = String::new();

    // First argument: owner (always present)
    cfg_args.push(minijinja::context! {
        expr => format!("(*C.{upper_prefix}{service_name}Opaque)(s.owner)"),
    });

    // Config parameters: collect preprocessing code and argument expressions
    for cfg_param in &cfg.params {
        let (pre, expr) =
            service_c_arg_expr_with_marshal(&cfg_param.name, &cfg_param.ty, api, &upper_prefix, ffi_prefix);
        preprocessing.push_str(&pre);
        cfg_args.push(minijinja::context! {
            expr => expr,
        });
    }

    if !preprocessing.is_empty() {
        out.push_str(&preprocessing);
    }

    out.push_str(&crate::backends::go::template_env::render(
        "service_configurator_call.jinja",
        minijinja::context! {
            service_lower => &service_lower,
            service_snake => &service_snake,
            cfg_method_snake => &cfg_method_snake,
            service_name => service_name,
            args => cfg_args,
        },
    ));
}

/// Generate an entrypoint method for one entrypoint.
fn gen_entrypoint_method(
    out: &mut String,
    service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    api: &ApiSurface,
    ffi_prefix: &str,
) {
    // Skip finalize entrypoints with non-representable return types (e.g., foreign framework routers).
    use crate::core::ir::EntrypointKind;
    if matches!(ep.kind, EntrypointKind::Finalize) && !entrypoint_return_representable(ep, api) {
        return;
    }
    let service_name = &service.name;
    let service_snake = service_name.to_snake_case();
    let service_lower = ffi_prefix.to_lowercase();
    let ep_method = &ep.method;
    let ep_method_pascal = ep_method.to_upper_camel_case();
    let ep_name_snake = ep_method.to_snake_case();

    // Build method signature with entrypoint params
    let mut params = vec![];
    for ep_param in &ep.params {
        let go_type = typeref_to_go_type(&ep_param.ty);
        params.push(format!("{} {}", ep_param.name, go_type));
    }
    let param_sig = if params.is_empty() {
        String::new()
    } else {
        params.join(", ")
    };

    // The C entrypoint returns either a `*mut T` opaque pointer (when this surface wraps the
    // entrypoint's return type) or an `i32` status code (0 = ok). Mirror that ABI here: expose the
    // opaque wrapper as a value; otherwise the call is status-only, reported through `error`. A
    // value type the surface does not wrap (e.g. a foreign framework type) has no C return form, so
    // it collapses to the status call rather than a bogus Go value.
    let upper_prefix = ffi_prefix.to_uppercase();
    let opaque_return = match &ep.return_type {
        TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n) => Some(n.clone()),
        _ => None,
    };
    let has_err = ep.error_type.is_some();
    let return_sig = match (&opaque_return, has_err) {
        (Some(t), true) => format!(" (*{t}, error)"),
        (Some(t), false) => format!(" *{t}"),
        (None, true) => " error".to_owned(),
        (None, false) => String::new(),
    };

    out.push_str(&crate::backends::go::template_env::render(
        "service_entrypoint_comment.jinja",
        minijinja::context! {
            ep_method_pascal => &ep_method_pascal,
            ep_method => ep_method,
        },
    ));
    if !ep.doc.is_empty() {
        out.push_str(&go_doc_block(&ep.doc));
    }

    let closed_return = match (&opaque_return, has_err) {
        (Some(_), true) => "\t\treturn nil, errors.New(\"service is closed\")\n",
        (Some(_), false) => "\t\treturn nil\n",
        (None, true) => "\t\treturn errors.New(\"service is closed\")\n",
        (None, false) => "\t\tpanic(\"service is closed\")\n",
    };
    out.push_str(&crate::backends::go::template_env::render(
        "service_method_header.jinja",
        minijinja::context! {
            service_name => service_name,
            method_name => &ep_method_pascal,
            params => &param_sig,
            return_sig => &return_sig,
            closed_return => closed_return,
        },
    ));

    // Call the C entrypoint, capturing its return when it carries a value or status.
    let capture = if opaque_return.is_some() || has_err {
        "ret := "
    } else {
        ""
    };
    out.push_str(&crate::backends::go::template_env::render(
        "service_entrypoint_call_header.jinja",
        minijinja::context! {
            capture => capture,
            service_lower => &service_lower,
            service_snake => &service_snake,
            ep_name_snake => &ep_name_snake,
            upper_prefix => &upper_prefix,
            service_name => service_name,
        },
    ));
    for ep_param in &ep.params {
        let expr = service_c_arg_expr(&ep_param.name, &ep_param.ty, api, &upper_prefix);
        emit_service_call_arg(out, &expr);
    }
    out.push_str("\t)\n");

    match (&opaque_return, has_err) {
        (Some(t), true) => {
            out.push_str(&crate::backends::go::template_env::render(
                "service_entrypoint_return_opaque_err.jinja",
                minijinja::context! {
                    ep_method => ep_method,
                    return_type => t,
                },
            ));
        }
        (Some(t), false) => {
            out.push_str(&crate::backends::go::template_env::render(
                "service_entrypoint_return_opaque.jinja",
                minijinja::context! {
                    return_type => t,
                },
            ));
        }
        (None, true) => {
            out.push_str(&crate::backends::go::template_env::render(
                "service_entrypoint_return_err.jinja",
                minijinja::context! {
                    ep_method => ep_method,
                },
            ));
        }
        (None, false) => {}
    }

    out.push_str("}\n\n");
}

// ──────────────────────────────────────────────────────────────── public entry point ──

/// Generate all service-API files for the Go backend.
///
/// Returns one `GeneratedFile` when services are present:
/// - `{output_dir}/service.go` — Go service wrapper
pub fn generate(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    pkg_name: &str,
    ffi_prefix: &str,
) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    let output_dir = {
        let mut d =
            crate::core::config::resolve_output_dir(config.output_paths.get("go"), &config.name, "packages/go/");
        if !d.ends_with('/') {
            d.push('/');
        }
        d
    };

    let service_go = gen_service_go(api, config, pkg_name, ffi_prefix);

    Ok(vec![GeneratedFile {
        path: PathBuf::from(&output_dir).join("service.go"),
        content: service_go,
        generated_header: true,
    }])
}

/// Generate a StartBackground convenience method that spawns the service in a goroutine.
///
/// This method is useful for e2e harnesses that need the server running in the background
/// while the test process continues. It blocks until the TCP socket is bound, ensuring
/// the server is reachable when the call returns.
fn gen_start_background_method(out: &mut String, service: &ServiceDef, _ffi_prefix: &str) {
    let service_name = &service.name;
    out.push_str(&crate::backends::go::template_env::render(
        "service_start_background.jinja",
        minijinja::context! {
            service_name => service_name,
        },
    ));
}

// ───────────────────────── Phase-C emission stubs (new IR sections) ──────────

/// Emit Go lifecycle-hook registration methods.
///
/// Stub — returns `""` until the Go Phase-C specialist implements
/// `app.OnRequest(fn)` / `app.PreHandler(fn)` / … generation.
pub(super) fn emit_lifecycle_hooks(hooks: &[crate::core::ir::LifecycleHookDef]) -> String {
    if hooks.is_empty() {
        return String::new();
    }
    tracing::debug!("lifecycle hook emission not implemented for go ({} hooks)", hooks.len());
    for _hook in hooks {}
    String::new()
}

/// Emit Go WebSocket route registration methods.
///
/// Stub — returns `""` until the Go Phase-C specialist implements
/// `app.WebSocket(path, handler)` generation.
pub(super) fn emit_websocket_routes(routes: &[crate::core::ir::WebSocketRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "WebSocket route emission not implemented for go ({} routes)",
        routes.len()
    );
    for _route in routes {}
    String::new()
}

/// Emit Go SSE route registration methods.
///
/// Stub — returns `""` until the Go Phase-C specialist implements
/// `app.SSE(path, producer)` generation.
pub(super) fn emit_sse_routes(routes: &[crate::core::ir::SseRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!("SSE route emission not implemented for go ({} routes)", routes.len());
    for _route in routes {}
    String::new()
}

/// Emit Go native error types.
///
/// Stub — returns `""` until the Go Phase-C specialist implements
/// typed `error` struct generation.
pub(super) fn emit_error_types(types: &[crate::core::ir::ErrorTypeDef]) -> String {
    if types.is_empty() {
        return String::new();
    }
    tracing::debug!("error type emission not implemented for go ({} types)", types.len());
    for _ty in types {}
    String::new()
}

/// Aggregate stub — forwards all four new IR sections for the Go backend.
pub(super) fn emit_new_ir_sections(api: &crate::core::ir::ApiSurface) -> String {
    let mut out = String::new();
    out.push_str(&emit_lifecycle_hooks(&api.lifecycle_hooks));
    out.push_str(&emit_websocket_routes(&api.websocket_routes));
    out.push_str(&emit_sse_routes(&api.sse_routes));
    out.push_str(&emit_error_types(&api.error_types));
    out
}

// ───────────────────────────────────────────────────────────────────── tests ──

#[cfg(test)]
mod tests;
