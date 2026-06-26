use super::{pyi_docstring, python_safe_name, substitute_capsule_type};
use crate::backends::pyo3::type_map::python_type;
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

    let mut lines = vec![format!("class {}(Protocol):", bridge.trait_name)];

    if emit_docstrings {
        if let Some(docstring) = pyi_docstring(&trait_def.doc, "    ") {
            lines.push(docstring);
        }
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
    for method in methods {
        if method.binding_excluded {
            continue;
        }
        body_emitted = true;
        let mut params: Vec<String> = vec!["self".to_string()];
        for p in &method.params {
            let param_type = substitute_capsule_type(&python_type(&p.ty), capsule_names);
            params.push(format!("{}: {}", p.name, param_type));
        }
        let return_type = substitute_capsule_type(&python_type(&method.return_type), capsule_names);
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
