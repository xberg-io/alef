//! Kotlin JVM binding generator — orchestration and `KotlinBackend` impl.
//!
//! The `KotlinBackend` struct implements [`Backend`] and dispatches to the
//! appropriate target-specific emitter based on the configured [`KotlinTarget`].

mod helpers;
pub mod jni_emitter;
pub mod literal_normalizer;
mod object_wrapper;
pub mod service_api;
mod shared;
pub mod trait_bridge;
mod traits;
mod typealiases;

use crate::backends::kotlin::naming::kotlin_target;
use crate::backends::kotlin::template_env;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{AdapterPattern, KotlinFfiStyle, KotlinTarget, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, ErrorDef, FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};
use std::collections::BTreeSet;
use std::path::PathBuf;

pub use shared::{kotlin_field_name, to_lower_camel, to_pascal_case, to_screaming_snake};

pub fn emit_type_pub(ty: &TypeDef, out: &mut String, imports: &mut BTreeSet<String>) {
    object_wrapper::emit_type_with_imports(
        ty,
        out,
        imports,
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    )
}

/// Like [`emit_type_pub`] but also threads an enum-name → default-variant map
/// so that fields whose declared type is a Named enum (e.g. `HeadingStyle`)
/// receive a constructor default like `= HeadingStyle.ATX`. The Jackson
/// Kotlin module otherwise raises `MissingKotlinParameterException` when the
/// wire JSON omits the key, which is the common case for partial-update
/// payloads sent from test fixtures (`mapper.readValue("{\"x\":true}",
/// ParseOptions::class.java)`).
pub fn emit_type_pub_with_enum_defaults(
    ty: &TypeDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    enum_defaults: &std::collections::HashMap<String, String>,
) {
    object_wrapper::emit_type_with_imports(
        ty,
        out,
        imports,
        enum_defaults,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    )
}

/// Like [`emit_type_pub_with_enum_defaults`] but also threads the set of
/// sealed-class names (Rust enums with `serde_tag` or `serde_untagged`).
/// Fields whose declared type references one of these names receive a
/// `@field:JsonSerialize(\`as\` = …)` (or `contentAs` for collections)
/// annotation so Jackson dispatches the parent's custom serializer instead
/// of the variant's default POJO serializer.  See `emit_type_with_imports`
/// for the full rationale.
pub fn emit_type_pub_with_enum_defaults_and_sealed_classes(
    ty: &TypeDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    enum_defaults: &std::collections::HashMap<String, String>,
    sealed_class_names: &std::collections::HashSet<String>,
) {
    object_wrapper::emit_type_with_imports(
        ty,
        out,
        imports,
        enum_defaults,
        sealed_class_names,
        &std::collections::HashSet::new(),
    )
}

/// Like [`emit_type_pub_with_enum_defaults_and_sealed_classes`] but also threads
/// the set of non-enum data class type names that have a Rust `Default` impl AND
/// whose Kotlin emission gives every constructor parameter a default value.
/// Fields whose declared type references such a name receive a constructor
/// default like `= PreprocessingOptions()` — preventing Jackson's Kotlin module
/// from raising `MissingKotlinParameterException` when the wire JSON omits the
/// nested struct.
pub fn emit_type_pub_with_defaults_sealed_and_constructible(
    ty: &TypeDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    enum_defaults: &std::collections::HashMap<String, String>,
    sealed_class_names: &std::collections::HashSet<String>,
    default_constructible_types: &std::collections::HashSet<String>,
) {
    object_wrapper::emit_type_with_imports(
        ty,
        out,
        imports,
        enum_defaults,
        sealed_class_names,
        default_constructible_types,
    )
}

pub fn emit_enum_pub(en: &EnumDef, out: &mut String, package: &str, text_types: &[String]) {
    object_wrapper::emit_enum(en, out, package, text_types)
}

pub fn emit_error_type_pub(error: &ErrorDef, out: &mut String, imports: &mut BTreeSet<String>) {
    object_wrapper::emit_error_type_with_imports(error, out, imports)
}

