//! Service-API codegen for the extendr (R) backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **`service.rs`** — Rust extendr glue that wraps each registered R
//!    closure as `Arc<dyn <HandlerContractDef::trait_name>>` via a sync
//!    callback bridge (R is single-threaded; calls are blocked on the
//!    current Tokio runtime). Builds the core service via the owner type's
//!    registration and run entrypoints, exposes an `#[extendr]` entry point.
//!
//! 2. **`service.R`** — An idiomatic R interface with S3 class, constructor,
//!    configurator methods, and registration helpers. The `run()` method
//!    delegates to the native extension.
//!
//! All names are derived entirely from the [`ApiSurface`] IR — no transport-
//! or domain-specific assumptions are made anywhere in this module.
//!
//! **Threading Model**: R is single-threaded. The generated Rust bridge runs
//! R closures synchronously by invoking them via extendr's Robj interface
//! on the current thread. This blocks the Tokio executor; callers must
//! `spawn_blocking()` if needed. The generated R entrypoint is not async and
//! uses `tokio::runtime::Builder::new_current_thread()` for blocking service
//! calls.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{
    ApiSurface, EntrypointKind, HandlerContractDef, ParamDef, RegistrationDef, RegistrationVariant, ServiceDef, TypeRef,
};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

fn render(template_name: &str, ctx: minijinja::Value) -> String {
    crate::backends::extendr::template_env::render(template_name, ctx)
}

/// Convert a `TypeRef` to a simple R type annotation string.
fn r_type_annotation(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "character".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "logical".to_owned(),
                _ => "numeric".to_owned(),
            }
        }
        TypeRef::Bytes => "raw".to_owned(),
        TypeRef::Optional(_) => "NULL or other".to_owned(),
        TypeRef::Vec(_) => "list".to_owned(),
        TypeRef::Map(_, _) => "list".to_owned(),
        TypeRef::Unit => "NULL".to_owned(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Json => "list".to_owned(),
        TypeRef::Path => "character".to_owned(),
        TypeRef::Duration => "numeric".to_owned(),
    }
}

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

// ────────────────────────────────────────────────────────── R output ──

/// Generate the idiomatic R service interface (`service.R`).
///
/// Produces an R source file containing one S3 class per service. Each class
/// exposes:
/// - A constructor mirroring [`ServiceDef::constructor`].
/// - Configurator methods from [`ServiceDef::configurators`].
/// - Registration helpers from [`ServiceDef::registrations`].
/// - A `run(...)` method derived from the first [`EntrypointKind::Run`]
///   entrypoint.
pub(super) fn gen_service_r(api: &ApiSurface, package_name: &str) -> String {
    let mut out = String::new();

    out.push_str("# Generated R interface for service API\n");
    out.push_str("# DO NOT EDIT: regenerate this file via alef\n\n");

    for service in &api.services {
        gen_service_class(&mut out, service, api, package_name);
    }

    out
}

