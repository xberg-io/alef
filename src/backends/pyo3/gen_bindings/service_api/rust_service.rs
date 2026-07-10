use super::helpers::find_contract;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, HandlerContractDef, ServiceDef, TypeRef};
use heck::{ToSnakeCase, ToUpperCamelCase};
use minijinja::context;

pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let mut out = String::new();

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_rs_header.rs.jinja",
        context! {},
    ));

    let referenced_contracts: Vec<&HandlerContractDef> = {
        let mut names: Vec<&str> = api
            .services
            .iter()
            .flat_map(|s| s.registrations.iter())
            .map(|r| r.callback_contract.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
            .iter()
            .filter_map(|n| find_contract(api, n))
            .filter(|c| c.trait_name != "WebSocketHandler" && c.trait_name != "SseEventProducer")
            .collect()
    };

    for contract in &referenced_contracts {
        gen_handler_bridge(&mut out, contract, &core_import);
    }

    for service in &api.services {
        for ep in &service.entrypoints {
            gen_run_pyfunction(&mut out, service, ep, api, &core_import);
        }
    }

    out
}

/// Emit the `Py{ContractName}Bridge` struct + trait impl.
///
/// Pattern mirrors the proven hand-written handler.rs: detect whether the
/// Python callable is a coroutine function; if so await it via
/// pyo3_async_runtimes; otherwise call it synchronously inside
/// `spawn_blocking` to avoid blocking the async executor.
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Py{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;

    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

    let req_type = if req_type.contains("::") {
        req_type.split("::").last().unwrap_or(req_type)
    } else {
        req_type
    };
    let resp_type = if resp_type.contains("::") {
        resp_type.split("::").last().unwrap_or(resp_type)
    } else {
        resp_type
    };

    let extra_param: String = contract
        .dispatch_extra_params
        .iter()
        .map(|p| format!(", {p}"))
        .collect();
    let wire_name = contract.wire_param_name.as_deref().unwrap_or("request");

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_handler_bridge_struct.rs.jinja",
        context! { trait_name => trait_name, bridge_name => bridge_name.as_str() },
    ));

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

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_handler_bridge_impl.rs.jinja",
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
            box_err => box_err,
            resp_path => resp_path,
            tail => tail,
        },
    ));
}

