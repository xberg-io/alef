//! Kotlin JVM binding generator — orchestration and `KotlinBackend` impl.
//!
//! The `KotlinBackend` struct implements [`Backend`] and dispatches to the
//! appropriate target-specific emitter based on the configured [`KotlinTarget`].

mod helpers;
pub mod jni_emitter;
mod object_wrapper;
mod shared;
pub mod trait_bridge;
mod traits;
mod typealiases;

use crate::naming::kotlin_target;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{AdapterPattern, KotlinFfiStyle, KotlinTarget, Language, ResolvedCrateConfig};
use alef_core::ir::{ApiSurface, EnumDef, ErrorDef, FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};
use std::collections::BTreeSet;
use std::path::PathBuf;

// Re-export shared utilities used by gen_native, gen_mpp, and the sibling
// alef-backend-kotlin-android crate.
pub use shared::{kotlin_field_name, to_lower_camel, to_pascal_case, to_screaming_snake};

// Re-export emitters used by gen_mpp and alef-backend-kotlin-android.
pub fn emit_type_pub(ty: &TypeDef, out: &mut String, imports: &mut BTreeSet<String>) {
    object_wrapper::emit_type_with_imports(ty, out, imports)
}

pub fn emit_enum_pub(en: &EnumDef, out: &mut String) {
    object_wrapper::emit_enum(en, out)
}

pub fn emit_error_type_pub(error: &ErrorDef, out: &mut String, imports: &mut BTreeSet<String>) {
    object_wrapper::emit_error_type_with_imports(error, out, imports)
}

fn effective_kotlin_exclude_types(config: &ResolvedCrateConfig) -> std::collections::HashSet<String> {
    let mut exclude_types: std::collections::HashSet<String> = config
        .ffi
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(kotlin) = &config.kotlin {
        exclude_types.extend(kotlin.exclude_types.iter().cloned());
    }
    exclude_types
}

fn effective_kotlin_exclude_functions(config: &ResolvedCrateConfig) -> std::collections::HashSet<String> {
    let mut exclude_functions: std::collections::HashSet<String> = config
        .ffi
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(kotlin) = &config.kotlin {
        exclude_functions.extend(kotlin.exclude_functions.iter().cloned());
    }
    exclude_functions
}

fn type_ref_references_excluded(ty: &TypeRef, exclude_types: &std::collections::HashSet<String>) -> bool {
    exclude_types.iter().any(|name| ty.references_named(name))
}

fn method_references_excluded(method: &MethodDef, exclude_types: &std::collections::HashSet<String>) -> bool {
    type_ref_references_excluded(&method.return_type, exclude_types)
        || method
            .params
            .iter()
            .any(|param| type_ref_references_excluded(&param.ty, exclude_types))
}

fn function_references_excluded(func: &FunctionDef, exclude_types: &std::collections::HashSet<String>) -> bool {
    type_ref_references_excluded(&func.return_type, exclude_types)
        || func
            .params
            .iter()
            .any(|param| type_ref_references_excluded(&param.ty, exclude_types))
}

/// Format a function parameter with its Kotlin type, collecting any needed imports.
pub fn format_param_pub(p: &ParamDef, imports: &mut BTreeSet<String>) -> String {
    object_wrapper::format_param_with_imports(p, imports)
}

/// Render a Kotlin type reference, collecting any needed imports.
pub fn kotlin_type_str_pub(ty: &TypeRef, optional: bool, imports: &mut BTreeSet<String>) -> String {
    object_wrapper::kotlin_type_with_string_imports(ty, optional, imports)
}

/// Emit a JVM function body (delegates to Bridge) inside an `object` block.
///
/// The empty `client_type_names` slice means callers from non-JVM emitters
/// (Android, MPP common-source) opt out of client-type wrapping. Returning a
/// client type from a flat function in those targets requires a backend-
/// specific surface that hasn't been wired up.
pub fn emit_function_jvm(f: &FunctionDef, out: &mut String, imports: &mut BTreeSet<String>, java_package: &str) {
    object_wrapper::emit_function(f, out, imports, java_package, &std::collections::HashSet::new())
}

