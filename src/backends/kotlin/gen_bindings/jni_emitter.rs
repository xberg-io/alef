//! JNI emission mode for the Kotlin backend.
//!
//! When `[crates.kotlin] ffi_style = "jni"` (or when forced by the Android
//! backend), this module emits:
//!
//! - `<Module>Bridge.kt` — a Kotlin `object` with `external fun` declarations
//!   and an `init { System.loadLibrary("<crate>_jni") }` block.
//! - `DefaultClient.kt` — a Kotlin class holding a `Long` handle that delegates
//!   every method to the Bridge object via JNI. Streaming methods use the same
//!   `callbackFlow` pattern as the Panama path but reference `handle: Long`
//!   instead of `inner: <JavaFacadeType>`.
//!
//! No `java.lang.foreign.*` imports are emitted anywhere in this module.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::core::backend::GeneratedFile;
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::config::{AdapterPattern, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};

use super::object_wrapper::{format_param_with_imports, kotlin_type_with_string_imports};
use super::shared::{to_lower_camel, to_pascal_case};

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
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
        .collect();

    // Opaque type names: Named params of this shape are handles (Long), not JSON (String).
    let opaque_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

    let mut body = String::new();
    // Suppress detekt TooManyFunctions: the bridge object has one external fun
    // per API function; large APIs naturally exceed the default threshold of 11.
    body.push_str("@Suppress(\"TooManyFunctions\")\n");
    body.push_str(&format!("object {bridge_name} {{\n"));
    body.push_str(&format!("    init {{ System.loadLibrary(\"{lib_name}\") }}\n"));

    // Collect native function names from the API to detect duplicates later.
    let mut emitted_native_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Emit one `external fun` per visible API function.
    // Every native method is annotated @Throws so typed catch blocks work in
    // Kotlin/Java callers — without this the JNI RuntimeException is wrapped in
    // UndeclaredThrowableException and silently bypasses catch(BridgeException).
    for f in &visible_functions {
        let native_name = format!("native{}", to_pascal_case(&f.name));
        emitted_native_names.insert(native_name.clone());
        let return_ty = jni_return_type_for_function(&f.return_type, &opaque_type_names);
        let jni_params = jni_params_for_function(f, &opaque_type_names);
        if matches!(f.return_type, TypeRef::Unit) {
            body.push_str(&format!(
                "\n    @Throws({exception_class}::class)\n    external fun {native_name}({jni_params})\n"
            ));
        } else {
            body.push_str(&format!(
                "\n    @Throws({exception_class}::class)\n    external fun {native_name}({jni_params}): {return_ty}\n"
            ));
        }
    }

    // Emit external funs for instance methods on opaque client types.
    let methods_emitted_before = body.matches("// JNI external funs for client instance methods").count();
    emit_method_jni_external_funs(&mut body, api, &exclude_functions, &exception_class);
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
                    let return_ty = jni_return_type(&method.return_type);
                    let params = if method.params.is_empty() {
                        "handle: Long".to_string()
                    } else if method.params.len() == 1 && is_binary_param_type(&method.params[0].ty) {
                        format!("handle: Long, {}: ByteArray", to_lower_camel(&method.params[0].name))
                    } else {
                        "handle: Long, requestJson: String".to_string()
                    };
                    if matches!(method.return_type, TypeRef::Unit) {
                        body.push_str(&format!(
                            "    @Throws({exception_class}::class)\n    external fun {native_name}({params})\n"
                        ));
                    } else {
                        body.push_str(&format!(
                            "    @Throws({exception_class}::class)\n    external fun {native_name}({params}): {return_ty}\n"
                        ));
                    }
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

    // Emit nativeFreeXxx destructors for opaque types.
    // - For types with instance methods (client types): destructor is called from close()
    // - For types returned by top-level functions: destructor is called after extraction
    let client_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && t.methods.iter().any(|m| !m.sanitized && !m.is_static))
        .map(|t| t.name.as_str())
        .collect();

    let top_level_opaque_returns: std::collections::BTreeSet<&str> = visible_functions
        .iter()
        .filter_map(|f| {
            if let TypeRef::Named(n) = &f.return_type {
                if opaque_type_names.contains(n.as_str()) && !client_type_names.contains(n.as_str()) {
                    return Some(n.as_str());
                }
            }
            None
        })
        .collect();

    // Emit destructors for all opaque types: client types (emitted by emit_method_jni_external_funs
    // and called from close()) and handle-only types (emitted here for top-level returns).
    let all_opaque_with_destructors: std::collections::BTreeSet<&str> = client_type_names
        .iter()
        .copied()
        .chain(top_level_opaque_returns.iter().copied())
        .collect();

    if !all_opaque_with_destructors.is_empty() {
        body.push_str("\n    // Destructor external funs for opaque handle types.\n");
        for type_name in &all_opaque_with_destructors {
            let free_name = format!("nativeFree{}", to_pascal_case(type_name));
            body.push_str(&format!("    external fun {free_name}(handle: Long)\n"));
        }
    }

    body.push_str("}\n");

    let mut content = String::new();
    content.push_str("// Generated by alef. Do not edit by hand.\n");
    // File-level @file:Suppress: silence ktlint/detekt rules that generated
    // JNI bridge code naturally violates (long external-fun signatures,
    // dense annotations, no trailing commas).
    content.push_str(
        "@file:Suppress(\n    \
         \"ktlint:standard:max-line-length\",\n    \
         \"ktlint:standard:trailing-comma-on-declaration-site\",\n    \
         \"ktlint:standard:trailing-comma-on-call-site\",\n    \
         \"ktlint:standard:annotation\",\n    \
         \"MaxLineLength\",\n    \
         \"TooManyFunctions\",\n    \
         \"LongParameterList\",\n\
         )\n\n",
    );
    content.push_str(&format!("package {package}\n\n"));
    content.push_str(&body);

    let path = jni_output_path(config, &format!("{bridge_name}.kt"));
    GeneratedFile {
        path,
        content,
        generated_header: false,
    }
}

