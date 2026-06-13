//! Service-API codegen for the C# backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **P/Invoke declarations** — [`DllImport`] stubs matching the C FFI contract
//!    (handlers, registration, entrypoints).
//! 2. **Service class** — An idiomatic C# wrapper that uses P/Invoke to invoke
//!    the Rust service, with registration methods and run/finalize entrypoints.
//!
//! The C# service class exposes:
//! - A constructor mirroring [`ServiceDef::constructor`].
//! - Configurator methods from [`ServiceDef::configurators`].
//! - Registration methods from [`ServiceDef::registrations`] that accept C# delegates
//!   and marshal them via `[UnmanagedCallersOnly]` trampolines + `GCHandle`.
//! - Entrypoint methods (run/finalize) from [`ServiceDef::entrypoints`].
//!
//! All names and signatures are derived entirely from the [`ApiSurface`] IR — no
//! transport- or domain-specific assumptions are made anywhere in this module.

use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointDef, ServiceDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use std::path::PathBuf;

/// Whether an entrypoint's return type can be represented over the C ABI.
/// Opaque types are representable only when this surface wraps them.
/// Unit/primitive/string/bytes/Named opaques are representable;
/// everything else (foreign framework types) is not representable.
fn entrypoint_return_representable(ep: &EntrypointDef, api: &ApiSurface) -> bool {
    match &ep.return_type {
        TypeRef::Unit | TypeRef::String | TypeRef::Char | TypeRef::Primitive(_) | TypeRef::Bytes => true,
        TypeRef::Named(n) => api.types.iter().any(|t| t.name == *n && t.is_opaque),
        _ => false,
    }
}

/// Map TypeRef to C# type name for metadata parameters and return types.
/// For opaque types in this surface, returns the C# wrapper class name (e.g., "GraphQLRouteConfig").
fn csharp_type_for_metadata(ty: &TypeRef, api: &ApiSurface) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "string".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::U8 => "byte".to_owned(),
                PrimitiveType::U16 => "ushort".to_owned(),
                PrimitiveType::U32 => "uint".to_owned(),
                PrimitiveType::U64 => "ulong".to_owned(),
                PrimitiveType::I8 => "sbyte".to_owned(),
                PrimitiveType::I16 => "short".to_owned(),
                PrimitiveType::I32 => "int".to_owned(),
                PrimitiveType::I64 => "long".to_owned(),
                PrimitiveType::F32 => "float".to_owned(),
                PrimitiveType::F64 => "double".to_owned(),
                PrimitiveType::Usize => "nuint".to_owned(),
                PrimitiveType::Isize => "nint".to_owned(),
            }
        }
        TypeRef::Bytes => "byte[]".to_owned(),
        TypeRef::Unit => "void".to_owned(),
        TypeRef::Named(name) => {
            // Check if this is an opaque type in the surface
            if api.types.iter().any(|t| t.name == *name && t.is_opaque) {
                csharp_type_name(name)
            } else {
                "string".to_owned() // Fallback for non-opaque Named or unknown types
            }
        }
        _ => "string".to_owned(), // Fallback for complex types
    }
}

fn metadata_param_decl_list(params: &[crate::core::ir::ParamDef], api: &ApiSurface) -> String {
    params
        .iter()
        .map(|param| {
            let ty = csharp_type_for_metadata(&param.ty, api);
            let name = param.name.to_lower_camel_case();
            format!("{ty} {name}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn handle_aware_arg(ty: &TypeRef, name: &str, api: &ApiSurface) -> String {
    if matches!(ty, TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n && t.is_opaque)) {
        format!("{name}.Handle")
    } else {
        name.to_owned()
    }
}

fn handle_aware_arg_lines(params: &[crate::core::ir::ParamDef], api: &ApiSurface, indent: &str) -> String {
    params
        .iter()
        .map(|param| {
            let name = param.name.to_lower_camel_case();
            let arg = handle_aware_arg(&param.ty, &name, api);
            format!(",\n{indent}{arg}")
        })
        .collect::<String>()
}

fn native_type_for_metadata(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::String | TypeRef::Char => "string",
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "int",
                PrimitiveType::U8 => "byte",
                PrimitiveType::U16 => "ushort",
                PrimitiveType::U32 => "uint",
                PrimitiveType::U64 => "ulong",
                PrimitiveType::I8 => "sbyte",
                PrimitiveType::I16 => "short",
                PrimitiveType::I32 => "int",
                PrimitiveType::I64 => "long",
                PrimitiveType::F32 => "float",
                PrimitiveType::F64 => "double",
                PrimitiveType::Usize => "nuint",
                PrimitiveType::Isize => "nint",
            }
        }
        _ => "IntPtr",
    }
}

