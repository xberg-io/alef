//! Service-API codegen for the JNI backend.
//!
//! Generates Rust JNI glue for service handler registration and lifecycle management.
//!
//! For each [`ServiceDef`]:
//! - A `Jni{ContractName}Bridge` struct that wraps a global JVM reference to a Java
//!   handler object and implements `Arc<dyn {HandlerContractDef::trait_name}>`
//! - `#[no_mangle] extern "system"` JNI entry points:
//!   - `register_{snake_service}_{registration_method}`: registers a Java handler
//!   - `run_{snake_service}` / `finalize_{snake_service}`: lifecycle entrypoints
//!
//! Thread safety: thread-attaches to JVM, calls Java handler methods with request JSON,
//! parses response JSON. No panics — all errors propagate as JNI exceptions.

use minijinja::context;

use crate::backends::jni::template_env;
use crate::codegen::naming::{pascal_to_snake, to_class_name};
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef};
use crate::core::jni::{bridge_method_name, jni_package, jni_symbol, service_bridge_class_name};
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

fn internal_symbol_component(name: &str) -> String {
    pascal_to_snake(name)
}

fn internal_class_component(name: &str) -> String {
    to_class_name(name)
}

fn render_service_param_decl(name: &str, type_name: &str) -> String {
    template_env::render(
        "service_param_decl.rs.jinja",
        context! {
            name => name,
            type_name => type_name,
        },
    )
}

/// Map a `TypeRef` to a JNI FFI type.
fn typeref_to_jni_type(ty: &TypeRef, _core_import: &str) -> String {
    match ty {
        TypeRef::String => "jni::objects::JString",
        TypeRef::Char => "c_char",
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "jboolean",
                PrimitiveType::U8 => "jbyte",
                PrimitiveType::U16 => "jchar",
                PrimitiveType::U32 => "jint",
                PrimitiveType::U64 => "jlong",
                PrimitiveType::I8 => "jbyte",
                PrimitiveType::I16 => "jshort",
                PrimitiveType::I32 => "jint",
                PrimitiveType::I64 => "jlong",
                PrimitiveType::F32 => "jfloat",
                PrimitiveType::F64 => "jdouble",
                PrimitiveType::Usize => "jlong",
                PrimitiveType::Isize => "jlong",
            }
        }
        TypeRef::Bytes => "*const u8",
        TypeRef::Unit => "()",
        _ => "jni::objects::JObject",
    }
    .to_owned()
}

// ──────────────────────────────────────────────────────────────── Rust glue ──

/// Generate the Rust JNI glue module (`service.rs`).
///
/// For each service this emits:
/// - A `Jni{ContractName}Bridge` struct holding a global JNI reference to the Java handler
/// - `impl` of the handler contract trait with async dispatch that:
///   - Attaches current thread to JVM
///   - Calls the Java handler method (passing request as JSON string)
///   - Parses response JSON
/// - `#[no_mangle] extern "system"` JNI entry points for handler registration and
///   service lifecycle (run/finalize)
pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let package = jni_package(config);
    let mut out = String::new();

    out.push_str(&template_env::render("service_header.rs.jinja", context! {}));

    // Emit service opaque types and constructor/destructor.
    // The JVM class hosting the `external fun`s is the per-service bridge object
    // `{ServicePascal}ServiceBridge` — it MUST match the Kotlin `object` name so the
    // `Java_*` symbols and the Kotlin `external fun` declarations link.
    for service in &api.services {
        let service_bridge_class = service_bridge_class_name(&service.name);
        gen_service_opaque(&mut out, service, &core_import, &package, &service_bridge_class);
    }

    // Emit one handler bridge per unique handler contract referenced by any registration
    let referenced_contracts: Vec<&HandlerContractDef> = {
        let mut names: Vec<&str> = api
            .services
            .iter()
            .flat_map(|s| s.registrations.iter())
            .map(|r| r.callback_contract.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        names.iter().filter_map(|n| find_contract(api, n)).collect()
    };

    for contract in &referenced_contracts {
        gen_handler_bridge(&mut out, contract, &core_import);
    }

    // Emit handler registration and lifecycle entry points per service
    for service in &api.services {
        let service_bridge_class = service_bridge_class_name(&service.name);
        for reg in &service.registrations {
            gen_register_jni_function(
                &mut out,
                service,
                reg,
                api,
                &core_import,
                &package,
                &service_bridge_class,
            );
        }
        for ep in &service.entrypoints {
            gen_entrypoint_jni_function(&mut out, service, ep, &core_import, &package, &service_bridge_class);
        }
    }

    out
}