/// Emit `external fun native{Owner}{Adapter}{Start,Next,Free}` declarations
/// for every streaming adapter with an owner type. Called from both
/// `emit_jni_bridge_object` (for the Bridge object body) and from tests.
/// `exception_class` is the simple name of the exception class emitted alongside
/// the Bridge object (e.g. `"DemoBridgeException"`).  Start and Next are annotated
/// with `@Throws` because they can propagate Rust errors; Free is infallible.
pub fn emit_streaming_jni_external_funs(out: &mut String, config: &ResolvedCrateConfig, exception_class: &str) {
    let streaming: Vec<_> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming) && a.owner_type.is_some())
        .collect();
    if streaming.is_empty() {
        return;
    }
    out.push_str("\n    // JNI streaming external funs — implementations are Rust JNI shims.\n");
    for adapter in &streaming {
        let Some(owner) = adapter.owner_type.as_deref() else {
            continue;
        };
        let owner_pascal = to_pascal_case(owner);
        let adapter_pascal = to_pascal_case(&adapter.name);
        let jni_start = format!("native{owner_pascal}{adapter_pascal}Start");
        let jni_next = format!("native{owner_pascal}{adapter_pascal}Next");
        let jni_free = format!("native{owner_pascal}{adapter_pascal}Free");
        out.push('\n');
        out.push_str(&format!(
            "    // Streaming JNI externs for {owner}.{}\n",
            to_lower_camel(&adapter.name)
        ));
        out.push_str(&format!(
            "    @Throws({exception_class}::class)\n    external fun {jni_start}(clientHandle: Long, requestJson: String): Long\n"
        ));
        out.push_str(&format!(
            "    @Throws({exception_class}::class)\n    external fun {jni_next}(streamHandle: Long): String?\n"
        ));
        // Free is infallible: it only drops the Rust Box, never throws.
        out.push_str(&format!("    external fun {jni_free}(streamHandle: Long)\n"));
    }
}

