fn emit_jni_client_method(
    m: &crate::core::ir::MethodDef,
    class_name: &str,
    bridge_name: &str,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    opaque_type_names: &std::collections::HashSet<&str>,
    config: &ResolvedCrateConfig,
) {
    emit_jni_client_method_docs(m, out);
    let capsule = match jni_capsule_projection(&m.return_type, config) {
        Ok(capsule) => capsule,
        Err(error) => {
            emit_jni_client_line_comment(out, &format!("ALEF ERROR: {error}"));
            return;
        }
    };
    let method_name = to_lower_camel(&m.name);
    let native_name = format!("native{}{}", to_pascal_case(class_name), to_pascal_case(&m.name));
    let async_kw = if m.is_async { "suspend " } else { "" };

    let params_with_types: Vec<String> = m.params.iter().map(|p| format_param_with_imports(p, imports)).collect();

    let wrapper_return_ty = if let Some(capsule) = &capsule {
        capsule.host_type.clone()
    } else if is_binary_return_type(&m.return_type) {
        "ByteArray".to_string()
    } else if is_optional_binary_return_type(&m.return_type) {
        "ByteArray?".to_string()
    } else {
        kotlin_type_with_string_imports(&m.return_type, false, imports)
    };

    out.push_str(&template_env::render(
        "jni_client_method_header.jinja",
        crate::alef_context! {
            async_kw => async_kw,
            method_name => method_name,
            params => params_with_types.join(", "),
            return_type => wrapper_return_ty,
        },
    ));

    let bridge_call = build_bridge_call(m, bridge_name, &native_name);
    emit_jni_client_method_body(m, out, imports, opaque_type_names, &bridge_call, capsule);
    out.push_str("    }\n\n");
}

fn emit_jni_client_method_body(
    method: &crate::core::ir::MethodDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    opaque_type_names: &std::collections::HashSet<&str>,
    bridge_call: &str,
    capsule: Option<JniCapsuleProjection>,
) {
    if let Some(capsule) = capsule {
        out.push_str(&template_env::render(
            "jni_capsule_body.jinja",
            crate::alef_context! {
                is_async => method.is_async,
                bridge_call => bridge_call,
                construct_expr => capsule.construct_expr,
                optional => capsule.optional,
            },
        ));
    } else {
        emit_method_body(method, out, bridge_call, imports, opaque_type_names);
    }
}

struct JniCapsuleProjection {
    host_type: String,
    construct_expr: String,
    optional: bool,
}

fn emit_jni_client_method_docs(m: &crate::core::ir::MethodDef, out: &mut String) {
    for line in m.doc.lines() {
        emit_jni_client_line_comment(out, line);
    }
}

fn emit_jni_client_line_comment(out: &mut String, line: &str) {
    out.push_str(&template_env::render(
        "line_comment.jinja",
        crate::alef_context! {
            indent => "    ",
            line => line,
        },
    ));
}

fn jni_capsule_projection(
    return_type: &TypeRef,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Option<JniCapsuleProjection>> {
    let (type_name, optional) = match return_type {
        TypeRef::Named(type_name) => (type_name, false),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(type_name) => (type_name, true),
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    let Some(capsule) = config
        .kotlin_android
        .as_ref()
        .and_then(|android| android.capsule_types.get(type_name))
    else {
        return Ok(None);
    };
    let host_type = capsule.required_host_type(type_name, "kotlin_android")?;
    Ok(Some(JniCapsuleProjection {
        host_type: if optional {
            format!("{}?", host_type.trim_end_matches('?'))
        } else {
            host_type.to_string()
        },
        construct_expr: capsule.construct_required("capsulePtr", type_name, "kotlin_android")?,
        optional,
    }))
}

/// Build the expression that calls the bridge, including any JSON serialisation.
///
/// Returns a string that produces the bridge's raw return value (String, ByteArray, Unit, etc.).
fn build_bridge_call(m: &crate::core::ir::MethodDef, bridge_name: &str, native_name: &str) -> String {
    if m.params.is_empty() {
        return format!("withHandle {{ handle -> {bridge_name}.{native_name}(handle) }}");
    }
    if m.params.len() == 1 && is_binary_param_type(&m.params[0].ty) {
        let p = &m.params[0];
        let param_name = to_lower_camel(&p.name);
        let arg = if p.optional {
            format!("{param_name} ?: ByteArray(0)")
        } else {
            param_name
        };
        return format!("withHandle {{ handle -> {bridge_name}.{native_name}(handle, {arg}) }}");
    }
    let request_json_expr = if m.params.len() == 1 {
        let p = &m.params[0];
        let param_name = to_lower_camel(&p.name);
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
    format!("withHandle {{ handle -> {bridge_name}.{native_name}(handle, {request_json_expr}) }}")
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
                crate::alef_context! {
                    is_async => m.is_async,
                    bridge_call => bridge_call,
                },
            ));
        }
        TypeRef::Optional(inner) if is_opaque_return => {
            if let TypeRef::Named(_) = inner.as_ref() {
                let wrapper = kotlin_type_with_string_imports(&m.return_type, false, imports);
                let base_wrapper = wrapper.trim_end_matches('?');
                out.push_str(&template_env::render(
                    "jni_opaque_optional_body.jinja",
                    crate::alef_context! {
                        is_async => m.is_async,
                        bridge_call => bridge_call,
                        wrapper_type => base_wrapper,
                    },
                ));
            }
        }
        _ if is_opaque_return => {
            let kotlin_ty = kotlin_type_with_string_imports(&m.return_type, false, imports);
            out.push_str(&template_env::render(
                "jni_opaque_body.jinja",
                crate::alef_context! {
                    is_async => m.is_async,
                    bridge_call => bridge_call,
                    wrapper_type => kotlin_ty,
                },
            ));
        }
        _ if needs_deserialize => {
            let kotlin_ty = return_kotlin_type.unwrap();
            let base_ty = kotlin_ty.trim_end_matches('?');
            let use_type_reference = base_ty.contains('<');
            let deserialize_call = if use_type_reference {
                imports.insert("import com.fasterxml.jackson.core.type.TypeReference".to_string());
                format!("MAPPER.readValue(responseJson, object : TypeReference<{base_ty}>() {{}})")
            } else {
                format!("MAPPER.readValue(responseJson, {base_ty}::class.java)")
            };
            // The shim returns a null `jstring` for `None`, and Jackson's `readValue` rejects a
            // null input rather than producing one, so the nullable arm has to test before it
            // deserialises. ~keep
            let body_template = if matches!(m.return_type, TypeRef::Optional(_)) {
                "jni_optional_deserialize_body.jinja"
            } else {
                "jni_deserialize_body.jinja"
            };
            out.push_str(&template_env::render(
                body_template,
                crate::alef_context! {
                    is_async => m.is_async,
                    bridge_call => bridge_call,
                    deserialize_call => deserialize_call,
                },
            ));
        }
        _ => {
            out.push_str(&template_env::render(
                "jni_passthrough_body.jinja",
                crate::alef_context! {
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

    out.push_str(&template_env::render(
        "jni_streaming_client_method.jinja",
        crate::alef_context! {
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
