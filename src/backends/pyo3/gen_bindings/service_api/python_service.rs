use super::helpers::{
    collect_service_named_types, collect_variant_runtime_types, format_docstring, python_type_annotation,
};
use super::registration_variants::gen_registration_method;
use crate::core::ir::{ApiSurface, EntrypointKind, ServiceDef};
use heck::ToSnakeCase;
use minijinja::context;
use std::collections::BTreeSet;

pub(super) fn gen_service_py(api: &ApiSurface, module_name: &str) -> String {
    let mut out = String::new();

    let mut named_types: BTreeSet<String> = BTreeSet::new();
    let mut runtime_types: BTreeSet<String> = BTreeSet::new();
    for service in &api.services {
        collect_service_named_types(service, &mut named_types);
        collect_variant_runtime_types(service, &mut runtime_types);
    }
    for n in &runtime_types {
        named_types.remove(n);
    }
    let any_registrations = api.services.iter().any(|s| !s.registrations.is_empty());

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_py_header.py.jinja",
        context! { module_name => module_name },
    ));

    if !runtime_types.is_empty() {
        let joined = runtime_types.iter().cloned().collect::<Vec<_>>().join(", ");
        out.push_str(&crate::backends::pyo3::template_env::render(
            "service_api_py_runtime_import.py.jinja",
            context! { module_name => module_name, imports => joined },
        ));
    }

    if any_registrations || !named_types.is_empty() {
        out.push('\n');
        out.push_str("if TYPE_CHECKING:\n");
        if any_registrations {
            out.push_str("    from collections.abc import Callable\n");
            if !named_types.is_empty() {
                out.push('\n');
            }
        }
        if !named_types.is_empty() {
            let joined = named_types.iter().cloned().collect::<Vec<_>>().join(", ");
            out.push_str(&crate::backends::pyo3::template_env::render(
                "service_api_py_type_checking_import.py.jinja",
                context! { module_name => module_name, imports => joined },
            ));
        }
    }
    out.push_str("\n\n");

    for service in &api.services {
        gen_service_class(&mut out, service, api, module_name);
    }

    out
}

fn gen_service_class(out: &mut String, service: &ServiceDef, api: &ApiSurface, module_name: &str) {
    let class_name = &service.name;

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_py_class_header.py.jinja",
        context! { class_name => class_name },
    ));
    if !service.doc.is_empty() {
        out.push_str(&format_docstring(&service.doc, 4));
        out.push('\n');
    }

    {
        let ctor = &service.constructor;
        let mut init_params = vec!["self".to_owned()];
        let mut init_args = Vec::new();
        for p in &ctor.params {
            let annotation = python_type_annotation(&p.ty);
            if p.optional {
                init_params.push(format!("{}: {} | None = None", p.name, annotation));
            } else {
                init_params.push(format!("{}: {}", p.name, annotation));
            }
            init_args.push(p.name.clone());
        }

        let param_sig = init_params.join(", ");
        out.push_str(&crate::backends::pyo3::template_env::render(
            "service_api_py_init_header.py.jinja",
            context! { param_sig => param_sig },
        ));
        if !ctor.doc.is_empty() {
            out.push_str(&format_docstring(&ctor.doc, 8));
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "service_api_py_registration_state.py.jinja",
            context! {},
        ));
        for arg in &init_args {
            out.push_str(&crate::backends::pyo3::template_env::render(
                "service_api_py_init_assignment.py.jinja",
                context! { arg => arg },
            ));
        }
        out.push('\n');
    }

    for method in &service.configurators {
        let mut params = vec!["self".to_owned()];
        for p in &method.params {
            let annotation = python_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("{}: {} | None = None", p.name, annotation));
            } else {
                params.push(format!("{}: {}", p.name, annotation));
            }
        }
        let param_sig = params.join(", ");
        let method_name = &method.name;
        out.push_str(&crate::backends::pyo3::template_env::render(
            "service_api_py_configurator_header.py.jinja",
            context! {
                method_name => method_name,
                param_sig => param_sig,
                class_name => class_name,
            },
        ));
        if !method.doc.is_empty() {
            out.push_str(&format_docstring(&method.doc, 8));
        }
        for p in &method.params {
            out.push_str(&crate::backends::pyo3::template_env::render(
                "service_api_py_configurator_assignment.py.jinja",
                context! { name => p.name.as_str() },
            ));
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "service_api_py_return_self.py.jinja",
            context! {},
        ));
    }

    for reg in &service.registrations {
        gen_registration_method(out, reg, service, api, module_name);
    }

    for ep in &service.entrypoints {
        let mut params = vec!["self".to_owned()];
        for p in &ep.params {
            let annotation = python_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("{}: {} | None = None", p.name, annotation));
            } else {
                params.push(format!("{}: {}", p.name, annotation));
            }
        }
        let param_sig = params.join(", ");
        let ep_name = &ep.method;

        match ep.kind {
            EntrypointKind::Run => {
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "service_api_py_entrypoint_header.py.jinja",
                    context! { ep_name => ep_name, param_sig => param_sig, return_type => "None" },
                ));
                if !ep.doc.is_empty() {
                    out.push_str(&format_docstring(&ep.doc, 8));
                }
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                let args = ep
                    .params
                    .iter()
                    .map(|p| format!(", {}", p.name))
                    .collect::<Vec<_>>()
                    .join("");
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "service_api_py_entrypoint_call.py.jinja",
                    context! {
                        return_prefix => "",
                        module_name => module_name,
                        native_fn => native_fn,
                        args => args,
                    },
                ));
                out.push('\n');
            }
            EntrypointKind::Finalize => {
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "service_api_py_entrypoint_header.py.jinja",
                    context! { ep_name => ep_name, param_sig => param_sig, return_type => "Any" },
                ));
                if !ep.doc.is_empty() {
                    out.push_str(&format_docstring(&ep.doc, 8));
                }
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                let args = ep
                    .params
                    .iter()
                    .map(|p| format!(", {}", p.name))
                    .collect::<Vec<_>>()
                    .join("");
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "service_api_py_entrypoint_call.py.jinja",
                    context! {
                        return_prefix => "return ",
                        module_name => module_name,
                        native_fn => native_fn,
                        args => args,
                    },
                ));
                out.push('\n');
            }
        }
    }
}