/// Emit the opaque service type and its constructor/destructor.
fn gen_service_opaque(
    out: &mut String,
    service: &ServiceDef,
    _core_import: &str,
    package: &str,
    service_bridge_class: &str,
) {
    let opaque_name = format!("{}Opaque", service.name);
    let service_snake = internal_symbol_component(&service.name);
    let owner_path = &service.rust_path;

    let ctor_method = bridge_method_name(&service.name, "new");
    let ctor_symbol = jni_symbol(package, service_bridge_class, &ctor_method);
    let dtor_method = bridge_method_name(&service.name, "free");
    let dtor_symbol = jni_symbol(package, service_bridge_class, &dtor_method);
    out.push_str(&template_env::render(
        "service_opaque.rs.jinja",
        context! {
            service_name => service.name,
            service_snake => service_snake,
            opaque_name => opaque_name,
            owner_path => owner_path,
            constructor_name => service.constructor.name,
            ctor_symbol => ctor_symbol,
            dtor_symbol => dtor_symbol,
        },
    ));
}

/// Emit the `Jni{ContractName}Bridge` struct + trait impl.
///
/// Holds a global JVM reference to a Java handler object. When dispatched:
/// 1. Attaches current thread to JVM (idempotent if already attached)
/// 2. Calls Java handler method via JNI, passing request as JSON string
/// 3. Parses response JSON
/// 4. Detaches if this thread wasn't previously attached
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Jni{}Bridge", internal_class_component(trait_name));
    let dispatch_name = &contract.dispatch.name;

    // Determine wire types
    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

    out.push_str(&template_env::render(
        "handler_bridge_struct.rs.jinja",
        context! {
            trait_name => trait_name,
            bridge_name => bridge_name,
        },
    ));

    // Leading dispatch parameters the bridge ignores (e.g. a foreign framework type the
    // contract's dispatch method receives but the wire bridge does not consume). Their concrete
    // types cannot be reconstructed from the sanitized surface, so the library supplies them
    // verbatim via `dispatch_extra_params`. Each is emitted as a `, {decl}` prefix argument.
    let extra_param: String = contract
        .dispatch_extra_params
        .iter()
        .map(|p| format!(", {p}"))
        .collect();
    let wire_name = contract.wire_param_name.as_deref().unwrap_or("request");

    // Build module paths for types. If the wire type includes the core import prefix, strip it
    // and add it back; otherwise use plain serde_json::Value if name is "Value".
    let req_path = if req_type == "Value" {
        "serde_json::Value".to_string()
    } else if req_type.contains("::") {
        req_type.split("::").last().unwrap_or(req_type).to_string()
    } else {
        format!("{core_import}::{req_type}")
    };
    let resp_path = if resp_type == "Value" {
        "serde_json::Value".to_string()
    } else if resp_type.contains("::") {
        resp_type.split("::").last().unwrap_or(resp_type).to_string()
    } else {
        format!("{core_import}::{resp_type}")
    };

    // The future's `Output` is the contract dispatch's real return type when the library
    // supplies one (`dispatch_return_type`); otherwise the bridge yields the wire response
    // wrapped in a boxed-error `Result`. When a `response_adapter` is configured, the inner
    // fallible computation produces the wire `Result` and the adapter converts it into the
    // dispatch return type — keeping the generator ignorant of the library's response model.
    let box_err = "Box<dyn std::error::Error + Send + Sync>";
    let wire_output = format!("Result<{resp_path}, {box_err}>");
    let output_type = contract
        .dispatch_return_type
        .clone()
        .unwrap_or_else(|| wire_output.clone());
    let tail = match &contract.response_adapter {
        Some(adapter) => format!("{adapter}(outcome)"),
        None => "outcome".to_string(),
    };

    // Trait impl. Returns a boxed future directly (canonical object-safe
    // async-trait shape) instead of via the async_trait macro, matching a
    // contract whose dispatch method is hand-written as
    // `-> Pin<Box<dyn Future<..> + Send + '_>>`.
    out.push_str(&template_env::render(
        "handler_bridge_impl.rs.jinja",
        context! {
            core_import => core_import,
            trait_name => trait_name,
            bridge_name => bridge_name,
            dispatch_name => dispatch_name,
            extra_param => extra_param,
            wire_name => wire_name,
            req_path => req_path,
            output_type => output_type,
            wire_output => wire_output,
            resp_path => resp_path,
            tail => tail,
        },
    ));
}

