//! Service-API codegen for the Swift backend (bridge-based via swift-bridge 0.1.59).
//!
//! Generates two outputs per [`ServiceDef`] with non-empty registrations:
//!
//! 1. **Rust extern "Rust" declarations** — added to the `#[swift_bridge::bridge] mod ffi`
//!    block in `packages/swift/rust/src/lib.rs`:
//!    - `extern "Rust" { type <ServiceName>; }` — opaque type declaration
//!    - `extern "Rust" { #[swift_bridge(init)] fn new(...) -> <ServiceName>; }`
//!    - `extern "Rust" { fn <configurator>(...); }` per configurator
//!    - `extern "Rust" { fn <register>_via_callback(..., ctx: *mut c_void, callback: extern "C" fn(...) -> *mut u8) -> i32; }`
//!      per registration (C-callback shim)
//!    - `extern "Rust" { fn <run>(...) -> Result<(), String>; }` per entrypoint (blocking, not async)
//!
//! 2. **Swift wrapper class** at `Sources/Spikard/<ServiceName>.swift`:
//!    - Public class wrapping the swift-bridge opaque type
//!    - Idiomatic Swift methods for constructor, configurators, registration (with closure boxing), and entrypoints
//!    - Handler boxes (reference type) to cross closures via C context pointers
//!    - @convention(c) trampolines for C callback interop

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, RegistrationDef, ServiceDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase};
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Format a multi-line Rust doc as a Swift `///` block at the given column
/// indent. Every non-blank line is prefixed with `/// `; blank lines stay as
/// bare `///` so paragraph breaks survive. Includes the trailing newline.
fn format_swift_comment(text: &str, indent: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for line in trimmed.lines() {
        if line.trim().is_empty() {
            out.push_str(&pad);
            out.push_str("///\n");
        } else {
            out.push_str(&pad);
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Whether an entrypoint's return type can be represented over the C ABI as a function return.
///
/// Unit/primitive/string/bytes map to a status code or scalar; a `Named` type is representable only
/// when this surface wraps it (so it can cross as an opaque handle). Anything else is not representable.
///
/// Also rejects entrypoints whose source method was sanitized — the IR sanitizer maps unknown
/// foreign return types (e.g. `axum::Router`) to `TypeRef::String`, which would otherwise pass
/// the surface check but produce a bridge signature that doesn't match the real Rust method.
fn entrypoint_return_representable(
    ep: &crate::core::ir::EntrypointDef,
    service: &ServiceDef,
    api: &ApiSurface,
) -> bool {
    if let Some(svc_type) = api.types.iter().find(|t| t.name == service.name)
        && let Some(method) = svc_type.methods.iter().find(|m| m.name == ep.method)
        && method.sanitized
    {
        return false;
    }
    match &ep.return_type {
        TypeRef::Unit | TypeRef::String | TypeRef::Char | TypeRef::Primitive(_) | TypeRef::Bytes => true,
        TypeRef::Named(n) => api.types.iter().any(|t| t.name == *n),
        _ => false,
    }
}

/// Map a `TypeRef` to a Swift type string for function parameters.
fn typeref_to_swift_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => "String".to_owned(),
        TypeRef::Char => "Character".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "Bool".to_owned(),
                PrimitiveType::U8 => "UInt8".to_owned(),
                PrimitiveType::U16 => "UInt16".to_owned(),
                PrimitiveType::U32 => "UInt32".to_owned(),
                PrimitiveType::U64 => "UInt64".to_owned(),
                PrimitiveType::I8 => "Int8".to_owned(),
                PrimitiveType::I16 => "Int16".to_owned(),
                PrimitiveType::I32 => "Int32".to_owned(),
                PrimitiveType::I64 => "Int64".to_owned(),
                PrimitiveType::F32 => "Float".to_owned(),
                PrimitiveType::F64 => "Double".to_owned(),
                PrimitiveType::Usize => "Int".to_owned(),
                PrimitiveType::Isize => "Int".to_owned(),
            }
        }
        TypeRef::Bytes => "Data".to_owned(),
        TypeRef::Unit => "Void".to_owned(),
        TypeRef::Named(n) => n.clone(),
        _ => "String".to_owned(), // Json, Vec, Map, etc. go through JSON serialization
    }
}

// ────────────────────────────────────────────────── Rust extern "Rust" output ──