/// Emit `external fun native{Owner}{Method}(handle: Long, requestJson: String): <ReturnType>`
/// declarations for every visible, non-sanitized, non-static instance method on every
/// opaque client type in the API surface, plus a `external fun nativeFree{Owner}(handle: Long)`
/// destructor declaration for each client type.
///
/// Methods with no params beyond `&self` produce `(handle: Long)` with no `requestJson`.
/// `Vec<u8>` return types produce `ByteArray`; `Unit` stays `Unit`; everything else
/// serialises via JSON → `String` (or `String?` for optionals).
/// `exception_class` is the simple name of the exception class so every method gets
/// an `@Throws` annotation that allows typed catch blocks to reach the error.
fn emit_method_jni_external_funs(
    out: &mut String,
    api: &ApiSurface,
    exclude_functions: &std::collections::HashSet<&str>,
    exception_class: &str,
) {
    let client_types: Vec<_> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && t.methods.iter().any(|m| !m.is_static))
        .collect();
    if client_types.is_empty() {
        return;
    }
    out.push_str("\n    // JNI external funs for client instance methods.\n");
    for ty in &client_types {
        let owner_pascal = to_pascal_case(&ty.name);
        for method in ty.methods.iter().filter(|m| !m.is_static) {
            if exclude_functions.contains(method.name.as_str()) {
                continue;
            }
            let native_name = format!("native{owner_pascal}{}", to_pascal_case(&method.name));
            let return_ty = jni_return_type(&method.return_type);
            // Methods with at least one param pass them all as a single JSON string.
            // Methods with no params are called with only the handle.
            let params = if method.params.is_empty() {
                "handle: Long".to_string()
            } else if method.params.len() == 1 && is_binary_param_type(&method.params[0].ty) {
                format!("handle: Long, {}: ByteArray", to_lower_camel(&method.params[0].name))
            } else {
                "handle: Long, requestJson: String".to_string()
            };
            if matches!(method.return_type, TypeRef::Unit) {
                out.push_str(&format!(
                    "    @Throws({exception_class}::class)\n    external fun {native_name}({params})\n"
                ));
            } else {
                out.push_str(&format!(
                    "    @Throws({exception_class}::class)\n    external fun {native_name}({params}): {return_ty}\n"
                ));
            }
        }
        // Emit destructor external fun so Bridge.kt declares the symbol that
        // DefaultClient.close() delegates to.  Destructors are infallible — no @Throws.
        let free_name = format!("nativeFree{owner_pascal}");
        out.push_str(&format!("    external fun {free_name}(handle: Long)\n"));
    }
}

// ---------------------------------------------------------------------------
// JNI DefaultClient emitter
// ---------------------------------------------------------------------------

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
        body.push_str("@Suppress(\"TooManyFunctions\")\n");
        body.push_str(&format!(
            "class {class_name} internal constructor(internal val handle: Long) : AutoCloseable {{\n"
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
            emit_jni_client_method(method, class_name, &bridge_name, &mut body, &mut imports);
        }

        // Streaming methods owned by this client type.
        for adapter in streaming_adapters
            .iter()
            .filter(|a| a.owner_type.as_deref() == Some(class_name.as_str()))
        {
            emit_jni_streaming_client_method(adapter, class_name, &bridge_name, &mut body);
        }

        let free_name = format!("nativeFree{class_name}");
        body.push_str(&format!(
            "    override fun close() {{ {bridge_name}.{free_name}(handle) }}\n"
        ));
        body.push_str("}\n");
    }

    let mut content = String::new();
    content.push_str("// Generated by alef. Do not edit by hand.\n");
    // File-level @file:Suppress for the JNI client class: silence ktlint/detekt
    // rules that the generated client wrapper naturally violates (long
    // coroutine method signatures, parameter counts, line-length).
    content.push_str(
        "@file:Suppress(\n    \
         \"ktlint:standard:max-line-length\",\n    \
         \"ktlint:standard:trailing-comma-on-declaration-site\",\n    \
         \"ktlint:standard:trailing-comma-on-call-site\",\n    \
         \"ktlint:standard:annotation\",\n    \
         \"MaxLineLength\",\n    \
         \"TooManyFunctions\",\n    \
         \"LongParameterList\",\n    \
         \"LongMethod\",\n\
         )\n\n",
    );
    content.push_str(&format!("package {pkg}\n\n"));
    for import in &imports {
        content.push_str(import);
        content.push('\n');
    }
    if !imports.is_empty() {
        content.push('\n');
    }
    content.push_str(&body);

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

/// Check if a type string refers to an enum in the API surface.
fn is_enum_param(ty: &str, enum_names: &std::collections::HashSet<&str>) -> bool {
    enum_names.contains(ty)
}

/// Extract the Named type from a TypeRef (e.g., "RouteBuilder" from TypeRef::Named("RouteBuilder")).
/// Used to identify enum parameter types.
fn extract_named_type(ty: &str) -> Option<&str> {
    if !ty.is_empty() && ty.chars().next().unwrap().is_uppercase() {
        Some(ty)
    } else {
        None
    }
}

