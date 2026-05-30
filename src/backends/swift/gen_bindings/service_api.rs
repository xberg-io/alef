//! Service-API codegen for the Swift backend.
//!
//! Generates one output per [`ServiceDef`] with non-empty registrations:
//!
//! **`Service.swift`** — An idiomatic Swift service class that wraps the C FFI contract,
//! providing typed registration methods and a run method that delegates to the C symbols.
//!
//! The generated Swift service:
//! - Wraps an opaque C handle (returned by C `_new` / freed by `_free`)
//! - Exposes registration methods that accept Swift closures
//! - Wraps each Swift closure as a C callback via a trampoline + context recovery
//! - Calls the C `_register_<method>` symbols with the C-compatible function pointer
//! - Calls the C `_run` / `_finalize` entrypoint symbols via `_ep_<name>`
//!
//! The C FFI contract is emitted by the `ffi` backend in `service.rs` and declares:
//! - `extern "C" fn {prefix}_{service}_new() -> *mut {Service}Opaque`
//! - `extern "C" fn {prefix}_{service}_free(*mut {Service}Opaque)`
//! - `extern "C" fn {prefix}_{service}_register_<method>(owner, callback, ctx, metadata...)`
//! - `extern "C" fn {prefix}_{service}_ep_<entrypoint>(owner, params...)`
//! - Callback typedef: `fn(*mut c_void, *const c_char) -> *mut c_char`

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
fn entrypoint_return_representable(ep: &crate::core::ir::EntrypointDef, api: &ApiSurface) -> bool {
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
        _ => "String".to_owned(), // Json, Vec, Map, etc. go through JSON serialization
    }
}

// ──────────────────────────────────────────────────────── Swift output ──

/// Generate the idiomatic Swift service class (`Service.swift`).
///
/// Produces a Swift class that wraps the C FFI contract and exposes:
/// - A constructor that calls the C `_new` symbol and retains the opaque handle.
/// - A deinit that frees the opaque handle via the C `_free` symbol.
/// - Registration methods that accept Swift closures and wrap them as C callbacks.
/// - A `run(...)` method that calls the C `_run` entrypoint.
pub(super) fn gen_service_swift(api: &ApiSurface, service: &ServiceDef) -> String {
    let mut out = String::new();

    let class_name = &service.name;
    let service_snake = class_name.to_snake_case();

    // Class definition with documentation
    if !service.doc.is_empty() {
        out.push_str(&format_swift_comment(&service.doc, 0));
    }
    out.push_str(&format!("public final class {class_name} {{\n\n"));

    // Opaque handle field
    out.push_str("    private var opaqueHandle: OpaquePointer?\n\n");

    // Retained handler boxes; the trampoline context is a pointer to one of these.
    out.push_str("    /// Retained handler boxes. Each box is passed to the C layer as the\n");
    out.push_str("    /// trampoline context pointer and released in `deinit` to avoid leaks.\n");
    out.push_str("    private var handlerBoxes: [UnsafeMutableRawPointer] = []\n\n");

    // Reference-type box so a closure can cross the C FFI boundary via an opaque pointer.
    out.push_str("    /// Boxes a handler closure so it can travel through a C context pointer.\n");
    out.push_str("    private final class HandlerBox {\n");
    out.push_str("        let handler: (String) -> String\n");
    out.push_str("        init(_ handler: @escaping (String) -> String) { self.handler = handler }\n");
    out.push_str("    }\n\n");

    // Constructor
    out.push_str("    /// Create a new service instance.\n");
    out.push_str("    public init() {\n");
    out.push_str(&format!(
        "        self.opaqueHandle = RustBridge.{service_snake}New()\n"
    ));
    out.push_str("    }\n\n");

    // Destructor
    out.push_str("    /// Free the service instance.\n");
    out.push_str("    deinit {\n");
    out.push_str("        if let handle = opaqueHandle {\n");
    out.push_str(&format!("            RustBridge.{service_snake}Free(handle)\n"));
    out.push_str("            opaqueHandle = nil\n");
    out.push_str("        }\n");
    out.push_str("        // Release every retained handler box.\n");
    out.push_str("        for boxPtr in handlerBoxes {\n");
    out.push_str("            Unmanaged<HandlerBox>.fromOpaque(boxPtr).release()\n");
    out.push_str("        }\n");
    out.push_str("        handlerBoxes.removeAll()\n");
    out.push_str("    }\n\n");

    // Registration methods
    for reg in &service.registrations {
        gen_registration_method(&mut out, service, reg, api, &service_snake);
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        // Skip finalize entrypoints whose return type can't be represented over the C ABI.
        if matches!(ep.kind, crate::core::ir::EntrypointKind::Finalize) && !entrypoint_return_representable(ep, api) {
            continue;
        }
        gen_entrypoint_method(&mut out, service, ep, &service_snake);
    }

    out.push_str("}\n");
    out
}