/// Emit a JNI function that registers a Java handler for a registration method.
///
/// Function signature (in Java):
/// ```java,ignore
/// public native void register{ServiceName}{MethodName}(Object handler);
/// ```
///
/// Convention: The Java handler object must have a public method `handle(String) -> String`
fn gen_register_jni_function(
    out: &mut String,
    service: &ServiceDef,
    reg: &RegistrationDef,
    api: &ApiSurface,
    core_import: &str,
    package: &str,
    service_bridge_class: &str,
) {
    let service_pascal = internal_class_component(&service.name);
    let method_pascal = internal_class_component(&reg.method);
    let contract_name = &reg.callback_contract;

    if let Some(contract) = find_contract(api, contract_name) {
        let bridge_name = format!("Jni{}Bridge", internal_class_component(contract_name));
        let opaque_name = format!("{}Opaque", service.name);
        let register_method = bridge_method_name(&service.name, &format!("register_{}", reg.method));
        let symbol = jni_symbol(package, service_bridge_class, &register_method);

        let mut metadata_params_decl = String::new();
        for meta_param in &reg.metadata_params {
            let rust_type = typeref_to_jni_type(&meta_param.ty, core_import);
            metadata_params_decl.push_str(&render_service_param_decl(&meta_param.name, &rust_type));
        }
        let dispatch_method_name = &contract.dispatch.name;
        let mut register_args: Vec<String> = reg.metadata_params.iter().map(|p| p.name.clone()).collect();
        register_args.push("handler_arc".to_string());
        out.push_str(&template_env::render(
            "registration_function.rs.jinja",
            context! {
                service_pascal => service_pascal,
                method_pascal => method_pascal,
                symbol => symbol,
                metadata_params_decl => metadata_params_decl,
                dispatch_method_name => dispatch_method_name,
                bridge_name => bridge_name,
                core_import => core_import,
                contract_name => contract_name,
                opaque_name => opaque_name,
                register_method => reg.method,
                register_args => register_args.join(", "),
                setup_block => "",
            },
        ));

        // Emit registration variants
        for variant in &reg.variants {
            gen_register_variant_jni_function(
                out,
                service,
                reg,
                variant,
                api,
                core_import,
                package,
                service_bridge_class,
            );
        }
    }
}

