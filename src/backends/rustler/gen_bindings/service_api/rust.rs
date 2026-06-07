//! Rust NIF and handler bridge generation for Rustler service APIs.

use crate::backends::rustler::gen_bindings::service_api::helpers::{find_contract, typeref_to_rust_type};
use crate::backends::rustler::gen_bindings::service_api::registration_nif::gen_registration_variant_nif;
use crate::backends::rustler::template_env::render;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, HandlerContractDef, ServiceDef, TypeRef};
use heck::{ToSnakeCase, ToUpperCamelCase};
use minijinja::context;

/// Generate the Rust rustler glue module (`service.rs`).
///
/// For each service this emits:
/// - A message-passing handler bridge struct that wraps a `LocalPid` and sends
///   `{:trait_call, ...}` messages to the Elixir GenServer, awaiting responses
///   via a `complete_trait_call` NIF.
/// - A `#[rustler::nif(schedule = "DirtyCpu")]` NIF function that accepts
///   registrations (as Elixir terms), builds the service, and drives entrypoints.
pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let mut out = String::new();

    out.push_str(&render(
        "service_api_rs_header.rs.jinja",
        context! {
            core_import => core_import,
        },
    ));

    // Global registry of pending oneshot senders, keyed by reply_id. The handler
    // bridge inserts a sender when it sends a `{:trait_call, ...}` message to the
    // Elixir GenServer; the GenServer (once it has processed the call) invokes
    // the `complete_trait_call` NIF with the reply_id and the JSON response,
    // which removes the sender from the map and forwards the response through
    // the oneshot channel.
    out.push_str(&render("service_api_trait_reply_support.rs.jinja", context! {}));

    // Emit one handler bridge per unique handler contract referenced
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

    // Emit one NIF per service × entrypoint
    for service in &api.services {
        for ep in &service.entrypoints {
            gen_run_nif(&mut out, service, ep, api, &core_import);
        }

        // Emit registration variant NIFs
        for reg in &service.registrations {
            for variant in &reg.variants {
                gen_registration_variant_nif(&mut out, service, reg, variant, api, &core_import);
            }
        }
    }

    out
}

