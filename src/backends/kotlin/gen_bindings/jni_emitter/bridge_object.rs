// ---------------------------------------------------------------------------
// Bridge object emitter
// ---------------------------------------------------------------------------

/// Emit `<PascalCrateName>Bridge.kt` — a Kotlin `object` containing:
/// - `init { System.loadLibrary("<crate>_jni") }`
/// - `external fun native<Method>(...)` for every visible API function
/// - `external fun native{Owner}{Adapter}{Start,Next,Free}` for every
///   streaming adapter with an `owner_type`.
pub fn emit_jni_bridge_object(api: &ApiSurface, config: &ResolvedCrateConfig) -> GeneratedFile {
    let module_name = to_pascal_case(&config.name);
    let bridge_name = format!("{module_name}Bridge");
    // The exception class is emitted alongside the Bridge object and referenced in
    // @Throws annotations so that callers can catch typed JNI errors.
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

    // Opaque type names: Named params of this shape are handles (Long), not JSON (String).
    let opaque_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

    // Host-native capsule (Language) passthrough configuration from kotlin_android.capsule_types.
    let kotlin_android_capsule_types: std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig> =
        config
            .kotlin_android
            .as_ref()
            .map(|c| c.capsule_types.clone())
            .unwrap_or_default();

    let mut body = String::new();
    // Suppress detekt TooManyFunctions: the bridge object has one external fun
    // per API function; large APIs naturally exceed the default threshold of 11.
    body.push_str(&template_env::render(
        "jni_bridge_object_header.jinja",
        minijinja::context! {
            bridge_name => bridge_name,
            lib_name => lib_name,
        },
    ));

    // Collect native function names from the API to detect duplicates later.
    let mut emitted_native_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Track destructor names that have been emitted to avoid duplication.
    let mut emitted_destructor_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Emit one `external fun` per visible API function.
    // Every native method is annotated @Throws so typed catch blocks work in
    // Kotlin/Java callers — without this the JNI RuntimeException is wrapped in
    // UndeclaredThrowableException and silently bypasses catch(BridgeException).
    for f in &visible_functions {
        let native_name = format!("native{}", to_pascal_case(&f.name));
        emitted_native_names.insert(native_name.clone());

        // Host-native capsule functions return Long (the raw pointer), not the JSON serialization
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

    // Emit external funs for instance methods on opaque client types.
    let methods_emitted_before = body.matches("// JNI external funs for client instance methods").count();
    emit_method_jni_external_funs(
        &mut body,
        api,
        &exclude_functions,
        &exception_class,
        &mut emitted_destructor_names,
    );
    let methods_emitted_after = body.matches("// JNI external funs for client instance methods").count();

    // Fallback: if emit_method_jni_external_funs didn't emit the comment (no client types found),
    // manually emit declarations for any opaque types with methods that the client generator found.
    if methods_emitted_before == methods_emitted_after {
        // Try to find opaque client types by looking for those with methods
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

    // Emit streaming external funs.
    emit_streaming_jni_external_funs(&mut body, config, &exception_class);

    // Emit nativeNew<TypeName> external funs for client_constructors entries.
    emit_constructor_jni_external_funs(&mut body, api, config, &exception_class);

    // Emit nativeRegister<Trait> / nativeUnregister<Trait> / nativeClear<Trait>s
    // external funs for every [[crates.trait_bridges]] entry whose configuration
    // does not exclude `kotlin_android`. Skip duplicates already emitted from the API.
    emit_trait_bridge_jni_external_funs(&mut body, config, &exception_class, &package, &emitted_native_names);

    // Emit nativeFreeXxx destructors for opaque types returned by top-level functions
    // that do NOT have instance methods. Client type destructors are already emitted
    // by emit_method_jni_external_funs at line 326 for ALL types with methods,
    // including those that may also be returned by top-level functions.
    let client_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && t.methods.iter().any(|m| !m.sanitized && !m.is_static))
        .map(|t| t.name.as_str())
        .collect();

    // Emit a `nativeFree<TypeName>` destructor for every opaque non-trait type
    // that is NOT a client. This mirrors the kotlin_android wrapper emitter
    // (`gen_bindings::emit_module_kt`), which now materialises an
    // AutoCloseable wrapper class for every opaque non-client type — its
    // `close()` body calls `Bridge.nativeFree{TypeName}(handle)`, so the JNI
    // bridge MUST declare a matching external fun or Kotlin compilation fails
    // with `Unresolved reference 'nativeFree<TypeName>'`.
    //
    // The previous filter only considered return types of top-level
    // functions, which missed opaque types whose only public entrypoint is a
    // static factory method (kept as `@staticmethod` on the class rather than
    // lifted to a free function in alef's IR — e.g. `TokenCounter::new()`).
    // The FFI layer still emits the `{prefix}_{type_snake}_free` C symbol
    // unconditionally for every opaque type, so the JNI side has a real
    // function to bind against.
    let handle_only_opaque_returns: std::collections::BTreeSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && !client_type_names.contains(t.name.as_str()))
        .map(|t| t.name.as_str())
        .collect();

    // Emit destructors ONLY for handle-only types (top-level returns, not client types).
    // Skip any that were already emitted to avoid duplicates.
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
