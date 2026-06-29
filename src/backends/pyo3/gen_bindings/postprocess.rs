use crate::core::config::{BridgeBinding, ResolvedCrateConfig};
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

pub(super) fn add_options_field_visitor_fallback(
    content: &mut String,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    core_import: &str,
) {
    for bridge in &config.trait_bridges {
        if bridge.bind_via != BridgeBinding::OptionsField {
            continue;
        }
        let Some(field_name) = bridge.resolved_options_field() else {
            continue;
        };
        let Some(options_type) = bridge.options_type.as_deref() else {
            continue;
        };
        let field_in_binding = api
            .types
            .iter()
            .filter(|t| t.name == options_type)
            .flat_map(|t| t.fields.iter())
            .any(|f| f.cfg.is_none() && f.name == field_name);
        if !field_in_binding {
            continue;
        }

        let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge, core_import);
        let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Py", bridge);
        let closing_pattern =
            format!("        std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n    }});");
        if let Some(pos) = content.find(&closing_pattern) {
            let before = &content[..pos];
            let after = &content[pos + closing_pattern.len()..];
            let fallback = format!(
                "        std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n    }}).or_else(|| {{\n        options.as_ref().and_then(|o| o.{field_name}.as_ref()).map(|v| {{\n            let py_obj: pyo3::Py<pyo3::PyAny> = Python::attach(|py| (*v.inner).clone_ref(py));\n            let bridge = {struct_name}::new(py_obj);\n            std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n        }})\n    }});"
            );
            *content = format!("{before}{fallback}{after}");
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
