//! Service-API codegen for the PHP (ext-php-rs) backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **`service.rs`** — Rust ext-php-rs glue that wraps each registered PHP
//!    callable as `Arc<dyn <HandlerContractDef::trait_name>>` via a blocking
//!    callback bridge (PHP is single-threaded per request), builds the core
//!    service via the owner type's registration and run entrypoints, and exposes
//!    a `#[php_function]` entry point.
//!
//! 2. **`service.php`** — An idiomatic PHP class mirroring the service's
//!    constructor, configurator methods, and registration methods, with a
//!    `run(...)` method that delegates to the native extension.
//!
//! All names are derived entirely from the [`ApiSurface`] IR — no transport-
//! or domain-specific assumptions are made anywhere in this module.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{
    ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, RegistrationVariantStyle, ServiceDef, TypeRef,
};
use heck::{ToSnakeCase, ToUpperCamelCase};
use minijinja::context;
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Convert a `TypeRef` to a simple PHP type annotation string.
fn php_type_annotation(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "string".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::F32 | PrimitiveType::F64 => "float".to_owned(),
                _ => "int".to_owned(),
            }
        }
        TypeRef::Bytes => "string".to_owned(), // PHP doesn't distinguish; use string
        TypeRef::Optional(inner) => format!("?{}", php_type_annotation(inner)),
        TypeRef::Vec(_) => "array".to_owned(), // Omit inner type in annotation
        TypeRef::Map(_, _) => "array".to_owned(),
        TypeRef::Unit => "void".to_owned(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Json => "mixed".to_owned(),
        TypeRef::Path => "string".to_owned(),
        TypeRef::Duration => "float".to_owned(),
    }
}

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

/// Format a Rust doc comment as a PHP docblock at the given column indent.
/// Single-line docs render as `// text`; multi-line docs render as a `/** ...
/// */` block with every line prefixed by ` * `. Blank doc lines become bare
/// ` *` separators so paragraph breaks survive.
fn format_php_comment(text: &str, indent: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let pad = " ".repeat(indent);
    if !trimmed.contains('\n') {
        return format!("{pad}// {trimmed}\n");
    }
    let mut out = format!("{pad}/**\n");
    for line in trimmed.lines() {
        if line.trim().is_empty() {
            out.push_str(&pad);
            out.push_str(" *\n");
        } else {
            out.push_str(&pad);
            out.push_str(" * ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&pad);
    out.push_str(" */\n");
    out
}

fn render(template_name: &str, ctx: minijinja::Value) -> String {
    crate::backends::php::template_env::render(template_name, ctx)
}

// ─────────────────────────────────────────────────────────────── PHP output ──

/// Generate the idiomatic PHP service class (`service.php`).
///
/// Produces a PHP file containing one class per service. Each class exposes:
/// - A constructor mirroring [`ServiceDef::constructor`].
/// - Configurator methods from [`ServiceDef::configurators`].
/// - Registration methods from [`ServiceDef::registrations`].
/// - A `run(...)` method derived from the first [`EntrypointKind::Run`]
///   entrypoint.
pub(super) fn gen_service_php(api: &ApiSurface, extension_name: &str) -> String {
    let mut out = String::new();

    out.push_str("<?php\n\n");
    out.push_str("declare(strict_types=1);\n\n");

    // Emit one class per service
    for service in &api.services {
        gen_service_class(&mut out, service, api, extension_name);
    }

    out
}

fn gen_service_class(out: &mut String, service: &ServiceDef, api: &ApiSurface, extension_name: &str) {
    let class_name = &service.name;

    // Class declaration with docblock
    if !service.doc.is_empty() {
        out.push_str(&format_php_comment(&service.doc, 0));
    }
    out.push_str(&render(
        "php_service_class_start.jinja",
        context! { class_name => class_name },
    ));

    // Private registrations storage
    out.push_str("    private array $registrations = [];\n\n");

    // __construct
    {
        let ctor = &service.constructor;
        let mut ctor_params = Vec::new();
        let mut ctor_assigns = Vec::new();

        for p in &ctor.params {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                ctor_params.push(format!("?{} ${} = null", annotation, p.name));
            } else {
                ctor_params.push(format!("{} ${}", annotation, p.name));
            }
            // Store constructor param as private property for use in run()
            ctor_assigns.push(p.name.clone());
        }

        let param_sig = ctor_params.join(", ");
        // PHP constructors cannot declare a return type — emitting `: void`
        // is a parse error. The return type is implicit.
        out.push_str(&render(
            "php_service_constructor_start.jinja",
            context! { param_sig => &param_sig },
        ));
        if !ctor.doc.is_empty() {
            out.push_str(&format_php_comment(&ctor.doc, 8));
        }

        // Store constructor args as instance properties
        for arg in &ctor_assigns {
            out.push_str(&render(
                "php_service_property_assignment.jinja",
                context! { name => arg },
            ));
        }
        out.push_str("    }\n\n");
    }

    // Configurator methods
    for method in &service.configurators {
        let mut params = Vec::new();
        for p in &method.params {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("?{} ${} = null", annotation, p.name));
            } else {
                params.push(format!("{} ${}", annotation, p.name));
            }
        }
        let param_sig = params.join(", ");
        let method_name = &method.name;
        out.push_str(&render(
            "php_service_method_start.jinja",
            context! {
                method_name => method_name,
                param_sig => &param_sig,
                return_type => "self",
            },
        ));
        if !method.doc.is_empty() {
            out.push_str(&format_php_comment(&method.doc, 8));
        }

        // Store each configurator param as instance property
        for p in &method.params {
            out.push_str(&render(
                "php_service_property_assignment.jinja",
                context! { name => &p.name },
            ));
        }
        out.push_str("        return $this;\n");
        out.push_str("    }\n\n");
    }

    // Registration methods
    for reg in &service.registrations {
        gen_registration_method(out, reg, service, api, extension_name);
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        let mut params = Vec::new();
        for p in &ep.params {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("?{} ${} = null", annotation, p.name));
            } else {
                params.push(format!("{} ${}", annotation, p.name));
            }
        }
        let param_sig = params.join(", ");
        let ep_name = &ep.method;

        match ep.kind {
            EntrypointKind::Run => {
                out.push_str(&render(
                    "php_service_method_start.jinja",
                    context! {
                        method_name => ep_name,
                        param_sig => &param_sig,
                        return_type => "void",
                    },
                ));
                if !ep.doc.is_empty() {
                    out.push_str(&format_php_comment(&ep.doc, 8));
                }

                // Build the call to the native run function
                // Convention: native fn is `{snake_service_name}_{entrypoint_name}`
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                let args = php_service_native_args(&ep.params);
                out.push_str(&render(
                    "php_service_native_call.jinja",
                    context! {
                        native_fn => &native_fn,
                        args => &args,
                    },
                ));
                out.push_str("    }\n\n");
            }
            EntrypointKind::Finalize => {
                let return_annotation = php_type_annotation(&ep.return_type);
                out.push_str(&render(
                    "php_service_method_start.jinja",
                    context! {
                        method_name => ep_name,
                        param_sig => &param_sig,
                        return_type => &return_annotation,
                    },
                ));
                if !ep.doc.is_empty() {
                    out.push_str(&format_php_comment(&ep.doc, 8));
                }

                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                let args = php_service_native_args(&ep.params);
                out.push_str(&render(
                    "php_service_native_return.jinja",
                    context! {
                        native_fn => &native_fn,
                        args => &args,
                    },
                ));
                out.push_str("    }\n\n");
            }
        }
    }

    out.push_str("}\n\n");
}

