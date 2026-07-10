//! Emits service App struct wrappers for the swift-bridge crate.
//!
//! Each service with registrations gets a wrapper struct that exposes:
//! - `App { inner: tokio::sync::Mutex<Option<source_crate::App>> }`
//! - `pub fn new() -> Self`
//! - `pub fn config(&mut self) -> ()`
//! - `pub fn run(self) -> Result<(), String>`
//!
//! Also emits wrapper-constructor free functions (e.g. `route_builder_new`) for registration variants
//! that need to construct a wrapper metadata param (e.g. `RouteBuilder::new(method, path)`) in Swift.
//! These are declared in the bridge `extern "Rust"` block and implemented here.

use crate::core::ir::{ApiSurface, WrapperConstructorArg};
use heck::ToSnakeCase;

/// Generate App wrapper struct and impl for all services with registrations.
pub fn emit_service_app_wrappers(api: &ApiSurface, source_crate: &str) -> String {
    let mut out = String::new();

    if api.services.is_empty() {
        return out;
    }

    for service in &api.services {
        if service.registrations.is_empty() {
            continue;
        }

        let service_name = &service.name;
        let service_path = if service.rust_path.is_empty() {
            format!("{source_crate}::{service_name}")
        } else {
            service.rust_path.clone()
        };
        let constructor = &service.constructor.name;

        out.push_str(&crate::backends::swift::template_env::render(
            "rust_service_app_wrapper.rs.jinja",
            minijinja::context! {
                service_name => service_name,
                service_path => service_path,
                constructor => constructor,
            },
        ));

        let service_snake_local = service_name.to_lowercase();
        out.push_str(&crate::backends::swift::template_env::render(
            "rust_service_app_free_fns.rs.jinja",
            minijinja::context! {
                service_name => service_name,
                service_snake => service_snake_local,
            },
        ));

        let mut seen_wrapper_fns = std::collections::HashSet::new();
        for reg in &service.registrations {
            for variant in &reg.variants {
                let Some(wc) = &variant.wrapper_call else { continue };
                let fn_name = format!("{}_new", wc.wrapper_type_name.to_snake_case());
                if !seen_wrapper_fns.insert(fn_name.clone()) {
                    continue;
                }
                let mut param_defs: Vec<String> = Vec::new();
                let mut call_args: Vec<String> = Vec::new();
                for arg in &wc.args {
                    match arg {
                        WrapperConstructorArg::Fixed { param_name, value_expr } => {
                            let type_name = if let Some(last_colon) = value_expr.rfind("::") {
                                if let Some(second_colon) = value_expr[..last_colon].rfind("::") {
                                    &value_expr[second_colon + 2..last_colon]
                                } else {
                                    &value_expr[..last_colon]
                                }
                            } else {
                                value_expr.as_str()
                            };
                            param_defs.push(format!("{param_name}: &{type_name}"));
                            let core_enum_path = if let Some(last_colon) = value_expr.rfind("::") {
                                &value_expr[..last_colon]
                            } else {
                                value_expr.as_str()
                            };
                            call_args.push(format!(
                                "serde_json::from_str::<{core_enum_path}>(&format!(\"\\\"{{}}\\\"\\n\", {param_name}.to_string())).unwrap_or_default()"
                            ));
                        }
                        WrapperConstructorArg::Free { param } => {
                            param_defs.push(format!("{}: {}", param.name, rust_type_for_param(&param.ty)));
                            call_args.push(param.name.clone());
                        }
                    }
                }
                let params_str = param_defs.join(", ");
                let call_args_str = call_args.join(", ");
                let wrapper_type = &wc.wrapper_type_name;
                let wrapper_path = &wc.wrapper_type_path;
                let ctor = &wc.constructor_method;
                out.push_str(&crate::backends::swift::template_env::render(
                    "rust_wrapper_constructor_fn.rs.jinja",
                    minijinja::context! {
                        wrapper_type => wrapper_type,
                        fn_name => fn_name,
                        params => params_str,
                        wrapper_path => wrapper_path,
                        constructor => ctor,
                        call_args => call_args_str,
                    },
                ));
            }
        }
    }

    out
}

/// Map a TypeRef to a Rust type string for function parameter declaration.
fn rust_type_for_param(ty: &crate::core::ir::TypeRef) -> &'static str {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::String => "String",
        TypeRef::Primitive(_) => "i64",
        _ => "String",
    }
}