fn gen_registration_method(
    out: &mut String,
    _service: &ServiceDef,
    reg: &RegistrationDef,
    api: &ApiSurface,
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

    let meta_sig = if meta_params.is_empty() {
        String::new()
    } else {
        format!(", {}", meta_params.join(", "))
    };

    if !reg.doc.is_empty() {
        out.push_str(&format_swift_comment(&reg.doc, 4));
    }

    // Handler closure parameter: (String) -> String
    out.push_str(&format!(
        "    public func {method_camel}(_ handler: @escaping (String) -> String{meta_sig}) {{\n"
    ));

    // Box the handler and retain it; the box pointer is the trampoline context.
    out.push_str("        // Box the handler and retain it; the box pointer is passed to the\n");
    out.push_str("        // C layer as the trampoline context and released in deinit.\n");
    out.push_str("        let handlerBox = HandlerBox(handler)\n");
    out.push_str("        let contextPtr = Unmanaged.passRetained(handlerBox).toOpaque()\n");
    out.push_str("        handlerBoxes.append(contextPtr)\n\n");

    // Emit C-compatible trampoline that recovers the boxed handler and invokes it.
    out.push_str("        // Create a C-compatible callback wrapper\n");
    out.push_str("        let trampolineFunc: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? = { contextPtr, requestPtr in\n");
    out.push_str("            guard let contextPtr = contextPtr else { return nil }\n");
    out.push_str("            guard let requestPtr = requestPtr else { return nil }\n\n");

    // Recover the boxed handler from the context pointer.
    out.push_str("            // Recover the boxed handler closure from the context pointer\n");
    out.push_str("            let handlerBox = Unmanaged<HandlerBox>.fromOpaque(contextPtr).takeUnretainedValue()\n");
    out.push_str("            let requestJSON = String(cString: requestPtr)\n");
    out.push_str("            let responseJSON = handlerBox.handler(requestJSON)\n\n");

    // Allocate and return response (caller frees).
    out.push_str("            // Allocate response string on C heap (caller must free)\n");
    out.push_str("            let responseBytes = responseJSON.utf8CString\n");
    out.push_str("            let responsePtr = UnsafeMutablePointer<CChar>.allocate(capacity: responseBytes.count)\n");
    out.push_str("            responsePtr.initialize(from: responseBytes, count: responseBytes.count)\n");
    out.push_str("            return responsePtr\n");
    out.push_str("        }\n\n");

    // Call C registration function with metadata
    out.push_str("        guard let handle = opaqueHandle else { return }\n\n");
    let method_camel_upper = format!(
        "{}{}",
        method_camel.chars().next().unwrap().to_uppercase(),
        &method_camel[1..]
    );
    out.push_str(&format!(
        "        RustBridge.{service_snake}Register{method_camel_upper}(\n            handle,\n            trampolineFunc,\n            contextPtr"
    ));

    // Add metadata parameters, marshaling opaque handles appropriately.
    // For Named types that are in the API surface, pass the underlying opaque handle.
    // For everything else (primitives, strings), pass the value as-is.
    for meta_param in &reg.metadata_params {
        match &meta_param.ty {
            TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n) => {
                // Opaque handle: pass the underlying pointer via Unsafe methods.
                // The parameter is a RustBridge.{Name}Ref or RustBridge.{Name}.
                // We extract the opaque pointer for FFI passing.
                out.push_str(&format!(",\n            {}.unsafelyUnwrapped", meta_param.name));
            }
            _ => {
                // Primitive or string: pass as-is.
                out.push_str(&format!(",\n            {}", meta_param.name));
            }
        }
    }

    out.push_str("\n        )\n");
    out.push_str("    }\n\n");
}

