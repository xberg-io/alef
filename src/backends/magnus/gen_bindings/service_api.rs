//! Service-API codegen for the Magnus (Ruby) backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **`service.rb`** — An idiomatic Ruby class mirroring the service's constructor,
//!    configurator methods, registration methods accepting blocks/procs, and a `run`
//!    entrypoint that delegates to the native extension.
//!
//! 2. **`service.rs`** — Magnus glue that wraps each registered Ruby proc as
//!    `Arc<dyn <HandlerContractDef::trait_name>>` via an async callback bridge.
//!    The bridge acquires the GVL (Global VM Lock) to call the proc with request
//!    DTO and interprets the response. Also defines native `#[magnus::function]` for
//!    the run entrypoint that collects registrations, builds the core service,
//!    and drives it.
//!
//! All names are derived entirely from the [`ApiSurface`] IR — no transport-
//! or domain-specific assumptions are made anywhere in this module.

use super::lifecycle_error_ws_sse;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

fn render(template_name: &str, ctx: minijinja::Value) -> String {
    crate::backends::magnus::template_env::render(template_name, ctx)
}

/// Convert a `TypeRef` to a simple Ruby type annotation string.
fn ruby_type_annotation(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "true | false".to_owned(),
                PrimitiveType::F32 | PrimitiveType::F64 => "Float".to_owned(),
                _ => "Integer".to_owned(),
            }
        }
        TypeRef::Bytes => "String".to_owned(),
        TypeRef::Optional(inner) => format!("{} | nil", ruby_type_annotation(inner)),
        TypeRef::Vec(inner) => format!("Array[{}]", ruby_type_annotation(inner)),
        TypeRef::Map(k, v) => format!("Hash[{}, {}]", ruby_type_annotation(k), ruby_type_annotation(v)),
        TypeRef::Unit => "void".to_owned(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Json => "Object".to_owned(),
        TypeRef::Path => "String".to_owned(),
        TypeRef::Duration => "Float".to_owned(),
    }
}

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

