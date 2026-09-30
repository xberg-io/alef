// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

struct JniBridgeInputs<'a> {
    bridge_name: String,
    exception_class: String,
    lib_name: String,
    package: String,
    exclude_functions: std::collections::HashSet<String>,
    visible_functions: Vec<&'a crate::core::ir::FunctionDef>,
    opaque_type_names: std::collections::HashSet<&'a str>,
    capsule_types: std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
}

/// Emit the Kotlin Bridge object containing every JNI `external fun` declaration.
pub fn emit_jni_bridge_object(api: &ApiSurface, config: &ResolvedCrateConfig) -> GeneratedFile {
    let inputs = jni_bridge_inputs(api, config);
    let mut body = template_env::render(
        "jni_bridge_object_header.jinja",
        minijinja::context! {
            bridge_name => inputs.bridge_name,
            lib_name => inputs.lib_name,
        },
    );
    let mut native_names = std::collections::HashSet::new();
    let mut destructor_names = std::collections::HashSet::new();
    emit_top_level_jni_external_funs(&mut body, &inputs, &mut native_names);
    emit_bridge_method_externals(api, &inputs, &mut body, &mut destructor_names);
    emit_value_method_jni_external_funs(&mut body, api, &inputs.exception_class);
    emit_streaming_jni_external_funs(&mut body, config, &inputs.exception_class);
    emit_constructor_jni_external_funs(&mut body, api, config, &inputs.exception_class);
    emit_trait_bridge_jni_external_funs(
        &mut body,
        config,
        &inputs.exception_class,
        &inputs.package,
        &native_names,
    );
    emit_handle_only_destructors(api, &inputs, &mut body, &mut destructor_names);
    render_jni_bridge_file(config, &inputs, body)
}

/// The `[crates.kotlin_android]`/`[crates.kotlin] exclude_functions` union that gates every
/// Kotlin-side emission -- the Bridge object's own `external fun` declarations, the wrapper
/// classes' calls, and destructor reachability. Deliberately excludes
/// `[crates.jni].exclude_functions`: that list only tells the JNI shim generator to skip
/// implementing one function's *own* native symbol (e.g. because the consumer hand-writes it),
/// it does not tell Kotlin to stop calling the function. See [`kotlin_visible_functions`]. ~keep
pub fn kotlin_exclude_functions(config: &ResolvedCrateConfig) -> std::collections::HashSet<String> {
    config
        .kotlin_android
        .as_ref()
        .map(|android| android.exclude_functions.iter().cloned().collect())
        .unwrap_or_else(|| {
            config
                .kotlin
                .as_ref()
                .map(|kotlin| kotlin.exclude_functions.iter().cloned().collect())
                .unwrap_or_default()
        })
}

/// Top-level functions the Kotlin/kotlin_android Bridge object actually declares and calls.
///
/// This is the reachability set every consumer of [`handle_only_type_names`] must build from --
/// including the JNI shim generator (`src/backends/jni`), which has its own, JNI-shim-only
/// exclusion list (`[crates.jni].exclude_functions`, see [`kotlin_exclude_functions`]) that must
/// never narrow which opaque return types still need a `nativeFree<Type>`: Kotlin keeps calling
/// a function -- and keeps needing to free what it returns -- even when the JNI backend was told
/// to skip generating that one function's own native shim. Feeding the JNI destructor emitter
/// this function (instead of its own jni-exclusion-narrowed function list) is what keeps the
/// Bridge object's destructor declarations and the JNI shim's destructor implementations from
/// drifting apart again. ~keep
pub fn kotlin_visible_functions<'a>(
    api: &'a ApiSurface,
    config: &ResolvedCrateConfig,
) -> Vec<&'a crate::core::ir::FunctionDef> {
    let exclude_functions = kotlin_exclude_functions(config);
    api.functions
        .iter()
        .filter(|function| {
            !function.sanitized
                && !exclude_functions.contains(function.name.as_str())
                && !trait_bridge_manages_jni_function(function.name.as_str(), config)
        })
        .collect()
}

fn jni_bridge_inputs<'a>(api: &'a ApiSurface, config: &ResolvedCrateConfig) -> JniBridgeInputs<'a> {
    let bridge_name = format!("{}Bridge", to_pascal_case(&config.name));
    let exclude_functions = kotlin_exclude_functions(config);
    let visible_functions = kotlin_visible_functions(api, config);
    JniBridgeInputs {
        exception_class: format!("{bridge_name}Exception"),
        bridge_name,
        lib_name: config.jni_lib_name(),
        package: jni_kotlin_package(config),
        exclude_functions,
        visible_functions,
        opaque_type_names: opaque_type_names(api),
        capsule_types: kotlin_android_capsule_types(config),
    }
}

