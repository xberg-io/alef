//! Elixir registration and configurator helper generation.

use crate::backends::rustler::gen_bindings::service_api::helpers::{push_elixir_doc, push_elixir_param};
use crate::backends::rustler::template_env::render;
use crate::core::ir::{ApiSurface, RegistrationDef, RegistrationVariantStyle, ServiceDef};
use minijinja::context;

pub(super) fn gen_registration_method(
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
        // Decorator, Attribute, Dsl and Hybrid all fall through to the hybrid form.
        // Per-backend specialization for the new styles is a Phase C concern.
        RegistrationVariantStyle::Hybrid
        | RegistrationVariantStyle::Decorator
        | RegistrationVariantStyle::Attribute
        | RegistrationVariantStyle::Dsl => {
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