fn pinvoke_param_lines(params: &[crate::core::ir::ParamDef]) -> String {
    params
        .iter()
        .map(|param| {
            let c_type = native_type_for_metadata(&param.ty);
            format!(",\n        {c_type} {}", param.name)
        })
        .collect::<String>()
}

// ──────────────────────────────────────────────────── C# Service Output ──

/// Generate the idiomatic C# service class wrapper.
///
/// The class exposes:
/// - Constructor reflecting the service's Rust constructor
/// - Configurators as fluent builder methods
/// - Registration methods that accept C# delegates
/// - Run/Finalize entrypoint methods
fn gen_service_cs(api: &ApiSurface, service: &ServiceDef, namespace: &str, prefix: &str) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::new();

    // Service class
    let class_name = to_csharp_name(&service.name);
    out.push_str(&render(
        "service_class_header.jinja",
        minijinja::context! {
            namespace,
            service_name => &service.name,
            class_name,
        },
    ));

    // Constructor
    {
        let ctor = &service.constructor;
        let params_decl = metadata_param_decl_list(&ctor.params, api);
        let native_new = format!("{}_{}_new", prefix.to_lowercase(), service.name.to_snake_case());
        out.push_str(&render(
            "service_constructor.jinja",
            minijinja::context! {
                service_name => &service.name,
                class_name,
                params_decl,
                native_new,
            },
        ));
    }

    // Configurator methods
    for method in &service.configurators {
        let method_name = &method.name;
        let params_decl = metadata_param_decl_list(&method.params, api);
        out.push_str(&render(
            "service_configurator_method.jinja",
            minijinja::context! {
                class_name,
                method_name,
                params_decl,
            },
        ));
    }

    // Registration methods
    for reg in &service.registrations {
        let reg_method = &reg.method;
        let service_snake = service.name.to_snake_case();
        let metadata_params = metadata_param_decl_list(&reg.metadata_params, api);
        let native_method = format!(
            "{}_{}_register_{}",
            prefix.to_lowercase(),
            service_snake,
            reg_method.to_snake_case()
        );
        let arg_lines = handle_aware_arg_lines(&reg.metadata_params, api, "            ");
        out.push_str(&render(
            "service_registration_method.jinja",
            minijinja::context! {
                method_name => reg_method,
                metadata_params,
                native_method,
                arg_lines,
            },
        ));

        // Registration variants
        for variant in &reg.variants {
            let variant_method_name = variant.name.to_upper_camel_case();
            let variant_fn_name = variant.name.to_snake_case();
            let doc = variant
                .doc
                .clone()
                .unwrap_or_else(|| format!("Register a handler via the {} variant.", variant.name));
            let signature_params = metadata_param_decl_list(&variant.signature_params, api);
            let native_method = format!("{}_{}_{}", prefix.to_lowercase(), service_snake, variant_fn_name);
            let arg_lines = handle_aware_arg_lines(&variant.signature_params, api, "            ");
            out.push_str(&render(
                "service_variant_registration_method.jinja",
                minijinja::context! {
                    method_name => variant_method_name,
                    doc,
                    signature_params,
                    native_method,
                    arg_lines,
                },
            ));
        }
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        let ep_method = &ep.method;
        let service_snake = service.name.to_snake_case();

        // Check if return type is representable (skip finalize if not)
        if !entrypoint_return_representable(ep, api) {
            continue;
        }

        // Mirror the C ABI: an entrypoint returns an opaque handle (IntPtr) when its return type is
        // an opaque this surface wraps, otherwise an i32 status code. The async-ness of the Rust
        // method does not change the C return shape.
        let returns_opaque =
            matches!(&ep.return_type, TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n && t.is_opaque));
        let return_type = if returns_opaque { "IntPtr" } else { "int" };
        let params_decl = metadata_param_decl_list(&ep.params, api);
        let native_method = format!(
            "{}_{}_ep_{}",
            prefix.to_lowercase(),
            service_snake,
            ep_method.to_snake_case()
        );
        let arg_lines = handle_aware_arg_lines(&ep.params, api, "            ");
        out.push_str(&render(
            "service_entrypoint_method.jinja",
            minijinja::context! {
                method_name => ep_method,
                return_type,
                params_decl,
                native_method,
                arg_lines,
            },
        ));
    }

    // Destructor / Dispose
    let service_snake = service.name.to_snake_case();
    let native_free = format!("{}_{}_free", prefix.to_lowercase(), service_snake);
    out.push_str(&render(
        "service_dispose_method.jinja",
        minijinja::context! { native_free },
    ));
    out.push_str(&render("service_handler_trampoline.jinja", minijinja::context! {}));

    out.push_str("}\n\n"); // Close class
    out.push_str("}\n"); // Close namespace

    out
}