/// Emit a JNI function for a registration variant (shortcut with pinned metadata).
///
/// Builds the wrapper type if present and forwards to the base register method.
#[allow(clippy::too_many_arguments)]
fn gen_register_variant_jni_function(
    out: &mut String,
    service: &ServiceDef,
    reg: &RegistrationDef,
    variant: &crate::core::ir::RegistrationVariant,
    api: &ApiSurface,
    core_import: &str,
    package: &str,
    service_bridge_class: &str,
) {
    let service_pascal = internal_class_component(&service.name);
    let variant_name = &variant.name;
    let contract_name = &reg.callback_contract;

    if let Some(contract) = find_contract(api, contract_name) {
        let bridge_name = format!("Jni{}Bridge", internal_class_component(contract_name));
        let opaque_name = format!("{}Opaque", service.name);
        let register_method = bridge_method_name(&service.name, &format!("register_{}_{}", reg.method, variant_name));
        let symbol = jni_symbol(package, service_bridge_class, &register_method);
        let dispatch_method_name = &contract.dispatch.name;

        let mut free_params_decl = String::new();
        for param in &variant.signature_params {
            let rust_type = typeref_to_jni_type(&param.ty, core_import);
            free_params_decl.push_str(&render_service_param_decl(&param.name, &rust_type));
        }

        // Build wrapper if wrapper_call is present
        let mut wrapper_block = String::new();
        if let Some(wc) = &variant.wrapper_call {
            let mut constructor_args = Vec::new();
            for arg in &wc.args {
                match arg {
                    crate::core::ir::WrapperConstructorArg::Fixed {
                        param_name: _,
                        value_expr,
                    } => {
                        constructor_args.push(value_expr.clone());
                    }
                    crate::core::ir::WrapperConstructorArg::Free { param } => {
                        constructor_args.push(param.name.clone());
                    }
                }
            }
            wrapper_block.push_str(&template_env::render(
                "wrapper_setup.rs.jinja",
                context! {
                    name => wc.metadata_param,
                    wrapper_type_path => wc.wrapper_type_path,
                    constructor_method => wc.constructor_method,
                    constructor_args => constructor_args.join(", "),
                },
            ));
        }

        // Build arguments for base register call
        let mut base_call_args = Vec::new();

        // Add wrapper param if present
        if let Some(wc) = &variant.wrapper_call {
            base_call_args.push(wc.metadata_param.clone());
        }

        // Add overridden metadata params
        for override_ in &variant.overrides {
            base_call_args.push(override_.value_expr.clone());
        }

        base_call_args.push("handler_arc".to_string());
        out.push_str(&template_env::render(
            "registration_function.rs.jinja",
            context! {
                service_pascal => service_pascal,
                method_pascal => variant_name,
                symbol => symbol,
                metadata_params_decl => free_params_decl,
                dispatch_method_name => dispatch_method_name,
                bridge_name => bridge_name,
                core_import => core_import,
                contract_name => contract_name,
                opaque_name => opaque_name,
                register_method => reg.method,
                register_args => base_call_args.join(", "),
                setup_block => wrapper_block,
            },
        ));
    }
}

