/// Groups the shim-emission context shared by [`emit_client_method_shims`] that would otherwise
/// push it over the parameter-count lint. Local to this file: `emit_client_shims` constructs it
/// once per type from its own (unchanged) parameter list. ~keep
struct ClientMethodShimContext<'a> {
    config: &'a ResolvedCrateConfig,
    package: &'a str,
    bridge: &'a str,
    exclude_functions: &'a std::collections::HashSet<&'a str>,
    opaque_type_names: &'a std::collections::HashSet<&'a str>,
}

/// The per-crate context `emit_client_shims` needs for every client type it is called with.
/// Local to this (merged, `include!`-based) `gen_shims` module: built once by the caller,
/// `emit_jni_client_type_shims` in `top_level.rs`, outside its per-type loop. ~keep
struct ClientTypeShimContext<'a> {
    api: &'a ApiSurface,
    config: &'a ResolvedCrateConfig,
    package: &'a str,
    bridge: &'a str,
    exclude_functions: &'a std::collections::HashSet<&'a str>,
    opaque_type_names: &'a std::collections::HashSet<&'a str>,
}

fn emit_client_shims(out: &mut String, ty: &TypeDef, context: &ClientTypeShimContext<'_>) {
    let method_context = ClientMethodShimContext {
        config: context.config,
        package: context.package,
        bridge: context.bridge,
        exclude_functions: context.exclude_functions,
        opaque_type_names: context.opaque_type_names,
    };
    emit_client_method_shims(out, ty, &method_context);
    emit_client_lifecycle_shims(out, ty, context.config, context.package, context.bridge);
    emit_client_streaming_shims(out, ty, context.api, context.config, context.package, context.bridge);
}

fn emit_client_method_shims(out: &mut String, ty: &TypeDef, context: &ClientMethodShimContext<'_>) {
    let capsule_types = jni_capsule_types(context.config);
    for method in ty.methods.iter().filter(|m| !m.sanitized && !m.is_static) {
        if context.exclude_functions.contains(method.name.as_str()) {
            continue;
        }
        let method_name = bridge_method_name(&ty.name, &method.name);
        let symbol = jni_symbol(context.package, context.bridge, &method_name);
        let receiver = MethodShimReceiver {
            type_name: &ty.name,
            receiver_is_mut: matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut)),
            receiver_owned: matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::Owned)),
        };
        emit_method_shim(
            out,
            &symbol,
            method,
            &receiver,
            context.opaque_type_names,
            &capsule_types,
        );
    }
}

fn emit_client_lifecycle_shims(
    out: &mut String,
    ty: &TypeDef,
    config: &ResolvedCrateConfig,
    package: &str,
    bridge: &str,
) {
    let free_name = destructor_method_name(&ty.name);
    let free_symbol = jni_symbol(package, bridge, &free_name);
    emit_destructor_shim(out, &free_symbol, &ty.name);

    if let Some(ctor) = config.client_constructors.get(&ty.name) {
        let ctor_method_name = format!("nativeNew{}", ty.name);
        let ctor_symbol = jni_symbol(package, bridge, &ctor_method_name);
        emit_constructor_shim(out, &ctor_symbol, ty, config, ctor);
    }
}

fn emit_client_streaming_shims(
    out: &mut String,
    ty: &TypeDef,
    _api: &ApiSurface,
    config: &ResolvedCrateConfig,
    package: &str,
    bridge: &str,
) {
    let streaming = config.adapters.iter().filter(|adapter| {
        matches!(adapter.pattern, AdapterPattern::Streaming) && adapter.owner_type.as_deref() == Some(ty.name.as_str())
    });
    for adapter in streaming {
        let (start_name, next_name, free_adapter_name) = streaming_method_names(&ty.name, &adapter.name);
        let start_symbol = jni_symbol(package, bridge, &start_name);
        let next_symbol = jni_symbol(package, bridge, &next_name);
        let free_symbol = jni_symbol(package, bridge, &free_adapter_name);
        emit_streaming_shims(out, &start_symbol, &next_symbol, &free_symbol, ty, adapter);
    }
}