/// Emit a single instance method on the JNI DefaultClient class.
///
/// Wrapper strategy:
/// - Bridge external fun takes `(handle: Long, requestJson: String)` for methods with params,
///   or just `(handle: Long)` for zero-param methods.
/// - When params are present, they are JSON-serialised via `MAPPER.writeValueAsString`:
///   - 1 param → `MAPPER.writeValueAsString(<paramName>)`
///   - 2+ params → `MAPPER.writeValueAsString(mapOf("p1" to p1, "p2" to p2, ...))`
/// - Complex return types (Named, Vec<non-u8>, Map, Optional) are deserialised via
///   `MAPPER.readValue(responseJson, ReturnType::class.java)`.
/// - `ByteArray` returns (Vec<u8>), `Boolean`, and primitive returns pass through directly.
/// - `Unit` returns drop response handling entirely.
/// - All wrappers run in `withContext(Dispatchers.IO)` when the method is async.
fn emit_jni_client_method(
    m: &crate::core::ir::MethodDef,
    class_name: &str,
    bridge_name: &str,
    out: &mut String,
    imports: &mut BTreeSet<String>,
) {
    if !m.doc.is_empty() {
        for line in m.doc.lines() {
            out.push_str(&format!("    // {line}\n"));
        }
    }
    let method_name = to_lower_camel(&m.name);
    let native_name = format!("native{}{}", to_pascal_case(class_name), to_pascal_case(&m.name));
    let async_kw = if m.is_async { "suspend " } else { "" };

    let params_with_types: Vec<String> = m.params.iter().map(|p| format_param_with_imports(p, imports)).collect();

    // Determine the public Kotlin return type for the wrapper signature.
    // Vec<u8> maps to ByteArray at the JNI boundary (no base64 overhead); the
    // generic Kotlin mapper would produce List<Byte> which is incompatible.
    // All other types use the standard Kotlin type mapper.
    let wrapper_return_ty = if is_binary_return_type(&m.return_type) {
        "ByteArray".to_string()
    } else if is_optional_binary_return_type(&m.return_type) {
        "ByteArray?".to_string()
    } else {
        kotlin_type_with_string_imports(&m.return_type, false, imports)
    };

    out.push_str(&format!(
        "    {async_kw}fun {method_name}({}): {wrapper_return_ty} {{\n",
        params_with_types.join(", ")
    ));

    // Build the bridge call expression, with JSON marshalling where needed.
    let bridge_call = build_bridge_call(m, bridge_name, &native_name);

    // Emit the method body with optional `withContext` wrapping.
    emit_method_body(m, out, &bridge_call, imports);

    out.push_str("    }\n\n");
}

/// Build the expression that calls the bridge, including any JSON serialisation.
///
/// Returns a string that produces the bridge's raw return value (String, ByteArray, Unit, etc.).
fn build_bridge_call(m: &crate::core::ir::MethodDef, bridge_name: &str, native_name: &str) -> String {
    if m.params.is_empty() {
        return format!("{bridge_name}.{native_name}(handle)");
    }
    if m.params.len() == 1 && is_binary_param_type(&m.params[0].ty) {
        let p = &m.params[0];
        let param_name = to_lower_camel(&p.name);
        let arg = if p.optional {
            format!("{param_name} ?: ByteArray(0)")
        } else {
            param_name
        };
        return format!("{bridge_name}.{native_name}(handle, {arg})");
    }
    // Build requestJson expression.
    let request_json_expr = if m.params.len() == 1 {
        let p = &m.params[0];
        let param_name = to_lower_camel(&p.name);
        // For optional (nullable) complex params, use `?.let { ... } ?: ""` so the
        // JNI shim receives the empty-string sentinel (not JSON `"null"`) for None.
        if p.optional {
            format!("{param_name}?.let {{ MAPPER.writeValueAsString(it) }} ?: \"\"")
        } else {
            format!("MAPPER.writeValueAsString({param_name})")
        }
    } else {
        let map_entries: Vec<String> = m
            .params
            .iter()
            .map(|p| {
                let name = to_lower_camel(&p.name);
                format!("\"{name}\" to {name}")
            })
            .collect();
        format!("MAPPER.writeValueAsString(mapOf({}))", map_entries.join(", "))
    };
    format!("{bridge_name}.{native_name}(handle, {request_json_expr})")
}