/// Emit a `DefaultClient` Kotlin class for types that have non-empty `methods`.
///
/// Returns `None` when no type in the API surface has methods (flat-function APIs
/// like kreuzberg keep working unchanged).  When Some, the returned
/// [`GeneratedFile`] should be appended to the JVM output list.
///
/// The emitted Kotlin class wraps the sibling Java facade type (same simple
/// name) and re-exposes each instance method as a Kotlin `suspend` function
/// that hops onto `Dispatchers.IO` before the blocking JNI call. Streaming
/// adapters (pattern = `streaming`) whose `owner_type` matches a client type
/// are also emitted as plain (non-suspend) wrapper methods that return
/// `Iterator<ItemType>` — iteration is lazy and blocking, so the caller
/// controls the thread context.
pub fn emit_jvm_client_class(api: &ApiSurface, config: &ResolvedCrateConfig) -> Option<GeneratedFile> {
    emit_jvm_client_class_with_package(api, config, None)
}

/// Variant of [`emit_jvm_client_class`] that lets callers override the
/// emitted Kotlin package. The kotlin/android backend uses this to thread
/// `[crates.kotlin_android] package` through instead of falling back to the
/// generic `[crates.kotlin] package` accessor (which would derive a
/// `com.github.<org>` fallback from the GitHub URL when the JVM-only Kotlin
/// crate config is absent).
pub fn emit_jvm_client_class_with_package(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    kotlin_package_override: Option<&str>,
) -> Option<GeneratedFile> {
    // A type qualifies for a coroutine-friendly wrapper class only when:
    //   * it is opaque-handle (constructed via a factory and freed via close),
    //   * AND it is not a trait (trait types are not emitted as concrete
    //     Java classes — referencing them would dangle),
    //   * AND it has at least one non-sanitized, non-static instance method.
    // Non-opaque value types (e.g. kreuzberg `ExtractionConfig` with a
    // `default()` static) keep flowing through the Java typealias as before.
    let exclude_types = effective_kotlin_exclude_types(config);
    let is_client_type = |t: &&TypeDef| {
        t.is_opaque
            && !t.is_trait
            && !exclude_types.contains(&t.name)
            && t.methods
                .iter()
                .any(|m| !m.sanitized && !m.is_static && !method_references_excluded(m, &exclude_types))
    };
    let client_types: Vec<&TypeDef> = api.types.iter().filter(is_client_type).collect();
    if client_types.is_empty() {
        return None;
    }

    let java_package = config.java_package();
    let configured_kotlin_package = kotlin_package_override
        .map(str::to_string)
        .unwrap_or_else(|| config.kotlin_package());
    let package = if configured_kotlin_package == java_package {
        format!("{configured_kotlin_package}.kt")
    } else {
        configured_kotlin_package.clone()
    };

    let kotlin_root = config
        .output_for("kotlin")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "packages/kotlin".to_string());
    let kotlin_root_path = std::path::PathBuf::from(&kotlin_root);
    let package_path = package.replace('.', "/");

    let mut content = String::new();
    content.push_str("// Generated by alef. Do not edit by hand.\n\n");
    content.push_str(&format!("package {package}\n\n"));

    // Imports needed by method signatures. The Java facade types are aliased per
    // client so we can keep method bodies short without leaking FQNs.
    let mut imports: BTreeSet<String> = BTreeSet::new();

    // Method signatures and bodies reference DTO types (`ChatCompletionRequest`,
    // `CrawlConfig`, …) by their simple name. Those DTOs live in
    // `java_package` (emitted by the Java backend / bundled into the AAR by
    // kotlin-android). When the Kotlin file's package differs (either via
    // the `.kt` sub-package fallback when they were originally equal, or via
    // a kotlin-android override like `dev.kreuzberg.kreuzcrawl.android`),
    // Kotlin does NOT inherit symbols from the parent package — so without
    // explicit per-type imports every bare type reference is unresolved.
    // (ktlint forbids wildcard imports under `standard:no-wildcard-imports`.)
    // The actual imports are added below once `client_types` + streaming
    // adapter metadata are scanned for `Named` types.
    let needs_java_pkg_imports = package != java_package;

    let has_async = client_types
        .iter()
        .any(|t| t.methods.iter().any(|m| !m.sanitized && m.is_async));
    if has_async {
        imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
        imports.insert("import kotlinx.coroutines.withContext".to_string());
    }

    // Collect streaming adapters and add required Flow imports when any adapter
    // is owned by one of the client types.
    let streaming_adapters_for_clients: Vec<&alef_core::config::AdapterConfig> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter(|a| {
            a.owner_type
                .as_deref()
                .map(|owner| client_types.iter().any(|t| t.name == owner))
                .unwrap_or(false)
        })
        .collect();
    if !streaming_adapters_for_clients.is_empty() {
        imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
        imports.insert("import kotlinx.coroutines.withContext".to_string());
        imports.insert("import kotlinx.coroutines.flow.Flow".to_string());
        imports.insert("import kotlinx.coroutines.flow.callbackFlow".to_string());
        imports.insert("import kotlinx.coroutines.channels.awaitClose".to_string());
    }

    // Pre-scan return + param types so we collect every import the body needs.
    // We deliberately walk only non-sanitized methods — sanitized ones are
    // skipped during emission and any imports they would have required are
    // therefore unused.
    let mut scan_imports: BTreeSet<String> = BTreeSet::new();
    for ty in &client_types {
        for m in ty
            .methods
            .iter()
            .filter(|m| !m.sanitized && !m.is_static && !method_references_excluded(m, &exclude_types))
        {
            kotlin_type_str_pub(&m.return_type, false, &mut scan_imports);
            for p in &m.params {
                format_param_pub(p, &mut scan_imports);
            }
        }
    }
    imports.extend(scan_imports);

    // Emit per-user-type imports for the Java DTO package when the Kotlin
    // file lives in a different package. See the `needs_java_pkg_imports`
    // declaration above for rationale.
    if needs_java_pkg_imports {
        let mut user_types: BTreeSet<String> = BTreeSet::new();
        for ty in &client_types {
            for m in ty
                .methods
                .iter()
                .filter(|m| !m.sanitized && !m.is_static && !method_references_excluded(m, &exclude_types))
            {
                collect_user_types(&m.return_type, &mut user_types);
                for p in &m.params {
                    collect_user_types(&p.ty, &mut user_types);
                }
            }
        }
        for adapter in &streaming_adapters_for_clients {
            if let Some(item) = adapter.item_type.as_deref() {
                user_types.insert(item.to_string());
            }
            for p in &adapter.params {
                let simple = p.ty.rsplit("::").next().unwrap_or(&p.ty);
                if simple.chars().next().is_some_and(char::is_uppercase) {
                    user_types.insert(simple.to_string());
                }
            }
        }
        for ty in &user_types {
            imports.insert(format!("import {java_package}.{ty}"));
        }
    }

    for import in &imports {
        content.push_str(import);
        content.push('\n');
    }
    if !imports.is_empty() {
        content.push('\n');
    }

    // All streaming adapters (for any owner type).
    let streaming_adapters: Vec<&alef_core::config::AdapterConfig> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .collect();

    // Emit one class per client type. Each wraps a same-named Java instance so
    // construction is delegated to a Kotlin-side facade factory (see the
    // `LiterLlm` object emitted by the flat-function pass) that produces a
    // configured Java client to pass to `DefaultClient(javaInstance)`.
    for ty in &client_types {
        let class_name = &ty.name;
        let java_fqn = format!("{java_package}.{class_name}");
        content.push_str(&format!(
            "/** Coroutine-friendly wrapper around the Java `{java_fqn}` facade. */\n"
        ));
        content.push_str(&format!(
            "class {class_name} internal constructor(internal val inner: {java_fqn}) : AutoCloseable {{\n"
        ));

        let mut method_imports: BTreeSet<String> = BTreeSet::new();
        for method in ty
            .methods
            .iter()
            .filter(|m| !m.sanitized && !m.is_static && !method_references_excluded(m, &exclude_types))
        {
            emit_client_method(method, &mut content, &mut method_imports);
        }

        // Emit callbackFlow wrappers for streaming adapters owned by this client
        // type. These adapters are not in the IR `methods` list — they are
        // declared in `config.adapters` and bridge via JNI native methods on the
        // Java facade class. Each wrapper returns `Flow<ChunkType>` so Android
        // callers can use idiomatic coroutine collection.
        for adapter in streaming_adapters
            .iter()
            .filter(|a| a.owner_type.as_deref() == Some(class_name.as_str()))
        {
            emit_streaming_client_method(adapter, class_name, &java_package, &mut content);
        }

        content.push_str("    override fun close() { inner.close() }\n");
        content.push_str("}\n");
    }

    let client_file_name = if client_types.len() == 1 {
        format!("{}.kt", client_types[0].name)
    } else {
        "DefaultClient.kt".to_string()
    };

    let path = if config.explicit_output.kotlin.is_some() {
        kotlin_root_path.join(&client_file_name)
    } else {
        kotlin_root_path
            .join("src/main/kotlin")
            .join(&package_path)
            .join(&client_file_name)
    };

    Some(GeneratedFile {
        path,
        content,
        generated_header: false,
    })
}

