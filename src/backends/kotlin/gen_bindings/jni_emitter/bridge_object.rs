// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

/// Emit `<PascalCrateName>Bridge.kt` — a Kotlin `object` containing:
/// - `init { System.loadLibrary("<crate>_jni") }`
/// - `external fun native<Method>(...)` for every visible API function
/// - `external fun native{Owner}{Adapter}{Start,Next,Free}` for every
///   streaming adapter with an `owner_type`.
pub fn emit_jni_bridge_object(api: &ApiSurface, config: &ResolvedCrateConfig) -> GeneratedFile {
    let module_name = to_pascal_case(&config.name);
    let bridge_name = format!("{module_name}Bridge");
    let exception_class = format!("{bridge_name}Exception");
    let lib_name = config.jni_lib_name();
    let package = jni_kotlin_package(config);

    let exclude_functions: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_else(|| {
            config
                .kotlin
                .as_ref()
                .map(|k| k.exclude_functions.iter().map(String::as_str).collect())
                .unwrap_or_default()
        });

    let visible_functions: Vec<_> = api
        .functions
        .iter()
        .filter(|f| {
            !exclude_functions.contains(f.name.as_str()) && !trait_bridge_manages_jni_function(f.name.as_str(), config)
        })
        .collect();

    let opaque_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

    let kotlin_android_capsule_types: std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig> =
        config
            .kotlin_android
            .as_ref()
            .map(|c| c.capsule_types.clone())
            .unwrap_or_default();

    let mut body = String::new();
    body.push_str(&template_env::render(
        "jni_bridge_object_header.jinja",
        minijinja::context! {
            bridge_name => bridge_name,
            lib_name => lib_name,
        },
    ));

    let mut emitted_native_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut emitted_destructor_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    for f in &visible_functions {
        let native_name = format!("native{}", to_pascal_case(&f.name));
        emitted_native_names.insert(native_name.clone());

        let return_ty = if is_capsule_function(f, &kotlin_android_capsule_types) {
            "Long"
        } else {
            jni_return_type_for_function(&f.return_type, &opaque_type_names)
        };

        let jni_params = jni_params_for_function(f, &opaque_type_names);
        body.push('\n');
        push_jni_external_fun(
            &mut body,
            &native_name,
            &jni_params,
            non_unit_return_type(&f.return_type, return_ty),
            Some(&exception_class),
        );
    }

    let methods_emitted_before = body.matches("// JNI external funs for client instance methods").count();
    emit_method_jni_external_funs(
        &mut body,
        api,
        &exclude_functions,
        &exception_class,
        &mut emitted_destructor_names,
    );
    let methods_emitted_after = body.matches("// JNI external funs for client instance methods").count();

    if methods_emitted_before == methods_emitted_after {
        let opaque_with_methods: Vec<_> = api
            .types
            .iter()
            .filter(|t| {
                t.is_opaque
                    && !t.is_trait
                    && !t.methods.is_empty()
                    && !exclude_functions
                        .iter()
                        .all(|&excluded| t.methods.iter().all(|m| excluded == m.name.as_str()))
            })
            .collect();
        if !opaque_with_methods.is_empty() {
            body.push_str("\n    // JNI external funs for client instance methods (fallback).\n");
            for ty in &opaque_with_methods {
                let owner_pascal = to_pascal_case(&ty.name);
                for method in &ty.methods {
                    if exclude_functions.contains(method.name.as_str()) {
                        continue;
                    }
                    let native_name = format!("native{owner_pascal}{}", to_pascal_case(&method.name));
                    let return_ty = jni_return_type_for_method(&method.return_type, &opaque_type_names);
                    let params = if method.params.is_empty() {
                        "handle: Long".to_string()
                    } else if method.params.len() == 1 && is_binary_param_type(&method.params[0].ty) {
                        format!("handle: Long, {}: ByteArray", to_lower_camel(&method.params[0].name))
                    } else {
                        "handle: Long, requestJson: String".to_string()
                    };
                    push_jni_external_fun(
                        &mut body,
                        &native_name,
                        &params,
                        non_unit_return_type(&method.return_type, return_ty),
                        Some(&exception_class),
                    );
                }
            }
        }
    }

    emit_streaming_jni_external_funs(&mut body, config, &exception_class);

    emit_constructor_jni_external_funs(&mut body, api, config, &exception_class);

    emit_trait_bridge_jni_external_funs(&mut body, config, &exception_class, &package, &emitted_native_names);

    let client_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && t.methods.iter().any(|m| !m.sanitized && !m.is_static))
        .map(|t| t.name.as_str())
        .collect();

    let handle_only_opaque_returns: std::collections::BTreeSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && !client_type_names.contains(t.name.as_str()))
        .map(|t| t.name.as_str())
        .collect();

    if !handle_only_opaque_returns.is_empty() {
        body.push_str("\n    // Destructor external funs for handle-only opaque types.\n");
        for type_name in &handle_only_opaque_returns {
            let free_name = format!("nativeFree{}", to_pascal_case(type_name));
            if !emitted_destructor_names.contains(&free_name) {
                push_jni_external_fun(&mut body, &free_name, "handle: Long", None, None);
            }
        }
    }

    body.push_str("}\n");

    let content = template_env::render(
        "jni_bridge_file.jinja",
        minijinja::context! {
            package => package,
            body => body,
        },
    );

    let path = jni_output_path(config, &format!("{bridge_name}.kt"));
    GeneratedFile {
        path,
        content,
        generated_header: false,
    }
}

fn trait_bridge_manages_jni_function(func_name: &str, config: &ResolvedCrateConfig) -> bool {
    let language_name = if config.kotlin_android.is_some() {
        "kotlin_android"
    } else {
        "kotlin"
    };
    config.trait_bridges.iter().any(|bridge| {
        !bridge.exclude_languages.iter().any(|lang| lang == language_name)
            && (bridge.register_fn.as_deref() == Some(func_name)
                || bridge.unregister_fn.as_deref() == Some(func_name)
                || bridge.clear_fn.as_deref() == Some(func_name))
    })
}