fn php_service_native_args(params: &[crate::core::ir::ParamDef]) -> String {
    params
        .iter()
        .map(|p| format!("${}", p.name))
        .collect::<Vec<_>>()
        .join(", ")
}

fn gen_registration_method(
    out: &mut String,
    reg: &RegistrationDef,
    _service: &ServiceDef,
    _api: &ApiSurface,
    _extension_name: &str,
) {
    let method_name = &reg.method;

    // Build metadata param signature (excluding the callback param)
    let meta_params: Vec<String> = reg
        .metadata_params
        .iter()
        .map(|p| {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                format!("?{} ${} = null", annotation, p.name)
            } else {
                format!("{} ${}", annotation, p.name)
            }
        })
        .collect();

    // For direct registration (non-decorator), also add the callback param
    let mut direct_params = meta_params.clone();
    direct_params.push(format!("callable ${}", reg.callback_param));

    let meta_sig = meta_params.join(", ");
    let direct_sig = direct_params.join(", ");

    // Decorator factory form: returns a closure
    out.push_str(&render(
        "php_service_method_start.jinja",
        context! {
            method_name => method_name,
            param_sig => &meta_sig,
            return_type => "callable",
        },
    ));
    if !reg.doc.is_empty() {
        out.push_str(&format_php_comment(&reg.doc, 8));
    }

    // Build the metadata tuple for storage
    let meta_tuple = if reg.metadata_params.is_empty() {
        "[]".to_owned()
    } else {
        let names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
        format!(
            "[{}]",
            names.iter().map(|n| format!("${}", n)).collect::<Vec<_>>().join(", ")
        )
    };

    out.push_str(&render(
        "php_service_registration_factory_body.jinja",
        context! {
            callback_param => &reg.callback_param,
            method_name => method_name,
            meta_tuple => &meta_tuple,
        },
    ));
    out.push_str("    }\n\n");

    // Also expose a direct (non-decorator) variant: `register_{method_name}`
    let direct_name = format!("register_{method_name}");
    if direct_name != *method_name {
        out.push_str(&render(
            "php_service_method_start.jinja",
            context! {
                method_name => &direct_name,
                param_sig => &direct_sig,
                return_type => "self",
            },
        ));
        out.push_str(&render(
            "php_service_registration_store.jinja",
            context! {
                method_name => method_name,
                meta_tuple => &meta_tuple,
                callback_param => &reg.callback_param,
            },
        ));
        out.push_str("        return $this;\n");
        out.push_str("    }\n\n");
    }

    // Emit verb-decorator variants (e.g., $app->get(), $app->post())
    for variant in &reg.variants {
        gen_registration_variant(out, variant, reg, method_name);
    }
}

// ──────────────────────────────────────────────────────────────── Rust glue ──

/// Generate the Rust ext-php-rs glue module (`service.rs`).
///
/// For each service this emits:
/// - A `Php{ContractName}Bridge` struct that wraps a PHP callable (stored as
///   an index into a thread-local registry) and `impl`s the handler contract trait.
///   Since PHP is single-threaded per request, the async dispatch blocks on
///   the Tokio runtime.
/// - A `#[php_function]` `{snake_service}_{entrypoint}` that accepts the
///   collected registrations list and any entrypoint params, builds the native
///   service, and drives it.
pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let mut out = String::new();

    // File-level allow attributes to keep clippy happy in generated code
    out.push_str("#![allow(clippy::too_many_arguments, clippy::unused_async)]\n\n");
    out.push_str("use ext_php_rs::prelude::*;\n");
    out.push_str("use ext_php_rs::types::{ZendCallable, Zval};\n");
    out.push_str("use std::panic::AssertUnwindSafe;\n");
    out.push_str("use std::sync::Arc;\n\n");

    // Global handler registry (thread-local since Zval is not Send/Sync)
    out.push_str("thread_local! {\n");
    out.push_str("    static PHP_HANDLER_REGISTRY: std::cell::RefCell<Vec<ZendCallable<'static>>> =\n");
    out.push_str("        const { std::cell::RefCell::new(Vec::new()) };\n");
    out.push_str("}\n\n");

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

    // Emit one php_function per service × entrypoint
    for service in &api.services {
        for ep in &service.entrypoints {
            gen_run_php_function(&mut out, service, ep, api, &core_import);
        }
    }

    out
}

