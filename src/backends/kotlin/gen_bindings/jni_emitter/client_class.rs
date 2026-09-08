/// Inputs shared while emitting a JNI client class file.
struct JniClientInputs<'a> {
    client_types: Vec<&'a crate::core::ir::TypeDef>,
    exclude_functions: std::collections::HashSet<&'a str>,
    bridge_name: String,
    package: String,
    opaque_type_names: std::collections::HashSet<&'a str>,
    streaming_adapters: Vec<&'a crate::core::config::AdapterConfig>,
}

/// Emit `DefaultClient.kt` for JNI mode, or `None` when no client type exists.
pub fn emit_jni_client_class(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    package: Option<&str>,
) -> Option<GeneratedFile> {
    let inputs = jni_client_inputs(api, config, package)?;
    let mut imports = collect_jni_client_imports(&inputs);
    collect_jni_client_type_imports(&inputs, &mut imports);
    let mut body = String::new();
    for type_def in &inputs.client_types {
        emit_jni_client_type(type_def, api, config, &inputs, &mut body, &mut imports);
    }
    let content = template_env::render(
        "jni_client_file.jinja",
        minijinja::context! {
            package => inputs.package,
            imports => imports.into_iter().collect::<Vec<_>>(),
            body => body,
        },
    );
    Some(GeneratedFile {
        path: jni_output_path(config, "DefaultClient.kt"),
        content,
        generated_header: false,
    })
}

fn jni_client_inputs<'a>(
    api: &'a ApiSurface,
    config: &'a ResolvedCrateConfig,
    package: Option<&str>,
) -> Option<JniClientInputs<'a>> {
    let client_types: Vec<_> = api
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
        .collect();
    if client_types.is_empty() {
        return None;
    }
    let exclude_functions = config
        .kotlin_android
        .as_ref()
        .map(|android| android.exclude_functions.iter().map(String::as_str).collect())
        .or_else(|| {
            config
                .kotlin
                .as_ref()
                .map(|kotlin| kotlin.exclude_functions.iter().map(String::as_str).collect())
        })
        .unwrap_or_default();
    Some(JniClientInputs {
        streaming_adapters: jni_client_streaming_adapters(config, &client_types),
        client_types,
        exclude_functions,
        bridge_name: format!("{}Bridge", to_pascal_case(&config.name)),
        package: package
            .map(str::to_string)
            .unwrap_or_else(|| jni_kotlin_package(config)),
        opaque_type_names: opaque_type_names(api),
    })
}

fn jni_client_streaming_adapters<'a>(
    config: &'a ResolvedCrateConfig,
    client_types: &[&crate::core::ir::TypeDef],
) -> Vec<&'a crate::core::config::AdapterConfig> {
    config
        .adapters
        .iter()
        .filter(|adapter| matches!(adapter.pattern, AdapterPattern::Streaming))
        .filter(|adapter| !adapter.skip_languages.iter().any(|language| language == "kotlin"))
        .filter(|adapter| {
            adapter
                .owner_type
                .as_deref()
                .is_some_and(|owner| client_types.iter().any(|type_def| type_def.name == owner))
        })
        .collect()
}

fn collect_jni_client_imports(inputs: &JniClientInputs<'_>) -> BTreeSet<String> {
    let mut imports = BTreeSet::new();
    let has_async = inputs.client_types.iter().any(|type_def| {
        type_def
            .methods
            .iter()
            .any(|method| !method.sanitized && method.is_async)
    });
    if has_async || !inputs.streaming_adapters.is_empty() {
        imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
        imports.insert("import kotlinx.coroutines.withContext".to_string());
    }
    if !inputs.streaming_adapters.is_empty() {
        imports.insert("import kotlinx.coroutines.flow.Flow".to_string());
        imports.insert("import kotlinx.coroutines.flow.callbackFlow".to_string());
        imports.insert("import kotlinx.coroutines.channels.awaitClose".to_string());
    }
    imports
}

fn collect_jni_client_type_imports(inputs: &JniClientInputs<'_>, imports: &mut BTreeSet<String>) {
    for type_def in &inputs.client_types {
        for method in type_def
            .methods
            .iter()
            .filter(|method| !method.sanitized && !method.is_static)
        {
            kotlin_type_with_string_imports(&method.return_type, false, imports);
            for param in &method.params {
                format_param_with_imports(param, imports);
            }
        }
    }
}

fn emit_jni_client_type(
    type_def: &crate::core::ir::TypeDef,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    inputs: &JniClientInputs<'_>,
    body: &mut String,
    imports: &mut BTreeSet<String>,
) {
    body.push_str(&template_env::render(
        "jni_client_class_header.jinja",
        minijinja::context! { class_name => type_def.name },
    ));
    emit_jni_client_companion(type_def, api, config, &inputs.bridge_name, body);
    emit_jni_client_methods(type_def, config, inputs, body, imports);
    emit_jni_client_streaming_methods(type_def, inputs, body);
    let free_name = format!("nativeFree{}", to_pascal_case(&type_def.name));
    body.push_str(&template_env::render(
        "jni_client_close_method.jinja",
        minijinja::context! {
            bridge_name => inputs.bridge_name,
            free_name => free_name,
        },
    ));
    body.push_str("}\n");
}

fn emit_jni_client_companion(
    type_def: &crate::core::ir::TypeDef,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    bridge_name: &str,
    body: &mut String,
) {
    let has_json_methods = type_def
        .methods
        .iter()
        .filter(|method| !method.sanitized && !method.is_static)
        .any(|method| !method.params.is_empty() || needs_json_deserialize(&method.return_type));
    let constructor = config.client_constructors.get(type_def.name.as_str());
    if !has_json_methods && constructor.is_none() {
        return;
    }
    body.push_str("    companion object {\n");
    if has_json_methods {
        body.push_str("        private val MAPPER = com.fasterxml.jackson.databind.ObjectMapper()\n");
        body.push_str("            .registerModule(com.fasterxml.jackson.datatype.jdk8.Jdk8Module())\n");
        body.push_str("            .findAndRegisterModules()\n");
        body.push_str(
            "            .setPropertyNamingStrategy(com.fasterxml.jackson.databind.PropertyNamingStrategies.SNAKE_CASE)\n",
        );
        body.push_str(
            "            .configure(com.fasterxml.jackson.databind.DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false)\n",
        );
    }
    if let Some(constructor) = constructor {
        emit_jni_client_factory(&type_def.name, bridge_name, constructor, api, body);
    }
    body.push_str("    }\n\n");
}

fn emit_jni_client_methods(
    type_def: &crate::core::ir::TypeDef,
    config: &ResolvedCrateConfig,
    inputs: &JniClientInputs<'_>,
    body: &mut String,
    imports: &mut BTreeSet<String>,
) {
    let methods = type_def.methods.iter().filter(|method| {
        !method.sanitized && !method.is_static && !inputs.exclude_functions.contains(method.name.as_str())
    });
    for method in methods {
        emit_jni_client_method(
            method,
            &type_def.name,
            &inputs.bridge_name,
            body,
            imports,
            &inputs.opaque_type_names,
            config,
        );
    }
}

fn emit_jni_client_streaming_methods(
    type_def: &crate::core::ir::TypeDef,
    inputs: &JniClientInputs<'_>,
    body: &mut String,
) {
    let adapters = inputs
        .streaming_adapters
        .iter()
        .filter(|adapter| adapter.owner_type.as_deref() == Some(type_def.name.as_str()));
    for adapter in adapters {
        emit_jni_streaming_client_method(adapter, &type_def.name, &inputs.bridge_name, body);
    }
}