/// Emit cleaned KDoc for a documentation string. Re-exported for sibling
/// crates (alef-backend-kotlin-android) so they can attach KDoc to their own
/// emitted declarations without depending on `alef-codegen` directly.
pub fn emit_kdoc_pub(out: &mut String, doc: &str, indent: &str) {
    helpers::emit_cleaned_kdoc(out, doc, indent);
}

fn effective_kotlin_exclude_types(config: &ResolvedCrateConfig, api: &ApiSurface) -> std::collections::HashSet<String> {
    let mut exclude_types: std::collections::HashSet<String> = config
        .ffi
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(kotlin) = &config.kotlin {
        exclude_types.extend(kotlin.exclude_types.iter().cloned());
    }
    if let Some(java) = &config.java {
        exclude_types.extend(java.exclude_types.iter().cloned());
    }
    exclude_types.extend(config.exclude.types.iter().cloned());
    exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    exclude_types.extend(
        config
            .opaque_types
            .iter()
            .filter(|(_, path)| path.contains('<'))
            .map(|(name, _)| name.clone()),
    );
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

/// Emit one Kotlin coroutine-wrapper file per opaque client type.
///
/// Returns an empty `Vec` when no type in the API surface has methods (flat-function
/// APIs like sample_core keep working unchanged). Otherwise, returns one
/// [`GeneratedFile`] per client type, each named after the wrapped type
/// (e.g. `Router.kt`, `GraphQLRouteConfig.kt`, `DefaultClient.kt`).
///
/// Splitting per type — instead of bundling unrelated wrappers into a single
/// `DefaultClient.kt` — prevents two failure modes that the bundled form invited:
///   1. Stale-orphan duplication: when the set of qualifying client types shrank
///      between alef versions (e.g. a type moved into `[crates.exclude]`), the
///      bundled `DefaultClient.kt` from the old run was overwritten in place but
///      any per-type files from older alef versions lingered with duplicate
///      `class Foo : AutoCloseable` declarations (compile error: "Redeclaration").
///   2. Misleading file naming: `DefaultClient.kt` containing `class Router` /
///      `class GraphQLRouteConfig` is unintuitive for IDE navigation and
///      file-grep workflows.
///
/// Each emitted Kotlin class wraps the sibling Java facade type (same simple
/// name) and re-exposes each instance method as a Kotlin `suspend` function
/// that hops onto `Dispatchers.IO` before the blocking JNI call. Streaming
/// adapters (pattern = `streaming`) whose `owner_type` matches a client type
/// are also emitted as plain (non-suspend) wrapper methods that return
/// `Flow<ChunkType>` — iteration uses `callbackFlow`, so the caller controls
/// the thread context.
pub fn emit_jvm_client_class(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<GeneratedFile> {
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
) -> Vec<GeneratedFile> {
    let exclude_types = effective_kotlin_exclude_types(config, api);
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
        return Vec::new();
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

    let streaming_adapters: Vec<&crate::core::config::AdapterConfig> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter(|a| !a.skip_languages.iter().any(|l| l == "kotlin"))
        .collect();

    let needs_java_pkg_imports = package != java_package;

    client_types
        .iter()
        .map(|ty| {
            emit_client_type_file(
                ty,
                &package,
                &java_package,
                &package_path,
                &kotlin_root_path,
                config,
                &exclude_types,
                &streaming_adapters,
                needs_java_pkg_imports,
            )
        })
        .collect()
}

/// Emit a single `<ClassName>.kt` file wrapping one opaque Java facade type.
#[allow(clippy::too_many_arguments)]
fn emit_client_type_file(
    ty: &TypeDef,
    package: &str,
    java_package: &str,
    package_path: &str,
    kotlin_root_path: &std::path::Path,
    config: &ResolvedCrateConfig,
    exclude_types: &std::collections::HashSet<String>,
    streaming_adapters: &[&crate::core::config::AdapterConfig],
    needs_java_pkg_imports: bool,
) -> GeneratedFile {
    let class_name = &ty.name;

    let mut imports: BTreeSet<String> = BTreeSet::new();

    let has_async = ty.methods.iter().any(|m| !m.sanitized && m.is_async);
    if has_async {
        imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
        imports.insert("import kotlinx.coroutines.withContext".to_string());
    }

    let owned_streaming_adapters: Vec<&crate::core::config::AdapterConfig> = streaming_adapters
        .iter()
        .copied()
        .filter(|a| a.owner_type.as_deref() == Some(class_name.as_str()))
        .collect();
    if !owned_streaming_adapters.is_empty() {
        imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
        imports.insert("import kotlinx.coroutines.withContext".to_string());
        imports.insert("import kotlinx.coroutines.flow.Flow".to_string());
        imports.insert("import kotlinx.coroutines.flow.callbackFlow".to_string());
        imports.insert("import kotlinx.coroutines.channels.awaitClose".to_string());
    }

    let mut scan_imports: BTreeSet<String> = BTreeSet::new();
    for m in ty
        .methods
        .iter()
        .filter(|m| !m.sanitized && !m.is_static && !method_references_excluded(m, exclude_types))
    {
        kotlin_type_str_pub(&m.return_type, false, &mut scan_imports);
        for p in &m.params {
            format_param_pub(p, &mut scan_imports);
        }
    }
    imports.extend(scan_imports);

    if needs_java_pkg_imports {
        let mut user_types: BTreeSet<String> = BTreeSet::new();
        for m in ty
            .methods
            .iter()
            .filter(|m| !m.sanitized && !m.is_static && !method_references_excluded(m, exclude_types))
        {
            collect_user_types(&m.return_type, &mut user_types);
            for p in &m.params {
                collect_user_types(&p.ty, &mut user_types);
            }
        }
        for adapter in &owned_streaming_adapters {
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
        for ty_name in &user_types {
            imports.insert(format!("import {java_package}.{ty_name}"));
        }
    }

    let mut body = String::new();

    let java_fqn = format!("{java_package}.{class_name}");
    body.push_str(&template_env::render(
        "client_class_header.jinja",
        minijinja::context! {
            java_fqn => java_fqn,
            class_name => class_name,
        },
    ));

    let needs_mapper = ty
        .methods
        .iter()
        .filter(|m| !m.sanitized && !m.is_static && !method_references_excluded(m, exclude_types))
        .any(|m| m.params.iter().any(|p| matches!(p.ty, TypeRef::Json)));
    if needs_mapper {
        body.push_str("    private companion object {\n");
        body.push_str("        private val MAPPER = com.fasterxml.jackson.databind.ObjectMapper()\n");
        body.push_str("    }\n\n");
    }

    let mut method_imports: BTreeSet<String> = BTreeSet::new();
    for method in ty
        .methods
        .iter()
        .filter(|m| !m.sanitized && !m.is_static && !method_references_excluded(m, exclude_types))
    {
        emit_client_method(method, &mut body, &mut method_imports);
    }

    for adapter in &owned_streaming_adapters {
        emit_streaming_client_method(adapter, class_name, java_package, &mut body);
    }

    body.push_str(&template_env::render(
        "client_close_method.jinja",
        minijinja::context! {},
    ));
    body.push_str("}\n");

    let content = shared::assemble_kt_file(package, &imports, &body);

    let client_file_name = format!("{class_name}.kt");
    let path = if config.explicit_output.kotlin.is_some() {
        kotlin_root_path.join(&client_file_name)
    } else {
        kotlin_root_path
            .join("src/main/kotlin")
            .join(package_path)
            .join(&client_file_name)
    };

    GeneratedFile {
        path,
        content,
        generated_header: false,
    }
}

/// Emit a single method on a client class, delegating to `inner.<method>(args)`.
///
/// Async methods hop onto `Dispatchers.IO` before invoking the blocking JNI
/// call so the suspend function yields its calling thread for the duration of
/// the request. Unit-returning sync methods drop the `return` keyword.
fn emit_client_method(m: &MethodDef, out: &mut String, imports: &mut BTreeSet<String>) {
    if !m.doc.is_empty() {
        for line in m.doc.lines() {
            out.push_str(&template_env::render(
                "line_comment.jinja",
                minijinja::context! {
                    indent => "    ",
                    line => line,
                },
            ));
        }
    }
    let method_name = to_lower_camel(&m.name);
    let return_ty = kotlin_type_str_pub(&m.return_type, false, imports);
    let async_kw = if m.is_async { "suspend " } else { "" };

    let params_with_types: Vec<String> = m.params.iter().map(|p| format_param_pub(p, imports)).collect();
    let call_args: String = m
        .params
        .iter()
        .map(|p| {
            let name = to_lower_camel(&p.name);
            if matches!(p.ty, TypeRef::Json) {
                format!("MAPPER.writeValueAsString({name})")
            } else {
                name
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let optional_suffix = if matches!(m.return_type, TypeRef::Optional(_)) {
        ".orElse(null)"
    } else {
        ""
    };
    out.push_str(&template_env::render(
        "kotlin_client_method.jinja",
        minijinja::context! {
            async_kw => async_kw,
            method_name => method_name,
            params => params_with_types.join(", "),
            return_type => return_ty,
            call_args => call_args,
            optional_suffix => optional_suffix,
            async => m.is_async,
            unit_return => matches!(m.return_type, TypeRef::Unit),
        },
    ));
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
/// lives in a different (typically sub-) package (e.g. `dev.sample_core.kt`
/// or `dev.sample_core.sample_worker.android`).
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
    adapter: &crate::core::config::AdapterConfig,
    class_name: &str,
    java_package: &str,
    out: &mut String,
) {
    let method_name = to_lower_camel(&adapter.name);
    let item_type = adapter.item_type.as_deref().unwrap_or("Any");
    let owner_pascal = to_pascal_case(class_name);
    let adapter_pascal = to_pascal_case(&adapter.name);
    let jni_start = format!("native{owner_pascal}{adapter_pascal}Start");
    let jni_next = format!("native{owner_pascal}{adapter_pascal}Next");
    let jni_free = format!("native{owner_pascal}{adapter_pascal}Free");

    let params: Vec<String> = adapter
        .params
        .iter()
        .map(|p| {
            let simple_ty = p.ty.rsplit("::").next().unwrap_or(&p.ty);
            let param_name = to_lower_camel(&p.name);
            format!("{param_name}: {simple_ty}")
        })
        .collect();

    let first_param_name = adapter
        .params
        .first()
        .map(|p| to_lower_camel(&p.name))
        .unwrap_or_else(|| "request".to_string());

    let java_fqn_inner = format!("{java_package}.{class_name}");

    out.push_str(&template_env::render(
        "kotlin_streaming_client_method.jinja",
        minijinja::context! {
            method_name => method_name,
            params => params.join(", "),
            item_type => item_type,
            java_fqn_inner => java_fqn_inner,
            class_name => class_name,
            jni_start => jni_start,
            jni_next => jni_next,
            jni_free => jni_free,
            first_param_name => first_param_name,
        },
    ));
}

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
            supports_service_api: true,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let mode = config.kotlin.as_ref().and_then(|k| k.mode.as_deref());
        if mode == Some("android") {
            anyhow::bail!(
                "`[crates.kotlin] mode = \"android\"` was removed in alef 0.16. \
                 Use `Language::KotlinAndroid` (slug `\"kotlin_android\"`) and the \
                 `alef-backend-kotlin-android` crate instead."
            );
        }
        if mode == Some("kmp") {
            let mut files = crate::backends::kotlin::gen_mpp::emit(api, config)?;
            post_process_kotlin_files(&mut files);
            return Ok(files);
        }
        if config.kotlin_ffi_style() == KotlinFfiStyle::Jni {
            let mut files = generate_jni(api, config)?;
            post_process_kotlin_files(&mut files);
            return Ok(files);
        }
        let mut files = match kotlin_target(config) {
            KotlinTarget::Jvm => generate_jvm(api, config)?,
            KotlinTarget::Native => crate::backends::kotlin::gen_native::emit(api, config)?,
            KotlinTarget::Multiplatform => crate::backends::kotlin::gen_mpp::emit(api, config)?,
        };
        post_process_kotlin_files(&mut files);
        Ok(files)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "gradle",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        service_api::generate(api, config)
    }
}

fn generate_jvm(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let java_package = config.java_package();
    let module_name = to_pascal_case(&config.name);
    let configured_kotlin_package = config.kotlin_package();
    let package = if configured_kotlin_package == java_package {
        format!("{configured_kotlin_package}.kt")
    } else {
        configured_kotlin_package
    };

    let exclude_functions = effective_kotlin_exclude_functions(config);
    let mut exclude_types = effective_kotlin_exclude_types(config, api);

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

    let trait_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_trait)
        .map(|t| t.name.as_str())
        .collect();
    let function_uses_trait = |f: &FunctionDef| -> bool {
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
        imports.insert(format!("import {java_package}.{module_name} as {BRIDGE_ALIAS}"));
        if visible_functions.iter().any(|f| f.is_async) {
            imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
            imports.insert("import kotlinx.coroutines.withContext".to_string());
        }

        body.push_str(&crate::backends::kotlin::template_env::render(
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
    content.push_str("// Generated by alef. Do not edit by hand.\n");
    content.push_str(
        "@file:Suppress(\n    \
         \"ktlint:standard:trailing-comma-on-call-site\",\n    \
         \"ktlint:standard:trailing-comma-on-declaration-site\",\n    \
         \"ktlint:standard:spacing-between-declarations-with-comments\",\n    \
         \"ktlint:standard:spacing-between-declarations-with-annotations\",\n    \
         \"ktlint:standard:when-entry-bracing\",\n    \
         \"ktlint:standard:blank-line-between-when-conditions\",\n    \
         \"ktlint:standard:blank-line-before-declaration\",\n    \
         \"ktlint:standard:chain-method-continuation\",\n    \
         \"ktlint:standard:annotation\",\n    \
         \"ktlint:standard:max-line-length\",\n    \
         \"ktlint:standard:no-semi\",\n    \
         \"ktlint:standard:statement-wrapping\",\n    \
         \"MaxLineLength\",\n    \
         \"TooManyFunctions\",\n    \
         \"FunctionParameterNaming\",\n    \
         \"LongParameterList\",\n    \
         \"CyclomaticComplexMethod\",\n    \
         \"LongMethod\",\n    \
         \"MagicNumber\",\n    \
         \"ReturnCount\",\n    \
         \"NestedBlockDepth\",\n\
         )\n\n",
    );
    content.push_str(&crate::backends::kotlin::template_env::render(
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

    files.extend(emit_jvm_client_class(api, config));

    Ok(files)
}

fn generate_jni(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = vec![jni_emitter::emit_jni_bridge_object(api, config)];
    if let Some(client_file) = jni_emitter::emit_jni_client_class(api, config, None) {
        files.push(client_file);
    }
    let package = jni_emitter::jni_kotlin_package(config);
    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg.exclude_languages.iter().any(|l| l == "kotlin") || bridge_cfg.register_fn.is_none() {
            continue;
        }
        if let Some(trait_def) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
            let (filename, content) = crate::backends::kotlin_android::trait_bridge::gen_jni_dispatcher_file(
                &package,
                &bridge_cfg.trait_name,
                bridge_cfg,
                trait_def,
                api,
                &std::collections::HashSet::new(),
            );
            files.push(GeneratedFile {
                path: jni_emitter::jni_output_path(config, &filename),
                content,
                generated_header: true,
            });
        }
    }
    Ok(files)
}

/// Apply post-processing fixes to generated Kotlin files.
fn post_process_kotlin_files(files: &mut [GeneratedFile]) {
    for file in files {
        if file.path.ends_with(".kt") {
            file.content = literal_normalizer::fix_float_literals(&file.content);
        }
    }
}
