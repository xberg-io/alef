//! Service-API codegen for the NAPI-RS (Node.js/TypeScript) backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **`service.rs`** — Rust napi glue that wraps each registered JavaScript
//!    callable as `Arc<dyn <HandlerContractDef::trait_name>>` via an async
//!    callback bridge using ThreadsafeFunction, builds the core service via the
//!    owner type's registration and run entrypoints, and exposes a `#[napi]`
//!    entry point.
//!
//! 2. **`service.ts`** — An idiomatic TypeScript class mirroring the service's
//!    constructor, configurator methods, and registration methods (supporting
//!    both decorator and direct-register duality), with a `run(...)`/entrypoint
//!    that delegates to the native function.
//!
//! All names are derived entirely from the [`ApiSurface`] IR — no transport-
//! or domain-specific assumptions are made anywhere in this module.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, ServiceDef, TypeRef};
use heck::{ToSnakeCase, ToUpperCamelCase};
use minijinja::context;
use std::path::PathBuf;

use crate::backends::napi::template_env::render;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Convert a `TypeRef` to a TypeScript type annotation string.
fn typescript_type_annotation(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "string".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "boolean".to_owned(),
                PrimitiveType::F32 | PrimitiveType::F64 => "number".to_owned(),
                _ => "number".to_owned(),
            }
        }
        TypeRef::Bytes => "Buffer".to_owned(),
        TypeRef::Optional(inner) => format!("{} | undefined", typescript_type_annotation(inner)),
        TypeRef::Vec(inner) => format!("{}[]", typescript_type_annotation(inner)),
        TypeRef::Map(k, v) => format!(
            "Record<{}, {}>",
            typescript_type_annotation(k),
            typescript_type_annotation(v)
        ),
        TypeRef::Unit => "void".to_owned(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Json => "any".to_owned(),
        TypeRef::Path => "string".to_owned(),
        TypeRef::Duration => "number".to_owned(),
    }
}

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

// ──────────────────────────────────────────────────────────── TypeScript output ──

/// Generate the idiomatic TypeScript service class (`service.ts`).
///
/// Produces a TypeScript module containing one class per service. Each class
/// exposes:
/// - A constructor mirroring [`ServiceDef::constructor`].
/// - Configurator methods from [`ServiceDef::configurators`].
/// - Registration methods from [`ServiceDef::registrations`] supporting both
///   decorator and direct-register patterns.
/// - A `run(...)` method derived from the first [`EntrypointKind::Run`]
///   entrypoint.
pub(super) fn gen_service_ts(api: &ApiSurface, native_module: &str, config: &ResolvedCrateConfig) -> String {
    let mut out = String::new();

    // Build type imports for the preamble template
    let mut imports = vec!["JsObject".to_owned()];
    for contract in &api.handler_contracts {
        // We'll import the wire DTO types for type annotations
        if let Some(req_ty) = &contract.wire_request_type {
            imports.push(req_ty.clone());
        }
        if let Some(resp_ty) = &contract.wire_response_type {
            imports.push(resp_ty.clone());
        }
    }
    // Add service constructor and configurator param types
    if let Some(service) = api.services.first() {
        for param in &service.constructor.params {
            if let TypeRef::Named(name) = &param.ty {
                imports.push(name.clone());
            }
        }
        for method in &service.configurators {
            for param in &method.params {
                if let TypeRef::Named(name) = &param.ty {
                    imports.push(name.clone());
                }
            }
        }
        // Add registration variant wrapper types, metadata param types, and fixed enum types
        for reg in &service.registrations {
            for variant in &reg.variants {
                if let Some(wrapper_call) = &variant.wrapper_call {
                    imports.push(wrapper_call.wrapper_type_name.clone());
                    // Extract enum types from Fixed args (e.g., "mycrate::Method::Get" → "Method")
                    for arg in &wrapper_call.args {
                        if let crate::core::ir::WrapperConstructorArg::Fixed {
                            param_name: _,
                            value_expr,
                        } = arg
                        {
                            // Split on :: and take the second-to-last part (the type name).
                            // "mycrate::Method::Get" → ["mycrate", "Method", "Get"] → "Method"
                            let parts: Vec<&str> = value_expr.split("::").collect();
                            if parts.len() >= 2 {
                                imports.push(parts[parts.len() - 2].to_string());
                            }
                        }
                    }
                }
                for param in &variant.signature_params {
                    if let TypeRef::Named(name) = &param.ty {
                        imports.push(name.clone());
                    }
                }
            }
        }
    }
    // Remove duplicates
    imports.sort();
    imports.dedup();

    let type_imports = imports.join(", ");

    // Only import the native function if the entrypoint method is not excluded.
    // Check if any entrypoint (typically `run`) is excluded for this service.
    let service_name = &api.services[0].name;
    let has_non_excluded_entrypoint = api.services[0].entrypoints.iter().any(|ep| {
        let method_key = format!("{}.{}", service_name, ep.method);
        !config.exclude.methods.contains(&method_key)
    });

    if has_non_excluded_entrypoint {
        let native_import = format!("{}_run", service_name.to_snake_case());
        out.push_str(&render(
            "service_ts_preamble.jinja",
            context! {
                type_imports,
                native_import,
            },
        ));
    } else {
        out.push_str(&render(
            "service_ts_preamble.jinja",
            context! {
                type_imports,
                native_import => "",
            },
        ));
    }

    for service in &api.services {
        gen_service_class_ts(&mut out, service, api, native_module, config);
    }

    out
}

