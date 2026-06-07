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
    // Generate wrapper for each function
    for func in &api.functions {
        // Skip functions explicitly excluded for this language backend. Excluded functions
        // are absent from the native Rust module (lib.rs), so generating an api.py wrapper
        // that calls `_rust.<name>` would produce an AttributeError at runtime.
        if exclude_functions.contains(&func.name) {
            continue;
        }
        // Build Python-side params applying seen_optional promotion.
        //
        // Python syntax requires params with defaults to follow params without defaults.
        // The PyO3 binding uses seen_optional promotion: once any optional param appears
        // in the Rust function signature, all subsequent params also get `= None` defaults
        // (wrapped in Option<T>). The Python wrapper must mirror this so callers can omit
        // those trailing params.
        //
        // Algorithm:
        //   1. Walk params in IR order, track seen_optional.
        //   2. A param is "promoted" if it is NOT optional in the IR but seen_optional is
        //      already true (an earlier param was optional).
        //   3. Partition into truly-required (not optional, not promoted) and
        //      all-with-defaults (optional || promoted).
        //   4. Emit truly-required first, then all-with-defaults — satisfying Python syntax.
        let mut seen_optional_so_far = false;
        let mut promoted_params: ahash::AHashSet<String> = ahash::AHashSet::new();
        for param in &func.params {
            if param.optional {
                seen_optional_so_far = true;
            } else if seen_optional_so_far {
                // This param is not optional in the IR but comes after an optional param
                // → the PyO3 binding promotes it to Option<T>; the Python wrapper must too.
                promoted_params.insert(param.name.clone());
            }
        }
        // Params whose type is a has-default struct (e.g. ExtractionResult) are given
        // `| None = None` defaults in the Python wrapper even when not IR-optional.
        // They must appear AFTER truly-required params to avoid a Python SyntaxError
        // ("non-default argument follows default argument"), so treat them as promoted.
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
            // Bridge params have their IR type sanitized to String, but callers pass
            // arbitrary Python objects implementing the visitor protocol — use `object`.
            let base_type = if bridge_param_names.contains(param.name.as_str()) {
                "object".to_string()
            } else {
                crate::backends::pyo3::type_map::python_type(&param.ty)
            };
            let needs_default = param.optional || promoted_params.contains(&param.name);
            // Required params whose type is a has-default struct are treated as optional
            // at the Python wrapper level: callers may omit them and the wrapper substitutes
            // a Rust default-constructed instance (e.g. `_rust.ExtractionConfig()`).
            // This prevents panics in the PyO3 binding when `None` is passed to a
            // function whose Rust signature wraps the param in `Option<T>` but immediately
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

        // Detect if this function has an options-field bridge (visitor embedded in options).
        // When it does, add a convenience `visitor: {type_alias} | None = None` kwarg.
        // We track: (options_param_name, options_type_name, visitor_kwarg_name, type_alias).
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
        // If the return type is marked is_return_type, it lives in the native module, not .options.
        // Qualify it with _rust. so the annotation matches where it's imported from, UNLESS
        // the type is in reexported_types (re-exported in the public __init__.py).
        // Handle Optional return types: _rust.Type | None, not (_rust.Type) | None.
        if let crate::core::ir::TypeRef::Named(name) = &func.return_type {
            if return_type_names.contains(name) && !reexported_names.contains(name.as_str()) {
                return_type_str = format!("_rust.{return_type_str}");
            }
        } else if let crate::core::ir::TypeRef::Optional(inner) = &func.return_type {
            if let crate::core::ir::TypeRef::Named(name) = inner.as_ref() {
                if return_type_names.contains(name) && !reexported_names.contains(name.as_str()) {
                    // Replace "Type | None" with "_rust.Type | None"
                    if let Some(base) = return_type_str.strip_suffix(" | None") {
                        return_type_str = format!("_rust.{} | None", base);
                    }
                }
            }
        }
        // Async pyo3 functions return a coroutine — the Python wrapper must be `async def`
        // so that `result = await fn(...)` works correctly and type checkers see the right type.
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
                // `    """..."""` is 10 chars of overhead; period may add 1 more char.
                // Limit content to 89 chars so that with a trailing period the full line stays ≤100.
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

        // For each param that has a converter, emit a local conversion variable.
        // Use the same required-first, optional-last order as the Python signature so that
        // positional calls to the native function match the pyo3 signature declaration.
        //
        // We classify the param's type by unwrapping `Optional`/`Vec` layers down to the
        // leaf `Named` type. The classification determines whether a scalar conversion or
        // a list-comprehension conversion is generated.
        // Each entry is (param_name, value_expr) — used to build keyword-argument calls so
        // that the generated `_rust.fn(path=path, config=_rust_config, ...)` form is
        // independent of the pyo3 signature parameter order.
        let mut call_args: Vec<(String, String)> = Vec::new();
        let (req_params, opt_params): (Vec<_>, Vec<_>) = func.params.iter().partition(|p| !is_with_default(p));
        for param in req_params.iter().chain(opt_params.iter()) {
            let class = classify_param_type(&param.ty);

            if let Some((name, wrapping)) = class {
                let pname = &param.name;
                let var = format!("_rust_{pname}");
                // A param is "optional" for the conversion guard when:
                //   - its IR type is Optional/OptionalVec, OR
                //   - the IR param itself is optional, OR
                //   - it was promoted to optional via seen_optional (comes after an optional param).
                let is_promoted = promoted_params.contains(pname.as_str());
                let optional =
                    matches!(wrapping, Wrapping::Optional | Wrapping::OptionalVec) || param.optional || is_promoted;
                let is_collection = matches!(wrapping, Wrapping::Vec | Wrapping::OptionalVec);

                // has_default struct: Python-side conversion via _to_rust_<snake>().
                if default_types.contains_key(name) {
                    let snake = name.to_snake_case();
                    // When this param is the options param of an options-field bridge, pass the
                    // visitor kwarg name as _visitor_override so the converter injects it.
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
                        // When this param is the options param of an options-field bridge, the
                        // converter handles all None cases itself — emit an unconditional call
                        // so that `visitor=visitor` is forwarded even when `options is None`.
                        let bridge_optional = optional
                            && !(options_field_bridges.contains_key(name) && options_field_visitor_kwarg.is_some());
                        if bridge_optional {
                            // Optional has-default param: use Rust default constructor when None
                            // instead of passing None to the Rust binding (which may panic on
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
                        // Required scalar (not optional and not promoted): when the converter
                        // returns None (caller passed None for a required param), substitute the
                        // Rust default constructor instead of raising ValueError.  This lets
                        // callers omit the config argument naturally.
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
                // Data enum (tagged union): wrap with `_rust.<EnumName>(value)` if not already.
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

        // Bridge `bind_via = "options_field"`: the Rust function has an additional visitor
        // kwarg (appended by gen_bridge_field_function) that is NOT in `func.params`. The
        // python wrapper takes a convenience `visitor=` kwarg and stuffs it into options
        // via `_visitor_override`, but the Rust function body actually reads the explicit
        // kwarg — pass it through as well so the visitor handle reaches the bridge.
        if let Some((_, _, kwarg_name, _)) = options_field_visitor_kwarg {
            call_args.push((kwarg_name.to_string(), kwarg_name.to_string()));
        }

        // Use keyword arguments so the call is independent of the pyo3 signature order.
        // This ensures wrapper-side required/optional reordering doesn't misalign slots.
        let kwargs: Vec<String> = call_args.iter().map(|(k, v)| format!("{k}={v}")).collect();
        // Async pyo3 functions return a coroutine that must be awaited by the Python caller.
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

    // Collect names already emitted in the main api.functions loop so we don't duplicate
    // a function that is both an api.function AND named in a trait-bridge config.
    // Trait bridges declare `clear_fn = "clear_text_backends"` etc.; that same function
    // is usually also configured as a regular call in [crates.e2e.calls.*] and ends up
    // in api.functions with a richer doc-comment from the Rust source. Without this guard,
    // api.py declares both — the second wins at import time but ruff flags F811.
    // Excluded functions were not emitted above; exclude them here too so a trait-bridge
    // registration function that shares the same name does get emitted.
    let emitted_function_names: AHashSet<String> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(&f.name))
        .map(|f| f.name.clone())
        .collect();

    // Emit pass-through wrappers for trait-bridge registration functions.
    // These functions are emitted as #[pyfunction] in the native Rust module but are not in
    // api.functions — they must be re-exported via api.py so callers can use the public package
    // path (e.g. `sample_core.register_text_backend`) rather than `sample_core._sample_core.register_text_backend`.
    for register_fn in crate::backends::pyo3::trait_bridge::collect_bridge_register_fns(trait_bridges) {
        if emitted_function_names.contains(&register_fn) {
            continue;
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "bridge_register_fn.jinja",
            minijinja::context! { register_fn => &register_fn },
        ));
    }

    // Emit pass-through wrappers for trait-bridge unregistration functions.
    // These allow callers to unregister a named backend via the public package path.
    for unregister_fn in crate::backends::pyo3::trait_bridge::collect_bridge_unregister_fns(trait_bridges) {
        if emitted_function_names.contains(&unregister_fn) {
            continue;
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "bridge_unregister_fn.jinja",
            minijinja::context! { unregister_fn => &unregister_fn },
        ));
    }

    // Emit pass-through wrappers for trait-bridge clear functions.
    // These allow callers to clear all registered backends for a plugin type.
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
