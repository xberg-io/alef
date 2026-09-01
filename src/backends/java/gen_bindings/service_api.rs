//! Service-API codegen for the Java backend.
//!
//! Generates Java source files for service lifecycle and handler registration using Panama FFM:
//! - Service class wrapping opaque owner handles (via downcalls to C FFI symbols)
//! - Handler functional interface that accepts request JSON and returns response JSON
//! - Registration methods that build upcall stubs from handlers and invoke the C FFI
//! - Entrypoint methods (run/finalize) driving the service lifecycle
//!
//! Panama FFM Pattern:
//! - `Linker.nativeLinker()` + `SymbolLookup.libraryLookup(...)` to locate C symbols
//! - `downcallHandle()` + `FunctionDescriptor` for C function invocations
//! - `upcallStub()` + `MethodHandle` to wrap Java callbacks for C to call back into Java
//! - `Arena` for managing lifetime of callback stubs + context pointers
//! - String marshalling via `MemorySegment` + `getString()` / `CLinker.C_CHAR.byteSize()`

use crate::backends::java::template_env;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, ParamDef, RegistrationDef, ServiceDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use minijinja::context;
use std::path::PathBuf;

/// Check if a TypeRef is an opaque (surface-wrapped Named type).
fn is_opaque_metadata(ty: &TypeRef, api: &ApiSurface) -> bool {
    matches!(ty, TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n && t.is_opaque))
}

/// Map TypeRef to Java parameter type.
/// For Named types that are in the API surface, return the wrapper class name (opaque handle).
/// For String/Char/primitives, return the Java type.
fn java_type_for_metadata(ty: &TypeRef, api: &ApiSurface) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_owned(),
        TypeRef::Path => "java.nio.file.Path".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "boolean".to_owned(),
                PrimitiveType::U8 | PrimitiveType::I8 => "byte".to_owned(),
                PrimitiveType::U16 | PrimitiveType::I16 => "short".to_owned(),
                PrimitiveType::U32 | PrimitiveType::I32 => "int".to_owned(),
                PrimitiveType::U64 | PrimitiveType::I64 => "long".to_owned(),
                PrimitiveType::F32 => "float".to_owned(),
                PrimitiveType::F64 => "double".to_owned(),
                PrimitiveType::Usize | PrimitiveType::Isize => "long".to_owned(),
            }
        }
        TypeRef::Bytes => "byte[]".to_owned(),
        TypeRef::Unit => "void".to_owned(),
        TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n) => n.clone(),
        _ => "Object".to_owned(),
    }
}

fn java_layout_for_metadata(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => "ValueLayout.ADDRESS",
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "ValueLayout.JAVA_BYTE",
                PrimitiveType::U8 | PrimitiveType::I8 => "ValueLayout.JAVA_BYTE",
                PrimitiveType::U16 | PrimitiveType::I16 => "ValueLayout.JAVA_SHORT",
                PrimitiveType::U32 | PrimitiveType::I32 => "ValueLayout.JAVA_INT",
                PrimitiveType::U64 | PrimitiveType::I64 => "ValueLayout.JAVA_LONG",
                PrimitiveType::F32 => "ValueLayout.JAVA_FLOAT",
                PrimitiveType::F64 => "ValueLayout.JAVA_DOUBLE",
                PrimitiveType::Usize | PrimitiveType::Isize => "ValueLayout.JAVA_LONG",
            }
        }
        TypeRef::Named(_) => "ValueLayout.JAVA_LONG",
        _ => "ValueLayout.ADDRESS",
    }
}

/// Build a Vec of (layout, param_name) tuples for Jinja emission.
/// Jinja template is responsible for commas and newlines.
fn descriptor_layouts_vec(params: &[ParamDef]) -> Vec<(String, String)> {
    params
        .iter()
        .map(|param| {
            (
                java_layout_for_metadata(&param.ty).to_owned(),
                param.name.to_lower_camel_case(),
            )
        })
        .collect()
}

