pub(crate) fn emit_lib_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let package = jni_kotlin_package(config);
    let bridge = bridge_class_name(&config.name);
    let core_crate = core_use_path(config);
    let error_class = resolve_error_class(config, &package);
    let enabled_features: std::collections::HashSet<&str> = config
        .features_for_language(Language::KotlinAndroid)
        .iter()
        .map(String::as_str)
        .collect();
    let cfg_filtered_api = api.with_cfg_filtered_deep(&enabled_features);

    let mut out = String::new();

    out.push_str(&template_env::render(
        "lib_header.rs.jinja",
        context! {
            core_crate => core_crate,
            error_class => error_class,
            crate_attributes => crate::codegen::shared::format_crate_attributes(&config.crate_attributes),
        },
    ));

    for trait_path in collect_trait_imports(&cfg_filtered_api) {
        out.push_str(&format!("use {trait_path};\n"));
    }

    emit_runtime_helpers(&mut out);

    let exclude_functions: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();

    // Type-level exclusions inherited from the shared `[crates.ffi].exclude_types`
    // plus the paired `[crates.kotlin_android].exclude_types` list, mirroring the
    // kotlin_android binding backend's `effective_exclude_types`. Without this the
    // JNI shims expose opaque client types (and any top-level function whose
    // signature references them) that every other FFI-derived binding drops.
    let exclude_types: std::collections::HashSet<&str> = {
        let mut set: std::collections::HashSet<&str> = config
            .ffi
            .as_ref()
            .map(|ffi| ffi.exclude_types.iter().map(String::as_str).collect())
            .unwrap_or_default();
        if let Some(ka) = config.kotlin_android.as_ref() {
            set.extend(ka.exclude_types.iter().map(String::as_str));
        }
        set
    };
    let signature_references_excluded = |params: &[ParamDef], return_type: &TypeRef| -> bool {
        let references_excluded = |ty: &TypeRef| exclude_types.iter().any(|name| ty.references_named(name));
        references_excluded(return_type) || params.iter().any(|param| references_excluded(&param.ty))
    };

    let trait_bridge_fn_names: std::collections::HashSet<&str> = config
        .trait_bridges
        .iter()
        .flat_map(|b| {
            [&b.register_fn, &b.unregister_fn, &b.clear_fn]
                .into_iter()
                .filter_map(|opt| opt.as_deref())
        })
        .collect();

    // JNI exposes one native symbol per function, so select the variant compiled by the
    // configured Android feature set before collapsing same-named real/fallback entries. ~keep
    let deduped_functions = crate::codegen::fn_dedup::dedup_same_name_functions(&cfg_filtered_api.functions);
    let visible_functions: Vec<_> = deduped_functions
        .iter()
        .filter(|f| {
            !f.sanitized
                && !exclude_functions.contains(f.name.as_str())
                && !trait_bridge_fn_names.contains(f.name.as_str())
                && !signature_references_excluded(&f.params, &f.return_type)
        })
        .collect();

    let opaque_type_names: std::collections::HashSet<&str> = cfg_filtered_api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

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

    let client_types: Vec<_> = cfg_filtered_api
        .types
        .iter()
        .filter(|t| {
            t.is_opaque
                && !t.is_trait
                && !exclude_types.contains(t.name.as_str())
                && t.methods.iter().any(|m| !m.sanitized && !m.is_static)
        })
        .collect();
    let client_type_names: std::collections::HashSet<&str> = client_types.iter().map(|t| t.name.as_str()).collect();

    for ty in &client_types {
        emit_client_shims(
            &mut out,
            ty,
            &cfg_filtered_api,
            config,
            &package,
            &bridge,
            &exclude_functions,
            &opaque_type_names,
        );
    }

    // Instance methods on value (data-class) types. These carry no handle, so the
    // shim rebuilds the receiver from JSON — see `value_method_shims.rs`.
    let serde_type_names = value_bridge_serde_type_names(&cfg_filtered_api);
    for ty in cfg_filtered_api
        .types
        .iter()
        .filter(|t| !t.is_opaque && !t.is_trait && !t.binding_excluded && !exclude_types.contains(t.name.as_str()))
    {
        emit_value_type_shims(&mut out, ty, &package, &bridge, &serde_type_names);
    }

    let top_level_opaque_returns: std::collections::HashSet<&str> = visible_functions
        .iter()
        .filter_map(|f| {
            if let TypeRef::Named(n) = &f.return_type
                && opaque_type_names.contains(n.as_str())
                && !client_type_names.contains(n.as_str())
            {
                return Some(n.as_str());
            }
            None
        })
        .collect();

    for type_name in &top_level_opaque_returns {
        let free_name = destructor_method_name(type_name);
        let free_symbol = jni_symbol(&package, &bridge, &free_name);
        emit_destructor_shim(&mut out, &free_symbol, type_name);
    }

    emit_trait_bridge_shims(&mut out, config, &cfg_filtered_api, &package, &bridge);

    if !config.components.is_empty() {
        emit_unsupported_component_shims(&mut out, &package, &bridge);
    }

    out
}
