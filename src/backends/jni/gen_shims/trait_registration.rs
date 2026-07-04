/// Emit JNI Rust shims for every configured `[[crates.trait_bridges]]` entry.
///
/// For each bridge whose `exclude_languages` does not contain `kotlin_android`,
/// emits:
/// - Trait adapter struct `Jni{Trait}Adapter` that wraps a global JNI reference
/// - Up to three `Java_*` symbols:
///   - `nativeRegister<Trait>(impl: I<Trait>)` — creates a global JNI reference,
///     wraps it in an adapter, and calls the host crate's `register_fn`.
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

    // Emit the registration functions
    for bridge_cfg in &bridges {
        let trait_pascal = internal_class_component(&bridge_cfg.trait_name);

        // Find the trait definition for method iteration
        let trait_def = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name);

        if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
            let native_name = format!("nativeRegister{trait_pascal}");
            let symbol = jni_symbol(package, bridge, &native_name);
            match trait_def {
                // With the trait definition available, emit the real bridge: wrapper
                // struct, dispatching trait impl, default delegates, and a registration
                // shim that hands the bridge to the host register_fn.
                Some(trait_def) if !trait_def.methods.is_empty() => {
                    let bridge_output = crate::backends::jni::trait_bridge::gen_plugin_trait_bridge(
                        trait_def,
                        bridge_cfg,
                        &symbol,
                        "core_crate",
                        &config.error_type_name(),
                        &config.error_constructor_expr(),
                        api,
                    );
                    out.push_str(&bridge_output.code);
                    out.push_str("\n\n");
                }
                // Without the trait definition (excluded from the surface) keep the
                // registration-accepting stub so linking still succeeds.
                _ => {
                    let has_super_trait = bridge_cfg.super_trait.is_some();
                    emit_trait_register_shim(out, &symbol, &trait_pascal, register_fn, trait_def, has_super_trait);
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

/// Emit `Java_*_nativeRegister<Trait>(impl: I<Trait>)` or
/// `Java_*_nativeRegister<Trait>(impl: I<Trait>, name: JString)` shim that creates a
/// global JNI reference, calls the host crate's configured `register_fn`, and manages
/// bridge lifetime.
///
/// When `has_super_trait` is true, the impl object's `name()` method is called.
/// When false, the name is passed as an explicit JString parameter (matching the Kotlin
/// no-super-trait register(impl, name) signature).
fn emit_trait_register_shim(
    out: &mut String,
    symbol: &str,
    trait_pascal: &str,
    register_fn: &str,
    _trait_def: Option<&TypeDef>,
    has_super_trait: bool,
) {
    out.push_str(&template_env::render(
        "trait_register_shim.rs.jinja",
        context! {
            symbol => symbol,
            pascal_trait => trait_pascal,
            register_fn => register_fn,
            has_super_trait => has_super_trait,
        },
    ));
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

// ---------------------------------------------------------------------------
// Inline helper emission
// ---------------------------------------------------------------------------
