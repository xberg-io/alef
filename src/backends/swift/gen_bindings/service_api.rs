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
//! 2. **Swift wrapper class** at `Sources/<ModuleName>/<ServiceName>.swift`:
//!    - Public class wrapping the swift-bridge opaque type
//!    - Idiomatic Swift methods for constructor, configurators, registration (with closure boxing), and entrypoints
//!    - Handler boxes (reference type) to cross closures via C context pointers
//!    - @convention(c) trampolines for C callback interop

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase};
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts
        .iter()
        .find(|contract| contract.trait_name == trait_name)
}

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
                PrimitiveType::Usize => "UInt".to_owned(),
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

/// Collect unique wrapper-constructor extern declarations from all registration variants.
///
/// For each `WrapperConstructorCall` on a `RegistrationVariant` (e.g. `RouteBuilder::new(method, path)`),
/// this emits an extern "Rust" free function declaration like:
///   `fn route_builder_new(method: &Method, path: String) -> RouteBuilder`
///
/// This is needed because swift-bridge represents enums as opaque classes (not mirrored Swift enums),
/// so `Method.Get` is invalid Swift syntax. Instead, the variant shorthand methods call
/// `routeBuilderNew(try methodFromJson("\"Get\""), path)` and delegate to the base registration.
///
/// Returns a deduplicated list of minijinja context values for the template.
fn collect_wrapper_constructor_externs(service: &ServiceDef) -> Vec<minijinja::Value> {
    use crate::core::ir::WrapperConstructorArg;
    use std::collections::HashSet;

    let mut seen: HashSet<String> = HashSet::new();
    let mut result: Vec<minijinja::Value> = Vec::new();

    for reg in &service.registrations {
        for variant in &reg.variants {
            let Some(wc) = &variant.wrapper_call else { continue };
            // Deduplicate by function name (wrapper_type_snake_new).
            let fn_snake = format!("{}_new", wc.wrapper_type_name.to_snake_case());
            if !seen.insert(fn_snake.clone()) {
                continue;
            }
            let fn_camel = fn_snake.to_lower_camel_case();

            // Build argument list for the extern declaration.
            // Fixed args with enum-typed value_expr become `&EnumType` params (opaque ref).
            // Free args become their declared type.
            let args: Vec<minijinja::Value> = wc
                .args
                .iter()
                .map(|arg| match arg {
                    WrapperConstructorArg::Fixed { param_name, value_expr } => {
                        // value_expr is e.g. "source_crate::Method::Get". Extract the type name.
                        // Format is `crate::TypeName::Variant` — split at last `::` twice.
                        let rust_type = if let Some(last_colon) = value_expr.rfind("::") {
                            if let Some(second_colon) = value_expr[..last_colon].rfind("::") {
                                // "source_crate::Method::Get" → "Method"
                                value_expr[second_colon + 2..last_colon].to_string()
                            } else {
                                // "Method::Get" → take before the "::"
                                value_expr[..last_colon].to_string()
                            }
                        } else {
                            value_expr.clone()
                        };
                        // Use `&TypeName` so swift-bridge maps it to `TypeNameRef` (opaque ref param).
                        minijinja::context! {
                            name => param_name,
                            rust_type => format!("&{rust_type}"),
                        }
                    }
                    WrapperConstructorArg::Free { param } => {
                        let rust_type = typeref_to_rust_ffi_type(&param.ty);
                        minijinja::context! {
                            name => &param.name,
                            rust_type => rust_type,
                        }
                    }
                })
                .collect();

            result.push(minijinja::context! {
                fn_snake => &fn_snake,
                fn_camel => fn_camel,
                wrapper_type_name => &wc.wrapper_type_name,
                wrapper_type_path => &wc.wrapper_type_path,
                constructor_method => &wc.constructor_method,
                args => args,
            });
        }
    }

    result
}