/// Map TypeRef to a Rust FFI type string.
fn typeref_to_rust_ffi_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => "String".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::U8 => "u8".to_owned(),
                PrimitiveType::U16 => "u16".to_owned(),
                PrimitiveType::U32 => "u32".to_owned(),
                PrimitiveType::U64 => "u64".to_owned(),
                PrimitiveType::I8 => "i8".to_owned(),
                PrimitiveType::I16 => "i16".to_owned(),
                PrimitiveType::I32 => "i32".to_owned(),
                PrimitiveType::I64 => "i64".to_owned(),
                PrimitiveType::F32 => "f32".to_owned(),
                PrimitiveType::F64 => "f64".to_owned(),
                PrimitiveType::Usize => "usize".to_owned(),
                PrimitiveType::Isize => "isize".to_owned(),
            }
        }
        TypeRef::Named(n) => n.clone(),
        _ => "String".to_owned(),
    }
}

/// Generate Rust extern "Rust" declarations for a service (INSIDE the bridge module).
/// These are appended to the `#[swift_bridge::bridge] mod ffi { ... }` block in lib.rs.
/// Registration callbacks are excluded — they go outside the bridge via `generate_rust_callback_c_functions`.
///
/// IMPORTANT: This emits a SINGLE consolidated `extern "Rust"` block containing the opaque type,
/// constructor, all configurators, and all entrypoints. swift-bridge 0.1.59 requires all methods
/// on an opaque type to be in a single block; splitting them across multiple blocks causes a
/// parse error: "expected path".
fn gen_service_rust_extern_blocks(service: &ServiceDef, api: &ApiSurface) -> String {
    let service_snake = service.name.to_snake_case();

    // Build configurator list for the template
    let configurators: Vec<minijinja::Value> = service
        .configurators
        .iter()
        .map(|config| {
            let config_snake = config.name.to_snake_case();
            let config_camel = config_snake.to_lower_camel_case();
            minijinja::context! {
                name => &config_snake,
                camel => &config_camel,
            }
        })
        .collect();

    // Build entrypoint list for the template (skip non-representable finalize)
    let entrypoints: Vec<minijinja::Value> = service
        .entrypoints
        .iter()
        .filter(|ep| {
            // Skip finalize entrypoints whose return type can't be represented over the C ABI.
            !matches!(ep.kind, crate::core::ir::EntrypointKind::Finalize)
                || entrypoint_return_representable(ep, service, api)
        })
        .map(|ep| {
            let ep_snake = ep.method.to_snake_case();
            let ep_camel = ep_snake.to_lower_camel_case();
            let params: Vec<minijinja::Value> = ep
                .params
                .iter()
                .map(|p| {
                    minijinja::context! {
                        name => &p.name,
                        rust_type => typeref_to_rust_ffi_type(&p.ty),
                    }
                })
                .collect();

            // Return type. swift-bridge 0.1.59 cannot parse `Result<T, E>` in extern blocks,
            // so error-returning functions return a JSON envelope string instead:
            // `{"ok": <value>}` on success or `{"err": "<message>"}` on failure.
            let return_type = match &ep.return_type {
                TypeRef::Unit => {
                    if ep.error_type.is_some() {
                        "String".to_owned()
                    } else {
                        "()".to_owned()
                    }
                }
                TypeRef::String => "String".to_owned(),
                _ => "String".to_owned(),
            };

            minijinja::context! {
                snake => &ep_snake,
                camel => &ep_camel,
                params => params,
                return_type => return_type,
            }
        })
        .collect();

    // Emit a single consolidated extern "Rust" block
    crate::backends::swift::template_env::render(
        "rust_extern_service_consolidated.rs.jinja",
        minijinja::context! {
            service_name => &service.name,
            service_snake => &service_snake,
            configurators => configurators,
            entrypoints => entrypoints,
        },
    )
}

/// Generate plain C functions for callback registration (OUTSIDE the bridge module).
/// These are emitted after the `#[swift_bridge::bridge] mod ffi { ... }` block closes.
fn gen_rust_callback_c_functions_for_service(service: &ServiceDef) -> String {
    let mut out = String::new();
    let service_snake = service.name.to_snake_case();

    for reg in &service.registrations {
        let reg_snake = reg.method.to_snake_case();
        let metadata_params: Vec<minijinja::Value> = reg
            .metadata_params
            .iter()
            .map(|mp| {
                minijinja::context! {
                    name => &mp.name,
                    rust_type => typeref_to_rust_ffi_type(&mp.ty),
                }
            })
            .collect();

        out.push_str(&crate::backends::swift::template_env::render(
            "rust_extern_c_register_via_callback.rs.jinja",
            minijinja::context! {
                service_snake => &service_snake,
                reg_snake => &reg_snake,
                service_name => &service.name,
                metadata_params => metadata_params,
            },
        ));
    }

    out
}