/// Emit the `#[pyfunction]` entry point for one service × entrypoint.
///
/// The function:
/// 1. Accepts the registrations list (`list[tuple[str, tuple, Callable]]`).
/// 2. Constructs the native service owner via its constructor (zero-arg form
///    since constructor params were already captured at `__init__` time and
///    are not yet threaded through — a deliberate first-pass simplification).
/// 3. Iterates registrations, wraps each callable in the appropriate bridge,
///    and calls the owner's registration method.
/// 4. Calls the owner's entrypoint (blocking if `Run`, awaiting via Tokio if async).
fn gen_run_pyfunction(
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

    let mut rust_params = vec![
        "_py: Python<'_>".to_owned(),
        "registrations: &Bound<'_, PyList>".to_owned(),
    ];
    for p in &ep.params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        rust_params.push(format!("{}: {}", p.name, rust_ty));
    }
    let param_sig = rust_params.join(", ");

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_pyfunction_header.rs.jinja",
        context! {
            owner_path => owner_path,
            ep_method => ep_method,
            fn_name => fn_name,
            param_sig => param_sig,
        },
    ));

    let ctor_call = build_ctor_call(service, owner_path, core_import);
    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_rs_owner_ctor.rs.jinja",
        context! { ctor_call => ctor_call },
    ));
    out.push('\n');

    out.push_str("    for entry in registrations.iter() {\n");
    out.push_str("        let tuple: &Bound<'_, PyTuple> = entry.cast()?;\n");
    out.push_str("        let method_name: String = tuple.get_item(0)?.extract()?;\n");
    out.push_str("        let callable = tuple.get_item(2)?;\n\n");

    out.push_str("        match method_name.as_str() {\n");
    for reg in &service.registrations {
        let reg_method = &reg.method;
        let contract_name = &reg.callback_contract;

        if let Some(contract) = find_contract(api, contract_name) {
            let bridge_name = format!("Py{}Bridge", contract.trait_name.to_upper_camel_case());
            let meta_count = reg.metadata_params.len();

            out.push_str(&crate::backends::pyo3::template_env::render(
                "service_api_registration_arm.rs.jinja",
                context! {
                    reg_method => reg_method,
                    bridge_name => bridge_name,
                    core_import => core_import,
                    contract_name => contract_name,
                },
            ));

            if meta_count > 0 {
                out.push_str("                let meta_item = tuple.get_item(1)?;\n");
                out.push_str("                let meta: &Bound<'_, PyTuple> = meta_item.cast()?;\n");
                for (i, meta_param) in reg.metadata_params.iter().enumerate() {
                    // `#[pyclass]` wrapping `inner: Arc<core>`. pyo3 can only extract the BINDING
                    let opaque_named = match &meta_param.ty {
                        TypeRef::Named(n) => api
                            .types
                            .iter()
                            .find(|t| &t.name == n && !t.is_trait && t.is_opaque)
                            .map(|_| n.clone()),
                        _ => None,
                    };
                    if let Some(name) = opaque_named {
                        out.push_str(&crate::backends::pyo3::template_env::render(
                            "service_api_registration_meta_opaque.rs.jinja",
                            context! {
                                param_name => meta_param.name.as_str(),
                                type_name => name,
                                core_import => core_import,
                                index => i,
                            },
                        ));
                    } else {
                        let rust_ty = typeref_to_rust_type(&meta_param.ty, core_import);
                        out.push_str(&crate::backends::pyo3::template_env::render(
                            "service_api_registration_meta_value.rs.jinja",
                            context! {
                                param_name => meta_param.name.as_str(),
                                rust_type => rust_ty,
                                index => i,
                            },
                        ));
                    }
                }
                let meta_args: Vec<String> = reg.metadata_params.iter().map(|p| p.name.clone()).collect();
                let args = if meta_args.is_empty() {
                    String::new()
                } else {
                    format!("{}, ", meta_args.join(", "))
                };
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "service_api_registration_owner_call.rs.jinja",
                    context! {
                        reg_method => reg_method,
                        args => args,
                    },
                ));
            } else {
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "service_api_registration_owner_call.rs.jinja",
                    context! {
                        reg_method => reg_method,
                        args => "",
                    },
                ));
            }

            if reg.error_type.is_some() {
                out.push_str(
                    "                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;\n",
                );
            } else {
                out.push_str("                    ;\n");
            }
            out.push_str("            }\n");
        }
    }
    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_unknown_registration_arm.rs.jinja",
        context! {},
    ));
    out.push_str("        }\n");
    out.push_str("    }\n\n");

    let ep_call = build_ep_call(ep, service, core_import);
    out.push_str(&ep_call);

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_pyfunction_footer.rs.jinja",
        context! {},
    ));
}

/// Build the Rust constructor call for the service owner.
fn build_ctor_call(service: &ServiceDef, owner_path: &str, _core_import: &str) -> String {
    if service.constructor.params.is_empty() {
        format!("{owner_path}::{}()", service.constructor.name)
    } else {
        format!("{owner_path}::{}()", service.constructor.name)
    }
}

/// Build the entrypoint invocation for a service method.
fn build_ep_call(ep: &crate::core::ir::EntrypointDef, _service: &ServiceDef, _core_import: &str) -> String {
    let ep_method = &ep.method;
    let ep_args: Vec<String> = ep.params.iter().map(|p| p.name.clone()).collect();
    let args_str = ep_args.join(", ");
    let bind = if matches!(ep.return_type, TypeRef::Unit) {
        ""
    } else {
        "let _ = "
    };

    if ep.is_async {
        format!(
            "    {bind}_py.detach(|| {{\n        \
             pyo3_async_runtimes::tokio::get_runtime().block_on(owner.{ep_method}({args_str}))\n    \
             }})\n        \
             .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;\n"
        )
    } else if ep.error_type.is_some() {
        format!(
            "    {bind}_py.detach(|| owner.{ep_method}({args_str}))\n        \
             .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;\n"
        )
    } else {
        format!("    {bind}_py.detach(|| owner.{ep_method}({args_str}));\n")
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