fn gen_service_class(out: &mut String, service: &ServiceDef, api: &ApiSurface, _package_name: &str) {
    let class_name = &service.name;
    let constructor = &service.constructor;

    // Service constructor function
    {
        let param_names: Vec<&str> = constructor.params.iter().map(|p| p.name.as_str()).collect();
        let params_str = if param_names.is_empty() {
            String::new()
        } else {
            param_names.join(" = , ").to_owned()
        };
        let param_docs: Vec<String> = constructor
            .params
            .iter()
            .map(|param| format!("#' @param {} {} Parameter", param.name, r_type_annotation(&param.ty)))
            .collect();
        let state_entries: Vec<String> = constructor
            .params
            .iter()
            .map(|param| format!("    .{} = {},", param.name, param.name))
            .collect();

        out.push_str(&render(
            "service_r_constructor.jinja",
            minijinja::context! {
                class_name => class_name,
                description => if !service.doc.is_empty() {
                    service.doc.trim().to_owned()
                } else {
                    "Initialize a service instance.".to_owned()
                },
                param_docs => param_docs,
                params_str => params_str,
                state_entries => state_entries,
            },
        ));
    }

    // Configurator methods
    for method in &service.configurators {
        let method_name = &method.name;
        let params_suffix = method
            .params
            .iter()
            .map(|param| format!(", {}", param.name))
            .collect::<String>();
        let param_docs: Vec<String> = method
            .params
            .iter()
            .map(|param| format!("#' @param {} {} Parameter", param.name, r_type_annotation(&param.ty)))
            .collect();
        let state_assignments: Vec<String> = method
            .params
            .iter()
            .map(|param| format!("  x$.{} <- {}", param.name, param.name))
            .collect();

        out.push_str(&render(
            "service_r_configurator.jinja",
            minijinja::context! {
                method_name => method_name,
                class_name => class_name,
                description => if !method.doc.is_empty() {
                    method.doc.trim().to_owned()
                } else {
                    "Apply a configuration.".to_owned()
                },
                param_docs => param_docs,
                params_suffix => params_suffix,
                state_assignments => state_assignments,
            },
        ));
    }

    // Registration methods
    for reg in &service.registrations {
        gen_registration_method(out, reg, service, api);
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        let ep_name = &ep.method;

        match ep.kind {
            EntrypointKind::Run => {
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                gen_service_entrypoint_r(
                    out,
                    class_name,
                    ep_name,
                    &if !ep.doc.is_empty() {
                        ep.doc.trim().to_owned()
                    } else {
                        "Execute the service.".to_owned()
                    },
                    &ep.params,
                    "Run the service",
                    "Invisibly NULL",
                    &native_fn,
                    true,
                );
            }
            EntrypointKind::Finalize => {
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                gen_service_entrypoint_r(
                    out,
                    class_name,
                    ep_name,
                    &if !ep.doc.is_empty() {
                        ep.doc.trim().to_owned()
                    } else {
                        "Finalize and return result.".to_owned()
                    },
                    &ep.params,
                    "Finalize the service",
                    "Result from finalization",
                    &native_fn,
                    false,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn gen_service_entrypoint_r(
    out: &mut String,
    class_name: &str,
    ep_name: &str,
    description: &str,
    params: &[ParamDef],
    title: &str,
    return_doc: &str,
    native_fn: &str,
    invisible_null: bool,
) {
    let params_suffix = params
        .iter()
        .map(|param| format!(", {}", param.name))
        .collect::<String>();
    let param_docs: Vec<String> = params
        .iter()
        .map(|param| format!("#' @param {} {} Parameter", param.name, r_type_annotation(&param.ty)))
        .collect();
    let native_args_suffix = params
        .iter()
        .map(|param| format!(", {}", param.name))
        .collect::<String>();

    out.push_str(&render(
        "service_r_entrypoint.jinja",
        minijinja::context! {
            title => title,
            description => description,
            class_name => class_name,
            param_docs => param_docs,
            return_doc => return_doc,
            ep_name => ep_name,
            params_suffix => params_suffix,
            native_fn => native_fn,
            native_args_suffix => native_args_suffix,
            invisible_null => invisible_null,
        },
    ));
}

fn gen_registration_method(out: &mut String, reg: &RegistrationDef, service: &ServiceDef, _api: &ApiSurface) {
    let method_name = &reg.method;
    let class_name = &service.name;
    let callback_param = &reg.callback_param;

    let meta_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
    let metadata_expr = if meta_names.is_empty() {
        "list()".to_owned()
    } else {
        format!("list({})", meta_names.join(", "))
    };
    let metadata_params_suffix = reg
        .metadata_params
        .iter()
        .map(|param| format!(", {}", param.name))
        .collect::<String>();
    let metadata_docs: Vec<String> = reg
        .metadata_params
        .iter()
        .map(|param| {
            format!(
                "#' @param {} {} Metadata parameter",
                param.name,
                r_type_annotation(&param.ty)
            )
        })
        .collect();

    out.push_str(&render(
        "service_r_registration.jinja",
        minijinja::context! {
            method_name => method_name,
            class_name => class_name,
            callback_param => callback_param,
            description => if !reg.doc.is_empty() {
                reg.doc.trim().to_owned()
            } else {
                "Register a callback handler.".to_owned()
            },
            metadata_docs => metadata_docs,
            metadata_params_suffix => metadata_params_suffix,
            metadata_expr => metadata_expr,
        },
    ));

    // Emit registration variants (shortcuts for common patterns)
    for variant in &reg.variants {
        gen_registration_variant(out, variant, reg, service, class_name, callback_param);
    }
}

/// Emit a registration variant (shortcut method) for the given variant definition.
fn gen_registration_variant(
    out: &mut String,
    variant: &RegistrationVariant,
    reg: &RegistrationDef,
    _service: &ServiceDef,
    class_name: &str,
    callback_param: &str,
) {
    use crate::core::ir::WrapperConstructorArg;

    let variant_name = &variant.name;
    let base_method = &reg.method;

    // Build wrapper constructor call expression if needed
    let wrapper_expr = if let Some(wc) = &variant.wrapper_call {
        let mut call_args = vec![];
        // Process constructor args in order: Fixed args substitute values, Free args pull from variant signature
        for arg in &wc.args {
            match arg {
                WrapperConstructorArg::Fixed { value_expr, .. } => {
                    call_args.push(value_expr.clone());
                }
                WrapperConstructorArg::Free { param } => {
                    call_args.push(param.name.clone());
                }
            }
        }
        Some(format!(
            "{}::{}({})",
            wc.wrapper_type_path,
            wc.constructor_method,
            call_args.join(", ")
        ))
    } else {
        None
    };

    // Render the variant R method using the template
    let rendered = crate::backends::extendr::template_env::render(
        "registration_variant.rs.jinja",
        minijinja::context! {
            variant_name => variant_name,
            class_name => class_name,
            callback_param => callback_param,
            base_method => base_method,
            doc => variant.doc.as_deref().unwrap_or(""),
            signature_params => variant.signature_params.iter().map(|p| minijinja::context! {
                name => p.name.as_str(),
                ty_annotation => r_type_annotation(&p.ty),
            }).collect::<Vec<_>>(),
            overrides => variant.overrides.iter().map(|o| minijinja::context! {
                param_name => o.param_name.as_str(),
                value_expr => o.value_expr.as_str(),
            }).collect::<Vec<_>>(),
            wrapper_expr => wrapper_expr.as_deref().unwrap_or(""),
        },
    );
    out.push_str(&rendered);
}

// ──────────────────────────────────────────────────────────────── Rust glue ──

/// Generate the Rust extendr glue module (`service.rs`).
///
/// For each service this emits:
/// - An `Extendr{ContractName}Bridge` struct that wraps an R closure (extendr::Robj)
///   and `impl`s the handler contract trait. Calls are made synchronously within
///   the current thread (R is single-threaded). If a tokio runtime is active,
///   calls block on it; otherwise, a local runtime is created.
/// - An `#[extendr]` function `{snake_service}_{entrypoint}` that accepts the
///   collected registrations list and any entrypoint params, builds the native
///   service, and drives it.
pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let mut out = String::new();

    // File-level allow attributes
    out.push_str("#![allow(clippy::too_many_arguments)]\n\n");
    out.push_str("use extendr_api::prelude::*;\n");
    out.push_str("use std::sync::Arc;\n");
    out.push_str("use std::future::Future;\n");
    out.push_str("use std::pin::Pin;\n\n");

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

    // Emit one extendr function per service × entrypoint
    for service in &api.services {
        for ep in &service.entrypoints {
            gen_run_extendr_function(&mut out, service, ep, api, &core_import);
        }
    }

    out
}

/// Emit the `Extendr{ContractName}Bridge` struct + trait impl.
///
/// The bridge wraps an R closure and implements the handler contract synchronously.
/// Since R is single-threaded, all calls are blocking and happen on the current thread.
/// The trait method still returns a pinned boxed future (the library's async requirement),
/// but the R call itself is synchronous within that future.
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Extendr{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;

    // Determine wire types
    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

    // Build full paths for request/response types; handle plain serde_json::Value specially
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
        "service_rs_handler_bridge.jinja",
        minijinja::context! {
            trait_name => trait_name,
            bridge_name => bridge_name,
            core_import => core_import,
            dispatch_name => dispatch_name,
            extra_param => extra_param,
            wire_name => wire_name,
            req_path => req_path,
            resp_path => resp_path,
            output_type => output_type,
            wire_output => wire_output,
            box_err => box_err,
            tail => tail,
        },
    ));
}

/// Emit the `#[extendr]` entry point for one service × entrypoint.
fn gen_run_extendr_function(
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

    let mut ep_param_decls = Vec::new();
    for p in &ep.params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        ep_param_decls.push(format!(", {}: {}", p.name, rust_ty));
    }

    // Build the owner instance via its constructor
    let ctor_call = build_ctor_call(service, owner_path, core_import);
    out.push_str(&render(
        "service_rs_run_function_header.jinja",
        minijinja::context! {
            owner_path => owner_path,
            ep_method => ep_method,
            fn_name => fn_name,
            ep_param_decls => ep_param_decls,
            ctor_call => ctor_call,
        },
    ));

    for reg in &service.registrations {
        let reg_method = &reg.method;
        let contract_name = &reg.callback_contract;

        if let Some(contract) = find_contract(api, contract_name) {
            let bridge_name = format!("Extendr{}Bridge", contract.trait_name.to_upper_camel_case());
            let metadata_bindings: Vec<String> = reg
                .metadata_params
                .iter()
                .enumerate()
                .map(|(i, meta_param)| {
                    let rust_ty = typeref_to_rust_type(&meta_param.ty, core_import);
                    format!(
                        "                                let {}: {} = meta_list.iter().nth({i}).ok_or(\"missing metadata\")?.clone().try_into()?;",
                        meta_param.name, rust_ty,
                    )
                })
                .collect();
            let meta_args: Vec<String> = reg.metadata_params.iter().map(|p| p.name.clone()).collect();
            let owner_call = if meta_args.is_empty() {
                format!("owner.{reg_method}(handler);")
            } else {
                format!("owner.{reg_method}({}, handler);", meta_args.join(", "))
            };

            out.push_str(&render(
                "service_rs_registration_match_arm.jinja",
                minijinja::context! {
                    reg_method => reg_method,
                    bridge_name => bridge_name,
                    core_import => core_import,
                    contract_name => contract_name,
                    has_metadata => !reg.metadata_params.is_empty(),
                    metadata_bindings => metadata_bindings,
                    owner_call => owner_call,
                },
            ));
        }
    }

    // Call the entrypoint
    let ep_call = build_ep_call(ep, service, core_import);
    out.push_str(&render(
        "service_rs_run_function_footer.jinja",
        minijinja::context! {
            ep_call => ep_call,
        },
    ));
}

/// Build the Rust constructor call for the service owner.
fn build_ctor_call(service: &ServiceDef, owner_path: &str, _core_import: &str) -> String {
    if service.constructor.params.is_empty() {
        format!("{owner_path}::{}()", service.constructor.name)
    } else {
        // For now, use the constructor with zero-value placeholders.
        // Full implementation would thread params through from R.
        format!("{owner_path}::{}()", service.constructor.name)
    }
}

/// Build the entrypoint invocation for a service method.
fn build_ep_call(ep: &crate::core::ir::EntrypointDef, _service: &ServiceDef, _core_import: &str) -> String {
    let ep_method = &ep.method;
    let ep_args: Vec<String> = ep.params.iter().map(|p| p.name.clone()).collect();
    let args_str = ep_args.join(", ");

    if ep.is_async {
        // Drive the async entrypoint on a blocking task
        format!(
            "    // Run async entrypoint in blocking context\n    \
             let rt = tokio::runtime::Runtime::new().map_err(|e| Error::other(e.to_string()))?;\n    \
             rt.block_on(async {{\n        \
                 owner.{ep_method}({args_str})\n            \
                     .map_err(|e| Error::other(e.to_string()))?;\n    \
             }});\n"
        )
    } else {
        if ep.error_type.is_some() {
            format!(
                "    owner.{ep_method}({args_str})\n        \
                 .map_err(|e| Error::other(e.to_string()))?;\n"
            )
        } else {
            format!("    owner.{ep_method}({args_str});\n")
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

/// Generate all service-API files for the extendr backend.
///
/// Returns up to two `GeneratedFile`s per non-empty service list:
/// - `{output_dir}/service.rs`   — Rust extendr glue
/// - `{r_pkg}/R/service.R`        — idiomatic R interface
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(config.output_paths.get("r"), &config.name, "crates/{name}-extendr/src/");
    let package_name = config.name.replace('-', "_");

    // Rust glue
    let service_rs = gen_service_rs(api, config);

    // R interface
    let service_r = gen_service_r(api, &package_name);

    // R package output base
    let output_base = PathBuf::from(format!("packages/r/{}", package_name));

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: output_base.join("R/service.R"),
            content: service_r,
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
            params: vec![],
            return_type: TypeRef::Unit,
            error_type: Some("ServiceError".to_owned()),
            doc: "Run the service.".to_owned(),
        };

        let service = ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![configurator],
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

    #[test]
    fn r_output_contains_constructor() {
        let surface = make_fixture_surface();
        let output = gen_service_r(&surface, "my_crate");
        assert!(
            output.contains("TestService <- function"),
            "expected `TestService <- function` in output:\n{output}"
        );
    }

    #[test]
    fn r_output_contains_configurator() {
        let surface = make_fixture_surface();
        let output = gen_service_r(&surface, "my_crate");
        assert!(
            output.contains("with_timeout.TestService <- function"),
            "expected `with_timeout.TestService` method:\n{output}"
        );
    }

    #[test]
    fn r_output_contains_registration() {
        let surface = make_fixture_surface();
        let output = gen_service_r(&surface, "my_crate");
        assert!(
            output.contains("add_handler.TestService <- function"),
            "expected `add_handler.TestService` method:\n{output}"
        );
    }

    #[test]
    fn r_output_contains_run_entrypoint() {
        let surface = make_fixture_surface();
        let output = gen_service_r(&surface, "my_crate");
        assert!(
            output.contains("run.TestService <- function"),
            "expected `run.TestService` entrypoint:\n{output}"
        );
        assert!(
            output.contains(".Call(`test_service_run`"),
            "expected native call to test_service_run:\n{output}"
        );
    }

    #[test]
    fn rust_output_contains_handler_bridge() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("pub struct ExtendrRequestHandlerBridge"),
            "expected `ExtendrRequestHandlerBridge` struct:\n{output}"
        );
    }

    #[test]
    fn rust_output_contains_bridge_impl() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("impl my_crate::RequestHandler for ExtendrRequestHandlerBridge"),
            "expected trait impl:\n{output}"
        );
        assert!(
            output.contains("fn handle(") && output.contains("Pin<Box<dyn") && output.contains("Future<Output"),
            "expected boxed-future dispatch method:\n{output}"
        );
    }

    #[test]
    fn rust_output_contains_extendr_function() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("#[extendr]"),
            "expected `#[extendr]` attribute:\n{output}"
        );
        assert!(
            output.contains("pub fn test_service_run("),
            "expected `test_service_run` function:\n{output}"
        );
    }

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
        assert!(paths.contains(&"service.R"), "expected service.R in output");
    }

    #[test]
    fn generate_returns_empty_for_no_services() {
        let surface = ApiSurface::default();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_test_config() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: "my-crate".to_owned(),
            ..ResolvedCrateConfig::default()
        }
    }
}
