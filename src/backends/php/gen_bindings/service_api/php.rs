use super::helpers::{build_php_wrapper_constructor_stmt, format_php_comment, render};
use super::type_mapping::php_type_annotation;
use crate::core::ir::{ApiSurface, EntrypointKind, RegistrationDef, RegistrationVariantStyle, ServiceDef};
use heck::ToSnakeCase;
use minijinja::context;

/// Generate the idiomatic PHP service class (`service.php`).
///
/// Produces a PHP file containing one class per service. Each class exposes:
/// - A constructor mirroring [`ServiceDef::constructor`].
/// - Configurator methods from [`ServiceDef::configurators`].
/// - Registration methods from [`ServiceDef::registrations`].
/// - A `run(...)` method derived from the first [`EntrypointKind::Run`]
///   entrypoint.
pub(in crate::backends::php::gen_bindings) fn gen_service_php(api: &ApiSurface, extension_name: &str) -> String {
    let mut out = String::new();

    out.push_str("<?php\n\n");
    out.push_str("declare(strict_types=1);\n\n");

    // Emit one class per service
    for service in &api.services {
        gen_service_class(&mut out, service, api, extension_name);
    }

    out
}

fn gen_service_class(out: &mut String, service: &ServiceDef, api: &ApiSurface, extension_name: &str) {
    let class_name = &service.name;

    // Class declaration with docblock
    if !service.doc.is_empty() {
        out.push_str(&format_php_comment(&service.doc, 0));
    }
    out.push_str(&render(
        "php_service_class_start.jinja",
        context! { class_name => class_name },
    ));

    // Private registrations storage
    out.push_str("    private array $registrations = [];\n\n");

    // __construct
    {
        let ctor = &service.constructor;
        let mut ctor_params = Vec::new();
        let mut ctor_assigns = Vec::new();

        for p in &ctor.params {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                ctor_params.push(format!("?{} ${} = null", annotation, p.name));
            } else {
                ctor_params.push(format!("{} ${}", annotation, p.name));
            }
            // Store constructor param as private property for use in run()
            ctor_assigns.push(p.name.clone());
        }

        let param_sig = ctor_params.join(", ");
        // PHP constructors cannot declare a return type — emitting `: void`
        // is a parse error. The return type is implicit.
        out.push_str(&render(
            "php_service_constructor_start.jinja",
            context! { param_sig => &param_sig },
        ));
        if !ctor.doc.is_empty() {
            out.push_str(&format_php_comment(&ctor.doc, 8));
        }

        // Store constructor args as instance properties
        for arg in &ctor_assigns {
            out.push_str(&render(
                "php_service_property_assignment.jinja",
                context! { name => arg },
            ));
        }
        out.push_str("    }\n\n");
    }

    // Configurator methods
    for method in &service.configurators {
        let mut params = Vec::new();
        for p in &method.params {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("?{} ${} = null", annotation, p.name));
            } else {
                params.push(format!("{} ${}", annotation, p.name));
            }
        }
        let param_sig = params.join(", ");
        let method_name = &method.name;
        out.push_str(&render(
            "php_service_method_start.jinja",
            context! {
                method_name => method_name,
                param_sig => &param_sig,
                return_type => "self",
            },
        ));
        if !method.doc.is_empty() {
            out.push_str(&format_php_comment(&method.doc, 8));
        }

        // Store each configurator param as instance property
        for p in &method.params {
            out.push_str(&render(
                "php_service_property_assignment.jinja",
                context! { name => &p.name },
            ));
        }
        out.push_str("        return $this;\n");
        out.push_str("    }\n\n");
    }

    // Registration methods
    for reg in &service.registrations {
        gen_registration_method(out, reg, service, api, extension_name);
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        let mut params = Vec::new();
        for p in &ep.params {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("?{} ${} = null", annotation, p.name));
            } else {
                params.push(format!("{} ${}", annotation, p.name));
            }
        }
        let param_sig = params.join(", ");
        let ep_name = &ep.method;

        match ep.kind {
            EntrypointKind::Run => {
                out.push_str(&render(
                    "php_service_method_start.jinja",
                    context! {
                        method_name => ep_name,
                        param_sig => &param_sig,
                        return_type => "void",
                    },
                ));
                if !ep.doc.is_empty() {
                    out.push_str(&format_php_comment(&ep.doc, 8));
                }

                // Build the call to the native run function
                // Convention: native fn is `{snake_service_name}_{entrypoint_name}`
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                let args = php_service_native_args(&ep.params);
                out.push_str(&render(
                    "php_service_native_call.jinja",
                    context! {
                        native_fn => &native_fn,
                        args => &args,
                    },
                ));
                out.push_str("    }\n\n");
            }
            EntrypointKind::Finalize => {
                let return_annotation = php_type_annotation(&ep.return_type);
                out.push_str(&render(
                    "php_service_method_start.jinja",
                    context! {
                        method_name => ep_name,
                        param_sig => &param_sig,
                        return_type => &return_annotation,
                    },
                ));
                if !ep.doc.is_empty() {
                    out.push_str(&format_php_comment(&ep.doc, 8));
                }

                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                let args = php_service_native_args(&ep.params);
                out.push_str(&render(
                    "php_service_native_return.jinja",
                    context! {
                        native_fn => &native_fn,
                        args => &args,
                    },
                ));
                out.push_str("    }\n\n");
            }
        }
    }

    out.push_str("}\n\n");
}

