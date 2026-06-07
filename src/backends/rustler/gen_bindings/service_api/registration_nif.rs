//! Rust registration variant NIF generation.

use crate::backends::rustler::gen_bindings::service_api::helpers::typeref_to_rust_type;
use crate::backends::rustler::template_env::render;
use crate::core::ir::{ApiSurface, RegistrationDef, ServiceDef, TypeRef};
use heck::ToSnakeCase;
use minijinja::context;

pub(super) fn gen_registration_variant_nif(
    out: &mut String,
    service: &ServiceDef,
    base_reg: &RegistrationDef,
    variant: &crate::core::ir::RegistrationVariant,
    api: &ApiSurface,
    core_import: &str,
) {
    let service_snake = service.name.to_snake_case();
    let variant_name = &variant.name;
    let nif_name = format!("{}_{}", service_snake, variant_name);
    let base_method = &base_reg.method;
    let contract_name = &base_reg.callback_contract;
    let bridge_wrapper = format!("Elixir{contract_name}Bridge");
    let owner_path = &service.rust_path;

    // Build NIF signature
    let mut params = vec!["registrations: rustler::Term<'_>".to_owned()];
    for param in &variant.signature_params {
        let rust_ty = typeref_to_rust_type(&param.ty, core_import);
        params.push(format!("{}: {}", param.name, rust_ty));
    }
    params.push("handler: rustler::LocalPid".to_owned());
    let param_sig = params.join(", ");

    let (wrapper_type_name, wrapper_type_path, constructor_method, wrapper_args) =
        if let Some(wrapper_call) = &variant.wrapper_call {
            let wrapper_args = wrapper_call
                .args
                .iter()
                .map(|arg| match arg {
                    crate::core::ir::WrapperConstructorArg::Fixed {
                        param_name: _,
                        value_expr,
                    } => format!("        {},\n", value_expr),
                    crate::core::ir::WrapperConstructorArg::Free { param } => {
                        format!("        {},\n", param.name)
                    }
                })
                .collect::<String>();
            (
                wrapper_call.wrapper_type_name.as_str(),
                wrapper_call.wrapper_type_path.as_str(),
                wrapper_call.constructor_method.as_str(),
                wrapper_args,
            )
        } else {
            ("", "", "", String::new())
        };

    out.push_str(&render(
        "service_api_registration_variant_nif_header.rs.jinja",
        context! {
            variant_name => variant_name,
            base_method => base_method,
            nif_name => nif_name,
            param_sig => param_sig,
            owner_path => owner_path,
            wrapper_type_name => wrapper_type_name,
            wrapper_type_path => wrapper_type_path,
            constructor_method => constructor_method,
            wrapper_args => wrapper_args,
        },
    ));

    let metadata_param_names: Vec<&str> = base_reg.metadata_params.iter().map(|p| p.name.as_str()).collect();

    let (has_metadata, trailing, tuple_types, opaque_bindings, metadata_args) = if !metadata_param_names.is_empty() {
        let trailing = if metadata_param_names.len() == 1 { "," } else { "" };
        let tuple_types = base_reg
            .metadata_params
            .iter()
            .map(|p| {
                // Opaque types use super:: to name the local lib-module wrapper that implements
                // rustler::Resource. The wildcard import in service.rs would shadow a bare name.
                if let TypeRef::Named(n) = &p.ty {
                    if api.types.iter().any(|t| &t.name == n && !t.is_trait && t.is_opaque) {
                        return format!("rustler::ResourceArc<super::{}>", n);
                    }
                }
                typeref_to_rust_type(&p.ty, core_import)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let tuple_types_with_trailing = format!("{}{}", tuple_types, trailing);

        let mut opaque_bindings = String::new();
        for meta_param in base_reg.metadata_params.iter() {
            let is_opaque = if let TypeRef::Named(n) = &meta_param.ty {
                api.types.iter().any(|t| &t.name == n && !t.is_trait && t.is_opaque)
            } else {
                false
            };
            if is_opaque {
                if let TypeRef::Named(n) = &meta_param.ty {
                    // ResourceArc<super::T> derefs to the local wrapper super::T; wrapper.inner
                    // is Arc<CoreType>. Use as_ref() then clone() to obtain an owned CoreType.
                    opaque_bindings.push_str(&render(
                        "service_api_opaque_metadata_binding.rs.jinja",
                        context! {
                            indent => "                ",
                            param_name => meta_param.name,
                            core_import => core_import,
                            type_name => n,
                        },
                    ));
                }
            }
        }

        (
            true,
            trailing,
            tuple_types_with_trailing,
            opaque_bindings,
            metadata_param_names.join(", "),
        )
    } else {
        (false, "", String::new(), String::new(), String::new())
    };

    out.push_str(&render(
        "service_api_registration_variant_dispatch.rs.jinja",
        context! {
            has_metadata => has_metadata,
            metadata_names => metadata_param_names.join(", "),
            trailing => trailing,
            tuple_types => tuple_types,
            opaque_bindings => opaque_bindings,
            bridge_wrapper => bridge_wrapper,
            core_import => core_import,
            contract_name => base_reg.callback_contract,
            base_method => base_method,
            metadata_args => metadata_args,
        },
    ));

    out.push_str(&render(
        "service_api_registration_variant_nif_footer.rs.jinja",
        context! {},
    ));
}
