//! Service-API codegen for the Rustler (Elixir) backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **`service.ex`** — Elixir module with a server-like class containing:
//!    - A constructor and configurator methods.
//!    - Registration decorator-style helpers that store callbacks.
//!    - A GenServer to dispatch trait_call messages to registered handlers.
//!    - A `run` entrypoint that marshals registrations to Rust.
//!
//! 2. **`service.rs`** — Rust rustler glue that:
//!    - Emits a message-passing handler bridge for each referenced `HandlerContractDef`.
//!    - Provides a `#[rustler::nif]` run function (with `schedule = "DirtyCpu"`) that
//!      receives registrations, builds the service, and drives entrypoints.
//!    - The bridge sends `{:trait_call, method, args_json, reply_id}` to the Elixir pid
//!      and awaits the response via a `complete_trait_call` NIF.
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

use crate::backends::rustler::template_env::render;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Convert a `TypeRef` to a simple Elixir type annotation string.
#[allow(dead_code)]
fn elixir_type_annotation(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String.t()".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "boolean()".to_owned(),
                PrimitiveType::F32 | PrimitiveType::F64 => "float()".to_owned(),
                _ => "integer()".to_owned(),
            }
        }
        TypeRef::Bytes => "binary()".to_owned(),
        TypeRef::Optional(inner) => format!("{} | nil", elixir_type_annotation(inner)),
        TypeRef::Vec(inner) => format!("list({})", elixir_type_annotation(inner)),
        TypeRef::Map(k, v) => format!(
            "map() :: %{{optional({}) => {}}}",
            elixir_type_annotation(k),
            elixir_type_annotation(v)
        ),
        TypeRef::Unit => "nil".to_owned(),
        TypeRef::Named(n) => n.to_string(),
        TypeRef::Json => "any()".to_owned(),
        TypeRef::Path => "String.t()".to_owned(),
        TypeRef::Duration => "non_neg_integer()".to_owned(),
    }
}

fn push_elixir_param(params: &mut String, name: &str, optional: bool) {
    params.push_str(", ");
    params.push_str(name);
    if optional {
        params.push_str(" \\\\ nil");
    }
}

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

// ──────────────────────────────────────────────────────────────── Elixir output ──

/// Generate the idiomatic Elixir service module (`service.ex`).
///
/// Produces an Elixir module containing:
/// - A struct holding configuration state and registrations.
/// - A constructor mirroring [`ServiceDef::constructor`].
/// - Configurator methods from [`ServiceDef::configurators`].
/// - Decorator-style registration helpers from [`ServiceDef::registrations`].
/// - A GenServer that handles `{:trait_call, ...}` messages from Rust.
/// - A `run` entrypoint that delegates to the native NIF.
pub(super) fn gen_service_ex(api: &ApiSurface, module_prefix: &str) -> String {
    let mut out = String::new();

    out.push_str("# This file is generated. Do not edit.\n\n");

    for service in &api.services {
        gen_service_module(&mut out, service, api, module_prefix);
    }

    out
}