fn php_service_native_args(params: &[crate::core::ir::ParamDef]) -> String {
    params
        .iter()
        .map(|p| format!("${}", p.name))
        .collect::<Vec<_>>()
        .join(", ")
}

fn gen_registration_method(
    out: &mut String,
    reg: &RegistrationDef,
    _service: &ServiceDef,
    _api: &ApiSurface,
    _extension_name: &str,
) {
    let method_name = &reg.method;

    // Build metadata param signature (excluding the callback param)
    let meta_params: Vec<String> = reg
        .metadata_params
        .iter()
        .map(|p| {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                format!("?{} ${} = null", annotation, p.name)
            } else {
                format!("{} ${}", annotation, p.name)
            }
        })
        .collect();

    // For direct registration (non-decorator), also add the callback param
    let mut direct_params = meta_params.clone();
    direct_params.push(format!("callable ${}", reg.callback_param));

    let meta_sig = meta_params.join(", ");
    let direct_sig = direct_params.join(", ");

    // Decorator factory form: returns a closure
    out.push_str(&render(
        "php_service_method_start.jinja",
        context! {
            method_name => method_name,
            param_sig => &meta_sig,
            return_type => "callable",
        },
    ));
    if !reg.doc.is_empty() {
        out.push_str(&format_php_comment(&reg.doc, 8));
    }

    // Build the metadata tuple for storage
    let meta_tuple = if reg.metadata_params.is_empty() {
        "[]".to_owned()
    } else {
        let names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
        format!(
            "[{}]",
            names.iter().map(|n| format!("${}", n)).collect::<Vec<_>>().join(", ")
        )
    };

    out.push_str(&render(
        "php_service_registration_factory_body.jinja",
        context! {
            callback_param => &reg.callback_param,
            method_name => method_name,
            meta_tuple => &meta_tuple,
        },
    ));
    out.push_str("    }\n\n");

    // Also expose a direct (non-decorator) variant: `register_{method_name}`
    let direct_name = format!("register_{method_name}");
    if direct_name != *method_name {
        out.push_str(&render(
            "php_service_method_start.jinja",
            context! {
                method_name => &direct_name,
                param_sig => &direct_sig,
                return_type => "self",
            },
        ));
        out.push_str(&render(
            "php_service_registration_store.jinja",
            context! {
                method_name => method_name,
                meta_tuple => &meta_tuple,
                callback_param => &reg.callback_param,
            },
        ));
        out.push_str("        return $this;\n");
        out.push_str("    }\n\n");
    }

    // Emit verb-decorator variants (e.g., $app->get(), $app->post())
    for variant in &reg.variants {
        gen_registration_variant(out, variant, reg, method_name);
    }
}

