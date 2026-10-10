use super::helpers::{build_php_wrapper_constructor_stmt, format_php_comment, render};
use super::type_mapping::php_type_annotation;
use crate::core::ir::{
    ApiSurface, EntrypointKind, ParamDef, RegistrationDef, RegistrationVariant, RegistrationVariantStyle, ServiceDef,
    TypeRef,
};
use heck::{ToLowerCamelCase, ToSnakeCase};

/// Generate the idiomatic PHP service class (`service.php`).
pub(in crate::backends::php::gen_bindings) fn gen_service_php(api: &ApiSurface, _extension_name: &str) -> String {
    let mut out = String::new();
    out.push_str("<?php\n\n");
    out.push_str("declare(strict_types=1);\n\n");

    for service in &api.services {
        gen_service_class(&mut out, service);
    }

    out
}

fn gen_service_class(out: &mut String, service: &ServiceDef) {
    if !service.doc.is_empty() {
        out.push_str(&format_php_comment(&service.doc, 0));
    }
    out.push_str("final class ");
    out.push_str(&service.name);
    out.push_str("\n{\n");
    out.push_str("    private array $registrations = [];\n\n");

    gen_constructor(out, service);
    for method in &service.configurators {
        gen_configurator(out, method);
    }
    for registration in &service.registrations {
        gen_registration(out, registration);
    }
    for entrypoint in &service.entrypoints {
        gen_entrypoint(out, service, entrypoint);
    }

    out.push_str("}\n\n");
}

fn gen_constructor(out: &mut String, service: &ServiceDef) {
    if !service.constructor.doc.is_empty() {
        out.push_str(&format_php_comment(&service.constructor.doc, 4));
    }
    out.push_str("    public function __construct()\n");
    out.push_str("    {\n");
    out.push_str("    }\n\n");
}

fn gen_configurator(out: &mut String, method: &crate::core::ir::MethodDef) {
    if !method.doc.is_empty() {
        out.push_str(&format_php_comment(&method.doc, 4));
    }
    let params = param_decl_list(&method.params);
    out.push_str(&render(
        "php_service_method_start.jinja",
        crate::alef_context! {
            method_name => &method.name,
            param_sig => &params,
            return_type => "self",
        },
    ));
    for param in &method.params {
        out.push_str(&format!("        $this->_{} = ${};\n", param.name, param.name));
    }
    out.push_str("        return $this;\n");
    out.push_str("    }\n\n");
}

fn gen_registration(out: &mut String, registration: &RegistrationDef) {
    gen_base_registration(out, registration);
    for variant in &registration.variants {
        match variant.style {
            RegistrationVariantStyle::VerbDecorator => gen_variant_direct(out, registration, variant),
            RegistrationVariantStyle::Builder => gen_variant_factory(out, registration, variant),
            RegistrationVariantStyle::Hybrid
            | RegistrationVariantStyle::Decorator
            | RegistrationVariantStyle::Attribute
            | RegistrationVariantStyle::Dsl => {
                gen_variant_direct(out, registration, variant);
                gen_variant_factory(out, registration, variant);
            }
        }
    }
}

fn gen_base_registration(out: &mut String, registration: &RegistrationDef) {
    if !registration.doc.is_empty() {
        out.push_str(&format_php_comment(&registration.doc, 4));
    }
    let mut params = param_decl_list(&registration.metadata_params);
    if !params.is_empty() {
        params.push_str(", ");
    }
    params.push_str(&format!("?callable ${} = null", registration.callback_param));

    out.push_str(&format!(
        "    public function {}({params}): mixed\n",
        registration.method
    ));
    out.push_str("    {\n");
    out.push_str(&format!("        if (${} !== null) {{\n", registration.callback_param));
    out.push_str(&render(
        "php_service_registration_store.jinja",
        crate::alef_context! {
            method_name => &registration.method,
            meta_tuple => metadata_array(&registration.metadata_params),
            callback_param => &registration.callback_param,
        },
    ));
    out.push_str("            return $this;\n");
    out.push_str("        }\n\n");
    out.push_str(&render(
        "php_service_registration_factory_body.jinja",
        crate::alef_context! {
            method_name => &registration.method,
            meta_tuple => metadata_array(&registration.metadata_params),
            callback_param => &registration.callback_param,
        },
    ));
    out.push_str("    }\n\n");
}

