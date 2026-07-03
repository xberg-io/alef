/// Emit `DefaultClient.kt` for the JNI mode.
///
/// Emits a `class DefaultClient internal constructor(internal val handle: Long) :
/// AutoCloseable` with:
/// - One `suspend fun` per non-sanitized, non-static instance method, calling
///   `<Module>Bridge.native<Method>(handle, ...)`.
/// - One `Flow<ChunkType>` streaming method per adapter owned by this type,
///   using `callbackFlow` + `handle` (not `inner`) as the first JNI argument.
/// - `override fun close() { <Module>Bridge.nativeFree<ClassName>(handle) }`
///
/// Returns `None` when no client types (opaque, with instance methods) exist.
pub fn emit_jni_client_class(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    package: Option<&str>,
) -> Option<GeneratedFile> {
    let is_client_type = |t: &&crate::core::ir::TypeDef| {
        t.is_opaque && !t.is_trait && t.methods.iter().any(|m| !m.sanitized && !m.is_static)
    };
    let client_types: Vec<_> = api.types.iter().filter(is_client_type).collect();
    if client_types.is_empty() {
        return None;
    }

    // Honour `[crates.kotlin_android].exclude_functions` / `[crates.kotlin].exclude_functions`
    // for instance methods, mirroring the top-level function filter at the start
    // of the bridge-object emitter (line 40-55 above).
    let exclude_functions: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .or_else(|| {
            config
                .kotlin
                .as_ref()
                .map(|k| k.exclude_functions.iter().map(String::as_str).collect())
        })
        .unwrap_or_default();

    let module_name = to_pascal_case(&config.name);
    let bridge_name = format!("{module_name}Bridge");
    let pkg = package
        .map(str::to_string)
        .unwrap_or_else(|| jni_kotlin_package(config));

    // Opaque type names: Named params of this shape are handles (Long), not JSON (String).
    let opaque_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

    let mut imports: BTreeSet<String> = BTreeSet::new();
    let mut body = String::new();

    let has_async = client_types
        .iter()
        .any(|t| t.methods.iter().any(|m| !m.sanitized && m.is_async));
    if has_async {
        imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
        imports.insert("import kotlinx.coroutines.withContext".to_string());
    }

    let streaming_adapters: Vec<_> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter(|a| !a.skip_languages.iter().any(|l| l == "kotlin"))
        .filter(|a| {
            a.owner_type
                .as_deref()
                .map(|owner| client_types.iter().any(|t| t.name == owner))
                .unwrap_or(false)
        })
        .collect();

    if !streaming_adapters.is_empty() {
        imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
        imports.insert("import kotlinx.coroutines.withContext".to_string());
        imports.insert("import kotlinx.coroutines.flow.Flow".to_string());
        imports.insert("import kotlinx.coroutines.flow.callbackFlow".to_string());
        imports.insert("import kotlinx.coroutines.channels.awaitClose".to_string());
    }

    for ty in &client_types {
        let class_name = &ty.name;

        // Pre-scan to collect type imports.
        for m in ty.methods.iter().filter(|m| !m.sanitized && !m.is_static) {
            kotlin_type_with_string_imports(&m.return_type, false, &mut imports);
            for p in &m.params {
                format_param_with_imports(p, &mut imports);
            }
        }
        for adapter in streaming_adapters
            .iter()
            .filter(|a| a.owner_type.as_deref() == Some(class_name.as_str()))
        {
            if let Some(item) = adapter.item_type.as_deref() {
                // Item type only references the simple name; no import needed in same pkg.
                let _ = item;
            }
        }

        // Suppress detekt TooManyFunctions: the number of methods scales with
        // the API surface; large APIs naturally exceed the default threshold of 11.
        body.push_str(&template_env::render(
            "jni_client_class_header.jinja",
            minijinja::context! {
                class_name => class_name,
            },
        ));

        // Emit MAPPER companion object for JSON serialisation/deserialisation.
        // Used by all method wrappers that marshal to/from the JNI String boundary.
        let has_json_methods = ty
            .methods
            .iter()
            .filter(|m| !m.sanitized && !m.is_static)
            .any(|m| !m.params.is_empty() || needs_json_deserialize(&m.return_type));
        let ctor_config = config.client_constructors.get(class_name.as_str());
        let needs_companion = has_json_methods || ctor_config.is_some();
        if needs_companion {
            body.push_str("    companion object {\n");
            if has_json_methods {
                body.push_str("        private val MAPPER = com.fasterxml.jackson.databind.ObjectMapper()\n");
                body.push_str("            .registerModule(com.fasterxml.jackson.datatype.jdk8.Jdk8Module())\n");
                body.push_str("            .findAndRegisterModules()\n");
                body.push_str(
                    "            .setPropertyNamingStrategy(com.fasterxml.jackson.databind.PropertyNamingStrategies.SNAKE_CASE)\n",
                );
            }
            if let Some(ctor) = ctor_config {
                emit_jni_client_factory(class_name, &bridge_name, ctor, api, &mut body);
            }
            body.push_str("    }\n\n");
        }

        for method in ty
            .methods
            .iter()
            .filter(|m| !m.sanitized && !m.is_static && !exclude_functions.contains(m.name.as_str()))
        {
            emit_jni_client_method(
                method,
                class_name,
                &bridge_name,
                &mut body,
                &mut imports,
                &opaque_type_names,
            );
        }

        // Streaming methods owned by this client type.
        for adapter in streaming_adapters
            .iter()
            .filter(|a| a.owner_type.as_deref() == Some(class_name.as_str()))
        {
            emit_jni_streaming_client_method(adapter, class_name, &bridge_name, &mut body);
        }

        let free_name = format!("nativeFree{class_name}");
        body.push_str(&template_env::render(
            "jni_client_close_method.jinja",
            minijinja::context! {
                bridge_name => bridge_name,
                free_name => free_name,
            },
        ));
        body.push_str("}\n");
    }

    // File-level @file:Suppress for the JNI client class silences ktlint/detekt
    // rules that the generated client wrapper naturally violates.
    let imports = imports.iter().cloned().collect::<Vec<_>>();
    let content = template_env::render(
        "jni_client_file.jinja",
        minijinja::context! {
            package => pkg,
            imports => imports,
            body => body,
        },
    );

    let path = jni_output_path(config, "DefaultClient.kt");
    Some(GeneratedFile {
        path,
        content,
        generated_header: false,
    })
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------
