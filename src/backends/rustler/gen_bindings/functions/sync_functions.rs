use super::default_deserialization::{render_fallible_deser_line, render_ok_expression};
use super::shared::{
    render_deser_line, render_named_deser_line, render_preamble, render_result_body, render_wrapped_body,
    resolve_core_type_path,
};
use crate::backends::rustler::gen_bindings::types::gen_rustler_wrap_return;
use crate::backends::rustler::template_env;
use crate::backends::rustler::type_map::RustlerMapper;
use crate::codegen::doc_emission;
use crate::codegen::shared;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{CoreWrapper, FunctionDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

/// Generate a Rustler NIF free function using the shared TypeMapper.
pub(in crate::backends::rustler::gen_bindings) fn gen_nif_function(
    func: &FunctionDef,
    mapper: &RustlerMapper,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    core_import: &str,
    cpu_bound_functions: &AHashSet<String>,
    types_by_name: &AHashMap<&str, &TypeDef>,
) -> String {
    let params_str = func
        .params
        .iter()
        .map(|p| {
            if let TypeRef::Named(n) = &p.ty {
                if opaque_types.contains(n) {
                    return format!("{}: rustler::ResourceArc<{}>", p.name, n);
                }
                // partial maps work — serde_json::from_str respects #[serde(default)].
                if default_types.contains(n) {
                    return format!("{}: Option<String>", p.name);
                }
                if p.optional {
                    return format!("{}: Option<{}>", p.name, n);
                }
            }
            if let TypeRef::Vec(inner) = &p.ty
                && let TypeRef::Named(inner_name) = inner.as_ref()
                && !opaque_types.contains(inner_name.as_str())
            {
                return format!("{}: Option<String>", p.name);
            }
            if matches!(&p.ty, TypeRef::Bytes) {
                return if p.optional {
                    format!("{}: Option<rustler::Binary>", p.name)
                } else {
                    format!("{}: rustler::Binary", p.name)
                };
            }
            let mapped = mapper.map_type(&p.ty);
            if p.optional {
                format!("{}: Option<{}>", p.name, mapped)
            } else {
                format!("{}: {}", p.name, mapped)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let return_type =
        crate::backends::rustler::gen_bindings::helpers::map_return_type(&func.return_type, mapper, opaque_types);
    let has_default_params = func
        .params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Named(n) if default_types.contains(n)));

    let has_batch_vec_params = func.params.iter().any(|p| {
        if let TypeRef::Vec(inner) = &p.ty
            && let TypeRef::Named(inner_name) = inner.as_ref()
        {
            return !opaque_types.contains(inner_name.as_str());
        }
        false
    });

    // `can_auto_delegate_function` disqualifies any required (non-`Option`) non-opaque `&Named`
    // param via `is_named_ref_param` — sound in general, since most backends' call-arg builders
    // only know how to `.into()` an *owned* value. This generator is not most backends: the
    // call-args closure below already converts that exact shape (`&{name}.clone().into()`), which
    // type-checks because every non-opaque Named binding type derives `Clone` (see
    // gen_bindings/types.rs) and relies on the same `From<BindingType> for CoreType` conversion the
    // non-ref `.into()` arm already uses. Recognize that case here so such functions delegate
    // instead of falling through to `gen_rustler_unimplemented_body`, which — for a non-fallible
    // return like a bare `f64` — emits `compile_error!` into the consumer's default build path.
    // Optional `&Named` params and `Vec<&Named>`/`Vec<&str>` params are left excluded: this
    // generator has no exercised call-site for those shapes, so a false positive here would trade
    // one compile break for a less obvious one. ~keep
    let can_delegate_with_required_named_ref_params = !func.sanitized
        && func.params.iter().all(|p| {
            !p.sanitized
                && shared::is_delegatable_param(&p.ty, opaque_types)
                && (!shared::is_named_ref_param_pub(p, opaque_types)
                    || (!p.optional && matches!(&p.ty, TypeRef::Named(name) if !opaque_types.contains(name.as_str()))))
        })
        && shared::is_delegatable_return(&func.return_type);
    let can_delegate = shared::can_auto_delegate_function(func, opaque_types)
        || has_default_params
        || has_batch_vec_params
        || can_delegate_with_required_named_ref_params;
    let deserialization_introduces_result =
        crate::backends::rustler::gen_bindings::public_api_args::function_deserialization_introduces_result(
            func,
            opaque_types,
            default_types,
        );
    let return_annotation = mapper.wrap_return(
        &return_type,
        func.error_type.is_some() || deserialization_introduces_result,
    );

    let body = if can_delegate {
        let mut deser_lines: Vec<String> = Vec::new();
        let call_args: Vec<String> = func
            .params
            .iter()
            .map(|p| {
                if let TypeRef::Named(n) = &p.ty
                    && default_types.contains(n) {
                        let core_ty = resolve_core_type_path(n, types_by_name, core_import);
                        deser_lines.push(render_fallible_deser_line(
                            &p.name,
                            &format!("{}_core", p.name),
                            &core_ty,
                            true,
                            &func.name,
                        ));
                        if p.optional {
                            return format!("{}_core", p.name);
                        } else if p.is_ref && p.is_mut {
                            let mut_name = format!("{}_mut", p.name);
                            deser_lines.push(format!("let mut {mut_name} = {}_core.unwrap_or_default();", p.name));
                            return format!("&mut {mut_name}");
                        } else if p.is_ref {
                            return format!("{}_core.as_ref().unwrap_or(&Default::default())", p.name);
                        } else {
                            return format!("{}_core.unwrap_or_default()", p.name);
                        }
                    }
                if let TypeRef::Vec(inner) = &p.ty
                    && let TypeRef::Named(inner_name) = inner.as_ref()
                        && !opaque_types.contains(inner_name.as_str()) {
                            let inner_ty = resolve_core_type_path(inner_name, types_by_name, core_import);
                            let core_ty = format!("Vec<{}>", inner_ty);
                            deser_lines.push(render_fallible_deser_line(
                                &p.name,
                                &format!("{}_core_option", p.name),
                                &core_ty,
                                true,
                                &func.name,
                            ));
                            deser_lines.push(
                                template_env::render(
                                    "rust_let_binding.jinja",
                                    minijinja::context! {
                                        var_name => if p.is_ref && p.is_mut { format!("mut {}_core", p.name) } else { format!("{}_core", p.name) },
                                        var_type => &core_ty,
                                        expr => &format!("{}_core_option.unwrap_or_default()", p.name),
                                    },
                                )
                                .trim_end()
                                .to_string(),
                            );
                            return if p.is_ref && p.is_mut {
                                format!("&mut {}_core", p.name)
                            } else if p.is_ref {
                                format!("&{}_core", p.name)
                            } else {
                                format!("{}_core", p.name)
                            };
                        }
                if let TypeRef::Map(_, _) = &p.ty
                    && p.map_is_ahash && p.map_key_is_cow {
                        let bound_name = format!("__{}_ahash", p.name);
                        deser_lines.push(format!(
                            "let {bound_name} = {}.map(|m| m.into_iter().map(|(k, v)| (std::borrow::Cow::Owned(k), serde_json::Value::String(v))).collect::<ahash::AHashMap<std::borrow::Cow<'static, str>, serde_json::Value>>());",
                            p.name
                        ));
                        return if p.optional && p.is_ref {
                            format!("{bound_name}.as_ref()")
                        } else if p.is_ref {
                            format!("{bound_name}.as_ref().unwrap()")
                        } else {
                            bound_name
                        };
                    }
                match &p.ty {
                    TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                        format!("&{}.inner.read().unwrap_or_else(|e| e.into_inner()).clone()", p.name)
                    }
                    TypeRef::Named(_) => {
                        if p.optional {
                            if p.is_ref {
                                format!("{}.as_ref().map(Into::into)", p.name)
                            } else {
                                format!("{}.map(Into::into)", p.name)
                            }
                        } else if p.is_ref {
                            format!("&{}.clone().into()", p.name)
                        } else {
                            format!("{}.into()", p.name)
                        }
                    }
                    TypeRef::String | TypeRef::Char if p.optional && p.is_ref => {
                        format!("{}.as_deref()", p.name)
                    }
                    TypeRef::String | TypeRef::Char if p.optional && p.core_wrapper == CoreWrapper::Cow => {
                        format!("{}.map(std::borrow::Cow::Owned)", p.name)
                    }
                    TypeRef::String | TypeRef::Char if p.optional => {
                        p.name.to_string()
                    }
                    TypeRef::String | TypeRef::Char if p.is_ref => {
                        format!("&{}", p.name)
                    }
                    TypeRef::String | TypeRef::Char if p.core_wrapper == CoreWrapper::Cow => {
                        format!("{}.into()", p.name)
                    }
                    TypeRef::String | TypeRef::Char => {
                        p.name.clone()
                    }
                    TypeRef::Path => {
                        if p.optional && p.is_ref {
                            format!("{}.as_deref().map(std::path::Path::new)", p.name)
                        } else if p.optional {
                            format!("{}.map(std::path::PathBuf::from)", p.name)
                        } else if p.is_ref {
                            format!("&std::path::PathBuf::from({})", p.name)
                        } else {
                            format!("std::path::PathBuf::from({})", p.name)
                        }
                    }
                    TypeRef::Bytes => {
                        if p.optional {
                            if p.is_ref {
                                format!("{}.map(|b| b.as_slice())", p.name)
                            } else {
                                format!("{}.map(|b| b.as_slice().to_vec())", p.name)
                            }
                        } else if p.is_ref {
                            format!("{}.as_slice()", p.name)
                        } else {
                            format!("{}.as_slice().to_vec()", p.name)
                        }
                    }
                    TypeRef::Json => {
                        if p.optional {
                            deser_lines.push(render_fallible_deser_line(
                                &p.name,
                                &format!("{}_json", p.name),
                                "serde_json::Value",
                                true,
                                &func.name,
                            ));
                            format!("{}_json", p.name)
                        } else {
                            deser_lines.push(render_fallible_deser_line(
                                &p.name,
                                &format!("{}_json", p.name),
                                "serde_json::Value",
                                false,
                                &func.name,
                            ));
                            format!("{}_json", p.name)
                        }
                    }
                    TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
                    TypeRef::Vec(inner) if p.is_ref && matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) => {
                        if p.optional {
                            deser_lines.push(render_named_deser_line("vec_str_refs_optional.rs.jinja", &p.name));
                        } else {
                            deser_lines.push(render_named_deser_line("vec_str_refs_required.rs.jinja", &p.name));
                        }
                        format!("&{}_refs", p.name)
                    }
                    TypeRef::Vec(_) => {
                        if p.is_ref {
                            if p.optional {
                                format!("{}_core.as_ref().map(|v| v.as_slice()).unwrap_or(&[])", p.name)
                            } else {
                                format!("&{}_core", p.name)
                            }
                        } else {
                            p.name.to_string()
                        }
                    }
                    TypeRef::Map(_, _) if p.map_is_btree => {
                        if p.is_ref {
                            let bound_name = format!("__{}_btree", p.name);
                            deser_lines.push(format!(
                                "let {bound_name} = {}.into_iter().collect::<std::collections::BTreeMap<_, _>>();",
                                p.name
                            ));
                            format!("&{bound_name}")
                        } else {
                            format!("{}.into_iter().collect::<std::collections::BTreeMap<_, _>>()", p.name)
                        }
                    }
                    _ => p.name.clone(),
                }
            })
            .collect();

        let preamble = render_preamble(&deser_lines);

        let core_fn_path = {
            let path = func.rust_path.replace('-', "_");
            if path.starts_with(core_import) {
                path
            } else {
                format!("{core_import}::{}", func.name)
            }
        };
        let core_call = format!("{core_fn_path}({})", call_args.join(", "));
        if func.error_type.is_some() {
            let wrap = gen_rustler_wrap_return("result", &func.return_type, "", opaque_types, func.returns_ref);
            render_result_body(&preamble, &core_call, &wrap)
        } else {
            let wrap = gen_rustler_wrap_return(&core_call, &func.return_type, "", opaque_types, func.returns_ref);
            if deserialization_introduces_result {
                render_wrapped_body(&preamble, &render_ok_expression(&wrap))
            } else {
                render_wrapped_body(&preamble, &wrap)
            }
        }
    } else if !func.sanitized && func.error_type.is_some() {
        let mut deser_lines: Vec<String> = Vec::new();
        let call_args: Vec<String> = func
            .params
            .iter()
            .map(|p| {
                if let TypeRef::Named(n) = &p.ty {
                    if opaque_types.contains(n) {
                        return format!("&{}.inner.read().unwrap_or_else(|e| e.into_inner()).clone()", p.name);
                    }
                    if default_types.contains(n) {
                        let core_ty = resolve_core_type_path(n, types_by_name, core_import);
                        deser_lines.push(render_fallible_deser_line(
                            &p.name,
                            &format!("{}_core", p.name),
                            &core_ty,
                            true,
                            &func.name,
                        ));
                        return if p.optional {
                            format!("{}_core", p.name)
                        } else if p.is_ref {
                            format!("{}_core.as_ref().unwrap_or(&Default::default())", p.name)
                        } else {
                            format!("{}_core.unwrap_or_default()", p.name)
                        };
                    }
                    let core_ty = resolve_core_type_path(n, types_by_name, core_import);
                    deser_lines.push(render_named_deser_line("named_param_to_json.rs.jinja", &p.name));
                    deser_lines.push(render_deser_line("named_param_from_json.rs.jinja", &p.name, &core_ty));
                    return if p.is_ref {
                        format!("&{}_core", p.name)
                    } else {
                        format!("{}_core", p.name)
                    };
                }
                match &p.ty {
                    TypeRef::String | TypeRef::Char if p.optional && p.is_ref => {
                        format!("{}.as_deref()", p.name)
                    }
                    TypeRef::String | TypeRef::Char if p.optional => p.name.to_string(),
                    TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
                    TypeRef::String | TypeRef::Char => p.name.clone(),
                    TypeRef::Path => {
                        if p.is_ref {
                            format!("&std::path::PathBuf::from({})", p.name)
                        } else {
                            format!("std::path::PathBuf::from({})", p.name)
                        }
                    }
                    TypeRef::Bytes => {
                        if p.optional {
                            if p.is_ref {
                                format!("{}.map(|b| b.as_slice())", p.name)
                            } else {
                                format!("{}.map(|b| b.as_slice().to_vec())", p.name)
                            }
                        } else if p.is_ref {
                            format!("{}.as_slice()", p.name)
                        } else {
                            format!("{}.as_slice().to_vec()", p.name)
                        }
                    }
                    TypeRef::Json => {
                        if p.optional {
                            deser_lines.push(format!(
                                "let {}_json: Option<serde_json::Value> = {}.map(|s| serde_json::from_str(&s)).transpose().map_err(|e| e.to_string())?;",
                                p.name, p.name
                            ));
                            format!("{}_json", p.name)
                        } else {
                            deser_lines.push(format!(
                                "let {}_json: serde_json::Value = serde_json::from_str(&{}).map_err(|e| e.to_string())?;",
                                p.name, p.name
                            ));
                            format!("{}_json", p.name)
                        }
                    }
                    TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
                    TypeRef::Vec(inner) if p.is_ref && matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) => {
                        if p.optional {
                            deser_lines.push(render_named_deser_line("vec_str_refs_optional.rs.jinja", &p.name));
                        } else {
                            deser_lines.push(render_named_deser_line("vec_str_refs_required.rs.jinja", &p.name));
                        }
                        format!("&{}_refs", p.name)
                    }
                    TypeRef::Vec(_) => {
                        if p.is_ref {
                            format!("&{}", p.name)
                        } else {
                            p.name.to_string()
                        }
                    }
                    TypeRef::Map(_, _) if p.map_is_btree => {
                        if p.is_ref {
                            let bound_name = format!("__{}_btree", p.name);
                            deser_lines.push(format!(
                                "let {bound_name} = {}.into_iter().collect::<std::collections::BTreeMap<_, _>>();",
                                p.name
                            ));
                            format!("&{bound_name}")
                        } else {
                            format!("{}.into_iter().collect::<std::collections::BTreeMap<_, _>>()", p.name)
                        }
                    }
                    _ => p.name.clone(),
                }
            })
            .collect();

        let preamble = render_preamble(&deser_lines);

        let core_fn_path = {
            let path = func.rust_path.replace('-', "_");
            if path.starts_with(core_import) {
                path
            } else {
                format!("{core_import}::{}", func.name)
            }
        };
        let core_call = format!("{core_fn_path}({})", call_args.join(", "));
        let wrap = gen_rustler_wrap_return("result", &func.return_type, "", opaque_types, func.returns_ref);
        render_result_body(&preamble, &core_call, &wrap)
    } else {
        // Locks the body's fallibility to the same expression as `return_annotation` above.
        // Vacuous today — `function_deserialization_introduces_result` is itself gated on
        // `can_delegate`, which is false on this branch — but the two must not drift: were
        // they to disagree, the NIF would be declared `-> Result<_, _>` while the body was
        // generated as infallible, putting `compile_error!` in the consumer's NIF crate for
        // a function that could have returned a plain `Err`. ~keep
        crate::backends::rustler::gen_bindings::helpers::gen_rustler_unimplemented_body(
            &func.return_type,
            &func.name,
            func.error_type.is_some() || deserialization_introduces_result,
        )
    };
    let mut out = String::new();
    doc_emission::emit_rustdoc(&mut out, &func.doc, "");
    let template_name = if cpu_bound_functions.contains(func.name.as_str()) {
        "dirty_cpu_nif_function.rs.jinja"
    } else {
        "nif_function.rs.jinja"
    };
    out.push_str(&template_env::render(
        template_name,
        minijinja::context! {
            func_name => &func.name,
            params_str => &params_str,
            ret => &return_annotation,
            body => &body,
        },
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::gen_nif_function;
    use crate::backends::rustler::type_map::RustlerMapper;
    use crate::core::ir::{FunctionDef, ParamDef, PrimitiveType, TypeRef};
    use ahash::{AHashMap, AHashSet};

    fn required_named_ref_param(name: &str, type_name: &str) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty: TypeRef::Named(type_name.to_string()),
            is_ref: true,
            ..ParamDef::default()
        }
    }

    /// Regression test for a shipped defect: a free function whose only non-delegatable params are
    /// required (non-`Option`) `&Named` non-opaque references, returning a bare non-fallible `f64`,
    /// used to fall through to `gen_rustler_unimplemented_body` and emit `compile_error!` into the
    /// consumer's default build path (`score_pair(query: &SampleVector, candidate:
    /// &SampleVector) -> f64`). It must now delegate to the real core call, since this
    /// generator's own call-arg closure already knows how to convert a required `&Named` param via
    /// `&{name}.clone().into()`.
    #[test]
    fn required_named_ref_params_with_bare_f64_return_delegate_instead_of_compile_error() {
        let func = FunctionDef {
            name: "score_pair".to_string(),
            rust_path: "sample_crate::vector_ops::score_pair".to_string(),
            params: vec![
                required_named_ref_param("query", "SampleVector"),
                required_named_ref_param("candidate", "SampleVector"),
            ],
            return_type: TypeRef::Primitive(PrimitiveType::F64),
            ..FunctionDef::default()
        };

        let body = gen_nif_function(
            &func,
            &RustlerMapper,
            &AHashSet::default(),
            &AHashSet::default(),
            "sample_crate",
            &AHashSet::default(),
            &AHashMap::default(),
        );

        assert!(
            !body.contains("compile_error!"),
            "a required &Named param must not force compile_error! for a non-fallible return: {body}"
        );
        assert!(
            body.contains("vector_ops::score_pair(&query.clone().into(), &candidate.clone().into())"),
            "expected the function to delegate to the real core call, got: {body}"
        );
    }
}
