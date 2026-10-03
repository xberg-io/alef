use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};

pub(super) fn clear_bridge_builder_opaque_params(content: &mut String, config: &ResolvedCrateConfig) {
    for bridge in config.trait_bridges_for(Language::Python) {
        if let Some(field_name) = bridge.resolved_options_field() {
            let param_name = bridge.param_name.as_deref().unwrap_or(field_name);
            let pattern = format!(".{}({}.as_ref().map(|v| &v.inner))", field_name, param_name);
            let replacement = format!(".{}(None)", field_name);
            *content = content.replace(&pattern, &replacement);
        }
    }
}

pub(super) fn wrap_optional_default_args(content: &mut String, api: &ApiSurface, config: &ResolvedCrateConfig) {
    for func in &api.functions {
        let bridge_param_indices: ahash::AHashSet<usize> =
            crate::codegen::generators::trait_bridge::options_field_bridge_sites(
                func,
                config.trait_bridges_for(Language::Python),
            )
            .into_iter()
            .map(|site| site.param_index)
            .collect();
        for (param_index, param) in func.params.iter().enumerate() {
            if bridge_param_indices.contains(&param_index) {
                continue;
            }
            if !param.optional {
                continue;
            }
            if let TypeRef::Named(name) = &param.ty
                && api.types.iter().any(|t| &t.name == name && t.has_default)
            {
                let core_var = format!("{}_core", param.name);
                let call_pattern = format!(", {core_var})");
                let call_replacement = format!(", Some({core_var}))");
                replace_in_function(content, &func.name, &call_pattern, &call_replacement);
            }
        }
    }
}

fn replace_in_function(content: &mut String, function_name: &str, pattern: &str, replacement: &str) {
    let marker = format!("pub fn {function_name}");
    let mut search_start = 0;
    while let Some(relative_start) = content[search_start..].find(&marker) {
        let start = search_start + relative_start;
        let end = content[start..]
            .find("\n#[")
            .map_or(content.len(), |relative_end| start + relative_end);
        let rewritten = content[start..end].replace(pattern, replacement);
        content.replace_range(start..end, &rewritten);
        search_start = start + rewritten.len();
    }
}