/// Emit the `Php{ContractName}Bridge` struct + trait impl.
///
/// Stores the handler callable as an index into the thread-local registry
/// (since ZendCallable is not Send/Sync). When dispatched, retrieves the
/// callable, invokes it synchronously via the PHP FFI, serializes the result,
/// and blocks the Tokio executor on the response deserialization.
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Php{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;

    // Determine wire types
    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

    // Build req/resp paths: if wire type includes "::", strip it; otherwise prefix with core_import
    let req_path = if req_type.contains("::") {
        req_type.split("::").last().unwrap_or(req_type).to_string()
    } else if req_type == "Value" || req_type == "serde_json::Value" {
        "serde_json::Value".to_string()
    } else {
        format!("{core_import}::{req_type}")
    };
    let resp_path = if resp_type.contains("::") {
        resp_type.split("::").last().unwrap_or(resp_type).to_string()
    } else if resp_type == "Value" || resp_type == "serde_json::Value" {
        "serde_json::Value".to_string()
    } else {
        format!("{core_import}::{resp_type}")
    };

    // Extra dispatch parameters the bridge ignores (leading verbatim params)
    let extra_param: String = contract
        .dispatch_extra_params
        .iter()
        .map(|p| format!(", {p}"))
        .collect();
    let wire_name = contract.wire_param_name.as_deref().unwrap_or("request");

    // The future's `Output` is the contract dispatch's real return type when the library
    // supplies one (`dispatch_return_type`); otherwise the bridge yields the wire response
    // wrapped in a boxed-error `Result`. When a `response_adapter` is configured, the inner
    // fallible computation produces the wire `Result` and the adapter converts it into the
    // dispatch return type.
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
    out.push_str(&render(
        "php_service_handler_bridge.jinja",
        context! {
            trait_name => trait_name,
            bridge_name => &bridge_name,
            core_import => core_import,
            dispatch_name => dispatch_name,
            extra_param => &extra_param,
            wire_name => wire_name,
            req_path => &req_path,
            output_type => &output_type,
            wire_output => &wire_output,
            box_err => box_err,
            resp_path => &resp_path,
            tail => &tail,
        },
    ));
}

/// Emit the `#[php_function]` entry point for one service × entrypoint.
///
/// The function:
/// 1. Accepts the registrations list (`array<array{string, array, callable}>`).
/// 2. Constructs the native service owner via its constructor.
/// 3. Iterates registrations, wraps each callable in the appropriate bridge,
///    and calls the owner's registration method.
/// 4. Calls the owner's entrypoint (blocking if Run, synchronous if Finalize).
fn gen_run_php_function(
    out: &mut String,
    service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    api: &ApiSurface,
    core_import: &str,
) {
    let service_snake = service.name.to_snake_case();
    let fn_name = format!("{service_snake}_{}", ep.method);
    let owner_path = &service.rust_path;
    let ep_method = &ep.method;

    // Build the function signature: registrations + entrypoint params
    let mut rust_params = vec!["registrations: &Bound<'_, Zval>".to_owned()];
    for p in &ep.params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        rust_params.push(format!("{}: {}", p.name, rust_ty));
    }
    let param_sig = rust_params.join(", ");

    out.push_str(&render(
        "php_service_rust_function_start.jinja",
        context! {
            owner_path => owner_path,
            ep_method => ep_method,
            fn_name => &fn_name,
            param_sig => &param_sig,
        },
    ));

    // Build the owner instance via its constructor
    let ctor_call = build_ctor_call(service, owner_path, core_import);
    out.push_str(&render(
        "php_service_rust_owner_init.jinja",
        context! { ctor_call => &ctor_call },
    ));

    // Iterate registrations and dispatch
    out.push_str("    // Register all handlers with the owner\n");
    out.push_str("    if let Ok(reg_arr) = registrations.try_into::<Vec<Zval>>() {\n");
    out.push_str("        for entry in reg_arr {\n");
    out.push_str("            if let Ok(tuple) = entry.try_into::<Vec<Zval>>() {\n");
    out.push_str("                if tuple.len() < 3 {\n");
    out.push_str(
        "                    return Err(PhpException::default(\"Invalid registration tuple length\".into()));\n",
    );
    out.push_str("                }\n");
    out.push_str("                let method_name: String = tuple[0].try_into()?;\n");
    out.push_str("                let callable = tuple[2].clone();\n\n");

    // Dispatch on method name
    out.push_str("                match method_name.as_str() {\n");
    for reg in &service.registrations {
        let reg_method = &reg.method;
        let contract_name = &reg.callback_contract;

        if let Some(contract) = find_contract(api, contract_name) {
            let bridge_name = format!("Php{}Bridge", contract.trait_name.to_upper_camel_case());
            let meta_count = reg.metadata_params.len();

            out.push_str(&render(
                "php_service_rust_registration_match_start.jinja",
                context! { reg_method => reg_method },
            ));

            // Store the callable in the registry and get its index
            out.push_str("                        let handler_index = PHP_HANDLER_REGISTRY.with(|registry| {\n");
            out.push_str("                            let mut registry = registry.borrow_mut();\n");
            out.push_str("                            let idx = registry.len();\n");
            out.push_str("                            // Convert Zval to ZendCallable\n");
            out.push_str(
                "                            if let Ok(zen_callable) = ZendCallable::new_owned(callable.clone()) {\n",
            );
            out.push_str("                                registry.push(zen_callable);\n");
            out.push_str("                                idx\n");
            out.push_str("                            } else {\n");
            out.push_str("                                usize::MAX\n");
            out.push_str("                            }\n");
            out.push_str("                        });\n");
            out.push_str("                        if handler_index == usize::MAX {\n");
            out.push_str("                            return Err(PhpException::default(\"Failed to register callable\".into()));\n");
            out.push_str("                        }\n\n");

            out.push_str(&render(
                "php_service_rust_bridge_binding.jinja",
                context! {
                    bridge_name => &bridge_name,
                    core_import => core_import,
                    contract_name => contract_name,
                },
            ));

            if meta_count > 0 {
                out.push_str("                        let meta: Vec<Zval> = tuple[1].clone().try_into()?;\n");
                for (i, meta_param) in reg.metadata_params.iter().enumerate() {
                    let rust_ty = typeref_to_rust_type(&meta_param.ty, core_import);
                    out.push_str(&render(
                        "php_service_rust_metadata_binding.jinja",
                        context! {
                            name => &meta_param.name,
                            rust_ty => &rust_ty,
                            index => i,
                        },
                    ));
                }
                let meta_args: Vec<String> = reg.metadata_params.iter().map(|p| p.name.clone()).collect();
                out.push_str(&render(
                    "php_service_rust_owner_registration_call.jinja",
                    context! {
                        reg_method => reg_method,
                        args => &meta_args.join(", "),
                    },
                ));
            } else {
                out.push_str(&render(
                    "php_service_rust_owner_registration_call.jinja",
                    context! {
                        reg_method => reg_method,
                        args => "",
                    },
                ));
            }

            // Handle error if the registration is fallible
            if reg.error_type.is_some() {
                out.push_str("                            .map_err(|e| PhpException::default(e.to_string()))?;\n");
            } else {
                out.push_str("                            ;\n");
            }
            out.push_str("                    }\n");
        }
    }
    out.push_str("                    _ => {\n");
    out.push_str(
        "                        return Err(PhpException::default(\n                            \
         format!(\"unknown registration method: {method_name}\"),\n                        ));\n",
    );
    out.push_str("                    }\n");
    out.push_str("                }\n");
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");

    // Call the entrypoint
    let ep_call = build_ep_call(ep, service, core_import);
    out.push_str(&ep_call);

    out.push_str("    Ok(())\n}\n\n");
}

