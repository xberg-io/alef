use std::collections::BTreeSet;
use std::path::Path;

use crate::backends::kotlin::{emit_kdoc_pub, to_pascal_case};
use crate::backends::kotlin_android::template_env;
use crate::codegen::naming::kotlin_android_wrapper_object_name;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;

use super::assemble_kt_content;

/// Emit `<Module>.kt` — a Kotlin `object` that re-exposes every free function
/// by delegating to `<Module>Bridge`. This preserves the ergonomic call-site
/// pattern `Module.foo(...)` while keeping all JNI declarations in the Bridge.
///
/// Opaque handle types appear in two shapes in the API:
///
/// 1. **Client shape** — an opaque type with instance methods (e.g.
///    `DefaultClient` in sample-llm).  This is handled by
///    `emit_jni_client_class` which emits a `DefaultClient.kt` class with one
///    `suspend fun` per method.  Top-level free functions that return this
///    type produce the same class.
/// 2. **Handle shape** — an opaque type with no instance methods (e.g.
///    `CrawlEngineHandle` in sample-crawler).  All operations on the handle are
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
pub(super) fn emit_module_kt(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    kotlin_source_dir: &Path,
    package: &str,
    files: &mut Vec<GeneratedFile>,
) {
    use crate::backends::kotlin::to_lower_camel;

    let module_name = kotlin_android_wrapper_object_name(&config.name);
    // The bridge object is emitted by `gen_bindings::jni_emitter` as
    // `<Crate>Bridge` via `crate::core::jni::bridge_class_name(&config.name)`.
    // Use the same helper here so the facade's `Bridge.nativeXxx(...)` calls
    // resolve — concatenating `{module_name}Bridge` produced
    // `<Crate>ConverterBridge`, which never matches the on-disk bridge object.
    let bridge_name = crate::core::jni::bridge_class_name(&config.name);

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

    // Collect "handle shape" types: every opaque non-trait type that is NOT
    // a client (i.e. has no visible instance methods). Each one gets its own
    // AutoCloseable wrapper class file.
    //
    // Previously the set was restricted to opaque types that *also* appeared
    // as a return or parameter on the free-function surface. That misses
    // opaque types whose only public entrypoint is a static factory method
    // (e.g. `TokenCounter::new() -> Self`, lifted by the FFI layer as
    // `{prefix}_token_counter_new` but kept as a `@staticmethod` on the
    // class in alef's IR rather than a top-level function) and whose `&mut
    // self` consumers have been excluded via `[crates.exclude].functions`
    // (e.g. `apply_strategy`). Such types still need a Kotlin wrapper to give
    // callers a way to free the handle, and the FFI layer always emits a
    // matching `{prefix}_{type_snake}_free` symbol so the JNI bridge can
    // declare its `nativeFree{Type}` external fun. Mirroring Java's
    // `gen_opaque_handle_class` policy (emit one wrapper per opaque type)
    // keeps the two backends in lockstep and eliminates the stale-wrapper
    // class of failure (`TokenCounter.kt` references `nativeFreeTokenCounter`
    // that was never emitted in the generated bridge file).
    let handle_only_types: std::collections::BTreeMap<&str, &crate::core::ir::TypeDef> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && !client_type_names.contains(t.name.as_str()))
        .map(|t| (t.name.as_str(), t))
        .collect();

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
        body.push_str(&template_env::render(
            "handle_wrapper_header.jinja",
            minijinja::context! {
                class_name => class_name,
                bridge_name => bridge_name,
                free_name => free_name,
            },
        ));

        // Collect streaming adapters owned by this opaque type.
        let streaming_adapters_for_type: Vec<&crate::core::config::AdapterConfig> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
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
                body.push_str(&template_env::render(
                    "android_streaming_method.jinja",
                    minijinja::context! {
                        method_name => method_name,
                        params => params.join(", "),
                        item_type => item_type,
                        bridge_name => bridge_name,
                        jni_start => jni_start,
                        jni_next => jni_next,
                        jni_free => jni_free,
                        first_param_name => first_param_name,
                    },
                ));
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
    let is_dto_named = |ty: &crate::core::ir::TypeRef| -> bool {
        match ty {
            crate::core::ir::TypeRef::Named(n) => !opaque_type_names.contains(n.as_str()),
            _ => false,
        }
    };

    // Helper to resolve the facade Kotlin type for a return TypeRef.
    // Opaque types become their wrapper class; non-opaque Named types are
    // exposed as-is (Jackson deserialization makes them real Kotlin objects);
    // generic containers (`Vec<_>`, `HashMap<_,_>`, and optional variants)
    // are rendered recursively via `render_kotlin_type` so Jackson's
    // `TypeReference<...>` and the public signature stay in lockstep.
    let facade_return_type = |ty: &crate::core::ir::TypeRef| -> String {
        if let crate::core::ir::TypeRef::Named(n) = ty {
            if opaque_type_names.contains(n.as_str()) {
                return n.clone();
            }
            // Non-opaque Named: expose the real Kotlin type.
            return n.clone();
        }
        // Generic container (Vec / Map / Option<Vec|Map>): render recursively.
        if matches!(
            unwrap_optional(ty),
            crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _)
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
    let facade_param_type = |ty: &crate::core::ir::TypeRef| -> String {
        let inner = unwrap_optional(ty);
        if let crate::core::ir::TypeRef::Named(n) = inner {
            if opaque_type_names.contains(n.as_str()) {
                return n.clone();
            }
            // Non-opaque Named: expose the real Kotlin type.
            return n.clone();
        }
        // Binary data (ByteArray/Vec<u8>) → ByteArray in the public API.
        // The wrapper base64-encodes to String before calling the JNI external fun,
        // which then decodes it back to bytes.
        let is_binary = match inner {
            crate::core::ir::TypeRef::Bytes => true,
            crate::core::ir::TypeRef::Vec(iv) => {
                matches!(
                    iv.as_ref(),
                    crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::U8)
                )
            }
            _ => false,
        };
        if is_binary {
            return "ByteArray".to_string();
        }
        if matches!(
            inner,
            crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _)
        ) {
            return render_kotlin_type(inner, &opaque_type_names);
        }
        jni_param_type_str(ty).to_string()
    };

    // Helper: detect if a TypeRef is a Vec of DTOs (needs deserialization).
    // Retained for the legacy DTO-list emission path; broader generic-container
    // detection lives in `is_generic_container` below.
    let is_vec_of_dtos = |ty: &crate::core::ir::TypeRef| -> bool {
        if let crate::core::ir::TypeRef::Vec(inner) = ty {
            if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
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
    let is_generic_container = |ty: &crate::core::ir::TypeRef| -> bool {
        let base = unwrap_optional(ty);
        matches!(
            base,
            crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _)
        )
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
        imports.insert("import com.fasterxml.jackson.annotation.JsonInclude".to_string());
        imports.insert("import com.fasterxml.jackson.databind.DeserializationFeature".to_string());
        imports.insert("import com.fasterxml.jackson.databind.PropertyNamingStrategies".to_string());
        imports.insert("import com.fasterxml.jackson.datatype.jdk8.Jdk8Module".to_string());
        imports.insert("import com.fasterxml.jackson.module.kotlin.KotlinFeature".to_string());
        imports.insert("import com.fasterxml.jackson.module.kotlin.KotlinModule".to_string());
        imports.insert("import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper".to_string());
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

    body.push_str(&template_env::render(
        "module_object_header.jinja",
        minijinja::context! {
            module_name => module_name,
        },
    ));
    if needs_jackson {
        // Rust serde defaults to snake_case wire keys; Kotlin properties are
        // camelCase. Configure the property naming strategy on the module
        // facade's mapper so Jackson translates between them automatically,
        // matching the streaming-method mapper above and the convention used
        // across the JVM/Kotlin backends.
        body.push_str("    /// Jackson module that marshals ByteArray as a JSON array of unsigned bytes,\n");
        body.push_str("    /// matching how Rust serde encodes Vec<u8> on the wire.\n");
        body.push_str("    /// Jackson's default writes ByteArray as a Base64 string, which Rust serde rejects\n");
        body.push_str("    /// with \"invalid type: string, expected a sequence\".\n");
        body.push_str(
            "    private val byteArrayModule = com.fasterxml.jackson.databind.module.SimpleModule().apply {\n",
        );
        body.push_str("        addSerializer(\n");
        body.push_str("            ByteArray::class.java,\n");
        body.push_str("            object : com.fasterxml.jackson.databind.ser.std.StdSerializer<ByteArray>(ByteArray::class.java) {\n");
        body.push_str("                override fun serialize(\n");
        body.push_str("                    value: ByteArray,\n");
        body.push_str("                    gen: com.fasterxml.jackson.core.JsonGenerator,\n");
        body.push_str("                    provider: com.fasterxml.jackson.databind.SerializerProvider,\n");
        body.push_str("                ) {\n");
        body.push_str("                    gen.writeStartArray()\n");
        body.push_str("                    for (b in value) gen.writeNumber(b.toInt() and 0xff)\n");
        body.push_str("                    gen.writeEndArray()\n");
        body.push_str("                }\n");
        body.push_str("            },\n");
        body.push_str("        )\n");
        body.push_str("        addDeserializer(\n");
        body.push_str("            ByteArray::class.java,\n");
        body.push_str("            object : com.fasterxml.jackson.databind.deser.std.StdDeserializer<ByteArray>(ByteArray::class.java) {\n");
        body.push_str("                override fun deserialize(\n");
        body.push_str("                    parser: com.fasterxml.jackson.core.JsonParser,\n");
        body.push_str("                    ctx: com.fasterxml.jackson.databind.DeserializationContext,\n");
        body.push_str("                ): ByteArray {\n");
        body.push_str(
            "                    val node = parser.codec.readTree<com.fasterxml.jackson.databind.JsonNode>(parser)\n",
        );
        body.push_str("                    return when {\n");
        body.push_str(
            "                        node.isArray -> ByteArray(node.size()) { i -> node.get(i).asInt().toByte() }\n",
        );
        body.push_str(
            "                        node.isTextual -> java.util.Base64.getDecoder().decode(node.asText())\n",
        );
        body.push_str("                        else -> ByteArray(0)\n");
        body.push_str("                    }\n");
        body.push_str("                }\n");
        body.push_str("            },\n");
        body.push_str("        )\n");
        body.push_str("    }\n\n");
        body.push_str("    private val mapper = jacksonObjectMapper()\n");
        body.push_str("        .registerModule(com.fasterxml.jackson.datatype.jdk8.Jdk8Module())\n");
        body.push_str("        .registerModule(byteArrayModule)\n");
        body.push_str("        .registerModule(\n");
        body.push_str("            com.fasterxml.jackson.module.kotlin.KotlinModule.Builder()\n");
        body.push_str(
            "                .configure(com.fasterxml.jackson.module.kotlin.KotlinFeature.NullIsSameAsDefault, true)\n",
        );
        body.push_str("                .configure(com.fasterxml.jackson.module.kotlin.KotlinFeature.NullToEmptyCollection, true)\n");
        body.push_str(
            "                .configure(com.fasterxml.jackson.module.kotlin.KotlinFeature.NullToEmptyMap, true)\n",
        );
        body.push_str("                .build(),\n");
        body.push_str("        )\n");
        body.push_str("        .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)\n");
        body.push_str(
            "        .setSerializationInclusion(com.fasterxml.jackson.annotation.JsonInclude.Include.NON_EMPTY)\n",
        );
        body.push_str("        .configure(com.fasterxml.jackson.databind.DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false)\n\n");
    }

    // Pre-compute all method names to detect potential async-naming collisions.
    // If a sync method's suspend wrapper would be named the same as an async method,
    // we'll skip the wrapper to avoid overload conflicts.
    let all_async_method_names: std::collections::HashSet<String> = visible_functions
        .iter()
        .filter(|f| f.name.ends_with("_async"))
        .map(|f| to_lower_camel(&f.name))
        .collect();

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
                            crate::core::ir::TypeRef::Named(n) => n.clone(),
                            _ => unreachable!(),
                        };
                        format!("{name}: {ty_name}? = null")
                    } else if opaque_type_names.contains(match inner {
                        crate::core::ir::TypeRef::Named(n) => n.as_str(),
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
                if let crate::core::ir::TypeRef::Named(n) = inner {
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
                // Binary data (ByteArray/Vec<u8>): Base64 encode to String for JNI.
                // JNI cannot pass ByteArray directly across the FFI boundary without
                // conversion; the FFI layer expects a Base64-encoded string that it
                // decodes. Kotlin's java.util.Base64.Encoder handles the conversion.
                let is_binary = match inner {
                    crate::core::ir::TypeRef::Bytes => true,
                    crate::core::ir::TypeRef::Vec(inner_ty) => {
                        matches!(
                            inner_ty.as_ref(),
                            crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::U8)
                        )
                    }
                    _ => false,
                };
                if is_binary {
                    if p.optional {
                        return format!("{name}?.let {{ java.util.Base64.getEncoder().encodeToString(it) }} ?: \"\"");
                    }
                    return format!("java.util.Base64.getEncoder().encodeToString({name})");
                }
                // Generic container (Vec<_> or HashMap<_,_>): serialize to JSON.
                if matches!(
                    inner,
                    crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _)
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
        let call_args = f
            .params
            .iter()
            .map(|p| to_lower_camel(&p.name))
            .collect::<Vec<_>>()
            .join(", ");
        let params_str = params.join(", ");

        // Determine body expression: deserialize from JSON when the return type
        // is a DTO or Vec<DTO>, wrap in opaque class when it is a handle, pass through
        // otherwise.
        let returns_opaque =
            matches!(&f.return_type, crate::core::ir::TypeRef::Named(n) if opaque_type_names.contains(n.as_str()));

        // Detect if the method name already indicates async (e.g. rerankAsync from
        // Rust's rerank_async). For these, emit only as suspend function to avoid
        // overload conflicts; the Rust function is already async, so no sync wrapper.
        let method_name_already_async = method_name.ends_with("Async");
        // Also check if a sync method's suspend wrapper would collide with an async method.
        // E.g. sync rerank's wrapper would be rerankAsync, which collides with rerank_async.
        let suspend_wrapper_name = format!("{}Async", method_name);
        let suspend_wrapper_would_collide =
            !method_name_already_async && all_async_method_names.contains(&suspend_wrapper_name);

        if returns_dto || returns_generic_container || returns_opaque || needs_jackson {
            // Suppress unused-warning on the legacy DTO-list flag — it remains
            // a useful diagnostic name for downstream readers but the generic
            // container path now subsumes its emission branch.
            let _ = returns_vec_of_dtos;
            // Emit a block body so we can introduce local vars for clarity.
            if returns_dto {
                let return_class = match &f.return_type {
                    crate::core::ir::TypeRef::Named(n) => n.clone(),
                    _ => unreachable!(),
                };
                // Only emit the sync wrapper if the method name doesn't already
                // indicate async. For async-named methods, emit only the suspend variant.
                if !method_name_already_async {
                    body.push_str(&template_env::render(
                        "android_facade_dto_method.jinja",
                        minijinja::context! {
                            method_name => method_name,
                            params => params_str,
                            return_type => return_ty,
                            bridge_call => bridge_call,
                            return_class => return_class,
                        },
                    ));
                }
                // Emit the suspend variant. For async-named functions, it's the only
                // method; for sync-named functions, it wraps the sync version above
                // (unless the wrapper name would collide).
                if !suspend_wrapper_would_collide {
                    emit_kdoc_pub(&mut body, &f.doc, "    ");
                    if method_name_already_async {
                        // For async-named functions, emit suspend with direct JNI call
                        body.push_str("    suspend fun ");
                        body.push_str(&method_name);
                        body.push('(');
                        body.push_str(&params_str);
                        body.push_str("): ");
                        body.push_str(&return_ty);
                        body.push_str(" = withContext(Dispatchers.IO) { ");
                        body.push_str("val resultJson = ");
                        body.push_str(&bridge_call);
                        body.push_str("; mapper.readValue(resultJson, ");
                        body.push_str(&return_class);
                        body.push_str("::class.java) }\n");
                    } else {
                        // For sync-named functions, emit suspend wrapper around sync version
                        body.push_str(&template_env::render(
                            "android_facade_async_method.jinja",
                            minijinja::context! {
                                method_name => method_name,
                                params => params_str,
                                return_type => return_ty,
                                args => call_args,
                            },
                        ));
                    }
                }
            } else if returns_generic_container {
                // Generic container return: Kotlin disallows generic type
                // arguments on `::class.java`, so we route through Jackson's
                // `TypeReference<T>`.  The TypeReference body is the fully
                // rendered Kotlin type (e.g. `List<String>`, `Map<String, Long>`,
                // `List<MyDto>?` — `render_kotlin_type` handles every Vec /
                // Map / Option permutation recursively).
                let type_ref_body = render_kotlin_type(&f.return_type, &opaque_type_names);
                // Only emit the sync wrapper if the method name doesn't already
                // indicate async.
                if !method_name_already_async {
                    body.push_str(&template_env::render(
                        "android_facade_generic_method.jinja",
                        minijinja::context! {
                            method_name => method_name,
                            params => params_str,
                            return_type => return_ty,
                            bridge_call => bridge_call,
                            type_ref_body => type_ref_body,
                        },
                    ));
                }
                // Emit the suspend variant (unless the wrapper name would collide).
                if !suspend_wrapper_would_collide {
                    emit_kdoc_pub(&mut body, &f.doc, "    ");
                    if method_name_already_async {
                        // For async-named functions, emit suspend with direct JNI call
                        body.push_str("    suspend fun ");
                        body.push_str(&method_name);
                        body.push('(');
                        body.push_str(&params_str);
                        body.push_str("): ");
                        body.push_str(&return_ty);
                        body.push_str(" = withContext(Dispatchers.IO) { ");
                        body.push_str("val resultJson = ");
                        body.push_str(&bridge_call);
                        body.push_str("; mapper.readValue(resultJson, object : TypeReference<");
                        body.push_str(&type_ref_body);
                        body.push_str(">() {}) }\n");
                    } else {
                        // For sync-named functions, emit suspend wrapper around sync version
                        body.push_str(&template_env::render(
                            "android_facade_async_method.jinja",
                            minijinja::context! {
                                method_name => method_name,
                                params => params_str,
                                return_type => return_ty,
                                args => call_args,
                            },
                        ));
                    }
                }
            } else if returns_opaque {
                let opaque_class = match &f.return_type {
                    crate::core::ir::TypeRef::Named(n) => n.clone(),
                    _ => unreachable!(),
                };
                body.push_str(&template_env::render(
                    "android_facade_expr_method.jinja",
                    minijinja::context! {
                        method_name => method_name,
                        params => params_str,
                        return_type => return_ty,
                        expression => format!("{opaque_class}({bridge_call})"),
                    },
                ));
            } else {
                body.push_str(&template_env::render(
                    "android_facade_expr_method.jinja",
                    minijinja::context! {
                        method_name => method_name,
                        params => params_str,
                        return_type => return_ty,
                        expression => bridge_call,
                    },
                ));
            }
        } else {
            body.push_str(&template_env::render(
                "android_facade_expr_method.jinja",
                minijinja::context! {
                    method_name => method_name,
                    params => params_str,
                    return_type => return_ty,
                    expression => bridge_call,
                },
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
fn unwrap_optional(ty: &crate::core::ir::TypeRef) -> &crate::core::ir::TypeRef {
    match ty {
        crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    }
}

/// Return a nullable Kotlin type string for an optional facade parameter.
/// Used when `p.optional == true` so callers can pass `null` to skip the
/// param (e.g. `timeoutSecs: Long? = null`).  Nullable is idiomatic Kotlin
/// and avoids sentinel zero-value collisions (`0L`, `""`).
fn kotlin_nullable_type_for_optional(ty: &crate::core::ir::TypeRef) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};
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
fn jni_zero_literal(ty: &crate::core::ir::TypeRef) -> &'static str {
    use crate::core::ir::{PrimitiveType, TypeRef};
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

fn jni_return_type_str(ty: &crate::core::ir::TypeRef) -> &'static str {
    use crate::core::ir::{PrimitiveType, TypeRef};
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
fn render_kotlin_type(ty: &crate::core::ir::TypeRef, opaque_type_names: &std::collections::HashSet<&str>) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};
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
fn jni_param_type_str(ty: &crate::core::ir::TypeRef) -> &'static str {
    use crate::core::ir::{PrimitiveType, TypeRef};
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
        // Binary data (ByteArray/Vec<u8>) → String in external fun signature.
        // The Kotlin wrapper Base64-encodes ByteArray to String before calling
        // the external fun; the FFI Rust side Base64-decodes the String.
        TypeRef::Bytes => "String",
        TypeRef::Vec(_) => "String",
        _ => "String",
    }
}