fn gen_entrypoint_method(
    out: &mut String,
    _service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    service_snake: &str,
) {
    let ep_method = &ep.method;
    let ep_camel = ep_method.to_lower_camel_case();

    if !ep.doc.is_empty() {
        out.push_str(&format_swift_comment(&ep.doc, 4));
    }

    // Build parameter signature
    let params: Vec<String> = ep
        .params
        .iter()
        .map(|p| {
            let swift_type = typeref_to_swift_type(&p.ty);
            format!("{}: {}", p.name, swift_type)
        })
        .collect();

    let param_sig = params.join(", ");

    // Determine if async
    let async_kw = if ep.is_async { " async" } else { "" };
    let throws_kw = if ep.error_type.is_some() { " throws" } else { "" };

    // Return type
    let return_type = if ep.return_type == TypeRef::Unit {
        "Void".to_owned()
    } else {
        typeref_to_swift_type(&ep.return_type)
    };

    out.push_str(&format!(
        "    public func {ep_camel}({param_sig}){async_kw}{throws_kw} -> {return_type} {{\n"
    ));

    // Call C entrypoint function
    out.push_str("        guard let handle = opaqueHandle else { throw ServiceError.invalidHandle }\n\n");

    let ep_camel_upper = format!("{}{}", ep_camel.chars().next().unwrap().to_uppercase(), &ep_camel[1..]);

    if ep.is_async {
        out.push_str(&format!(
            "        return try await withUnsafeThrowingContinuation {{ continuation in\n\
             \x20\x20\x20\x20Task {{\n\
             \x20\x20\x20\x20\x20\x20RustBridge.{service_snake}Ep{ep_camel_upper}(\n\
             \x20\x20\x20\x20\x20\x20\x20\x20handle"
        ));
    } else {
        out.push_str(&format!(
            "        RustBridge.{service_snake}Ep{ep_camel_upper}(\n            handle"
        ));
    }

    // Add entrypoint parameters
    for ep_param in &ep.params {
        out.push_str(&format!(",\n            {}", ep_param.name));
    }

    out.push_str("\n        ");

    if ep.is_async {
        out.push_str(")\n        }\n        }\n");
    } else {
        out.push_str(")\n");
    }

    out.push_str("    }\n\n");
}

// ──────────────────────────────────────────────────────── public entry point ──

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
            output.contains("RustBridge.test_serviceFree"),
            "expected C free call in deinit:\n{output}"
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
            output.contains("Unmanaged<HandlerBox>.fromOpaque(boxPtr).release()"),
            "expected boxes to be released in deinit:\n{output}"
        );
        // The broken dual-meaning context model must be gone.
        assert!(
            !output.contains("handlerRegistry"),
            "stale handlerRegistry field should be removed:\n{output}"
        );
        assert!(
            !output.contains("Int(bitPattern: contextPtr)"),
            "broken bitPattern-as-index lookup should be removed:\n{output}"
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
            output.contains("async"),
            "expected async keyword for async entrypoint:\n{output}"
        );
        assert!(
            output.contains("RustBridge.test_serviceEpRun"),
            "expected C run symbol call:\n{output}"
        );
    }

    #[test]
    fn test_gen_service_swift_contains_c_ffi_symbols() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let output = gen_service_swift(&api, service);

        // C symbols from FFI contract
        assert!(
            output.contains("RustBridge.test_serviceNew"),
            "expected C new symbol:\n{output}"
        );
        assert!(
            output.contains("RustBridge.test_serviceFree"),
            "expected C free symbol:\n{output}"
        );
        assert!(
            output.contains("RustBridge.test_serviceRegisterAddHandler"),
            "expected C register symbol:\n{output}"
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
}