// ──────────────────────────────────────────────────────────── Swift output ──

/// Generate the idiomatic Swift service class (`Service.swift`).
///
/// Produces a Swift class that wraps the swift-bridge opaque type and exposes:
/// - A constructor that calls the swift-bridge `_new` function
/// - Configurator methods that chain (return self)
/// - Registration methods that accept Swift closures and wrap them via @convention(c) trampolines
/// - A `run(...)` method that calls the swift-bridge entrypoint
pub(super) fn gen_service_swift(api: &ApiSurface, service: &ServiceDef) -> String {
    let mut out = String::new();

    let class_name = &service.name;
    let service_snake = class_name.to_snake_case();
    let service_camel = service_snake.to_lower_camel_case();

    // File header with Foundation import
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_file_header.swift.jinja",
        minijinja::Value::from(()),
    ));

    // Emit @_silgen_name declarations for callback registration functions (defined outside the bridge module).
    for reg in &service.registrations {
        let reg_snake = reg.method.to_snake_case();
        let metadata_params: Vec<minijinja::Value> = reg
            .metadata_params
            .iter()
            .map(|mp| {
                minijinja::context! {
                    name => &mp.name,
                    swift_type => typeref_to_swift_type(&mp.ty),
                }
            })
            .collect();

        out.push_str(&crate::backends::swift::template_env::render(
            "swift_silgen_callback.swift.jinja",
            minijinja::context! {
                service_snake => &service_snake,
                reg_snake => &reg_snake,
                metadata_params => metadata_params,
            },
        ));
    }

    // Class header with doc comment
    let doc = if !service.doc.is_empty() {
        format_swift_comment(&service.doc, 0)
    } else {
        String::new()
    };
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_class_header.swift.jinja",
        minijinja::context! {
            class_name => class_name,
            doc => &doc,
        },
    ));

    // Constructor
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_init.swift.jinja",
        minijinja::context! {
            service_snake => &service_snake,
            service_camel => &service_camel,
        },
    ));

    // Destructor
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_deinit.swift.jinja",
        minijinja::Value::from(()),
    ));

    // Configurator methods (chaining)
    for config in &service.configurators {
        let config_name = &config.name;
        let config_camel = config_name.to_lower_camel_case();
        let doc = if !config.doc.is_empty() {
            format_swift_comment(&config.doc, 4)
        } else {
            String::new()
        };

        out.push_str(&crate::backends::swift::template_env::render(
            "swift_configurator.swift.jinja",
            minijinja::context! {
                service_snake => &service_snake,
                config_name => config_name,
                config_camel => &config_camel,
                doc => &doc,
            },
        ));
    }

    // Registration methods
    for reg in &service.registrations {
        gen_registration_method(&mut out, service, reg, api, &service_snake);
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        // Skip finalize entrypoints whose return type can't be represented over the C ABI.
        if matches!(ep.kind, crate::core::ir::EntrypointKind::Finalize) && !entrypoint_return_representable(ep, service, api) {
            continue;
        }
        gen_entrypoint_method(&mut out, service, ep, &service_snake);
    }

    // Class footer
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_class_footer.swift.jinja",
        minijinja::Value::from(()),
    ));

    out
}

fn gen_registration_method(
    out: &mut String,
    _service: &ServiceDef,
    reg: &RegistrationDef,
    _api: &ApiSurface,
    service_snake: &str,
) {
    let method_name = &reg.method;
    let method_camel = method_name.to_lower_camel_case();

    // Build metadata param signature (excluding the callback param)
    let meta_params: Vec<String> = reg
        .metadata_params
        .iter()
        .map(|p| {
            let swift_type = typeref_to_swift_type(&p.ty);
            format!("{}: {}", p.name, swift_type)
        })
        .collect();

    let meta_sig = meta_params.join(", ");

    let doc = if !reg.doc.is_empty() {
        format_swift_comment(&reg.doc, 4)
    } else {
        String::new()
    };

    let metadata_params: Vec<minijinja::Value> = reg
        .metadata_params
        .iter()
        .map(|mp| {
            minijinja::context! {
                name => &mp.name,
            }
        })
        .collect();

    out.push_str(&crate::backends::swift::template_env::render(
        "swift_registration.swift.jinja",
        minijinja::context! {
            doc => &doc,
            method_camel => &method_camel,
            meta_params => &meta_sig,
            service_snake => service_snake,
            method_name => method_name,
            metadata_params => metadata_params,
        },
    ));
}

