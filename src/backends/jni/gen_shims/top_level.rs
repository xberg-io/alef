pub(crate) fn emit_lib_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let package = jni_kotlin_package(config);
    let bridge = bridge_class_name(&config.name);
    let core_crate = core_use_path(config);
    let error_class = resolve_error_class(config, &package);

    let mut out = String::new();

    out.push_str(&template_env::render(
        "lib_header.rs.jinja",
        context! {
            core_crate => core_crate,
            error_class => error_class,
        },
    ));

    // Per-trait `use` clauses for trait-method dispatch. The lib_header glob
    // `use core_crate::*;` only brings items declared at the crate root, so
    // traits that live in submodules and are NOT `pub use`-d at the root
    // (Tier-B Rust-public extension points) need explicit imports here.
    // Without these, every emitted `client.trait_method(...)` call fails
    // with `no method named X found for reference &T`.
    //
    // Mirrors what extendr / rustler / wasm / php / magnus / ffi / dart /
    // pyo3 / napi already do via the same shared helper.
    for trait_path in collect_trait_imports(api) {
        out.push_str(&format!("use {trait_path};\n"));
    }

    // Shared runtime helpers.
    emit_runtime_helpers(&mut out);

    // Collect visible top-level functions.
    let exclude_functions: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();

    // Trait-bridge register / unregister / clear functions are also emitted by
    // `emit_trait_bridge_shims` below; iterating them again as plain top-level
    // functions would emit duplicate `Java_*_native…` symbols and break linking.
    let trait_bridge_fn_names: std::collections::HashSet<&str> = config
        .trait_bridges
        .iter()
        .flat_map(|b| {
            [&b.register_fn, &b.unregister_fn, &b.clear_fn]
                .into_iter()
                .filter_map(|opt| opt.as_deref())
        })
        .collect();

    let visible_functions: Vec<_> = api
        .functions
        .iter()
        .filter(|f| {
            !f.sanitized
                && !exclude_functions.contains(f.name.as_str())
                && !trait_bridge_fn_names.contains(f.name.as_str())
        })
        .collect();

    // Collect opaque type names for handle-vs-JSON dispatch.
    let opaque_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

    // Top-level function shims.
    for f in &visible_functions {
        let method_name = bridge_method_name("", &f.name);
        let symbol = jni_symbol(&package, &bridge, &method_name);
        emit_function_shim(
            &mut out,
            &symbol,
            &f.rust_path,
            &f.params,
            &f.return_type,
            f.is_async,
            f.error_type.is_some(),
            &opaque_type_names,
            &config.name,
        );
    }

    // Opaque client type shims (types that have instance methods).
    let client_types: Vec<_> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && t.methods.iter().any(|m| !m.sanitized && !m.is_static))
        .collect();
    let client_type_names: std::collections::HashSet<&str> = client_types.iter().map(|t| t.name.as_str()).collect();

    for ty in &client_types {
        emit_client_shims(
            &mut out,
            ty,
            api,
            config,
            &package,
            &bridge,
            &exclude_functions,
            &opaque_type_names,
        );
    }

    // Emit destructors for opaque types that are returned by top-level functions
    // but do NOT have instance methods (those are handled by emit_client_shims above).
    let top_level_opaque_returns: std::collections::HashSet<&str> = visible_functions
        .iter()
        .filter_map(|f| {
            if let TypeRef::Named(n) = &f.return_type {
                if opaque_type_names.contains(n.as_str()) && !client_type_names.contains(n.as_str()) {
                    return Some(n.as_str());
                }
            }
            None
        })
        .collect();

    for type_name in &top_level_opaque_returns {
        let free_name = destructor_method_name(type_name);
        let free_symbol = jni_symbol(&package, &bridge, &free_name);
        emit_destructor_shim(&mut out, &free_symbol, type_name);
    }

    // Trait-bridge shims (Java_*_nativeRegister<Trait> / nativeUnregister<Trait> /
    // nativeClear<Trait>s).  Bridges with `kotlin_android` in `exclude_languages`
    // are skipped.
    emit_trait_bridge_shims(&mut out, config, api, &package, &bridge);

    out
}
