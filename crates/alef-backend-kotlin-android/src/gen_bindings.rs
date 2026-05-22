//! Kotlin source emitter for the AAR module (JNI mode).
//!
//! Produces a pure-Kotlin JNI layout with no bundled Java facade:
//!
//! - `<Module>Bridge.kt` — a Kotlin `object` with `external fun` JNI
//!   declarations and `init { System.loadLibrary("<crate>_jni") }`.
//! - `DefaultClient.kt` — coroutine-friendly client class holding a `Long`
//!   handle when the API has methodful types.
//! - Data classes, enums, and error types are emitted as `.kt` files via the
//!   kotlin backend's pub helpers.
//!
//! `KotlinFfiStyle::Jni` is forced by the parent backend (`lib.rs`) before
//! this module is called, so `config.kotlin_ffi_style()` will always return
//! `KotlinFfiStyle::Jni` here.

use std::collections::BTreeSet;
use std::path::Path;

use alef_backend_kotlin::{
    emit_enum_pub, emit_error_type_pub, emit_jni_bridge_object, emit_jni_client_class, emit_kdoc_pub,
    emit_type_pub_with_defaults_sealed_and_constructible, kotlin_type_str_pub, to_lower_camel, to_pascal_case,
};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::ir::{ApiSurface, TypeDef};
use alef_core::jni::bridge_class_name;

use crate::naming::kotlin_package;

