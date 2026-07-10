use crate::codegen::doc_emission::doc_first_paragraph_joined;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, TypeDef};
use ahash::{AHashMap, AHashSet};
use heck::ToSnakeCase;

use super::helper_type_mapping::classify_param_type;
use super::return_error::emit_function_return_call;
use super::signature_params::emit_param_conversion;
use crate::backends::pyo3::gen_bindings::enums::{Wrapping, sanitize_python_doc};

type OptionsFieldBridges<'a> = AHashMap<&'a str, (&'a str, &'a str, Option<&'a str>)>;

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_function_wrappers(
    out: &mut String,
    api: &ApiSurface,
    trait_bridges: &[TraitBridgeConfig],
    capsule_types: &std::collections::HashMap<String, crate::core::config::CapsuleTypeConfig>,
    exclude_functions: &AHashSet<String>,
    bridge_param_names: &AHashSet<&str>,
    options_field_bridges: &OptionsFieldBridges<'_>,
    default_types: &AHashMap<String, &TypeDef>,
    data_enum_names: &AHashSet<&str>,
    return_type_names: &AHashSet<String>,
    reexported_names: &AHashSet<&str>,
) {
    for func in &api.functions {
        if exclude_functions.contains(&func.name) {
            continue;
        }
        let mut seen_optional_so_far = false;
        let mut promoted_params: ahash::AHashSet<String> = ahash::AHashSet::new();
        for param in &func.params {
            if param.optional {
                seen_optional_so_far = true;
            } else if seen_optional_so_far {
                promoted_params.insert(param.name.clone());
            }
        }
        for param in &func.params {
            if !param.optional
                && !promoted_params.contains(&param.name)
                && !bridge_param_names.contains(param.name.as_str())
            {
                let leaf_name = match &param.ty {
                    crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                    crate::core::ir::TypeRef::Optional(inner) => {
                        if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
                            Some(n.as_str())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if leaf_name.is_some_and(|n| default_types.contains_key(n)) {
                    promoted_params.insert(param.name.clone());
                }
            }
        }

        let mut sig_parts = Vec::new();
        let is_with_default = |p: &&crate::core::ir::ParamDef| p.optional || promoted_params.contains(&p.name);
        let (required, optional): (Vec<_>, Vec<_>) = func.params.iter().partition(|p| !is_with_default(p));
        for param in required.iter().chain(optional.iter()) {
            let base_type = if bridge_param_names.contains(param.name.as_str()) {
                "object".to_string()
            } else {
                crate::backends::pyo3::type_map::python_type(&param.ty)
            };
            let needs_default = param.optional || promoted_params.contains(&param.name);
            // calls `.expect("'param' is required")`.
            let is_has_default_param = !bridge_param_names.contains(param.name.as_str()) && {
                let leaf_name = match &param.ty {
                    crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                    crate::core::ir::TypeRef::Optional(inner) => {
                        if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
                            Some(n.as_str())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                leaf_name.is_some_and(|n| default_types.contains_key(n))
            };
            let py_type = if needs_default || is_has_default_param {
                if base_type.ends_with("| None") {
                    format!("{} = None", base_type)
                } else {
                    format!("{} | None = None", base_type)
                }
            } else {
                base_type
            };
            sig_parts.push(format!("{}: {}", param.name, py_type));
        }

        let options_field_visitor_kwarg: Option<(&str, &str, &str, Option<&str>)> = func.params.iter().find_map(|p| {
            let type_name = match &p.ty {
                crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                crate::core::ir::TypeRef::Optional(inner) => {
                    if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            }?;
            let (kwarg_name, _field_name, type_alias) = options_field_bridges.get(type_name)?;
            Some((p.name.as_str(), type_name, *kwarg_name, *type_alias))
        });
        if let Some((_, _, kwarg_name, type_alias)) = options_field_visitor_kwarg {
            let visitor_type = type_alias.unwrap_or("object");
            sig_parts.push(format!("{kwarg_name}: {visitor_type} | None = None"));
        }

        let mut return_type_str = crate::backends::pyo3::type_map::python_type(&func.return_type);
        if let crate::core::ir::TypeRef::Named(name) = &func.return_type {
            if return_type_names.contains(name) && !reexported_names.contains(name.as_str()) {
                return_type_str = format!("_rust.{return_type_str}");
            }
        } else if let crate::core::ir::TypeRef::Optional(inner) = &func.return_type {
            if let crate::core::ir::TypeRef::Named(name) = inner.as_ref() {
                if return_type_names.contains(name) && !reexported_names.contains(name.as_str()) {
                    if let Some(base) = return_type_str.strip_suffix(" | None") {
                        return_type_str = format!("_rust.{} | None", base);
                    }
                }
            }
        }
        let def_keyword = if func.is_async { "async def" } else { "def" };
        let has_builtin_param = sig_parts.iter().any(|p| {
            crate::backends::pyo3::gen_stubs::is_python_builtin_name(p.split(':').next().unwrap_or("").trim())
        });
        let single_line = format!(
            "{def_keyword} {}({}) -> {}:\n",
            func.name,
            sig_parts.join(", "),
            return_type_str
        );
        if single_line.len() <= 100 && !has_builtin_param {
            out.push_str(&crate::backends::pyo3::template_env::render(
                "function_signature_single_line.jinja",
                minijinja::context! {
                    def_keyword => def_keyword,
                    name => &func.name,
                    params => sig_parts.join(", "),
                    return_type => &return_type_str,
                },
            ));
        } else {
            out.push_str(&crate::backends::pyo3::template_env::render(
                "function_signature_multiline_start.jinja",
                minijinja::context! {
                    def_keyword => def_keyword,
                    name => &func.name,
                },
            ));
            for param in &sig_parts {
                let name = param.split(':').next().unwrap_or("").trim();
                if crate::backends::pyo3::gen_stubs::is_python_builtin_name(name) {
                    out.push_str(&crate::backends::pyo3::template_env::render(
                        "function_signature_multiline_param_noqa.jinja",
                        minijinja::context! { param => param },
                    ));
                } else {
                    out.push_str(&crate::backends::pyo3::template_env::render(
                        "function_signature_multiline_param.jinja",
                        minijinja::context! { param => param },
                    ));
                }
            }
            out.push_str(&crate::backends::pyo3::template_env::render(
                "function_signature_multiline_end.jinja",
                minijinja::context! { return_type => &return_type_str },
            ));
        }
        {
            let doc_with_period = if !func.doc.is_empty() {
                let doc_first_para = doc_first_paragraph_joined(&func.doc);
                let doc_sanitized = sanitize_python_doc(&doc_first_para);
                let doc_content = if doc_sanitized.len() > 89 {
                    doc_sanitized[..89].to_string()
                } else {
                    doc_sanitized
                };
                if doc_content.ends_with('.') {
                    doc_content
                } else {
                    format!("{}.", doc_content)
                }
            } else {
                use heck::ToSnakeCase;
                let snake = func.name.to_snake_case();
                let sentence = snake.replace('_', " ");
                let mut chars = sentence.chars();
                let capitalized = match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                };
                format!("{}.", capitalized)
            };
            out.push_str(&crate::backends::pyo3::template_env::render(
                "function_docstring.jinja",
                minijinja::context! { doc => &doc_with_period },
            ));
        }

        let mut call_args: Vec<(String, String)> = Vec::new();
        let (req_params, opt_params): (Vec<_>, Vec<_>) = func.params.iter().partition(|p| !is_with_default(p));
        for param in req_params.iter().chain(opt_params.iter()) {
            let class = classify_param_type(&param.ty);

            if let Some((name, wrapping)) = class {
                let pname = &param.name;
                let var = format!("_rust_{pname}");
                let is_promoted = promoted_params.contains(pname.as_str());
                let optional =
                    matches!(wrapping, Wrapping::Optional | Wrapping::OptionalVec) || param.optional || is_promoted;
                let is_collection = matches!(wrapping, Wrapping::Vec | Wrapping::OptionalVec);

                if default_types.contains_key(name) {
                    let snake = name.to_snake_case();
                    let scalar_expr = if options_field_bridges.contains_key(name) {
                        if let Some((_, _, kwarg_name, _)) = options_field_visitor_kwarg {
                            format!("_to_rust_{snake}({pname}, _visitor_override={kwarg_name})")
                        } else {
                            format!("_to_rust_{snake}({pname})")
                        }
                    } else {
                        format!("_to_rust_{snake}({pname})")
                    };
                    if is_collection {
                        let element_expr = format!("_to_rust_{snake}(__item)");
                        let body = format!("[{element_expr} for __item in {pname}]");
                        emit_param_conversion(out, &var, pname, &body, optional);
                    } else {
                        let bridge_optional = optional
                            && !(options_field_bridges.contains_key(name) && options_field_visitor_kwarg.is_some());
                        if bridge_optional {
                            // `.expect("'config' is required")`).
                            out.push_str(&crate::backends::pyo3::template_env::render(
                                "config_conversion_ternary.jinja",
                                minijinja::context! {
                                    var => &var,
                                    body => &scalar_expr,
                                    pname => pname,
                                    name => name,
                                },
                            ));
                        } else {
                            emit_param_conversion(out, &var, pname, &scalar_expr, false);
                        }
                        if !param.optional && !is_promoted && !is_collection {
                            out.push_str(&crate::backends::pyo3::template_env::render(
                                "config_default_on_none.jinja",
                                minijinja::context! {
                                    var => &var,
                                    name => name,
                                },
                            ));
                        }
                    }
                    call_args.push((pname.clone(), var));
                    continue;
                }
                if data_enum_names.contains(name) {
                    let scalar_expr =
                        format!("(_rust.{name}({pname}) if not isinstance({pname}, _rust.{name}) else {pname})");
                    if is_collection {
                        let element_expr =
                            format!("(_rust.{name}(__item) if not isinstance(__item, _rust.{name}) else __item)");
                        let body = format!("[{element_expr} for __item in {pname}]");
                        emit_param_conversion(out, &var, pname, &body, optional);
                    } else {
                        emit_param_conversion(out, &var, pname, &scalar_expr, optional);
                    }
                    call_args.push((pname.clone(), var));
                    continue;
                }
            }
            call_args.push((param.name.clone(), param.name.clone()));
        }

        if let Some((_, _, kwarg_name, _)) = options_field_visitor_kwarg {
            call_args.push((kwarg_name.to_string(), kwarg_name.to_string()));
        }

        let kwargs: Vec<String> = call_args.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let return_prefix = if func.is_async { "await " } else { "" };

        emit_function_return_call(
            out,
            &func.return_type,
            capsule_types,
            return_prefix,
            &func.name,
            &kwargs,
        );
        out.push_str("\n\n");
    }

    let emitted_function_names: AHashSet<String> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(&f.name))
        .map(|f| f.name.clone())
        .collect();

    // These functions are emitted as #[pyfunction] in the native Rust module but are not in
    for bridge in trait_bridges {
        let Some(register_fn) = bridge.register_fn.as_deref() else {
            continue;
        };
        if emitted_function_names.contains(register_fn) {
            continue;
        }
        let backend_type = if api.types.iter().any(|t| t.name == bridge.trait_name) {
            bridge.trait_name.as_str()
        } else {
            "object"
        };
        out.push_str(&crate::backends::pyo3::template_env::render(
            "bridge_register_fn.jinja",
            minijinja::context! { register_fn => register_fn, backend_type => backend_type },
        ));
    }

    for unregister_fn in crate::backends::pyo3::trait_bridge::collect_bridge_unregister_fns(trait_bridges) {
        if emitted_function_names.contains(&unregister_fn) {
            continue;
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "bridge_unregister_fn.jinja",
            minijinja::context! { unregister_fn => &unregister_fn },
        ));
    }

    for clear_fn in crate::backends::pyo3::trait_bridge::collect_bridge_clear_fns(trait_bridges) {
        if emitted_function_names.contains(&clear_fn) {
            continue;
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "bridge_clear_fn.jinja",
            minijinja::context! { clear_fn => &clear_fn },
        ));
    }
}