fn opaque_type_names(api: &ApiSurface) -> std::collections::HashSet<&str> {
    api.types
        .iter()
        .filter(|type_def| type_def.is_opaque && !type_def.is_trait)
        .map(|type_def| type_def.name.as_str())
        .collect()
}

fn kotlin_android_capsule_types(
    config: &ResolvedCrateConfig,
) -> std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig> {
    config
        .kotlin_android
        .as_ref()
        .map(|android| android.capsule_types.clone())
        .unwrap_or_default()
}

fn emit_top_level_jni_external_funs(
    body: &mut String,
    inputs: &JniBridgeInputs<'_>,
    native_names: &mut std::collections::HashSet<String>,
) {
    for function in &inputs.visible_functions {
        let native_name = format!("native{}", to_pascal_case(&function.name));
        native_names.insert(native_name.clone());
        let return_type = if is_capsule_function(function, &inputs.capsule_types) {
            "Long"
        } else {
            jni_return_type_for_function(&function.return_type, &inputs.opaque_type_names)
        };
        body.push('\n');
        push_jni_external_fun(
            body,
            &native_name,
            &jni_params_for_function(function, &inputs.opaque_type_names),
            non_unit_return_type(&function.return_type, return_type),
            Some(&inputs.exception_class),
        );
    }
}

fn emit_bridge_method_externals(
    api: &ApiSurface,
    inputs: &JniBridgeInputs<'_>,
    body: &mut String,
    destructor_names: &mut std::collections::HashSet<String>,
) {
    let marker = "// JNI external funs for client instance methods";
    let methods_before = body.matches(marker).count();
    emit_method_jni_external_funs(
        body,
        api,
        &inputs.exclude_functions,
        &inputs.capsule_types,
        &inputs.exception_class,
        destructor_names,
    );
    if methods_before == body.matches(marker).count() {
        emit_fallback_method_externals(api, inputs, body);
    }
}

fn emit_fallback_method_externals(api: &ApiSurface, inputs: &JniBridgeInputs<'_>, body: &mut String) {
    let types = api.types.iter().filter(|type_def| {
        type_def.is_opaque
            && !type_def.is_trait
            && type_def
                .methods
                .iter()
                .any(|method| !method.sanitized && !method.is_static)
            && !inputs
                .exclude_functions
                .iter()
                .all(|excluded| type_def.methods.iter().all(|method| excluded == &method.name))
    });
    let mut emitted_header = false;
    for type_def in types {
        if !emitted_header {
            body.push_str("\n    // JNI external funs for client instance methods (fallback).\n");
            emitted_header = true;
        }
        emit_fallback_type_methods(type_def, inputs, body);
    }
}

fn emit_fallback_type_methods(type_def: &crate::core::ir::TypeDef, inputs: &JniBridgeInputs<'_>, body: &mut String) {
    let owner_pascal = to_pascal_case(&type_def.name);
    for method in &type_def.methods {
        if method.sanitized || method.is_static || inputs.exclude_functions.contains(method.name.as_str()) {
            continue;
        }
        let native_name = format!("native{owner_pascal}{}", to_pascal_case(&method.name));
        let return_type =
            jni_return_type_for_method(&method.return_type, &inputs.opaque_type_names, &inputs.capsule_types);
        push_jni_external_fun(
            body,
            &native_name,
            &method_jni_params(method),
            non_unit_return_type(&method.return_type, return_type),
            Some(&inputs.exception_class),
        );
    }
}

