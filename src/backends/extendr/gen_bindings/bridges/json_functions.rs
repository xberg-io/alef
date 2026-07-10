use crate::backends::extendr::template_env;
use crate::codegen::generators::RustBindingConfig;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{FunctionDef, TypeRef};
use ahash::AHashSet;

pub fn return_type_needs_json(
    ret: &TypeRef,
    extendr_incompatible_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
) -> bool {
    match ret {
        TypeRef::Named(n) => {
            if enum_names.contains(n.as_str()) {
                return true;
            }
            extendr_incompatible_types.contains(n.as_str())
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(_) => true,
            TypeRef::Vec(_) => true,
            _ => false,
        },
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if enum_names.contains(n.as_str()) => true,
            TypeRef::Named(n) if !opaque_types.contains(n.as_str()) && !enum_names.contains(n.as_str()) => true,
            TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                TypeRef::Named(_) => true,
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

/// Generate a JSON-bridged `#[extendr]` free function.
///
/// When a function's return type or parameter types cannot be handled by extendr's automatic
/// Robj conversions, this generates a wrapper that:
///   - For incompatible return types (ExtractionResult, Vec<ExtractionResult>, Vec<Vec<f32/f64>>,
///     Option<Enum>): serializes the Rust result to a JSON string via serde_json.
///   - For incompatible parameter types (Vec<Struct>): takes a JSON `String` and deserializes it.
///   - Async functions use the TokioBlockOn pattern (no `async fn`).
pub fn gen_extendr_json_bridged_function(
    func: &FunctionDef,
    mapper: &dyn TypeMapper,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    cfg: &RustBindingConfig,
    extendr_incompatible_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
) -> String {
    use crate::codegen::generators::binding_helpers::gen_call_args_cfg;

    let err_map = ".map_err(|e| extendr_api::Error::Other(e.to_string().replace(\":\", \"_\").replace(\"/\", \"_\").replace(\"-\", \"_\").chars().take(255).collect::<String>()))";
    let rt_new = format!("tokio::runtime::Runtime::new(){err_map}?");

    let return_type_requires_json = matches!(&func.return_type, TypeRef::Named(n)
        if extendr_incompatible_types.contains(n.as_str()))
        || matches!(&func.return_type, TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::Named(n)
                if extendr_incompatible_types.contains(n.as_str())))
        || matches!(&func.return_type, TypeRef::Vec(inner)
            if matches!(inner.as_ref(), TypeRef::Named(n)
                if extendr_incompatible_types.contains(n.as_str())));

    let mut sig_params: Vec<String> = Vec::new();
    let mut body_preamble = String::new();

    for param in &func.params {
        let needs_json_vec = match &param.ty {
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(_) => true,
                _ => false,
            },
            TypeRef::Optional(opt_inner) => match opt_inner.as_ref() {
                TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                    TypeRef::Named(_) => true,
                    _ => false,
                },
                _ => false,
            },
            _ => false,
        };
        let needs_json_enum = matches!(&param.ty, TypeRef::Named(n)
            if enum_names.contains(n.as_str()))
            || matches!(&param.ty, TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n.as_str())));
        let needs_by_ref_struct = cfg.named_non_opaque_params_by_ref
            && !param.optional
            && matches!(&param.ty, TypeRef::Named(n)
                if !opaque_types.contains(n.as_str())
                    && !enum_names.contains(n.as_str())
                    && !extendr_incompatible_types.contains(n.as_str()));
        // by-ref config: these types have no `#[extendr] impl` block emitted, so neither
        let is_named_incompatible = matches!(&param.ty, TypeRef::Named(n)
            if extendr_incompatible_types.contains(n.as_str()));
        let needs_json_struct = !needs_json_enum
            && !needs_by_ref_struct
            && (is_named_incompatible
                || (matches!(&param.ty, TypeRef::Named(n)
                    if !opaque_types.contains(n.as_str())
                        && !enum_names.contains(n.as_str())
                        && !extendr_incompatible_types.contains(n.as_str()))
                    || matches!(&param.ty, TypeRef::Optional(inner)
                    if matches!(inner.as_ref(), TypeRef::Named(n)
                        if !opaque_types.contains(n.as_str())
                            && !enum_names.contains(n.as_str())
                            && !extendr_incompatible_types.contains(n.as_str()))))
                    && (param.optional || !cfg.named_non_opaque_params_by_ref || return_type_requires_json));
        if needs_json_vec {
            let (core_ty_path, is_optional) = match &param.ty {
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(n) => (format!("{core_import}::{n}"), false),
                    _ => unreachable!(),
                },
                TypeRef::Optional(opt_inner) => match opt_inner.as_ref() {
                    TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                        TypeRef::Named(n) => (format!("{core_import}::{n}"), true),
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
            let mut_kw = if param.is_mut { "mut " } else { "" };
            let param_is_optional = param.optional || is_optional;
            if param_is_optional {
                sig_params.push(format!("{}: Option<String>", param.name));
                body_preamble.push_str(&crate::backends::extendr::template_env::render(
                    "json_vec_optional_preamble.jinja",
                    minijinja::context! {
                        name => &param.name,
                        ty => &core_ty_path,
                        err_map => &err_map,
                        mut_kw => &mut_kw,
                    },
                ));
                body_preamble.push_str("    ");
            } else {
                sig_params.push(format!("{}: String", param.name));
                body_preamble.push_str(&crate::backends::extendr::template_env::render(
                    "json_vec_required_preamble.jinja",
                    minijinja::context! {
                        name => &param.name,
                        ty => &core_ty_path,
                        err_map => &err_map,
                        mut_kw => &mut_kw,
                    },
                ));
                body_preamble.push_str("    ");
            }
        } else if needs_by_ref_struct {
            let local_name = match &param.ty {
                TypeRef::Named(n) => n.clone(),
                _ => unreachable!(),
            };
            sig_params.push(format!("{}: &{local_name}", param.name));
        } else if needs_json_struct || needs_json_enum {
            let (core_ty_path, is_optional) = match &param.ty {
                TypeRef::Named(n) => (format!("{core_import}::{n}"), false),
                TypeRef::Optional(opt_inner) => match opt_inner.as_ref() {
                    TypeRef::Named(n) => (format!("{core_import}::{n}"), true),
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
            let mut_kw = if param.is_mut { "mut " } else { "" };
            let param_is_optional = param.optional || is_optional;
            if param_is_optional {
                sig_params.push(format!("{}: Option<String>", param.name));
                body_preamble.push_str(&template_env::render(
                    "json_struct_optional_preamble.jinja",
                    minijinja::context! {
                        mut_kw => mut_kw,
                        name => &param.name,
                        ty => &core_ty_path,
                        err => &err_map,
                    },
                ));
                body_preamble.push_str("    ");
            } else {
                sig_params.push(format!("{}: String", param.name));
                body_preamble.push_str(&template_env::render(
                    "json_struct_required_preamble.jinja",
                    minijinja::context! {
                        mut_kw => mut_kw,
                        name => &param.name,
                        ty => &core_ty_path,
                        err => &err_map,
                    },
                ));
                body_preamble.push_str("    ");
            }
        } else {
            let ty_str = mapper.map_type(&param.ty);
            let sig_ty = if matches!(&param.ty, TypeRef::Named(n) if !opaque_types.contains(n.as_str())) {
                if param.optional {
                    format!("extendr_api::Nullable<&{ty_str}>")
                } else {
                    format!("&{ty_str}")
                }
            } else if let TypeRef::Optional(inner) = &param.ty {
                let inner_name = if let TypeRef::Named(n) = inner.as_ref() {
                    if !opaque_types.contains(n.as_str()) {
                        Some(n.clone())
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(n) = inner_name {
                    format!("extendr_api::Nullable<&{n}>")
                } else {
                    format!("Option<{ty_str}>")
                }
            } else if param.optional {
                format!("Option<{ty_str}>")
            } else {
                ty_str
            };
            sig_params.push(format!("{}: {sig_ty}", param.name));
        }
    }

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    let mut named_let_bindings = String::new();
    for param in &func.params {
        let needs_json = matches!(&param.ty, TypeRef::Vec(inner)
            if matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str())));
        let needs_json_enum = matches!(&param.ty, TypeRef::Named(n)
            if enum_names.contains(n.as_str()))
            || matches!(&param.ty, TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n.as_str())));
        // types unconditionally route through JSON because they lack #[extendr] impl blocks
        let is_named_incompatible = matches!(&param.ty, TypeRef::Named(n)
            if extendr_incompatible_types.contains(n.as_str()));
        let needs_by_ref_struct = cfg.named_non_opaque_params_by_ref
            && !param.optional
            && matches!(&param.ty, TypeRef::Named(n)
                if !opaque_types.contains(n.as_str())
                    && !enum_names.contains(n.as_str())
                    && !extendr_incompatible_types.contains(n.as_str()));
        let needs_json_struct = !needs_json_enum
            && !needs_by_ref_struct
            && (is_named_incompatible
                || (matches!(&param.ty, TypeRef::Named(n)
                    if !opaque_types.contains(n.as_str())
                        && !enum_names.contains(n.as_str())
                        && !extendr_incompatible_types.contains(n.as_str()))
                    || matches!(&param.ty, TypeRef::Optional(inner)
                    if matches!(inner.as_ref(), TypeRef::Named(n)
                        if !opaque_types.contains(n.as_str())
                            && !enum_names.contains(n.as_str()))))
                    && (param.optional || !cfg.named_non_opaque_params_by_ref || return_type_requires_json));
        if !needs_json && !needs_json_struct && !needs_json_enum {
            if let TypeRef::Named(n) = &param.ty {
                if !opaque_types.contains(n.as_str()) {
                    if param.optional {
                        named_let_bindings.push_str(&template_env::render(
                            "named_let_optional_binding.jinja",
                            minijinja::context! {
                                name => &param.name,
                                ci => core_import,
                                n => n,
                            },
                        ));
                        named_let_bindings.push_str("    ");
                    } else {
                        named_let_bindings.push_str(&template_env::render(
                            "named_let_required_binding.jinja",
                            minijinja::context! {
                                name => &param.name,
                                ci => core_import,
                                n => n,
                            },
                        ));
                        named_let_bindings.push_str("    ");
                    }
                }
            }
        }
    }

    let final_call_args: Vec<String> = func
        .params
        .iter()
        .map(|param| {
            let needs_json = match &param.ty {
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(_n) => true,
                    _ => false,
                },
                _ => false,
            };
            let needs_json_enum = matches!(&param.ty, TypeRef::Named(n)
                if enum_names.contains(n.as_str()))
                || matches!(&param.ty, TypeRef::Optional(inner)
                    if matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n.as_str())));
            let is_named_incompatible = matches!(&param.ty, TypeRef::Named(n)
                if extendr_incompatible_types.contains(n.as_str()));
            let needs_json_struct = !needs_json_enum
                && (is_named_incompatible
                    || (matches!(&param.ty, TypeRef::Named(n)
                        if !opaque_types.contains(n.as_str())
                            && !enum_names.contains(n.as_str())
                            && !extendr_incompatible_types.contains(n.as_str()))
                        || matches!(&param.ty, TypeRef::Optional(inner)
                        if matches!(inner.as_ref(), TypeRef::Named(n)
                            if !opaque_types.contains(n.as_str())
                                && !enum_names.contains(n.as_str())
                                && !extendr_incompatible_types.contains(n.as_str()))))
                        && (param.optional || !cfg.named_non_opaque_params_by_ref || return_type_requires_json));
            if needs_json {
                if param.optional {
                    format!("{}_core.as_deref().unwrap_or_default()", param.name)
                } else if param.is_mut {
                    format!("&mut {}_core", param.name)
                } else if param.is_ref {
                    format!("&{}_core", param.name)
                } else {
                    format!("{}_core", param.name)
                }
            } else if needs_json_struct || needs_json_enum {
                if param.optional && param.is_ref {
                    format!("{}_core.as_ref()", param.name)
                } else if param.optional {
                    format!("{}_core", param.name)
                } else if param.is_mut {
                    format!("&mut {}_core", param.name)
                } else if param.is_ref {
                    format!("&{}_core", param.name)
                } else {
                    format!("{}_core", param.name)
                }
            } else if matches!(&param.ty, TypeRef::Named(n) if !opaque_types.contains(n.as_str())) {
                if cfg.named_non_opaque_params_by_ref && !param.optional {
                    if param.is_ref {
                        format!("&{}_core", param.name)
                    } else {
                        format!("{}_core", param.name)
                    }
                } else if param.optional {
                    if param.is_ref {
                        format!("{}_core.as_ref()", param.name)
                    } else {
                        format!("{}_core", param.name)
                    }
                } else if param.is_mut {
                    format!("&mut {}_core", param.name)
                } else if param.is_ref {
                    format!("&{}_core", param.name)
                } else {
                    format!("{}_core", param.name)
                }
            } else {
                gen_call_args_cfg(
                    std::slice::from_ref(param),
                    opaque_types,
                    cfg.cast_uints_to_i32,
                    cfg.cast_large_ints_to_f64,
                )
            }
        })
        .collect();
    let final_call_args_str = final_call_args.join(", ");

    let params_need_json_deserialize = func.params.iter().any(|p| match &p.ty {
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(_n) => true,
            _ => false,
        },
        TypeRef::Named(n) => {
            (enum_names.contains(n.as_str())
                || extendr_incompatible_types.contains(n.as_str())
                || !opaque_types.contains(n.as_str()))
                && (p.optional
                    || !cfg.named_non_opaque_params_by_ref
                    || enum_names.contains(n.as_str())
                    || extendr_incompatible_types.contains(n.as_str()))
        }
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n)
            if (enum_names.contains(n.as_str())
                || extendr_incompatible_types.contains(n.as_str())
                || !opaque_types.contains(n.as_str()))),
        _ => false,
    });
    let effectively_fallible = func.error_type.is_some() || params_need_json_deserialize;

    let (ret_type, result_convert, wrap_in_result_closure) = match &func.return_type {
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            if effectively_fallible {
                let ser = format!(
                    "result.map(|v| serde_json::to_string(&v){err_map}).transpose()",
                    err_map = err_map
                );
                ("Option<String>".to_string(), ser, true)
            } else {
                let ser = "result.map(|v| serde_json::to_string(&v).expect(\"serialization failed\"))".to_string();
                ("Option<String>".to_string(), ser, false)
            }
        }
        _ => {
            if effectively_fallible {
                let ser = format!("serde_json::to_string(&result){err_map}");
                ("String".to_string(), ser, true)
            } else {
                (
                    "String".to_string(),
                    "serde_json::to_string(&result).expect(\"serialization failed\")".to_string(),
                    false,
                )
            }
        }
    };

    let binding_conversion: Option<String> = match &func.return_type {
        TypeRef::Named(n) if extendr_incompatible_types.contains(n.as_str()) => {
            Some(format!("let result: {n} = result.into();"))
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) if extendr_incompatible_types.contains(n.as_str()) => Some(format!(
                "let result: Vec<{n}> = result.into_iter().map(Into::into).collect();"
            )),
            _ => None,
        },
        _ => None,
    };
    let convert = binding_conversion.as_deref().unwrap_or("");

    let core_call = format!("{core_fn_path}({final_call_args_str})");

    let core_call_with_err = if func.error_type.is_some() {
        format!("{core_call}{err_map}?")
    } else {
        core_call.clone()
    };

    let inner_body = if func.is_async {
        if func.error_type.is_some() {
            format!(
                "{body_preamble}{named_let_bindings}\
                 let rt = {rt_new};\n    \
                 let result = rt.block_on(async {{ {core_call}.await{err_map} }})?;\n    \
                 {convert}\n    \
                 {result_convert}",
                body_preamble = body_preamble,
                named_let_bindings = named_let_bindings,
                rt_new = rt_new,
                core_call = core_call,
                err_map = err_map,
                convert = convert,
                result_convert = result_convert,
            )
        } else {
            format!(
                "{body_preamble}{named_let_bindings}\
                 let rt = {rt_new};\n    \
                 let result = rt.block_on(async {{ {core_call}.await }});\n    \
                 {convert}\n    \
                 {result_convert}",
                body_preamble = body_preamble,
                named_let_bindings = named_let_bindings,
                rt_new = rt_new,
                core_call = core_call,
                convert = convert,
                result_convert = result_convert,
            )
        }
    } else {
        format!(
            "{body_preamble}{named_let_bindings}\
             let result = {core_call_with_err};\n    \
             {convert}\n    \
             {result_convert}",
            body_preamble = body_preamble,
            named_let_bindings = named_let_bindings,
            core_call_with_err = core_call_with_err,
            convert = convert,
            result_convert = result_convert,
        )
    };

    let body = if wrap_in_result_closure {
        let closure_ret_type = match &func.return_type {
            TypeRef::Optional(_) => "Result<Option<String>>".to_string(),
            _ => "Result<String>".to_string(),
        };
        format!(
            "match (|| -> {closure_ret_type} {{ {} }})() {{\n    \
             Ok(v) => v,\n    \
             Err(e) => extendr_api::throw_r_error(&format!(\"{{:?}}\", e)),\n    \
             }}",
            inner_body,
            closure_ret_type = closure_ret_type
        )
    } else {
        inner_body
    };

    let params_str = sig_params.join(", ");
    let allow = if effectively_fallible {
        "#[allow(clippy::missing_errors_doc)]\n"
    } else {
        ""
    };
    format!(
        "{allow}#[extendr]\npub fn {}({params_str}) -> {ret_type} {{\n    {body}\n}}",
        func.name
    )
}