/// Generate P/Invoke declarations for native service functions.
///
/// Mirrors the C FFI contract exactly: constructor/destructor, registration functions,
/// and entrypoint functions with their exact signatures and names.
/// Opaque metadata parameters are marshalled as IntPtr (handle);
/// other Named/complex types are not expected in P/Invoke metadata.
fn gen_native_methods_cs(api: &ApiSurface, namespace: &str, prefix: &str) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::new();
    out.push_str(&render(
        "service_native_methods_header.jinja",
        minijinja::context! { namespace },
    ));

    // Service constructors and destructors
    for service in &api.services {
        let service_snake = service.name.to_snake_case();

        // Constructor
        let dll_name = format!("{}_ffi", prefix.to_lowercase());
        out.push_str(&render(
            "service_native_ctor_free.jinja",
            minijinja::context! {
                dll_name,
                new_method => format!("{}_{}_new", prefix.to_lowercase(), service_snake),
                free_method => format!("{}_{}_free", prefix.to_lowercase(), service_snake),
            },
        ));

        // Registration functions
        for reg in &service.registrations {
            let reg_method_snake = reg.method.to_snake_case();
            out.push_str(&render(
                "service_pinvoke_declaration.jinja",
                minijinja::context! {
                    dll_name => format!("{}_ffi", prefix.to_lowercase()),
                    return_type => "int",
                    method_name => format!("{}_{}_register_{}", prefix.to_lowercase(), service_snake, reg_method_snake),
                    base_params => "        IntPtr owner,\n        HandlerCallback callback,\n        IntPtr ctx",
                    param_lines => pinvoke_param_lines(&reg.metadata_params),
                },
            ));

            // Variant P/Invoke declarations
            for variant in &reg.variants {
                let variant_fn_name = variant.name.to_snake_case();
                out.push_str(&render(
                    "service_pinvoke_declaration.jinja",
                    minijinja::context! {
                        dll_name => format!("{}_ffi", prefix.to_lowercase()),
                        return_type => "int",
                        method_name => format!("{}_{}_{}", prefix.to_lowercase(), service_snake, variant_fn_name),
                        base_params => "        IntPtr owner,\n        HandlerCallback callback,\n        IntPtr ctx",
                        param_lines => pinvoke_param_lines(&variant.signature_params),
                    },
                ));
            }
        }

        // Entrypoint functions
        for ep in &service.entrypoints {
            // Skip non-representable finalize entrypoints (e.g., foreign framework returns)
            if !entrypoint_return_representable(ep, api) {
                continue;
            }

            let ep_method_snake = ep.method.to_snake_case();
            // Mirror the C ABI exactly: the ffi entrypoint glue returns `*mut T` for a finalize
            // whose return this surface wraps (IntPtr), otherwise an `i32` status code — never the
            // Rust method's nominal return type (e.g. Unit/String are still i32 status over C).
            let returns_opaque =
                matches!(&ep.return_type, TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n && t.is_opaque));
            let return_type = if returns_opaque { "IntPtr" } else { "int" };
            out.push_str(&render(
                "service_pinvoke_declaration.jinja",
                minijinja::context! {
                    dll_name => format!("{}_ffi", prefix.to_lowercase()),
                    return_type,
                    method_name => format!("{}_{}_ep_{}", prefix.to_lowercase(), service_snake, ep_method_snake),
                    base_params => "        IntPtr owner",
                    param_lines => pinvoke_param_lines(&ep.params),
                },
            ));
        }
    }

    out.push_str("}\n\n"); // Close class
    out.push_str("}\n"); // Close namespace

    out
}