/// Build the Rust constructor call for the service owner.
fn build_ctor_call(service: &ServiceDef, owner_path: &str, _core_import: &str) -> String {
    if service.constructor.params.is_empty() {
        format!("{owner_path}::{}()", service.constructor.name)
    } else {
        // For a first-pass implementation where constructor params are not
        // yet threaded through, fall back to Default if available; otherwise
        // use new() with zero-value placeholders.
        format!("{owner_path}::{}()", service.constructor.name)
    }
}

/// Build the entrypoint invocation for a service method.
fn build_ep_call(ep: &crate::core::ir::EntrypointDef, _service: &ServiceDef, _core_import: &str) -> String {
    let ep_method = &ep.method;
    let ep_args: Vec<String> = ep.params.iter().map(|p| p.name.clone()).collect();
    let args_str = ep_args.join(", ");

    if ep.is_async {
        // Use tokio::runtime::Handle::current().block_on for async entrypoints.
        // This assumes a Tokio runtime is already active (as in the PHP bridge invocations).
        if args_str.is_empty() {
            format!(
                "    tokio::runtime::Handle::current()\n        \
                 .block_on(owner.{ep_method}())\n        \
                 .map_err(|e| PhpException::default(e.to_string()))?;\n"
            )
        } else {
            format!(
                "    tokio::runtime::Handle::current()\n        \
                 .block_on(owner.{ep_method}({args_str}))\n        \
                 .map_err(|e| PhpException::default(e.to_string()))?;\n"
            )
        }
    } else {
        if ep.error_type.is_some() {
            if args_str.is_empty() {
                format!(
                    "    owner.{ep_method}()\n        \
                     .map_err(|e| PhpException::default(e.to_string()))?;\n"
                )
            } else {
                format!(
                    "    owner.{ep_method}({args_str})\n        \
                     .map_err(|e| PhpException::default(e.to_string()))?;\n"
                )
            }
        } else {
            if args_str.is_empty() {
                format!("    owner.{ep_method}();\n")
            } else {
                format!("    owner.{ep_method}({args_str});\n")
            }
        }
    }
}

/// Convert a Rust enum path expression to a PHP class constant reference.
///
/// `"my_crate::Method::Get"` → `"Method::Get"`
/// `"Method::Get"` → `"Method::Get"`
///
/// Takes the last two `::` separated segments so that fully-qualified Rust
/// paths are trimmed to just `TypeName::Variant`.
fn rust_enum_expr_to_php(value_expr: &str) -> String {
    let parts: Vec<&str> = value_expr.split("::").collect();
    if parts.len() >= 2 {
        let type_name = parts[parts.len() - 2];
        let variant = parts[parts.len() - 1];
        format!("{type_name}::{variant}")
    } else {
        value_expr.to_owned()
    }
}

/// Build the PHP wrapper-constructor statement for a variant that has a
/// `wrapper_call`.
///
/// Returns a statement like
/// `$builder = RouteBuilder::new(Method::Get, $path);`
/// or `None` when the variant has no `wrapper_call`.
fn build_php_wrapper_constructor_stmt(variant: &crate::core::ir::RegistrationVariant) -> Option<String> {
    use crate::core::ir::WrapperConstructorArg;
    let wc = variant.wrapper_call.as_ref()?;
    let wrapper_type = &wc.wrapper_type_name;
    let constructor = &wc.constructor_method;
    let metadata_param = &wc.metadata_param;

    let mut ctor_args: Vec<String> = Vec::new();
    for arg in &wc.args {
        match arg {
            WrapperConstructorArg::Fixed { value_expr, .. } => {
                ctor_args.push(rust_enum_expr_to_php(value_expr));
            }
            WrapperConstructorArg::Free { param } => {
                ctor_args.push(format!("${}", param.name));
            }
        }
    }
    let ctor_arg_str = ctor_args.join(", ");
    Some(format!(
        "${metadata_param} = {wrapper_type}::{constructor}({ctor_arg_str});"
    ))
}