/// Emit the method body lines (withContext wrapper, return, JSON deserialisation).
fn emit_method_body(
    m: &crate::core::ir::MethodDef,
    out: &mut String,
    bridge_call: &str,
    imports: &mut BTreeSet<String>,
) {
    let needs_deserialize = needs_json_deserialize(&m.return_type);
    let return_kotlin_type = if needs_deserialize {
        Some(kotlin_type_with_string_imports(&m.return_type, false, imports))
    } else {
        None
    };

    match &m.return_type {
        TypeRef::Unit => {
            if m.is_async {
                out.push_str(&format!("        withContext(Dispatchers.IO) {{ {bridge_call} }}\n"));
            } else {
                out.push_str(&format!("        {bridge_call}\n"));
            }
        }
        _ if needs_deserialize => {
            // Bridge returns JSON String; deserialise to the rich Kotlin type.
            let kotlin_ty = return_kotlin_type.unwrap();
            // Strip trailing `?` from the class literal used in readValue.
            let base_ty = kotlin_ty.trim_end_matches('?');
            // Kotlin disallows generic type arguments on `::class.java`. When
            // `base_ty` carries any angle-bracketed generics (e.g.
            // `List<String>`, `Map<String, Long>`, `List<MyDto>`), route the
            // deserialisation through Jackson's `TypeReference<T>` instead.
            let use_type_reference = base_ty.contains('<');
            let deserialize_call = if use_type_reference {
                imports.insert("import com.fasterxml.jackson.core.type.TypeReference".to_string());
                format!("MAPPER.readValue(responseJson, object : TypeReference<{base_ty}>() {{}})")
            } else {
                format!("MAPPER.readValue(responseJson, {base_ty}::class.java)")
            };
            if m.is_async {
                out.push_str("        return withContext(Dispatchers.IO) {\n");
                out.push_str(&format!("            val responseJson = {bridge_call}\n"));
                out.push_str(&format!("            {deserialize_call}\n"));
                out.push_str("        }\n");
            } else {
                out.push_str(&format!("        val responseJson = {bridge_call}\n"));
                out.push_str(&format!("        return {deserialize_call}\n"));
            }
        }
        _ => {
            // Primitive, Boolean, ByteArray, String — pass through.
            if m.is_async {
                out.push_str(&format!(
                    "        return withContext(Dispatchers.IO) {{ {bridge_call} }}\n"
                ));
            } else {
                out.push_str(&format!("        return {bridge_call}\n"));
            }
        }
    }
}

/// Returns true when the IR type is `Vec<u8>` (binary data → `ByteArray`).
fn is_vec_u8(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(crate::core::ir::PrimitiveType::U8))
    )
}

fn is_binary_return_type(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Bytes) || is_vec_u8(ty)
}

fn is_optional_binary_return_type(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Optional(inner) if is_binary_return_type(inner))
}

fn is_binary_param_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Bytes => true,
        TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::Primitive(crate::core::ir::PrimitiveType::U8)),
        TypeRef::Optional(inner) => is_binary_param_type(inner),
        _ => false,
    }
}

/// Returns true when the bridge return type is a JSON String that must be
/// deserialised into a richer Kotlin type in the wrapper body.
fn needs_json_deserialize(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(_)),
        TypeRef::Map(_, _) => true,
        TypeRef::Vec(inner) => {
            // Vec<u8> → ByteArray (pass-through); other Vec → JSON String → needs deserialize.
            !matches!(inner.as_ref(), TypeRef::Primitive(crate::core::ir::PrimitiveType::U8))
        }
        _ => false,
    }
}