// ──────────────────────────────────────────────────── public entry point ──

/// Generate all service-API files for the C# backend.
///
/// Returns two `GeneratedFile`s per non-empty service list:
/// - One C# service class file per service
/// - One P/Invoke native methods file (shared across all services)
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    let namespace = config.csharp_namespace();
    let prefix = config.ffi_prefix();

    let output_dir = config
        .output_paths
        .get("csharp")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "packages/csharp/".to_owned());

    let base_path = PathBuf::from(&output_dir).join(namespace.replace('.', "/"));

    let mut files = Vec::new();

    // Generate one service class per service
    for service in &api.services {
        let service_cs = gen_service_cs(api, service, &namespace, &prefix);
        let class_name = to_csharp_name(&service.name);
        files.push(GeneratedFile {
            path: base_path.join(format!("{}.cs", class_name)),
            content: service_cs,
            generated_header: false, // Header already included
        });
    }

    // Generate P/Invoke native methods
    let native_methods = gen_native_methods_cs(api, &namespace, &prefix);
    files.push(GeneratedFile {
        path: base_path.join("ServiceNativeMethods.cs"),
        content: native_methods,
        generated_header: false,
    });

    Ok(files)
}

// ───────────────────────── Phase-C emission stubs (new IR sections) ──────────

/// Emit C# lifecycle-hook registration methods. Stub.
pub(super) fn emit_lifecycle_hooks(hooks: &[crate::core::ir::LifecycleHookDef]) -> String {
    if hooks.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "lifecycle hook emission not implemented for csharp ({} hooks)",
        hooks.len()
    );
    for _hook in hooks {}
    String::new()
}

/// Emit C# WebSocket route registration methods. Stub.
pub(super) fn emit_websocket_routes(routes: &[crate::core::ir::WebSocketRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "WebSocket route emission not implemented for csharp ({} routes)",
        routes.len()
    );
    for _route in routes {}
    String::new()
}

/// Emit C# SSE route registration methods. Stub.
pub(super) fn emit_sse_routes(routes: &[crate::core::ir::SseRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "SSE route emission not implemented for csharp ({} routes)",
        routes.len()
    );
    for _route in routes {}
    String::new()
}

/// Emit C# native error types. Stub.
pub(super) fn emit_error_types(types: &[crate::core::ir::ErrorTypeDef]) -> String {
    if types.is_empty() {
        return String::new();
    }
    tracing::debug!("error type emission not implemented for csharp ({} types)", types.len());
    for _ty in types {}
    String::new()
}

/// Aggregate stub — forwards all four new IR sections for the C# backend.
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
mod tests {
    use super::*;
    use crate::core::ir::{
        EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, RegistrationDef, ServiceDef, TypeRef,
    };

