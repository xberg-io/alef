#[allow(clippy::too_many_arguments)]
fn emit_client_shims(
    out: &mut String,
    ty: &TypeDef,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    package: &str,
    bridge: &str,
    exclude_functions: &std::collections::HashSet<&str>,
    opaque_type_names: &std::collections::HashSet<&str>,
) {
    emit_client_method_shims(out, ty, config, package, bridge, exclude_functions, opaque_type_names);
    emit_client_lifecycle_shims(out, ty, config, package, bridge);
    emit_client_streaming_shims(out, ty, api, config, package, bridge);
}

#[allow(clippy::too_many_arguments)]
fn emit_client_method_shims(
    out: &mut String,
    ty: &TypeDef,
    config: &ResolvedCrateConfig,
    package: &str,
    bridge: &str,
    exclude_functions: &std::collections::HashSet<&str>,
    opaque_type_names: &std::collections::HashSet<&str>,
) {
    let capsule_types = jni_capsule_types(config);
    for method in ty.methods.iter().filter(|m| !m.sanitized && !m.is_static) {
        if exclude_functions.contains(method.name.as_str()) {
            continue;
        }
        let method_name = bridge_method_name(&ty.name, &method.name);
        let symbol = jni_symbol(package, bridge, &method_name);
        let receiver_is_mut = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));
        let receiver_owned = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::Owned));
        emit_method_shim(
            out,
            &symbol,
            &ty.name,
            method,
            receiver_is_mut,
            receiver_owned,
            opaque_type_names,
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