/// Emit a JNI function for a service entrypoint (run or finalize).
///
/// Function signatures (in Java):
/// ```java,ignore
/// public native void run{ServiceName}(long ownerHandle, String addr, ...);
/// public native long finalize{ServiceName}(long ownerHandle, ...);
/// ```
fn gen_entrypoint_jni_function(
    out: &mut String,
    service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    core_import: &str,
    package: &str,
    service_bridge_class: &str,
) {
    let service_pascal = internal_class_component(&service.name);
    let ep_pascal = internal_class_component(&ep.method);
    let opaque_name = format!("{}Opaque", service.name);
    let ep_method = bridge_method_name(&service.name, &ep.method);
    let symbol = jni_symbol(package, service_bridge_class, &ep_method);

    let mut params_decl = String::new();
    for ep_param in &ep.params {
        let jni_type = typeref_to_jni_type(&ep_param.ty, core_import);
        params_decl.push_str(&render_service_param_decl(&ep_param.name, &jni_type));
    }
    let call_args = ep
        .params
        .iter()
        .map(|param| param.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    match ep.kind {
        EntrypointKind::Run => {
            out.push_str(&template_env::render(
                "entrypoint_run.rs.jinja",
                context! {
                    service_pascal => service_pascal,
                    ep_pascal => ep_pascal,
                    symbol => symbol,
                    params_decl => params_decl,
                    opaque_name => opaque_name,
                    ep_method => ep.method,
                    call_args => call_args,
                },
            ));
        }
        EntrypointKind::Finalize => {
            out.push_str(&template_env::render(
                "entrypoint_finalize.rs.jinja",
                context! {
                    service_pascal => service_pascal,
                    ep_pascal => ep_pascal,
                    symbol => symbol,
                    params_decl => params_decl,
                    opaque_name => opaque_name,
                    ep_method => ep.method,
                    call_args => call_args,
                },
            ));
        }
    }
}

// ──────────────────────────────────────────────────────── public entry point ──

/// Generate all service-API files for the JNI backend.
///
/// Returns up to one `GeneratedFile`:
/// - `crates/{name}-jni/src/service.rs` — Rust JNI glue for service lifecycle
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    let jni_crate = format!("{}-jni", config.jni_crate_base());
    let output_dir = PathBuf::from(format!("crates/{jni_crate}/src/service.rs"));

    let service_rs = gen_service_rs(api, config);

    Ok(vec![GeneratedFile {
        path: output_dir,
        content: service_rs,
        generated_header: true,
    }])
}

// ───────────────────────── Phase-C emission stubs (new IR sections) ──────────

/// Emit JNI lifecycle-hook registration methods.
///
/// Stub — returns `""` until the JNI Phase-C specialist implements generation.
pub(super) fn emit_lifecycle_hooks(hooks: &[crate::core::ir::LifecycleHookDef]) -> String {
    if hooks.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "lifecycle hook emission not implemented for jni ({} hooks)",
        hooks.len()
    );
    for _hook in hooks {}
    String::new()
}

/// Emit JNI WebSocket route registration methods. Stub.
pub(super) fn emit_websocket_routes(routes: &[crate::core::ir::WebSocketRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "WebSocket route emission not implemented for jni ({} routes)",
        routes.len()
    );
    for _route in routes {}
    String::new()
}