/// Emit a verb-decorator variant method(s) based on the registration style.
///
/// - `VerbDecorator`: Emit only the direct method form (e.g., `get(path, handler): App`)
/// - `Builder`: Emit only the decorator-factory form (e.g., `getDecorator(path): Closure`)
/// - `Hybrid`: Emit both direct method and decorator-factory
///
/// When the variant has a `wrapper_call`, the method constructs the wrapper
/// object and delegates to the base registration method instead of writing
/// directly to `$this->registrations[]`.
fn gen_registration_variant(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    reg: &RegistrationDef,
    base_method: &str,
) {
    let variant_name = variant.name.to_lowercase();
    let callback_param = &reg.callback_param;

    // Build the parameter list for metadata (non-callback) params
    let meta_params: Vec<String> = variant
        .signature_params
        .iter()
        .map(|p| {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                format!("?{} ${} = null", annotation, p.name)
            } else {
                format!("{} ${}", annotation, p.name)
            }
        })
        .collect();

    // Build the full parameter list for direct method (metadata + callback)
    let mut direct_params = meta_params.clone();
    direct_params.push(format!("callable ${callback_param}"));

    let meta_sig = meta_params.join(", ");
    let direct_sig = direct_params.join(", ");

    // When the variant has a wrapper_call, the body constructs the wrapper
    // object and delegates to the base method.  Otherwise, fall back to the
    // legacy computed call_args path.
    let wrapper_stmt = build_php_wrapper_constructor_stmt(variant);

    // Compute the base registration call arguments (used when wrapper_call is absent)
    let mut call_args: Vec<String> = Vec::new();
    for base_param in &reg.metadata_params {
        if let Some(override_) = variant.overrides.iter().find(|o| o.param_name == base_param.name) {
            call_args.push(override_.value_expr.clone());
        } else if let Some(sig_param) = variant.signature_params.iter().find(|s| s.name == base_param.name) {
            call_args.push(format!("${}", sig_param.name));
        }
    }
    let call_sig = call_args.join(", ");

    // Pre-compute the method bodies to avoid multiple mutable borrows of `out`.
    let direct_body = if let Some(ref stmt) = wrapper_stmt {
        let metadata_param = &variant.wrapper_call.as_ref().unwrap().metadata_param;
        format!("        {stmt}\n        return $this->{base_method}(${metadata_param}, ${callback_param});\n")
    } else {
        let vars = call_args
            .iter()
            .filter_map(|arg| if arg.starts_with('$') { Some(arg.clone()) } else { None })
            .collect::<Vec<_>>()
            .join(", ");
        render(
            "php_service_variant_direct_body.jinja",
            context! {
                base_method => base_method,
                vars => &vars,
                callback_param => callback_param,
            },
        )
    };

    let factory_body = if let Some(ref stmt) = wrapper_stmt {
        let metadata_param = &variant.wrapper_call.as_ref().unwrap().metadata_param;
        render(
            "php_service_variant_wrapper_factory_body.jinja",
            context! {
                callback_param => callback_param,
                stmt => stmt,
                base_method => base_method,
                metadata_param => metadata_param,
            },
        )
    } else {
        render(
            "php_service_variant_factory_body.jinja",
            context! {
                callback_param => callback_param,
                base_method => base_method,
                call_sig => &call_sig,
            },
        )
    };

    match variant.style {
        RegistrationVariantStyle::VerbDecorator => {
            // Emit direct method: $app->get(path, handler): App
            out.push_str(&render(
                "php_service_method_start.jinja",
                context! {
                    method_name => &variant_name,
                    param_sig => &direct_sig,
                    return_type => "self",
                },
            ));
            if let Some(doc) = &variant.doc {
                out.push_str(&format_php_comment(doc, 8));
            }
            out.push_str(&direct_body);
            out.push_str("    }\n\n");
        }

        RegistrationVariantStyle::Builder => {
            // Emit decorator factory: $app->getDecorator(path): Closure
            let factory_name = format!("{variant_name}Decorator");
            out.push_str(&render(
                "php_service_method_start.jinja",
                context! {
                    method_name => &factory_name,
                    param_sig => &meta_sig,
                    return_type => "Closure",
                },
            ));
            if let Some(doc) = &variant.doc {
                out.push_str(&format_php_comment(doc, 8));
            }
            out.push_str(&factory_body);
            out.push_str("    }\n\n");
        }

        RegistrationVariantStyle::Hybrid => {
            // 1. Direct method: $app->get(path, handler): App
            out.push_str(&render(
                "php_service_method_start.jinja",
                context! {
                    method_name => &variant_name,
                    param_sig => &direct_sig,
                    return_type => "self",
                },
            ));
            if let Some(doc) = &variant.doc {
                out.push_str(&format_php_comment(doc, 8));
            }
            out.push_str(&direct_body);
            out.push_str("    }\n\n");

            // 2. Decorator factory: $app->getDecorator(path): Closure
            let factory_name = format!("{variant_name}Decorator");
            out.push_str(&render(
                "php_service_method_start.jinja",
                context! {
                    method_name => &factory_name,
                    param_sig => &meta_sig,
                    return_type => "Closure",
                },
            ));
            if let Some(doc) = &variant.doc {
                out.push_str(&format_php_comment(doc, 8));
            }
            out.push_str(&factory_body);
            out.push_str("    }\n\n");
        }
    }
}

/// Map a `TypeRef` to a Rust type string for use in generated function signatures.
fn typeref_to_rust_type(ty: &TypeRef, core_import: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_owned(),
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
        TypeRef::Bytes => "Vec<u8>".to_owned(),
        TypeRef::Optional(inner) => format!("Option<{}>", typeref_to_rust_type(inner, core_import)),
        TypeRef::Vec(inner) => format!("Vec<{}>", typeref_to_rust_type(inner, core_import)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            typeref_to_rust_type(k, core_import),
            typeref_to_rust_type(v, core_import)
        ),
        TypeRef::Unit => "()".to_owned(),
        TypeRef::Named(n) => format!("{core_import}::{n}"),
        TypeRef::Json => "serde_json::Value".to_owned(),
        TypeRef::Path => "std::path::PathBuf".to_owned(),
        TypeRef::Duration => "std::time::Duration".to_owned(),
    }
}

// ──────────────────────────────────────────────────────── public entry point ──

