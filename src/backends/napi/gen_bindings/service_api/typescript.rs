use heck::{ToLowerCamelCase, ToSnakeCase};
use minijinja::context;

use crate::backends::napi::template_env::render;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, RegistrationDef, ServiceDef, TypeRef};

use super::helpers::typescript_type_annotation;

/// Classified import lists for the generated TypeScript service preamble.
///
/// Names referenced only in type position (handler DTOs, constructor/
/// configurator parameter types, registration signature parameter types) go
/// into `type_imports` — emitted as `import type { … }`. Names referenced at
/// runtime — wrapper constructors (`new RouteBuilder(…)`) and enum types whose
/// variants are reached via member access in registration variant bodies
/// (`Method.Get`) — go into `value_imports`, emitted as a plain
/// `import { … }`. The native bridge function names are split into a third
/// list so the line is omitted entirely when every entrypoint is excluded
/// from the binding surface.
struct ServiceImports {
    type_imports: Vec<String>,
    value_imports: Vec<String>,
    native_imports: Vec<String>,
}

fn classify_service_imports(api: &ApiSurface, _config: &ResolvedCrateConfig) -> ServiceImports {
    let mut type_imports: Vec<String> = Vec::new();
    let mut value_imports: Vec<String> = Vec::new();
    let mut native_imports: Vec<String> = Vec::new();

    // Wire DTOs (RequestData, Response, etc.) are deliberately NOT pre-seeded
    // here. napi-rs generated handler signatures use `(...args: any[])` so
    // wire-type names never appear in the rendered service body; importing
    // them produces TS6196 (declared but unused). They will be picked up
    // organically via constructor/configurator/variant signature_params below
    // if they ever appear in a position where TypeScript can type-check them.
    let _ = &api.handler_contracts;

    if let Some(service) = api.services.first() {
        for param in &service.constructor.params {
            if let TypeRef::Named(name) = &param.ty {
                type_imports.push(name.clone());
            }
        }
        for method in &service.configurators {
            for param in &method.params {
                if let TypeRef::Named(name) = &param.ty {
                    type_imports.push(name.clone());
                }
            }
        }

        for reg in &service.registrations {
            for variant in &reg.variants {
                if let Some(wrapper_call) = &variant.wrapper_call {
                    value_imports.push(wrapper_call.wrapper_type_name.clone());
                    for arg in &wrapper_call.args {
                        if let crate::core::ir::WrapperConstructorArg::Fixed {
                            param_name: _,
                            value_expr,
                        } = arg
                        {
                            // "mycrate::Method::Get" → second-to-last segment is the type name.
                            let parts: Vec<&str> = value_expr.split("::").collect();
                            if parts.len() >= 2 {
                                value_imports.push(parts[parts.len() - 2].to_string());
                            }
                        }
                    }
                }
                for param in &variant.signature_params {
                    if let TypeRef::Named(name) = &param.ty {
                        type_imports.push(name.clone());
                    }
                }
            }
        }

        let service_name = &service.name;
        for ep in &service.entrypoints {
            // napi-rs auto-camelCases Rust function names at the JS boundary
            // (e.g. `app_into_router` → `appIntoRouter`), so the imported
            // symbol must match the camelCase form. Service entrypoints are
            // explicit config and always import their free-function shim — the
            // wrapper class needs it regardless of `exclude.methods`.
            native_imports.push(format!("{}_{}", service_name.to_snake_case(), ep.method).to_lower_camel_case());
        }
    }

    type_imports.sort();
    type_imports.dedup();
    value_imports.sort();
    value_imports.dedup();
    native_imports.sort();
    native_imports.dedup();

    // A name that is referenced as a value must not also appear in
    // `import type { … }` — TypeScript would otherwise flag it unused or the
    // value import would shadow the type import in the type-only namespace.
    type_imports.retain(|name| !value_imports.contains(name));

    ServiceImports {
        type_imports,
        value_imports,
        native_imports,
    }
}