fn gen_entrypoint_method(
    out: &mut String,
    _service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    service_snake: &str,
) {
    let ep_method = &ep.method;
    let ep_camel = ep_method.to_lower_camel_case();

    let doc = if !ep.doc.is_empty() {
        format_swift_comment(&ep.doc, 4)
    } else {
        String::new()
    };

    // Build parameter signature
    let params: Vec<String> = ep
        .params
        .iter()
        .map(|p| {
            let swift_type = typeref_to_swift_type(&p.ty);
            format!("{}: {}", p.name, swift_type)
        })
        .collect();

    let param_sig = if params.is_empty() {
        String::new()
    } else {
        params.join(", ")
    };

    // Return type
    let return_type = if ep.return_type == TypeRef::Unit {
        "Void".to_owned()
    } else {
        typeref_to_swift_type(&ep.return_type)
    };

    let throws_kw = if ep.error_type.is_some() { " throws" } else { "" };

    let ep_params: Vec<minijinja::Value> = ep
        .params
        .iter()
        .map(|p| {
            minijinja::context! {
                name => &p.name,
            }
        })
        .collect();

    out.push_str(&crate::backends::swift::template_env::render(
        "swift_entrypoint.swift.jinja",
        minijinja::context! {
            doc => &doc,
            ep_camel => &ep_camel,
            param_sig => &param_sig,
            throws_kw => throws_kw,
            return_type => &return_type,
            service_snake => service_snake,
            ep_method => ep_method,
            params => ep_params,
            has_error => ep.error_type.is_some(),
            has_return_value => ep.return_type != TypeRef::Unit,
        },
    ));
}

// ─────────────────────────────────────────────────── public entry points ──

/// Generate all service-API files for the Swift backend.
///
/// Returns one `GeneratedFile` per service when services are present:
/// - `{output_dir}/Service.swift` — Swift service class
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    let mut files = Vec::new();

    for service in &api.services {
        if service.registrations.is_empty() {
            continue;
        }

        let module_name = config.swift_module();
        let base_dir =
            crate::core::config::resolve_output_dir(config.output_paths.get("swift"), &config.name, "packages/swift");
        let base_path = PathBuf::from(&base_dir);

        let path = if config.explicit_output.swift.is_some() {
            base_path.join(format!("{}.swift", service.name))
        } else {
            base_path
                .join("Sources")
                .join(&module_name)
                .join(format!("{}.swift", service.name))
        };

        let content = gen_service_swift(api, service);

        files.push(GeneratedFile {
            path,
            content,
            generated_header: true,
        });
    }

    Ok(files)
}

/// Generate Rust extern "Rust" blocks for service-API declarations.
/// These are inserted into the swift-bridge bridge module in the rust crate.
pub fn generate_rust_extern_blocks(api: &ApiSurface) -> anyhow::Result<Vec<String>> {
    let mut blocks = Vec::new();

    for service in &api.services {
        if service.registrations.is_empty() {
            continue;
        }
        blocks.push(gen_service_rust_extern_blocks(service, api));
    }

    Ok(blocks)
}