/// Emit a `Flow<ChunkType>` callbackFlow method for a streaming adapter,
/// using `handle: Long` as the first argument to the JNI start function
/// (instead of `inner: <JavaFacadeType>` used in Panama mode).
fn emit_jni_streaming_client_method(
    adapter: &crate::core::config::AdapterConfig,
    class_name: &str,
    bridge_name: &str,
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

    // Suppress detekt TooGenericExceptionCaught: the callbackFlow catch intentionally
    // catches Throwable to forward JNI RuntimeException, OOM Error, and any other
    // throwable into the Flow as a terminal signal for proper collector error handling.
    out.push_str("    @Suppress(\"TooGenericExceptionCaught\")\n");
    out.push_str(&format!(
        "    fun {method_name}({}): kotlinx.coroutines.flow.Flow<{item_type}> = kotlinx.coroutines.flow.callbackFlow {{\n",
        params.join(", ")
    ));
    out.push_str("        val mapper = com.fasterxml.jackson.databind.ObjectMapper()\n");
    out.push_str("            .registerModule(com.fasterxml.jackson.datatype.jdk8.Jdk8Module())\n");
    out.push_str("            .findAndRegisterModules()\n");
    out.push_str(
        "            .setPropertyNamingStrategy(com.fasterxml.jackson.databind.PropertyNamingStrategies.SNAKE_CASE)\n",
    );
    out.push_str("        val streamHandle: Long = withContext(Dispatchers.IO) {\n");
    out.push_str(&format!(
        "            {bridge_name}.{jni_start}(handle, mapper.writeValueAsString({first_param_name}))\n"
    ));
    out.push_str("        }\n");
    out.push_str("        try {\n");
    out.push_str("            while (true) {\n");
    out.push_str("                val chunkJson: String? = withContext(Dispatchers.IO) {\n");
    out.push_str(&format!("                    {bridge_name}.{jni_next}(streamHandle)\n"));
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
    out.push_str(&format!("            {bridge_name}.{jni_free}(streamHandle)\n"));
    out.push_str("        }\n");
    out.push_str("    }\n\n");
}

/// Map an IR `TypeRef` to a JNI-compatible Kotlin type string for `external fun` return types
/// on instance methods (where opaque handle semantics do not apply to the return).
///
/// JNI external funs must use primitive-width types and `String` for text.
/// Complex types (structs, enums) are passed as JSON-encoded `String` values.
/// `Vec<u8>` maps to `ByteArray` so binary responses (images, speech audio) avoid
/// base64 overhead through Jackson.
fn jni_return_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Unit => "Unit",
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "Boolean",
                PrimitiveType::I8 => "Byte",
                PrimitiveType::I16 => "Short",
                PrimitiveType::I32 => "Int",
                PrimitiveType::I64 => "Long",
                PrimitiveType::U8 => "Byte",
                PrimitiveType::U16 => "Short",
                PrimitiveType::U32 => "Int",
                PrimitiveType::U64 => "Long",
                PrimitiveType::F32 => "Float",
                PrimitiveType::F64 => "Double",
                PrimitiveType::Usize | PrimitiveType::Isize => "Long",
            }
        }
        TypeRef::String => "String",
        TypeRef::Bytes => "ByteArray",
        // Optional return → nullable String (JSON-encoded or null)
        TypeRef::Optional(inner) if is_binary_return_type(inner) => "ByteArray?",
        TypeRef::Optional(_) => "String?",
        // Named types (structs, enums, errors) → JSON-encoded String
        TypeRef::Named(_) => "String",
        // Vec<u8> (binary data) → ByteArray; other collections → JSON-encoded String
        TypeRef::Vec(inner) => {
            if matches!(inner.as_ref(), TypeRef::Primitive(crate::core::ir::PrimitiveType::U8)) {
                "ByteArray"
            } else {
                "String"
            }
        }
        TypeRef::Map(_, _) => "String",
        // Opaque handle → Long
        _ => "Long",
    }
}

/// Map an IR `TypeRef` to a JNI-compatible Kotlin type string for top-level function
/// return types, where opaque named types become `Long` (raw handle) instead of `String`.
fn jni_return_type_for_function(ty: &TypeRef, opaque_type_names: &std::collections::HashSet<&str>) -> &'static str {
    if let TypeRef::Named(n) = ty {
        if opaque_type_names.contains(n.as_str()) {
            return "Long";
        }
    }
    jni_return_type(ty)
}

