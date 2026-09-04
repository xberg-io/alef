use crate::codegen::doc_emission::doc_first_paragraph_joined;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, TypeDef};
use ahash::{AHashMap, AHashSet};
use heck::ToSnakeCase;

use super::helper_type_mapping::classify_param_type;
use super::return_error::emit_function_return_call;
use super::signature_params::emit_param_conversion;
use crate::backends::pyo3::gen_bindings::enums::{Wrapping, sanitize_python_doc};
use crate::backends::pyo3::py_signature::{leaf_named_type, python_signature_params};

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
    options_publishable_return_types: &std::collections::HashSet<String>,
) {
    for func in &api.functions {
        if exclude_functions.contains(&func.name) {
            continue;
        }
        // The facade can construct a `Default` for a param the caller omits, so unlike the `.pyi`
        // stub it grants those params a `= None`. Both go through the shared decision so the two
        // artifacts can never disagree about which params exist or in what order.
        //
        // `default_types` is wider than "has a usable no-argument constructor": it is unioned
        // with `options_dataclass_types` (see `orchestration.rs`'s doc on `default_types`) so a
        // type reachable only as a *required* nested field -- no core `Default` impl of its own,
        // e.g. `ChunkClassificationConfig { definitions, llm, batch_size, max_concurrency, .. }`
        // -- is still a member. Granting such a param a facade `= None` promises a fallback the
        // facade cannot deliver: the only synthesizable "default" is `_rust.{Type}()`, and that
        // bare call raises `TypeError: missing N required positional arguments` the instant the
        // caller actually omits the argument. Gating on `has_default` restricts the promise to
        // types alef knows have a real Rust `Default` impl (a true no-argument construction),
        // which is the only case `_rust.{Type}()` below can honour; every other default_types
        // member still gets its `_to_rust_*` conversion (it is still in `default_types`), it just
        // does not get a facade-invented default -- it stays required, matching the `.pyi` stub
        // and the native constructor exactly. ~keep
        let signature = python_signature_params(&func.params, |param| {
            !bridge_param_names.contains(param.name.as_str())
                && leaf_named_type(param).is_some_and(|name| default_types.get(name).is_some_and(|t| t.has_default))
        });
        let promoted_params: ahash::AHashSet<&str> = signature
            .iter()
            .filter(|entry| entry.defaulted && !entry.param.optional)
            .map(|entry| entry.param.name.as_str())
            .collect();

        let mut sig_parts = Vec::new();
        for entry in &signature {
            let param = entry.param;
            let base_type = if bridge_param_names.contains(param.name.as_str()) {
                "object".to_string()
            } else {
                crate::backends::pyo3::type_map::python_type(&param.ty)
            };
            let py_type = if entry.defaulted {
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

        // `_rust.` is the private extension module. Prefixing it is right only while the public
        // package has no type of its own under that name -- which is a question for the emitter
        // that writes `options.py`, not a rule to restate here. `options_publishable_return_types`
        // is the union of the public *input* dataclasses and the return-only `TypedDict`s
        // (mirrors `orchestration.rs`'s `options_publishable_return_types` doc) -- a plain
        // function's return type is routinely a dataclass `options.py` also accepts as a param
        // elsewhere, not only a return-only `TypedDict`, and checking the narrower TypedDict-only
        // set alone left that shape's `-> ReturnType` annotation naming the public dataclass while
        // the body handed back the untouched native pyclass. ~keep
        let return_leaf = match &func.return_type {
            crate::core::ir::TypeRef::Named(name) => Some(name.as_str()),
            crate::core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
                crate::core::ir::TypeRef::Named(name) => Some(name.as_str()),
                _ => None,
            },
            _ => None,
        };
        let public_return_leaf = return_leaf.filter(|name| options_publishable_return_types.contains(*name));
        let needs_native_prefix = return_leaf.is_some_and(|name| {
            return_type_names.contains(name) && !reexported_names.contains(name) && public_return_leaf.is_none()
        });

        let mut return_type_str = crate::backends::pyo3::type_map::python_type(&func.return_type);
        if needs_native_prefix {
            return_type_str = match return_type_str.strip_suffix(" | None") {
                Some(base) => format!("_rust.{base} | None"),
                None => format!("_rust.{return_type_str}"),
            };
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
        for entry in &signature {
            let param = entry.param;
            let class = classify_param_type(&param.ty);

            if let Some((name, wrapping)) = class {
                let pname = &param.name;
                let var = format!("_rust_{pname}");
                let is_promoted = promoted_params.contains(pname.as_str());
                let optional =
                    matches!(wrapping, Wrapping::Optional | Wrapping::OptionalVec) || param.optional || is_promoted;
                let is_collection = matches!(wrapping, Wrapping::Vec | Wrapping::OptionalVec);

                if default_types.contains_key(name) {
                    // `_rust.{name}()` (below, and in `config_default_on_none.jinja`) is only a
                    // safe fallback when alef knows `{name}` has a real Rust `Default` impl --
                    // see the doc on `default_types.get(name)` above the `signature` binding.
                    // Every other `default_types` member (e.g. `ChunkClassificationConfig`,
                    // reachable only through the `options_dataclass_types` closure) has genuinely
                    // required fields with no sensible zero-argument construction, so on a `None`
                    // this branch passes `None` straight through instead of fabricating an
                    // instance the native constructor would reject with a `TypeError`. ~keep
                    let type_has_default = default_types.get(name).is_some_and(|t| t.has_default);
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
                        if bridge_optional && type_has_default {
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
                            emit_param_conversion(out, &var, pname, &scalar_expr, bridge_optional);
                        }
                        if !param.optional && !is_promoted && !is_collection && type_has_default {
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

        let return_converter =
            public_return_leaf.map(crate::backends::pyo3::gen_bindings::types::from_native_converter_name);

        emit_function_return_call(
            out,
            &func.return_type,
            capsule_types,
            return_prefix,
            &func.name,
            &kwargs,
            return_converter.as_deref(),
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
    // the facade only when PyO3 actually wrote them: the bridge must target this language and
    // (for `register_fn`) carry `registry_getter` too — see `bridge_register_symbol`. ~keep
    for bridge in trait_bridges {
        if crate::backends::pyo3::trait_bridge::active_bridge_trait(bridge, api).is_none() {
            continue;
        }
        let Some(register_fn) = crate::codegen::generators::trait_bridge::bridge_register_symbol(bridge) else {
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

    for unregister_fn in crate::backends::pyo3::trait_bridge::collect_bridge_unregister_fns(trait_bridges, api) {
        if emitted_function_names.contains(&unregister_fn) {
            continue;
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "bridge_unregister_fn.jinja",
            minijinja::context! { unregister_fn => &unregister_fn },
        ));
    }

    for clear_fn in crate::backends::pyo3::trait_bridge::collect_bridge_clear_fns(trait_bridges, api) {
        if emitted_function_names.contains(&clear_fn) {
            continue;
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "bridge_clear_fn.jinja",
            minijinja::context! { clear_fn => &clear_fn },
        ));
    }
}