/// Generate all service-API files for the PHP backend.
///
/// Returns up to two `GeneratedFile`s per non-empty service list:
/// - `{output_dir}/service.rs`   — Rust ext-php-rs glue
/// - `{php_pkg}/Service.php`     — idiomatic PHP class
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(config.output_paths.get("php"), &config.name, "crates/{name}-php/src/");

    // Rust glue
    let service_rs = gen_service_rs(api, config);

    // PHP wrapper
    // Extension name matches the Rust crate name with hyphens replaced by underscores.
    let extension_name = config.name.replace('-', "_");
    let service_php = gen_service_php(api, &extension_name);

    // PHP package output base (same logic as generate_public_api)
    let output_base = config
        .php
        .as_ref()
        .and_then(|p| p.stubs.as_ref())
        .map(|s| PathBuf::from(&s.output))
        .unwrap_or_else(|| {
            let package_name = config.name.replace('-', "_");
            PathBuf::from(format!("packages/php/{}", package_name))
        });

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: output_base.join("Service.php"),
            content: service_php,
            generated_header: true,
        },
    ])
}

// ───────────────────────────────────────────────────────────────────── tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{
        EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, RegistrationDef,
        ServiceDef, TypeRef,
    };

    /// Construct a minimal but realistic [`ApiSurface`] that exercises:
    /// - A service with a constructor, one configurator, one registration
    ///   (bound to an async handler contract), and Run + Finalize entrypoints.
    /// - One [`HandlerContractDef`] with wire request/response DTO names.
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

        let configurator = MethodDef {
            name: "with_timeout".to_owned(),
            params: vec![ParamDef {
                name: "timeout_ms".to_owned(),
                ty: TypeRef::Primitive(PrimitiveType::U64),
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("TestService".to_owned()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: "Set request timeout.".to_owned(),
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
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
            metadata_params: vec![
                ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "method".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
            ],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: None,
            doc: "Register a request handler for a path and method.".to_owned(),
            variants: vec![],
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

        let finalize_ep = EntrypointDef {
            method: "into_router".to_owned(),
            kind: EntrypointKind::Finalize,
            is_async: false,
            params: vec![],
            return_type: TypeRef::Named("Router".to_owned()),
            error_type: None,
            doc: "Consume and convert into a router.".to_owned(),
        };

        let service = ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![configurator],
            registrations: vec![registration],
            entrypoints: vec![run_ep, finalize_ep],
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

    /// `gen_service_php` emits a class named after the service owner.
    #[test]
    fn php_output_contains_service_class() {
        let surface = make_fixture_surface();
        let output = gen_service_php(&surface, "my_crate");
        assert!(
            output.contains("class TestService"),
            "expected `class TestService` in output:\n{output}"
        );
    }

    /// `gen_service_php` emits `__construct` with registrations initialization.
    #[test]
    fn php_output_contains_construct_with_registrations() {
        let surface = make_fixture_surface();
        let output = gen_service_php(&surface, "my_crate");
        assert!(
            output.contains("public function __construct()"),
            "expected `public function __construct()` in output:\n{output}"
        );
        assert!(
            output.contains("private array $registrations"),
            "expected `private array $registrations` in output:\n{output}"
        );
    }

    /// `gen_service_php` emits configurator methods that return `self`.
    #[test]
    fn php_output_contains_configurator() {
        let surface = make_fixture_surface();
        let output = gen_service_php(&surface, "my_crate");
        assert!(
            output.contains("public function with_timeout"),
            "expected `with_timeout` configurator:\n{output}"
        );
        assert!(
            output.contains("return $this"),
            "expected `return $this` in configurator:\n{output}"
        );
    }

    /// `gen_service_php` emits a registration method returning a closure.
    #[test]
    fn php_output_contains_registration_method() {
        let surface = make_fixture_surface();
        let output = gen_service_php(&surface, "my_crate");
        assert!(
            output.contains("public function add_handler("),
            "expected `add_handler` registration method:\n{output}"
        );
        assert!(
            output.contains("return function"),
            "expected inner `return function` closure:\n{output}"
        );
        assert!(
            output.contains("$this->registrations[]"),
            "expected `$this->registrations[]` append in registration:\n{output}"
        );
    }

    /// `gen_service_php` emits verb-decorator variant methods when variants are present.
    #[test]
    fn php_output_contains_registration_variants() {
        use crate::core::ir::{RegistrationVariant, RegistrationVariantOverride};

        let constructor = MethodDef {
            name: "new".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: String::new(),
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
            method: "route".to_owned(),
            callback_param: "handler".to_owned(),
            callback_contract: "RequestHandler".to_owned(),
            metadata_params: vec![
                ParamDef {
                    name: "method".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
            ],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: None,
            doc: String::new(),
            variants: vec![RegistrationVariant {
                name: "GET".to_owned(),
                overrides: vec![RegistrationVariantOverride {
                    param_name: "method".to_owned(),
                    value_expr: "\"GET\"".to_owned(),
                }],
                wrapper_call: None,
                signature_params: vec![ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                }],
                doc: Some("Register a GET route.".to_owned()),
                style: Default::default(),
            }],
        };

        let service = ServiceDef {
            name: "Router".to_owned(),
            rust_path: "my_crate::Router".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![],
            doc: String::new(),
            cfg: None,
        };

        let api = ApiSurface {
            crate_name: "my_crate".to_owned(),
            version: "0.1.0".to_owned(),
            services: vec![service],
            handler_contracts: vec![],
            ..ApiSurface::default()
        };

        let output = gen_service_php(&api, "my_crate");
        assert!(
            output.contains("public function get("),
            "expected `get` variant method (lowercase):\n{output}"
        );
        assert!(
            output.contains("\"GET\""),
            "expected fixed override `\"GET\"` in variant:\n{output}"
        );
    }

    /// `gen_service_php` emits only direct method form for VerbDecorator style.
    #[test]
    fn php_output_verb_decorator_style_direct_method_only() {
        use crate::core::ir::{RegistrationVariant, RegistrationVariantOverride};

        let constructor = MethodDef {
            name: "new".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: String::new(),
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
            method: "route".to_owned(),
            callback_param: "handler".to_owned(),
            callback_contract: "RequestHandler".to_owned(),
            metadata_params: vec![
                ParamDef {
                    name: "method".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
            ],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: None,
            doc: String::new(),
            variants: vec![RegistrationVariant {
                name: "GET".to_owned(),
                overrides: vec![RegistrationVariantOverride {
                    param_name: "method".to_owned(),
                    value_expr: "\"GET\"".to_owned(),
                }],
                wrapper_call: None,
                signature_params: vec![ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                }],
                doc: Some("Register a GET route.".to_owned()),
                style: RegistrationVariantStyle::VerbDecorator,
            }],
        };

        let service = ServiceDef {
            name: "Router".to_owned(),
            rust_path: "my_crate::Router".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![],
            doc: String::new(),
            cfg: None,
        };

        let api = ApiSurface {
            crate_name: "my_crate".to_owned(),
            version: "0.1.0".to_owned(),
            services: vec![service],
            handler_contracts: vec![],
            ..ApiSurface::default()
        };

        let output = gen_service_php(&api, "my_crate");

        // Should contain direct method
        assert!(
            output.contains("public function get(string $path, callable $handler): self"),
            "expected direct method form for VerbDecorator:\n{output}"
        );

        // Should NOT contain factory method
        assert!(
            !output.contains("public function getDecorator("),
            "VerbDecorator should not emit factory method:\n{output}"
        );
    }

    /// `gen_service_php` emits only decorator-factory form for Builder style.
    #[test]
    fn php_output_builder_style_factory_only() {
        use crate::core::ir::{RegistrationVariant, RegistrationVariantOverride};

        let constructor = MethodDef {
            name: "new".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: String::new(),
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
            method: "route".to_owned(),
            callback_param: "handler".to_owned(),
            callback_contract: "RequestHandler".to_owned(),
            metadata_params: vec![
                ParamDef {
                    name: "method".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
            ],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: None,
            doc: String::new(),
            variants: vec![RegistrationVariant {
                name: "GET".to_owned(),
                overrides: vec![RegistrationVariantOverride {
                    param_name: "method".to_owned(),
                    value_expr: "\"GET\"".to_owned(),
                }],
                wrapper_call: None,
                signature_params: vec![ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                }],
                doc: Some("Register a GET route.".to_owned()),
                style: RegistrationVariantStyle::Builder,
            }],
        };

        let service = ServiceDef {
            name: "Router".to_owned(),
            rust_path: "my_crate::Router".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![],
            doc: String::new(),
            cfg: None,
        };

        let api = ApiSurface {
            crate_name: "my_crate".to_owned(),
            version: "0.1.0".to_owned(),
            services: vec![service],
            handler_contracts: vec![],
            ..ApiSurface::default()
        };

        let output = gen_service_php(&api, "my_crate");

        // Should contain factory method only
        assert!(
            output.contains("public function getDecorator(string $path): Closure"),
            "expected factory method for Builder style:\n{output}"
        );

        // Should NOT contain direct method
        assert!(
            !output.contains("public function get(string $path, callable $handler): self"),
            "Builder style should not emit direct method:\n{output}"
        );
    }

    /// `gen_service_php` emits both forms for Hybrid style.
    #[test]
    fn php_output_hybrid_style_both_forms() {
        use crate::core::ir::{RegistrationVariant, RegistrationVariantOverride};

        let constructor = MethodDef {
            name: "new".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: String::new(),
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
            method: "route".to_owned(),
            callback_param: "handler".to_owned(),
            callback_contract: "RequestHandler".to_owned(),
            metadata_params: vec![
                ParamDef {
                    name: "method".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
            ],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: None,
            doc: String::new(),
            variants: vec![RegistrationVariant {
                name: "GET".to_owned(),
                overrides: vec![RegistrationVariantOverride {
                    param_name: "method".to_owned(),
                    value_expr: "\"GET\"".to_owned(),
                }],
                wrapper_call: None,
                signature_params: vec![ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                }],
                doc: Some("Register a GET route.".to_owned()),
                style: RegistrationVariantStyle::Hybrid,
            }],
        };

        let service = ServiceDef {
            name: "Router".to_owned(),
            rust_path: "my_crate::Router".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![],
            doc: String::new(),
            cfg: None,
        };

        let api = ApiSurface {
            crate_name: "my_crate".to_owned(),
            version: "0.1.0".to_owned(),
            services: vec![service],
            handler_contracts: vec![],
            ..ApiSurface::default()
        };

        let output = gen_service_php(&api, "my_crate");

        // Should contain both direct method and factory method
        assert!(
            output.contains("public function get(string $path, callable $handler): self"),
            "expected direct method form for Hybrid:\n{output}"
        );

        assert!(
            output.contains("public function getDecorator(string $path): Closure"),
            "expected factory method for Hybrid style:\n{output}"
        );
    }

    /// `gen_service_php` emits the `run` entrypoint.
    #[test]
    fn php_output_contains_run_entrypoint() {
        let surface = make_fixture_surface();
        let output = gen_service_php(&surface, "my_crate");
        assert!(
            output.contains("public function run("),
            "expected `public function run(` entrypoint:\n{output}"
        );
        assert!(
            output.contains("test_service_run("),
            "expected native call `test_service_run(` in run:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge struct.
    #[test]
    fn rust_output_contains_handler_bridge_struct() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("pub struct PhpRequestHandlerBridge"),
            "expected `PhpRequestHandlerBridge` struct:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge trait impl.
    #[test]
    fn rust_output_contains_handler_bridge_impl() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("impl my_crate::RequestHandler for PhpRequestHandlerBridge"),
            "expected trait impl:\n{output}"
        );
        assert!(
            output.contains("fn handle(") && output.contains("Pin<Box<dyn std::future::Future<Output"),
            "expected boxed-future dispatch method:\n{output}"
        );
    }

    /// `gen_service_rs` emits the `#[php_function]` run entry point.
    #[test]
    fn rust_output_contains_php_function_run() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("#[php_function]"),
            "expected `#[php_function]` attribute:\n{output}"
        );
        assert!(
            output.contains("pub fn test_service_run("),
            "expected `test_service_run` function:\n{output}"
        );
    }

    /// `gen_service_rs` emits registration dispatch via `match method_name`.
    #[test]
    fn rust_output_contains_registration_dispatch() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("\"add_handler\""),
            "expected `\"add_handler\"` match arm:\n{output}"
        );
        assert!(
            output.contains("Arc<dyn my_crate::RequestHandler>"),
            "expected Arc wrapping of handler:\n{output}"
        );
    }

    /// Full `generate()` call returns two files when services are non-empty.
    #[test]
    fn generate_returns_two_files_for_non_empty_services() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert_eq!(files.len(), 2, "expected 2 generated files, got {}", files.len());
        let paths: Vec<&str> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(paths.contains(&"service.rs"), "expected service.rs in output");
        assert!(paths.contains(&"Service.php"), "expected Service.php in output");
    }

    /// Full `generate()` returns empty for a surface with no services.
    #[test]
    fn generate_returns_empty_for_no_services() {
        let surface = ApiSurface::default();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    /// Verify that required &str/String parameters emit non-nullable PHP signatures,
    /// while Option<T> parameters emit nullable signatures with = null defaults.
    /// This is a regression test for the over-propagation of nullable params.
    #[test]
    fn php_output_required_params_not_nullable() {
        let constructor = MethodDef {
            name: "new".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: String::new(),
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

        let service = ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![],
            entrypoints: vec![EntrypointDef {
                method: "extract".to_owned(),
                kind: EntrypointKind::Run,
                is_async: false,
                params: vec![
                    ParamDef {
                        name: "path".to_owned(),
                        ty: TypeRef::String, // required &str/String
                        optional: false,
                        default: None,
                        ..ParamDef::default()
                    },
                    ParamDef {
                        name: "mime_type".to_owned(),
                        ty: TypeRef::Optional(Box::new(TypeRef::String)), // Option<&str>
                        optional: true,
                        default: None,
                        ..ParamDef::default()
                    },
                ],
                return_type: TypeRef::Unit,
                error_type: None,
                doc: String::new(),
            }],
            doc: String::new(),
            cfg: None,
        };

        let api = ApiSurface {
            crate_name: "my_crate".to_owned(),
            version: "0.1.0".to_owned(),
            services: vec![service],
            handler_contracts: vec![],
            ..ApiSurface::default()
        };

        let output = gen_service_php(&api, "my_crate");

        // Required string param must NOT be nullable (no ? prefix, no = null)
        assert!(
            output.contains("string $path,"),
            "required path param must be non-nullable: {output}"
        );

        // Option<T> param must be nullable with = null default
        assert!(
            output.contains("?string $mime_type = null"),
            "Option<T> mime_type param must be nullable with = null: {output}"
        );

        // Ensure the bad pattern does NOT appear
        assert!(
            !output.contains("?string $path"),
            "required path must not be nullable: {output}"
        );
    }

    /// `gen_registration_variant` with a `wrapper_call` emits wrapper construction
    /// and delegates to the base method instead of pushing to `$this->registrations[]`.
    #[test]
    fn php_output_wrapper_call_delegates_to_base_method() {
        use crate::core::ir::{
            ParamDef, RegistrationVariant, RegistrationVariantStyle, TypeRef, WrapperConstructorArg,
            WrapperConstructorCall,
        };

        let constructor = MethodDef {
            name: "new".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: String::new(),
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
            method: "route".to_owned(),
            callback_param: "handler".to_owned(),
            callback_contract: "RequestHandler".to_owned(),
            metadata_params: vec![ParamDef {
                name: "builder".to_owned(),
                ty: TypeRef::Named("RouteBuilder".to_owned()),
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: None,
            doc: String::new(),
            variants: vec![RegistrationVariant {
                name: "GET".to_owned(),
                overrides: vec![],
                wrapper_call: Some(WrapperConstructorCall {
                    metadata_param: "builder".to_owned(),
                    wrapper_type_path: "my_crate::RouteBuilder".to_owned(),
                    wrapper_type_name: "RouteBuilder".to_owned(),
                    constructor_method: "new".to_owned(),
                    args: vec![
                        WrapperConstructorArg::Fixed {
                            param_name: "method".to_owned(),
                            value_expr: "my_crate::Method::Get".to_owned(),
                        },
                        WrapperConstructorArg::Free {
                            param: ParamDef {
                                name: "path".to_owned(),
                                ty: TypeRef::String,
                                optional: false,
                                default: None,
                                ..ParamDef::default()
                            },
                        },
                    ],
                }),
                signature_params: vec![ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                }],
                doc: Some("Register a GET route.".to_owned()),
                style: RegistrationVariantStyle::Hybrid,
            }],
        };

        let service = ServiceDef {
            name: "Router".to_owned(),
            rust_path: "my_crate::Router".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![],
            doc: String::new(),
            cfg: None,
        };

        let api = ApiSurface {
            crate_name: "my_crate".to_owned(),
            version: "0.1.0".to_owned(),
            services: vec![service],
            handler_contracts: vec![],
            ..ApiSurface::default()
        };

        let output = gen_service_php(&api, "my_crate");

        // Wrapper construction statement must appear
        assert!(
            output.contains("$builder = RouteBuilder::new(Method::Get, $path);"),
            "expected wrapper construction statement:\n{output}"
        );

        // Delegation to base method must appear
        assert!(
            output.contains("return $this->route($builder, $handler);"),
            "expected delegation to base route() method:\n{output}"
        );

        // Must NOT push directly to registrations[] (that would be the old broken path)
        assert!(
            !output.contains("$this->registrations[] = ['route', [], $handler]"),
            "must not push empty metadata to registrations[]:\n{output}"
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