/// Emit a verb-decorator variant method(s) based on the registration style.
///
/// - `VerbDecorator`: Emit only the direct method form (e.g., `get(path, handler): App`)
/// - `Builder`: Emit only the decorator-factory form (e.g., `getDecorator(path): Closure`)
/// - `Hybrid`: Emit both direct method and decorator-factory
///
/// When the variant has a `wrapper_call`, the method constructs the wrapper
/// object and delegates to the base registration method instead of writing
/// directly to `$this->registrations[]`.
fn gen_registration_variant(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    reg: &RegistrationDef,
    base_method: &str,
) {
    let variant_name = variant.name.to_lowercase();
    let callback_param = &reg.callback_param;

    // Build the parameter list for metadata (non-callback) params
    let meta_params: Vec<String> = variant
        .signature_params
        .iter()
        .map(|p| {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                format!("?{} ${} = null", annotation, p.name)
            } else {
                format!("{} ${}", annotation, p.name)
            }
        })
        .collect();

    // Build the full parameter list for direct method (metadata + callback)
    let mut direct_params = meta_params.clone();
    direct_params.push(format!("callable ${callback_param}"));

    let meta_sig = meta_params.join(", ");
    let direct_sig = direct_params.join(", ");

    // When the variant has a wrapper_call, the body constructs the wrapper
    // object and delegates to the base method.  Otherwise, fall back to the
    // legacy computed call_args path.
    let wrapper_stmt = build_php_wrapper_constructor_stmt(variant);

    // Compute the base registration call arguments (used when wrapper_call is absent)
    let mut call_args: Vec<String> = Vec::new();
    for base_param in &reg.metadata_params {
        if let Some(override_) = variant.overrides.iter().find(|o| o.param_name == base_param.name) {
            call_args.push(override_.value_expr.clone());
        } else if let Some(sig_param) = variant.signature_params.iter().find(|s| s.name == base_param.name) {
            call_args.push(format!("${}", sig_param.name));
        }
    }
    let call_sig = call_args.join(", ");

    // Pre-compute the method bodies to avoid multiple mutable borrows of `out`.
    let direct_body = if let Some(ref stmt) = wrapper_stmt {
        let metadata_param = &variant.wrapper_call.as_ref().unwrap().metadata_param;
        format!("        {stmt}\n        return $this->{base_method}(${metadata_param}, ${callback_param});\n")
    } else {
        let vars = call_args
            .iter()
            .filter_map(|arg| if arg.starts_with('$') { Some(arg.clone()) } else { None })
            .collect::<Vec<_>>()
            .join(", ");
        render(
            "php_service_variant_direct_body.jinja",
            context! {
                base_method => base_method,
                vars => &vars,
                callback_param => callback_param,
            },
        )
    };

    let factory_body = if let Some(ref stmt) = wrapper_stmt {
        let metadata_param = &variant.wrapper_call.as_ref().unwrap().metadata_param;
        render(
            "php_service_variant_wrapper_factory_body.jinja",
            context! {
                callback_param => callback_param,
                stmt => stmt,
                base_method => base_method,
                metadata_param => metadata_param,
            },
        )
    } else {
        render(
            "php_service_variant_factory_body.jinja",
            context! {
                callback_param => callback_param,
                base_method => base_method,
                call_sig => &call_sig,
            },
        )
    };

    match variant.style {
        RegistrationVariantStyle::VerbDecorator => {
            // Emit direct method: $app->get(path, handler): App
            out.push_str(&render(
                "php_service_method_start.jinja",
                context! {
                    method_name => &variant_name,
                    param_sig => &direct_sig,
                    return_type => "self",
                },
            ));
            if let Some(doc) = &variant.doc {
                out.push_str(&format_php_comment(doc, 8));
            }
            out.push_str(&direct_body);
            out.push_str("    }\n\n");
        }

        RegistrationVariantStyle::Builder => {
            // Emit decorator factory: $app->getDecorator(path): Closure
            let factory_name = format!("{variant_name}Decorator");
            out.push_str(&render(
                "php_service_method_start.jinja",
                context! {
                    method_name => &factory_name,
                    param_sig => &meta_sig,
                    return_type => "Closure",
                },
            ));
            if let Some(doc) = &variant.doc {
                out.push_str(&format_php_comment(doc, 8));
            }
            out.push_str(&factory_body);
            out.push_str("    }\n\n");
        }

        // Decorator, Attribute, Dsl and Hybrid all fall through to the hybrid form.
        // Per-backend specialization for the new styles is a Phase C concern.
        RegistrationVariantStyle::Hybrid
        | RegistrationVariantStyle::Decorator
        | RegistrationVariantStyle::Attribute
        | RegistrationVariantStyle::Dsl => {
            // 1. Direct method: $app->get(path, handler): App
            out.push_str(&render(
                "php_service_method_start.jinja",
                context! {
                    method_name => &variant_name,
                    param_sig => &direct_sig,
                    return_type => "self",
                },
            ));
            if let Some(doc) = &variant.doc {
                out.push_str(&format_php_comment(doc, 8));
            }
            out.push_str(&direct_body);
            out.push_str("    }\n\n");

            // 2. Decorator factory: $app->getDecorator(path): Closure
            let factory_name = format!("{variant_name}Decorator");
            out.push_str(&render(
                "php_service_method_start.jinja",
                context! {
                    method_name => &factory_name,
                    param_sig => &meta_sig,
                    return_type => "Closure",
                },
            ));
            if let Some(doc) = &variant.doc {
                out.push_str(&format_php_comment(doc, 8));
            }
            out.push_str(&factory_body);
            out.push_str("    }\n\n");
        }
    }
}