fn gen_service_class_ts(
    out: &mut String,
    service: &ServiceDef,
    api: &ApiSurface,
    _native_module: &str,
    config: &ResolvedCrateConfig,
) {
    let class_name = &service.name;

    // Class docstring
    let class_doc = if service.doc.is_empty() {
        String::new()
    } else {
        service.doc.trim().replace('\n', "\n * ")
    };
    out.push_str(&render(
        "service_ts_class_header.jinja",
        context! {
            class_doc,
            class_name,
        },
    ));

    // Static factory method for Node.js binding compatibility
    {
        let ctor = &service.constructor;
        let mut params = Vec::new();
        for p in &ctor.params {
            let ty = typescript_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("{}: {} = undefined", p.name, ty));
            } else {
                params.push(format!("{}: {}", p.name, ty));
            }
        }

        let param_sig = params.join(", ");
        let args = ctor
            .params
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&render(
            "service_ts_static_new.jinja",
            context! {
                class_name,
                param_sig,
                args,
            },
        ));
    }

    // Constructor
    {
        let ctor = &service.constructor;
        let mut params = Vec::new();
        for p in &ctor.params {
            let ty = typescript_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("{}: {} = undefined", p.name, ty));
            } else {
                params.push(format!("{}: {}", p.name, ty));
            }
        }

        let param_sig = params.join(", ");
        let doc = ctor.doc.trim().replace('\n', "\n   * ");
        out.push_str(&render(
            "service_ts_constructor.jinja",
            context! {
                doc,
                param_sig,
            },
        ));
    }

    // Configurator methods
    for method in &service.configurators {
        let mut params = Vec::new();
        for p in &method.params {
            let ty = typescript_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("{}: {} = undefined", p.name, ty));
            } else {
                params.push(format!("{}: {}", p.name, ty));
            }
        }

        let param_sig = params.join(", ");
        let method_name = &method.name;
        let doc = method.doc.trim().replace('\n', "\n   * ");
        out.push_str(&render(
            "service_ts_configurator.jinja",
            context! {
                doc,
                method_name,
                param_sig,
            },
        ));
    }

    // Registration methods: support both decorator and direct patterns
    for reg in &service.registrations {
        gen_registration_method_ts(out, reg, service, api);
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        let method_key = format!("{}.{}", class_name, ep.method);
        let is_excluded = config.exclude.methods.contains(&method_key);

        // Skip generating entrypoint methods that are excluded from bindings
        if is_excluded {
            continue;
        }

        let mut params = Vec::new();
        for p in &ep.params {
            let ty = typescript_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("{}: {} = undefined", p.name, ty));
            } else {
                params.push(format!("{}: {}", p.name, ty));
            }
        }

        let param_sig = params.join(", ");
        let ep_name = &ep.method;

        let doc = ep.doc.trim().replace('\n', "\n   * ");
        let native_fn = format!("{}_{}", class_name.to_snake_case(), ep_name);
        let native_args = ep.params.iter().map(|p| format!(", {}", p.name)).collect::<String>();

        match ep.kind {
            EntrypointKind::Run => {
                out.push_str(&render(
                    "service_ts_entrypoint_run.jinja",
                    context! {
                        doc,
                        ep_name,
                        param_sig,
                        native_fn,
                        native_args,
                    },
                ));
            }
            EntrypointKind::Finalize => {
                let return_ty = typescript_type_annotation(&ep.return_type);
                out.push_str(&render(
                    "service_ts_entrypoint_finalize.jinja",
                    context! {
                        doc,
                        ep_name,
                        param_sig,
                        return_ty,
                        native_fn,
                        native_args,
                    },
                ));
            }
        }
    }

    while out.ends_with("\n\n") {
        out.pop();
    }
    out.push_str("}\n");
}

fn gen_registration_method_ts(out: &mut String, reg: &RegistrationDef, service: &ServiceDef, _api: &ApiSurface) {
    let method_name = &reg.method;
    let _class_name = &service.name;

    // Build metadata param signature (excluding the callback param)
    let mut meta_params: Vec<String> = reg
        .metadata_params
        .iter()
        .map(|p| {
            let ty = typescript_type_annotation(&p.ty);
            if p.optional {
                format!("{}: {} = undefined", p.name, ty)
            } else {
                format!("{}: {}", p.name, ty)
            }
        })
        .collect();

    // Decorator-factory form: supports @app.register(meta1, meta2) decorator syntax
    let meta_sig = meta_params.join(", ");

    // Closure that collects metadata and the callback
    let meta_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
    let meta_array = if meta_names.is_empty() {
        "[]".to_owned()
    } else {
        format!("[{}]", meta_names.join(", "))
    };

    let doc = reg.doc.trim().replace('\n', "\n   * ");
    out.push_str(&render(
        "service_ts_registration_method.jinja",
        context! {
            doc,
            method_name,
            meta_sig,
            meta_array,
        },
    ));

    // Also expose a direct (non-decorator) register variant
    let direct_name = format!("register_{method_name}");
    if direct_name != *method_name {
        meta_params.push("handler: (...args: any[]) => any".to_string());
        let full_sig = meta_params.join(", ");
        out.push_str(&render(
            "service_ts_registration_direct_method.jinja",
            context! {
                method_name,
                direct_name,
                full_sig,
                meta_array,
            },
        ));
    }

    // Emit registration variants (shortcut methods)
    for variant in &reg.variants {
        gen_registration_variant_method_ts(out, variant, reg, service);
    }
}

