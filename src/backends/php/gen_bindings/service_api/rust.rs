use super::helpers::{build_ctor_call, build_ep_call, render};
use super::type_mapping::{find_contract, typeref_to_rust_type};
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, HandlerContractDef, ServiceDef};
use heck::{ToSnakeCase, ToUpperCamelCase};
use minijinja::context;

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
pub(in crate::backends::php::gen_bindings) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let mut out = String::new();

    out.push_str("#![allow(clippy::too_many_arguments, clippy::unused_async)]\n\n");
    out.push_str("use ext_php_rs::prelude::*;\n");
    out.push_str("use ext_php_rs::types::{ZendCallable, Zval};\n");
    out.push_str("use std::panic::AssertUnwindSafe;\n");
    out.push_str("use std::sync::Arc;\n\n");

    out.push_str("thread_local! {\n");
    out.push_str("    static PHP_HANDLER_REGISTRY: std::cell::RefCell<Vec<ZendCallable<'static>>> =\n");
    out.push_str("        const { std::cell::RefCell::new(Vec::new()) };\n");
    out.push_str("}\n\n");

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

    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

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

    let extra_param: String = contract
        .dispatch_extra_params
        .iter()
        .map(|p| format!(", {p}"))
        .collect();
    let wire_name = contract.wire_param_name.as_deref().unwrap_or("request");

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

    let ctor_call = build_ctor_call(service, owner_path, core_import);
    out.push_str(&render(
        "php_service_rust_owner_init.jinja",
        context! { ctor_call => &ctor_call },
    ));

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

    let ep_call = build_ep_call(ep, service, core_import);
    out.push_str(&ep_call);

    out.push_str("    Ok(())\n}\n\n");
}
