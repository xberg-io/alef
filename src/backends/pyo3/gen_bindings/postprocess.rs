use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, TypeRef};

pub(super) fn clear_bridge_builder_opaque_params(content: &mut String, config: &ResolvedCrateConfig) {
    for bridge in &config.trait_bridges {
        if let Some(field_name) = bridge.resolved_options_field() {
            let param_name = bridge.param_name.as_deref().unwrap_or(field_name);
            let pattern = format!(".{}({}.as_ref().map(|v| &v.inner))", field_name, param_name);
            let replacement = format!(".{}(None)", field_name);
            *content = content.replace(&pattern, &replacement);
        }
    }
}

pub(super) fn wrap_optional_default_args(content: &mut String, api: &ApiSurface) {
    for func in &api.functions {
        for param in &func.params {
            if !param.optional {
                continue;
            }
            if let TypeRef::Named(name) = &param.ty
                && api.types.iter().any(|t| &t.name == name && t.has_default)
            {
                let core_var = format!("{}_core", param.name);
                let call_pattern = format!(", {core_var})");
                let call_replacement = format!(", Some({core_var}))");
                *content = content.replace(&call_pattern, &call_replacement);
            }
        }
    }
}
