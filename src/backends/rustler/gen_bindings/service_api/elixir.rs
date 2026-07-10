//! Elixir service module generation for Rustler service APIs.

use crate::backends::rustler::gen_bindings::service_api::helpers::{elixir_heredoc_body, push_elixir_doc};
use crate::backends::rustler::gen_bindings::service_api::registration::gen_registration_method;
use crate::backends::rustler::template_env::render;
use crate::core::ir::{ApiSurface, EntrypointKind, ServiceDef};
use heck::ToSnakeCase;
use minijinja::context;

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

    emit_conn_struct(&mut out, module_prefix);

    for service in &api.services {
        gen_service_module(&mut out, service, api, module_prefix);
    }

    out
}

fn emit_conn_struct(out: &mut String, module_prefix: &str) {
    let conn_module = prefixed_module(module_prefix, "Conn");
    out.push_str(&render(
        "service_api_conn_struct.ex.jinja",
        context! {
            conn_module => conn_module,
        },
    ));
}

fn prefixed_module(module_prefix: &str, module: &str) -> String {
    if module_prefix.is_empty() {
        module.to_owned()
    } else {
        format!("{module_prefix}.{module}")
    }
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

    for reg in &service.registrations {
        gen_registration_method(out, reg, service, api, module_prefix);
    }

    gen_genserver_module(out, service, api);

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
