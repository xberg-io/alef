/// Emit JNI Rust shims for every configured `[[crates.trait_bridges]]` entry.
///
/// For each bridge whose `exclude_languages` does not contain `kotlin_android`,
/// emits up to three `Java_*` symbols:
///   - `nativeRegister<Trait>(impl: I<Trait>)` — only emitted when the configured
///     trait resolves in the API surface with a non-empty method set, via
///     `gen_plugin_trait_bridge`. Otherwise generation panics rather than emitting
///     a shim that accepts the JNI call without ever invoking `register_fn`.
///   - `nativeUnregister<Trait>(name: String)` — calls the host crate's
///     `unregister_fn(&name)` and surfaces any `Err(_)` as a thrown JNI exception.
///   - `nativeClear<Trait>s()` — calls the host crate's `clear_fn()` similarly.
fn emit_trait_bridge_shims(
    out: &mut String,
    config: &ResolvedCrateConfig,
    api: &ApiSurface,
    package: &str,
    bridge: &str,
) {
    let bridges: Vec<_> = config
        .trait_bridges
        .iter()
        .filter(|b| !b.exclude_languages.iter().any(|l| l == "kotlin_android"))
        .collect();
    if bridges.is_empty() {
        return;
    }
    out.push_str("\n// ---------------------------------------------------------------------------\n");
    out.push_str("// Trait-bridge shims\n");
    out.push_str("// ---------------------------------------------------------------------------\n\n");

    for bridge_cfg in &bridges {
        let trait_pascal = internal_class_component(&bridge_cfg.trait_name);

        let trait_def = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name);

        if bridge_cfg.register_fn.is_some() {
            let native_name = format!("nativeRegister{trait_pascal}");
            let symbol = jni_symbol(package, bridge, &native_name);
            match trait_def {
                Some(trait_def) if !trait_def.methods.is_empty() => {
                    let bridge_output = crate::backends::jni::trait_bridge::gen_plugin_trait_bridge(
                        trait_def,
                        bridge_cfg,
                        &symbol,
                        "core_crate",
                        crate::backends::jni::trait_bridge::BridgeErrorBinding {
                            error_type: &config.error_type_name(),
                            error_constructor: &config.error_constructor_expr(),
                        },
                        api,
                    );
                    out.push_str(&bridge_output.code);
                    out.push_str("\n\n");
                }
                _ => {
                    panic!(
                        "JNI trait-bridge generator: crate `{}`, bridge `{bridge}` configures `register_fn` for trait `{}`, but the trait is either not resolvable in the API surface or has no own methods, so `gen_plugin_trait_bridge` cannot bridge it; configure `register_fn` only for traits that resolve to a non-empty method set",
                        config.name, bridge_cfg.trait_name,
                    );
                }
            }
        }
        if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
            let native_name = format!("nativeUnregister{trait_pascal}");
            let symbol = jni_symbol(package, bridge, &native_name);
            emit_trait_unregister_shim(out, &symbol, unregister_fn);
        }
        if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
            let native_name = format!("nativeClear{trait_pascal}s");
            let symbol = jni_symbol(package, bridge, &native_name);
            emit_trait_clear_shim(out, &symbol, clear_fn);
        }
    }
}

/// Emit `Java_*_nativeUnregister<Trait>(name: String)` shim that calls the
/// host crate's configured `unregister_fn`.
fn emit_trait_unregister_shim(out: &mut String, symbol: &str, unregister_fn: &str) {
    out.push_str(&template_env::render(
        "trait_unregister_shim.rs.jinja",
        context! {
            symbol => symbol,
            unregister_fn => unregister_fn,
        },
    ));
}

/// Emit `Java_*_nativeClear<Trait>s()` shim that calls the host crate's
/// configured `clear_fn`.
fn emit_trait_clear_shim(out: &mut String, symbol: &str, clear_fn: &str) {
    out.push_str(&template_env::render(
        "trait_clear_shim.rs.jinja",
        context! {
            symbol => symbol,
            clear_fn => clear_fn,
        },
    ));
}