/// Format a Rust doc as an Elixir heredoc body at the given column indent.
/// Returns just the lines between `"""` markers (does not emit the markers
/// themselves). Each non-blank source line is indented to `indent` spaces so
/// the closing `"""` at the same column strips that prefix from the heredoc
/// at compile time; blank lines stay bare.
fn elixir_heredoc_body(text: &str, indent: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for line in trimmed.lines() {
        if line.trim().is_empty() {
            out.push('\n');
        } else {
            out.push_str(&pad);
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn push_elixir_doc(out: &mut String, doc: &str, attr: &str) {
    if doc.is_empty() {
        return;
    }
    out.push_str(&render(
        "service_api_doc.ex.jinja",
        context! {
            attr => attr,
            body => elixir_heredoc_body(doc, 2),
        },
    ));
}

fn gen_service_module(out: &mut String, service: &ServiceDef, api: &ApiSurface, module_prefix: &str) {
    let module_name = if !module_prefix.is_empty() {
        format!("{}.{}", module_prefix, service.name)
    } else {
        service.name.clone()
    };
    let module_snake = service.name.to_snake_case();

    let doc_body = if service.doc.is_empty() {
        String::new()
    } else {
        elixir_heredoc_body(&service.doc, 2)
    };
    out.push_str(&render(
        "service_api_module_header.ex.jinja",
        context! {
            module_name => module_name,
            doc_body => doc_body,
            module_prefix => module_prefix,
        },
    ));

    let mut all_fields = vec!["registrations".to_owned()];
    all_fields.extend(service.constructor.params.iter().map(|p| p.name.clone()));
    for method in &service.configurators {
        all_fields.extend(method.params.iter().map(|p| p.name.clone()));
    }
    // Format fields with proper commas for mix format
    let formatted_fields = all_fields
        .iter()
        .enumerate()
        .map(|(i, field)| {
            let comma = if i < all_fields.len() - 1 { "," } else { "" };
            format!("    :{}{}\n", field, comma)
        })
        .collect::<String>();
    out.push_str(&render(
        "service_api_struct.ex.jinja",
        context! {
            formatted_fields => formatted_fields,
        },
    ));

    // Constructor
    {
        let ctor = &service.constructor;
        let params = if ctor.params.is_empty() {
            ["_options \\\\ []".to_owned()]
        } else {
            ["options \\\\ []".to_owned()]
        };
        let mut field_inits = vec!["registrations: []".to_owned()];

        for p in &ctor.params {
            if p.optional {
                field_inits.push(format!("{}: Keyword.get(options, :{}, nil)", p.name, p.name));
            } else {
                field_inits.push(format!("{}: Keyword.fetch!(options, :{})", p.name, p.name));
            }
        }

        push_elixir_doc(out, &ctor.doc, "doc");
        // Format field inits with proper commas for mix format
        let formatted_inits = field_inits
            .iter()
            .enumerate()
            .map(|(i, init)| {
                let comma = if i < field_inits.len() - 1 { "," } else { "" };
                format!("      {}{}\n", init, comma)
            })
            .collect::<String>();
        out.push_str(&render(
            "service_api_constructor.ex.jinja",
            context! {
                params => params.join(", "),
                formatted_inits => formatted_inits,
            },
        ));
    }

    // Configurator methods
    for method in &service.configurators {
        let method_name = &method.name;
        let mut params = vec!["self".to_owned()];
        for p in &method.params {
            if p.optional {
                params.push(format!("{} \\\\ nil", p.name));
            } else {
                params.push(p.name.clone());
            }
        }

        push_elixir_doc(out, &method.doc, "doc");
        let updates = method.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>();
        out.push_str(&render(
            "service_api_configurator.ex.jinja",
            context! {
                method_name => method_name,
                params => params.join(", "),
                updates => updates,
            },
        ));
    }

    // Registration methods as decorator-style helpers
    for reg in &service.registrations {
        gen_registration_method(out, reg, service, api, module_prefix);
    }

    // GenServer module for dispatching trait_call messages
    gen_genserver_module(out, service, api);

    // Entrypoint methods
    for ep in &service.entrypoints {
        let ep_name = &ep.method;
        let mut params = vec!["self".to_owned()];
        for p in &ep.params {
            if p.optional {
                params.push(format!("{} \\\\ nil", p.name));
            } else {
                params.push(p.name.clone());
            }
        }

        match ep.kind {
            EntrypointKind::Run => {
                push_elixir_doc(out, &ep.doc, "doc");
                let native_fn = format!("{}_{}", module_snake, ep_name);
                let call_args = ep.params.iter().map(|p| format!(", {}", p.name)).collect::<String>();
                out.push_str(&render(
                    "service_api_entrypoint.ex.jinja",
                    context! {
                        ep_name => ep_name,
                        params => params.join(", "),
                        native_fn => native_fn,
                        call_args => call_args,
                    },
                ));
            }
            EntrypointKind::Finalize => {
                push_elixir_doc(out, &ep.doc, "doc");
                let native_fn = format!("{}_{}", module_snake, ep_name);
                let call_args = ep.params.iter().map(|p| format!(", {}", p.name)).collect::<String>();
                out.push_str(&render(
                    "service_api_entrypoint.ex.jinja",
                    context! {
                        ep_name => ep_name,
                        params => params.join(", "),
                        native_fn => native_fn,
                        call_args => call_args,
                    },
                ));
            }
        }
    }

    out.push_str("end\n\n");
}

fn gen_registration_method(
    out: &mut String,
    reg: &RegistrationDef,
    _service: &ServiceDef,
    _api: &ApiSurface,
    module_prefix: &str,
) {
    let method_name = &reg.method;

    push_elixir_doc(out, &reg.doc, "doc");
    let mut params = "self".to_owned();
    for p in &reg.metadata_params {
        push_elixir_param(&mut params, &p.name, p.optional);
    }
    params.push_str(", handler");

    // Build metadata tuple
    let meta_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
    let meta_tuple = if meta_names.is_empty() {
        "{}".to_owned()
    } else {
        format!("{{{}}}", meta_names.join(", "))
    };

    out.push_str(&render(
        "service_api_registration_method.ex.jinja",
        context! {
            method_name => method_name,
            params => params,
            meta_tuple => meta_tuple,
        },
    ));

    // Emit a simple HandlerWrapper GenServer if this is the route registration
    if method_name == "route" {
        out.push_str(&render("service_api_handler_wrapper.ex.jinja", context! {}));
    }

    // Emit registration variants (decorator-style shortcuts)
    for variant in &reg.variants {
        gen_registration_variant_method(out, variant, reg, module_prefix);
    }
}

fn gen_registration_variant_method(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
    module_prefix: &str,
) {
    match variant.style {
        RegistrationVariantStyle::VerbDecorator => {
            emit_verb_decorator_variant(out, variant, base_reg, module_prefix);
        }
        RegistrationVariantStyle::Builder => {
            emit_builder_variant(out, variant, base_reg, module_prefix);
        }
        RegistrationVariantStyle::Hybrid => {
            emit_verb_decorator_variant(out, variant, base_reg, module_prefix);
            emit_builder_variant(out, variant, base_reg, module_prefix);
        }
    }
}

/// Convert a Rust enum value expression (e.g. `"my_crate::Method::Get"`) to an
/// Elixir function call (e.g. `"Bindings.Method.get()"`).
///
/// The last two `::` segments are taken as `TypeName::VariantName`; the variant
/// name is lowercased to form the Elixir function call. When `module_prefix` is
/// non-empty it is prepended, otherwise the bare type name is used.
fn rust_enum_expr_to_elixir(value_expr: &str, module_prefix: &str) -> String {
    let parts: Vec<&str> = value_expr.split("::").collect();
    if parts.len() >= 2 {
        let type_name = parts[parts.len() - 2];
        let variant = parts[parts.len() - 1].to_lowercase();
        if module_prefix.is_empty() {
            format!("{type_name}.{variant}()")
        } else {
            format!("{module_prefix}.{type_name}.{variant}()")
        }
    } else {
        // Fallback: use verbatim (already a literal)
        value_expr.to_owned()
    }
}

/// Build the Elixir expression that constructs the wrapper value for a variant,
/// e.g. `builder = Bindings.RouteBuilder.new(Bindings.Method.get(), path)`.
///
/// Returns `None` when the variant has no `wrapper_call`.
fn build_elixir_wrapper_constructor_expr(
    variant: &crate::core::ir::RegistrationVariant,
    module_prefix: &str,
) -> Option<String> {
    let wc = variant.wrapper_call.as_ref()?;
    let mut call_args: Vec<String> = Vec::new();
    for arg in &wc.args {
        match arg {
            crate::core::ir::WrapperConstructorArg::Fixed { value_expr, .. } => {
                call_args.push(rust_enum_expr_to_elixir(value_expr, module_prefix));
            }
            crate::core::ir::WrapperConstructorArg::Free { param } => {
                call_args.push(param.name.clone());
            }
        }
    }
    let type_name = &wc.wrapper_type_name;
    let ctor = &wc.constructor_method;
    let qualified_type = if module_prefix.is_empty() {
        type_name.clone()
    } else {
        format!("{module_prefix}.{type_name}")
    };
    let call_expr = if ctor.is_empty() || ctor == "__init__" {
        format!("{qualified_type}({})", call_args.join(", "))
    } else {
        format!("{qualified_type}.{ctor}({})", call_args.join(", "))
    };
    Some(format!("{} = {}", wc.metadata_param, call_expr))
}

/// Emit the verb-decorator form: `def variant(app, path, ..., handler) do ... end`.
///
/// When the variant has a `wrapper_call`, the body constructs the wrapper object
/// (e.g. `builder = RouteBuilder.new(Method.get(), path)`) and delegates to the
/// base registration method. Without a wrapper, it delegates directly passing the
/// free params to the base method.
fn emit_verb_decorator_variant(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
    module_prefix: &str,
) {
    let variant_name = &variant.name;
    let base_method = &base_reg.method;

    if let Some(doc) = &variant.doc {
        push_elixir_doc(out, doc, "doc");
    }

    // Emit signature: app, then signature_params, then handler
    let mut params = "app".to_owned();
    for param in &variant.signature_params {
        push_elixir_param(&mut params, &param.name, param.optional);
    }
    params.push_str(", handler");

    let (wrapper_expr, call_args) =
        if let Some(wrapper_expr) = build_elixir_wrapper_constructor_expr(variant, module_prefix) {
            (
                wrapper_expr,
                format!(
                    "app, {}",
                    base_reg
                        .metadata_params
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        } else {
            // Direct pattern: build call args by substituting overrides into the base params.
            let mut call_args: Vec<String> = Vec::new();
            for base_param in &base_reg.metadata_params {
                if let Some(override_) = variant.overrides.iter().find(|o| o.param_name == base_param.name) {
                    // Fixed override: convert Rust expression to Elixir
                    call_args.push(rust_enum_expr_to_elixir(&override_.value_expr, module_prefix));
                } else if let Some(sig_param) = variant.signature_params.iter().find(|s| s.name == base_param.name) {
                    call_args.push(sig_param.name.clone());
                }
            }
            let call_args = if call_args.is_empty() {
                "app".to_owned()
            } else {
                format!("app, {}", call_args.join(", "))
            };
            (String::new(), call_args)
        };

    out.push_str(&render(
        "service_api_verb_decorator.ex.jinja",
        context! {
            variant_name => variant_name,
            params => params,
            wrapper_expr => wrapper_expr,
            base_method => base_method,
            call_args => call_args,
        },
    ));
}

/// Emit the builder form: `def variant_decorator(app, path, ...) do ... end` returning a closure.
///
/// The returned closure accepts a handler and delegates to the verb-decorator form of the same
/// variant. When a `wrapper_call` is present, the wrapper is built inside the closure so each
/// call produces a fresh wrapper instance.
fn emit_builder_variant(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    _base_reg: &RegistrationDef,
    module_prefix: &str,
) {
    let variant_name = &variant.name;
    let builder_name = format!("{}_decorator", variant_name);

    if let Some(doc) = &variant.doc {
        push_elixir_doc(out, doc, "doc");
    }

    // Emit signature: app, then signature_params (no handler)
    let mut params = "app".to_owned();
    for param in &variant.signature_params {
        push_elixir_param(&mut params, &param.name, param.optional);
    }
    let call_args = std::iter::once("app".to_owned())
        .chain(variant.signature_params.iter().map(|p| p.name.clone()))
        .collect::<Vec<_>>()
        .join(", ");

    out.push_str(&render(
        "service_api_builder_variant.ex.jinja",
        context! {
            builder_name => builder_name,
            params => params,
            variant_name => variant_name,
            call_args => call_args,
        },
    ));
    let _ = module_prefix; // consumed via emit_verb_decorator_variant delegation
}

fn gen_genserver_module(out: &mut String, service: &ServiceDef, _api: &ApiSurface) {
    let module_name = &service.name;
    let server_module = format!("{}.Handler", module_name);

    out.push_str(&render(
        "service_api_genserver.ex.jinja",
        context! {
            server_module => server_module,
        },
    ));
}

// ──────────────────────────────────────────────────────────────── Rust glue ──

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

/// Emit a NIF for one registration variant.
///
/// The variant builds a wrapper (if `wrapper_call` is set) and calls the base
/// registration method with the constructed wrapper + fixed args + free args.
fn gen_registration_variant_nif(
    out: &mut String,
    service: &ServiceDef,
    base_reg: &RegistrationDef,
    variant: &crate::core::ir::RegistrationVariant,
    api: &ApiSurface,
    core_import: &str,
) {
    let service_snake = service.name.to_snake_case();
    let variant_name = &variant.name;
    let nif_name = format!("{}_{}", service_snake, variant_name);
    let base_method = &base_reg.method;
    let contract_name = &base_reg.callback_contract;
    let bridge_wrapper = format!("Elixir{contract_name}Bridge");
    let owner_path = &service.rust_path;

    // Build NIF signature
    let mut params = vec!["registrations: rustler::Term<'_>".to_owned()];
    for param in &variant.signature_params {
        let rust_ty = typeref_to_rust_type(&param.ty, core_import);
        params.push(format!("{}: {}", param.name, rust_ty));
    }
    params.push("handler: rustler::LocalPid".to_owned());
    let param_sig = params.join(", ");

    let (wrapper_type_name, wrapper_type_path, constructor_method, wrapper_args) =
        if let Some(wrapper_call) = &variant.wrapper_call {
            let wrapper_args = wrapper_call
                .args
                .iter()
                .map(|arg| match arg {
                    crate::core::ir::WrapperConstructorArg::Fixed {
                        param_name: _,
                        value_expr,
                    } => format!("        {},\n", value_expr),
                    crate::core::ir::WrapperConstructorArg::Free { param } => {
                        format!("        {},\n", param.name)
                    }
                })
                .collect::<String>();
            (
                wrapper_call.wrapper_type_name.as_str(),
                wrapper_call.wrapper_type_path.as_str(),
                wrapper_call.constructor_method.as_str(),
                wrapper_args,
            )
        } else {
            ("", "", "", String::new())
        };

    out.push_str(&render(
        "service_api_registration_variant_nif_header.rs.jinja",
        context! {
            variant_name => variant_name,
            base_method => base_method,
            nif_name => nif_name,
            param_sig => param_sig,
            owner_path => owner_path,
            wrapper_type_name => wrapper_type_name,
            wrapper_type_path => wrapper_type_path,
            constructor_method => constructor_method,
            wrapper_args => wrapper_args,
        },
    ));

    let metadata_param_names: Vec<&str> = base_reg.metadata_params.iter().map(|p| p.name.as_str()).collect();

    let (has_metadata, trailing, tuple_types, opaque_bindings, metadata_args) = if !metadata_param_names.is_empty() {
        let trailing = if metadata_param_names.len() == 1 { "," } else { "" };
        let tuple_types = base_reg
            .metadata_params
            .iter()
            .map(|p| {
                // Opaque types use super:: to name the local lib-module wrapper that implements
                // rustler::Resource. The wildcard import in service.rs would shadow a bare name.
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

        let mut opaque_bindings = String::new();
        for meta_param in base_reg.metadata_params.iter() {
            let is_opaque = if let TypeRef::Named(n) = &meta_param.ty {
                api.types.iter().any(|t| &t.name == n && !t.is_trait && t.is_opaque)
            } else {
                false
            };
            if is_opaque {
                if let TypeRef::Named(n) = &meta_param.ty {
                    // ResourceArc<super::T> derefs to the local wrapper super::T; wrapper.inner
                    // is Arc<CoreType>. Use as_ref() then clone() to obtain an owned CoreType.
                    opaque_bindings.push_str(&render(
                        "service_api_opaque_metadata_binding.rs.jinja",
                        context! {
                            indent => "                ",
                            param_name => meta_param.name,
                            core_import => core_import,
                            type_name => n,
                        },
                    ));
                }
            }
        }

        (
            true,
            trailing,
            tuple_types_with_trailing,
            opaque_bindings,
            metadata_param_names.join(", "),
        )
    } else {
        (false, "", String::new(), String::new(), String::new())
    };

    out.push_str(&render(
        "service_api_registration_variant_dispatch.rs.jinja",
        context! {
            has_metadata => has_metadata,
            metadata_names => metadata_param_names.join(", "),
            trailing => trailing,
            tuple_types => tuple_types,
            opaque_bindings => opaque_bindings,
            bridge_wrapper => bridge_wrapper,
            core_import => core_import,
            contract_name => base_reg.callback_contract,
            base_method => base_method,
            metadata_args => metadata_args,
        },
    ));

    out.push_str(&render(
        "service_api_registration_variant_nif_footer.rs.jinja",
        context! {},
    ));
}

/// Map a `TypeRef` to a Rust type string for use in generated NIF signatures.
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

/// Generate all service-API files for the rustler backend.
///
/// Returns up to two `GeneratedFile`s per non-empty service list:
/// - `{output_dir}/service.rs`   — Rust rustler glue
/// - `{elixir_pkg}/service.ex`   — idiomatic Elixir module
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(
        config.output_paths.get("elixir"),
        &config.name,
        "packages/elixir/native/{name}_nif/src/",
    );

    // Rust glue
    let service_rs = gen_service_rs(api, config);

    // Elixir module — pass the consumer's module prefix so the
    // service module can `alias <Prefix>.Native`.
    let (_, module_prefix) = super::helpers::get_module_info(api, config);
    let service_ex = gen_service_ex(api, &module_prefix);

    // Determine Elixir package output directory
    let elixir_pkg = config.output_paths.get("elixir").map(PathBuf::from).unwrap_or_else(|| {
        let app_name = config.elixir_app_name();
        PathBuf::from(format!("packages/elixir/lib/{}", app_name))
    });

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: elixir_pkg.join("service.ex"),
            content: service_ex,
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

    /// `gen_service_ex` emits a module named after the service owner.
    #[test]
    fn elixir_output_contains_service_module() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        // The compiled namespace is implicitly `Elixir.<Name>`, so the emitted
        // source must NOT re-prefix it (`defmodule Elixir.<Name>` compiles to
        // `Elixir.Elixir.<Name>`).
        assert!(
            output.contains("defmodule TestService do"),
            "expected `defmodule TestService do` in output:\n{output}"
        );
    }

    /// `gen_service_ex` emits a struct definition.
    #[test]
    fn elixir_output_contains_struct_definition() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(
            output.contains("defstruct"),
            "expected `defstruct` in output:\n{output}"
        );
        assert!(
            output.contains(":registrations"),
            "expected `:registrations` field in output:\n{output}"
        );
    }

    /// `gen_service_ex` emits a constructor.
    #[test]
    fn elixir_output_contains_constructor() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(output.contains("def new("), "expected `def new(` in output:\n{output}");
    }

    /// `gen_service_ex` emits configurator methods.
    #[test]
    fn elixir_output_contains_configurator() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(
            output.contains("def with_timeout("),
            "expected `with_timeout` configurator:\n{output}"
        );
    }

    /// `gen_service_ex` emits a registration method.
    #[test]
    fn elixir_output_contains_registration() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(
            output.contains("def add_handler("),
            "expected `add_handler` registration method:\n{output}"
        );
    }

    /// `gen_service_ex` emits a GenServer module.
    #[test]
    fn elixir_output_contains_genserver_module() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(
            output.contains("defmodule TestService.Handler do"),
            "expected `TestService.Handler` GenServer:\n{output}"
        );
        assert!(
            output.contains("use GenServer"),
            "expected `use GenServer` in output:\n{output}"
        );
    }

    /// `gen_service_ex` emits the `run` entrypoint.
    #[test]
    fn elixir_output_contains_run_entrypoint() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");
        assert!(output.contains("def run("), "expected `def run(` in output:\n{output}");
    }

    /// `gen_service_rs` emits the handler bridge struct.
    #[test]
    fn rust_output_contains_handler_bridge_struct() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("pub struct ElixirRequestHandlerBridge"),
            "expected `ElixirRequestHandlerBridge` struct:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge trait impl.
    #[test]
    fn rust_output_contains_handler_bridge_impl() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("impl my_crate::RequestHandler for ElixirRequestHandlerBridge"),
            "expected trait impl:\n{output}"
        );
        assert!(
            output.contains("fn handle(") && output.contains("Pin<Box<dyn std::future::Future<Output"),
            "expected boxed-future dispatch method:\n{output}"
        );
    }

    /// `gen_service_rs` emits the `#[rustler::nif]` run entry point.
    #[test]
    fn rust_output_contains_nif_run() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("#[rustler::nif(schedule = \"DirtyCpu\")]"),
            "expected `#[rustler::nif(schedule = \"DirtyCpu\")]` attribute:\n{output}"
        );
        assert!(
            output.contains("pub fn test_service_run("),
            "expected `test_service_run` function:\n{output}"
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
        assert!(paths.contains(&"service.ex"), "expected service.ex in output");
    }

    /// Full `generate()` returns empty for a surface with no services.
    #[test]
    fn generate_returns_empty_for_no_services() {
        let surface = ApiSurface::default();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    /// Elixir GenServer `handle_cast` actually decodes args and calls handler.
    #[test]
    fn elixir_genserver_handle_cast_decodes_args_and_dispatches() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");

        // Assert that handle_cast decodes args_json
        assert!(
            output.contains("decode_args_and_dispatch(method, args_json, registrations)"),
            "expected decode_args_and_dispatch call in handle_cast:\n{output}"
        );

        // Assert that it calls complete_trait_call with reply_id
        assert!(
            output.contains("Native.complete_trait_call(reply_id, response)"),
            "expected Native.complete_trait_call(reply_id, response) call:\n{output}"
        );

        // Assert that there are NO stub comments or empty placeholders
        assert!(
            !output.contains("simplified stub"),
            "found 'simplified stub' comment — dispatch should not be stubbed:\n{output}"
        );
        assert!(
            !output.contains("placeholder"),
            "found unsupported comment in dispatch logic:\n{output}"
        );
        assert!(
            !output.contains("# This is a simplified stub"),
            "found stub marker in dispatch:\n{output}"
        );
    }

    /// Elixir GenServer dispatch helper decodes JSON and calls registered handler.
    #[test]
    fn elixir_genserver_dispatch_helper_invokes_handler() {
        let surface = make_fixture_surface();
        let output = gen_service_ex(&surface, "");

        // Assert that decode_args_and_dispatch helper exists
        assert!(
            output.contains("defp decode_args_and_dispatch(method, args_json, registrations) do"),
            "expected decode_args_and_dispatch helper function:\n{output}"
        );

        // Assert that it decodes JSON
        assert!(
            output.contains("Jason.decode(args_json)"),
            "expected Jason.decode(args_json) in dispatch:\n{output}"
        );

        // Assert that it calls the registered handler
        assert!(
            output.contains("response = handler.(args)"),
            "expected handler.(args) invocation:\n{output}"
        );

        // Assert that response is encoded back to JSON
        assert!(
            output.contains("Jason.encode(response)"),
            "expected Jason.encode(response) in dispatch:\n{output}"
        );

        // Assert that find_handler helper looks up by method name
        assert!(
            output.contains("defp find_handler"),
            "expected find_handler helper function:\n{output}"
        );
    }

    /// Rust NIF parses registrations and constructs service owner.
    #[test]
    fn rust_nif_parses_registrations_and_constructs_owner() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);

        // Assert that registrations are parsed from Elixir term
        assert!(
            output.contains("let registration_list: Vec<rustler::Term<'_>> = registrations"),
            "expected registration list parsing in NIF:\n{output}"
        );

        // Assert that service owner is constructed
        assert!(
            output.contains("let mut owner = my_crate::TestService::new()"),
            "expected owner construction in NIF:\n{output}"
        );

        // Assert that registrations are iterated and dispatched
        assert!(
            output.contains("for reg_entry in registration_list"),
            "expected registration iteration in NIF:\n{output}"
        );

        // Assert that no stub markers remain
        assert!(
            !output.contains("placeholder: parse registrations"),
            "found placeholder in registration parsing — should be implemented:\n{output}"
        );
        assert!(
            !output.contains("For now, return a stub"),
            "found stub return in NIF — should be fully implemented:\n{output}"
        );
    }

    /// No empty-JSON or stub responses in generated code.
    ///
    /// Verifies that the Rust NIF actually invokes `owner.run(...)` or `owner.finalize(...)`
    /// and does not emit stub placeholder responses.
    #[test]
    fn no_stub_responses_in_generated_code() {
        let surface = make_fixture_surface();
        let config = make_test_config();

        let elixir_output = gen_service_ex(&surface, "");
        let rust_output = gen_service_rs(&surface, &config);

        // Elixir should not return empty JSON map
        assert!(
            !elixir_output.contains("response = {:ok, %{}}"),
            "found stub response {{:ok, %{{}}}} in Elixir generated code:\n{elixir_output}"
        );

        // Elixir should not have commented-out complete_trait_call
        assert!(
            !elixir_output.contains("# Native.complete_trait_call"),
            "found commented-out complete_trait_call in Elixir:\n{elixir_output}"
        );

        // Rust should not contain stub comment markers
        assert!(
            !rust_output.contains("would be called here"),
            "found 'would be called here' stub comment in Rust NIF:\n{rust_output}"
        );
        assert!(
            !rust_output.contains("would happen here"),
            "found 'would happen here' stub comment in Rust NIF:\n{rust_output}"
        );

        // Rust should actually call owner.run(...) or owner.finalize(...)
        assert!(
            rust_output.contains("owner.run(") || rust_output.contains("owner.finalize("),
            "Rust NIF should call owner.run(...) or owner.finalize(...), found neither:\n{rust_output}"
        );

        // Rust should register handlers before calling entrypoint
        assert!(
            rust_output.contains("ElixirRequestHandlerBridge"),
            "Rust NIF should create handler bridge instances:\n{rust_output}"
        );

        // Regression: Rust should NOT contain illegal if-let type ascription pattern
        // (`: Result<...> =` on if-let patterns is a syntax error in Rust)
        assert!(
            !rust_output.contains("): Result<"),
            "found illegal if-let type ascription pattern '): Result<' in generated Rust:\n{rust_output}"
        );

        // Rust Term args must be lifetime-annotated (Term<'_> or Term<'a>)
        assert!(
            rust_output.contains("Term<'_>"),
            "expected lifetime-annotated Term<'_> in generated Rust NIF signature:\n{rust_output}"
        );
    }

    /// Verify that registration variant style is respected in generated Elixir code.
    ///
    /// Regression test for issue #26: the rustler backend must pattern-match on
    /// `RegistrationVariantStyle` and emit the appropriate Elixir registration forms.
    #[test]
    fn registration_variant_style_hybrid_emits_both_forms() {
        let mut surface = make_fixture_surface();
        let _config = make_test_config();

        // Attach a Hybrid-styled variant `get` so the variant emission loop runs.
        // The base `add_handler` is emitted unconditionally by gen_registration_method;
        // RegistrationVariantStyle gates only the per-variant verb/builder emission.
        surface.services[0].registrations[0]
            .variants
            .push(crate::core::ir::RegistrationVariant {
                name: "get".to_owned(),
                overrides: vec![crate::core::ir::RegistrationVariantOverride {
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
                doc: None,
                style: RegistrationVariantStyle::Hybrid,
            });

        let elixir_output = gen_service_ex(&surface, "");

        // Hybrid → verb-decorator form
        assert!(
            elixir_output.contains("def get(app, path, handler) do"),
            "expected verb-decorator form 'def get(app, path, handler) do' in Elixir output:\n{elixir_output}"
        );

        // Hybrid → builder form
        assert!(
            elixir_output.contains("def get_decorator(app, path) do"),
            "expected builder form 'def get_decorator(app, path) do' in Elixir output:\n{elixir_output}"
        );
    }

    /// Verify that send_trait_call message is emitted in generated handler bridge.
    ///
    /// Regression test for issue #119: the handler bridge must send the trait_call message
    /// to the Elixir GenServer via OwnedEnv::send_and_clear, not just await silently.
    #[test]
    fn handler_bridge_sends_trait_call_message() {
        let surface = make_fixture_surface();
        let config = make_test_config();

        let rust_output = gen_service_rs(&surface, &config);

        // Verify that OwnedEnv is imported
        assert!(
            rust_output.contains("OwnedEnv"),
            "expected OwnedEnv import in generated code"
        );

        // Verify that send_and_clear is called
        assert!(
            rust_output.contains("env.send_and_clear(&pid"),
            "expected env.send_and_clear(&pid, ...) call in generated handler bridge:\n{rust_output}"
        );

        // Verify that trait_call atom is sent
        assert!(
            rust_output.contains("Atom::from_str(env, \"trait_call\")"),
            "expected atom::from_str for 'trait_call' in generated message:\n{rust_output}"
        );

        // Verify that the method name is included in the message
        assert!(
            rust_output.contains("method_name"),
            "expected method_name variable in trait_call message"
        );

        // Verify that request_json is included
        assert!(
            rust_output.contains("request_json_clone"),
            "expected request JSON to be sent in trait_call message"
        );

        // Verify that reply_id is included
        assert!(
            rust_output.contains("reply_id)"),
            "expected reply_id in trait_call tuple"
        );

        // Regression: ensure the old commented-out line is not present
        assert!(
            !rust_output.contains("// crate::nif_support::send_trait_call"),
            "found old commented-out send_trait_call in output — should be replaced with real call"
        );

        // Verify spawn_blocking wraps the send
        assert!(
            rust_output.contains("tokio::task::spawn_blocking(move || {"),
            "expected spawn_blocking to wrap the message send"
        );
    }

    /// Verify that Rust codegen emits core crate import + trait implementation.
    /// This tests GAP 1 (core import) and GAP 3 (trait cast).
    #[test]
    fn rust_codegen_emits_core_import_and_trait_impl() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let rust_output = gen_service_rs(&surface, &config);

        // GAP 1: Verify core crate import
        assert!(
            rust_output.contains("use my_crate::*;"),
            "expected core crate wildcard import in gen_service_rs output:\n{rust_output}"
        );

        // GAP 3: Verify bridge trait implementation
        assert!(
            rust_output.contains("impl my_crate::RequestHandler for ElixirRequestHandlerBridge"),
            "expected trait impl for bridge in generated output:\n{rust_output}"
        );

        // Verify handler variable bindings for trait casting
        assert!(
            rust_output.contains("let handler: Arc<dyn my_crate::RequestHandler> = Arc::new(bridge);"),
            "expected handler trait cast in registration code:\n{rust_output}"
        );

        // Verify bridge struct definition
        assert!(
            rust_output.contains("pub struct ElixirRequestHandlerBridge"),
            "expected ElixirRequestHandlerBridge struct definition:\n{rust_output}"
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