/// Emit a TypeScript shortcut method for one registration variant.
///
/// The emission style depends on [`RegistrationVariant::style`]:
/// - [`RegistrationVariantStyle::VerbDecorator`] — only the direct method form
///   (`app.get(path, handler)` returning `this` for chaining).
/// - [`RegistrationVariantStyle::Builder`] — only the decorator-factory form
///   (`app.get(path)` returning a function that accepts the handler).
/// - [`RegistrationVariantStyle::Hybrid`] — both forms (overloaded).
fn gen_registration_variant_method_ts(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    reg: &RegistrationDef,
    _service: &ServiceDef,
) {
    use crate::core::ir::RegistrationVariantStyle;

    let variant_name = &variant.name;
    let base_method = &reg.method;

    // Build signature from variant's signature_params (without handler)
    let variant_params_no_handler: Vec<String> = variant
        .signature_params
        .iter()
        .map(|p| {
            let ty = typescript_type_annotation(&p.ty);
            if p.optional {
                format!("{}: {} = undefined", p.name, ty)
            } else {
                format!("{}: {}", p.name, ty)
            }
        })
        .collect();

    // Metadata array (shared by both forms)
    let metadata_array = if let Some(wrapper_call) = &variant.wrapper_call {
        let wrapper_type = &wrapper_call.wrapper_type_name;

        // Build the constructor args by substituting Fixed args and pulling Free args
        let mut ctor_args = Vec::new();
        for arg in &wrapper_call.args {
            match arg {
                crate::core::ir::WrapperConstructorArg::Fixed {
                    param_name: _,
                    value_expr,
                } => {
                    // Fixed args are Rust value expressions like "mycrate::Method::Get".
                    // Extract the type and variant for TypeScript: "mycrate::Method::Get" → "Method.Get"
                    let parts: Vec<&str> = value_expr.split("::").collect();
                    let ts_expr = if parts.len() >= 2 {
                        format!("{}.{}", parts[parts.len() - 2], parts[parts.len() - 1])
                    } else {
                        value_expr.clone()
                    };
                    ctor_args.push(ts_expr);
                }
                crate::core::ir::WrapperConstructorArg::Free { param } => {
                    // Free args come from the variant's signature params
                    ctor_args.push(param.name.clone());
                }
            }
        }
        let ctor_arg_str = ctor_args.join(", ");
        let metadata_param = &wrapper_call.metadata_param;

        // Return a tuple: (wrapper construction code, metadata array expression)
        let wrapper_code = format!("    const {metadata_param} = new {wrapper_type}({ctor_arg_str});\n");
        (wrapper_code, format!("[{metadata_param}]"))
    } else {
        // No wrapper constructor: build metadata array from variant params
        let mut metadata_values = Vec::new();
        for param in &variant.signature_params {
            metadata_values.push(param.name.clone());
        }

        let metadata_expr = if metadata_values.is_empty() {
            "[]".to_owned()
        } else {
            format!("[{}]", metadata_values.join(", "))
        };
        ("".to_owned(), metadata_expr)
    };

    match variant.style {
        RegistrationVariantStyle::VerbDecorator => {
            // Direct method form only: `app.get(path, handler): this`
            emit_variant_direct_method(
                out,
                variant_name,
                &variant_params_no_handler,
                base_method,
                &metadata_array.0,
                &metadata_array.1,
                variant,
            );
        }
        RegistrationVariantStyle::Builder => {
            // Decorator-factory form only: `app.get(path): (handler) => any`
            emit_variant_decorator_factory(
                out,
                variant_name,
                &variant_params_no_handler,
                base_method,
                &metadata_array.0,
                &metadata_array.1,
                variant,
            );
        }
        RegistrationVariantStyle::Hybrid => {
            // Both forms — emit as TypeScript method overloads (two declaration
            // signatures + one implementation that branches on the optional
            // `handler` argument). Emitting them as two separate method bodies
            // produced `Identifier 'get' has already been declared` oxlint
            // errors because JavaScript classes do not support runtime method
            // overloading the way Rust traits do.
            emit_variant_hybrid_overloaded(
                out,
                variant_name,
                &variant_params_no_handler,
                base_method,
                &metadata_array.0,
                &metadata_array.1,
                variant,
            );
        }
    }
}

/// Emit the direct method form for a registration variant: `app.get(path, handler): this`.
fn emit_variant_direct_method(
    out: &mut String,
    variant_name: &str,
    variant_params: &[String],
    base_method: &str,
    wrapper_code: &str,
    metadata_array: &str,
    variant: &crate::core::ir::RegistrationVariant,
) {
    let mut full_params = variant_params.to_vec();
    full_params.push("handler: (...args: any[]) => any".to_string());
    let full_sig = full_params.join(", ");

    let doc = variant
        .doc
        .as_deref()
        .map(|doc| doc.trim().replace('\n', "\n   * "))
        .unwrap_or_else(|| format!("Register a {variant_name} callback directly."));
    out.push_str(&render(
        "service_ts_variant_direct.jinja",
        context! {
            doc,
            variant_name,
            full_sig,
            wrapper_code,
            base_method,
            metadata_array,
        },
    ));
}

/// Emit the decorator-factory form for a registration variant: `app.get(path): (handler) => any`.
fn emit_variant_decorator_factory(
    out: &mut String,
    variant_name: &str,
    variant_params: &[String],
    base_method: &str,
    wrapper_code: &str,
    metadata_array: &str,
    variant: &crate::core::ir::RegistrationVariant,
) {
    let sig = variant_params.join(", ");

    let doc = variant
        .doc
        .as_deref()
        .map(|doc| doc.trim().replace('\n', "\n   * "))
        .unwrap_or_else(|| format!("Register a {variant_name} callback via decorator factory."));
    out.push_str(&render(
        "service_ts_variant_decorator.jinja",
        context! {
            doc,
            variant_name,
            sig,
            wrapper_code,
            base_method,
            metadata_array,
        },
    ));
}