/// Build the `external fun native<Method>(...)` parameter list for a function.
///
/// Opaque named types are passed as `Long` (raw handle pointer).
/// Complex non-opaque types (named structs, vec, map, optional-named) are serialized
/// to JSON `String` by the caller. Primitive types map directly to JNI primitives.
fn jni_params_for_function(
    f: &crate::core::ir::FunctionDef,
    opaque_type_names: &std::collections::HashSet<&str>,
) -> String {
    f.params
        .iter()
        .map(|p| {
            let jni_ty = jni_param_type_for_function(&p.ty, opaque_type_names);
            let name = to_lower_camel(&p.name);
            format!("{name}: {jni_ty}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// JNI param type for top-level function params.
///
/// Opaque named types → `Long`; everything else falls through to `jni_param_type`.
fn jni_param_type_for_function(ty: &TypeRef, opaque_type_names: &std::collections::HashSet<&str>) -> &'static str {
    // Unwrap Optional to check the inner type.
    let base = match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    if let TypeRef::Named(n) = base {
        if opaque_type_names.contains(n.as_str()) {
            return "Long";
        }
    }
    jni_param_type(ty)
}

fn jni_param_type(ty: &TypeRef) -> &'static str {
    if is_binary_param_type(ty) {
        return "ByteArray";
    }
    match ty {
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "Boolean",
                PrimitiveType::I8 => "Byte",
                PrimitiveType::I16 => "Short",
                PrimitiveType::I32 => "Int",
                PrimitiveType::I64 => "Long",
                PrimitiveType::U8 => "Byte",
                PrimitiveType::U16 => "Short",
                PrimitiveType::U32 => "Int",
                PrimitiveType::U64 => "Long",
                PrimitiveType::F32 => "Float",
                PrimitiveType::F64 => "Double",
                PrimitiveType::Usize | PrimitiveType::Isize => "Long",
            }
        }
        TypeRef::String => "String",
        // All complex types (named, optional, vec, map) are passed as JSON String.
        _ => "String",
    }
}

/// Emit `external fun nativeNew<TypeName>(params...): Long` declarations in the
/// Bridge object for every entry in `config.client_constructors` that names an
/// opaque type in the API surface.
///
/// Each `*const c_char` param maps to `String`; enum (Named) params that refer to
/// enums in the API surface map to `Int`; other param types are mapped to `Long`.
/// The return type is always `Long` (raw Box pointer).
fn emit_constructor_jni_external_funs(
    out: &mut String,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    exception_class: &str,
) {
    let opaque_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

    let enum_names: std::collections::HashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();

    let mut sorted: Vec<(&str, &ClientConstructorConfig)> = config
        .client_constructors
        .iter()
        .filter(|(name, _)| opaque_names.contains(name.as_str()))
        .map(|(name, ctor)| (name.as_str(), ctor))
        .collect();
    sorted.sort_by_key(|(name, _)| *name);

    if sorted.is_empty() {
        return;
    }

    out.push_str("\n    // JNI constructor external funs — implementations are Rust JNI shims.\n");
    for (type_name, ctor) in sorted {
        let native_name = format!("nativeNew{}", to_pascal_case(type_name));
        let params: Vec<String> = ctor
            .params
            .iter()
            .map(|p| {
                let kt_ty = if p.ty.contains("c_char") {
                    "String".to_string()
                } else if is_enum_param(&p.ty, &enum_names) {
                    "Int".to_string()
                } else {
                    "Long".to_string()
                };
                let param_name = to_lower_camel(&p.name);
                format!("{param_name}: {kt_ty}")
            })
            .collect();
        let params_str = params.join(", ");
        out.push_str(&format!(
            "    @Throws({exception_class}::class)\n    external fun {native_name}({params_str}): Long\n"
        ));
    }
}

/// Emit a `fun create(params...): TypeName` factory method inside the
/// companion object of the JNI client class.  Calls `Bridge.nativeNew<TypeName>(...)`
/// and wraps the returned `Long` handle in a new instance.
///
/// Enum-typed parameters are converted to their discriminant (`ordinal`) when passed
/// to the native function.
fn emit_jni_client_factory(
    class_name: &str,
    bridge_name: &str,
    ctor: &ClientConstructorConfig,
    api: &ApiSurface,
    out: &mut String,
) {
    let native_name = format!("nativeNew{}", to_pascal_case(class_name));

    let enum_names: std::collections::HashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();

    let params: Vec<String> = ctor
        .params
        .iter()
        .map(|p| {
            let kt_ty = if p.ty.contains("c_char") {
                "String".to_string()
            } else if is_enum_param(&p.ty, &enum_names) {
                let enum_name = extract_named_type(&p.ty).unwrap_or("Any");
                enum_name.to_string()
            } else {
                "Long".to_string()
            };
            let param_name = to_lower_camel(&p.name);
            format!("{param_name}: {kt_ty}")
        })
        .collect();

    let call_args: Vec<String> = ctor
        .params
        .iter()
        .map(|p| {
            let param_name = to_lower_camel(&p.name);
            if is_enum_param(&p.ty, &enum_names) {
                format!("{param_name}.ordinal")
            } else {
                param_name
            }
        })
        .collect();

    let params_str = params.join(", ");
    let call_args_str = call_args.join(", ");
    out.push_str(&format!(
        "        fun create({params_str}): {class_name} = {class_name}({bridge_name}.{native_name}({call_args_str}))\n"
    ));
}

