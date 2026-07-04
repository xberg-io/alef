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
) -> Option<String> {
    let methods = bridge.resolve_methods(api);
    if methods.is_empty() {
        return None;
    }
    let trait_def = api.types.iter().find(|t| t.name == bridge.trait_name)?;

    // Plugin-style bridges (registered via `register_*`) only require the trait's
    // non-defaulted methods at runtime — the bridge forwards Rust-defaulted methods
    // when the host defines them and falls back to the Rust default otherwise, and
    // the Plugin lifecycle hooks are no-ops when absent. Emitting defaulted methods
    // as required Protocol members would reject every minimal (and valid) backend,
    // so the Protocol lists the required contract and the docstring documents the
    // optional surface. Visitor-style (options-field) bridges keep the full method
    // list: their hosts are ad-hoc override maps, not registered plugin objects.
    let is_plugin_bridge = bridge.register_fn.is_some();
    let (required, optional): (Vec<&crate::core::ir::MethodDef>, Vec<&crate::core::ir::MethodDef>) =
        methods.iter().partition(|m| !(is_plugin_bridge && m.has_default_impl));

    // Types excluded from the public binding surface are never emitted as `.pyi` classes; a
    // Protocol method referencing one (e.g. `-> InternalDocument`) would be an undefined name.
    // Substitute them with their JSON marshaling form, matching the runtime bridge.
    let excluded: std::collections::HashSet<&str> = api
        .excluded_type_paths
        .keys()
        .map(String::as_str)
        .chain(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.as_str()))
        .collect();

    let mut lines = vec![format!("class {}(Protocol):", bridge.trait_name)];

    // The optional-methods note documents generated behavior that is otherwise
    // invisible (the bridge forwards these when present), so it is emitted even
    // when rustdoc-derived docstrings are disabled.
    let mut doc = if emit_docstrings {
        trait_def.doc.clone()
    } else {
        String::new()
    };
    if !optional.is_empty() {
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

    // Each method becomes a Protocol method stub with `self` and the IR params.
    // Method bodies are `...` (Protocol convention). Skipped methods that the FFI
    // backend bypasses (`ffi_skip_methods`) are still part of the trait surface a
    // host-language visitor must implement, so include them too.
    //
    // Host callbacks are always `def`, never `async def`, regardless of the Rust trait:
    // the bridge invokes the method synchronously on a `tokio::spawn_blocking` worker and
    // uses its return value directly (see trait_bridge/generator.rs). A host coroutine would
    // never be awaited. The return type stays the native result type — the bridge now accepts
    // the binding's native object on return (with a mapping as fallback).
    let mut body_emitted = false;
    for method in required {
        if method.binding_excluded {
            continue;
        }
        body_emitted = true;
        let mut params: Vec<String> = vec!["self".to_string()];
        for p in &method.params {
            let param_type = substitute_capsule_type(
                &python_type(&substitute_excluded_types(&p.ty, &excluded)),
                capsule_names,
            );
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
        // Empty Protocol body is a syntax error; emit a single `...` placeholder.
        lines.push("    ...".to_string());
    }

    Some(lines.join("\n"))
}