/// Generate Rust extern "Rust" declarations for a service (INSIDE the bridge module).
/// These are appended to the `#[swift_bridge::bridge] mod ffi { ... }` block in lib.rs.
/// Registration callbacks are excluded — they go outside the bridge via `generate_rust_callback_c_functions`.
///
/// Split into TWO blocks to work around swift-bridge 0.1.59 parse error ("expected path"):
/// Block 1: Type declaration + constructor
/// Block 2: Instance methods (configurators, entrypoints) using `associated_to` attribute
fn gen_service_rust_extern_blocks(service: &ServiceDef, api: &ApiSurface) -> String {
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

    // Emit Block 1: Type declaration + constructor
    let mut out = crate::backends::swift::template_env::render(
        "rust_extern_service_type_and_constructor.rs.jinja",
        minijinja::context! {
            service_name => &service.name,
        },
    );

    // Emit Block 2: Methods with associated_to (always — block 2 also carries the
    // `<service>_raw_ptr` helper needed by the @_silgen_name registration shims).
    let service_snake = service.name.to_snake_case();
    let service_camel = service_snake.to_lower_camel_case();

    // Collect unique WrapperConstructorCall signatures from all registration variants.
    // These become `route_builder_new`-style free functions in the extern "Rust" block
    // so Swift can construct wrapper metadata params (e.g. RouteBuilder) without
    // relying on non-existent static enum member syntax (swift-bridge enums are opaque
    // classes, not mirrored Swift enums with static members).
    let wrapper_constructors = collect_wrapper_constructor_externs(service);

    out.push_str(&crate::backends::swift::template_env::render(
        "rust_extern_service_methods.rs.jinja",
        minijinja::context! {
            service_name => &service.name,
            service_snake => &service_snake,
            service_camel => &service_camel,
            configurators => configurators,
            entrypoints => entrypoints,
            wrapper_constructors => wrapper_constructors,
        },
    ));

    out
}

