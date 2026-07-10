use super::trait_names::is_trait_method_name;
use crate::codegen::generators::binding_helpers::{
    apply_return_newtype_unwrap, gen_async_body, gen_call_args, gen_call_args_cfg,
    gen_call_args_with_let_bindings_json_str, gen_named_let_bindings_pub, gen_unimplemented_body, has_named_params,
    wrap_return_with_mutex_mapped,
};
use crate::codegen::generators::{AdapterBodies, AsyncPattern, RustBindingConfig};
use crate::codegen::shared::{function_params, function_sig_defaults};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use ahash::AHashSet;

/// Generate a static method.
pub fn gen_static_method(
    method: &MethodDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    typ: &TypeDef,
    adapter_bodies: &AdapterBodies,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
) -> String {
    let type_name = &typ.name;
    let core_type_path = typ.rust_path.replace('-', "_");
    let map_fn = |ty: &crate::core::ir::TypeRef| mapper.map_type(ty);
    let params = function_params(&method.params, &map_fn);
    let return_type = mapper.map_type(&method.return_type);
    let ret = mapper.wrap_return(&return_type, method.error_type.is_some());

    let core_import = cfg.core_import;

    let use_let_bindings = has_named_params(&method.params, opaque_types);
    let (call_args, ref_let_bindings) = if use_let_bindings {
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

    let lifetime_bindings = if typ.has_lifetime_params {
        let mut bindings = String::new();
        for p in &method.params {
            match &p.ty {
                TypeRef::String => {
                    if p.optional {
                        bindings.push_str(&format!("let {}_converted = {}.map(Into::into);\n    ", p.name, p.name));
                    } else {
                        bindings.push_str(&format!(
                            "let {}_converted: std::borrow::Cow<'_, str> = {}.into();\n    ",
                            p.name, p.name
                        ));
                    }
                }
                TypeRef::Map(_, _) => {
                    bindings.push_str(&format!("let {}_converted = {}.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<std::collections::BTreeMap<_, _>>();\n    ", p.name, p.name));
                }
                TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String) => {
                    bindings.push_str(&format!("let {}_converted = {}.map(Into::into);\n    ", p.name, p.name));
                }
                _ => {}
            }
        }
        bindings
    } else {
        String::new()
    };

    let is_borrowed_to_owned = method.name.contains("borrowed_attributes");
    let (call_args, method_name_override) = if !lifetime_bindings.is_empty() {
        let mut adjusted = call_args.clone();
        for p in &method.params {
            match &p.ty {
                TypeRef::Map(_, _) => {
                    if is_borrowed_to_owned && p.is_ref {
                        adjusted = adjusted.replace(&format!("&{}", p.name), &format!("{}_converted", p.name));
                    } else {
                        adjusted = adjusted.replace(&p.name.to_string(), &format!("{}_converted", p.name));
                    }
                }
                TypeRef::String => {
                    adjusted = adjusted.replace(&p.name.to_string(), &format!("{}_converted", p.name));
                }
                TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String) => {
                    adjusted = adjusted.replace(&p.name.to_string(), &format!("{}_converted", p.name));
                }
                _ => {}
            }
        }
        let override_name = if is_borrowed_to_owned {
            Some(method.name.replace("borrowed", "owned"))
        } else {
            None
        };
        (adjusted, override_name)
    } else {
        (call_args, None)
    };

    let actual_method_name = method_name_override.as_deref().unwrap_or(&method.name);

    let can_delegate = crate::codegen::shared::can_auto_delegate(method, opaque_types)
        || crate::codegen::shared::can_auto_delegate_with_named_let_bindings(method, opaque_types);

    let adapter_key = format!("{}.{}", type_name, method.name);
    let adapter_override = adapter_bodies.get(&adapter_key).cloned();

    let body = if let Some(adapter_body) = adapter_override {
        adapter_body
    } else if !can_delegate {
        gen_unimplemented_body(
            &method.return_type,
            &format!("{type_name}::{}", method.name),
            method.error_type.is_some(),
            cfg,
            &method.params,
            opaque_types,
        )
    } else if method.is_async {
        let core_call = format!("{core_type_path}::{}({call_args})", actual_method_name);
        let return_wrap = format!("{return_type}::from(result)");
        gen_async_body(
            &core_call,
            cfg,
            method.error_type.is_some(),
            &return_wrap,
            false,
            "",
            matches!(method.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        let core_call = format!("{core_type_path}::{}({call_args})", actual_method_name);
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
            let val_expr = apply_return_newtype_unwrap("val", &method.return_newtype_wrapper);
            let wrapped = wrap_return_with_mutex_mapped(
                &val_expr,
                &method.return_type,
                type_name,
                opaque_types,
                mutex_types,
                typ.is_opaque,
                method.returns_ref,
                method.returns_cow,
                mapper,
            );
            if wrapped == val_expr {
                format!("{core_call}{err_conv}")
            } else if wrapped == format!("{val_expr}.into()") {
                format!("{core_call}.map(Into::into){err_conv}")
            } else if let Some(type_path) = wrapped.strip_suffix(&format!("::from({val_expr})")) {
                format!("{core_call}.map({type_path}::from){err_conv}")
            } else {
                format!("{core_call}.map(|val| {wrapped}){err_conv}")
            }
        } else {
            let unwrapped_call = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
            wrap_return_with_mutex_mapped(
                &unwrapped_call,
                &method.return_type,
                type_name,
                opaque_types,
                mutex_types,
                typ.is_opaque,
                method.returns_ref,
                method.returns_cow,
                mapper,
            )
        }
    };
    let body = if ref_let_bindings.is_empty() && lifetime_bindings.is_empty() {
        body
    } else {
        format!("{ref_let_bindings}{lifetime_bindings}{body}")
    };

    let static_needs_py = method.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;

    let ret = if static_needs_py {
        "PyResult<Bound<'py, PyAny>>".to_string()
    } else {
        ret
    };
    let method_lifetime = if static_needs_py { "<'py>" } else { "" };

    let (sig_start, sig_params, sig_end) = if params.len() > 100 {
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
        if static_needs_py {
            (
                format!("pub fn {}{method_lifetime}(py: Python<'py>,\n        ", method.name),
                wrapped_params,
                "\n    ) -> ".to_string(),
            )
        } else {
            (
                format!("pub fn {}(\n        ", method.name),
                wrapped_params,
                "\n    ) -> ".to_string(),
            )
        }
    } else if static_needs_py {
        (
            format!("pub fn {}{method_lifetime}(py: Python<'py>, ", method.name),
            params,
            ") -> ".to_string(),
        )
    } else {
        (format!("pub fn {}(", method.name), params, ") -> ".to_string())
    };

    let total_params = method.params.len() + if static_needs_py { 1 } else { 0 };
    let sig_defaults = if cfg.needs_signature {
        function_sig_defaults(&method.params)
    } else {
        String::new()
    };
    let static_attr_str = if let Some(attr) = cfg.static_attr {
        format!("#[{attr}]")
    } else {
        String::new()
    };

    let mut out = String::with_capacity(1024);
    if total_params > 7 {
        out.push_str("    #[allow(clippy::too_many_arguments)]\n");
    }
    if method.error_type.is_some() {
        out.push_str("    #[allow(clippy::missing_errors_doc)]\n");
    }
    if is_trait_method_name(&method.name) {
        out.push_str("    #[allow(clippy::should_implement_trait)]\n");
    }
    if !static_attr_str.is_empty() {
        out.push_str(&crate::codegen::template_env::render(
            "generators/methods/static_attr.jinja",
            minijinja::context! {
                static_attr_str => static_attr_str,
            },
        ));
    }
    if cfg.needs_signature {
        out.push_str(&crate::codegen::template_env::render(
            "generators/methods/signature_attr.jinja",
            minijinja::context! {
                signature_prefix => &cfg.signature_prefix,
                sig_defaults => sig_defaults,
                signature_suffix => &cfg.signature_suffix,
            },
        ));
    }
    out.push_str(&crate::codegen::template_env::render(
        "generators/methods/method_body.jinja",
        minijinja::context! {
            sig_start => sig_start,
            sig_params => sig_params,
            sig_end => sig_end,
            ret => ret,
            body => body,
        },
    ));
    out
}
