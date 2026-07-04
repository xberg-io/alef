/// Emit `external fun nativeRegister<Trait>`, `nativeUnregister<Trait>`, and
/// `nativeClear<Trait>s` declarations for every configured `[[crates.trait_bridges]]`
/// entry that does not list `kotlin_android` in its `exclude_languages` list.
///
/// The register fun signature receives the user-implemented `I<Trait>` interface
/// as a generic JVM `Any` reference; the Rust JNI shim is responsible for holding a
/// global reference and trampolining trait method calls back into the JVM.
///
/// Each generated `external fun` is annotated `@Throws(<Bridge>Exception::class)`
/// because both the Rust registration logic and the upcall vtable assembly can fail.
fn emit_trait_bridge_jni_external_funs(
    out: &mut String,
    config: &ResolvedCrateConfig,
    exception_class: &str,
    kotlin_package: &str,
    emitted_native_names: &std::collections::HashSet<String>,
) {
    let bridges: Vec<_> = config
        .trait_bridges
        .iter()
        .filter(|b| !b.exclude_languages.iter().any(|l| l == "kotlin_android"))
        .collect();
    if bridges.is_empty() {
        return;
    }
    out.push_str("\n    // JNI trait-bridge external funs — implementations are Rust JNI shims.\n");
    for bridge in &bridges {
        let trait_pascal = to_pascal_case(&bridge.trait_name);
        // Registration receives the generated <Trait>JniDispatcher (which wraps the
        // user's I<Trait>), not the interface itself: the dispatcher exposes the
        // non-suspend JSON dispatch entry point the Rust bridge calls. It lives in
        // the same package as the bridge object; the fully-qualified reference
        // avoids an extra import in the bridge file.
        let dispatcher_fqn = format!("{kotlin_package}.{trait_pascal}JniDispatcher");
        if bridge.register_fn.is_some() {
            let native_name = format!("nativeRegister{trait_pascal}");
            // Skip if already emitted from the API.
            if !emitted_native_names.contains(&native_name) {
                out.push('\n');
                push_jni_external_fun(
                    out,
                    &native_name,
                    &format!("impl: {dispatcher_fqn}"),
                    None,
                    Some(exception_class),
                );
            }
        }
        if bridge.unregister_fn.is_some() {
            let native_name = format!("nativeUnregister{trait_pascal}");
            // Skip if already emitted from the API.
            if !emitted_native_names.contains(&native_name) {
                push_jni_external_fun(out, &native_name, "name: String", None, Some(exception_class));
            }
        }
        if bridge.clear_fn.is_some() {
            let native_name = format!("nativeClear{trait_pascal}s");
            // Skip if already emitted from the API.
            if !emitted_native_names.contains(&native_name) {
                push_jni_external_fun(out, &native_name, "", None, Some(exception_class));
            }
        }
    }
}
