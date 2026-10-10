//! Opaque-type struct and method generators, split out of types.rs.

use super::*;

/// Generate an opaque wasm-bindgen struct with inner Arc or Arc<Mutex<>>.
pub(in crate::backends::wasm::gen_bindings) fn gen_opaque_struct(
    typ: &TypeDef,
    core_import: &str,
    prefix: &str,
    source_crate_remaps: &[(&str, &str)],
) -> String {
    let js_name = format!("{prefix}{}", typ.name);
    // ~keep See `methods::gen_method`: the override crate is the only one this binding depends
    // on, so the source-crate prefix the IR carries has to be rewritten before it is emitted.
    let core_path = crate::codegen::conversions::core_type_path_remapped(typ, core_import, source_crate_remaps);

    let has_mut_methods = typ
        .methods
        .iter()
        .any(|m| matches!(m.receiver.as_ref(), Some(ReceiverKind::RefMut)));

    let mut out = String::with_capacity(256);
    out.push_str(&emit_rustdoc(&typ.doc));
    out.push_str(&crate::backends::wasm::template_env::render(
        "gen_opaque_struct",
        crate::alef_context! {
            struct_name => js_name,
            unprefixed_name => typ.name,
            core_path => core_path,
            has_mut_methods => has_mut_methods,
        },
    ));
    out
}

/// Generate wasm-bindgen methods for an opaque struct.
#[allow(clippy::too_many_arguments)]
pub(in crate::backends::wasm::gen_bindings) fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &WasmMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    source_crate_remaps: &[(&str, &str)],
    type_paths: &AHashMap<String, String>,
    prefix: &str,
    adapter_bodies: &crate::adapters::AdapterBodies,
    mutex_types: &AHashSet<String>,
    streaming_item_types: &ahash::AHashMap<String, String>,
    wasm_skipped_methods: &AHashSet<String>,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    let js_name = format!("{prefix}{}", typ.name);
    let mut impl_builder = ImplBuilder::new(&js_name);

    // under #[cfg(target_arch = "wasm32")], so guard its impl block identically
    let bridge_config = trait_bridges
        .iter()
        .find(|bridge| bridge.type_alias.as_deref() == Some(typ.name.as_str()));
    let is_bridge_type_alias = bridge_config.is_some();
    if is_bridge_type_alias {
        impl_builder.add_attr("cfg(target_arch = \"wasm32\")");
    }
    impl_builder.add_attr("wasm_bindgen");

    impl_builder.add_method(&gen_transfer_copy_method(typ, prefix));

    if is_bridge_type_alias && typ.methods.is_empty() {
        let bridge_config = bridge_config.expect("checked bridge alias");
        let module_name = crate::backends::wasm::trait_bridge::wasm_bridge_module_name(bridge_config);
        let bridge_struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Wasm", bridge_config);
        let constructor = crate::backends::wasm::template_env::render(
            "gen_visitor_handle_constructor",
            crate::alef_context! {
                struct_name => js_name,
                module_name => module_name,
                bridge_struct_name => bridge_struct_name,
            },
        );
        impl_builder.add_method(&constructor);
    }

    for method in &typ.methods {
        if method.name == "default" {
            continue;
        }
        let method_key = format!("{}.{}", typ.name, method.name);
        if wasm_skipped_methods.contains(&method_key) {
            continue;
        }
        if method.is_static {
            impl_builder.add_method(&gen_opaque_static_method(
                method,
                OpaqueStaticMethodContext {
                    mapper,
                    type_name: &typ.name,
                    opaque_types,
                    core_import,
                    source_crate_remaps,
                    qualified_type_path: &crate::codegen::conversions::core_type_path_remapped(
                        typ,
                        core_import,
                        source_crate_remaps,
                    ),
                    type_paths,
                    prefix,
                    mutex_types,
                },
            ));
        } else {
            impl_builder.add_method(&gen_opaque_method(
                method,
                mapper,
                &typ.name,
                opaque_types,
                core_import,
                source_crate_remaps,
                type_paths,
                prefix,
                adapter_bodies,
                mutex_types,
                streaming_item_types,
            ));
        }
    }

    impl_builder.build()
}