fn gen_variant_direct(out: &mut String, registration: &RegistrationDef, variant: &RegistrationVariant) {
    if let Some(doc) = &variant.doc {
        out.push_str(&format_php_comment(doc, 4));
    }
    let mut params = param_decl_list(&variant.signature_params);
    if !params.is_empty() {
        params.push_str(", ");
    }
    params.push_str(&format!("callable ${}", registration.callback_param));

    out.push_str(&format!(
        "    public function {}({params}): self\n",
        variant_method_name(variant)
    ));
    out.push_str("    {\n");
    if let Some(stmt) = build_php_wrapper_constructor_stmt(variant) {
        out.push_str("        ");
        out.push_str(&stmt);
        out.push('\n');
        let metadata_param = &variant
            .wrapper_call
            .as_ref()
            .expect("wrapper call exists")
            .metadata_param;
        out.push_str(&format!(
            "        return $this->{}(${}, ${});\n",
            registration.method, metadata_param, registration.callback_param
        ));
    } else {
        out.push_str(&render(
            "php_service_variant_direct_body.jinja",
            crate::alef_context! {
                base_method => &registration.method,
                vars => variant_metadata_values(registration, variant).join(", "),
                callback_param => &registration.callback_param,
            },
        ));
    }
    out.push_str("    }\n\n");
}

fn gen_variant_factory(out: &mut String, registration: &RegistrationDef, variant: &RegistrationVariant) {
    if let Some(doc) = &variant.doc {
        out.push_str(&format_php_comment(doc, 4));
    }
    let params = param_decl_list(&variant.signature_params);
    out.push_str(&format!(
        "    public function {}Decorator({params}): Closure\n",
        variant_method_name(variant)
    ));
    out.push_str("    {\n");
    if let Some(stmt) = build_php_wrapper_constructor_stmt(variant) {
        let metadata_param = &variant
            .wrapper_call
            .as_ref()
            .expect("wrapper call exists")
            .metadata_param;
        out.push_str(&render(
            "php_service_variant_wrapper_factory_body.jinja",
            crate::alef_context! {
                callback_param => &registration.callback_param,
                stmt => stmt,
                base_method => &registration.method,
                metadata_param => metadata_param,
            },
        ));
    } else {
        out.push_str(&render(
            "php_service_variant_factory_body.jinja",
            crate::alef_context! {
                callback_param => &registration.callback_param,
                base_method => &registration.method,
                call_sig => variant_metadata_values(registration, variant).join(", "),
            },
        ));
    }
    out.push_str("    }\n\n");
}

fn gen_entrypoint(out: &mut String, service: &ServiceDef, entrypoint: &crate::core::ir::EntrypointDef) {
    let params = param_decl_list(&entrypoint.params);
    let args = entrypoint
        .params
        .iter()
        .map(|param| format!("${}", param.name))
        .collect::<Vec<_>>()
        .join(", ");
    let native_fn = format!("{}_{}", service.name.to_snake_case(), entrypoint.method);

    match entrypoint.kind {
        EntrypointKind::Run => {
            if !entrypoint.doc.is_empty() {
                out.push_str(&format_php_comment(&entrypoint.doc, 4));
            }
            out.push_str(&render(
                "php_service_method_start.jinja",
                crate::alef_context! {
                    method_name => &entrypoint.method,
                    param_sig => &params,
                    return_type => "void",
                },
            ));
            out.push_str(&render(
                "php_service_native_call.jinja",
                crate::alef_context! {
                    native_fn => native_fn,
                    args => args,
                },
            ));
            out.push_str("    }\n\n");
        }
        EntrypointKind::Finalize => {
            if !entrypoint.doc.is_empty() {
                out.push_str(&format_php_comment(&entrypoint.doc, 4));
            }
            out.push_str(&render(
                "php_service_method_start.jinja",
                crate::alef_context! {
                    method_name => &entrypoint.method,
                    param_sig => &params,
                    return_type => php_type_annotation(&entrypoint.return_type),
                },
            ));
            out.push_str(&render(
                "php_service_native_return.jinja",
                crate::alef_context! {
                    native_fn => native_fn,
                    args => args,
                },
            ));
            out.push_str("    }\n\n");
        }
    }
}

fn param_decl_list(params: &[ParamDef]) -> String {
    params.iter().map(param_decl).collect::<Vec<_>>().join(", ")
}

fn param_decl(param: &ParamDef) -> String {
    let is_optional = param.optional || matches!(param.ty, TypeRef::Optional(_));
    let mut ty = php_type_annotation(&param.ty);
    if is_optional && !ty.starts_with('?') {
        ty.insert(0, '?');
    }
    if is_optional {
        format!("{ty} ${} = null", param.name)
    } else {
        format!("{ty} ${}", param.name)
    }
}

fn metadata_array(params: &[ParamDef]) -> String {
    let values = params
        .iter()
        .map(|param| format!("${}", param.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
}

fn variant_metadata_values(registration: &RegistrationDef, variant: &RegistrationVariant) -> Vec<String> {
    registration
        .metadata_params
        .iter()
        .map(|param| {
            variant
                .overrides
                .iter()
                .find(|override_value| override_value.param_name == param.name)
                .map(|override_value| override_value.value_expr.clone())
                .unwrap_or_else(|| format!("${}", param.name))
        })
        .collect()
}

fn variant_method_name(variant: &RegistrationVariant) -> String {
    variant.name.to_lower_camel_case()
}