/// Format a multi-line Rust doc comment as a Ruby block comment indented at
/// `indent` spaces. Every non-blank line is prefixed with `# `; blank lines
/// stay blank (so paragraph breaks survive). Trailing newline is included.
fn format_ruby_comment(text: &str, indent: usize) -> String {
    let trimmed = text.trim();
    let pad = " ".repeat(indent);
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for line in trimmed.lines() {
        if line.trim().is_empty() {
            out.push_str(&pad);
            out.push_str("#\n");
        } else {
            out.push_str(&pad);
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

// ─────────────────────────────────────────────────────────── Ruby output ──

/// Generate the idiomatic Ruby service class (`service.rb`).
///
/// Produces a Ruby module containing one class per service. Each class exposes:
/// - A constructor mirroring [`ServiceDef::constructor`].
/// - Configurator methods from [`ServiceDef::configurators`].
/// - Registration methods from [`ServiceDef::registrations`] that accept blocks.
/// - A `run(...)` method derived from the first [`EntrypointKind::Run`] entrypoint.
pub(super) fn gen_service_rb(api: &ApiSurface, native_module_name: &str, gem_require_name: &str) -> String {
    let mut out = String::new();

    out.push_str(&render(
        "service_rb_header.rb.jinja",
        minijinja::context! {
            gem_require_name => gem_require_name,
            has_services => !api.services.is_empty(),
            native_module_name => native_module_name,
        },
    ));

    if !api.services.is_empty() {
        for service in &api.services {
            gen_service_class(&mut out, service, api, native_module_name);
        }
        out.push_str("end\n");
    }

    // Emit error classes
    out.push('\n');
    out.push_str(&lifecycle_error_ws_sse::gen_error_classes(api));

    out
}

fn gen_service_class(out: &mut String, service: &ServiceDef, api: &ApiSurface, native_module_name: &str) {
    let class_name = &service.name;
    out.push_str(&render(
        "service_rb_class_header.rb.jinja",
        minijinja::context! {
            doc_comment => format_ruby_comment(&service.doc, 2),
            class_name => class_name,
        },
    ));

    // initialize
    {
        let ctor = &service.constructor;
        let mut init_params = Vec::new();
        let mut stored_args = Vec::new();

        for p in &ctor.params {
            let type_annotation = ruby_type_annotation(&p.ty);
            if p.optional {
                init_params.push(format!("{}: {} | nil = nil", p.name, type_annotation));
            } else {
                init_params.push(format!("{}: {}", p.name, type_annotation));
            }
            stored_args.push(p.name.clone());
        }

        let param_sig = if init_params.is_empty() {
            String::new()
        } else {
            format!("({})", init_params.join(", "))
        };

        out.push_str(&render(
            "service_rb_initialize.rb.jinja",
            minijinja::context! {
                param_sig => param_sig,
                doc_comment => format_ruby_comment(&ctor.doc, 6),
                stored_args => stored_args,
            },
        ));
    }

    // Configurator methods — positional params so callers can pass objects directly
    // without keyword syntax (e.g. `app.config(ServerConfig.new(...))`).
    for method in &service.configurators {
        let mut params = Vec::new();
        for p in &method.params {
            if p.optional {
                params.push(format!("{} = nil", p.name));
            } else {
                params.push(p.name.clone());
            }
        }
        let param_sig = if params.is_empty() {
            String::new()
        } else {
            format!("({})", params.join(", "))
        };
        let method_name = &method.name;

        let params: Vec<&str> = method.params.iter().map(|p| p.name.as_str()).collect();
        out.push_str(&render(
            "service_rb_configurator.rb.jinja",
            minijinja::context! {
                method_name => method_name,
                param_sig => param_sig,
                doc_comment => format_ruby_comment(&method.doc, 6),
                params => params,
            },
        ));
    }

    // Registration methods accepting blocks
    for reg in &service.registrations {
        gen_registration_method(out, reg, service, api, native_module_name);
    }

    // Lifecycle hook registration methods
    lifecycle_error_ws_sse::gen_lifecycle_hooks_for_class(out, &api.lifecycle_hooks);

    // WebSocket registration
    lifecycle_error_ws_sse::gen_websocket_methods_for_class(out, &api.websocket_routes);

    // SSE registration
    lifecycle_error_ws_sse::gen_sse_methods_for_class(out, &api.sse_routes);

    // Entrypoint methods — positional params for direct invocation.
    for ep in &service.entrypoints {
        let mut params = Vec::new();
        for p in &ep.params {
            if p.optional {
                params.push(format!("{} = nil", p.name));
            } else {
                params.push(p.name.clone());
            }
        }
        let param_sig = if params.is_empty() {
            String::new()
        } else {
            format!("({})", params.join(", "))
        };
        let ep_name = &ep.method;

        match ep.kind {
            EntrypointKind::Run => {
                // Convention: native fn is `{snake_service_name}_{entrypoint_name}`
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                let call_args = ep.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
                out.push_str(&render(
                    "service_rb_entrypoint.rb.jinja",
                    minijinja::context! {
                        method_name => ep_name,
                        param_sig => param_sig,
                        doc_comment => format_ruby_comment(&ep.doc, 6),
                        native_module_name => native_module_name,
                        native_fn => native_fn,
                        call_args => call_args,
                    },
                ));
            }
            EntrypointKind::Finalize => {
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                let call_args = ep.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
                out.push_str(&render(
                    "service_rb_entrypoint.rb.jinja",
                    minijinja::context! {
                        method_name => ep_name,
                        param_sig => param_sig,
                        doc_comment => format_ruby_comment(&ep.doc, 6),
                        native_module_name => native_module_name,
                        native_fn => native_fn,
                        call_args => call_args,
                    },
                ));
            }
        }
    }

    out.push_str("  end\n\n");
}

fn gen_registration_method(
    out: &mut String,
    reg: &RegistrationDef,
    _service: &ServiceDef,
    _api: &ApiSurface,
    _native_module_name: &str,
) {
    let method_name = &reg.method;

    // Build metadata param signature (excluding the callback param)
    let meta_params: Vec<String> = reg
        .metadata_params
        .iter()
        .map(|p| {
            let annotation = ruby_type_annotation(&p.ty);
            if p.optional {
                format!("{}: {} | nil = nil", p.name, annotation)
            } else {
                format!("{}: {}", p.name, annotation)
            }
        })
        .collect();

    // For the main method signature, use positional params (no type annotations)
    let positional_params: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
    let param_sig = if positional_params.is_empty() {
        "(&block)".to_owned()
    } else {
        format!("({}, &block)", positional_params.join(", "))
    };

    // Collect metadata param names for the tuple
    let meta_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
    let meta_tuple = if meta_names.is_empty() {
        "[]".to_owned()
    } else {
        format!("[{}]", meta_names.join(", "))
    };

    out.push_str(&render(
        "service_rb_registration_method.rb.jinja",
        minijinja::context! {
            method_name => method_name,
            param_sig => param_sig,
            doc_comment => format_ruby_comment(&reg.doc, 6),
            meta_tuple => meta_tuple,
        },
    ));

    // Also expose a positional companion `register_{method_name}(meta..., handler)`
    // so harness code and scripts can pass a callable without using Ruby block syntax.
    let direct_name = format!("register_{method_name}");
    if direct_name != *method_name {
        // Use plain positional params only — Ruby forbids positional params after
        // keyword params, so the companion method takes all args positionally.
        let direct_param_sig = if meta_params.is_empty() {
            format!("({callback})", callback = reg.callback_param)
        } else {
            let positional_meta: Vec<String> = reg.metadata_params.iter().map(|p| p.name.clone()).collect();
            format!("({}, {})", positional_meta.join(", "), reg.callback_param)
        };
        out.push_str(&render(
            "service_rb_direct_registration_method.rb.jinja",
            minijinja::context! {
                direct_name => direct_name,
                direct_param_sig => direct_param_sig,
                method_name => method_name,
                meta_tuple => meta_tuple,
                callback => reg.callback_param,
            },
        ));
    }

    // Emit registration variants (shortcuts for common patterns)
    for variant in &reg.variants {
        gen_registration_variant(out, variant, reg);
    }
}

fn gen_registration_variant(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
) {
    let variant_name = &variant.name;
    let _base_method = &base_reg.method;

    // Build the free params (non-fixed) for the variant signature
    let mut free_params_sig = Vec::new();
    for param in &variant.signature_params {
        let annotation = ruby_type_annotation(&param.ty);
        if param.optional {
            free_params_sig.push(format!("{}: {} | nil = nil", param.name, annotation));
        } else {
            free_params_sig.push(format!("{}: {}", param.name, annotation));
        }
    }

    // Build metadata array to pass to the Rust side.
    // For variants with wrapper_call, include the args in declaration order.
    let mut meta_items = Vec::new();
    if let Some(wc) = &variant.wrapper_call {
        for arg in &wc.args {
            match arg {
                crate::core::ir::WrapperConstructorArg::Fixed {
                    param_name: _,
                    value_expr,
                } => {
                    meta_items.push(value_expr.clone());
                }
                crate::core::ir::WrapperConstructorArg::Free { param } => {
                    meta_items.push(param.name.clone());
                }
            }
        }
    } else {
        // For non-wrapper variants, add overridden values
        for override_ in &variant.overrides {
            meta_items.push(override_.value_expr.clone());
        }
    }

    let meta_tuple = if meta_items.is_empty() {
        "[]".to_owned()
    } else {
        format!("[{}]", meta_items.join(", "))
    };

    // In Ruby, blocks are the idiomatic callback mechanism, making all three
    // RegistrationVariantStyle variants semantically equivalent at the API surface.
    // - RegistrationVariantStyle::VerbDecorator: handler as block param
    // - RegistrationVariantStyle::Builder: handler as block (no decorator factory)
    // - RegistrationVariantStyle::Hybrid: handler as block (same as both above)
    // Ruby's unified block form satisfies all three patterns — a block IS a closure
    // that can serve as either a direct callback or a factory result. The style
    // field is preserved in the IR so other backends (e.g., Python) can distinguish
    // between decorator-factory and direct-method forms; Ruby emits one unified form.
    let _ = variant.style; // acknowledged; no Ruby-specific branching needed

    let param_sig = if free_params_sig.is_empty() {
        "(&block)".to_owned()
    } else {
        format!("({}, &block)", free_params_sig.join(", "))
    };

    let doc_comment = if let Some(doc) = &variant.doc {
        format_ruby_comment(doc, 6)
    } else {
        format!("      # Register a handler for the {variant_name} variant.\n")
    };
    out.push_str(&render(
        "service_rb_registration_variant.rb.jinja",
        minijinja::context! {
            variant_name => variant_name,
            param_sig => param_sig,
            doc_comment => doc_comment,
            meta_tuple => meta_tuple,
        },
    ));
}

// ──────────────────────────────────────────────────────────────── Rust glue ──

/// Generate the Magnus Rust glue module (`service.rs`).
///
/// For each service this emits:
/// - A `Rb{ContractName}Bridge` struct that wraps a `Opaque<Value>` callable
///   and `impl`s the handler contract trait, acquiring the GVL (Global VM Lock)
///   to call the proc synchronously with JSON request/response.
/// - A `#[magnus::function]` `{snake_service}_{entrypoint}` that accepts
///   the collected registrations list and any entrypoint params, builds the
///   native service, and drives it.
pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let mut out = String::new();

    // File-level allow attributes to keep clippy happy in generated code
    out.push_str(&render("service_rs_header.rs.jinja", minijinja::context! {}));

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

    // Emit one function per service × entrypoint
    for service in &api.services {
        for ep in &service.entrypoints {
            gen_run_function(&mut out, service, ep, api, &core_import);
        }
    }

    out
}

/// Emit the `Rb{ContractName}Bridge` struct + trait impl.
///
/// The bridge wraps a Ruby proc (stored as `Opaque<Value>`) and implements
/// the handler contract trait. It acquires the GVL via `rb_sys::rb_thread_call_with_gvl()`
/// (when called from a non-GVL tokio worker) or directly when already on a Ruby thread.
/// The proc is called with JSON request/response serialization.
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Rb{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;

    // Determine wire types — use plain serde_json::Value, not re-exported from core
    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

    // Special handling: if the wire type includes the core import prefix, strip it
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

    // Trait impl — build the request and response paths using core_import
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

    // Returns a boxed future directly (canonical object-safe async-trait shape)
    // rather than via the async_trait macro, matching a contract whose dispatch
    // method is hand-written as `-> Pin<Box<dyn Future<..> + Send + '_>>`.
    out.push_str(&render(
        "service_rs_handler_bridge.rs.jinja",
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

    // Emit the helper function that safely calls a Ruby proc with the GVL acquired.
    out.push_str(&render(
        "service_rs_ruby_proc_gvl_helpers.rs.jinja",
        minijinja::context! {},
    ));
}

/// Emit a match arm for a registration variant shortcut (e.g., "get", "post").
///
/// The variant may have a wrapper constructor call (e.g., `RouteBuilder::new(method, path)`)
/// with fixed and free arguments. Free arguments are extracted from the metadata array,
/// fixed arguments are verbatim Rust expressions.
fn gen_variant_match_arm(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
    contract_name: &str,
    bridge_name: &str,
    core_import: &str,
    api: &ApiSurface,
) {
    let variant_name = &variant.name;
    let base_method = &base_reg.method;

    out.push_str(&render(
        "service_rs_variant_match_arm_header.rs.jinja",
        minijinja::context! {
            variant_name => variant_name,
            bridge_name => bridge_name,
            core_import => core_import,
            contract_name => contract_name,
        },
    ));

    // Extract metadata and build wrapper constructor call if needed
    if let Some(wc) = &variant.wrapper_call {
        // Extract free params from metadata array
        let mut free_params = Vec::new();
        for arg in &wc.args {
            if let crate::core::ir::WrapperConstructorArg::Free { param } = arg {
                free_params.push(param.clone());
            }
        }

        if !free_params.is_empty() {
            out.push_str(&render(
                "service_rs_meta_array_extract.rs.jinja",
                minijinja::context! {},
            ));

            for (i, param) in free_params.iter().enumerate() {
                render_metadata_param_extract(out, param, i, core_import, api);
            }
        }

        // Build constructor call with fixed and free args
        let mut call_args = Vec::new();
        for arg in &wc.args {
            match arg {
                crate::core::ir::WrapperConstructorArg::Fixed {
                    param_name: _,
                    value_expr,
                } => {
                    call_args.push(value_expr.clone());
                }
                crate::core::ir::WrapperConstructorArg::Free { param } => {
                    call_args.push(param.name.clone());
                }
            }
        }
        let call_expr = format!(
            "{}::{}({})",
            wc.wrapper_type_path,
            wc.constructor_method,
            call_args.join(", ")
        );
        out.push_str(&render(
            "service_rs_wrapper_owner_call.rs.jinja",
            minijinja::context! {
                metadata_param => &wc.metadata_param,
                wrapper_type_path => &wc.wrapper_type_path,
                call_expr => &call_expr,
                base_method => base_method,
            },
        ));
    } else {
        // No wrapper call; use override values directly
        // For now, just call the base method with handler only
        out.push_str(&render(
            "service_rs_owner_call.rs.jinja",
            minijinja::context! {
                method_name => base_method,
                args => "",
                fallible => base_reg.error_type.is_some(),
            },
        ));
    }

    // Handle error if the registration is fallible
    if variant.wrapper_call.is_some() && base_reg.error_type.is_some() {
        out.push_str(
            "\n                    .map_err(|e| magnus::Error::new(ruby.exception_runtime_error(), e.to_string()))?;\n",
        );
    } else if variant.wrapper_call.is_some() {
        out.push_str(";\n");
    }
    out.push_str("            }\n");
}

fn render_metadata_param_extract(
    out: &mut String,
    param: &crate::core::ir::ParamDef,
    index: usize,
    core_import: &str,
    api: &ApiSurface,
) {
    let rust_ty = typeref_to_rust_type(&param.ty, core_import);
    match &param.ty {
        TypeRef::String | TypeRef::Char => out.push_str(&render(
            "service_rs_metadata_extract_entry.rs.jinja",
            minijinja::context! {
                param_name => &param.name,
                rust_ty => &rust_ty,
                extract_ty => "String",
                index => index as isize,
            },
        )),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            let extract_ty = match p {
                PrimitiveType::Bool => "bool",
                PrimitiveType::F32 | PrimitiveType::F64 => "f64",
                _ => "i64",
            };
            out.push_str(&render(
                "service_rs_metadata_extract_entry.rs.jinja",
                minijinja::context! {
                    param_name => &param.name,
                    rust_ty => &rust_ty,
                    extract_ty => extract_ty,
                    index => index as isize,
                },
            ));
        }
        TypeRef::Named(n) if is_variant_wrapper_type(api, n) => {
            let clone_expr = format!("{}.inner.as_ref().clone()", param.name);
            out.push_str(&render(
                "service_rs_metadata_extract_try_convert.rs.jinja",
                minijinja::context! {
                    param_name => &param.name,
                    binding_ty => format!("&crate::{n}"),
                    index => index as isize,
                    clone_expr => clone_expr,
                },
            ));
        }
        TypeRef::Named(_) => {
            let clone_expr = format!("(*{}).clone()", param.name);
            out.push_str(&render(
                "service_rs_metadata_extract_try_convert.rs.jinja",
                minijinja::context! {
                    param_name => &param.name,
                    binding_ty => format!("&{rust_ty}"),
                    index => index as isize,
                    clone_expr => clone_expr,
                },
            ));
        }
        _ => out.push_str(&render(
            "service_rs_metadata_extract_try_convert.rs.jinja",
            minijinja::context! {
                param_name => &param.name,
                binding_ty => &rust_ty,
                index => index as isize,
                clone_expr => "",
            },
        )),
    }
}

/// Check if a named type is a variant wrapper by looking up in the API surface.
fn is_variant_wrapper_type(api: &ApiSurface, type_name: &str) -> bool {
    api.types.iter().any(|t| t.name == type_name && t.is_variant_wrapper)
}

/// Emit the `#[magnus::function]` entry point for one service × entrypoint.
///
/// The function:
/// 1. Accepts the registrations array (each entry is `[method_name, metadata_array, proc]`).
/// 2. Constructs the native service owner via its constructor as a mutable owned value.
/// 3. Iterates registrations (within GVL), wraps each proc in the appropriate bridge,
///    and calls the owner's registration method with the handler trait object.
/// 4. Releases GVL and calls the owner's entrypoint (blocking if sync, driving async via block_on).
fn gen_run_function(
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

    // Build parameter list: registrations + entrypoint params (no &Ruby - use Ruby::get())
    let mut fn_params = vec!["registrations: Value".to_owned()];
    for p in &ep.params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        fn_params.push(format!("{}: {}", p.name, rust_ty));
    }
    let fn_param_sig = fn_params.join(", ");

    // Build the owner instance as a mutable owned value (no Arc<Mutex<…>> wrapping)
    let ctor_call = build_ctor_call(service, owner_path, core_import);
    out.push_str(&render(
        "service_rs_run_function_header.rs.jinja",
        minijinja::context! {
            owner_path => owner_path,
            ep_method => ep_method,
            fn_name => fn_name,
            fn_param_sig => fn_param_sig,
            ctor_call => ctor_call,
        },
    ));
    for reg in &service.registrations {
        let reg_method = &reg.method;
        let contract_name = &reg.callback_contract;

        if let Some(contract) = find_contract(api, contract_name) {
            let bridge_name = format!("Rb{}Bridge", contract.trait_name.to_upper_camel_case());
            let meta_count = reg.metadata_params.len();

            out.push_str(&render(
                "service_rs_registration_match_arm_header.rs.jinja",
                minijinja::context! {
                    reg_method => reg_method,
                    bridge_name => bridge_name,
                    core_import => core_import,
                    contract_name => contract_name,
                },
            ));

            if meta_count > 0 {
                out.push_str(&render(
                    "service_rs_meta_array_extract.rs.jinja",
                    minijinja::context! {},
                ));

                for (i, meta_param) in reg.metadata_params.iter().enumerate() {
                    render_metadata_param_extract(out, meta_param, i, core_import, api);
                }

                let meta_args: Vec<String> = reg.metadata_params.iter().map(|p| p.name.clone()).collect();
                out.push_str(&render(
                    "service_rs_owner_call.rs.jinja",
                    minijinja::context! {
                        method_name => reg_method,
                        args => meta_args.join(", "),
                        fallible => reg.error_type.is_some(),
                    },
                ));
            } else {
                out.push_str(&render(
                    "service_rs_owner_call.rs.jinja",
                    minijinja::context! {
                        method_name => reg_method,
                        args => "",
                        fallible => reg.error_type.is_some(),
                    },
                ));
            }
            out.push_str("            }\n");

            // Emit match arms for variants
            for variant in &reg.variants {
                gen_variant_match_arm(&mut *out, variant, reg, contract_name, &bridge_name, core_import, api);
            }
        }
    }
    // Call the entrypoint on the owned, registered owner
    let ep_call = build_ep_call(ep, service, core_import);
    out.push_str(&render(
        "service_rs_run_function_footer.rs.jinja",
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
        // For now, zero-arg constructor. Can be extended to thread constructor params.
        format!("{owner_path}::{}()", service.constructor.name)
    }
}

/// Build the entrypoint invocation for a service method.
///
/// For async entrypoints that consume `self` (e.g., `run(self)`), we release the
/// GVL via `rb_thread_call_without_gvl` and drive a `new_current_thread` Tokio
/// runtime on the same OS thread. This allows the handler bridge to call
/// `rb_thread_call_with_gvl` from within the runtime's tasks.
///
/// Using `Handle::current().block_on()` fails because no Tokio reactor exists on
/// the Ruby main thread. Using `spawn_blocking` would create a non-Ruby OS thread.
fn build_ep_call(ep: &crate::core::ir::EntrypointDef, service: &ServiceDef, _core_import: &str) -> String {
    let ep_method = &ep.method;
    let ep_args: Vec<String> = ep.params.iter().map(|p| p.name.clone()).collect();
    let args_str = ep_args.join(", ");
    let owner_path = &service.rust_path;
    let fn_name = format!("{}_run", service.name.to_snake_case());
    // Bind non-Unit returns to `_` so the unwrapped value (after `?`-propagation) doesn't
    // trigger `unused_must_use` for `Result`-returning entrypoints like `into_router`.
    let bind = if matches!(ep.return_type, TypeRef::Unit) {
        ""
    } else {
        "let _ = "
    };

    if ep.is_async {
        // Release the GVL and run a current-thread Tokio runtime for the async entrypoint.
        // SAFETY: called on a Ruby thread (GVL held); `rb_thread_call_without_gvl` releases
        // it and invokes the callback on the SAME OS thread, making `rb_thread_call_with_gvl`
        // valid from within the callback's Tokio tasks.
        render(
            "service_rs_async_entrypoint_call.rs.jinja",
            minijinja::context! {
                fn_name => fn_name,
                owner_path => owner_path,
                ep_method => ep_method,
                args_str => args_str,
            },
        )
    } else if ep.error_type.is_some() {
        format!(
            "    {bind}owner.{ep_method}({args_str})\n        \
             .map_err(|e| magnus::Error::new(ruby.exception_runtime_error(), e.to_string()))?;\n"
        )
    } else {
        format!("    {bind}owner.{ep_method}({args_str});\n")
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

/// Generate all service-API files for the Magnus backend.
///
/// Returns up to two `GeneratedFile`s per non-empty service list:
/// - `{output_dir}/service.rs`   — Magnus Rust glue
/// - `{ruby_pkg}/service.rb`     — idiomatic Ruby class
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(config.output_paths.get("ruby"), &config.name, "crates/{name}-rb/src/");
    // Convert crate name to PascalCase module name (same as in mod.rs)
    let native_module_name = api.crate_name.to_upper_camel_case();

    // Rust glue
    let service_rs = gen_service_rs(api, config);

    // Ruby wrapper
    let gem_name = config.ruby_gem_name();
    let gem_name_snake = gem_name.replace('-', "_");
    let service_rb = gen_service_rb(api, &native_module_name, &gem_name_snake);

    // Ruby lib output base: service.rb is runtime code and must live under lib/,
    // not under sig/ (which is for type stubs loaded only by steep/sorbet).
    // Mirror the path used by the public_api generator in mod.rs.
    let lib_dir = resolve_output_dir(config.output_paths.get("ruby_lib"), &config.name, "packages/ruby/lib/");
    let output_base = PathBuf::from(&lib_dir).join(&gem_name_snake);

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: output_base.join("service.rb"),
            content: service_rb,
            generated_header: true,
        },
    ])
}

// ───────────────────────── Phase-C emission stubs (new IR sections) ──────────

/// Emit Magnus/Ruby lifecycle-hook registration methods.
///
/// Stub: walks the collection, logs once when non-empty, returns `""`.
/// Replace this body with Jinja-driven generation in the Magnus Phase-C pass.
pub(super) fn emit_lifecycle_hooks(hooks: &[crate::core::ir::LifecycleHookDef]) -> String {
    if hooks.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "lifecycle hook emission not implemented for magnus ({} hooks)",
        hooks.len()
    );
    for _hook in hooks {}
    String::new()
}

