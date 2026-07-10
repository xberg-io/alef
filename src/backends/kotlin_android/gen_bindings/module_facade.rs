use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::backends::kotlin::{emit_kdoc_pub, to_pascal_case};
use crate::backends::kotlin_android::template_env;
use crate::codegen::naming::kotlin_android_wrapper_object_name;
use crate::core::backend::GeneratedFile;
use crate::core::config::{HostCapsuleTypeConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};

use super::assemble_kt_content;

mod helpers;
use self::helpers::{jni_zero_literal, kotlin_nullable_type_for_optional, unwrap_optional};

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
    let bridge_name = crate::core::jni::bridge_class_name(&config.name);

    let opaque_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

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
        .filter(|f| {
            !exclude_functions.contains(f.name.as_str())
                && !trait_bridge_manages_android_function(f.name.as_str(), config)
        })
        .collect();

    let handle_only_types: std::collections::BTreeMap<&str, &crate::core::ir::TypeDef> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && !client_type_names.contains(t.name.as_str()))
        .map(|t| (t.name.as_str(), t))
        .collect();

    for (type_name, type_def) in &handle_only_types {
        let class_name = *type_name;
        let free_name = format!("nativeFree{}", to_pascal_case(class_name));
        let mut body = String::new();
        let mut imports: BTreeSet<String> = BTreeSet::new();

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

        if !streaming_adapters_for_type.is_empty() {
            imports.insert("import com.fasterxml.jackson.databind.ObjectMapper".to_string());
            imports.insert("import com.fasterxml.jackson.datatype.jdk8.Jdk8Module".to_string());
            imports.insert("import com.fasterxml.jackson.databind.PropertyNamingStrategies".to_string());
            imports.insert("import kotlinx.coroutines.Dispatchers".to_string());
            imports.insert("import kotlinx.coroutines.flow.Flow".to_string());
            imports.insert("import kotlinx.coroutines.flow.callbackFlow".to_string());
            imports.insert("import kotlinx.coroutines.withContext".to_string());
            imports.insert("import kotlinx.coroutines.channels.awaitClose".to_string());

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

    let is_dto_named = |ty: &crate::core::ir::TypeRef| -> bool {
        match ty {
            crate::core::ir::TypeRef::Named(n) => !opaque_type_names.contains(n.as_str()),
            _ => false,
        }
    };

    let facade_return_type = |ty: &crate::core::ir::TypeRef| -> String {
        if let crate::core::ir::TypeRef::Named(n) = ty {
            if opaque_type_names.contains(n.as_str()) {
                return n.clone();
            }
            return n.clone();
        }
        if matches!(
            unwrap_optional(ty),
            crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _)
        ) {
            return render_kotlin_type(ty, &opaque_type_names);
        }
        jni_return_type_str(ty).to_string()
    };

    let facade_param_type = |ty: &crate::core::ir::TypeRef| -> String {
        let inner = unwrap_optional(ty);
        if let crate::core::ir::TypeRef::Named(n) = inner {
            if opaque_type_names.contains(n.as_str()) {
                return n.clone();
            }
            return n.clone();
        }
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

    let is_vec_of_dtos = |ty: &crate::core::ir::TypeRef| -> bool {
        if let crate::core::ir::TypeRef::Vec(inner) = ty {
            if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
                return !opaque_type_names.contains(n.as_str());
            }
        }
        false
    };

    let is_generic_container = |ty: &crate::core::ir::TypeRef| -> bool {
        let base = unwrap_optional(ty);
        matches!(
            base,
            crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _)
        )
    };

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
            "        .setSerializationInclusion(com.fasterxml.jackson.annotation.JsonInclude.Include.NON_NULL)\n",
        );
        body.push_str("        .configure(com.fasterxml.jackson.databind.DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false)\n\n");
    }

    let all_async_method_names: std::collections::HashSet<String> = visible_functions
        .iter()
        .filter(|f| f.name.ends_with("_async"))
        .map(|f| to_lower_camel(&f.name))
        .collect();

    let kotlin_android_capsule_types: HashMap<String, HostCapsuleTypeConfig> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.capsule_types.clone())
        .unwrap_or_default();

    for f in &visible_functions {
        if let Some(capsule_cfg) = get_capsule_config(f, &kotlin_android_capsule_types) {
            emit_kdoc_pub(&mut body, &f.doc, "    ");
            emit_capsule_function_wrapper(&mut body, f, &bridge_name, capsule_cfg);
            body.push('\n');
            continue;
        }

        emit_kdoc_pub(&mut body, &f.doc, "    ");
        let method_name = to_lower_camel(&f.name);
        let native_name = format!("native{}", to_pascal_case(&f.name));
        let return_ty = facade_return_type(&f.return_type);
        let returns_dto = is_dto_named(&f.return_type);
        let returns_vec_of_dtos = is_vec_of_dtos(&f.return_type);
        let returns_generic_container = is_generic_container(&f.return_type);

        let params: Vec<String> = f
            .params
            .iter()
            .map(|p| {
                let name = to_lower_camel(&p.name);
                let inner = unwrap_optional(&p.ty);
                let is_dto = is_dto_named(inner);
                if p.optional {
                    if is_dto {
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

        let bridge_args: Vec<String> = f
            .params
            .iter()
            .map(|p| {
                let name = to_lower_camel(&p.name);
                let inner = unwrap_optional(&p.ty);
                if let crate::core::ir::TypeRef::Named(n) = inner {
                    if opaque_type_names.contains(n.as_str()) {
                        return format!("{name}.handle");
                    }
                    if p.optional {
                        return format!("{name}?.let {{ mapper.writeValueAsString(it) }} ?: \"\"");
                    }
                    return format!("mapper.writeValueAsString({name})");
                }
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
                if matches!(
                    inner,
                    crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Map(_, _)
                ) {
                    if p.optional {
                        return format!("{name}?.let {{ mapper.writeValueAsString(it) }} ?: \"\"");
                    }
                    return format!("mapper.writeValueAsString({name})");
                }
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

        let returns_opaque =
            matches!(&f.return_type, crate::core::ir::TypeRef::Named(n) if opaque_type_names.contains(n.as_str()));
        let generate_config = config
            .generate_overrides
            .get("kotlin_android")
            .unwrap_or(&config.generate);

        let method_name_already_async = method_name.ends_with("Async");
        let suspend_wrapper_name = format!("{}Async", method_name);
        let suspend_wrapper_would_collide =
            !method_name_already_async && all_async_method_names.contains(&suspend_wrapper_name);
        let emit_suspend_wrapper =
            method_name_already_async || (generate_config.async_wrappers && !suspend_wrapper_would_collide);

        if returns_dto || returns_generic_container || returns_opaque || needs_jackson {
            let _ = returns_vec_of_dtos;
            if returns_dto {
                let return_class = match &f.return_type {
                    crate::core::ir::TypeRef::Named(n) => n.clone(),
                    _ => unreachable!(),
                };
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
                if emit_suspend_wrapper {
                    emit_kdoc_pub(&mut body, &f.doc, "    ");
                    if method_name_already_async {
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
                let type_ref_body = render_kotlin_type(&f.return_type, &opaque_type_names);
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
                if emit_suspend_wrapper {
                    emit_kdoc_pub(&mut body, &f.doc, "    ");
                    if method_name_already_async {
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

fn trait_bridge_manages_android_function(func_name: &str, config: &ResolvedCrateConfig) -> bool {
    config.trait_bridges.iter().any(|bridge| {
        !bridge.exclude_languages.iter().any(|lang| lang == "kotlin_android")
            && (bridge.register_fn.as_deref() == Some(func_name)
                || bridge.unregister_fn.as_deref() == Some(func_name)
                || bridge.clear_fn.as_deref() == Some(func_name))
    })
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
        TypeRef::Bytes => "String",
        TypeRef::Vec(_) => "String",
        _ => "String",
    }
}

/// Returns a reference to the host capsule config if the function returns a capsule type,
/// otherwise returns None.
fn get_capsule_config<'a>(
    func: &crate::core::ir::FunctionDef,
    capsule_types: &'a HashMap<String, HostCapsuleTypeConfig>,
) -> Option<&'a HostCapsuleTypeConfig> {
    if let TypeRef::Named(name) = &func.return_type {
        capsule_types.get(name.as_str())
    } else {
        None
    }
}

/// Emit a Kotlin function wrapper that constructs the host-native Language (capsule type)
/// from the raw JNI pointer returned by the bridge's external fun.
///
/// The wrapper:
/// 1. Calls the JNI external fun (which returns a Long native pointer)
/// 2. Guards against null (grammar not found)
/// 3. Constructs the host Language using the configured construct_expr
fn emit_capsule_function_wrapper(
    body: &mut String,
    func: &crate::core::ir::FunctionDef,
    bridge_name: &str,
    capsule_cfg: &HostCapsuleTypeConfig,
) {
    use crate::backends::kotlin::to_lower_camel;

    let method_name = to_lower_camel(&func.name);
    let native_name = format!("native{}", to_pascal_case(&func.name));
    let host_type = match capsule_cfg.required_host_type("Language", "kotlin_android") {
        Ok(t) => t.to_string(),
        Err(e) => {
            body.push_str(&format!("    // ALEF ERROR: {e}\n"));
            return;
        }
    };

    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let name = to_lower_camel(&p.name);
            let ty = jni_param_type_str(&p.ty);
            format!("{name}: {ty}")
        })
        .collect();
    let params_str = params.join(", ");

    let bridge_args: Vec<String> = func.params.iter().map(|p| to_lower_camel(&p.name)).collect();
    let bridge_call = format!("{bridge_name}.{native_name}({})", bridge_args.join(", "));

    let construct_expr = match capsule_cfg.construct_required("cLangPtr", "Language", "kotlin_android") {
        Ok(c) => c,
        Err(e) => {
            body.push_str(&format!("    // ALEF ERROR: {e}\n"));
            return;
        }
    };

    let (exception_type, error_message) = if func.error_type.is_some() {
        (format!("{}Exception", bridge_name), "\"Function failed\"")
    } else {
        (
            "IllegalArgumentException".to_string(),
            "\"Unexpected null return from native function\"",
        )
    };

    body.push_str("    fun ");
    body.push_str(&method_name);
    body.push('(');
    body.push_str(&params_str);
    body.push_str("): ");
    body.push_str(&host_type);
    body.push_str(" {\n");
    body.push_str("        val cLangPtr = ");
    body.push_str(&bridge_call);
    body.push('\n');
    body.push_str("        if (cLangPtr == 0L) {\n");
    body.push_str("            throw ");
    body.push_str(&exception_type);
    body.push('(');
    body.push_str(error_message);
    body.push_str(")\n");
    body.push_str("        }\n");
    body.push_str("        return ");
    body.push_str(&construct_expr);
    body.push('\n');
    body.push_str("    }\n");
}

#[cfg(test)]
mod tests;
