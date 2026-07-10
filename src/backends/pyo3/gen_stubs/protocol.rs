use super::{pyi_docstring, python_safe_name, substitute_capsule_type};
use crate::backends::pyo3::type_map::python_type;
use crate::codegen::shared::substitute_excluded_types;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::ApiSurface;

/// Generate a `class TraitName(Protocol):` stub for an `OptionsField` trait bridge.
///
/// Returns `None` when the bridge's trait is absent from the API surface (e.g. excluded
/// from the public surface) — callers fall back to the legacy `type_alias` name in that
/// case so the stub still compiles.
///
/// The method signatures come from `TraitBridgeConfig::resolve_methods(api)`, which
/// looks up `bridge.trait_name` in `api.types` — the same source the trait-bridge code
/// generators use to emit the runtime vtable. This guarantees the Protocol surface in
/// the `.pyi` matches the methods the bridge actually forwards through PyO3.
pub(super) fn gen_visitor_protocol_stub(
    bridge: &TraitBridgeConfig,
    api: &ApiSurface,
    capsule_names: &std::collections::HashSet<&str>,
    emit_docstrings: bool,
    options_types: &std::collections::HashSet<String>,
) -> Option<String> {
    let methods = bridge.resolve_methods(api);
    if methods.is_empty() {
        return None;
    }
    let trait_def = api.types.iter().find(|t| t.name == bridge.trait_name)?;

    let is_plugin_bridge = bridge.register_fn.is_some();
    let (required, optional): (Vec<&crate::core::ir::MethodDef>, Vec<&crate::core::ir::MethodDef>) =
        methods.iter().partition(|m| !(is_plugin_bridge && m.has_default_impl));

    let excluded: std::collections::HashSet<&str> = api
        .excluded_type_paths
        .keys()
        .map(String::as_str)
        .chain(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.as_str()))
        .collect();

    let mut lines = vec![format!("class {}(Protocol):", bridge.trait_name)];

    let mut doc = if emit_docstrings {
        trait_def.doc.clone()
    } else {
        String::new()
    };
    if emit_docstrings && !optional.is_empty() {
        let optional_list = optional
            .iter()
            .map(|m| format!("`{}`", python_safe_name(&m.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let lifecycle_note = if bridge.super_trait.is_some() {
            " The lifecycle hooks `initialize()` and `shutdown()` (and `name()` / `version()`) are likewise optional."
        } else {
            ""
        };
        if !doc.is_empty() {
            doc.push_str("\n\n");
        }
        doc.push_str(&format!(
            "Optional methods a backend may additionally implement — the bridge calls them when the object defines them, otherwise the trait's Rust default behavior applies: {optional_list}.{lifecycle_note}"
        ));
    }
    if let Some(docstring) = pyi_docstring(&doc, "    ") {
        lines.push(docstring);
    }

    let mut body_emitted = false;
    for method in required {
        if method.binding_excluded {
            continue;
        }
        body_emitted = true;
        let mut params: Vec<String> = vec!["self".to_string()];
        for p in &method.params {
            let param_type = match &p.ty {
                crate::core::ir::TypeRef::Named(n) if is_plugin_bridge && options_types.contains(n) => {
                    format!("options.{n}")
                }
                _ => substitute_capsule_type(
                    &python_type(&substitute_excluded_types(&p.ty, &excluded)),
                    capsule_names,
                ),
            };
            params.push(format!("{}: {}", p.name, param_type));
        }
        let return_type = substitute_capsule_type(
            &python_type(&substitute_excluded_types(&method.return_type, &excluded)),
            capsule_names,
        );
        let safe_name = python_safe_name(&method.name);
        let signature = format!("    def {}({}) -> {}: ...", safe_name, params.join(", "), return_type);
        lines.push(signature);
    }

    if !body_emitted {
        lines.push("    ...".to_string());
    }

    Some(lines.join("\n"))
}
