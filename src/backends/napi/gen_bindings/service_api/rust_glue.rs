use heck::{ToSnakeCase, ToUpperCamelCase};
use minijinja::context;

use crate::backends::napi::template_env::render;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef};

use super::helpers::{find_contract, typeref_to_rust_type};

pub(in crate::backends::napi::gen_bindings) fn gen_service_rs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> String {
    let core_import = config.core_import_name();
    let mut out = String::new();

    out.push_str(&render("service_rs_preamble.jinja", context! {}));

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

    // Registration happens directly via variant methods on JsApp,
    // which are exposed as #[napi] methods in the JsApp impl block.
    // No need for separate registration functions — variants call
    // the base registration method on their inner Arc<Mutex<App>> directly.

    // Emit one napi function per service × entrypoint. Service entrypoints are
    // declared explicitly under `[[crates.services.entrypoints]]` and are the
    // intended public surface — they bypass `exclude.methods`, which is a
    // general per-method blacklist used to suppress the standard type-method
    // generator's placeholder for items that can't be auto-delegated (e.g.
    // consuming-self) but that the service wrapper *does* need to call via
    // the registration-replay free function pattern.
    for service in &api.services {
        for ep in &service.entrypoints {
            gen_run_napi_function(&mut out, service, ep, api, &core_import);
        }
    }

    // Emit per-verb registration shortcuts and entrypoint methods wrapped in an impl block per service.
    // These methods use `&self` with interior mutability (via the configured
    // `host_app_inner_accessor`) and live inside an `impl` block — emitting them as
    // top-level free functions produces invalid Rust.
    let prefix = config.node_type_prefix();
    for service in &api.services {
        let has_variants = service.registrations.iter().any(|r| !r.variants.is_empty());
        let has_entrypoints = !service.entrypoints.is_empty();

        if !has_variants && !has_entrypoints {
            continue;
        }

        let app_type_name = format!("{prefix}{}", service.name);
        let mut impl_methods = String::new();

        // Collect wrapper type names referenced as base-registration metadata
        // params so we can emit `use crate::JsXxx;` imports for them.
        let mut wrapper_imports: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for reg in &service.registrations {
            for p in &reg.metadata_params {
                let rust_ty = typeref_to_rust_type(&p.ty, &core_import);
                let bare_name = rust_ty.rsplit("::").next().unwrap_or(&rust_ty);
                wrapper_imports.insert(format!("{prefix}{bare_name}"));
            }
        }
        let wrapper_use_items = wrapper_imports.into_iter().collect::<Vec<_>>().join(", ");

        // Emit base registration methods (route, etc.) so TS can call them directly
        // with already-constructed wrapper objects (JsRouteBuilder, etc.)
        for reg in &service.registrations {
            gen_base_registration_napi_method(&mut impl_methods, service, reg, api, &core_import, config);
        }

        // Emit variant methods (per-verb registration shortcuts) so the TS
        // wrapper class can delegate per-variant calls to the napi class
        // without round-tripping callbacks through serde.
        for reg in &service.registrations {
            for variant in &reg.variants {
                gen_variant_napi_method(&mut impl_methods, service, reg, variant, api, &core_import, config);
            }
        }

        // Emit entrypoint methods (run, finalize) when configured with host_app_inner_accessor
        if has_entrypoints {
            let has_accessor = config
                .services
                .iter()
                .find(|sc| sc.owner_type == service.name)
                .and_then(|sc| sc.host_app_inner_accessor.as_deref())
                .is_some();

            if has_accessor {
                for ep in &service.entrypoints {
                    gen_entrypoint_napi_method(&mut impl_methods, service, ep, api, &core_import, config);
                }
            }
        }

        // Indent all method bodies by 4 spaces to sit inside the impl block
        let indented: String = impl_methods
            .lines()
            .map(|line| {
                if line.is_empty() {
                    String::new()
                } else {
                    format!("    {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        if !indented.is_empty() {
            let impl_methods = if indented.ends_with('\n') {
                indented
            } else {
                format!("{indented}\n")
            };
            out.push_str(&render(
                "service_rs_impl_block.jinja",
                context! {
                    app_type_name,
                    impl_methods,
                    wrapper_use_items => wrapper_use_items.clone(),
                    has_wrapper_imports => !wrapper_use_items.is_empty(),
                },
            ));
        }
    }

    out
}

/// Emit the `{ContractName}Bridge` struct + trait impl.
///
/// Pattern mirrors the proven hand-written handler.rs: wrap the ThreadsafeFunction
/// and call it with the request DTO, await the Promise, and extract the response DTO.
///
/// The ThreadsafeFunction uses `serde_json::Value` for both request and response,
/// which napi 3.x supports natively via its serde bridge.
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;

    // Determine wire types for conversion on the handler side
    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

    // The ThreadsafeFunction uses serde_json::Value for both input and output, which
    // napi 3.x supports natively. The JS side receives and returns JSON-serializable
    // values that are transparently converted by napi's serde bridge.
    out.push_str(&render(
        "service_rs_handler_bridge_header.jinja",
        context! {
            trait_name,
            bridge_name,
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

    // Compute request/response path types (mirror pyo3's req_path/resp_path construction)
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
    // Fully qualify `Result` as `std::result::Result` so the bare `Result`
    // resolved through `use napi::bindgen_prelude::*` (which re-exports
    // `napi::Result<T, S = Status>`) does not shadow it. Without the
    // qualification the wire_output annotation parses as
    // `napi::Result<Response, Box<dyn Error>>` =
    // `std::result::Result<Response, napi::Error<Box<dyn Error>>>`, which
    // fails the `S: AsRef<str>` bound on `napi::Error<S>`.
    let wire_output = format!("std::result::Result<{resp_path}, {box_err}>");
    let output_type = contract
        .dispatch_return_type
        .clone()
        .unwrap_or_else(|| wire_output.clone());
    let tail = match &contract.response_adapter {
        Some(adapter) => format!("{adapter}(outcome)"),
        None => "outcome".to_string(),
    };

    // Trait impl: call the ThreadsafeFunction and await the Promise. The
    // method returns a boxed future directly (matching the canonical
    // object-safe async-trait shape the contract declares) rather than via
    // the async_trait macro, so it satisfies traits whose dispatch method is
    // hand-written as `-> Pin<Box<dyn Future<..> + Send + '_>>`.
    //
    // We serialize the request to a serde_json::Value before calling, then
    // deserialize the serde_json::Value response. napi 3.x implements
    // ToNapiValue/FromNapiValue for serde_json::Value via its serde bridge.
    //
    // CRITICAL: Explicitly type the `resp_json` result to avoid type inference
    // from propagating the outer Box<dyn Error + Send + Sync> error type back into
    // the napi::Error generic type, which would fail the `S: AsRef<str>` bound.
    out.push_str(&render(
        "service_rs_handler_bridge_impl.jinja",
        context! {
            core_import,
            trait_name,
            bridge_name,
            dispatch_name,
            extra_param,
            wire_name,
            req_path,
            output_type,
            wire_output,
            box_err,
            tail,
        },
    ));
}

/// Emit the `#[napi]` entry point for one service × entrypoint.
///
/// The function:
/// 1. Accepts the registrations list (Vec of [method_name, metadata, callback] tuples).
/// 2. Constructs the native service owner via its constructor.
/// 3. Iterates registrations, wraps each callable in the appropriate bridge,
///    and calls the owner's registration method.
/// 4. Calls the owner's entrypoint (awaiting if async).
fn gen_run_napi_function(
    out: &mut String,
    service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    _api: &ApiSurface,
    core_import: &str,
) {
    let service_snake = service.name.to_snake_case();
    let fn_name = format!("{service_snake}_{}", ep.method);
    let owner_path = &service.rust_path;
    let ep_method = &ep.method;

    // Build the function signature
    let mut rust_params = vec![
        "registrations: Vec<(String, Vec<serde_json::Value>, ThreadsafeFunction<serde_json::Value, Either<Promise<HandlerReturn>, HandlerReturn>>)>".to_owned(),
    ];
    for p in &ep.params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        rust_params.push(format!("{}: {}", p.name, rust_ty));
    }
    let param_sig = rust_params.join(", ");

    // Return type
    let return_ty = match ep.kind {
        EntrypointKind::Run => "()".to_owned(),
        EntrypointKind::Finalize => {
            // For Finalize, we'd need to return a DTO or Object — for now use ()
            "()".to_owned()
        }
    };

    out.push_str(&render(
        "service_rs_run_function_header.jinja",
        context! {
            owner_path,
            ep_method,
            fn_name,
            param_sig,
            return_ty,
        },
    ));

    // Build the owner instance via its constructor
    let ctor_call = build_ctor_call_napi(service, owner_path);
    out.push_str(&render(
        "service_rs_owner_ctor.jinja",
        context! {
            ctor_call,
        },
    ));

    // Registrations are now handled directly via variant methods that call
    // nativeRegisterRoute, so we no longer need to dispatch them here.
    // The owner already has the registered routes from those direct calls.

    // Call the entrypoint
    let ep_call = build_ep_call_napi(ep, service, core_import);
    out.push_str(&ep_call);

    out.push_str("    Ok(())\n}\n\n");
}

/// Build the Rust constructor call for the service owner.
fn build_ctor_call_napi(service: &ServiceDef, owner_path: &str) -> String {
    if service.constructor.params.is_empty() {
        format!("{owner_path}::{}()", service.constructor.name)
    } else {
        // For a first-pass implementation where constructor params are not
        // yet threaded through, fall back to zero-arg constructor.
        format!("{owner_path}::{}()", service.constructor.name)
    }
}

/// Build the entrypoint invocation for a service method.
fn build_ep_call_napi(ep: &crate::core::ir::EntrypointDef, _service: &ServiceDef, _core_import: &str) -> String {
    let ep_method = &ep.method;
    let ep_args: Vec<String> = ep.params.iter().map(|p| p.name.clone()).collect();
    let args_str = ep_args.join(", ");
    // Bind non-Unit returns to `_` so the unwrapped value (after `?`-propagation) doesn't
    // trigger `unused_must_use` for `Result`-returning entrypoints like `into_router`.
    let bind = if matches!(ep.return_type, TypeRef::Unit) {
        ""
    } else {
        "let _ = "
    };

    if ep.is_async {
        // Drive the async entrypoint directly (this function is already async)
        format!(
            "    {bind}owner.{ep_method}({args_str})\n        \
             .await\n        \
             .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;\n"
        )
    } else if ep.error_type.is_some() {
        format!(
            "    {bind}owner.{ep_method}({args_str})\n        \
             .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;\n"
        )
    } else {
        format!("    {bind}owner.{ep_method}({args_str});\n")
    }
}

/// Emit one `#[napi]` method for a base registration on the App class.
///
/// The method accepts already-constructed wrapper objects (e.g. JsRouteBuilder)
/// from the TS side, extracts the inner Rust value, creates the handler bridge,
/// and calls the owner's registration method.
fn gen_base_registration_napi_method(
    out: &mut String,
    service: &ServiceDef,
    reg: &RegistrationDef,
    api: &ApiSurface,
    core_import: &str,
    config: &ResolvedCrateConfig,
) {
    let base_method = &reg.method;
    let contract_name = &reg.callback_contract;

    // Look up the optional inner-accessor expression
    let inner_accessor: String = config
        .services
        .iter()
        .find(|sc| sc.owner_type == service.name)
        .and_then(|sc| sc.host_app_inner_accessor.as_deref())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| "self".to_owned());

    // Build method signature from registration's metadata_params.
    //
    // napi-rs cannot synthesize `FromNapiValue` for bare core types, so the
    // public-facing param uses the wrapper class and is unwrapped into the
    // core type inside the function body before being forwarded to
    // `inner.method(..)`.
    let prefix = config.node_type_prefix();
    let mut rust_params = vec!["&self".to_owned()];
    let mut unwrap_lines = String::new();
    for p in &reg.metadata_params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        let bare_name = rust_ty.rsplit("::").next().unwrap_or(&rust_ty);
        let wrapper_ty = format!("{prefix}{bare_name}");
        rust_params.push(format!("{}: &{wrapper_ty}", p.name));
        // Wrappers are emitted with shape `struct {wrapper} { inner: Arc<T> }`,
        // so `(*name.inner).clone()` produces a fresh `T`.
        unwrap_lines.push_str(&format!("        let {0} = (*{0}.inner).clone();\n", p.name));
    }
    rust_params.push(
        "handler: ThreadsafeFunction<serde_json::Value, Either<Promise<HandlerReturn>, HandlerReturn>>".to_string(),
    );
    let param_sig = rust_params.join(", ");

    let doc = reg
        .doc
        .trim()
        .lines()
        .map(|line| {
            if line.is_empty() {
                "    ///".to_owned()
            } else {
                format!("    /// {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    out.push_str(&render(
        "service_rs_base_registration_method_header.jinja",
        context! {
            base_method,
            doc,
            param_sig,
        },
    ));

    // Unwrap wrapper params into core types before forwarding.
    out.push_str(&unwrap_lines);

    // Create the handler bridge
    if let Some(contract) = find_contract(api, contract_name) {
        let bridge_name = format!("{}Bridge", contract.trait_name.to_upper_camel_case());
        out.push_str(&render(
            "service_rs_handler_arc.jinja",
            context! {
                bridge_name,
                core_import,
                contract_name,
            },
        ));
    }

    // Call the base registration method via the inner accessor
    let meta_names: Vec<String> = reg.metadata_params.iter().map(|p| p.name.clone()).collect();
    let meta_args = meta_names.join(", ");

    if inner_accessor == "self" {
        if !meta_names.is_empty() {
            out.push_str(&render(
                "service_rs_base_registration_call.jinja",
                context! {
                    receiver => "self",
                    base_method,
                    meta_args,
                    has_meta => true,
                },
            ));
        } else {
            out.push_str(&render(
                "service_rs_base_registration_call.jinja",
                context! {
                    receiver => "self",
                    base_method,
                    meta_args => "",
                    has_meta => false,
                },
            ));
        }
    } else {
        out.push_str(&render(
            "service_rs_inner_accessor.jinja",
            context! {
                inner_accessor,
            },
        ));
        if !meta_names.is_empty() {
            out.push_str(&render(
                "service_rs_base_registration_call.jinja",
                context! {
                    receiver => "inner",
                    base_method,
                    meta_args,
                    has_meta => true,
                },
            ));
        } else {
            out.push_str(&render(
                "service_rs_base_registration_call.jinja",
                context! {
                    receiver => "inner",
                    base_method,
                    meta_args => "",
                    has_meta => false,
                },
            ));
        }
    }

    // Handle error if the registration is fallible
    if reg.error_type.is_some() {
        out.push_str("        .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;\n");
    } else {
        out.push_str("        ;\n");
    }

    out.push_str(&render("service_rs_unit_ok_footer.jinja", context! {}));
}

/// Emit one `#[napi]` async shortcut method for a registration variant on the App class.
///
/// The method signature mirrors the variant's `signature_params` + a handler callback,
/// builds the wrapper via the `WrapperConstructorCall` (if present), and delegates to
/// the base registration method on the inner host-app value.
fn gen_variant_napi_method(
    out: &mut String,
    service: &ServiceDef,
    reg: &RegistrationDef,
    variant: &crate::core::ir::RegistrationVariant,
    api: &ApiSurface,
    core_import: &str,
    config: &ResolvedCrateConfig,
) {
    let variant_name = &variant.name;
    let base_method = &reg.method;
    let contract_name = &reg.callback_contract;

    // Look up the optional inner-accessor expression for this service's config.
    // When present, verb methods call `{accessor}.{base_method}(...)` instead of
    // `self.{base_method}(...)`, allowing the wrapper type to dereference an inner
    // field (e.g. `Arc<Mutex<Owner>>`) before dispatching.
    let inner_accessor: String = config
        .services
        .iter()
        .find(|sc| sc.owner_type == service.name)
        .and_then(|sc| sc.host_app_inner_accessor.as_deref())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| "self".to_owned());

    // Build method signature from variant's signature_params.
    // napi-rs only accepts `&self` (or no receiver) on `#[napi]` methods — `&mut self`
    // is rejected by the `napi` macro. Mutation of the underlying owner is expected to
    // go through interior mutability via `host_app_inner_accessor` (e.g.
    // `self.inner.lock().expect(...)` when the wrapper holds an `Arc<Mutex<_>>`).
    let mut rust_params = vec!["&self".to_owned()];
    for p in &variant.signature_params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        rust_params.push(format!("{}: {}", p.name, rust_ty));
    }
    rust_params.push(
        "handler: ThreadsafeFunction<serde_json::Value, Either<Promise<HandlerReturn>, HandlerReturn>>".to_string(),
    );
    let param_sig = rust_params.join(", ");

    let doc = variant.doc.as_deref().unwrap_or("").trim();
    out.push_str(&render(
        "service_rs_variant_method_header.jinja",
        context! {
            variant_name,
            doc,
            param_sig,
        },
    ));

    // If there's a wrapper constructor call, build the wrapper first
    if let Some(wrapper_call) = &variant.wrapper_call {
        let wrapper_path = &wrapper_call.wrapper_type_path;
        let constructor = &wrapper_call.constructor_method;

        // Build the constructor args
        let mut ctor_args = Vec::new();
        for arg in &wrapper_call.args {
            match arg {
                crate::core::ir::WrapperConstructorArg::Fixed {
                    param_name: _,
                    value_expr,
                } => {
                    ctor_args.push(value_expr.clone());
                }
                crate::core::ir::WrapperConstructorArg::Free { param } => {
                    ctor_args.push(param.name.clone());
                }
            }
        }
        let ctor_arg_str = ctor_args.join(", ");

        let metadata_param = &wrapper_call.metadata_param;
        out.push_str(&render(
            "service_rs_wrapper_ctor.jinja",
            context! {
                metadata_param,
                wrapper_path,
                constructor,
                ctor_arg_str,
            },
        ));
    }

    // Build the metadata argument list for the base registration call.
    // When a wrapper_call is present, its metadata_param IS the single metadata
    // argument and the signature_params have already been consumed by the wrapper
    // constructor — do NOT include them again.
    // When no wrapper_call is present, metadata comes directly from signature_params.
    let mut metadata_names: Vec<String> = Vec::new();
    if let Some(wrapper_call) = &variant.wrapper_call {
        metadata_names.push(wrapper_call.metadata_param.clone());
    } else {
        for p in &variant.signature_params {
            metadata_names.push(p.name.clone());
        }
    }

    // Create the handler bridge and call base registration
    if let Some(contract) = find_contract(api, contract_name) {
        let bridge_name = format!("{}Bridge", contract.trait_name.to_upper_camel_case());
        out.push_str(&render(
            "service_rs_handler_arc.jinja",
            context! {
                bridge_name,
                core_import,
                contract_name,
            },
        ));
    }

    // Call the base registration method via the inner accessor or self
    let meta_args = metadata_names.join(", ");
    if inner_accessor == "self" {
        if !metadata_names.is_empty() {
            out.push_str(&render(
                "service_rs_base_registration_call.jinja",
                context! {
                    receiver => "self",
                    base_method,
                    meta_args,
                    has_meta => true,
                },
            ));
        } else {
            out.push_str(&render(
                "service_rs_base_registration_call.jinja",
                context! {
                    receiver => "self",
                    base_method,
                    meta_args => "",
                    has_meta => false,
                },
            ));
        }
    } else {
        // Accessor may return &mut or MutexGuard — bind to a named variable
        out.push_str(&render(
            "service_rs_inner_accessor.jinja",
            context! {
                inner_accessor,
            },
        ));
        if !metadata_names.is_empty() {
            out.push_str(&render(
                "service_rs_base_registration_call.jinja",
                context! {
                    receiver => "inner",
                    base_method,
                    meta_args,
                    has_meta => true,
                },
            ));
        } else {
            out.push_str(&render(
                "service_rs_base_registration_call.jinja",
                context! {
                    receiver => "inner",
                    base_method,
                    meta_args => "",
                    has_meta => false,
                },
            ));
        }
    }

    // Handle error if the registration is fallible
    if reg.error_type.is_some() {
        out.push_str("        .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;\n");
    } else {
        out.push_str("        ;\n");
    }

    out.push_str(&render("service_rs_unit_ok_footer.jinja", context! {}));
}

/// Emit one `#[napi]` method for a service entrypoint (run or finalize) on the App class.
///
/// The method delegates to the inner owner's entrypoint via the configured
/// `host_app_inner_accessor`, allowing the TypeScript service wrapper to call
/// the entrypoint directly as `nativeRun()` or `nativeIntoRouter()` without
/// relying on free functions.
fn gen_entrypoint_napi_method(
    out: &mut String,
    service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    _api: &ApiSurface,
    core_import: &str,
    config: &ResolvedCrateConfig,
) {
    let ep_method = &ep.method;
    let js_name = format!("native{}", ep_method.to_upper_camel_case());

    // Look up the inner-accessor expression for this service
    let inner_accessor: String = config
        .services
        .iter()
        .find(|sc| sc.owner_type == service.name)
        .and_then(|sc| sc.host_app_inner_accessor.as_deref())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| "self".to_owned());

    // Build method signature from entrypoint params
    let mut rust_params = vec!["&self".to_owned()];
    for p in &ep.params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        rust_params.push(format!("{}: {}", p.name, rust_ty));
    }
    let param_sig = rust_params.join(", ");

    // Return `()` for both Run and Finalize: the inner Router (for Finalize) is
    // not host-serialisable, and JS only needs the side-effect / validation.
    let _ = EntrypointKind::Run; // pacify dead-code lint if unused
    let return_ty = match ep.kind {
        EntrypointKind::Run | EntrypointKind::Finalize => "()".to_owned(),
    };

    let doc = ep
        .doc
        .trim()
        .lines()
        .map(|line| {
            if line.is_empty() {
                "///".to_owned()
            } else {
                format!("/// {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let async_kw = if ep.is_async { "async " } else { "" };
    out.push_str(&render(
        "service_rs_entrypoint_method_header.jinja",
        context! {
            ep_method,
            doc,
            js_name,
            async_kw,
            param_sig,
            return_ty,
        },
    ));

    // Build parameter list for the inner call
    let ep_args: Vec<String> = ep.params.iter().map(|p| p.name.clone()).collect();
    let args_str = ep_args.join(", ");
    // Bind non-Unit returns to `_` so the unwrapped value (after `?`-propagation) doesn't
    // trigger `unused_must_use` for `Result`-returning entrypoints like `into_router`.
    let bind = if matches!(ep.return_type, TypeRef::Unit) {
        ""
    } else {
        "let _ = "
    };

    // Run/Finalize entrypoints conventionally consume `self` by value, so we move
    // the owner out of the lock with `std::mem::take` (requires the owner type to
    // implement `Default`) and drop the guard before any `.await`.
    if inner_accessor == "self" {
        if ep.is_async {
            out.push_str(&render(
                "service_rs_entrypoint_call.jinja",
                context! {
                    bind,
                    receiver => "self",
                    ep_method,
                    args_str,
                    is_async => true,
                },
            ));
        } else {
            out.push_str(&render(
                "service_rs_entrypoint_call.jinja",
                context! {
                    bind,
                    receiver => "self",
                    ep_method,
                    args_str,
                    is_async => false,
                },
            ));
        }
    } else {
        out.push_str(&render(
            "service_rs_take_owner.jinja",
            context! {
                inner_accessor,
            },
        ));
        if ep.is_async {
            out.push_str(&render(
                "service_rs_entrypoint_call.jinja",
                context! {
                    bind,
                    receiver => "owner",
                    ep_method,
                    args_str,
                    is_async => true,
                },
            ));
        } else {
            out.push_str(&render(
                "service_rs_entrypoint_call.jinja",
                context! {
                    bind,
                    receiver => "owner",
                    ep_method,
                    args_str,
                    is_async => false,
                },
            ));
        }
    }

    // Handle error if the entrypoint is fallible
    if ep.error_type.is_some() {
        out.push_str("        .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;\n");
    } else {
        out.push_str("        ;\n");
    }

    out.push_str(&render("service_rs_unit_ok_footer.jinja", context! {}));
}