/// Emit JNI SSE route registration methods. Stub.
pub(super) fn emit_sse_routes(routes: &[crate::core::ir::SseRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!("SSE route emission not implemented for jni ({} routes)", routes.len());
    for _route in routes {}
    String::new()
}

/// Emit JNI native error types. Stub.
pub(super) fn emit_error_types(types: &[crate::core::ir::ErrorTypeDef]) -> String {
    if types.is_empty() {
        return String::new();
    }
    tracing::debug!("error type emission not implemented for jni ({} types)", types.len());
    for _ty in types {}
    String::new()
}

/// Aggregate stub — forwards all four new IR sections for the JNI backend.
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

    /// Construct a minimal but realistic [`ApiSurface`] that exercises:
    /// - A service with a constructor, one registration, and Run entrypoint
    /// - One [`HandlerContractDef`] with wire request/response DTO names
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
            error_type: None,
            doc: "Register a request handler.".to_owned(),
            variants: vec![],
            ..Default::default()
        };

        let run_ep = EntrypointDef {
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
            error_type: Some("ServiceError".to_owned()),
            doc: "Run the service.".to_owned(),
        };

        let service = ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![run_ep],
            doc: "A test service owner.".to_owned(),
            cfg: None,
        };

        let dispatch_method = MethodDef {
            name: "handle".to_owned(),
            params: vec![ParamDef {
                name: "request".to_owned(),
                ty: TypeRef::Named("RequestData".to_owned()),
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("ResponseData".to_owned()),
            is_async: true,
            is_static: false,
            error_type: Some("HandlerError".to_owned()),
            doc: "Dispatch a request.".to_owned(),
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
        };

        let contract = HandlerContractDef {
            trait_name: "RequestHandler".to_owned(),
            rust_path: "my_crate::RequestHandler".to_owned(),
            dispatch: dispatch_method,
            optional_methods: vec![],
            wire_request_type: Some("RequestData".to_owned()),
            wire_response_type: Some("ResponseData".to_owned()),
            dispatch_extra_params: vec![],
            wire_param_name: None,
            dispatch_return_type: None,
            response_adapter: None,
            doc: "Async trait for handling requests.".to_owned(),
        };

        ApiSurface {
            crate_name: "my_crate".to_owned(),
            version: "0.1.0".to_owned(),
            services: vec![service],
            handler_contracts: vec![contract],
            ..ApiSurface::default()
        }
    }

    /// Construct a fixture with registration variants for testing variant emission.
    fn make_fixture_surface_with_variants() -> ApiSurface {
        let mut surface = make_fixture_surface();
        if let Some(service) = surface.services.first_mut() {
            if let Some(reg) = service.registrations.first_mut() {
                // Add a "get" variant
                reg.variants.push(crate::core::ir::RegistrationVariant {
                    name: "get".to_owned(),
                    overrides: vec![crate::core::ir::RegistrationVariantOverride {
                        param_name: "path".to_owned(),
                        value_expr: "\"/api\"".to_owned(),
                    }],
                    wrapper_call: None,
                    signature_params: vec![],
                    doc: Some("Register a GET handler.".to_owned()),
                    style: Default::default(),
                    ..Default::default()
                });
            }
        }
        surface
    }

    /// `gen_service_rs` emits the JNI handler bridge struct.
    #[test]
    fn rust_output_contains_handler_bridge_struct() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("pub struct JniRequestHandlerBridge"),
            "expected `JniRequestHandlerBridge` struct in output:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge impl with async dispatch.
    #[test]
    fn rust_output_contains_handler_bridge_impl() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("impl my_crate::RequestHandler for JniRequestHandlerBridge"),
            "expected trait impl:\n{output}"
        );
        assert!(
            output.contains("fn handle(") && output.contains("Pin<Box<dyn std::future::Future<Output"),
            "expected boxed-future dispatch method:\n{output}"
        );
    }

    /// `gen_service_rs` emits JNI thread attachment code.
    #[test]
    fn rust_output_contains_jni_thread_attach() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("attach_current_thread"),
            "expected JVM thread attachment:\n{output}"
        );
    }

    /// `gen_service_rs` emits JSON serialization of request.
    #[test]
    fn rust_output_contains_json_serialization() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("serde_json::to_string(&request)"),
            "expected request JSON serialization:\n{output}"
        );
        assert!(
            output.contains("serde_json::from_str(&result_json)"),
            "expected response JSON deserialization:\n{output}"
        );
    }

    /// `gen_service_rs` emits JNI native method call.
    #[test]
    fn rust_output_contains_jni_method_call() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("call_method_unchecked"),
            "expected JNI method call:\n{output}"
        );
    }

    /// `gen_service_rs` emits registration entry point function that builds and calls the bridge.
    #[test]
    fn rust_output_register_calls_owner_method() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("#[no_mangle]"),
            "expected #[no_mangle] attribute:\n{output}"
        );
        assert!(
            output.contains("extern \"system\""),
            "expected extern system ABI:\n{output}"
        );
        assert!(
            output.contains("nativeTestServiceRegisterAddHandler"),
            "expected register function for TestService.add_handler:\n{output}"
        );
        // Verify the register function actually calls owner.add_handler
        assert!(
            output.contains(".inner.add_handler("),
            "register function must call owner.add_handler():\n{output}"
        );
        // Verify it creates the bridge
        assert!(
            output.contains("JniRequestHandlerBridge"),
            "register function must create the bridge:\n{output}"
        );
        // Verify it creates a GlobalRef and jmethodID
        assert!(
            output.contains("new_global_ref"),
            "register function must create global reference to handler:\n{output}"
        );
        assert!(
            output.contains("get_method_id"),
            "register function must cache method ID:\n{output}"
        );
    }

    /// `gen_service_rs` emits run entrypoint function that builds and drives the owner.
    #[test]
    fn rust_output_run_calls_owner_entrypoint() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("nativeTestServiceRun"),
            "expected run entrypoint function:\n{output}"
        );
        // Verify the run function creates a tokio runtime
        assert!(
            output.contains("tokio::runtime::Runtime::new"),
            "run function must create tokio runtime:\n{output}"
        );
        // Verify it dereferences and calls the owner's run method
        assert!(
            output.contains("owner_ref.run("),
            "run function must call owner.run():\n{output}"
        );
        // Verify it blocks on the async runtime
        assert!(
            output.contains("block_on"),
            "run function must block_on the async entrypoint:\n{output}"
        );
    }

    /// `gen_service_rs` emits opaque type and constructor.
    #[test]
    fn rust_output_contains_service_opaque_and_constructor() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        // Verify opaque struct is defined
        assert!(
            output.contains("pub struct TestServiceOpaque"),
            "expected TestServiceOpaque struct:\n{output}"
        );
        // Verify constructor entry point
        assert!(
            output.contains("nativeTestServiceNew"),
            "expected nativeTestServiceNew entry point:\n{output}"
        );
        // Verify it calls the Rust constructor
        assert!(
            output.contains("my_crate::TestService::new()"),
            "constructor must call the Rust service constructor:\n{output}"
        );
        // Verify it returns jlong (via Box::into_raw)
        assert!(
            output.contains("Box::into_raw"),
            "constructor must return raw pointer as jlong:\n{output}"
        );
    }

    /// `gen_service_rs` emits destructor for opaque handle.
    #[test]
    fn rust_output_contains_service_destructor() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        // Verify free entry point
        assert!(
            output.contains("nativeTestServiceFree"),
            "expected nativeTestServiceFree entry point:\n{output}"
        );
        // Verify it reconstructs from raw pointer
        assert!(
            output.contains("Box::from_raw"),
            "destructor must reconstruct from raw pointer:\n{output}"
        );
        // Verify it validates null pointer
        assert!(
            output.contains("if handle != 0"),
            "destructor must check for null pointer:\n{output}"
        );
    }

    /// `gen_service_rs` emits SAFETY comments on unsafe blocks.
    #[test]
    fn rust_output_contains_safety_comments() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("// SAFETY:"),
            "expected SAFETY comments on unsafe:\n{output}"
        );
    }

    /// Full `generate()` call returns one file when services are non-empty.
    #[test]
    fn generate_returns_one_file_for_non_empty_services() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert_eq!(files.len(), 1, "expected 1 generated file, got {}", files.len());
        let path = files[0].path.file_name().unwrap().to_str().unwrap();
        assert_eq!(path, "service.rs", "expected service.rs, got {path}");
    }

    /// Full `generate()` returns empty for a surface with no services.
    #[test]
    fn generate_returns_empty_for_no_services() {
        let surface = ApiSurface::default();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    /// `gen_service_rs` emits registration variant native functions when variants are present.
    #[test]
    fn rust_output_contains_registration_variants() {
        let surface = make_fixture_surface_with_variants();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        // Verify the variant registration function is emitted
        assert!(
            output.contains("nativeTestServiceRegisterAddHandlerGet"),
            "expected variant registration function nativeTestServiceRegisterAddHandlerGet:\n{output}"
        );
        // Verify it builds wrapper and calls base registration
        assert!(
            output.contains("inner.add_handler("),
            "variant function must call the base registration method:\n{output}"
        );
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_test_config() -> ResolvedCrateConfig {
        use crate::core::config::resolved::ResolvedCrateConfig;
        ResolvedCrateConfig {
            name: "my-crate".to_owned(),
            ..ResolvedCrateConfig::default()
        }
    }
}