/// Emit all Kotlin source files for the AAR module.
///
/// `kotlin_source_dir` is the resolved Kotlin source destination —
/// `<project_root>/src/main/kotlin/<dotted_package_as_path>/` in the Gradle
/// Android source-set layout.
pub fn emit(api: &ApiSurface, config: &ResolvedCrateConfig, kotlin_source_dir: &Path) -> Vec<GeneratedFile> {
    let package = kotlin_package(config);
    let mut files = Vec::new();

    // Collect type aliases that should be treated as excluded.
    //
    // Two sources contribute:
    //   1. `kotlin_android.exclude_types` (explicit user override).
    //   2. Trait-bridge `type_alias` values whose `param_name` is listed in
    //      `kotlin_android.exclude_functions`. When the user opts out of the
    //      bridge function (e.g. because the JNI bridge has no trait-handle
    //      implementation yet), the bridge's type alias is still unresolvable
    //      in Kotlin code — its corresponding `options_field` on the bridge's
    //      `options_type` would emit a dangling reference like
    //      `val visitor: VisitorHandle?`. Treat the alias as excluded so the
    //      field is dropped along with the function.
    let kotlin_android_excluded_function_names: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let mut effective_excluded_types: std::collections::HashSet<String> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    for bridge in &config.trait_bridges {
        if bridge.exclude_languages.iter().any(|l| l == "kotlin_android") {
            if let Some(alias) = &bridge.type_alias {
                effective_excluded_types.insert(alias.clone());
            }
        }
        if let Some(name) = bridge.param_name.as_deref() {
            if kotlin_android_excluded_function_names.contains(name) {
                if let Some(alias) = &bridge.type_alias {
                    effective_excluded_types.insert(alias.clone());
                }
            }
        }
    }

    // Build an `enum_name → default_variant` map so the data-class emitter
    // can synthesise constructor defaults for Named enum fields (e.g.
    // `headingStyle: HeadingStyle = HeadingStyle.ATX`). The Jackson Kotlin
    // module rejects deserialization of partial JSON (e.g. tests that pass
    // `{"extract_metadata":true}` into `ConversionOptions`) when a
    // non-nullable field has no default — every non-optional Named enum
    // field needs one to round-trip.
    //
    // Only true Kotlin enums are included here. Tagged/untagged enums (with
    // `serde_tag` or `serde_untagged` set) are emitted as sealed classes in
    // Kotlin and are handled differently (variant names are PascalCase, not
    // SCREAMING_SNAKE_CASE).
    //
    // Enums without a declared `#[default]` variant map to an empty string;
    // the emitter treats this as "no synthesisable default" and falls
    // through to the type-based path (null for optional fields, no default
    // for required ones). The mere presence of the entry distinguishes
    // true enums from data-class struct types and sealed classes.
    let enum_defaults: std::collections::HashMap<String, String> = api
        .enums
        .iter()
        .filter(|en| en.serde_tag.is_none() && !en.serde_untagged && en.variants.iter().all(|v| v.fields.is_empty()))
        .map(|en| {
            let default_variant = en
                .variants
                .iter()
                .find(|v| v.is_default)
                .map(|v| v.name.clone())
                .unwrap_or_default();
            (en.name.clone(), default_variant)
        })
        .collect();

    // Build the set of Kotlin sealed-class names — Rust enums emitted as
    // sealed classes because they are tagged (`#[serde(tag = "...")]`) or
    // untagged (`#[serde(untagged)]`).  Data-class fields whose declared
    // type references one of these names need a `@field:JsonSerialize(\`as\` = ...)`
    // (or `contentAs` for collections) annotation so Jackson routes through
    // the sealed class's custom serializer (which emits the discriminator)
    // instead of the runtime variant's default POJO serializer.
    let sealed_class_names: std::collections::HashSet<String> = api
        .enums
        .iter()
        .filter(|en| en.serde_tag.is_some() || en.serde_untagged)
        .map(|en| en.name.clone())
        .collect();

    // Non-enum, non-trait, non-opaque data class types whose Rust source has a
    // `Default` impl (has_default = true). For fields whose declared type
    // references one of these names, the emitter can safely synthesize
    // `= Name()` as the constructor default — preventing Jackson's Kotlin
    // module from raising `MissingKotlinParameterException` when the wire
    // JSON omits the nested struct (the common shape of partial-update
    // payloads in test fixtures).
    let default_constructible_types: std::collections::HashSet<String> = api
        .types
        .iter()
        .filter(|t| !t.is_trait && !t.is_opaque && t.has_default)
        .map(|t| t.name.clone())
        .collect();

    // Bridge object: external fun declarations + System.loadLibrary init block.
    let mut bridge_file = emit_jni_bridge_object(api, config);
    bridge_file.path = kotlin_source_dir.join(bridge_file.path.file_name().expect("bridge file must have a filename"));
    files.push(bridge_file);

    // BridgeException: a RuntimeException subclass thrown by the JNI shim when
    // a native call fails.  The Rust ERROR_CLASS constant references this class
    // as `<package>/<BridgeName>Exception`.  Without this file the JVM raises
    // NoClassDefFoundError on the first JNI call that needs to propagate an error.
    let bridge = bridge_class_name(&config.name);
    let exception_class = format!("{bridge}Exception");
    let exception_content = format!(
        "// Generated by alef. Do not edit by hand.\n\n\
         package {package}\n\n\
         class {exception_class}(message: String?, cause: Throwable?) : RuntimeException(message, cause) {{\n\
         {}\n\
         }}\n",
        "    constructor(message: String?) : this(message, null)"
    );
    files.push(GeneratedFile {
        path: kotlin_source_dir.join(format!("{exception_class}.kt")),
        content: exception_content,
        generated_header: false,
    });

    // DefaultClient.kt — only emitted when the API has methodful opaque types.
    if let Some(mut client_file) = emit_jni_client_class(api, config, Some(&package)) {
        client_file.path = kotlin_source_dir.join("DefaultClient.kt");
        files.push(client_file);
    }

    // Data classes, enums, and error types as pure Kotlin.
    for ty in &api.types {
        if ty.is_opaque || ty.is_trait || ty.binding_excluded {
            continue;
        }
        // Skip whole types whose name is in the effective exclude set
        // (e.g. trait-bridge `VisitorHandle` when the bridge function is
        // excluded — the alias has no Kotlin representation).
        if effective_excluded_types.contains(&ty.name) {
            continue;
        }
        let mut imports: BTreeSet<String> = BTreeSet::new();
        let mut body = String::new();
        // Drop fields whose type references an excluded alias so the data
        // class definition does not emit a dangling reference. The bridge
        // function is already filtered out of the module facade, so its
        // companion field cannot be set by callers — defaulting it to
        // `Default::default()` Rust-side preserves runtime correctness.
        let needs_field_filter = ty
            .fields
            .iter()
            .any(|f| effective_excluded_types.iter().any(|name| f.ty.references_named(name)));
        if needs_field_filter {
            let mut filtered = ty.clone();
            filtered
                .fields
                .retain(|f| !effective_excluded_types.iter().any(|name| f.ty.references_named(name)));
            emit_type_pub_with_defaults_sealed_and_constructible(
                &filtered,
                &mut body,
                &mut imports,
                &enum_defaults,
                &sealed_class_names,
                &default_constructible_types,
            );
        } else {
            emit_type_pub_with_defaults_sealed_and_constructible(
                ty,
                &mut body,
                &mut imports,
                &enum_defaults,
                &sealed_class_names,
                &default_constructible_types,
            );
        }
        if body.trim().is_empty() {
            continue;
        }
        let content = assemble_kt_content(&package, &imports, &body);
        files.push(GeneratedFile {
            path: kotlin_source_dir.join(format!("{}.kt", ty.name)),
            content,
            generated_header: false,
        });
    }

    for en in &api.enums {
        if en.binding_excluded {
            continue;
        }
        let mut body = String::new();
        emit_enum_pub(en, &mut body, &package);
        if body.trim().is_empty() {
            continue;
        }
        let content = assemble_kt_content(&package, &BTreeSet::new(), &body);
        files.push(GeneratedFile {
            path: kotlin_source_dir.join(format!("{}.kt", en.name)),
            content,
            generated_header: false,
        });
    }

    for error in &api.errors {
        let mut imports: BTreeSet<String> = BTreeSet::new();
        let mut body = String::new();
        emit_error_type_pub(error, &mut body, &mut imports);
        if body.trim().is_empty() {
            continue;
        }
        let content = assemble_kt_content(&package, &imports, &body);
        files.push(GeneratedFile {
            path: kotlin_source_dir.join(format!("{}.kt", error.name)),
            content,
            generated_header: false,
        });
    }

    emit_trait_interfaces(api, config, kotlin_source_dir, &package, &mut files);

    // Emit the free-function facade object (Module.kt) when visible functions exist.
    emit_module_kt(api, config, kotlin_source_dir, &package, &mut files);

    files
}