/// Generate a method for an opaque wasm-bindgen struct that delegates to self.inner.
#[allow(clippy::too_many_arguments)]
fn gen_opaque_method(
    method: &MethodDef,
    mapper: &WasmMapper,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    source_crate_remaps: &[(&str, &str)],
    type_paths: &AHashMap<String, String>,
    prefix: &str,
    adapter_bodies: &crate::adapters::AdapterBodies,
    mutex_types: &AHashSet<String>,
    streaming_item_types: &ahash::AHashMap<String, String>,
) -> String {
    let type_is_mutex_wrapped = mutex_types.contains(type_name);
    let is_ref_mut = matches!(method.receiver.as_ref(), Some(ReceiverKind::RefMut));

    let can_delegate_base = shared::can_auto_delegate(method, opaque_types);
    let can_delegate = if is_ref_mut && type_is_mutex_wrapped && method.trait_source.is_none() {
        !method.sanitized
            && method
                .params
                .iter()
                .all(|p| !p.sanitized && shared::is_delegatable_param(&p.ty, opaque_types))
            && shared::is_opaque_delegatable_type(&method.return_type)
    } else {
        can_delegate_base
    };
    let adapter_key = format!("{type_name}.{}", method.name);
    let has_adapter = adapter_bodies.contains_key(&adapter_key);

    let params_unused = !can_delegate && !has_adapter && !method.is_async;
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = wasm_mapped_param_type(p, mapper);
            let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
            format_param_unused(&p.name, &mapped_ty, params_unused)
        })
        .collect();

    let adapter_key_for_stream = format!("{}.{}", type_name, method.name);
    let stream_item = streaming_item_types.get(&adapter_key_for_stream);
    let return_type = if stream_item.is_some() {
        format!("{}Iterator", method.name.to_pascal_case())
    } else {
        wasm_mapped_return_type(&method.return_type, &method.return_newtype_wrapper, mapper)
    };
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let async_kw = if method.is_async { "async " } else { "" };

    let needs_clone = matches!(method.receiver, Some(ReceiverKind::Owned));

    let body = if can_delegate {
        let let_bindings =
            wasm_newtype_param_bindings(&method.params, mapper, core_import, source_crate_remaps, type_paths);
        let call_args = wasm_call_args(&method.params, opaque_types, false);
        let core_call = if is_ref_mut {
            format!("self.inner.lock().unwrap().{}({})", method.name, call_args)
        } else if needs_clone {
            if type_is_mutex_wrapped {
                format!("self.inner.lock().unwrap().clone().{}({})", method.name, call_args)
            } else {
                format!("(*self.inner).clone().{}({})", method.name, call_args)
            }
        } else if type_is_mutex_wrapped {
            format!("self.inner.lock().unwrap().{}({})", method.name, call_args)
        } else {
            format!("self.inner.{}({})", method.name, call_args)
        };
        if method.is_async {
            let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
            let result_wrap = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                wasm_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                )
            });
            if method.error_type.is_some() {
                format!(
                    "{let_bindings}let result = {core_call}.await\n        \
                     .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
                     Ok({result_wrap})"
                )
            } else {
                format!("{let_bindings}let result = {core_call}.await;\n    {result_wrap}")
            }
        } else if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{let_bindings}{core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok(())")
            } else {
                let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
                let wrap = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                    wasm_wrap_return(
                        &result,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        true,
                        method.returns_ref,
                        method.returns_cow,
                        prefix,
                        mutex_types,
                    )
                });
                format!(
                    "{let_bindings}let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
                )
            }
        } else {
            let result = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
            let wrapped = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                wasm_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                )
            });
            format!("{let_bindings}{wrapped}")
        }
    } else if let Some(body) = adapter_bodies.get(&adapter_key) {
        body.clone()
    } else {
        gen_wasm_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };

    let return_annotation = if has_adapter
        && adapter_bodies
            .get(&adapter_key)
            .is_some_and(|b| b.contains("js_sys::Array") || b.contains("serde_wasm_bindgen::to_value"))
    {
        "Result<JsValue, JsValue>".to_string()
    } else {
        return_annotation
    };

    let mut attrs = emit_rustdoc(&method.doc);
    if method.params.len() + 1 > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    if method.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    if generators::is_trait_method_name(&method.name) {
        attrs.push_str("#[allow(clippy::should_implement_trait)]\n");
    }
    format!(
        "{attrs}#[wasm_bindgen{js_name_attr}]\npub {async_kw}fn {}(&self, {}) -> {} {{\n    \
         {body}\n}}",
        method.name,
        params.join(", "),
        return_annotation
    )
}

struct OpaqueStaticMethodContext<'a> {
    mapper: &'a WasmMapper,
    type_name: &'a str,
    opaque_types: &'a AHashSet<String>,
    core_import: &'a str,
    source_crate_remaps: &'a [(&'a str, &'a str)],
    qualified_type_path: &'a str,
    type_paths: &'a AHashMap<String, String>,
    prefix: &'a str,
    mutex_types: &'a AHashSet<String>,
}

/// Generate a static method for an opaque wasm-bindgen struct.
/// Static methods call CoreType::method() instead of self.inner.method().
fn gen_opaque_static_method(method: &MethodDef, context: OpaqueStaticMethodContext<'_>) -> String {
    let OpaqueStaticMethodContext {
        mapper,
        type_name,
        opaque_types,
        core_import,
        source_crate_remaps,
        qualified_type_path,
        type_paths,
        prefix,
        mutex_types,
    } = context;
    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = wasm_mapped_param_type(p, mapper);
            let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
            format_param_unused(&p.name, &mapped_ty, !can_delegate)
        })
        .collect();

    let return_type = wasm_mapped_return_type(&method.return_type, &method.return_newtype_wrapper, mapper);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let body = if can_delegate {
        let let_bindings =
            wasm_newtype_param_bindings(&method.params, mapper, core_import, source_crate_remaps, type_paths);
        let call_args = wasm_call_args(&method.params, opaque_types, false);
        let core_call = format!("{qualified_type_path}::{}({call_args})", method.name);
        if method.error_type.is_some() {
            let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
            let wrap = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                wasm_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                )
            });
            let await_suffix = if method.is_async { ".await" } else { "" };
            format!(
                "{let_bindings}let result = {core_call}{await_suffix}.map_err(|e| \
                 JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
            )
        } else if method.is_async {
            let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
            let wrapped = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                wasm_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                )
            });
            format!("{let_bindings}let result = {core_call}.await;\n    {wrapped}")
        } else {
            let result = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
            let wrapped = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                wasm_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                )
            });
            format!("{let_bindings}{wrapped}")
        }
    } else {
        gen_wasm_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };

    let mut attrs = emit_rustdoc(&method.doc);
    if method.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    if method.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    if generators::is_trait_method_name(&method.name) {
        attrs.push_str("#[allow(clippy::should_implement_trait)]\n");
    }
    format!(
        "{attrs}#[wasm_bindgen{js_name_attr}]\npub {async_kw}fn {method_name}({params}) -> {return_annotation} {{\n    \
         {body}\n}}",
        async_kw = if method.is_async { "async " } else { "" },
        method_name = method.name,
        params = params.join(", "),
    )
}