/// Generate plain C functions for callback registration (OUTSIDE the bridge module).
/// These are emitted after the `#[swift_bridge::bridge] mod ffi { ... }` block closes in lib.rs.
pub fn generate_rust_callback_c_functions(api: &ApiSurface) -> anyhow::Result<Vec<String>> {
    let mut funcs = Vec::new();

    for service in &api.services {
        if service.registrations.is_empty() {
            continue;
        }
        funcs.push(gen_rust_callback_c_functions_for_service(service));
    }

    Ok(funcs)
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
            error_type: None,
            doc: "Register a request handler.".to_owned(),
        };

        let run_entrypoint = EntrypointDef {
            method: "run".to_owned(),
            kind: EntrypointKind::Run,
            is_async: false,
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
    fn test_gen_service_swift_contains_class() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        assert!(
            output.contains("public final class TestService"),
            "expected `public final class TestService` in output:\n{output}"
        );
    }

    #[test]
    fn test_gen_service_swift_contains_init_and_deinit() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        assert!(
            output.contains("public init()"),
            "expected `public init()` in output:\n{output}"
        );
        assert!(output.contains("deinit"), "expected `deinit` in output:\n{output}");
        assert!(
            output.contains("handlerBoxes.removeAll()"),
            "expected handler box cleanup in deinit:\n{output}"
        );
    }

    #[test]
    fn test_gen_service_swift_boxes_handler() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        assert!(
            output.contains("private final class HandlerBox"),
            "expected HandlerBox reference type:\n{output}"
        );
        assert!(
            output.contains("private var handlerBoxes: [UnsafeMutableRawPointer]"),
            "expected retained-box tracking array:\n{output}"
        );
        assert!(
            output.contains("Unmanaged.passRetained(handlerBox).toOpaque()"),
            "expected the handler box to be retained as the context pointer:\n{output}"
        );
        assert!(
            output.contains("Unmanaged<HandlerBox>.fromOpaque(contextPtr).release()"),
            "expected boxes to be released in deinit:\n{output}"
        );
    }

    #[test]
    fn test_gen_service_swift_contains_registration_method() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        assert!(
            output.contains("public func addHandler"),
            "expected registration method `addHandler`:\n{output}"
        );
        assert!(
            output.contains("@convention(c)"),
            "expected C-compatible closure:\n{output}"
        );
        assert!(
            output.contains("trampolineFunc"),
            "expected C trampoline function:\n{output}"
        );
    }

    #[test]
    fn test_gen_service_swift_contains_context_recovery() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        assert!(
            output.contains("Unmanaged<HandlerBox>.fromOpaque(contextPtr).takeUnretainedValue()"),
            "expected the boxed handler to be recovered from the context pointer:\n{output}"
        );
        assert!(
            output.contains("handlerBox.handler(requestJSON)"),
            "expected the recovered handler to be invoked with the request:\n{output}"
        );
    }

    #[test]
    fn test_gen_service_swift_contains_run_method() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        assert!(
            output.contains("public func run"),
            "expected `run` entrypoint method:\n{output}"
        );
        assert!(
            output.contains("inner.run("),
            "expected instance method call to inner.run():\n{output}"
        );
    }

    #[test]
    fn test_gen_rust_extern_blocks_contains_type_decl() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_rust_extern_blocks(service, &api);

        assert!(
            output.contains("type TestService;"),
            "expected opaque type declaration:\n{output}"
        );
        assert!(
            output.contains("extern \"Rust\""),
            "expected extern \"Rust\" block:\n{output}"
        );
    }

    #[test]
    fn test_gen_rust_extern_blocks_excludes_callback_registration() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_rust_extern_blocks(service, &api);

        // Callback registration should NOT be in the bridge module
        assert!(
            !output.contains("extern \"C\" fn(*mut std::ffi::c_void, *const u8, usize) -> *mut u8"),
            "expected raw pointer callback signature to be EXCLUDED from bridge module:\n{output}"
        );
        assert!(
            !output.contains("_via_callback"),
            "expected callback-shim registration method to be EXCLUDED from bridge module:\n{output}"
        );
    }

    #[test]
    fn test_generate_rust_callback_c_functions_contains_callback_signature() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_rust_callback_c_functions_for_service(service);

        // Callback registration SHOULD be in the C function output
        assert!(
            output.contains("extern \"C\" fn"),
            "expected extern \"C\" fn in callback C function:\n{output}"
        );
        assert!(
            output.contains("_via_callback"),
            "expected callback-shim function name:\n{output}"
        );
        assert!(
            output.contains("*mut std::ffi::c_void"),
            "expected raw c_void pointer in callback:\n{output}"
        );
        assert!(
            output.contains("#[unsafe(no_mangle)]") || output.contains("#[no_mangle]"),
            "expected #[unsafe(no_mangle)] or #[no_mangle] on extern \"C\" function:\n{output}"
        );
    }

    #[test]
    fn test_gen_rust_extern_blocks_contains_result_return() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_rust_extern_blocks(service, &api);

        // Fallible entrypoints return a JSON envelope string (swift-bridge 0.1.59
        // cannot parse Result<T, E> in extern blocks).
        assert!(
            output.contains("-> String") || output.contains("-> Result<(), String>"),
            "expected entrypoint return type (JSON envelope or unit):\n{output}"
        );
    }

    #[test]
    fn test_generate_returns_file_for_non_empty_services() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let files = generate(&api, &config).expect("generate should not fail");
        assert!(!files.is_empty(), "expected at least one generated file");

        let has_service_file = files.iter().any(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.ends_with("TestService.swift"))
                .unwrap_or(false)
        });
        assert!(has_service_file, "expected TestService.swift in output");
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
    fn test_generate_skips_services_without_registrations() {
        let mut api = make_fixture_surface();
        api.services[0].registrations.clear();

        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let files = generate(&api, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for service without registrations");
    }

    #[test]
    fn test_swift_wrapper_no_dlsym_or_dlopen() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        assert!(
            !output.contains("dlsym"),
            "expected no dlsym (swift-bridge-based, not raw C lookup):\n{output}"
        );
        assert!(
            !output.contains("dlopen"),
            "expected no dlopen (swift-bridge-based, not raw C lookup):\n{output}"
        );
    }

    #[test]
    fn test_rust_extern_blocks_no_raw_symbol_hardcode() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_rust_extern_blocks(service, &api);

        // No hardcoded HTTP/framework names — everything from IR
        assert!(
            !output.contains("\"http\""),
            "expected no hardcoded HTTP references:\n{output}"
        );
        assert!(
            !output.contains("\"handler\""),
            "expected no hardcoded handler-trait names:\n{output}"
        );
    }

    #[test]
    fn test_registration_no_empty_leading_comma() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        // Should not have double comma like "(_ handler: ..., , builder: ...)"
        assert!(
            !output.contains(", , "),
            "expected no double comma in registration signature:\n{output}"
        );
    }

    #[test]
    fn test_switch_case_on_own_lines() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        // switch/case should not collapse onto the same line as preceding code
        assert!(
            !output.contains(")        switch"),
            "expected switch on its own line, not collapsed:\n{output}"
        );
        assert!(
            !output.contains("case .success:            break        case .failure"),
            "expected each case on its own line:\n{output}"
        );
    }

    #[test]
    fn test_swift_uses_silgen_not_bridge_method() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        // Should use @_silgen_name'd C function, NOT inner.addHandlerViaCallback()
        assert!(
            !output.contains("inner.addHandlerViaCallback("),
            "expected callback to use @_silgen_name C function, NOT swift-bridge method:\n{output}"
        );
        assert!(
            output.contains("_test_service_add_handler_via_callback("),
            "expected call to @_silgen_name'd C function:\n{output}"
        );
    }

    #[test]
    fn test_swift_contains_silgen_declaration() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        // Should have @_silgen_name declaration at module scope
        assert!(
            output.contains("@_silgen_name(\"test_service_add_handler_via_callback\")"),
            "expected @_silgen_name declaration for callback C function:\n{output}"
        );
        assert!(
            output.contains("private func _test_service_add_handler_via_callback("),
            "expected private func declaration for silgen'd C function:\n{output}"
        );
    }

    #[test]
    fn test_named_metadata_types_preserved() {
        let mut api = make_fixture_surface();
        // Change the metadata param type from String to a Named type
        api.services[0].registrations[0].metadata_params[0].ty = TypeRef::Named("RouteBuilder".to_owned());

        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        // Should use RouteBuilder as the type, not String
        assert!(
            output.contains("path: RouteBuilder"),
            "expected Named metadata param typed as RouteBuilder, not String:\n{output}"
        );
    }

    #[test]
    fn test_skip_non_representable_finalize() {
        let mut api = make_fixture_surface();
        // Add a finalize entrypoint with a non-representable return type (Vec<String>)
        api.services[0].entrypoints.push(crate::core::ir::EntrypointDef {
            method: "into_router".to_owned(),
            kind: crate::core::ir::EntrypointKind::Finalize,
            is_async: false,
            params: vec![],
            return_type: TypeRef::Vec(Box::new(TypeRef::String)),
            error_type: None,
            doc: "Build the router.".to_owned(),
        });

        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        // Should not contain intoRouter method
        assert!(
            !output.contains("func intoRouter"),
            "expected finalize with non-representable return to be skipped:\n{output}"
        );
    }

    #[test]
    fn test_rust_extern_has_swift_bridge_names() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_rust_extern_blocks(service, &api);

        // Should have #[swift_bridge(swift_name = ...)] attributes for clarity
        assert!(
            output.contains("#[swift_bridge(swift_name ="),
            "expected swift_bridge swift_name attribute:\n{output}"
        );
    }
}