pub(in crate::backends::napi::gen_bindings) fn gen_service_ts(
    api: &ApiSurface,
    native_module: &str,
    config: &ResolvedCrateConfig,
) -> String {
    let mut out = String::new();

    let imports = classify_service_imports(api, config);
    out.push_str(&render(
        "service_ts_preamble.jinja",
        context! {
            type_imports => imports.type_imports.join(", "),
            value_imports => imports.value_imports.join(", "),
            native_imports => imports.native_imports.join(", "),
        },
    ));

    for service in &api.services {
        gen_service_class_ts(&mut out, service, api, native_module, config);
    }

    // Add explicit export statements for all services so they're available to CommonJS
    // (service.cjs will convert these to module.exports)
    let service_names: Vec<&str> = api.services.iter().map(|s| s.name.as_str()).collect();
    if !service_names.is_empty() {
        out.push_str(&format!("\nexport {{ {} }};\n", service_names.join(", ")));
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
    let native_class_name = format!("{}{}", config.node_type_prefix(), service.name);

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
            native_class_name => native_class_name.as_str(),
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
        let args = ctor
            .params
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let doc = ctor.doc.trim().replace('\n', "\n   * ");
        out.push_str(&render(
            "service_ts_constructor.jinja",
            context! {
                doc,
                param_sig,
                args,
                native_class_name => native_class_name.as_str(),
            },
        ));
    }

    // Configurator methods
    for method in &service.configurators {
        let mut params = Vec::new();
        for p in &method.params {
            let ty = typescript_type_annotation(&p.ty);
            // Configurator body is a stub `return this;` — params are
            // intentionally unused until alef supports persisting config
            // through the binding. TypeScript convention: prefix with `_` so
            // `noUnusedParameters` accepts the declaration.
            let display_name = format!("_{}", p.name);
            if p.optional {
                params.push(format!("{}: {} = undefined", display_name, ty));
            } else {
                params.push(format!("{}: {}", display_name, ty));
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

    // Entrypoint methods — service entrypoints are declared explicitly in
    // `[[crates.services.entrypoints]]` and always belong on the wrapper class,
    // even when the same method is listed in `exclude.methods` to suppress the
    // standard type-method placeholder. See the parallel comment in
    // `service_api::rust_glue` for the full rationale.
    for ep in &service.entrypoints {
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
        let native_method = ep_name.to_lower_camel_case();
        let native_args = ep.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");

        match ep.kind {
            EntrypointKind::Run => {
                out.push_str(&render(
                    "service_ts_entrypoint_run.jinja",
                    context! {
                        doc,
                        ep_name,
                        param_sig,
                        native_method,
                        native_args,
                    },
                ));
            }
            EntrypointKind::Finalize => {
                // The Rust bridge for Finalize hardcodes its return type to
                // `napi::Result<()>` (see `rust_glue::gen_run_function`'s
                // Finalize arm) and is unconditionally declared `pub async
                // fn`. So the JS-side function always returns
                // `Promise<void>`. The IR's `ep.return_type` describes the
                // *Rust* service method's return (e.g. `Router`) — using it
                // here yields a TS signature that does not match the bridge
                // and trips TS2322 at the `return native_fn(...)` site.
                let return_ty = "Promise<void>".to_owned();
                out.push_str(&render(
                    "service_ts_entrypoint_finalize.jinja",
                    context! {
                        doc,
                        ep_name,
                        param_sig,
                        return_ty,
                        native_method,
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

    // Positional metadata params forwarded to the underlying napi method.
    // The base registration emits a Rust signature with one positional param
    // per metadata entry plus `handler`, so the TS wrapper must call
    // `this._app.method(meta1, meta2, ..., fn)` — not an array.
    let meta_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
    let has_meta = !meta_names.is_empty();
    let meta_args = meta_names.join(", ");

    let doc = reg.doc.trim().replace('\n', "\n   * ");
    out.push_str(&render(
        "service_ts_registration_method.jinja",
        context! {
            doc,
            method_name,
            meta_sig,
            meta_args,
            has_meta,
        },
    ));

    // Also expose a direct (non-decorator) register variant. The wrapper method
    // is a JS class method, so it must be lowerCamelCase even though the IR's
    // `reg.method` is snake_case (matches Rust convention). Without conversion
    // consumers hit `TypeError: app.registerRoute is not a function` because
    // the wrapper exposes `register_route` instead.
    let direct_name = format!("register_{method_name}").to_lower_camel_case();
    if direct_name != *method_name {
        meta_params.push("handler: (...args: any[]) => any".to_string());
        let full_sig = meta_params.join(", ");
        out.push_str(&render(
            "service_ts_registration_direct_method.jinja",
            context! {
                method_name,
                direct_name,
                full_sig,
                meta_args,
                has_meta,
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
        // napi-rs does not emit JS `new`-able constructors for Rust types — Rust
        // constructors are exposed as static methods on the class (typically
        // named `new`). Call the static factory instead of `new WrapperType(…)`
        // to avoid TS2350 (`Only a void function can be called with the 'new'
        // keyword`).
        let constructor_method = &wrapper_call.constructor_method;

        // Return a tuple: (wrapper construction code, metadata array expression)
        let wrapper_code =
            format!("    const {metadata_param} = {wrapper_type}.{constructor_method}({ctor_arg_str});\n");
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

    let native_args = variant
        .signature_params
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

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
            native_args,
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

    let native_args = variant
        .signature_params
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

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
            native_args,
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
    let native_args = variant
        .signature_params
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

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
            native_args,
        },
    ));
}

#[cfg(test)]
mod classify_service_imports_tests {
    use super::{ServiceImports, classify_service_imports};
    use crate::core::config::resolved::ResolvedCrateConfig;
    use crate::core::ir::{
        ApiSurface, EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, RegistrationDef,
        RegistrationVariant, RegistrationVariantStyle, ServiceDef, TypeRef, WrapperConstructorArg,
        WrapperConstructorCall,
    };

    fn named_param(name: &str, type_name: &str) -> ParamDef {
        ParamDef {
            name: name.to_owned(),
            ty: TypeRef::Named(type_name.to_owned()),
            ..ParamDef::default()
        }
    }

    fn empty_method(name: &str) -> MethodDef {
        MethodDef {
            name: name.to_owned(),
            params: Vec::new(),
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: false,
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
            version: Default::default(),
        }
    }

    fn handler_contract(req: &str, resp: &str) -> HandlerContractDef {
        HandlerContractDef {
            trait_name: "Handler".to_owned(),
            rust_path: "test::Handler".to_owned(),
            dispatch: empty_method("dispatch"),
            optional_methods: Vec::new(),
            wire_request_type: Some(req.to_owned()),
            wire_response_type: Some(resp.to_owned()),
            dispatch_extra_params: Vec::new(),
            wire_param_name: None,
            dispatch_return_type: None,
            response_adapter: None,
            doc: String::new(),
        }
    }

    fn route_variant() -> RegistrationVariant {
        RegistrationVariant {
            name: "get".to_owned(),
            overrides: Vec::new(),
            wrapper_call: Some(WrapperConstructorCall {
                metadata_param: "builder".to_owned(),
                wrapper_type_path: "test::RouteBuilder".to_owned(),
                wrapper_type_name: "RouteBuilder".to_owned(),
                constructor_method: "new".to_owned(),
                args: vec![
                    WrapperConstructorArg::Fixed {
                        param_name: "method".to_owned(),
                        value_expr: "test::Method::Get".to_owned(),
                    },
                    WrapperConstructorArg::Free {
                        param: named_param("path", "Path"),
                    },
                ],
            }),
            signature_params: vec![named_param("path", "Path")],
            doc: None,
            style: RegistrationVariantStyle::Hybrid,
        }
    }

    fn fixture_surface() -> ApiSurface {
        let service = ServiceDef {
            name: "App".to_owned(),
            rust_path: "test::App".to_owned(),
            constructor: MethodDef {
                params: vec![named_param("config", "ServerConfig")],
                ..empty_method("new")
            },
            configurators: Vec::new(),
            registrations: vec![RegistrationDef {
                method: "route".to_owned(),
                callback_param: "handler".to_owned(),
                callback_contract: "Handler".to_owned(),
                metadata_params: vec![named_param("builder", "RouteBuilder")],
                receiver: None,
                return_type: TypeRef::Unit,
                error_type: None,
                doc: String::new(),
                variants: vec![route_variant()],
            }],
            entrypoints: vec![
                EntrypointDef {
                    method: "run".to_owned(),
                    kind: EntrypointKind::Run,
                    is_async: true,
                    params: Vec::new(),
                    return_type: TypeRef::Unit,
                    error_type: None,
                    doc: String::new(),
                },
                EntrypointDef {
                    method: "into_router".to_owned(),
                    kind: EntrypointKind::Finalize,
                    is_async: false,
                    params: Vec::new(),
                    return_type: TypeRef::Named("Router".to_owned()),
                    error_type: None,
                    doc: String::new(),
                },
            ],
            doc: String::new(),
            cfg: None,
        };
        ApiSurface {
            crate_name: "test".to_owned(),
            version: "0.0.0".to_owned(),
            handler_contracts: vec![handler_contract("RequestData", "Response")],
            services: vec![service],
            ..ApiSurface::default()
        }
    }

    #[test]
    fn classifies_wrappers_and_fixed_enums_as_value_imports() {
        let api = fixture_surface();
        let config = ResolvedCrateConfig::default();

        let ServiceImports {
            type_imports,
            value_imports,
            native_imports,
        } = classify_service_imports(&api, &config);

        assert_eq!(value_imports, vec!["Method", "RouteBuilder"]);
        // Wire DTOs (`RequestData`, `Response`) are intentionally NOT
        // pre-seeded — they would be flagged TS6196 since napi-rs handler
        // signatures use `(...args: any[])` and never reference them.
        assert_eq!(type_imports, vec!["Path".to_owned(), "ServerConfig".to_owned(),]);
        // napi-rs auto-camelCases at the JS boundary.
        assert_eq!(native_imports, vec!["appIntoRouter", "appRun"]);
    }

    #[test]
    fn entrypoint_imports_ignore_exclude_methods() {
        // Service entrypoints are declared explicitly under
        // `[[crates.services.entrypoints]]` and must always import their
        // free-function shim — they are the wrapper class's entry into the
        // native binding. `exclude.methods` is a generic per-method blacklist
        // used to suppress the standard type-method placeholder for items that
        // cannot be auto-delegated (consuming-self, async, etc.); it must not
        // suppress entrypoint imports, because the wrapper still needs to
        // reach the registration-replay free function.
        let api = fixture_surface();
        let mut config = ResolvedCrateConfig::default();
        config.exclude.methods.push("App.run".to_owned());
        config.exclude.methods.push("App.into_router".to_owned());

        let imports = classify_service_imports(&api, &config);

        assert_eq!(imports.native_imports, vec!["appIntoRouter", "appRun"]);
    }

    #[test]
    fn does_not_pre_seed_jsobject() {
        let api = fixture_surface();
        let imports = classify_service_imports(&api, &ResolvedCrateConfig::default());

        assert!(
            !imports.type_imports.iter().any(|n| n == "JsObject"),
            "JsObject must not appear in type imports — it is not exported by the napi-rs runtime"
        );
        assert!(
            !imports.value_imports.iter().any(|n| n == "JsObject"),
            "JsObject must not appear in value imports either"
        );
    }
}
