//! NAPI opaque-type method wrappers, split out of types.rs.

use super::*;

/// Generate NAPI methods for an opaque struct (delegates to self.inner).
///
/// NOTE: NAPI-RS (TypeScript/Node.js) intentionally does NOT emit fluent builder classes
/// for DTO types (non-opaque structs with #[napi(object)]). TypeScript idiomatically uses
/// object literals for construction: `convert(html, { headingStyle: 'atx', wrap: true })`
/// rather than fluent builders like `new ConversionOptionsBuilder().headingStyle(...).build()`.
/// This contrasts with the Java backend, which nests builders inside records for idiomatic
/// Java construction patterns. DTO methods (withers returning Self) are exposed as free functions
/// accepting the instance as the first parameter (see gen_dto_method_fns).
#[allow(clippy::too_many_arguments)]
pub(in crate::backends::napi::gen_bindings) fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &NapiMapper,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    adapter_bodies: &crate::adapters::AdapterBodies,
    streaming_item_types: &ahash::AHashMap<String, String>,
    capsule_type_names: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::NodeCapsuleTypeConfig>,
) -> String {
    let mut impl_builder = ImplBuilder::new(&format!("{prefix}{}", typ.name));
    impl_builder.add_attr("napi");

    let (instance, statics) = partition_methods(&typ.methods);

    for method in &instance {
        if opaque_instance_method_is_dropped(method, &typ.name, adapter_bodies, capsule_type_names, opaque_types) {
            continue;
        }
        impl_builder.add_method(&gen_opaque_instance_method(
            method,
            mapper,
            typ,
            cfg,
            opaque_types,
            prefix,
            adapter_bodies,
            streaming_item_types,
            mutex_types,
            capsule_types,
        ));
    }
    for method in &statics {
        if opaque_static_method_is_dropped(method, &typ.name, adapter_bodies) {
            continue;
        }
        impl_builder.add_method(&gen_static_method(
            method,
            mapper,
            typ,
            cfg,
            opaque_types,
            prefix,
            mutex_types,
        ));
    }

    impl_builder.build()
}

/// True when `gen_opaque_struct_methods` drops an opaque instance method entirely — no
/// `#[napi]` wrapper is generated for it, for any of three reasons: `sanitized` with no adapter
/// override; a capsule-typed return (an external capsule type can't be constructed from a plain
/// `#[napi]` return); or an opaque-by-value param with no adapter override (opaque types only
/// implement `FromNapiValue` by reference, so a plain `#[napi]` method can't take one by value).
///
/// `gen_dts` (`errors.rs`) calls this same function to decide whether to declare the method in
/// `index.d.ts`, so the `.d.ts` can never promise an instance method the runtime binding drops.
/// ~keep
pub(in crate::backends::napi::gen_bindings) fn opaque_instance_method_is_dropped(
    method: &MethodDef,
    type_name: &str,
    adapter_bodies: &crate::adapters::AdapterBodies,
    capsule_type_names: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
) -> bool {
    let adapter_key = format!("{type_name}.{}", method.name);
    if method.sanitized && !adapter_bodies.contains_key(&adapter_key) {
        return true;
    }
    let returns_capsule = match &method.return_type {
        TypeRef::Named(name) => capsule_type_names.contains(name),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => capsule_type_names.contains(name),
            _ => false,
        },
        _ => false,
    };
    if returns_capsule {
        return true;
    }
    // FromNapiValue and cannot appear as plain `#[napi]` method params. These methods (e.g.
    let has_opaque_by_value_param = method.params.iter().any(|p| {
        let inner_ty = match &p.ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            other => other,
        };
        matches!(inner_ty, TypeRef::Named(name) if opaque_types.contains(name) && !p.is_ref)
    });
    has_opaque_by_value_param && !adapter_bodies.contains_key(&adapter_key)
}

/// True when `gen_opaque_struct_methods` drops an opaque static method entirely — `sanitized`
/// with no adapter override is the only reason (unlike instance methods, `gen_static_method`
/// always emits a real `#[napi]` wrapper, falling back to an unimplemented body rather than
/// dropping the method, when it can't auto-delegate).
///
/// `gen_dts` calls this same function so `index.d.ts` never promises a static method the
/// runtime binding drops. ~keep
pub(in crate::backends::napi::gen_bindings) fn opaque_static_method_is_dropped(
    method: &MethodDef,
    type_name: &str,
    adapter_bodies: &crate::adapters::AdapterBodies,
) -> bool {
    let adapter_key = format!("{type_name}.{}", method.name);
    method.sanitized && !adapter_bodies.contains_key(&adapter_key)
}

