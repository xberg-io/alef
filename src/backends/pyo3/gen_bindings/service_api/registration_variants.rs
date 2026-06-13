use super::helpers::{find_contract, format_docstring, python_type_annotation};
use crate::core::ir::{ApiSurface, RegistrationDef, RegistrationVariantStyle, ServiceDef};
use heck::ToShoutySnakeCase;
use minijinja::context;
use std::collections::BTreeSet;

fn build_wrapper_constructor_expr(variant: &crate::core::ir::RegistrationVariant) -> Option<String> {
    let wc = variant.wrapper_call.as_ref()?;
    let mut call_args = Vec::new();

    for arg in &wc.args {
        match arg {
            crate::core::ir::WrapperConstructorArg::Fixed {
                param_name: _,
                value_expr,
            } => {
                // Convert a Rust enum path like `my_crate::Method::Get` into the
                // Python form `Method.GET`: take the last two `::` segments
                // (the enum type name and the variant name) and apply the same
                // SHOUTY_SNAKE_CASE rename the pyclass codegen emits via
                // `#[pyo3(name = "…")]`. Non-`::` values pass through verbatim
                // — the library author owns them.
                let segments: Vec<&str> = value_expr.split("::").collect();
                if segments.len() >= 2 {
                    let class = segments[segments.len() - 2];
                    let variant_name = segments[segments.len() - 1].to_shouty_snake_case();
                    call_args.push(format!("{class}.{variant_name}"));
                } else {
                    call_args.push(value_expr.clone());
                }
            }
            crate::core::ir::WrapperConstructorArg::Free { param } => {
                call_args.push(param.name.clone());
            }
        }
    }

    // PyO3 opaque wrappers expose a `new` classmethod (emitted by gen_bindings/types.rs)
    // rather than a `__init__` constructor, so build `WrapperType.new(...)` when the IR
    // names a constructor method. The bare `WrapperType(...)` form remains for backends
    // that bind a true `__init__`.
    let call_expr = if wc.constructor_method.is_empty() || wc.constructor_method == "__init__" {
        format!("{}({})", wc.wrapper_type_name, call_args.join(", "))
    } else {
        format!(
            "{}.{}({})",
            wc.wrapper_type_name,
            wc.constructor_method,
            call_args.join(", ")
        )
    };
    Some(format!("{} = {}", wc.metadata_param, call_expr))
}

/// Compute the shared metadata tuple string and the set of consumed param names
/// for a registration variant. Used by both emission forms so the logic is not
/// duplicated.
fn variant_meta_tuple(variant: &crate::core::ir::RegistrationVariant, base_reg: &RegistrationDef) -> (String, String) {
    let wrapper_consumed: BTreeSet<&str> = if let Some(wc) = &variant.wrapper_call {
        let mut s = BTreeSet::new();
        s.insert(wc.metadata_param.as_str());
        for arg in &wc.args {
            match arg {
                crate::core::ir::WrapperConstructorArg::Fixed { param_name, .. } => {
                    s.insert(param_name.as_str());
                }
                crate::core::ir::WrapperConstructorArg::Free { param } => {
                    s.insert(param.name.as_str());
                }
            }
        }
        s
    } else {
        BTreeSet::new()
    };
    let overridden: BTreeSet<&str> = variant.overrides.iter().map(|o| o.param_name.as_str()).collect();
    let mut meta_items: Vec<String> = Vec::new();
    if let Some(wc) = &variant.wrapper_call {
        meta_items.push(wc.metadata_param.clone());
    }
    for p in &base_reg.metadata_params {
        if wrapper_consumed.contains(p.name.as_str()) || overridden.contains(p.name.as_str()) {
            continue;
        }
        meta_items.push(p.name.clone());
    }
    let base_method = base_reg.method.clone();
    let meta_tuple = if meta_items.is_empty() {
        "()".to_owned()
    } else if meta_items.len() == 1 {
        format!("({},)", meta_items[0])
    } else {
        format!("({})", meta_items.join(", "))
    };
    (base_method, meta_tuple)
}