fn metadata_setup(param: &ParamDef) -> String {
    let param_name = param.name.to_lower_camel_case();
    let template = match param.ty {
        TypeRef::String | TypeRef::Char => Some("marshal_string.jinja"),
        TypeRef::Path => Some("marshal_path.jinja"),
        _ => None,
    };
    template.map_or_else(String::new, |name| {
        template_env::render(
            name,
            // The service class owns a shared `Arena.ofShared()` for upcall stubs, which must
            // outlive the call. Metadata strings must not go there: the FFI copies them into an
            // owned Rust `String` before returning, so allocating them in the shared arena leaks
            // one buffer per call until `close()`. `callArena` is the per-call confined arena
            // opened by every service downcall template. ~keep
            context! {
                cname => format!("c{param_name}"),
                name => param_name,
                arena => "callArena",
            },
        )
    })
}

fn metadata_arg_expr(param: &ParamDef, api: &ApiSurface) -> String {
    let param_name = param.name.to_lower_camel_case();
    if is_opaque_metadata(&param.ty, api) {
        format!("{param_name}.handle().address()")
    } else if matches!(param.ty, TypeRef::String | TypeRef::Char | TypeRef::Path) {
        format!("c{param_name}")
    } else if matches!(param.ty, TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)) {
        format!("(byte) ({param_name} ? 1 : 0)")
    } else {
        param_name
    }
}

/// Whether a `Finalize` entrypoint's return value survives the crossing.
///
/// The C ABI is the authority here, not this backend: `backends::ffi::gen_bindings::service_api`
/// picks `AlefHandle` as the entrypoint return type only for a `Named` type this surface wraps,
/// and renders `i32` for everything else — with `1` on the error path, which is what proves the
/// `i32` is a *status code* rather than the user's value. A primitive/string/bytes `Finalize`
/// return is therefore dropped, never carried, so blessing it here would emit a Java method that
/// hands the caller a status code dressed up as their result. ~keep
fn finalize_return_representable(return_type: &TypeRef, api: &ApiSurface) -> bool {
    match return_type {
        TypeRef::Unit => true,
        TypeRef::Named(name) => api.types.iter().any(|typ| typ.name == *name),
        _ => false,
    }
}

/// Reject a service parameter the C ABI cannot carry.
///
/// `locator` reads `{Service}.{member}[.{variant}].{param}` so a failure names the declaration.
fn validate_service_param(locator: &str, param: &ParamDef) -> anyhow::Result<()> {
    if param.optional {
        anyhow::bail!("{locator}: optional service parameters are unsupported — the C ABI carries no presence flag");
    }
    match &param.ty {
        TypeRef::Bytes => anyhow::bail!(
            "{locator}: `bytes` service parameters are unsupported — the C ABI passes `*const u8` \
             with no length carrier alongside it"
        ),
        TypeRef::Named(name) => anyhow::bail!(
            "{locator}: named parameters are unsupported until the generated C header declares a \
             `{name}` carrier this runtime can marshal"
        ),
        _ => Ok(()),
    }
}

/// Reject a callback wire type the handler bridge cannot marshal as JSON.
fn validate_callback_wire_type(
    api: &ApiSurface,
    locator: &str,
    role: &str,
    wire_type: Option<&str>,
) -> anyhow::Result<()> {
    let Some(name) = wire_type else {
        anyhow::bail!("{locator}: the callback {role} type is missing, so the handler bridge has nothing to marshal");
    };
    if api.types.iter().any(|typ| typ.name == name && !typ.has_serde) {
        anyhow::bail!(
            "{locator}: callback {role} type {name} does not derive serde, so the handler bridge \
             cannot marshal it as JSON"
        );
    }
    Ok(())
}