/// Emit the message-passing handler bridge struct + trait impl.
///
/// The Elixir GenServer pattern:
/// 1. Rust bridge holds a `LocalPid` (safe to send across threads via Rustler's guarantees).
/// 2. When dispatch is called, bridge serializes request to JSON and sends:
///    `{:trait_call, method_name, args_json, reply_id}` to the pid.
/// 3. Bridge awaits response via a oneshot channel, keyed by reply_id.
/// 4. Elixir GenServer receives, calls the registered handler, and sends back a response.
/// 5. Bridge receives and deserializes response to the wire response type.
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Elixir{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;
    let _unused = bridge_name.clone(); // silence warnings, used in format!() strings

    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

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

    // Build request/response type paths
    let req_path = if req_type == "Value" {
        "serde_json::Value".to_string()
    } else {
        format!("{core_import}::{req_type}")
    };
    let resp_path = if resp_type == "Value" {
        "serde_json::Value".to_string()
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

    out.push_str(&render(
        "service_api_handler_bridge.rs.jinja",
        context! {
            trait_name => trait_name,
            bridge_name => bridge_name,
            core_import => core_import,
            dispatch_name => dispatch_name,
            extra_param => extra_param,
            wire_name => wire_name,
            req_path => req_path,
            output_type => output_type,
            wire_output => wire_output,
            box_err => box_err,
            resp_path => resp_path,
            tail => tail,
        },
    ));
}

/// Emit the `#[rustler::nif]` entry point for one service × entrypoint.
///
/// The function:
/// 1. Accepts the registrations list and any entrypoint params.
/// 2. Constructs the native service owner via its constructor.
/// 3. Iterates registrations and wraps each in the appropriate bridge.
/// 4. Calls the owner's registration methods.
/// 5. Calls the owner's entrypoint (blocking if `Run`, async if async).
fn gen_run_nif(
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

    // Build the function signature with lifetime-annotated Term
    let mut params = vec!["registrations: rustler::Term<'_>".to_owned()];
    for p in &ep.params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        params.push(format!("{}: {}", p.name, rust_ty));
    }
    let param_sig = params.join(", ");

    let ep_param_names = ep.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>();
    out.push_str(&render(
        "service_api_run_nif_header.rs.jinja",
        context! {
            owner_path => owner_path,
            ep_method => ep_method,
            ep_params => ep_param_names,
            fn_name => fn_name,
            param_sig => param_sig,
        },
    ));

    // Generate dispatch for each registration
    for (i, reg) in service.registrations.iter().enumerate() {
        let contract_name = &reg.callback_contract;
        let reg_method = &reg.method;
        let metadata_param_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
        let bridge_wrapper = format!("Elixir{contract_name}Bridge");
        let prefix = if i == 0 { "            " } else { "            } else " };

        // Decode metadata if present
        let (has_metadata, trailing, tuple_types, opaque_bindings, args_list) = if !metadata_param_names.is_empty() {
            // The Elixir registration method always wraps metadata in a tuple `{...}`
            // (see gen_registration_method), so a single param `path` arrives as the
            // 1-element Elixir tuple `{path}`. A 1-element Elixir tuple decodes to a Rust
            // 1-tuple `(T,)`, so emit a trailing comma when there is exactly one param.
            let trailing = if metadata_param_names.len() == 1 { "," } else { "" };
            let tuple_types = reg
                .metadata_params
                .iter()
                .map(|p| {
                    // Opaque types are passed as ResourceArc<super::T> where super::T is the
                    // local lib-module wrapper (implements rustler::Resource). The wildcard
                    // import in service.rs would shadow a bare `T` name, so qualify with `super::`.
                    if let TypeRef::Named(n) = &p.ty {
                        if api.types.iter().any(|t| &t.name == n && !t.is_trait && t.is_opaque) {
                            return format!("rustler::ResourceArc<super::{}>", n);
                        }
                    }
                    typeref_to_rust_type(&p.ty, core_import)
                })
                .collect::<Vec<_>>()
                .join(", ");
            let tuple_types_with_trailing = format!("{}{}", tuple_types, trailing);
            // Decode and bind opaque metadata params to locals for later use.
            // ResourceArc<super::T> derefs to super::T (the local wrapper); wrapper.inner is
            // Arc<CoreType>. Call as_ref() on the Arc to get &CoreType, then clone to own it.
            let mut opaque_bindings = String::new();
            for meta_param in reg.metadata_params.iter() {
                let is_opaque = if let TypeRef::Named(n) = &meta_param.ty {
                    api.types.iter().any(|t| &t.name == n && !t.is_trait && t.is_opaque)
                } else {
                    false
                };
                if is_opaque {
                    if let TypeRef::Named(n) = &meta_param.ty {
                        opaque_bindings.push_str(&render(
                            "service_api_opaque_metadata_binding.rs.jinja",
                            context! {
                                indent => "                    ",
                                param_name => meta_param.name,
                                core_import => core_import,
                                type_name => n,
                            },
                        ));
                    }
                }
            }
            let args_list = metadata_param_names
                .iter()
                .map(|name| format!("{}, ", name))
                .collect::<String>();
            (true, trailing, tuple_types_with_trailing, opaque_bindings, args_list)
        } else {
            (false, "", String::new(), String::new(), String::new())
        };

        out.push_str(&render(
            "service_api_registration_dispatch.rs.jinja",
            context! {
                prefix => prefix,
                reg_method => reg_method,
                has_metadata => has_metadata,
                metadata_names => metadata_param_names.join(", "),
                trailing => trailing,
                tuple_types => tuple_types,
                opaque_bindings => opaque_bindings,
                bridge_wrapper => bridge_wrapper,
                core_import => core_import,
                trait_name => reg.callback_contract,
                args_list => args_list,
            },
        ));
    }

    if !service.registrations.is_empty() {
        out.push_str("            }\n");
    }
    let ep_params = ep.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
    let entrypoint_call = render(
        "service_api_entrypoint_call.rs.jinja",
        context! {
            is_run => matches!(ep.kind, EntrypointKind::Run),
            ep_method => ep_method,
            ep_params => ep_params,
        },
    );

    out.push_str(&render(
        "service_api_run_nif_footer.rs.jinja",
        context! {
            entrypoint_call => entrypoint_call,
        },
    ));
}

// Registration-variant NIF emission lives in registration_nif.rs.