/// Emit the direct verb-decorator form: `def get(self, path, handler) -> ClassName`.
///
/// Used when `style` is `VerbDecorator` or `Hybrid`.
fn emit_direct_method(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
    class_name: &str,
    free_params_sig: &[String],
    meta_tuple: &str,
) {
    let variant_name = &variant.name;
    let base_method = &base_reg.method;

    let params_sig = if free_params_sig.is_empty() {
        "self, handler: Callable[..., Any]".to_owned()
    } else {
        format!("self, {}, handler: Callable[..., Any]", free_params_sig.join(", "))
    };

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_py_direct_variant_header.py.jinja",
        context! { variant_name => variant_name, params_sig => params_sig, class_name => class_name },
    ));

    if let Some(doc) = &variant.doc {
        out.push_str(&format_docstring(doc, 8));
    } else {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "service_api_py_direct_variant_doc.py.jinja",
            context! { variant_name => variant_name },
        ));
    }

    if let Some(wrapper_expr) = build_wrapper_constructor_expr(variant) {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "service_api_py_statement.py.jinja",
            context! { statement => wrapper_expr },
        ));
    }

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_py_append_registration.py.jinja",
        context! { base_method => base_method, meta_tuple => meta_tuple, callback => "handler" },
    ));
    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_py_return_self.py.jinja",
        context! {},
    ));
}

/// Emit the builder/decorator-factory form: `def get_decorator(self, path) -> Callable`.
///
/// Used when `style` is `Builder` or `Hybrid`.
fn emit_decorator_factory(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
    free_params_sig: &[String],
    meta_tuple: &str,
) {
    let variant_name = &variant.name;
    let base_method = &base_reg.method;
    let decorator_name = format!("{variant_name}_decorator");

    let params_sig_no_handler = if free_params_sig.is_empty() {
        "self".to_owned()
    } else {
        format!("self, {}", free_params_sig.join(", "))
    };

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_py_decorator_factory_header.py.jinja",
        context! { decorator_name => decorator_name, params_sig => params_sig_no_handler },
    ));
    if let Some(doc) = &variant.doc {
        let decorator_doc = format!("Decorator form for {}", doc.trim_start());
        out.push_str(&format_docstring(&decorator_doc, 8));
    } else {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "service_api_py_decorator_variant_doc.py.jinja",
            context! { variant_name => variant_name },
        ));
    }

    if let Some(wrapper_expr) = build_wrapper_constructor_expr(variant) {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "service_api_py_statement.py.jinja",
            context! { statement => wrapper_expr },
        ));
    }

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_py_decorator_body.py.jinja",
        context! { base_method => base_method, meta_tuple => meta_tuple },
    ));
}

/// Emit a single overloaded method that acts as both direct registration and decorator factory.
/// This is the "Decorator" style: `def get(self, path, handler=None)`.
///
/// When `handler` is provided, registers directly and returns `self` (chainable).
/// When `handler` is `None`, returns a decorator function.
///
/// Used when `style` is `Decorator`.
fn emit_decorator_overload(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
    class_name: &str,
    free_params_sig: &[String],
    meta_tuple: &str,
) {
    let variant_name = &variant.name;
    let base_method = &base_reg.method;

    let params_sig = if free_params_sig.is_empty() {
        "self, handler: Callable[..., Any] | None = None".to_owned()
    } else {
        format!(
            "self, {}, handler: Callable[..., Any] | None = None",
            free_params_sig.join(", ")
        )
    };

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_py_decorator_overload_header.py.jinja",
        context! { variant_name => variant_name, params_sig => params_sig, class_name => class_name },
    ));

    if let Some(doc) = &variant.doc {
        out.push_str(&format_docstring(doc, 8));
    } else {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "service_api_py_direct_variant_doc.py.jinja",
            context! { variant_name => variant_name },
        ));
    }

    if let Some(wrapper_expr) = build_wrapper_constructor_expr(variant) {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "service_api_py_statement.py.jinja",
            context! { statement => wrapper_expr },
        ));
    }

    // Render the overload body: if handler is None, return decorator; else register + return self
    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_py_decorator_overload_body.py.jinja",
        context! {
            base_method => base_method,
            meta_tuple => meta_tuple,
            callback => "handler",
        },
    ));
}