fn emit_trait_interfaces(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    kotlin_source_dir: &Path,
    package: &str,
    files: &mut Vec<GeneratedFile>,
) {
    for bridge in &config.trait_bridges {
        if bridge
            .exclude_languages
            .iter()
            .any(|language| language == "kotlin_android")
        {
            continue;
        }
        let Some(trait_def) = api
            .types
            .iter()
            .find(|typ| typ.is_trait && typ.name == bridge.trait_name && !typ.binding_excluded)
        else {
            continue;
        };

        let interface_name = format!("I{}", trait_def.name);
        let mut imports = BTreeSet::new();
        let mut body = String::new();
        emit_kdoc_pub(&mut body, &trait_def.doc, "");
        body.push_str(&format!("interface {interface_name} {{\n"));
        if bridge.super_trait.is_some() {
            body.push_str("    fun name(): String\n");
            body.push_str("    fun version(): String\n");
            body.push_str("    fun initialize() {}\n");
            body.push_str("    fun shutdown() {}\n");
        }
        emit_trait_methods(trait_def, &mut imports, &mut body);
        body.push_str("}\n");

        let content = assemble_kt_content(package, &imports, &body);
        files.push(GeneratedFile {
            path: kotlin_source_dir.join(format!("{interface_name}.kt")),
            content,
            generated_header: false,
        });
    }
}