/// Emit BOTH hybrid forms as TypeScript method overloads — two signature
/// declarations and one implementation body that branches on whether the
/// optional `handler` argument was supplied.
fn emit_variant_hybrid_overloaded(
    out: &mut String,
    variant_name: &str,
    variant_params: &[String],
    base_method: &str,
    wrapper_code: &str,
    metadata_array: &str,
    variant: &crate::core::ir::RegistrationVariant,
) {
    let direct_params = {
        let mut p = variant_params.to_vec();
        p.push("handler: (...args: any[]) => any".to_string());
        p.join(", ")
    };
    let factory_params = variant_params.join(", ");
    let impl_params = {
        let mut p = variant_params.to_vec();
        p.push("handler?: (...args: any[]) => any".to_string());
        p.join(", ")
    };

    let doc = variant
        .doc
        .as_deref()
        .map(|doc| doc.trim().replace('\n', "\n   * "))
        .unwrap_or_else(|| {
            format!(
                "Register a {variant_name} callback. Call with `handler` for direct registration; omit\n   * `handler` to receive a decorator factory that accepts it lazily."
            )
        });
    out.push_str(&render(
        "service_ts_variant_hybrid.jinja",
        context! {
            doc,
            variant_name,
            direct_params,
            factory_params,
            impl_params,
            wrapper_code,
            base_method,
            metadata_array,
        },
    ));
}

// ──────────────────────────────────────────────────────────────── Rust glue ──