/// Emit a registration variant (shortcut method) for the given variant definition.
///
/// Which forms are emitted depends on [`RegistrationVariant::style`]:
/// - [`RegistrationVariantStyle::VerbDecorator`] — only the direct method form
///   (`def get(self, path, handler)`).
/// - [`RegistrationVariantStyle::Builder`] — only the decorator-factory form
///   (`def get_decorator(self, path) -> Callable`).
/// - [`RegistrationVariantStyle::Hybrid`] (default) — both forms.
fn gen_registration_variant(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
    _service: &ServiceDef,
    class_name: &str,
) {
    // Build the free params (non-fixed) for the variant signature
    let mut free_params_sig = Vec::new();
    for param in &variant.signature_params {
        let annotation = python_type_annotation(&param.ty);
        if param.optional {
            free_params_sig.push(format!("{}: {} | None = None", param.name, annotation));
        } else {
            free_params_sig.push(format!("{}: {}", param.name, annotation));
        }
    }

    let (_base_method, meta_tuple) = variant_meta_tuple(variant, base_reg);

    match variant.style {
        RegistrationVariantStyle::VerbDecorator => {
            emit_direct_method(out, variant, base_reg, class_name, &free_params_sig, &meta_tuple);
        }
        RegistrationVariantStyle::Builder => {
            emit_decorator_factory(out, variant, base_reg, &free_params_sig, &meta_tuple);
        }
        RegistrationVariantStyle::Decorator => {
            // Python-specific: overloaded method acts as both direct and decorator factory.
            emit_decorator_overload(out, variant, base_reg, class_name, &free_params_sig, &meta_tuple);
        }
        // Attribute and Dsl are not applicable to Python; fall through to Hybrid.
        RegistrationVariantStyle::Hybrid | RegistrationVariantStyle::Attribute | RegistrationVariantStyle::Dsl => {
            emit_direct_method(out, variant, base_reg, class_name, &free_params_sig, &meta_tuple);
            emit_decorator_factory(out, variant, base_reg, &free_params_sig, &meta_tuple);
        }
    }
}

pub(super) fn gen_registration_method(
    out: &mut String,
    reg: &RegistrationDef,
    service: &ServiceDef,
    api: &ApiSurface,
    _module_name: &str,
) {
    let method_name = &reg.method;
    let class_name = &service.name;

    // Find the contract to get wire-type doc info
    let _contract = find_contract(api, &reg.callback_contract);

    // Build metadata param signature (excluding the callback param)
    let mut meta_params: Vec<String> = reg
        .metadata_params
        .iter()
        .map(|p| {
            let annotation = python_type_annotation(&p.ty);
            if p.optional {
                format!("{}: {} | None = None", p.name, annotation)
            } else {
                format!("{}: {}", p.name, annotation)
            }
        })
        .collect();
    meta_params.insert(0, "self".to_owned());

    // Decorator factory form: `def method(self, *meta_params) -> Callable`
    // This lets the user write:
    //   @app.register(meta1, meta2)
    //   async def handler(request): ...
    let meta_sig = meta_params.join(", ");

    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_py_registration_method_header.py.jinja",
        context! { method_name => method_name, meta_sig => meta_sig },
    ));
    if !reg.doc.is_empty() {
        out.push_str(&format_docstring(&reg.doc, 8));
    }

    // Collect metadata param names for the closure
    let meta_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
    let meta_tuple = if meta_names.is_empty() {
        "()".to_owned()
    } else if meta_names.len() == 1 {
        format!("({},)", meta_names[0])
    } else {
        format!("({})", meta_names.join(", "))
    };

    // PEP8 / ruff-format: nested function definitions inside a method body
    // get a leading and trailing blank line so they read as a logical block.
    out.push_str(&crate::backends::pyo3::template_env::render(
        "service_api_py_decorator_body.py.jinja",
        context! { base_method => method_name, meta_tuple => meta_tuple },
    ));

    // Also expose a plain (non-decorator) register variant for direct use:
    // `app.register_handler(meta1, meta2, handler=fn)`
    let direct_name = format!("register_{method_name}");
    if direct_name != *method_name {
        // Only add when the name differs (avoid collision if method is already named "register_*")
        out.push_str(&crate::backends::pyo3::template_env::render(
            "service_api_py_direct_registration.py.jinja",
            context! {
                direct_name => direct_name,
                meta_sig => meta_sig,
                callback_param => reg.callback_param.as_str(),
                class_name => class_name,
                method_name => method_name,
                meta_tuple => meta_tuple,
            },
        ));
    }

    // Emit registration variants (shortcuts for common patterns)
    for variant in &reg.variants {
        gen_registration_variant(out, variant, reg, service, class_name);
    }
}