fn emit_trait_methods(trait_def: &TypeDef, imports: &mut BTreeSet<String>, body: &mut String) {
    for method in &trait_def.methods {
        if method.sanitized || method.is_static {
            continue;
        }
        emit_kdoc_pub(body, &method.doc, "    ");
        let suspend_keyword = if method.is_async { "suspend " } else { "" };
        let method_name = to_lower_camel(&method.name);
        let params = method
            .params
            .iter()
            .map(|param| {
                let name = to_lower_camel(&param.name);
                let ty = kotlin_type_str_pub(&param.ty, param.optional, imports);
                format!("{name}: {ty}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let return_type = kotlin_type_str_pub(&method.return_type, false, imports);
        if return_type == "Unit" {
            body.push_str(&format!("    {suspend_keyword}fun {method_name}({params})\n"));
        } else {
            body.push_str(&format!(
                "    {suspend_keyword}fun {method_name}({params}): {return_type}\n"
            ));
        }
    }
}

/// Emit `<Module>.kt` — a Kotlin `object` that re-exposes every free function
/// by delegating to `<Module>Bridge`. This preserves the ergonomic call-site
/// pattern `Module.foo(...)` while keeping all JNI declarations in the Bridge.
///
/// Opaque handle types appear in two shapes in the API:
///
/// 1. **Client shape** — an opaque type with instance methods (e.g.
///    `DefaultClient` in liter-llm).  This is handled by
///    `emit_jni_client_class` which emits a `DefaultClient.kt` class with one
///    `suspend fun` per method.  Top-level free functions that return this
///    type produce the same class.
/// 2. **Handle shape** — an opaque type with no instance methods (e.g.
///    `CrawlEngineHandle` in kreuzcrawl).  All operations on the handle are
///    top-level free functions taking the handle as their first parameter.
///    This emitter generates a one-off `<TypeName>.kt` wrapper class
///    implementing `AutoCloseable` whose `close()` calls the bridge's
///    `nativeFree<TypeName>` destructor.
///
/// In both cases:
/// - Top-level fns returning the opaque type return the wrapper class instead
///   of `Long` (the raw bridge return).
/// - Top-level fns taking the opaque type as a parameter accept the wrapper
///   class and pass `.handle` to the bridge call.
fn emit_module_kt(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    kotlin_source_dir: &Path,
    package: &str,
    files: &mut Vec<GeneratedFile>,
) {
    use alef_backend_kotlin::to_lower_camel;

    let module_name = to_pascal_case(&config.name);
    let bridge_name = format!("{module_name}Bridge");

    // Set of all opaque (non-trait) type names.
    let opaque_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

    // Subset of opaque types that have at least one visible instance method.
    // These are the "client shape" — `emit_jni_client_class` already emits a
    // Kotlin class for them with AutoCloseable + close().
    let client_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && t.methods.iter().any(|m| !m.sanitized && !m.is_static))
        .map(|t| t.name.as_str())
        .collect();

    let exclude_functions: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();

    let visible_functions: Vec<_> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
        .collect();

    // Collect "handle shape" types: opaque types that are NOT clients but
    // appear somewhere on the free-function surface (as a return or a
    // parameter).  Each one gets its own AutoCloseable wrapper class file.
    let handle_only_types: std::collections::BTreeMap<&str, &alef_core::ir::TypeDef> = {
        let mut map = std::collections::BTreeMap::new();
        let by_name: std::collections::HashMap<&str, &alef_core::ir::TypeDef> =
            api.types.iter().map(|t| (t.name.as_str(), t)).collect();
        for f in &visible_functions {
            if let alef_core::ir::TypeRef::Named(n) = &f.return_type {
                if opaque_type_names.contains(n.as_str()) && !client_type_names.contains(n.as_str()) {
                    if let Some(ty) = by_name.get(n.as_str()) {
                        map.insert(n.as_str(), *ty);
                    }
                }
            }
            for p in &f.params {
                if let alef_core::ir::TypeRef::Named(n) = unwrap_optional(&p.ty) {
                    if opaque_type_names.contains(n.as_str()) && !client_type_names.contains(n.as_str()) {
                        if let Some(ty) = by_name.get(n.as_str()) {
                            map.insert(n.as_str(), *ty);
                        }
                    }
                }
            }
        }
        map
    };

    // Emit a one-off wrapper class file per handle-only opaque type.
    for (type_name, type_def) in &handle_only_types {
        let class_name = *type_name;
        let free_name = format!("nativeFree{}", to_pascal_case(class_name));
        let mut body = String::new();
        let mut imports: BTreeSet<String> = BTreeSet::new();

        // Emit the IR-supplied rustdoc for the opaque type; omit Kdoc if no doc
        // is provided to avoid leaking implementation details (JNI handles are internal).
        if !type_def.doc.is_empty() {
            emit_kdoc_pub(&mut body, &type_def.doc, "");
        }
        body.push_str(&format!(
            "class {class_name} internal constructor(internal val handle: Long) : AutoCloseable {{\n"
        ));
        body.push_str(&format!(
            "    override fun close() {{ {bridge_name}.{free_name}(handle) }}\n"
        ));

        // Collect streaming adapters owned by this opaque type.
        let streaming_adapters_for_type: Vec<&alef_core::config::AdapterConfig> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, alef_core::config::AdapterPattern::Streaming))
            .filter(|a| !a.skip_languages.iter().any(|l| l == "kotlin_android"))
            .filter(|a| {
                a.owner_type
                    .as_deref()
                    .map(|owner| owner == class_name)
                    .unwrap_or(false)
            })
            .collect();

        // If there are streaming adapters, emit Flow wrapper methods and add imports.
        if !streaming_adapters_for_type.is_empty() {
            imports.insert("import com.fasterxml.jackson.databind.ObjectMapper".to_string());
            imports.insert("import com.fasterxml.jackson.datatype.jdk8.Jdk8Module".to_string());
            imports.insert("import com.fasterxml.jackson.databind.PropertyNamingStrategies".to_string());
            imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
            imports.insert("import kotlinx.coroutines.flow.Flow".to_string());
            imports.insert("import kotlinx.coroutines.flow.callbackFlow".to_string());
            imports.insert("import kotlinx.coroutines.withContext".to_string());
            imports.insert("import kotlinx.coroutines.channels.awaitClose".to_string());

            // Add a mapper field and emit streaming methods.
            body.push_str("\n    private val mapper = ObjectMapper()\n");
            body.push_str("        .registerModule(Jdk8Module())\n");
            body.push_str("        .findAndRegisterModules()\n");
            body.push_str("        .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)\n\n");

            for adapter in &streaming_adapters_for_type {
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

                // Note: We do not add imports for param and item types here because
                // they are expected to be simple names (CrawlEvent, CrawlStreamRequest, etc.)
                // that exist in the same package as this generated file. The Kotlin backend
                // is responsible for emitting those DTOs alongside this file. If types
                // were to come from a different package, they would have full paths in the
                // adapter config, and we'd need to strip to simple names here. For now,
                // the safe assumption is same-package types with no import needed.

                // ktfmt collapses expression-bodied functions back to a single line, so
                // we emit the natural `fun ... = callbackFlow {` shape and rely on ktfmt for
                // formatting. The ktlint multiline-expression-wrapping rule is disabled in
                // the generated `.editorconfig` because it conflicts with ktfmt here.
                body.push_str("    @Suppress(\"TooGenericExceptionCaught\")\n");
                body.push_str(&format!(
                    "    fun {method_name}({}): Flow<{item_type}> = callbackFlow {{\n",
                    if params.is_empty() {
                        "".to_string()
                    } else {
                        params.join(", ")
                    }
                ));
                body.push_str("        val streamHandle: Long = withContext(Dispatchers.IO) {\n");
                body.push_str(&format!(
                    "            {bridge_name}.{jni_start}(handle, mapper.writeValueAsString({first_param_name}))\n"
                ));
                body.push_str("        }\n");
                body.push_str("        try {\n");
                body.push_str("            while (true) {\n");
                body.push_str("                val chunkJson: String? = withContext(Dispatchers.IO) {\n");
                body.push_str(&format!("                    {bridge_name}.{jni_next}(streamHandle)\n"));
                body.push_str("                }\n");
                body.push_str("                if (chunkJson == null) break\n");
                body.push_str(&format!(
                    "                val chunk = mapper.readValue(chunkJson, {item_type}::class.java)\n"
                ));
                body.push_str("                send(chunk)\n");
                body.push_str("            }\n");
                body.push_str("            close()\n");
                body.push_str("        } catch (e: Throwable) {\n");
                body.push_str("            close(e)\n");
                body.push_str("        }\n");
                body.push_str("        awaitClose {\n");
                body.push_str(&format!("            {bridge_name}.{jni_free}(streamHandle)\n"));
                body.push_str("        }\n");
                body.push_str("    }\n\n");
            }
        }

        body.push_str("}\n");
        let content = assemble_kt_content(package, &imports, &body);
        files.push(GeneratedFile {
            path: kotlin_source_dir.join(format!("{class_name}.kt")),
            content,
            generated_header: false,
        });
    }

    if visible_functions.is_empty() {
        return;
    }

    // Helper: return true when the TypeRef is a non-opaque named type, i.e. a
    // data-class DTO that crosses the JNI boundary as JSON and should be
    // serialized/deserialized by Jackson in the high-level facade.
    let is_dto_named = |ty: &alef_core::ir::TypeRef| -> bool {
        match ty {
            alef_core::ir::TypeRef::Named(n) => !opaque_type_names.contains(n.as_str()),
            _ => false,
        }
    };

    // Helper to resolve the facade Kotlin type for a return TypeRef.
    // Opaque types become their wrapper class; non-opaque Named types are
    // exposed as-is (Jackson deserialization makes them real Kotlin objects);
    // generic containers (`Vec<_>`, `HashMap<_,_>`, and optional variants)
    // are rendered recursively via `render_kotlin_type` so Jackson's
    // `TypeReference<...>` and the public signature stay in lockstep.
    let facade_return_type = |ty: &alef_core::ir::TypeRef| -> String {
        if let alef_core::ir::TypeRef::Named(n) = ty {
            if opaque_type_names.contains(n.as_str()) {
                return n.clone();
            }
            // Non-opaque Named: expose the real Kotlin type.
            return n.clone();
        }
        // Generic container (Vec / Map / Option<Vec|Map>): render recursively.
        if matches!(
            unwrap_optional(ty),
            alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _)
        ) {
            return render_kotlin_type(ty, &opaque_type_names);
        }
        jni_return_type_str(ty).to_string()
    };

    // Helper to resolve the facade Kotlin type for a parameter TypeRef.
    // Non-opaque Named types (optionally wrapped) become their Kotlin class so
    // callers pass typed objects rather than raw JSON strings.  Generic
    // containers (Vec / HashMap, possibly Option-wrapped) render recursively
    // so the public signature matches the Jackson-serialized payload.
    let facade_param_type = |ty: &alef_core::ir::TypeRef| -> String {
        let inner = unwrap_optional(ty);
        if let alef_core::ir::TypeRef::Named(n) = inner {
            if opaque_type_names.contains(n.as_str()) {
                return n.clone();
            }
            // Non-opaque Named: expose the real Kotlin type.
            return n.clone();
        }
        if matches!(
            inner,
            alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _)
        ) {
            return render_kotlin_type(inner, &opaque_type_names);
        }
        jni_param_type_str(ty).to_string()
    };

    // Helper: detect if a TypeRef is a Vec of DTOs (needs deserialization).
    // Retained for the legacy DTO-list emission path; broader generic-container
    // detection lives in `is_generic_container` below.
    let is_vec_of_dtos = |ty: &alef_core::ir::TypeRef| -> bool {
        if let alef_core::ir::TypeRef::Vec(inner) = ty {
            if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                return !opaque_type_names.contains(n.as_str());
            }
        }
        false
    };

    // Helper: detect any generic-container TypeRef whose deserialization in
    // Kotlin requires `TypeReference<T>` rather than a `Class<T>` literal.
    // Matches `Vec<_>`, `HashMap<_, _>`, and either wrapped in a single
    // `Option<...>`.  The inner element type may itself be any TypeRef —
    // primitives, strings, named DTOs, or further nested generics.
    let is_generic_container = |ty: &alef_core::ir::TypeRef| -> bool {
        let base = unwrap_optional(ty);
        matches!(base, alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _))
    };

    // Determine whether any function needs Jackson serialization/deserialization.
    // If so, emit a private mapper field and add the necessary imports.
    let needs_jackson = visible_functions.iter().any(|f| {
        is_dto_named(&f.return_type)
            || is_generic_container(&f.return_type)
            || f.params
                .iter()
                .any(|p| is_dto_named(unwrap_optional(&p.ty)) || is_generic_container(unwrap_optional(&p.ty)))
    });

    let mut imports: BTreeSet<String> = BTreeSet::new();
    if needs_jackson {
        imports.insert("import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper".to_string());
        imports.insert("import com.fasterxml.jackson.databind.PropertyNamingStrategies".to_string());
        imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
        imports.insert("import kotlinx.coroutines.withContext".to_string());
    }
    // Check if any function returns a generic container that needs TypeReference.
    // Vec<_>, HashMap<_,_>, and Option<Vec<_>> / Option<HashMap<_,_>> all
    // require `TypeReference<...>` because Kotlin disallows generic type
    // arguments on `::class.java` literals.
    let has_generic_container_return = visible_functions.iter().any(|f| is_generic_container(&f.return_type));
    if has_generic_container_return {
        imports.insert("import com.fasterxml.jackson.core.type.TypeReference".to_string());
    }

    let mut body = String::new();

    body.push_str(&format!("object {module_name} {{\n"));
    if needs_jackson {
        // Rust serde defaults to snake_case wire keys; Kotlin properties are
        // camelCase. Configure the property naming strategy on the module
        // facade's mapper so Jackson translates between them automatically,
        // matching the streaming-method mapper above and the convention used
        // across the JVM/Kotlin backends.
        body.push_str("    private val mapper = jacksonObjectMapper()\n");
        body.push_str("        .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)\n\n");
    }

    for f in &visible_functions {
        emit_kdoc_pub(&mut body, &f.doc, "    ");
        let method_name = to_lower_camel(&f.name);
        let native_name = format!("native{}", to_pascal_case(&f.name));
        let return_ty = facade_return_type(&f.return_type);
        let returns_dto = is_dto_named(&f.return_type);
        let returns_vec_of_dtos = is_vec_of_dtos(&f.return_type);
        let returns_generic_container = is_generic_container(&f.return_type);

        // Build the public param list. Optional non-opaque Named params become
        // `TypeName? = null`; required ones become `TypeName`.
        let params: Vec<String> = f
            .params
            .iter()
            .map(|p| {
                let name = to_lower_camel(&p.name);
                let inner = unwrap_optional(&p.ty);
                let is_dto = is_dto_named(inner);
                if p.optional {
                    if is_dto {
                        // TypeName? = null — typed nullable with null default.
                        let ty_name = match inner {
                            alef_core::ir::TypeRef::Named(n) => n.clone(),
                            _ => unreachable!(),
                        };
                        format!("{name}: {ty_name}? = null")
                    } else if opaque_type_names.contains(match inner {
                        alef_core::ir::TypeRef::Named(n) => n.as_str(),
                        _ => "",
                    }) {
                        format!("{name}: {} = null", facade_param_type(&p.ty))
                    } else {
                        let ty = kotlin_nullable_type_for_optional(&p.ty);
                        format!("{name}: {ty} = null")
                    }
                } else {
                    let ty = facade_param_type(&p.ty);
                    format!("{name}: {ty}")
                }
            })
            .collect();

        // Build the bridge argument list. DTO params are serialized to JSON;
        // opaque params are unwrapped to `.handle`; generic containers
        // (`Vec<_>`, `HashMap<_,_>`) are serialized; everything else passes
        // through.
        let bridge_args: Vec<String> = f
            .params
            .iter()
            .map(|p| {
                let name = to_lower_camel(&p.name);
                let inner = unwrap_optional(&p.ty);
                // Opaque handle: unwrap to `.handle`.
                if let alef_core::ir::TypeRef::Named(n) = inner {
                    if opaque_type_names.contains(n.as_str()) {
                        return format!("{name}.handle");
                    }
                    // Non-opaque DTO: serialize to JSON.
                    if p.optional {
                        // Serialize if non-null, fall back to empty string.
                        return format!("{name}?.let {{ mapper.writeValueAsString(it) }} ?: \"\"");
                    }
                    return format!("mapper.writeValueAsString({name})");
                }
                // Generic container (Vec<_> or HashMap<_,_>): serialize to JSON.
                if matches!(
                    inner,
                    alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _)
                ) {
                    if p.optional {
                        return format!("{name}?.let {{ mapper.writeValueAsString(it) }} ?: \"\"");
                    }
                    return format!("mapper.writeValueAsString({name})");
                }
                // Nullable primitive scalar or String: null-coalesce to the JNI
                // zero value so the non-nullable `external fun` signature is satisfied.
                if p.optional {
                    let zero = jni_zero_literal(inner);
                    return format!("{name} ?: {zero}");
                }
                name
            })
            .collect();

        let bridge_call = format!("{bridge_name}.{native_name}({})", bridge_args.join(", "));

        // Determine body expression: deserialize from JSON when the return type
        // is a DTO or Vec<DTO>, wrap in opaque class when it is a handle, pass through
        // otherwise.
        let returns_opaque =
            matches!(&f.return_type, alef_core::ir::TypeRef::Named(n) if opaque_type_names.contains(n.as_str()));

        if returns_dto || returns_generic_container || returns_opaque || needs_jackson {
            // Suppress unused-warning on the legacy DTO-list flag — it remains
            // a useful diagnostic name for downstream readers but the generic
            // container path now subsumes its emission branch.
            let _ = returns_vec_of_dtos;
            // Emit a block body so we can introduce local vars for clarity.
            if returns_dto {
                let return_class = match &f.return_type {
                    alef_core::ir::TypeRef::Named(n) => n.clone(),
                    _ => unreachable!(),
                };
                body.push_str(&format!(
                    "    fun {method_name}({}): {return_ty} {{\n",
                    params.join(", ")
                ));
                body.push_str(&format!("        val resultJson = {bridge_call}\n"));
                body.push_str(&format!(
                    "        return mapper.readValue(resultJson, {return_class}::class.java)\n"
                ));
                body.push_str("    }\n\n");
                // Emit the suspend companion variant.
                emit_kdoc_pub(&mut body, &f.doc, "    ");
                body.push_str(&format!(
                    "    suspend fun {method_name}Async({}): {return_ty} =\n",
                    params.join(", ")
                ));
                body.push_str(&format!(
                    "        withContext(Dispatchers.IO) {{ {method_name}({}) }}\n",
                    f.params
                        .iter()
                        .map(|p| to_lower_camel(&p.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                body.push('\n');
            } else if returns_generic_container {
                // Generic container return: Kotlin disallows generic type
                // arguments on `::class.java`, so we route through Jackson's
                // `TypeReference<T>`.  The TypeReference body is the fully
                // rendered Kotlin type (e.g. `List<String>`, `Map<String, Long>`,
                // `List<MyDto>?` — `render_kotlin_type` handles every Vec /
                // Map / Option permutation recursively).
                let type_ref_body = render_kotlin_type(&f.return_type, &opaque_type_names);
                body.push_str(&format!(
                    "    fun {method_name}({}): {return_ty} {{\n",
                    params.join(", ")
                ));
                body.push_str(&format!("        val resultJson = {bridge_call}\n"));
                body.push_str(&format!(
                    "        return mapper.readValue(resultJson, object : TypeReference<{type_ref_body}>() {{}})\n"
                ));
                body.push_str("    }\n\n");
                // Emit the suspend companion variant.
                emit_kdoc_pub(&mut body, &f.doc, "    ");
                body.push_str(&format!(
                    "    suspend fun {method_name}Async({}): {return_ty} =\n",
                    params.join(", ")
                ));
                body.push_str(&format!(
                    "        withContext(Dispatchers.IO) {{ {method_name}({}) }}\n",
                    f.params
                        .iter()
                        .map(|p| to_lower_camel(&p.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                body.push('\n');
            } else if returns_opaque {
                let opaque_class = match &f.return_type {
                    alef_core::ir::TypeRef::Named(n) => n.clone(),
                    _ => unreachable!(),
                };
                body.push_str(&format!(
                    "    fun {method_name}({}): {return_ty} = {opaque_class}({bridge_call})\n",
                    params.join(", ")
                ));
            } else {
                body.push_str(&format!(
                    "    fun {method_name}({}): {return_ty} = {bridge_call}\n",
                    params.join(", ")
                ));
            }
        } else {
            body.push_str(&format!(
                "    fun {method_name}({}): {return_ty} = {bridge_call}\n",
                params.join(", "),
            ));
        }
    }
    body.push_str("}\n");

    let content = assemble_kt_content(package, &imports, &body);
    files.push(GeneratedFile {
        path: kotlin_source_dir.join(format!("{module_name}.kt")),
        content,
        generated_header: false,
    });
}

/// Return the inner `TypeRef` if `ty` is `Optional(inner)`, otherwise return `ty`.
fn unwrap_optional(ty: &alef_core::ir::TypeRef) -> &alef_core::ir::TypeRef {
    match ty {
        alef_core::ir::TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    }
}

/// Assemble a complete `.kt` file from package, imports, and body.
fn assemble_kt_content(package: &str, imports: &BTreeSet<String>, body: &str) -> String {
    let mut content = String::new();
    content.push_str("// Generated by alef. Do not edit by hand.\n");
    // File-level suppression annotations silence ktlint / detekt rules that are
    // inherently violated by generated code (trailing commas, annotation spacing,
    // when-entry bracing, etc.) and cannot be trivially fixed without a full
    // reformatter post-processing step.
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
         \"LongMethod\",\n\
         )\n\n",
    );
    content.push_str(&format!("package {package}\n\n"));
    for import in imports {
        content.push_str(import);
        content.push('\n');
    }
    if !imports.is_empty() {
        content.push('\n');
    }
    content.push_str(body);
    content
}

/// Return a nullable Kotlin type string for an optional facade parameter.
/// Used when `p.optional == true` so callers can pass `null` to skip the
/// param (e.g. `timeoutSecs: Long? = null`).  Nullable is idiomatic Kotlin
/// and avoids sentinel zero-value collisions (`0L`, `""`).
fn kotlin_nullable_type_for_optional(ty: &alef_core::ir::TypeRef) -> String {
    use alef_core::ir::{PrimitiveType, TypeRef};
    let base = match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    let non_null = match base {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "Boolean",
            PrimitiveType::I8 | PrimitiveType::U8 => "Byte",
            PrimitiveType::I16 | PrimitiveType::U16 => "Short",
            PrimitiveType::I32 | PrimitiveType::U32 => "Int",
            PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize => "Long",
            PrimitiveType::F32 => "Float",
            PrimitiveType::F64 => "Double",
        },
        TypeRef::String => "String",
        TypeRef::Named(n) => return format!("{n}?"),
        _ => "String",
    };
    format!("{non_null}?")
}

/// Return the Kotlin literal zero-value for a JNI primitive type.
///
/// Used when null-coalescing an optional facade param to satisfy the non-nullable
/// `external fun` bridge signature: `timeoutSecs ?: 0L`, `modelHint ?: ""`, etc.
fn jni_zero_literal(ty: &alef_core::ir::TypeRef) -> &'static str {
    use alef_core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::String => "\"\"",
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "false",
            PrimitiveType::F32 | PrimitiveType::F64 => "0.0",
            PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize => "0L",
            // All other integer widths map to Int at the JNI boundary.
            _ => "0",
        },
        // Named, Vec, Map and anything else: not expected here (handled by
        // earlier branches), but fall back to "" so we produce valid Kotlin.
        _ => "\"\"",
    }
}

fn jni_return_type_str(ty: &alef_core::ir::TypeRef) -> &'static str {
    use alef_core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Unit => "Unit",
        TypeRef::Primitive(p) => match p {
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
        },
        TypeRef::String => "String",
        TypeRef::Optional(_) => "String?",
        TypeRef::Bytes => "ByteArray",
        // Vec<u8> (binary data) → ByteArray; other collections → JSON-encoded String
        TypeRef::Vec(inner) => {
            if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) {
                "ByteArray"
            } else {
                "String"
            }
        }
        TypeRef::Named(_) | TypeRef::Map(_, _) => "String",
        _ => "Long",
    }
}