fn validate_registration(api: &ApiSurface, service: &ServiceDef, registration: &RegistrationDef) -> anyhow::Result<()> {
    let locator = format!("{}.{}", service.name, registration.method);
    let contract = api
        .handler_contracts
        .iter()
        .find(|contract| contract.trait_name == registration.callback_contract)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{locator}: callback contract {} is not declared on this surface",
                registration.callback_contract
            )
        })?;
    validate_callback_wire_type(api, &locator, "request", contract.wire_request_type.as_deref())?;
    validate_callback_wire_type(api, &locator, "response", contract.wire_response_type.as_deref())?;

    for param in &registration.metadata_params {
        validate_service_param(&format!("{locator}.{}", param.name), param)?;
    }

    for variant in &registration.variants {
        let variant_locator = format!("{locator}.{}", variant.name);
        if variant.wrapper_call.is_none() {
            anyhow::bail!(
                "{variant_locator}: registration variants without an FFI wrapper constructor are \
                 unsupported — there is no C symbol to bind"
            );
        }
        for param in &variant.signature_params {
            validate_service_param(&format!("{variant_locator}.{}", param.name), param)?;
        }
    }
    Ok(())
}

/// Fail generation loudly for every service shape whose value the C ABI would silently drop.
///
/// A Finalize entrypoint returning anything but `()` or a surface-wrapped type crosses the C
/// ABI as an `i32` status code, so its value is dropped with no diagnostic. ~keep
fn validate_service_abi(api: &ApiSurface, service: &ServiceDef) -> anyhow::Result<()> {
    for registration in &service.registrations {
        validate_registration(api, service, registration)?;
    }

    for entrypoint in &service.entrypoints {
        let locator = format!("{}.{}", service.name, entrypoint.method);
        for param in &entrypoint.params {
            validate_service_param(&format!("{locator}.{}", param.name), param)?;
        }
        if matches!(entrypoint.kind, EntrypointKind::Finalize)
            && !finalize_return_representable(&entrypoint.return_type, api)
        {
            anyhow::bail!("{locator} return: a Finalize entrypoint may only return `()` or a type this surface wraps");
        }
    }
    Ok(())
}

fn metadata_arg_comment(param: &ParamDef, api: &ApiSurface, default_comment: &str) -> String {
    if is_opaque_metadata(&param.ty, api) {
        "opaque handle".to_owned()
    } else {
        default_comment.to_owned()
    }
}

