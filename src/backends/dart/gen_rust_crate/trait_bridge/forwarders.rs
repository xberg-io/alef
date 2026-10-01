use crate::core::config::TraitBridgeConfig;

/// Emit a Dart-side `register_*` forwarder for a configured trait bridge.
///
/// Wraps the user's `{Trait}DartImpl` in `std::sync::Arc::new(...)` and registers
/// it directly via the configured `registry_getter` (mirroring the PyO3/NAPI
/// approach). Going through the registry handle — rather than the host crate's
/// `register_*` free function — sidesteps the host's `pub(crate)` / `#[cfg(test)]`
/// restrictions on those wrappers (notably for `EmbeddingBackend`).
///
/// The forwarder returns `Result<(), String>` because FRB requires owned, FFI-
/// safe error types — the host's typed error is stringified for transport.
///
/// When `register_fn` is unset on the bridge config, no code is emitted.
pub(super) fn emit_register_forwarder(
    out: &mut String,
    bridge_config: &TraitBridgeConfig,
    struct_name: &str,
    source_crate_name: &str,
) {
    let Some(register_fn) = bridge_config.register_fn.as_deref() else {
        return;
    };
    let Some(registry_getter) = bridge_config.registry_getter.as_deref() else {
        return;
    };
    let extra_args = bridge_config
        .register_extra_args
        .as_deref()
        .map(|a| format!(", {a}"))
        .unwrap_or_default();
    let trait_path = format!("{source_crate_name}::plugins::{}", bridge_config.trait_name);

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_trait_register_forwarder.jinja",
        minijinja::context! {
            trait_name => bridge_config.trait_name.as_str(),
            registry_getter => registry_getter,
            register_fn => register_fn,
            struct_name => struct_name,
            trait_path => trait_path.as_str(),
            extra_args => extra_args.as_str(),
        },
    ));
}

/// Emit a Dart-side `unregister_*` forwarder for a configured trait bridge.
///
/// Removes a previously-registered plugin by name via the configured `registry_getter`.
/// Stringifies the host error. No-op when `unregister_fn` is unset on the bridge config.
pub(super) fn emit_unregister_forwarder(out: &mut String, bridge_config: &TraitBridgeConfig, _source_crate_name: &str) {
    let Some(unregister_fn) = bridge_config.unregister_fn.as_deref() else {
        return;
    };
    let Some(registry_getter) = bridge_config.registry_getter.as_deref() else {
        return;
    };

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_trait_unregister_forwarder.jinja",
        minijinja::context! {
            trait_name => bridge_config.trait_name.as_str(),
            registry_getter => registry_getter,
            unregister_fn => unregister_fn,
        },
    ));
}

/// Emit a Rust-side `clear_*` forwarder for a configured trait bridge.
///
/// Calls the configured host clear function so host-managed recovery semantics are preserved.
/// Stringifies the host error. No-op when `clear_fn` is unset on the bridge config.
pub(super) fn emit_clear_forwarder(out: &mut String, bridge_config: &TraitBridgeConfig, source_crate_name: &str) {
    let Some(clear_fn) = bridge_config.clear_fn.as_deref() else {
        return;
    };
    let host_path = crate::codegen::generators::trait_bridge::host_function_path_for_config(
        bridge_config,
        source_crate_name,
        clear_fn,
    );

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_trait_clear_forwarder.jinja",
        minijinja::context! {
            trait_name => bridge_config.trait_name.as_str(),
            clear_fn => clear_fn,
            host_path => host_path,
        },
    ));
}