fn emit_handle_only_destructors(
    api: &ApiSurface,
    inputs: &JniBridgeInputs<'_>,
    body: &mut String,
    destructor_names: &mut std::collections::HashSet<String>,
) {
    let client_types: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|type_def| {
            type_def.is_opaque
                && !type_def.is_trait
                && type_def
                    .methods
                    .iter()
                    .any(|method| !method.sanitized && !method.is_static)
        })
        .map(|type_def| type_def.name.as_str())
        .collect();
    let capsule_type_names: std::collections::HashSet<&str> = inputs.capsule_types.keys().map(String::as_str).collect();
    let returns = handle_only_type_names(
        api,
        &inputs.visible_functions,
        &inputs.exclude_functions,
        &inputs.opaque_type_names,
        &capsule_type_names,
        &client_types,
    );
    if returns.is_empty() {
        return;
    }
    body.push_str("\n    // Destructor external funs for handle-only opaque types.\n");
    for type_name in &returns {
        let free_name = format!("nativeFree{}", to_pascal_case(type_name));
        if destructor_names.insert(free_name.clone()) {
            push_jni_external_fun(body, &free_name, "handle: Long", None, None);
        }
    }
}

/// Opaque type names that exist on the Kotlin side purely as JNI handle wrappers -- no
/// client class, no capsule host type -- because some visible top-level function or
/// instance method returns them. A capsule type (`[crates.kotlin_android.capsule_types]`)
/// is deliberately excluded: its host runtime owns and frees the pointer, so it never gets
/// a `nativeFree<Type>` destructor.
///
/// Shared by [`emit_handle_only_destructors`] (declares each `nativeFree<Type>` external
/// fun the Bridge object owns), the kotlin_android handle-wrapper emitter
/// (`kotlin_android::gen_bindings::module_facade::handle_wrappers`, which emits the `.kt`
/// wrapper class whose `close()` calls it), and the `jni` shim generator
/// (`src/backends/jni/gen_shims/top_level.rs`, which must implement the exact same set of
/// `Java_..._nativeFree<Type>` symbols) -- so the declaration, its call site, and its native
/// implementation can never name a different set of types. Returns owned names rather than
/// borrowing from `api` so every call site can build it from its own differently-lifetimed
/// inputs without all three having to share a single lifetime parameter.
///
/// `visible_functions` and `exclude_functions` must come from [`kotlin_visible_functions`] /
/// [`kotlin_exclude_functions`] (or an equivalent that mirrors what Kotlin actually calls) --
/// never from a native-shim-only exclusion list such as `[crates.jni].exclude_functions`, which
/// narrows only which function gets its *own* generated native shim, not which functions Kotlin
/// still calls (and therefore which return types still need freeing). ~keep
pub fn handle_only_type_names(
    api: &ApiSurface,
    visible_functions: &[&crate::core::ir::FunctionDef],
    exclude_functions: &std::collections::HashSet<String>,
    opaque_type_names: &std::collections::HashSet<&str>,
    capsule_type_names: &std::collections::HashSet<&str>,
    client_types: &std::collections::HashSet<&str>,
) -> std::collections::BTreeSet<String> {
    let function_returns = visible_functions
        .iter()
        .filter_map(|function| named_return_type(&function.return_type));
    function_returns
        .chain(visible_method_return_names(api, exclude_functions))
        .filter(|type_name| {
            opaque_type_names.contains(*type_name)
                && !client_types.contains(*type_name)
                && !capsule_type_names.contains(*type_name)
        })
        .map(str::to_string)
        .collect()
}

fn visible_method_return_names<'a>(
    api: &'a ApiSurface,
    excluded_functions: &'a std::collections::HashSet<String>,
) -> impl Iterator<Item = &'a str> {
    api.types
        .iter()
        .filter(|type_def| type_def.is_opaque && !type_def.is_trait && !type_def.binding_excluded)
        .flat_map(|type_def| type_def.methods.iter())
        .filter(|method| !method.sanitized && !method.is_static && !excluded_functions.contains(method.name.as_str()))
        .filter_map(|method| named_return_type(&method.return_type))
}

fn render_jni_bridge_file(
    config: &ResolvedCrateConfig,
    inputs: &JniBridgeInputs<'_>,
    mut body: String,
) -> GeneratedFile {
    body.push_str("}\n");
    let content = template_env::render(
        "jni_bridge_file.jinja",
        minijinja::context! {
            package => inputs.package,
            body => body,
        },
    );
    GeneratedFile {
        path: jni_output_path(config, &format!("{}.kt", inputs.bridge_name)),
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
    config
        .trait_bridges_for(crate::core::config::Language::Kotlin)
        .any(|bridge| {
            !bridge.exclude_languages.iter().any(|lang| lang == language_name)
                && (bridge.register_fn.as_deref() == Some(func_name)
                    || bridge.unregister_fn.as_deref() == Some(func_name)
                    || bridge.clear_fn.as_deref() == Some(func_name))
        })
}