/// Generate the idiomatic Java service class wrapper using Panama FFM.
///
/// The class exposes:
/// - Constructor that invokes the C FFI `{prefix}_{service}_new()` via downcall
/// - Registration methods that build upcall stubs from handlers and register them
/// - Run/Finalize entrypoint methods that invoke C FFI entrypoint downcalls
/// - AutoCloseable interface with close() to invoke the C FFI `_free()` downcall
/// - All Panama FFM binding details (Linker, downcallHandle, FunctionDescriptor, etc.)
fn gen_service_class(api: &ApiSurface, service: &ServiceDef, package: &str, config: &ResolvedCrateConfig) -> String {
    let mut out = String::new();

    let class_name = &service.name;
    let service_snake = service.name.to_snake_case();
    let ffi_prefix = config.ffi_prefix().to_lowercase();

    let mut bindings_doc = String::new();
    for reg in &service.registrations {
        bindings_doc.push_str(&template_env::render(
            "service_binding_doc_registration.jinja",
            context! {
                ffi_prefix => &ffi_prefix,
                service_snake => &service_snake,
                method_snake => reg.method.to_snake_case(),
            },
        ));
    }
    for ep in &service.entrypoints {
        bindings_doc.push_str(&template_env::render(
            "service_binding_doc_entrypoint.jinja",
            context! {
                ffi_prefix => &ffi_prefix,
                service_snake => &service_snake,
                method_snake => ep.method.to_snake_case(),
            },
        ));
    }

    out.push_str(&template_env::render(
        "service_class_header.jinja",
        context! {
            package => package,
            service_name => &service.name,
            service_snake => &service_snake,
            ffi_prefix => &ffi_prefix,
            bindings_doc => bindings_doc,
            class_name => class_name,
        },
    ));

    out.push_str(&template_env::render(
        "service_constructor.jinja",
        context! {
            service_name => &service.name,
            class_name => class_name,
            ffi_prefix => &ffi_prefix,
            service_snake => &service_snake,
        },
    ));

    for reg in &service.registrations {
        let reg_method = &reg.method;
        let reg_method_camel = reg_method.to_upper_camel_case();
        let reg_method_snake = reg_method.to_snake_case();

        let mut metadata_docs = String::new();
        let mut metadata_signature = String::new();
        for meta_param in &reg.metadata_params {
            let java_type = java_type_for_metadata(&meta_param.ty, api);
            let param_name = meta_param.name.to_lower_camel_case();
            metadata_docs.push_str(&template_env::render(
                "service_metadata_param_doc.jinja",
                context! {
                    param_name => &param_name,
                    java_type => &java_type,
                },
            ));
            let signature_param = template_env::render(
                "service_metadata_signature_param.jinja",
                context! {
                    java_type => &java_type,
                    param_name => &param_name,
                },
            );
            metadata_signature.push_str(signature_param.trim_end());
        }

        let descriptor_layouts_vec = descriptor_layouts_vec(&reg.metadata_params);
        let metadata_setup: String = reg.metadata_params.iter().map(metadata_setup).collect();
        let invoke_args_vec: Vec<_> = reg
            .metadata_params
            .iter()
            .map(|meta_param| {
                (
                    metadata_arg_expr(meta_param, api),
                    metadata_arg_comment(meta_param, api, "metadata"),
                )
            })
            .collect();

        out.push_str(&template_env::render(
            "service_registration_method.jinja",
            context! {
                reg_method => reg_method,
                ffi_prefix => &ffi_prefix,
                service_snake => &service_snake,
                reg_method_snake => &reg_method_snake,
                metadata_docs => metadata_docs,
                method_name => format!("register{class_name}{reg_method_camel}"),
                metadata_signature => metadata_signature,
                class_name => class_name,
                descriptor_layouts => descriptor_layouts_vec,
                metadata_setup => metadata_setup,
                invoke_args => invoke_args_vec,
            },
        ));
    }

    for reg in &service.registrations {
        for variant in reg.variants.iter().filter(|variant| variant.wrapper_call.is_some()) {
            let variant_method_name = variant.name.to_lower_camel_case();
            let ffi_symbol = format!("{}_{}_{}", ffi_prefix, service_snake, variant.name.to_snake_case());
            let doc = variant.doc.clone();
            let signature_params = variant
                .signature_params
                .iter()
                .map(|param| {
                    format!(
                        "{} {}",
                        java_type_for_metadata(&param.ty, api),
                        param.name.to_lower_camel_case()
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let descriptor_layouts = descriptor_layouts_vec(&variant.signature_params);
            let metadata_setup: String = variant.signature_params.iter().map(metadata_setup).collect();
            let invoke_args: Vec<String> = variant
                .signature_params
                .iter()
                .map(|param| metadata_arg_expr(param, api))
                .collect();

            let ctx = context! {
                method_name => variant_method_name.clone(),
                variant_name_display => variant.name.to_lower_camel_case(),
                ffi_symbol => ffi_symbol.clone(),
                doc => doc,
                class_name => class_name,
                signature_params => signature_params,
                descriptor_layouts => descriptor_layouts,
                metadata_setup => metadata_setup,
                invoke_args => invoke_args,
            };

            let rendered = template_env::render("registration_variant.java.jinja", ctx);
            out.push_str(&rendered);
            out.push_str("\n\n");
        }
    }

    for ep in &service.entrypoints {
        if matches!(ep.kind, EntrypointKind::Finalize) && !finalize_return_representable(&ep.return_type, api) {
            continue;
        }
        let ep_method = &ep.method;
        let ep_method_snake = ep_method.to_snake_case();

        let returns_opaque =
            matches!(&ep.return_type, TypeRef::Named(name) if api.types.iter().any(|typ| typ.name == *name));
        let return_type = if returns_opaque { "long" } else { "void" };

        let params_signature = ep
            .params
            .iter()
            .map(|param| {
                let java_type = java_type_for_metadata(&param.ty, api);
                let param_name = param.name.to_lower_camel_case();
                format!("{java_type} {param_name}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let return_layout = if returns_opaque {
            "                ValueLayout.JAVA_LONG,   // return AlefHandle\n"
        } else {
            "                ValueLayout.JAVA_INT,    // return int status\n"
        };
        let descriptor_layouts_vec = descriptor_layouts_vec(&ep.params);
        let metadata_setup: String = ep.params.iter().map(metadata_setup).collect();
        let invoke_args_vec: Vec<String> = ep
            .params
            .iter()
            .map(|param| {
                if is_opaque_metadata(&param.ty, api) {
                    format!("{}.handle().address()", param.name.to_lower_camel_case())
                } else if matches!(param.ty, TypeRef::String | TypeRef::Char | TypeRef::Path) {
                    format!("c{}", param.name.to_lower_camel_case())
                } else {
                    param.name.to_lower_camel_case()
                }
            })
            .collect();

        out.push_str(&template_env::render(
            "service_entrypoint_method.jinja",
            context! {
                ep_method => ep_method,
                ffi_prefix => &ffi_prefix,
                service_snake => &service_snake,
                ep_method_snake => &ep_method_snake,
                return_type => return_type,
                params_signature => params_signature,
                return_layout => return_layout,
                returns_opaque => returns_opaque,
                descriptor_layouts => descriptor_layouts_vec,
                metadata_setup => metadata_setup,
                invoke_args => invoke_args_vec,
            },
        ));
    }

    out.push_str(&template_env::render(
        "service_close.jinja",
        context! {
            ffi_prefix => &ffi_prefix,
            service_snake => &service_snake,
        },
    ));

    out
}

/// Generate the @FunctionalInterface Callable interface.
///
/// A simple interface that handlers must implement to be passed to registration methods.
fn gen_callable_interface(package: &str) -> String {
    template_env::render("service_callable_interface.jinja", context! { package => package })
}

/// Generate all service-API files for the Java backend.
///
/// Returns Java source files using Panama FFM:
/// - One service class per [`ServiceDef`] (Panama downcalls + upcalls)
/// - One Callable interface (shared)
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }
    for service in &api.services {
        validate_service_abi(api, service)?;
    }
    let package = config.java_package();
    let package_path = package.replace('.', "/");

    let output_dir = config
        .output_for("java")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "packages/java/src/main/java/".to_string());

    let base_path = if output_dir.ends_with(&package_path) || output_dir.ends_with(&format!("{}/", package_path)) {
        PathBuf::from(&output_dir)
    } else {
        PathBuf::from(&output_dir).join(&package_path)
    };

    let mut files = Vec::new();

    // These two templates used to open with a hand-written `// Auto-generated by alef`
    // banner, which `content_has_alef_marker` does not recognize (it matches
    // "auto-generated by alef" / "Generated by alef", both case-sensitively). A markable
    // `.java` file with no recognized marker is skipped by `finalize_hashes` and refused
    // by `write_files_report`'s ownership guard on every later regeneration, so the banner
    // is gone and the pipeline prepends the real header instead. ~keep
    for service in &api.services {
        let service_class = gen_service_class(api, service, &package, config);
        files.push(GeneratedFile {
            path: base_path.join(format!("{}.java", service.name)),
            content: service_class,
            generated_header: true,
        });
    }

    files.push(GeneratedFile {
        path: base_path.join("Callable.java"),
        content: gen_callable_interface(&package),
        generated_header: true,
    });

    Ok(files)
}

#[cfg(test)]
mod tests;