/// Generate plain C functions for callback registration (OUTSIDE the bridge module).
/// These are emitted after the `#[swift_bridge::bridge] mod ffi { ... }` block closes.
fn gen_rust_callback_c_functions_for_service(api: &ApiSurface, service: &ServiceDef) -> String {
    let mut out = String::new();
    let source_crate = api.crate_name.replace('-', "_");
    let service_snake = service.name.to_snake_case();

    for reg in &service.registrations {
        let reg_snake = reg.method.to_snake_case();
        let contract = find_contract(api, &reg.callback_contract);
        let trait_path = contract
            .map(|c| {
                if c.rust_path.is_empty() {
                    format!("{source_crate}::{}", c.trait_name)
                } else {
                    c.rust_path.clone()
                }
            })
            .unwrap_or_else(|| format!("{source_crate}::{}", reg.callback_contract));
        let request_path = contract
            .and_then(|c| c.wire_request_type.as_deref())
            .map(|name| qualify_rust_type(name, &source_crate))
            .unwrap_or_else(|| "serde_json::Value".to_string());
        let response_path = contract
            .and_then(|c| c.wire_response_type.as_deref())
            .map(|name| qualify_rust_type(name, &source_crate))
            .unwrap_or_else(|| "serde_json::Value".to_string());
        let output_type = contract
            .and_then(|c| c.dispatch_return_type.as_deref())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Result<{response_path}, Box<dyn std::error::Error + Send + Sync>>"));
        let response_adapter = contract
            .and_then(|c| c.response_adapter.as_deref())
            .map(|adapter| format!("{adapter}(outcome)"))
            .unwrap_or_else(|| "outcome".to_string());
        let metadata_params: Vec<minijinja::Value> = reg
            .metadata_params
            .iter()
            .map(|mp| {
                // Named metadata-param types are emitted as swift-bridge wrapper newtypes
                // (`pub struct Foo(pub crate_path::Foo)`), so calling through to the inner
                // service requires `.0` to unwrap. Primitives + String pass through directly.
                let is_opaque_wrapper = matches!(&mp.ty, TypeRef::Named(_));
                minijinja::context! {
                    name => &mp.name,
                    rust_type => typeref_to_rust_ffi_type(&mp.ty),
                    is_opaque_wrapper => is_opaque_wrapper,
                }
            })
            .collect();

        out.push_str(&crate::backends::swift::template_env::render(
            "rust_extern_c_register_via_callback.rs.jinja",
            minijinja::context! {
                service_snake => &service_snake,
                reg_snake => &reg_snake,
                method_name => &reg.method,
                service_name => &service.name,
                source_crate => &source_crate,
                trait_path => &trait_path,
                request_path => &request_path,
                response_path => &response_path,
                output_type => &output_type,
                response_adapter => &response_adapter,
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
pub(super) fn gen_service_swift(api: &ApiSurface, service: &ServiceDef, config: &ResolvedCrateConfig) -> String {
    let mut out = String::new();

    let class_name = &service.name;
    let service_snake = class_name.to_snake_case();
    let service_camel = service_snake.to_lower_camel_case();

    // File header with Foundation import.
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_file_header.swift.jinja",
        minijinja::Value::from(()),
    ));
    // The wrapper references swift-bridge generated opaque types (`RustBridge.App`,
    // `RouteBuilder`, etc.). Those types live in a sibling Swift target named
    // `RustBridge`; an explicit `import RustBridge` is required for resolution.
    out.push_str("import RustBridge\n\n");

    // FFI @_silgen_name declarations for server config management.
    // Get the FFI prefix from config for generating generic symbol names.
    let ffi_prefix = config
        .ffi
        .as_ref()
        .and_then(|f| f.prefix.as_deref())
        .unwrap_or(&config.name)
        .to_string();
    let ffi_decls = format!(
        "@_silgen_name(\"{}_server_config_from_json\")\n\
         private func _{}_server_config_from_json(_ json: UnsafePointer<CChar>) -> UnsafeMutableRawPointer?\n\n\
         @_silgen_name(\"{}_server_config_free\")\n\
         private func _{}_server_config_free(_ ptr: UnsafeMutableRawPointer?)\n\n\
         @_silgen_name(\"{}_app_config\")\n\
         private func _{}_app_config(_ app: UnsafeMutablePointer<OpaquePointer>, _ config: UnsafeMutableRawPointer?) -> UnsafeMutableRawPointer?\n\n",
        ffi_prefix, ffi_prefix, ffi_prefix, ffi_prefix, ffi_prefix, ffi_prefix
    );
    out.push_str(&ffi_decls);

    // Error type used by every generated entrypoint / registration method. Defined
    // once per service file so the wrapper class methods can `throw` it without
    // forcing a separate shared module on consumers.
    out.push_str(
        "/// Errors thrown by service wrapper methods.\n\
         public enum ServiceError: Error {\n\
         \x20\x20\x20\x20/// The service handle was already consumed or never initialised.\n\
         \x20\x20\x20\x20case invalidHandle\n\
         \x20\x20\x20\x20/// The C-side registration call returned a non-zero status code.\n\
         \x20\x20\x20\x20case registrationFailed\n\
         \x20\x20\x20\x20/// The service runtime returned the given error envelope.\n\
         \x20\x20\x20\x20case runtime(String)\n\
         }\n\n",
    );

    // Emit @_silgen_name declarations for callback registration functions (defined outside the bridge module).
    for reg in &service.registrations {
        let reg_snake = reg.method.to_snake_case();
        let metadata_params: Vec<minijinja::Value> = reg
            .metadata_params
            .iter()
            .map(|mp| {
                // The silgen-imported C symbol takes the swift-bridge generated type, which
                // lives in the `RustBridge` sibling target. Named opaque metadata params
                // must therefore be referenced as `RustBridge.<Type>` here, even though the
                // user-facing wrapper class accepts the bridge type directly.
                let swift_ty = typeref_to_swift_type(&mp.ty);
                let bridge_ty = match &mp.ty {
                    TypeRef::Named(n) => format!("RustBridge.{n}"),
                    _ => swift_ty.clone(),
                };
                minijinja::context! {
                    name => &mp.name,
                    swift_type => bridge_ty,
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
            service_name => class_name,
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

    // Special config method with host and port parameters (always emitted).
    // This method constructs a ServerConfig JSON, calls the FFI to create and apply it.
    // Get the FFI prefix from config (e.g., "mylib") for generating symbol names.
    let ffi_prefix = config
        .ffi
        .as_ref()
        .and_then(|f| f.prefix.as_deref())
        .unwrap_or(&config.name)
        .to_string();
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_config_method.swift.jinja",
        minijinja::context! {
            service_snake => &service_snake,
            ffi_prefix => &ffi_prefix,
        },
    ));

    // Registration methods
    for reg in &service.registrations {
        gen_registration_method(&mut out, service, reg, api, &service_snake);
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        // Skip finalize entrypoints whose return type can't be represented over the C ABI.
        if matches!(ep.kind, crate::core::ir::EntrypointKind::Finalize)
            && !entrypoint_return_representable(ep, service, api)
        {
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

    // Build metadata param signature (excluding the callback param). Named opaque
    // metadata params are exposed as their `RustBridge.<Type>` swift-bridge wrapper:
    // the user obtains the value via the bridge module and passes it straight through.
    let meta_params: Vec<String> = reg
        .metadata_params
        .iter()
        .map(|p| {
            let swift_type = match &p.ty {
                TypeRef::Named(n) => format!("RustBridge.{n}"),
                _ => typeref_to_swift_type(&p.ty),
            };
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

    let service_camel = service_snake.to_lower_camel_case();
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_registration.swift.jinja",
        minijinja::context! {
            doc => &doc,
            method_camel => &method_camel,
            meta_params => &meta_sig,
            service_snake => service_snake,
            service_camel => &service_camel,
            method_name => method_name,
            metadata_params => metadata_params,
        },
    ));

    // Emit variant methods
    for variant in &reg.variants {
        gen_registration_variant(out, service_snake, reg, variant);
    }
}

fn gen_registration_variant(
    out: &mut String,
    service_snake: &str,
    reg: &RegistrationDef,
    variant: &crate::core::ir::RegistrationVariant,
) {
    use crate::core::ir::WrapperConstructorArg;

    let variant_name = &variant.name;
    let service_camel = service_snake.to_lower_camel_case();

    // Build signature params with Swift types
    let signature_params: Vec<minijinja::Value> = variant
        .signature_params
        .iter()
        .map(|p| {
            let swift_type = match &p.ty {
                TypeRef::Named(n) => format!("RustBridge.{n}"),
                _ => typeref_to_swift_type(&p.ty),
            };
            minijinja::context! {
                name => &p.name,
                swift_type => swift_type,
            }
        })
        .collect();

    let doc = if let Some(doc_str) = &variant.doc {
        format_swift_comment(doc_str, 4)
    } else {
        // Default doc referencing the base registration
        let default_doc = format!("Shortcut for `{}`.", reg.method);
        format_swift_comment(&default_doc, 4)
    };

    // When the variant has a WrapperConstructorCall, emit a method that:
    //   1. Constructs the wrapper type using the bridge factory (e.g. routeBuilderNew)
    //   2. Delegates to the base Swift registration method (e.g. self.route(handler, builder:))
    //
    // This avoids the invalid `RustBridge.Method.Get` syntax — swift-bridge generates enums as
    // opaque classes, not mirrored Swift enums with static member constants. The
    // `<type>FromJson("\"Variant\"")` factory constructs an opaque instance from its serde
    // wire name, then the wrapper constructor factory combines it with free args.
    if let Some(wrapper_call) = &variant.wrapper_call {
        // Build the argument expression for the wrapper constructor factory call.
        // Fixed args: use `try <TypeFromJson>("\"Variant\"")` factory syntax.
        // Free args: use the param name directly.
        let factory_fn_camel = format!("{}_new", wrapper_call.wrapper_type_name.to_snake_case()).to_lower_camel_case();
        let factory_args: Vec<String> = wrapper_call
            .args
            .iter()
            .map(|arg| match arg {
                WrapperConstructorArg::Fixed {
                    param_name: _,
                    value_expr,
                } => {
                    // value_expr is e.g. "source_crate::Method::Get"
                    // Extract type name and variant name for the from_json factory call.
                    if let Some(last_colon) = value_expr.rfind("::") {
                        let variant_str = &value_expr[last_colon + 2..];
                        if let Some(second_colon) = value_expr[..last_colon].rfind("::") {
                            let type_name = &value_expr[second_colon + 2..last_colon];
                            // type_name "Method" → factory "methodFromJson"
                            let factory_name = format!(
                                "{}FromJson",
                                type_name
                                    .chars()
                                    .next()
                                    .map(|c| c.to_lowercase().to_string())
                                    .unwrap_or_default()
                                    + &type_name[1..]
                            );
                            format!("try {factory_name}(\"\\\"{variant_str}\\\"\")")
                        } else {
                            // Fallback: just the variant name as a string
                            format!("\"{variant_str}\"")
                        }
                    } else {
                        value_expr.clone()
                    }
                }
                WrapperConstructorArg::Free { param } => param.name.clone(),
            })
            .collect();
        let factory_args_str = factory_args.join(", ");
        // The wrapper param name in the base method signature (e.g. "builder" for RouteBuilder).
        let base_param_name = &wrapper_call.metadata_param;
        let base_method_camel = reg.method.to_lower_camel_case();

        out.push_str(&crate::backends::swift::template_env::render(
            "swift_registration_variant_delegate.swift.jinja",
            minijinja::context! {
                doc => &doc,
                variant_name => variant_name,
                signature_params => signature_params,
                factory_fn_camel => factory_fn_camel,
                factory_args_str => factory_args_str,
                wrapper_type_name => &wrapper_call.wrapper_type_name,
                base_param_name => base_param_name,
                base_method_camel => base_method_camel,
            },
        ));
    } else {
        // No wrapper call — fall through to the direct C-callback invocation pattern.
        let wrapper_call_args: Vec<String> = variant.signature_params.iter().map(|p| p.name.clone()).collect();

        out.push_str(&crate::backends::swift::template_env::render(
            "swift_registration_variant.swift.jinja",
            minijinja::context! {
                doc => &doc,
                variant_name => variant_name,
                signature_params => signature_params,
                service_snake => service_snake,
                service_camel => &service_camel,
                base_method_name => &reg.method,
                wrapper_call_args => wrapper_call_args,
            },
        ));
    }
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

        let content = gen_service_swift(api, service, config);

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
        funcs.push(gen_rust_callback_c_functions_for_service(api, service));
    }

    Ok(funcs)
}

fn qualify_rust_type(type_name: &str, source_crate: &str) -> String {
    if type_name.contains("::") {
        type_name.to_string()
    } else {
        format!("{source_crate}::{type_name}")
    }
}

// ───────────────────────── Phase-C emission stubs (new IR sections) ──────────

/// Emit Swift lifecycle-hook registration methods. Stub.
pub(super) fn emit_lifecycle_hooks(hooks: &[crate::core::ir::LifecycleHookDef]) -> String {
    if hooks.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "lifecycle hook emission not implemented for swift ({} hooks)",
        hooks.len()
    );
    for _hook in hooks {}
    String::new()
}

/// Emit Swift WebSocket route registration methods. Stub.
pub(super) fn emit_websocket_routes(routes: &[crate::core::ir::WebSocketRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "WebSocket route emission not implemented for swift ({} routes)",
        routes.len()
    );
    for _route in routes {}
    String::new()
}

/// Emit Swift SSE route registration methods. Stub.
pub(super) fn emit_sse_routes(routes: &[crate::core::ir::SseRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!("SSE route emission not implemented for swift ({} routes)", routes.len());
    for _route in routes {}
    String::new()
}

/// Emit Swift native error types. Stub.
pub(super) fn emit_error_types(types: &[crate::core::ir::ErrorTypeDef]) -> String {
    if types.is_empty() {
        return String::new();
    }
    tracing::debug!("error type emission not implemented for swift ({} types)", types.len());
    for _ty in types {}
    String::new()
}

/// Aggregate stub — forwards all four new IR sections for the Swift backend.
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