/// Generate the Rust napi glue module (`service.rs`).
///
/// For each service this emits:
/// - A `{ContractName}Bridge` struct that wraps a `ThreadsafeFunction` callable
///   and implements the handler contract trait, using NAPI's ThreadsafeFunction
///   to call JavaScript async callables from Rust async code.
/// - A `#[napi]` `{snake_service}_{entrypoint}` function that accepts the
///   collected registrations list and any entrypoint params, builds the native
///   service, and drives it.
pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
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

    // Emit one napi function per service × entrypoint (skip if excluded)
    for service in &api.services {
        for ep in &service.entrypoints {
            let method_key = format!("{}.{}", service.name, ep.method);
            if config.exclude.methods.contains(&method_key) {
                continue;
            }
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

        // Emit variant methods (per-verb registration shortcuts)
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
    api: &ApiSurface,
    core_import: &str,
) {
    let service_snake = service.name.to_snake_case();
    let fn_name = format!("{service_snake}_{}", ep.method);
    let owner_path = &service.rust_path;
    let ep_method = &ep.method;

    // Build the function signature
    let mut rust_params = vec![
        "registrations: Vec<(String, Vec<serde_json::Value>, ThreadsafeFunction<serde_json::Value, serde_json::Value>)>".to_owned(),
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

    // Iterate registrations and dispatch
    out.push_str("    for (method_name, _metadata, handler_fn) in registrations {\n");
    out.push_str("        match method_name.as_str() {\n");

    for reg in &service.registrations {
        let reg_method = &reg.method;
        let contract_name = &reg.callback_contract;

        if let Some(contract) = find_contract(api, contract_name) {
            // Check if any metadata param is opaque. If so, skip this registration
            // in the generic app_run function since opaque types cannot be serialized
            // from JavaScript. Verb shortcuts (get, post, etc.) use wrapper_call to
            // construct opaque params, so they handle the registration differently.
            let has_opaque_metadata = reg.metadata_params.iter().any(|p| {
                if let TypeRef::Named(n) = &p.ty {
                    api.types
                        .iter()
                        .find(|t| &t.name == n && !t.is_trait && t.is_opaque)
                        .is_some()
                } else {
                    false
                }
            });

            if has_opaque_metadata {
                // Skip generic registration for this method in app_run.
                // Verb variants (which use wrapper_call to construct opaque params)
                // are sufficient for the binding-side API.
                continue;
            }

            let bridge_name = format!("{}Bridge", contract.trait_name.to_upper_camel_case());

            out.push_str(&render(
                "service_rs_registration_arm_header.jinja",
                context! {
                    reg_method,
                    bridge_name,
                    core_import,
                    contract_name,
                },
            ));

            // Extract and convert metadata params from serde_json::Value entries
            if !reg.metadata_params.is_empty() {
                for (idx, param) in reg.metadata_params.iter().enumerate() {
                    let param_name = &param.name;
                    let rust_ty = typeref_to_rust_type(&param.ty, core_import);
                    let extraction = gen_metadata_extraction(&param.ty, core_import, api);
                    out.push_str(&render(
                        "service_rs_metadata_binding.jinja",
                        context! {
                            param_name,
                            rust_ty,
                            idx,
                            extraction,
                        },
                    ));
                }
                let meta_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
                let meta_args = meta_names.join(", ");
                out.push_str(&render(
                    "service_rs_registration_call.jinja",
                    context! {
                        reg_method,
                        meta_args,
                        has_meta => true,
                    },
                ));
            } else {
                out.push_str(&render(
                    "service_rs_registration_call.jinja",
                    context! {
                        reg_method,
                        meta_args => "",
                        has_meta => false,
                    },
                ));
            }

            // Handle error if the registration is fallible
            if reg.error_type.is_some() {
                out.push_str(
                    "                    .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;\n",
                );
            } else {
                out.push_str("                    ;\n");
            }
            out.push_str("            }\n");
        }
    }

    out.push_str("            _ => {}\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");

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

    if ep.is_async {
        // Drive the async entrypoint directly (this function is already async)
        format!(
            "    owner.{ep_method}({args_str})\n        \
             .await\n        \
             .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;\n"
        )
    } else {
        if ep.error_type.is_some() {
            format!(
                "    owner.{ep_method}({args_str})\n        \
                 .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;\n"
            )
        } else {
            format!("    owner.{ep_method}({args_str});\n")
        }
    }
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
    rust_params.push("handler: ThreadsafeFunction<serde_json::Value, serde_json::Value>".to_string());
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

    // Run/Finalize entrypoints conventionally consume `self` by value, so we move
    // the owner out of the lock with `std::mem::take` (requires the owner type to
    // implement `Default`) and drop the guard before any `.await`.
    if inner_accessor == "self" {
        if ep.is_async {
            out.push_str(&render(
                "service_rs_entrypoint_call.jinja",
                context! {
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

/// Generate code to extract and convert a metadata parameter from a `serde_json::Value`.
///
/// Returns a Rust expression that converts `val` (a `&serde_json::Value`) to the target type.
/// The generated code uses serde_json's native coercion and type conversions.
#[allow(clippy::only_used_in_recursion)]
fn gen_metadata_extraction(ty: &TypeRef, core_import: &str, api: &ApiSurface) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => {
            "val.as_str().ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"expected string metadata\"))?.to_owned()".to_owned()
        }
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => {
                    "val.as_bool().ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"expected bool metadata\"))?".to_owned()
                }
                PrimitiveType::F64 => {
                    "val.as_f64().ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"expected number metadata\"))?".to_owned()
                }
                PrimitiveType::F32 => {
                    "val.as_f64().ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"expected number metadata\"))? as f32".to_owned()
                }
                PrimitiveType::U8 => {
                    "val.as_u64().ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"expected number metadata\"))? as u8".to_owned()
                }
                PrimitiveType::U16 => {
                    "val.as_u64().ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"expected number metadata\"))? as u16".to_owned()
                }
                PrimitiveType::U32 => {
                    "val.as_u64().ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"expected number metadata\"))? as u32".to_owned()
                }
                PrimitiveType::U64 | PrimitiveType::Usize => {
                    "val.as_u64().ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"expected number metadata\"))?".to_owned()
                }
                PrimitiveType::I8 => {
                    "val.as_i64().ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"expected number metadata\"))? as i8".to_owned()
                }
                PrimitiveType::I16 => {
                    "val.as_i64().ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"expected number metadata\"))? as i16".to_owned()
                }
                PrimitiveType::I32 => {
                    "val.as_i64().ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"expected number metadata\"))? as i32".to_owned()
                }
                PrimitiveType::I64 | PrimitiveType::Isize => {
                    "val.as_i64().ok_or_else(|| napi::Error::new(napi::Status::InvalidArg, \"expected number metadata\"))?".to_owned()
                }
            }
        }
        TypeRef::Optional(inner) => {
            let inner_extraction = gen_metadata_extraction(inner, core_import, api);
            format!("if val.is_null() {{ None }} else {{ Some({{ {inner_extraction} }}) }}")
        }
        TypeRef::Named(n) => {
            // Check if this Named type is opaque in the API surface
            let is_opaque = api.types
                .iter()
                .find(|t| &t.name == n && !t.is_trait && t.is_opaque)
                .is_some();

            if is_opaque {
                // For opaque types: deserialize as the NAPI binding wrapper class,
                // then unwrap .inner to get the core type.
                // This follows the pattern: extract wrapper, then unwrap.inner
                format!(
                    "{{ \
                        let binding = serde_json::from_value::<crate::{name}>(val.clone()) \
                            .map_err(|e| napi::Error::from_reason(format!(\"opaque type deserialization failed: {{}}\", e)))?; \
                        binding.inner.clone() \
                    }}",
                    name = n
                )
            } else {
                // For non-opaque Named types: deserialize directly via serde_json
                "serde_json::from_value(val.clone())
                    .map_err(|e| napi::Error::from_reason(format!(\"metadata deserialization failed: {}\", e)))?".to_owned()
            }
        }
        _ => {
            // For other complex types: deserialize directly from serde_json::Value
            "serde_json::from_value(val.clone())
                .map_err(|e| napi::Error::from_reason(format!(\"metadata deserialization failed: {}\", e)))?".to_owned()
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

/// Generate all service-API files for the napi backend.
///
/// Returns up to four `GeneratedFile`s per non-empty service list:
/// - `{output_dir}/service.rs`   — Rust napi glue
/// - `{node_pkg}/service.ts`     — idiomatic TypeScript class (legacy: packages/node/)
/// - `crates/{name}-node/service.ts` — idiomatic TypeScript class (for index.js re-export)
/// - `crates/{name}-node/service.js` — idiomatic JavaScript class (transpiled from TS, runtime-compatible)
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(config.output_paths.get("node"), &config.name, "crates/{name}-node/src/");
    let crate_root = {
        let p = PathBuf::from(&output_dir);
        match p.file_name().and_then(|n| n.to_str()) {
            Some("src") => p.parent().map(|parent| parent.to_path_buf()).unwrap_or(p),
            _ => p,
        }
    };
    let package_name = config.name.replace('-', "_");

    // Rust glue
    let service_rs = gen_service_rs(api, config);

    // TypeScript wrapper
    let service_ts = gen_service_ts(api, &package_name, config);

    // JavaScript version (TypeScript with types stripped)
    let _service_js = strip_typescript_annotations(&service_ts);

    // Node package output base: derive from package_name or use default
    let output_base = config
        .node
        .as_ref()
        .and_then(|n| n.package_name.as_ref())
        .map(|p| PathBuf::from(format!("packages/node/{}", p)))
        .unwrap_or_else(|| PathBuf::from(format!("packages/node/{}", package_name)));

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: output_base.join("service.ts"),
            content: service_ts.clone(),
            generated_header: true,
        },
        GeneratedFile {
            path: crate_root.join("service.ts"),
            content: service_ts,
            generated_header: true,
        },
        // Note: service.js is transpiled from service.ts by removing type annotations.
        // Since JavaScript doesn't have TypeScript's type syntax, we emit service.ts as-is
        // but the post-build step in index.js will reference './service' which requires
        // Node to either: (a) have TypeScript loader registered, or (b) have service.ts
        // compiled to JS during build. For now, we omit service.js and rely on index.js
        // to use a try-catch fallback.
        // GeneratedFile {
        //     path: crate_root.join("service.js"),
        //     content: service_js,
        //     generated_header: true,
        // },
    ])
}

/// Convert TypeScript service wrapper to CommonJS JavaScript.
/// Simple approach: convert imports and output with // type comments removed.
/// Uses a brute-force regex-like approach to handle `: Type` patterns.
fn strip_typescript_annotations(ts_code: &str) -> String {
    let mut result = String::new();

    for line in ts_code.lines() {
        let mut modified_line = line.to_string();

        // Convert `import type { ... } from 'module'` → `const { ... } = require('module')`
        if modified_line.trim().starts_with("import type {") && modified_line.contains("from") {
            if let Some(start_brace) = modified_line.find('{') {
                if let Some(end_brace) = modified_line.rfind('}') {
                    if let Some(from_pos) = modified_line.find("from") {
                        let imports = modified_line[start_brace..=end_brace].to_string();
                        let module_part = modified_line[from_pos..].trim();
                        modified_line = format!("const {imports} = require({}", &module_part[5..]);
                    }
                }
            }
            result.push_str(&modified_line);
            result.push('\n');
            continue;
        }

        // Convert `import { ... } from 'module'` → `const { ... } = require('module')`
        if modified_line.trim().starts_with("import {") && modified_line.contains("from") {
            if let Some(start_brace) = modified_line.find('{') {
                if let Some(end_brace) = modified_line.rfind('}') {
                    if let Some(from_pos) = modified_line.find("from") {
                        let imports = modified_line[start_brace..=end_brace].to_string();
                        let module_part = modified_line[from_pos..].trim();
                        modified_line = format!("const {imports} = require({}", &module_part[5..]);
                    }
                }
            }
            result.push_str(&modified_line);
            result.push('\n');
            continue;
        }

        // Remove `export` keyword from class declarations (they're not needed in CommonJS)
        if modified_line.trim().starts_with("export class") {
            modified_line = modified_line.replace("export class", "class");
        }

        // Remove `private` keyword
        if modified_line.contains("private ") {
            modified_line = modified_line.replace("private ", "");
        }

        // Remove `: Type` where Type is anything up to ) or , or = or {
        // Using a character-by-character approach
        let mut output = String::new();
        let chars: Vec<char> = modified_line.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if i < chars.len() - 1 && chars[i] == ':' && !modified_line[..i].ends_with("://") {
                // Found a potential type annotation. Skip to the next ) , { or =
                let mut j = i + 1;
                // Skip whitespace
                while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t') {
                    j += 1;
                }
                // Skip the type (everything until we hit a boundary)
                let mut paren_depth = 0;
                let mut angle_depth = 0;
                while j < chars.len() {
                    match chars[j] {
                        '(' => paren_depth += 1,
                        ')' => {
                            if paren_depth == 0 {
                                break;
                            }
                            paren_depth -= 1;
                        }
                        '<' => angle_depth += 1,
                        '>' => angle_depth -= 1,
                        ',' | '=' | '{' | ';' if paren_depth == 0 && angle_depth == 0 => {
                            break;
                        }
                        _ => {}
                    }
                    j += 1;
                }
                // We've found the end of the type annotation. Skip to j.
                i = j;
                // Trim trailing space from output if present
                while !output.is_empty() && output.ends_with(' ') {
                    output.pop();
                }
                // Don't add extra spaces
                if i < chars.len() && chars[i] != ',' && chars[i] != ')' && !output.is_empty() {
                    output.push(' ');
                }
                continue;
            }

            output.push(chars[i]);
            i += 1;
        }

        modified_line = output;

        result.push_str(&modified_line);
        result.push('\n');
    }

    result
}

// ───────────────────────────────────────────────────────────────────── tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{
        EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind,
        RegistrationDef, ServiceDef, TypeRef,
    };

    /// Construct a minimal test config with default exclude settings.
    fn make_test_config() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: "my_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        }
    }

    /// Construct a minimal but realistic [`ApiSurface`] that exercises:
    /// - A service with a constructor, one configurator, one registration
    ///   (bound to an async handler contract), and Run entrypoint.
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
            receiver: Some(ReceiverKind::RefMut),
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
            receiver: Some(ReceiverKind::RefMut),
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
            receiver: Some(ReceiverKind::Ref),
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

    #[test]
    fn typescript_output_contains_service_class() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_ts(&surface, "my_crate", &config);
        assert!(
            output.contains("export class TestService"),
            "expected `export class TestService` in output:\n{output}"
        );
    }

    #[test]
    fn typescript_output_contains_constructor() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_ts(&surface, "my_crate", &config);
        assert!(
            output.contains("constructor()"),
            "expected `constructor()` in output:\n{output}"
        );
    }

    #[test]
    fn typescript_output_contains_private_registrations() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_ts(&surface, "my_crate", &config);
        assert!(
            output.contains("private _registrations"),
            "expected `private _registrations` in output:\n{output}"
        );
    }

    #[test]
    fn typescript_output_contains_configurator() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_ts(&surface, "my_crate", &config);
        assert!(
            output.contains("with_timeout(timeout_ms: number)"),
            "expected `with_timeout` configurator:\n{output}"
        );
        assert!(
            output.contains("return this;"),
            "expected `return this;` in configurator:\n{output}"
        );
    }

    #[test]
    fn typescript_output_contains_registration_method() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_ts(&surface, "my_crate", &config);
        assert!(
            output.contains("add_handler(path: string, method: string)"),
            "expected `add_handler` registration method:\n{output}"
        );
    }

    #[test]
    fn typescript_output_contains_run_entrypoint() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_ts(&surface, "my_crate", &config);
        assert!(
            output.contains("async run(addr: string)"),
            "expected `async run` entrypoint:\n{output}"
        );
    }

    #[test]
    fn rust_output_contains_handler_bridge() {
        let surface = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "my_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("pub struct RequestHandlerBridge"),
            "expected `RequestHandlerBridge` struct in output:\n{output}"
        );
    }

    #[test]
    fn rust_output_contains_run_function() {
        let surface = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "my_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("pub async fn test_service_run"),
            "expected `test_service_run` function in output:\n{output}"
        );
    }

    #[test]
    fn rust_output_contains_thread_safe_function() {
        let surface = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "my_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("ThreadsafeFunction"),
            "expected `ThreadsafeFunction` in output:\n{output}"
        );
    }

    #[test]
    fn rust_output_implements_trait() {
        let surface = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "my_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("impl my_crate::RequestHandler for RequestHandlerBridge"),
            "expected trait impl in output:\n{output}"
        );
    }

    #[test]
    fn rust_output_extracts_metadata_params() {
        let surface = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "my_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };
        let output = gen_service_rs(&surface, &config);

        // Assert metadata params are extracted as real typed variables, not stubs
        assert!(
            !output.contains("/* placeholder: extract metadata */"),
            "expected no placeholder in output:\n{output}"
        );
        assert!(
            !output.contains("placeholder: extract metadata"),
            "expected no unsupported marker in output:\n{output}"
        );

        // Assert the "path" metadata param is extracted and declared with proper type
        assert!(
            output.contains("let path: String"),
            "expected `let path: String` extraction in output:\n{output}"
        );

        // Assert the "method" metadata param is extracted and declared with proper type
        assert!(
            output.contains("let method: String"),
            "expected `let method: String` extraction in output:\n{output}"
        );

        // Assert both metadata params are passed to the registration method call
        assert!(
            output.contains("owner.add_handler(path, method, handler)"),
            "expected owner.add_handler(path, method, handler) call in output:\n{output}"
        );

        // Assert metadata is accessed from the _metadata vector
        assert!(
            output.contains("_metadata.get("),
            "expected _metadata.get(...) access in output:\n{output}"
        );
    }

    #[test]
    fn registration_variants_emit_napi_methods() {
        use crate::core::ir::{RegistrationVariant, WrapperConstructorArg, WrapperConstructorCall};

        let mut surface = make_fixture_surface();

        // Add a variant to the registration
        if let Some(reg) = surface.services[0].registrations.first_mut() {
            reg.variants.push(RegistrationVariant {
                name: "get".to_owned(),
                overrides: vec![],
                wrapper_call: Some(WrapperConstructorCall {
                    metadata_param: "builder".to_owned(),
                    wrapper_type_path: "my_crate::RouteBuilder".to_owned(),
                    wrapper_type_name: "RouteBuilder".to_owned(),
                    constructor_method: "new".to_owned(),
                    args: vec![
                        WrapperConstructorArg::Fixed {
                            param_name: "method".to_owned(),
                            value_expr: "my_crate::Method::GET".to_owned(),
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
                doc: Some("Register a GET handler.".to_owned()),
                style: Default::default(),
            });
        }

        let config = ResolvedCrateConfig {
            name: "my_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };
        let output = gen_service_rs(&surface, &config);

        // Assert the variant methods are wrapped in an impl block (default prefix "Js")
        assert!(
            output.contains("impl JsTestService {"),
            "expected `impl JsTestService {{` wrapping in output:\n{output}"
        );

        // Assert the use statement is emitted before the impl block
        assert!(
            output.contains("use crate::JsTestService;"),
            "expected `use crate::JsTestService;` in output:\n{output}"
        );

        // Assert the variant method is emitted with #[napi] (indented inside impl block)
        assert!(
            output.contains("#[napi]\n    pub fn get("),
            "expected `#[napi]\\n    pub fn get(` inside impl block in output:\n{output}"
        );

        // Assert the wrapper builder is constructed
        assert!(
            output.contains("my_crate::RouteBuilder::new("),
            "expected wrapper constructor call in output:\n{output}"
        );

        // Assert the fixed arg is substituted
        assert!(
            output.contains("my_crate::Method::GET"),
            "expected fixed arg substitution in output:\n{output}"
        );
    }

    #[test]
    fn typescript_output_skips_excluded_entrypoint() {
        let surface = make_fixture_surface();
        let mut config = make_test_config();
        // Add the entrypoint method to the exclude list
        config.exclude.methods.push("TestService.run".to_string());

        let output = gen_service_ts(&surface, "my_crate", &config);

        // Should NOT contain the run method when excluded
        assert!(
            !output.contains("async run(addr: string)"),
            "excluded `async run` entrypoint should not be present:\n{output}"
        );

        // Should NOT import the native function when excluded
        assert!(
            !output.contains("import { test_service_run }"),
            "excluded native function import should not be present:\n{output}"
        );

        // But should still contain the class
        assert!(
            output.contains("export class TestService"),
            "service class should still be present even with excluded entrypoint:\n{output}"
        );
    }

    #[test]
    fn rust_output_skips_excluded_entrypoint() {
        let surface = make_fixture_surface();
        let mut config = make_test_config();
        // Add the entrypoint method to the exclude list
        config.exclude.methods.push("TestService.run".to_string());

        let output = gen_service_rs(&surface, &config);

        // Should NOT contain the napi function when excluded
        assert!(
            !output.contains("pub async fn test_service_run"),
            "excluded `test_service_run` napi function should not be present:\n{output}"
        );

        // But should still contain the handler bridge
        assert!(
            output.contains("pub struct RequestHandlerBridge"),
            "RequestHandlerBridge should still be present even with excluded entrypoint:\n{output}"
        );
    }

    #[test]
    fn typescript_variant_verb_decorator_style() {
        use crate::core::ir::{RegistrationVariant, RegistrationVariantStyle};

        let mut surface = make_fixture_surface();

        if let Some(reg) = surface.services[0].registrations.first_mut() {
            reg.variants.push(RegistrationVariant {
                name: "get".to_owned(),
                overrides: vec![],
                wrapper_call: None,
                signature_params: vec![ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                }],
                doc: Some("Register a GET handler.".to_owned()),
                style: RegistrationVariantStyle::VerbDecorator,
            });
        }

        let config = make_test_config();
        let output = gen_service_ts(&surface, "my_crate", &config);

        // VerbDecorator should emit only the direct form: get(path, handler): this
        assert!(
            output.contains("get(path: string, handler: (...args: any[]) => any): this"),
            "expected VerbDecorator form `get(path, handler): this` in output:\n{output}"
        );

        // Should return `this` for chaining
        assert!(
            output.contains("return this;"),
            "expected `return this;` for chaining in VerbDecorator form:\n{output}"
        );

        // Should NOT emit decorator-factory form
        let get_count = output.matches("  get(").count();
        assert_eq!(
            get_count, 1,
            "expected exactly one `get(` method in VerbDecorator style, found {}: {}",
            get_count, output
        );
    }

    #[test]
    fn typescript_variant_builder_style() {
        use crate::core::ir::{RegistrationVariant, RegistrationVariantStyle};

        let mut surface = make_fixture_surface();

        if let Some(reg) = surface.services[0].registrations.first_mut() {
            reg.variants.push(RegistrationVariant {
                name: "get".to_owned(),
                overrides: vec![],
                wrapper_call: None,
                signature_params: vec![ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                }],
                doc: Some("Register a GET handler.".to_owned()),
                style: RegistrationVariantStyle::Builder,
            });
        }

        let config = make_test_config();
        let output = gen_service_ts(&surface, "my_crate", &config);

        // Builder should emit only the decorator-factory form: get(path) returns a function
        assert!(
            output.contains("get(path: string): (fn: (...args: any[]) => any) => (...args: any[]) => any"),
            "expected Builder form `get(path): (fn) => ...` in output:\n{output}"
        );

        // Should return the handler unchanged (for decorator form)
        assert!(
            output.contains("return fn;"),
            "expected `return fn;` in Builder form:\n{output}"
        );

        // Should NOT emit direct form with handler parameter
        assert!(
            !output.contains("get(path: string, handler: (...args: any[]) => any): this"),
            "Builder form should not emit direct method with handler parameter:\n{output}"
        );
    }

    #[test]
    fn typescript_variant_hybrid_style() {
        use crate::core::ir::{RegistrationVariant, RegistrationVariantStyle};

        let mut surface = make_fixture_surface();

        if let Some(reg) = surface.services[0].registrations.first_mut() {
            reg.variants.push(RegistrationVariant {
                name: "get".to_owned(),
                overrides: vec![],
                wrapper_call: None,
                signature_params: vec![ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                }],
                doc: Some("Register a GET handler.".to_owned()),
                style: RegistrationVariantStyle::Hybrid,
            });
        }

        let config = make_test_config();
        let output = gen_service_ts(&surface, "my_crate", &config);

        // Hybrid should emit both forms
        assert!(
            output.contains("get(path: string, handler: (...args: any[]) => any): this"),
            "expected Hybrid to include direct form `get(path, handler): this`:\n{output}"
        );

        assert!(
            output.contains("get(path: string): (fn: (...args: any[]) => any) => (...args: any[]) => any"),
            "expected Hybrid to include factory form `get(path): (fn) => ...`:\n{output}"
        );

        // Should have both `return this;` and `return fn;`
        let this_count = output.matches("return this;").count();
        let fn_count = output.matches("return fn;").count();
        assert!(
            this_count >= 1 && fn_count >= 1,
            "Hybrid form should have both return forms; this={}, fn={}: {}",
            this_count,
            fn_count,
            output
        );
    }

    #[test]
    fn rust_output_emits_entrypoint_methods_with_inner_accessor() {
        let config = {
            let mut cfg = make_test_config();
            // Register TestService with a host_app_inner_accessor so entrypoint methods are emitted
            cfg.services = vec![crate::core::config::ServiceConfig {
                owner_type: "TestService".to_string(),
                constructor: None,
                configurators: vec![],
                registrations: vec![],
                entrypoints: vec![],
                skip_languages: vec![],
                host_app_inner_accessor: Some("self.inner.lock().expect(\"mutex poisoned\")".to_string()),
            }];
            cfg
        };

        let api = make_fixture_surface();
        let output = gen_service_rs(&api, &config);

        // Should emit entrypoint method on the wrapper class (not just free function)
        assert!(
            output.contains("#[napi(js_name = \"nativeRun\")]"),
            "entrypoint method should have napi attribute with js_name; output:\n{output}"
        );

        assert!(
            output.contains("pub async fn run(&self, addr: String)"),
            "entrypoint method should be emitted as async method on wrapper; output:\n{output}"
        );

        // Should use the configured inner accessor to move the owner out before awaiting.
        assert!(
            output.contains("let mut guard = self.inner.lock().expect(\"mutex poisoned\");"),
            "entrypoint method should use configured inner accessor; output:\n{output}"
        );

        assert!(
            output.contains("owner.run(addr)"),
            "entrypoint method should call the inner method; output:\n{output}"
        );

        // The free function should still be emitted for backward compatibility
        assert!(
            output.contains("pub async fn test_service_run("),
            "free function entrypoint should still be emitted; output:\n{output}"
        );
    }

    #[test]
    fn rust_output_skips_entrypoint_methods_without_inner_accessor() {
        let config = make_test_config();
        // No host_app_inner_accessor configured, so entrypoint methods should NOT be emitted

        let api = make_fixture_surface();
        let output = gen_service_rs(&api, &config);

        // Should NOT emit entrypoint method on the wrapper class
        assert!(
            !output.contains("#[napi(js_name = \"nativeRun\")]"),
            "entrypoint method should not be emitted without host_app_inner_accessor; output:\n{output}"
        );

        // The free function should still be emitted
        assert!(
            output.contains("pub async fn test_service_run("),
            "free function entrypoint should still be emitted; output:\n{output}"
        );
    }
}
