use crate::codegen::conversions::ConversionConfig;
use crate::codegen::conversions::helpers::{core_type_path_remapped, field_references_excluded_type, is_newtype};
use crate::core::ir::{CoreWrapper, TypeDef, TypeRef};
use ahash::AHashSet;

use super::fields::field_conversion_from_core_cfg;
use super::wrappers::apply_core_wrapper_from_core;

/// Generate `impl From<core::Type> for BindingType` (core -> binding).
pub fn gen_from_core_to_binding(typ: &TypeDef, core_import: &str, opaque_types: &AHashSet<String>) -> String {
    gen_from_core_to_binding_cfg(typ, core_import, opaque_types, &ConversionConfig::default())
}

/// Generate `impl From<core::Type> for BindingType` with backend-specific config.
pub fn gen_from_core_to_binding_cfg(
    typ: &TypeDef,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    config: &ConversionConfig,
) -> String {
    let core_path = core_type_path_remapped(typ, core_import, config.source_crate_remaps);
    let binding_name = format!("{}{}", config.type_name_prefix, typ.name);

    if is_newtype(typ) {
        let field = &typ.fields[0];
        let newtype_inner_expr = match &field.ty {
            TypeRef::Named(_) => "val.0.into()".to_string(),
            TypeRef::Path => "val.0.to_string_lossy().to_string()".to_string(),
            TypeRef::Duration => "val.0.as_millis() as u64".to_string(),
            _ => "val.0".to_string(),
        };
        return crate::codegen::template_env::render(
            "conversions/core_to_binding_impl",
            minijinja::context! {
                core_path => core_path,
                binding_name => binding_name,
                has_lifetime_params => typ.has_lifetime_params,
                is_newtype => true,
                newtype_inner_expr => newtype_inner_expr,
                fields => vec![] as Vec<String>,
            },
        );
    }

    let optionalized = config.optionalize_defaults && typ.has_default;

    let mut fields = Vec::new();
    for field in &typ.fields {
        if field.binding_excluded {
            continue;
        }
        if !config.exclude_types.is_empty() && field_references_excluded_type(&field.ty, config.exclude_types) {
            continue;
        }
        if field.cfg.is_some()
            && !config.never_skip_cfg_field_names.contains(&field.name)
            && config.strip_cfg_fields_from_binding_struct
        {
            continue;
        }
        let base_conversion = field_conversion_from_core_cfg(
            &field.name,
            &field.ty,
            field.optional,
            field.sanitized,
            opaque_types,
            config,
        );
        let base_conversion = if field.is_boxed && matches!(&field.ty, TypeRef::Named(_)) {
            if field.optional {
                let src = format!("{}: val.{}.map(Into::into)", field.name, field.name);
                let dst = format!("{}: val.{}.map(|v| (*v).into())", field.name, field.name);
                if base_conversion == src { dst } else { base_conversion }
            } else {
                base_conversion.replace(&format!("val.{}", field.name), &format!("(*val.{})", field.name))
            }
        } else {
            base_conversion
        };
        let base_conversion = if field.newtype_wrapper.is_some() {
            match &field.ty {
                TypeRef::Optional(_) => base_conversion.replace(
                    &format!("val.{}", field.name),
                    &format!("val.{}.map(|v| v.0)", field.name),
                ),
                TypeRef::Vec(_) => base_conversion.replace(
                    &format!("val.{}", field.name),
                    &format!("val.{}.iter().map(|v| v.0).collect::<Vec<_>>()", field.name),
                ),
                _ if field.optional => base_conversion.replace(
                    &format!("val.{}", field.name),
                    &format!("val.{}.map(|v| v.0)", field.name),
                ),
                _ => base_conversion.replace(&format!("val.{}", field.name), &format!("val.{}.0", field.name)),
            }
        } else {
            base_conversion
        };
        let is_flattened_optional = field.optional && matches!(field.ty, TypeRef::Optional(_));
        let base_conversion = if is_flattened_optional {
            if let TypeRef::Optional(inner) = &field.ty {
                let inner_conv = field_conversion_from_core_cfg(
                    &field.name,
                    inner.as_ref(),
                    true,
                    field.sanitized,
                    opaque_types,
                    config,
                );
                inner_conv.replace(&format!("val.{}", field.name), &format!("val.{}.flatten()", field.name))
            } else {
                base_conversion
            }
        } else {
            base_conversion
        };
        let needs_some_wrap = !is_flattened_optional
            && ((optionalized && !field.optional)
                || (config.option_duration_on_defaults
                    && typ.has_default
                    && !field.optional
                    && matches!(field.ty, TypeRef::Duration)));
        let conversion = if needs_some_wrap {
            if let Some(expr) = base_conversion.strip_prefix(&format!("{}: ", field.name)) {
                format!("{}: Some({})", field.name, expr)
            } else {
                base_conversion
            }
        } else {
            base_conversion
        };
        let is_opaque_no_wrapper_field = field.core_wrapper == CoreWrapper::None
            && matches!(&field.ty, TypeRef::Named(n) if config
                .opaque_types
                .is_some_and(|opaque| opaque.contains(n.as_str())));
        let conversion = if is_opaque_no_wrapper_field {
            if config.trait_bridge_field_is_arc_wrapper(&field.name) {
                if let TypeRef::Named(name) = &field.ty {
                    let wrapper = format!("{}{}", config.type_name_prefix, name);
                    if field.optional {
                        format!(
                            "{}: val.{}.map(|v| {wrapper} {{ inner: std::sync::Arc::new(v) }})",
                            field.name, field.name
                        )
                    } else {
                        format!(
                            "{}: {wrapper} {{ inner: std::sync::Arc::new(val.{}) }}",
                            field.name, field.name
                        )
                    }
                } else {
                    format!("{}: Default::default()", field.name)
                }
            } else {
                format!("{}: Default::default()", field.name)
            }
        } else if !field.sanitized || field.core_wrapper == crate::core::ir::CoreWrapper::Cow {
            apply_core_wrapper_from_core(
                &conversion,
                &field.name,
                &field.ty,
                &field.core_wrapper,
                &field.vec_inner_core_wrapper,
                field.optional,
            )
        } else {
            conversion
        };
        let binding_field = config.binding_field_name_owned(&typ.name, &field.name);
        let conversion = if binding_field != field.name {
            if let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) {
                format!("{binding_field}: {expr}")
            } else {
                conversion
            }
        } else {
            conversion
        };
        fields.push(conversion);
    }

    crate::codegen::template_env::render(
        "conversions/core_to_binding_impl",
        minijinja::context! {
            core_path => core_path,
            binding_name => binding_name,
            has_lifetime_params => typ.has_lifetime_params,
            is_newtype => false,
            newtype_inner_expr => "",
            fields => fields,
        },
    )
}