/// Generate an opaque instance method that delegates to self.inner.
#[allow(clippy::too_many_arguments)]
pub(in crate::backends::napi::gen_bindings) fn gen_opaque_instance_method(
    method: &MethodDef,
    mapper: &NapiMapper,
    typ: &TypeDef,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    adapter_bodies: &crate::adapters::AdapterBodies,
    streaming_item_types: &ahash::AHashMap<String, String>,
    mutex_types: &AHashSet<String>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::NodeCapsuleTypeConfig>,
) -> String {
    let has_promoted_required = !promoted_required(&method.params).is_empty();
    let params = function_params(&method.params, &|ty| {
        if let crate::core::ir::TypeRef::Named(name) = ty
            && let Some(capsule_cfg) = capsule_types.get(name)
        {
            return format!(
                "{}::{}",
                capsule_cfg.from_module.replace('-', "_"),
                capsule_cfg.type_name
            );
        }
        mapper.map_type(ty)
    });
    let adapter_key_for_stream = format!("{}.{}", typ.name, method.name);
    let stream_item = streaming_item_types.get(&adapter_key_for_stream);
    let return_type = if stream_item.is_some() {
        format!("{}Iterator", method.name.to_pascal_case())
    } else {
        mapper.map_type(&method.return_type)
    };
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some() || has_promoted_required);

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let async_kw = if method.is_async { "async " } else { "" };

    let type_name = &typ.name;
    let is_owned_receiver = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::Owned));
    let is_ref_mut_receiver = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));

    let has_mut_methods = typ
        .methods
        .iter()
        .any(|m| matches!(m.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut)));

    let self_ref_return =
        method.returns_ref && matches!(&method.return_type, crate::core::ir::TypeRef::Named(n) if n == type_name);

    let call_args = napi_gen_call_args(&method.params, opaque_types);

    let has_named_ref_param = method
        .params
        .iter()
        .any(|p| crate::codegen::shared::is_named_ref_param_pub(p, opaque_types));
    let opaque_can_delegate = !method.sanitized
        && (!is_ref_mut_receiver || has_mut_methods)
        && (!is_owned_receiver || typ.is_clone)
        && !has_named_ref_param
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && crate::codegen::shared::is_delegatable_param(&p.ty, opaque_types))
        && crate::codegen::shared::is_opaque_delegatable_type(&method.return_type);

    let make_async_core_call = |method_name: &str| -> String {
        if has_mut_methods && !is_ref_mut_receiver {
            format!("inner.lock().unwrap().{method_name}({call_args})")
        } else {
            format!("inner.{method_name}({call_args})")
        }
    };

    let async_result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
    let async_result_wrap = napi_wrap_return(
        &async_result,
        &method.return_type,
        type_name,
        opaque_types,
        true,
        method.returns_ref,
        prefix,
        mutex_types,
    );

    let adapter_key = format!("{type_name}.{}", method.name);
    let body = if let Some(adapter_body) = adapter_bodies.get(&adapter_key) {
        adapter_body.clone()
    } else if !opaque_can_delegate {
        if cfg.has_serde
            && !method.sanitized
            && generators::has_named_params(&method.params, opaque_types)
            && method.error_type.is_some()
            && crate::codegen::shared::is_opaque_delegatable_type(&method.return_type)
        {
            let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
            let mut serde_bindings =
                generators::gen_serde_let_bindings(&method.params, opaque_types, cfg.core_import, err_conv, "        ");
            serde_bindings.push_str(&napi_newtype_param_bindings(&method.params));
            let serde_call_args = if has_promoted_required {
                generators::gen_call_args_with_let_bindings_mutex_no_promote(&method.params, opaque_types, mutex_types)
            } else {
                generators::gen_call_args_with_let_bindings_mutex(&method.params, opaque_types, mutex_types)
            };
            let core_call = if has_mut_methods {
                format!("self.inner.lock().unwrap().{}({serde_call_args})", method.name)
            } else {
                format!("self.inner.{}({serde_call_args})", method.name)
            };
            let await_suffix = if method.is_async { ".await" } else { "" };
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{serde_bindings}{core_call}{await_suffix}{err_conv}?;\n    Ok(())")
            } else if self_ref_return {
                format!(
                    "{serde_bindings}{core_call}{await_suffix}{err_conv}?;\n    Ok(Self {{ inner: self.inner.clone() }})"
                )
            } else {
                let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
                let wrap = napi_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    prefix,
                    mutex_types,
                );
                format!("{serde_bindings}let result = {core_call}{await_suffix}{err_conv}?;\n    Ok({wrap})")
            }
        } else {
            generators::gen_unimplemented_body(
                &method.return_type,
                &format!("{type_name}.{}", method.name),
                method.error_type.is_some(),
                cfg,
                &method.params,
                opaque_types,
            )
        }
    } else if method.is_async {
        let inner_clone_line = format!(
            "let inner = self.inner.clone();\n    {}",
            napi_newtype_param_bindings(&method.params)
        );
        let core_call_str = make_async_core_call(&method.name);
        generators::gen_async_body(
            &core_call_str,
            cfg,
            method.error_type.is_some(),
            &async_result_wrap,
            true,
            &inner_clone_line,
            matches!(method.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        let use_let_bindings = generators::has_named_params(&method.params, opaque_types);
        let (mut let_bindings, call_args_for_call) = if use_let_bindings {
            let bindings = if has_promoted_required {
                generators::gen_named_let_bindings_no_promote(&method.params, opaque_types, cfg.core_import)
            } else {
                generators::gen_named_let_bindings_pub(&method.params, opaque_types, cfg.core_import)
            };
            let args = napi_apply_primitive_casts_to_call_args(
                &if has_promoted_required {
                    generators::gen_call_args_with_let_bindings_mutex_no_promote(
                        &method.params,
                        opaque_types,
                        mutex_types,
                    )
                } else {
                    generators::gen_call_args_with_let_bindings_mutex(&method.params, opaque_types, mutex_types)
                },
                &method.params,
            );
            (bindings, args)
        } else {
            (String::new(), napi_gen_call_args(&method.params, opaque_types))
        };
        let_bindings.push_str(&napi_newtype_param_bindings(&method.params));
        let core_call = if is_owned_receiver {
            format!("(*self.inner).clone().{}({})", method.name, call_args_for_call)
        } else if has_mut_methods {
            format!("self.inner.lock().unwrap().{}({})", method.name, call_args_for_call)
        } else {
            format!("self.inner.{}({})", method.name, call_args_for_call)
        };
        if method.error_type.is_some() {
            let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{let_bindings}{core_call}{err_conv}?;\n    Ok(())")
            } else if self_ref_return {
                format!("{let_bindings}{core_call}{err_conv}?;\n    Ok(Self {{ inner: self.inner.clone() }})")
            } else {
                let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
                let wrap = napi_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    prefix,
                    mutex_types,
                );
                format!("{let_bindings}let result = {core_call}{err_conv}?;\n    Ok({wrap})")
            }
        } else if self_ref_return {
            format!("{let_bindings}{core_call};\n    Self {{ inner: self.inner.clone() }}")
        } else {
            let result = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
            format!(
                "{let_bindings}{}",
                napi_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    prefix,
                    mutex_types,
                )
            )
        }
    };

    let body = wrap_promoted_required_body(
        body,
        &method.params,
        method.error_type.is_some(),
        method.is_async,
        matches!(method.return_type, TypeRef::Unit),
    );

    let mut attrs = String::new();
    let sanitized_method_doc =
        crate::codegen::doc_emission::sanitize_rust_idioms(&method.doc, crate::codegen::doc_emission::DocTarget::TsDoc);
    crate::codegen::doc_emission::emit_rustdoc(&mut attrs, &sanitized_method_doc, "");
    if method.params.len() + 1 > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    if method.error_type.is_some() || has_promoted_required {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    if generators::is_trait_method_name(&method.name) {
        attrs.push_str("#[allow(clippy::should_implement_trait)]\n");
    }
    // A gated method keeps its gate rather than being filtered: `#[napi]` expands to a real Rust
    // item, the scaffolded `-node` crate declares every referenced cfg feature as a default-on
    // passthrough (`scaffold::languages::node`), and `rustc` resolves the gate — the same contract
    // top-level functions already get from `support::prepend_cfg`. `method.cfg` already carries its
    // `impl` block's gate AND-combined; `typ.cfg` is deliberately not folded in, because nothing in
    // this backend gates the struct or its impl block on it. ~keep
    attrs.push_str(&method.rust_cfg_attribute());
    format!(
        "{attrs}#[napi{js_name_attr}]\npub {async_kw}fn {}(&self, {params}) -> {return_annotation} {{\n    \
         {body}\n}}",
        method.name
    )
}