    fn make_fixture_surface() -> ApiSurface {
        let constructor = MethodDef {
            name: "new".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: "Create a new service owner.".to_owned(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let registration = RegistrationDef {
            method: "add_handler".to_owned(),
            callback_param: "handler".to_owned(),
            callback_contract: "RequestHandler".to_owned(),
            metadata_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: Some("HandlerError".to_owned()),
            doc: "Register a request handler.".to_owned(),
            variants: vec![
                crate::core::ir::RegistrationVariant {
                    name: "get".to_owned(),
                    overrides: vec![],
                    wrapper_call: None,
                    signature_params: vec![ParamDef {
                        name: "path".to_owned(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        ..ParamDef::default()
                    }],
                    doc: Some("Register a GET handler.".to_owned()),
                    style: Default::default(),
                    ..Default::default()
                },
                crate::core::ir::RegistrationVariant {
                    name: "post".to_owned(),
                    overrides: vec![],
                    wrapper_call: None,
                    signature_params: vec![ParamDef {
                        name: "path".to_owned(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        ..ParamDef::default()
                    }],
                    doc: None,
                    style: Default::default(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let run_entrypoint = EntrypointDef {
            method: "run".to_owned(),
            kind: EntrypointKind::Run,
            is_async: true,
            params: vec![ParamDef {
                name: "addr".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            error_type: Some("IoError".to_owned()),
            doc: "Start the service.".to_owned(),
        };

        let handler_contract = HandlerContractDef {
            trait_name: "RequestHandler".to_owned(),
            rust_path: "my_crate::RequestHandler".to_owned(),
            dispatch: MethodDef {
                name: "handle".to_owned(),
                params: vec![ParamDef {
                    name: "req".to_owned(),
                    ty: TypeRef::Named("RequestData".to_owned()),
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Named("Response".to_owned()),
                is_async: true,
                is_static: false,
                error_type: None,
                doc: "Handle a request.".to_owned(),
                receiver: Some(crate::core::ir::ReceiverKind::Ref),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            optional_methods: vec![],
            wire_request_type: Some("RequestData".to_owned()),
            wire_response_type: Some("Response".to_owned()),
            dispatch_extra_params: vec![],
            wire_param_name: None,
            dispatch_return_type: None,
            response_adapter: None,
            doc: "Handler contract.".to_owned(),
        };

        ApiSurface {
            crate_name: "test_crate".to_owned(),
            version: "1.0.0".to_owned(),
            services: vec![ServiceDef {
                name: "TestService".to_owned(),
                rust_path: "my_crate::TestService".to_owned(),
                constructor,
                configurators: vec![],
                registrations: vec![registration],
                entrypoints: vec![run_entrypoint],
                doc: "Test service.".to_owned(),
                cfg: None,
            }],
            handler_contracts: vec![handler_contract],
            ..ApiSurface::default()
        }
    }

    #[test]
    fn test_gen_service_cs_contains_class() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let cs = gen_service_cs(&api, service, "MyNamespace", "test");

        assert!(cs.contains("public class TestService"));
        assert!(cs.contains("private IntPtr _handle"));
        assert!(cs.contains("public TestService()"));
    }

    #[test]
    fn test_gen_service_cs_contains_registration_method() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let cs = gen_service_cs(&api, service, "MyNamespace", "test");

        assert!(cs.contains("public int add_handler("));
        assert!(cs.contains("GCHandle.Alloc(handler, GCHandleType.Normal)"));
        assert!(cs.contains("_handlerCallback"));
        assert!(cs.contains("_registeredCallbacks[ctx] = handle"));
    }

    #[test]
    fn test_gen_service_cs_contains_run_method() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let cs = gen_service_cs(&api, service, "MyNamespace", "test");

        assert!(cs.contains("public int run("));
        assert!(cs.contains("NativeMethods.test_test_service_ep_run"));
    }

    #[test]
    fn test_gen_service_cs_contains_unmanaged_callback() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let cs = gen_service_cs(&api, service, "MyNamespace", "test");

        assert!(cs.contains("public static IntPtr HandlerTrampoline"));
        assert!(cs.contains("_handlerCallback = HandlerTrampoline"));
        assert!(cs.contains("GCHandle.FromIntPtr(ctx)"));
        assert!(cs.contains("Marshal.PtrToStringUTF8"));
    }

    #[test]
    fn test_gen_service_cs_trampoline_invokes_delegate() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let cs = gen_service_cs(&api, service, "MyNamespace", "test");

        // Verify the delegate type is Func<string, string>
        assert!(
            cs.contains("if (handle.Target is Func<string, string> handler)"),
            "trampoline must cast to Func<string, string>"
        );

        // Verify the delegate is actually invoked
        assert!(
            cs.contains("handler(requestStr)"),
            "trampoline must invoke the handler with request string"
        );

        // Verify the response from the delegate is marshalled (not a hardcoded response)
        assert!(
            cs.contains("string responseStr = handler(requestStr);"),
            "trampoline must capture delegate result into responseStr"
        );

        // Verify there is NO hardcoded "{}" response or "stub" comment
        assert!(
            !cs.contains("\"stub implementation\""),
            "trampoline must not have stub implementation comment"
        );
        assert!(
            !cs.contains("string responseStr = \"{}\""),
            "trampoline must not return hardcoded {{}} response"
        );

        // Verify the marshalled response is properly allocated in native memory
        assert!(
            cs.contains("Marshal.StringToCoTaskMemUTF8(responseStr)"),
            "trampoline must marshal the response back to native memory"
        );
    }

    #[test]
    fn test_gen_native_methods_cs_contains_callback_typedef() {
        let api = make_fixture_surface();
        let native = gen_native_methods_cs(&api, "MyNamespace", "test");

        assert!(native.contains("delegate IntPtr HandlerCallback"));
        assert!(native.contains("[UnmanagedFunctionPointer(CallingConvention.Cdecl)]"));
    }

    #[test]
    fn test_gen_native_methods_cs_contains_pinvoke_decls() {
        let api = make_fixture_surface();
        let native = gen_native_methods_cs(&api, "MyNamespace", "test");

        assert!(native.contains("[DllImport("));
        assert!(native.contains("test_test_service_new()"));
        assert!(native.contains("test_test_service_free"));
        assert!(native.contains("test_test_service_register_add_handler"));
        assert!(native.contains("test_test_service_ep_run"));
    }

    #[test]
    fn test_generate_returns_files() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let files = generate(&api, &config).expect("generate should not fail");
        assert!(!files.is_empty(), "expected at least one file");

        let has_service_class = files
            .iter()
            .any(|f| f.path.to_string_lossy().contains("TestService.cs"));
        let has_native_methods = files
            .iter()
            .any(|f| f.path.to_string_lossy().contains("ServiceNativeMethods.cs"));

        assert!(has_service_class, "expected TestService.cs in output");
        assert!(has_native_methods, "expected ServiceNativeMethods.cs in output");
    }

    #[test]
    fn test_generate_returns_empty_for_no_services() {
        let api = ApiSurface::default();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let files = generate(&api, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    #[test]
    fn test_gen_service_cs_contains_variant_methods() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let cs = gen_service_cs(&api, service, "MyNamespace", "test");

        // Test Get variant method
        assert!(
            cs.contains("public int Get("),
            "expected Get variant method in service class"
        );
        assert!(
            cs.contains("Register a GET handler"),
            "expected Get variant documentation"
        );

        // Test Post variant method
        assert!(
            cs.contains("public int Post("),
            "expected Post variant method in service class"
        );
        assert!(
            cs.contains("Register a handler via the post variant"),
            "expected Post variant auto-generated documentation"
        );

        // Test variant P/Invoke calls
        assert!(
            cs.contains("NativeMethods.test_test_service_get("),
            "expected Get variant P/Invoke call"
        );
        assert!(
            cs.contains("NativeMethods.test_test_service_post("),
            "expected Post variant P/Invoke call"
        );
    }

    #[test]
    fn test_gen_native_methods_cs_contains_variant_pinvoke_decls() {
        let api = make_fixture_surface();
        let native = gen_native_methods_cs(&api, "MyNamespace", "test");

        // Test Get variant P/Invoke declaration
        assert!(
            native.contains("public static extern int test_test_service_get("),
            "expected Get variant P/Invoke declaration"
        );

        // Test Post variant P/Invoke declaration
        assert!(
            native.contains("public static extern int test_test_service_post("),
            "expected Post variant P/Invoke declaration"
        );

        // Both should have the standard parameters
        assert!(
            native.contains("IntPtr owner,") && native.contains("HandlerCallback callback,"),
            "expected variant P/Invoke to have owner and callback parameters"
        );
    }
}