/// Emit `external fun nativeRegister<Trait>`, `nativeUnregister<Trait>`, and
/// `nativeClear<Trait>s` declarations for every configured `[[crates.trait_bridges]]`
/// entry that does not list `kotlin_android` in its `exclude_languages` list.
///
/// The register fun signature receives the user-implemented `I<Trait>` interface
/// as a generic JVM `Any` reference; the Rust JNI shim is responsible for holding a
/// global reference and trampolining trait method calls back into the JVM.
///
/// Each generated `external fun` is annotated `@Throws(<Bridge>Exception::class)`
/// because both the Rust registration logic and the upcall vtable assembly can fail.
fn emit_trait_bridge_jni_external_funs(
    out: &mut String,
    config: &ResolvedCrateConfig,
    exception_class: &str,
    kotlin_package: &str,
    emitted_native_names: &std::collections::HashSet<String>,
) {
    let bridges: Vec<_> = config
        .trait_bridges
        .iter()
        .filter(|b| !b.exclude_languages.iter().any(|l| l == "kotlin_android"))
        .collect();
    if bridges.is_empty() {
        return;
    }
    out.push_str("\n    // JNI trait-bridge external funs — implementations are Rust JNI shims.\n");
    for bridge in &bridges {
        let trait_pascal = to_pascal_case(&bridge.trait_name);
        // The managed Kotlin interface lives in the same package as the bridge object;
        // the fully-qualified reference is used so callers can pass any class that
        // implements I<Trait> without an extra import in the bridge file.
        let iface_fqn = format!("{kotlin_package}.I{trait_pascal}");
        if bridge.register_fn.is_some() {
            let native_name = format!("nativeRegister{trait_pascal}");
            // Skip if already emitted from the API.
            if !emitted_native_names.contains(&native_name) {
                out.push_str(&format!(
                    "\n    @Throws({exception_class}::class)\n    external fun {native_name}(impl: {iface_fqn})\n"
                ));
            }
        }
        if bridge.unregister_fn.is_some() {
            let native_name = format!("nativeUnregister{trait_pascal}");
            // Skip if already emitted from the API.
            if !emitted_native_names.contains(&native_name) {
                out.push_str(&format!(
                    "    @Throws({exception_class}::class)\n    external fun {native_name}(name: String)\n"
                ));
            }
        }
        if bridge.clear_fn.is_some() {
            let native_name = format!("nativeClear{trait_pascal}s");
            // Skip if already emitted from the API.
            if !emitted_native_names.contains(&native_name) {
                out.push_str(&format!(
                    "    @Throws({exception_class}::class)\n    external fun {native_name}()\n"
                ));
            }
        }
    }
}

/// Resolve the Kotlin package for JNI-mode output.
///
/// Prefers `[crates.kotlin_android] package`, then `[crates.kotlin] package`,
/// then falls back to `config.kotlin_package()`.
fn jni_kotlin_package(config: &ResolvedCrateConfig) -> String {
    config
        .kotlin_android
        .as_ref()
        .and_then(|a| a.package.clone())
        .or_else(|| config.kotlin.as_ref().and_then(|k| k.package.clone()))
        .unwrap_or_else(|| config.kotlin_package())
}

/// Resolve the output path for a JNI-mode Kotlin file.
///
/// Uses `[crates.output] kotlin_android` when available, otherwise falls
/// back to `[crates.output] kotlin`, and finally the conventional
/// `packages/kotlin/src/main/kotlin/<pkg>/` layout.
fn jni_output_path(config: &ResolvedCrateConfig, filename: &str) -> PathBuf {
    if let Some(android_out) = config.output_for("kotlin_android") {
        return android_out.join(filename);
    }
    let kotlin_root = config
        .output_for("kotlin")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "packages/kotlin".to_string());
    let package = jni_kotlin_package(config);
    let package_path = package.replace('.', "/");
    if config.explicit_output.kotlin.is_some() {
        PathBuf::from(&kotlin_root).join(filename)
    } else {
        PathBuf::from(&kotlin_root)
            .join("src/main/kotlin")
            .join(&package_path)
            .join(filename)
    }
}