/// Emit Magnus/Ruby WebSocket route registration methods.
///
/// Stub — returns `""` until the Magnus Phase-C specialist implements
/// `app.websocket(path) { |socket| … }` generation.
pub(super) fn emit_websocket_routes(routes: &[crate::core::ir::WebSocketRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "WebSocket route emission not implemented for magnus ({} routes)",
        routes.len()
    );
    for _route in routes {}
    String::new()
}

/// Emit Magnus/Ruby SSE route registration methods.
///
/// Stub — returns `""` until the Magnus Phase-C specialist implements
/// `app.sse(path) { … }` generation.
pub(super) fn emit_sse_routes(routes: &[crate::core::ir::SseRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "SSE route emission not implemented for magnus ({} routes)",
        routes.len()
    );
    for _route in routes {}
    String::new()
}

/// Emit Magnus/Ruby native error classes.
///
/// Stub — returns `""` until the Magnus Phase-C specialist implements
/// Ruby `StandardError` subclass generation.
pub(super) fn emit_error_types(types: &[crate::core::ir::ErrorTypeDef]) -> String {
    if types.is_empty() {
        return String::new();
    }
    tracing::debug!("error type emission not implemented for magnus ({} types)", types.len());
    for _ty in types {}
    String::new()
}

/// Aggregate stub — forwards all four new IR sections for the Magnus backend.
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