/// Emit a single method on a client class, delegating to `inner.<method>(args)`.
///
/// Async methods hop onto `Dispatchers.IO` before invoking the blocking JNI
/// call so the suspend function yields its calling thread for the duration of
/// the request. Unit-returning sync methods drop the `return` keyword.
fn emit_client_method(m: &MethodDef, out: &mut String, imports: &mut BTreeSet<String>) {
    if !m.doc.is_empty() {
        for line in m.doc.lines() {
            out.push_str(&format!("    // {line}\n"));
        }
    }
    let method_name = to_lower_camel(&m.name);
    let return_ty = kotlin_type_str_pub(&m.return_type, false, imports);
    let async_kw = if m.is_async { "suspend " } else { "" };

    let params_with_types: Vec<String> = m.params.iter().map(|p| format_param_pub(p, imports)).collect();
    let call_args: String = m
        .params
        .iter()
        .map(|p| to_lower_camel(&p.name))
        .collect::<Vec<_>>()
        .join(", ");

    out.push_str(&format!(
        "    {async_kw}fun {method_name}({}): {return_ty} {{\n",
        params_with_types.join(", ")
    ));
    if m.is_async {
        out.push_str(&format!(
            "        return withContext(Dispatchers.IO) {{ inner.{method_name}({call_args}) }}\n"
        ));
    } else if matches!(m.return_type, TypeRef::Unit) {
        out.push_str(&format!("        inner.{method_name}({call_args})\n"));
    } else {
        out.push_str(&format!("        return inner.{method_name}({call_args})\n"));
    }
    out.push_str("    }\n\n");
}

