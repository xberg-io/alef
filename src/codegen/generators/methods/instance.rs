use super::trait_names::is_trait_method_name;
use crate::codegen::generators::binding_helpers::{
    apply_return_newtype_unwrap, gen_async_body, gen_call_args, gen_call_args_cfg,
    gen_call_args_with_let_bindings_json_str, gen_lossy_binding_to_core_fields, gen_lossy_binding_to_core_fields_mut,
    gen_named_let_bindings_pub, gen_serde_let_bindings, gen_unimplemented_body, has_named_params,
    is_simple_non_opaque_param, wrap_return_with_mutex_mapped,
};
use crate::codegen::generators::{AdapterBodies, AsyncPattern, RustBindingConfig};
use crate::codegen::shared::{function_params, function_sig_defaults};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use ahash::AHashSet;

/// Generate an instance method.
///
/// When `is_opaque` is true, generates delegation to `self.inner` via Arc clone
/// instead of converting self to core type.
///
/// `opaque_types` is the set of opaque type names, used for correct return wrapping.
/// `mutex_types` is the subset of opaque types whose `inner` field is `Arc<Mutex<T>>`;
/// method dispatch uses `.lock().unwrap()` for these types.
#[allow(clippy::too_many_arguments)]
pub fn gen_method(
    method: &MethodDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    typ: &TypeDef,
    is_opaque: bool,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    adapter_bodies: &AdapterBodies,
) -> String {
    let type_name = &typ.name;
    let core_type_path = typ.rust_path.replace('-', "_");

    let map_fn = |ty: &crate::core::ir::TypeRef| mapper.map_type(ty);
    let params = function_params(&method.params, &map_fn);
    let return_type = mapper.map_type(&method.return_type);
    let ret = mapper.wrap_return(&return_type, method.error_type.is_some());

    let core_import = cfg.core_import;

    let has_ref_named_params = has_named_params(&method.params, opaque_types);
    let (call_args, ref_let_bindings) = if has_ref_named_params {
        (
            gen_call_args_with_let_bindings_json_str(&method.params, opaque_types),
            gen_named_let_bindings_pub(&method.params, opaque_types, core_import),
        )
    } else if cfg.cast_uints_to_i32 || cfg.cast_large_ints_to_f64 {
        (
            gen_call_args_cfg(
                &method.params,
                opaque_types,
                cfg.cast_uints_to_i32,
                cfg.cast_large_ints_to_f64,
            ),
            String::new(),
        )
    } else {
        (gen_call_args(&method.params, opaque_types), String::new())
    };

    let is_owned_receiver = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::Owned));
    let is_ref_mut_receiver = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));

    let is_functional_ref_mut = !is_opaque
        && is_ref_mut_receiver
        && !method.sanitized
        && method.trait_source.is_none()
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && crate::codegen::shared::is_delegatable_param(&p.ty, opaque_types));

    let is_trait_method = method.trait_source.is_some();

    let self_needs_mutex = is_opaque && mutex_types.contains(type_name.as_str());

    let opaque_can_delegate = is_opaque
        && !method.sanitized
        && (!is_ref_mut_receiver || self_needs_mutex)
        && (!is_trait_method || self_needs_mutex)
        && (!is_owned_receiver || typ.is_clone)
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && crate::codegen::shared::is_opaque_delegatable_type(&p.ty))
        && crate::codegen::shared::is_opaque_delegatable_type(&method.return_type);

    let make_core_call = |method_name: &str| -> String {
        if is_opaque {
            if is_owned_receiver {
                if self_needs_mutex {
                    format!("self.inner.lock().unwrap().clone().{method_name}({call_args})")
                } else {
                    format!("(*self.inner).clone().{method_name}({call_args})")
                }
            } else if self_needs_mutex {
                format!("self.inner.lock().unwrap().{method_name}({call_args})")
            } else {
                format!("self.inner.{method_name}({call_args})")
            }
        } else {
            format!("{core_type_path}::from(self.clone()).{method_name}({call_args})")
        }
    };

    let make_async_core_call = |method_name: &str| -> String {
        if is_opaque {
            if self_needs_mutex {
                format!("inner.lock().unwrap().{method_name}({call_args})")
            } else {
                format!("inner.{method_name}({call_args})")
            }
        } else {
            format!("{core_type_path}::from(self.clone()).{method_name}({call_args})")
        }
    };

    let result_expr = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
    let async_result_wrap = if is_opaque {
        wrap_return_with_mutex_mapped(
            &result_expr,
            &method.return_type,
            type_name,
            opaque_types,
            mutex_types,
            is_opaque,
            method.returns_ref,
            method.returns_cow,
            mapper,
        )
    } else {
        match &method.return_type {
            TypeRef::Named(_) | TypeRef::Json => format!("{result_expr}.into()"),
            _ => result_expr.clone(),
        }
    };

    let adapter_key_inner = format!("{}.{}", type_name, method.name);
    let adapter_override = adapter_bodies.get(&adapter_key_inner).cloned();

    let body = if let Some(adapter_body) = adapter_override {
        adapter_body
    } else if !opaque_can_delegate {
        if cfg.has_serde
            && is_opaque
            && !method.sanitized
            && !is_trait_method
            && has_named_params(&method.params, opaque_types)
            && method.error_type.is_some()
            && crate::codegen::shared::is_opaque_delegatable_type(&method.return_type)
        {
            // NOTE: Only executed when has_serde=true, ensuring serde_json calls are gated.
            let err_conv = match cfg.async_pattern {
                AsyncPattern::Pyo3FutureIntoPy => {
                    ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))"
                }
                AsyncPattern::NapiNativeAsync => {
                    ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
                }
                AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))",
                AsyncPattern::TokioBlockOn => {
                    ".map_err(|e| extendr_api::Error::Other(e.to_string().replace(\":\", \"_\").replace(\"/\", \"_\").replace(\"-\", \"_\").chars().take(255).collect::<String>()))"
                }
                _ => ".map_err(|e| e.to_string())",
            };
            let serde_bindings =
                gen_serde_let_bindings(&method.params, opaque_types, cfg.core_import, err_conv, "        ");
            let serde_call_args = gen_call_args_with_let_bindings_json_str(&method.params, opaque_types);
            let core_call = if self_needs_mutex {
                format!("self.inner.lock().unwrap().{}({serde_call_args})", method.name)
            } else {
                format!("self.inner.{}({serde_call_args})", method.name)
            };
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{serde_bindings}{core_call}{err_conv}?;\n        Ok(())")
            } else {
                let wrap = wrap_return_with_mutex_mapped(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    mutex_types,
                    is_opaque,
                    method.returns_ref,
                    method.returns_cow,
                    mapper,
                );
                format!("{serde_bindings}let result = {core_call}{err_conv}?;\n        Ok({wrap})")
            }
        } else if is_functional_ref_mut {
            let field_conversions = gen_lossy_binding_to_core_fields_mut(
                typ,
                cfg.core_import,
                cfg.option_duration_on_defaults,
                opaque_types,
                cfg.cast_uints_to_i32,
                cfg.cast_large_ints_to_f64,
                cfg.lossy_skip_types,
            );
            let core_call = format!("core_self.{}({call_args})", method.name);
            if method.error_type.is_some() {
                let err_conv = match cfg.async_pattern {
                    AsyncPattern::Pyo3FutureIntoPy => {
                        ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))"
                    }
                    AsyncPattern::NapiNativeAsync => {
                        ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
                    }
                    AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))",
                    AsyncPattern::TokioBlockOn => {
                        ".map_err(|e| extendr_api::Error::Other(e.to_string().replace(\":\", \"_\").replace(\"/\", \"_\").replace(\"-\", \"_\").chars().take(255).collect::<String>()))"
                    }
                    _ => ".map_err(|e| e.to_string())",
                };
                format!("{field_conversions}{core_call}{err_conv}?;\n        Ok(core_self.into())")
            } else {
                format!("{field_conversions}{core_call};\n        core_self.into()")
            }
        } else if !is_opaque
            && !method.sanitized
            && method
                .params
                .iter()
                .all(|p| !p.sanitized && is_simple_non_opaque_param(&p.ty))
            && crate::codegen::shared::is_delegatable_return(&method.return_type)
        {
            let is_ref_mut = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));
            let field_conversions = if is_ref_mut {
                gen_lossy_binding_to_core_fields_mut(
                    typ,
                    cfg.core_import,
                    cfg.option_duration_on_defaults,
                    opaque_types,
                    cfg.cast_uints_to_i32,
                    cfg.cast_large_ints_to_f64,
                    cfg.lossy_skip_types,
                )
            } else {
                gen_lossy_binding_to_core_fields(
                    typ,
                    cfg.core_import,
                    cfg.option_duration_on_defaults,
                    opaque_types,
                    cfg.cast_uints_to_i32,
                    cfg.cast_large_ints_to_f64,
                    cfg.lossy_skip_types,
                )
            };
            let core_call = format!("core_self.{}({call_args})", method.name);
            let newtype_suffix = if method.return_newtype_wrapper.is_some() {
                ".0"
            } else {
                ""
            };
            let result_wrap = match &method.return_type {
                TypeRef::Named(n) if n == type_name && (method.returns_cow || method.returns_ref) => {
                    ".into_owned().into()".to_string()
                }
                TypeRef::Named(_) if method.returns_cow || method.returns_ref => ".into_owned().into()".to_string(),
                TypeRef::Named(n) if n == type_name => ".into()".to_string(),
                TypeRef::Named(_) => ".into()".to_string(),
                TypeRef::String => {
                    if method.returns_ref {
                        ".to_owned()".to_string()
                    } else {
                        String::new()
                    }
                }
                TypeRef::Path => {
                    if method.returns_ref {
                        ".to_owned()".to_string()
                    } else {
                        ".to_string_lossy().to_string()".to_string()
                    }
                }
                TypeRef::Bytes => ".to_vec()".to_string(),
                TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                    if method.returns_ref {
                        ".map(|v| v.clone().into())".to_string()
                    } else {
                        ".map(Into::into)".to_string()
                    }
                }
                TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Bytes) => {
                    if method.returns_ref {
                        ".map(|v| v.to_owned())".to_string()
                    } else {
                        String::new()
                    }
                }
                TypeRef::Primitive(p) => {
                    use crate::codegen::conversions::helpers::{needs_f64_cast, needs_i32_cast};
                    if cfg.cast_uints_to_i32 && needs_i32_cast(p) {
                        " as i32".to_string()
                    } else if cfg.cast_large_ints_to_f64 && needs_f64_cast(p) {
                        " as f64".to_string()
                    } else {
                        String::new()
                    }
                }
                TypeRef::Optional(inner) => match inner.as_ref() {
                    TypeRef::Primitive(p) => {
                        use crate::codegen::conversions::helpers::{needs_f64_cast, needs_i32_cast};
                        if cfg.cast_uints_to_i32 && needs_i32_cast(p) {
                            ".map(|v| v as i32)".to_string()
                        } else if cfg.cast_large_ints_to_f64 && needs_f64_cast(p) {
                            ".map(|v| v as f64)".to_string()
                        } else {
                            String::new()
                        }
                    }
                    TypeRef::Vec(vec_inner) if matches!(vec_inner.as_ref(), TypeRef::Named(_)) => {
                        if method.returns_ref {
                            ".as_ref().map(|v| v.iter().map(|x| x.clone().into()).collect())".to_string()
                        } else {
                            ".map(|v| v.into_iter().map(Into::into).collect())".to_string()
                        }
                    }
                    _ => String::new(),
                },
                TypeRef::Map(_, _) => {
                    if method.returns_ref {
                        ".iter().map(|(k, v)| (k.clone(), v.clone())).collect()".to_string()
                    } else {
                        String::new()
                    }
                }
                // For `&[T]` (returns_ref=true) use `.iter()` to avoid clippy::into_iter_on_ref.
                TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                    if method.returns_ref {
                        ".iter().map(|v| v.clone().into()).collect()".to_string()
                    } else {
                        ".into_iter().map(Into::into).collect()".to_string()
                    }
                }
                _ => String::new(),
            };
            if method.error_type.is_some() {
                let err_conv = match cfg.async_pattern {
                    AsyncPattern::Pyo3FutureIntoPy => {
                        ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))"
                    }
                    AsyncPattern::NapiNativeAsync => {
                        ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
                    }
                    AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))",
                    _ => ".map_err(|e| e.to_string())",
                };
                format!(
                    "{field_conversions}let result = {core_call}{err_conv}?;\n        Ok(result{newtype_suffix}{result_wrap})"
                )
            } else {
                format!("{field_conversions}{core_call}{newtype_suffix}{result_wrap}")
            }
        } else if is_opaque
            && !method.sanitized
            && (!is_ref_mut_receiver || self_needs_mutex)
            && (!is_owned_receiver || typ.is_clone)
            && method.error_type.is_none()
            && method
                .params
                .iter()
                .all(|p| !p.sanitized && crate::codegen::shared::is_opaque_delegatable_type(&p.ty))
            && matches!(&method.return_type, TypeRef::Named(n) if n == type_name)
        {
            let core_call = if is_owned_receiver {
                if self_needs_mutex {
                    format!("self.inner.lock().unwrap().clone().{}({call_args})", method.name)
                } else {
                    format!("(*self.inner).clone().{}({call_args})", method.name)
                }
            } else if self_needs_mutex {
                format!("self.inner.lock().unwrap().{}({call_args})", method.name)
            } else {
                format!("self.inner.{}({call_args})", method.name)
            };
            let unwrapped = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
            let arc_expr = if self_needs_mutex {
                format!("Arc::new(std::sync::Mutex::new({unwrapped}))")
            } else {
                format!("Arc::new({unwrapped})")
            };
            format!("Self {{ inner: {arc_expr} }}")
        } else if !is_opaque
            && !method.sanitized
            && !is_ref_mut_receiver
            && (!is_owned_receiver || typ.is_clone)
            && method.error_type.is_none()
            && method
                .params
                .iter()
                .all(|p| !p.sanitized && is_simple_non_opaque_param(&p.ty))
            && matches!(&method.return_type, TypeRef::Named(n) if n == type_name)
        {
            let is_ref_mut = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));
            let field_conversions = if is_ref_mut {
                gen_lossy_binding_to_core_fields_mut(
                    typ,
                    cfg.core_import,
                    cfg.option_duration_on_defaults,
                    opaque_types,
                    cfg.cast_uints_to_i32,
                    cfg.cast_large_ints_to_f64,
                    cfg.lossy_skip_types,
                )
            } else {
                gen_lossy_binding_to_core_fields(
                    typ,
                    cfg.core_import,
                    cfg.option_duration_on_defaults,
                    opaque_types,
                    cfg.cast_uints_to_i32,
                    cfg.cast_large_ints_to_f64,
                    cfg.lossy_skip_types,
                )
            };
            let core_call = format!("core_self.{}({call_args})", method.name);
            let newtype_suffix = if method.return_newtype_wrapper.is_some() {
                ".0"
            } else {
                ""
            };
            let result_wrap = if method.returns_cow || method.returns_ref {
                ".into_owned().into()"
            } else {
                ".into()"
            };
            format!("{field_conversions}{core_call}{newtype_suffix}{result_wrap}")
        } else {
            gen_unimplemented_body(
                &method.return_type,
                &format!("{type_name}.{}", method.name),
                method.error_type.is_some(),
                cfg,
                &method.params,
                opaque_types,
            )
        }
    } else if method.is_async {
        let mut inner_clone_line = if is_opaque {
            "let inner = self.inner.clone();\n        ".to_string()
        } else {
            String::new()
        };
        if cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy && !ref_let_bindings.is_empty() {
            inner_clone_line.push_str(&ref_let_bindings);
        }
        let core_call_str = make_async_core_call(&method.name);
        gen_async_body(
            &core_call_str,
            cfg,
            method.error_type.is_some(),
            &async_result_wrap,
            is_opaque,
            &inner_clone_line,
            matches!(method.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        let core_call = make_core_call(&method.name);
        if method.error_type.is_some() {
            let err_conv = match cfg.async_pattern {
                AsyncPattern::Pyo3FutureIntoPy => {
                    ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))"
                }
                AsyncPattern::NapiNativeAsync => {
                    ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
                }
                AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))",
                AsyncPattern::TokioBlockOn => {
                    ".map_err(|e| extendr_api::Error::Other(e.to_string().replace(\":\", \"_\").replace(\"/\", \"_\").replace(\"-\", \"_\").chars().take(255).collect::<String>()))"
                }
                _ => ".map_err(|e| e.to_string())",
            };
            if is_opaque {
                if matches!(method.return_type, TypeRef::Unit) {
                    format!("{core_call}{err_conv}?;\n        Ok(())")
                } else {
                    let wrap = wrap_return_with_mutex_mapped(
                        &result_expr,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        mutex_types,
                        is_opaque,
                        method.returns_ref,
                        method.returns_cow,
                        mapper,
                    );
                    format!("let result = {core_call}{err_conv}?;\n        Ok({wrap})")
                }
            } else {
                format!("{core_call}{err_conv}")
            }
        } else if is_opaque {
            let unwrapped_call = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
            let wrapped = wrap_return_with_mutex_mapped(
                &unwrapped_call,
                &method.return_type,
                type_name,
                opaque_types,
                mutex_types,
                is_opaque,
                method.returns_ref,
                method.returns_cow,
                mapper,
            );
            let cast = crate::codegen::generators::binding_helpers::primitive_return_cast_suffix(
                &method.return_type,
                cfg.cast_uints_to_i32,
                cfg.cast_large_ints_to_f64,
            );
            format!("{wrapped}{cast}")
        } else {
            core_call
        }
    };
    let adapter_key = format!("{}.{}", type_name, method.name);
    let has_adapter = adapter_bodies.contains_key(&adapter_key);

    // NOTE: For async Pyo3 methods with bindings, the bindings are moved INSIDE the async
    let body = if ref_let_bindings.is_empty() || has_adapter {
        body
    } else if method.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy {
        body
    } else {
        format!("{ref_let_bindings}{body}")
    };

    let needs_py = method.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;

    let body = if needs_py && !opaque_can_delegate && !has_adapter {
        let err_msg = format!("Not implemented: {type_name}.{}", method.name);
        let suppress = if method.params.is_empty() {
            String::new()
        } else {
            let names: Vec<&str> = method.params.iter().map(|p| p.name.as_str()).collect();
            if names.len() == 1 {
                format!("let _ = {};\n        ", names[0])
            } else {
                format!("let _ = ({});\n        ", names.join(", "))
            }
        };
        format!("{suppress}Err(pyo3::exceptions::PyNotImplementedError::new_err(\"{err_msg}\"))")
    } else {
        body
    };
    let self_param = match (needs_py, params.is_empty()) {
        (true, true) => "&self, py: Python<'py>",
        (true, false) => "&self, py: Python<'py>, ",
        (false, true) => "&self",
        (false, false) => "&self, ",
    };

    let ret = if needs_py {
        "PyResult<Bound<'py, PyAny>>".to_string()
    } else if is_functional_ref_mut {
        mapper.wrap_return("Self", method.error_type.is_some())
    } else {
        ret
    };
    let method_lifetime = if needs_py { "<'py>" } else { "" };

    let (sig_start, sig_params, sig_end) = if self_param.len() + params.len() > 100 {
        let wrapped_params = method
            .params
            .iter()
            .map(|p| {
                let ty = if p.optional {
                    format!("Option<{}>", mapper.map_type(&p.ty))
                } else {
                    mapper.map_type(&p.ty)
                };
                format!("{}: {}", p.name, ty)
            })
            .collect::<Vec<_>>()
            .join(",\n        ");
        let py_param = if needs_py { "\n        py: Python<'py>," } else { "" };
        (
            format!(
                "pub fn {}{method_lifetime}(\n        &self,{}\n        ",
                method.name, py_param
            ),
            wrapped_params,
            "\n    ) -> ".to_string(),
        )
    } else {
        (
            format!("pub fn {}{method_lifetime}({}", method.name, self_param),
            params,
            ") -> ".to_string(),
        )
    };

    let total_params = method.params.len() + 1 + if needs_py { 1 } else { 0 };
    let sig_defaults = if cfg.needs_signature {
        function_sig_defaults(&method.params)
    } else {
        String::new()
    };

    crate::codegen::template_env::render(
        "generators/methods/method_signature.jinja",
        minijinja::context! {
            has_too_many_arguments => total_params > 7,
            has_missing_errors_doc => method.error_type.is_some(),
            has_should_implement_trait => is_trait_method_name(&method.name),
            needs_signature => cfg.needs_signature,
            signature_prefix => cfg.signature_prefix,
            sig_defaults => sig_defaults,
            signature_suffix => cfg.signature_suffix,
            sig_start => sig_start,
            sig_params => sig_params,
            sig_end => sig_end,
            ret => ret,
            body => body,
        },
    )
}
