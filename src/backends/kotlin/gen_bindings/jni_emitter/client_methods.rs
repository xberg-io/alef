fn emit_jni_client_method(
    m: &crate::core::ir::MethodDef,
    class_name: &str,
    bridge_name: &str,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    opaque_type_names: &std::collections::HashSet<&str>,
) {
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

    out.push_str(&template_env::render(
        "jni_client_method_header.jinja",
        minijinja::context! {
            async_kw => async_kw,
            method_name => method_name,
            params => params_with_types.join(", "),
            return_type => wrapper_return_ty,
        },
    ));

    // Build the bridge call expression, with JSON marshalling where needed.
    let bridge_call = build_bridge_call(m, bridge_name, &native_name);

    // Emit the method body with optional `withContext` wrapping.
    emit_method_body(m, out, &bridge_call, imports, opaque_type_names);

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
    opaque_type_names: &std::collections::HashSet<&str>,
) {
    let needs_deserialize = needs_json_deserialize_for_method(&m.return_type, opaque_type_names);
    let return_kotlin_type = if needs_deserialize {
        Some(kotlin_type_with_string_imports(&m.return_type, false, imports))
    } else {
        None
    };

    // Check if this is an opaque type return (including optional)
    let is_opaque_return = match &m.return_type {
        TypeRef::Named(n) => opaque_type_names.contains(n.as_str()),
        TypeRef::Optional(inner) => {
            if let TypeRef::Named(n) = inner.as_ref() {
                opaque_type_names.contains(n.as_str())
            } else {
                false
            }
        }
        _ => false,
    };

    match &m.return_type {
        TypeRef::Unit => {
            out.push_str(&template_env::render(
                "jni_unit_body.jinja",
                minijinja::context! {
                    is_async => m.is_async,
                    bridge_call => bridge_call,
                },
            ));
        }
        TypeRef::Optional(inner) if is_opaque_return => {
            // Bridge returns a primitive Long handle (0L = None); wrap in opaque type constructor
            if let TypeRef::Named(_) = inner.as_ref() {
                let wrapper = kotlin_type_with_string_imports(&m.return_type, false, imports);
                let base_wrapper = wrapper.trim_end_matches('?');
                out.push_str(&template_env::render(
                    "jni_opaque_optional_body.jinja",
                    minijinja::context! {
                        is_async => m.is_async,
                        bridge_call => bridge_call,
                        wrapper_type => base_wrapper,
                    },
                ));
            }
        }
        _ if is_opaque_return => {
            // Bridge returns Long (raw handle); wrap in opaque type constructor
            let kotlin_ty = kotlin_type_with_string_imports(&m.return_type, false, imports);
            out.push_str(&template_env::render(
                "jni_opaque_body.jinja",
                minijinja::context! {
                    is_async => m.is_async,
                    bridge_call => bridge_call,
                    wrapper_type => kotlin_ty,
                },
            ));
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
            out.push_str(&template_env::render(
                "jni_deserialize_body.jinja",
                minijinja::context! {
                    is_async => m.is_async,
                    bridge_call => bridge_call,
                    deserialize_call => deserialize_call,
                },
            ));
        }
        _ => {
            // Primitive, Boolean, ByteArray, String — pass through.
            out.push_str(&template_env::render(
                "jni_passthrough_body.jinja",
                minijinja::context! {
                    is_async => m.is_async,
                    bridge_call => bridge_call,
                },
            ));
        }
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
    out.push_str(&template_env::render(
        "jni_streaming_client_method.jinja",
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