/// Emit a `Flow<ChunkType>` wrapper for a streaming adapter method using
/// `callbackFlow`.
///
/// The generated method drives the three JNI native functions emitted on the
/// Java facade class (`native{Owner}{Adapter}Start`, `native{Owner}{Adapter}Next`,
/// `native{Owner}{Adapter}Free`) from within a `callbackFlow` block so Android
/// callers can use idiomatic `collect { chunk -> … }` coroutine patterns.
///
/// The owner type and adapter name determine the JNI method names:
/// - start: `native{PascalOwner}{PascalAdapter}Start(inner, requestJson)`
/// - next:  `native{PascalOwner}{PascalAdapter}Next(streamHandle)`
/// - free:  `native{PascalOwner}{PascalAdapter}Free(streamHandle)`
///
/// Generated form:
/// ```kotlin
///     fun chatStream(req: ChatCompletionRequest): kotlinx.coroutines.flow.Flow<ChatCompletionChunk> =
///         kotlinx.coroutines.flow.callbackFlow {
///             val handle: Long = withContext(Dispatchers.IO) {
///                 Bridge.nativeDefaultClientChatStreamStart(inner, MAPPER.writeValueAsString(req))
///             }
///             try {
///                 while (true) {
///                     val chunkJson: String? = withContext(Dispatchers.IO) {
///                         Bridge.nativeDefaultClientChatStreamNext(handle)
///                     }
///                     if (chunkJson == null) break
///                     val chunk = MAPPER.readValue(chunkJson, ChatCompletionChunk::class.java)
///                     send(chunk)
///                 }
///                 close()
///             } catch (e: Throwable) {
///                 close(e)
///             }
///             awaitClose {
///                 Bridge.nativeDefaultClientChatStreamFree(handle)
///             }
///         }
/// ```
/// Walk a `TypeRef` and collect every `Named` simple-name into `out`.
///
/// Used by `emit_jvm_client_class_with_package` to derive explicit per-type
/// Kotlin imports for the Java DTO package when the emitted Kotlin file
/// lives in a different (typically sub-) package (e.g. `dev.kreuzberg.kt`
/// or `dev.kreuzberg.kreuzcrawl.android`).
fn collect_user_types(ty: &TypeRef, out: &mut BTreeSet<String>) {
    match ty {
        TypeRef::Named(name) => {
            out.insert(name.clone());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_user_types(inner, out),
        TypeRef::Map(k, v) => {
            collect_user_types(k, out);
            collect_user_types(v, out);
        }
        _ => {}
    }
}

fn emit_streaming_client_method(
    adapter: &alef_core::config::AdapterConfig,
    class_name: &str,
    java_package: &str,
    out: &mut String,
) {
    let method_name = to_lower_camel(&adapter.name);
    let item_type = adapter.item_type.as_deref().unwrap_or("Any");
    // Derive the JNI method name prefix from owner + adapter names.
    // E.g. owner="DefaultClient", adapter="chat_stream" →
    //   nativeDefaultClientChatStreamStart
    let owner_pascal = to_pascal_case(class_name);
    let adapter_pascal = to_pascal_case(&adapter.name);
    let jni_start = format!("native{owner_pascal}{adapter_pascal}Start");
    let jni_next = format!("native{owner_pascal}{adapter_pascal}Next");
    let jni_free = format!("native{owner_pascal}{adapter_pascal}Free");

    // Build Kotlin parameter list — strip Rust module paths from type names.
    let params: Vec<String> = adapter
        .params
        .iter()
        .map(|p| {
            let simple_ty = p.ty.rsplit("::").next().unwrap_or(&p.ty);
            let param_name = to_lower_camel(&p.name);
            format!("{param_name}: {simple_ty}")
        })
        .collect();

    // Arguments to serialize as the request JSON (first param only for streaming).
    let first_param_name = adapter
        .params
        .first()
        .map(|p| to_lower_camel(&p.name))
        .unwrap_or_else(|| "request".to_string());

    let java_fqn_inner = format!("{java_package}.{class_name}");

    out.push_str(&format!(
        "    fun {method_name}({}): kotlinx.coroutines.flow.Flow<{item_type}> = kotlinx.coroutines.flow.callbackFlow {{\n",
        params.join(", ")
    ));
    // Capture inner locally so the lambda can reference it without capturing `this`.
    out.push_str(&format!(
        "        val inner: {java_fqn_inner} = this@{class_name}.inner\n"
    ));
    // Start the native stream on the IO dispatcher. The ObjectMapper is
    // allocated per-call here; a shared instance is not accessible from
    // the Java facade because the generated MAPPER field is private.
    out.push_str("        val mapper = com.fasterxml.jackson.databind.ObjectMapper()\n");
    out.push_str("            .registerModule(com.fasterxml.jackson.datatype.jdk8.Jdk8Module())\n");
    out.push_str("            .findAndRegisterModules()\n");
    out.push_str(
        "            .setPropertyNamingStrategy(com.fasterxml.jackson.databind.PropertyNamingStrategies.SNAKE_CASE)\n",
    );
    out.push_str("        val streamHandle: Long = withContext(Dispatchers.IO) {\n");
    out.push_str(&format!(
        "            Bridge.{jni_start}(inner, mapper.writeValueAsString({first_param_name}))\n"
    ));
    out.push_str("        }\n");
    out.push_str("        try {\n");
    out.push_str("            while (true) {\n");
    out.push_str("                val chunkJson: String? = withContext(Dispatchers.IO) {\n");
    out.push_str(&format!("                    Bridge.{jni_next}(streamHandle)\n"));
    out.push_str("                }\n");
    out.push_str("                if (chunkJson == null) break\n");
    out.push_str(&format!(
        "                val chunk = mapper.readValue(chunkJson, {item_type}::class.java)\n"
    ));
    out.push_str("                send(chunk)\n");
    out.push_str("            }\n");
    out.push_str("            close()\n");
    out.push_str("        } catch (e: Throwable) {\n");
    out.push_str("            close(e)\n");
    out.push_str("        }\n");
    out.push_str("        awaitClose {\n");
    out.push_str(&format!("            Bridge.{jni_free}(streamHandle)\n"));
    out.push_str("        }\n");
    out.push_str("    }\n\n");
}

// ---------------------------------------------------------------------------
// KotlinBackend
// ---------------------------------------------------------------------------

const BRIDGE_ALIAS: &str = "Bridge";

pub struct KotlinBackend;

impl Backend for KotlinBackend {
    fn name(&self) -> &str {
        "kotlin"
    }

    fn language(&self) -> Language {
        Language::Kotlin
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_callbacks: false,
            supports_streaming: true,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        // `mode = "android"` was the legacy in-band Android emission path. It has
        // been removed in alef 0.16 in favour of the dedicated
        // `alef-backend-kotlin-android` crate exposed as `Language::KotlinAndroid`.
        let mode = config.kotlin.as_ref().and_then(|k| k.mode.as_deref());
        if mode == Some("android") {
            anyhow::bail!(
                "`[crates.kotlin] mode = \"android\"` was removed in alef 0.16. \
                 Use `Language::KotlinAndroid` (slug `\"kotlin_android\"`) and the \
                 `alef-backend-kotlin-android` crate instead."
            );
        }
        // "kmp" mode forces Multiplatform emission.
        if mode == Some("kmp") {
            return crate::gen_mpp::emit(api, config);
        }
        // Dispatch by FFI style first; JNI mode is independent of target.
        if config.kotlin_ffi_style() == KotlinFfiStyle::Jni {
            return generate_jni(api, config);
        }
        // Default: dispatch by `target` (preserves existing behaviour).
        match kotlin_target(config) {
            KotlinTarget::Jvm => generate_jvm(api, config),
            KotlinTarget::Native => crate::gen_native::emit(api, config),
            KotlinTarget::Multiplatform => crate::gen_mpp::emit(api, config),
        }
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "gradle",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// JVM code generation
// ---------------------------------------------------------------------------

fn generate_jvm(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let java_package = config.java_package();
    let module_name = to_pascal_case(&config.name);
    // If the user's Kotlin and Java packages collide on the same FQN as the
    // generated module, the Kotlin object would shadow the Java facade class
    // (both compile to `<package>/<module>.class`). Push the Kotlin code into
    // a `.kt` sub-package in that case so the Java class remains importable.
    let configured_kotlin_package = config.kotlin_package();
    let package = if configured_kotlin_package == java_package {
        format!("{configured_kotlin_package}.kt")
    } else {
        configured_kotlin_package
    };

    let exclude_functions = effective_kotlin_exclude_functions(config);
    let mut exclude_types = effective_kotlin_exclude_types(config);

    // Types qualifying for a hand-written Kotlin wrapper class (see
    // `emit_jvm_client_class`) are opaque handles with at least one
    // non-sanitized, non-static instance method. Skip emitting a Java→Kotlin
    // typealias for them — the wrapper class would otherwise collide with the
    // alias in the same package and `compileKotlin` would fail with
    // `Redeclaration: DefaultClient`. Non-opaque value types (e.g. `Config`
    // structs with only a `default()` static) keep flowing through the alias.
    let client_type_names: std::collections::HashSet<String> = api
        .types
        .iter()
        .filter(|t| {
            t.is_opaque
                && !t.is_trait
                && !exclude_types.contains(&t.name)
                && t.methods
                    .iter()
                    .any(|m| !m.sanitized && !m.is_static && !method_references_excluded(m, &exclude_types))
        })
        .map(|t| t.name.clone())
        .collect();
    exclude_types.extend(client_type_names.iter().cloned());

    let configured_trait_bridges: std::collections::HashSet<&str> = config
        .trait_bridges
        .iter()
        .filter(|b| !b.exclude_languages.contains(&"kotlin".to_string()))
        .map(|b| b.trait_name.as_str())
        .collect();

    let mut imports: BTreeSet<String> = BTreeSet::new();
    let mut body = String::new();

    let exclude_type_names: std::collections::HashSet<&str> = exclude_types.iter().map(String::as_str).collect();
    typealiases::emit_typealiases(
        api,
        &java_package,
        &exclude_type_names,
        &configured_trait_bridges,
        &mut body,
    );

    // Functions whose signature involves a trait type are excluded — the Java
    // facade handles trait registration via a separate trait-bridge interface
    // that we don't expose through the Kotlin wrapper. Trait registry helpers
    // (register_*, unregister_*, list_*, clear_*) follow the same pattern.
    let trait_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_trait)
        .map(|t| t.name.as_str())
        .collect();
    let function_uses_trait = |f: &FunctionDef| -> bool {
        // Only skip functions that take a trait type as a parameter (those
        // need the Java facade's trait-bridge wrapper, which can't be reached
        // via a typealias-only Kotlin shim). Functions returning trait types
        // or trait registry helpers (register_*, list_*, clear_*) flow through
        // the Java facade unchanged.
        f.params
            .iter()
            .any(|p| traits::type_ref_uses_named(&p.ty, &trait_type_names))
    };

    let visible_functions: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| {
            !exclude_functions.contains(&f.name)
                && !function_uses_trait(f)
                && !function_references_excluded(f, &exclude_types)
        })
        .collect();

    if !visible_functions.is_empty() {
        // Import the Java facade class with an alias so it does not collide with the
        // Kotlin object that wraps it (both share the PascalCase crate name).
        imports.insert(format!("import {java_package}.{module_name} as {BRIDGE_ALIAS}"));
        if visible_functions.iter().any(|f| f.is_async) {
            imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
            imports.insert("import kotlinx.coroutines.withContext".to_string());
        }

        body.push_str(&crate::template_env::render(
            "object_declaration.jinja",
            minijinja::context! {
                name => module_name,
            },
        ));
        body.push('\n');
        for f in &visible_functions {
            object_wrapper::emit_function(f, &mut body, &mut imports, &java_package, &exclude_type_names);
            body.push('\n');
        }
        body.push_str("}\n");
    }

    let mut content = String::new();
    content.push_str("// Generated by alef. Do not edit by hand.\n\n");
    content.push_str(&crate::template_env::render(
        "package_declaration.jinja",
        minijinja::context! {
            package => package,
        },
    ));
    content.push_str("\n\n");
    for import in &imports {
        content.push_str(import);
        content.push('\n');
    }
    if !imports.is_empty() {
        content.push('\n');
    }
    content.push_str(&body);

    let package_path = package.replace('.', "/");
    let kotlin_root = config
        .output_for("kotlin")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "packages/kotlin".to_string());
    let kotlin_root_path = PathBuf::from(&kotlin_root);
    // Explicit `[crates.output] kotlin = "..."` is treated as the final
    // package directory. Without an override the backend constructs the
    // canonical Maven-style `src/main/kotlin/<package>/` layout under the
    // template-derived base.
    let path = if config.explicit_output.kotlin.is_some() {
        kotlin_root_path.join(format!("{module_name}.kt"))
    } else {
        kotlin_root_path
            .join("src/main/kotlin")
            .join(&package_path)
            .join(format!("{module_name}.kt"))
    };

    let mut files = vec![GeneratedFile {
        path,
        content,
        generated_header: false,
    }];

    // Emit DefaultClient.kt when the API surface contains types with methods.
    if let Some(client_file) = emit_jvm_client_class(api, config) {
        files.push(client_file);
    }

    Ok(files)
}

// ---------------------------------------------------------------------------
// JNI code generation
// ---------------------------------------------------------------------------

fn generate_jni(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = vec![jni_emitter::emit_jni_bridge_object(api, config)];
    if let Some(client_file) = jni_emitter::emit_jni_client_class(api, config, None) {
        files.push(client_file);
    }
    Ok(files)
}