/// Recursively render a Kotlin type for a `TypeRef`, suitable for both
/// public signatures and Jackson `TypeReference<...>` bodies.
///
/// Handles every container shape that can survive a JNI JSON round-trip:
/// primitives, strings, named DTOs, opaque-handle types (rendered as the
/// wrapper class — note that those should not normally appear inside a
/// Jackson container, but the renderer stays consistent), nested `Vec<_>`,
/// `Map<K, V>`, and `Option<_>`.  Optional inner types render with Kotlin's
/// nullable `?` suffix; opaque handles inside containers render as their
/// wrapper class name (callers are responsible for converting opaque
/// payloads — Jackson never sees raw handles).
fn render_kotlin_type(ty: &alef_core::ir::TypeRef, opaque_type_names: &std::collections::HashSet<&str>) -> String {
    use alef_core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Unit => "Unit".to_string(),
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "Boolean".to_string(),
            PrimitiveType::I8 | PrimitiveType::U8 => "Byte".to_string(),
            PrimitiveType::I16 | PrimitiveType::U16 => "Short".to_string(),
            PrimitiveType::I32 | PrimitiveType::U32 => "Int".to_string(),
            PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize => "Long".to_string(),
            PrimitiveType::F32 => "Float".to_string(),
            PrimitiveType::F64 => "Double".to_string(),
        },
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "ByteArray".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Json => "Any".to_string(),
        TypeRef::Duration => "Long".to_string(),
        TypeRef::Named(n) => {
            // Both opaque-wrapper class names and DTO class names are rendered
            // verbatim — they share the same Kotlin identifier shape.
            let _ = opaque_type_names;
            n.clone()
        }
        TypeRef::Vec(inner) => format!("List<{}>", render_kotlin_type(inner, opaque_type_names)),
        TypeRef::Map(k, v) => format!(
            "Map<{}, {}>",
            render_kotlin_type(k, opaque_type_names),
            render_kotlin_type(v, opaque_type_names)
        ),
        TypeRef::Optional(inner) => {
            // `String?`, `List<String>?`, `Map<String, Long>?`.
            format!("{}?", render_kotlin_type(inner, opaque_type_names))
        }
    }
}

/// Map a `TypeRef` to a JNI parameter type string for the delegate wrapper.
fn jni_param_type_str(ty: &alef_core::ir::TypeRef) -> &'static str {
    use alef_core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Primitive(p) => match p {
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
        },
        TypeRef::String => "String",
        _ => "String",
    }
}
