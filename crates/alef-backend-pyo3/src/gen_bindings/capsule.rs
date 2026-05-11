//! PyO3 capsule-type codegen: PyCapsule_New / PyCapsule_GetPointer wrappers.
//!
//! When `[crates.python.capsule_types]` is configured, types listed there are NOT
//! emitted as `#[pyclass]` opaque wrappers. Instead, functions that return or accept
//! these types get hand-crafted bodies that use the CPython PyCapsule API to pass raw
//! pointers through to the Python-side `tree_sitter` (or similar) package.
//!
//! Two flavors:
//!
//! 1. **Capsule round-trip** (`CapsuleTypeConfig::Capsule(name)`)
//!    The Rust type has `into_raw()` and (implicitly) `from_raw()`. On return, we call
//!    `PyCapsule_New(value.into_raw(), name, None)`. On input, we call
//!    `PyCapsule_GetPointer` + `from_raw()`.
//!
//! 2. **Python-side construction** (`CapsuleTypeConfig::ConstructFrom { python_type, construct_from }`)
//!    The type has no `into_raw()`. The binding calls a Python factory that accepts the
//!    dependent capsule argument (e.g. `tree_sitter.Parser(language)`).

use alef_codegen::type_mapper::TypeMapper;
use alef_core::config::CapsuleTypeConfig;
use alef_core::ir::{FunctionDef, TypeRef};
use std::collections::HashMap;

/// Returns `true` when this function either returns a capsule type or has a capsule-typed parameter.
pub(super) fn function_involves_capsule(
    func: &FunctionDef,
    capsule_types: &HashMap<String, CapsuleTypeConfig>,
) -> bool {
    if return_type_name(func, capsule_types).is_some() {
        return true;
    }
    params_with_capsule(func, capsule_types).next().is_some()
}

/// Returns the capsule return type name if the function returns a capsule type.
pub(super) fn return_type_name<'a>(
    func: &'a FunctionDef,
    capsule_types: &'a HashMap<String, CapsuleTypeConfig>,
) -> Option<&'a str> {
    fn named_from_ref(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) => named_from_ref(inner),
            _ => None,
        }
    }
    let name = named_from_ref(&func.return_type)?;
    if capsule_types.contains_key(name) {
        Some(name)
    } else {
        None
    }
}

/// Returns an iterator of (param_name, type_name) for parameters whose type is a capsule type.
pub(super) fn params_with_capsule<'a>(
    func: &'a FunctionDef,
    capsule_types: &'a HashMap<String, CapsuleTypeConfig>,
) -> impl Iterator<Item = (&'a str, &'a str)> {
    func.params.iter().filter_map(|p| {
        let name = match &p.ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    Some(n.as_str())
                } else {
                    None
                }
            }
            _ => None,
        }?;
        if capsule_types.contains_key(name) {
            Some((p.name.as_str(), name))
        } else {
            None
        }
    })
}

/// Generate a custom `#[pyfunction]` for a function that involves capsule types.
///
/// This replaces the default `generators::gen_function` call for such functions.
pub(super) fn gen_capsule_function(
    func: &FunctionDef,
    capsule_types: &HashMap<String, CapsuleTypeConfig>,
    core_import: &str,
    error_converters: &[String],
) -> String {
    use heck::ToSnakeCase;

    let mapper = crate::type_map::Pyo3Mapper::new();

    let mut out = String::new();

    // Build the `#[pyfunction]` signature.
    // All non-capsule params are passed as-is; capsule params are accepted as `Py<PyAny>`.
    let mut sig_params: Vec<String> = Vec::new();
    // The function always needs `py: Python<'_>` to call into Python.
    sig_params.push("py: pyo3::Python<'_>".to_string());
    for param in &func.params {
        let type_str = match &param.ty {
            TypeRef::Named(n) if capsule_types.contains_key(n.as_str()) => {
                // Capsule params are received as Py<PyAny> from the Python side.
                "pyo3::Py<pyo3::PyAny>".to_string()
            }
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    if capsule_types.contains_key(n.as_str()) {
                        "Option<pyo3::Py<pyo3::PyAny>>".to_string()
                    } else {
                        mapper.map_type(inner)
                    }
                } else {
                    mapper.map_type(&param.ty)
                }
            }
            _ => mapper.map_type(&param.ty),
        };
        let opt = if param.optional {
            " = None".to_string()
        } else {
            String::new()
        };
        sig_params.push(format!("{}: {}{}", param.name, type_str, opt));
    }

    // Determine return type.
    let ret_capsule_name = return_type_name(func, capsule_types);
    let return_type_str = "pyo3::PyResult<pyo3::Py<pyo3::PyAny>>";

    out.push_str("#[pyo3::prelude::pyfunction]\n");

    // Build pyo3(signature = (...)) if there are optional params (beyond `py`).
    let has_optional = func.params.iter().any(|p| p.optional);
    if has_optional {
        let sig_names: Vec<String> = func
            .params
            .iter()
            .map(|p| {
                if p.optional {
                    format!("{} = None", p.name)
                } else {
                    p.name.clone()
                }
            })
            .collect();
        out.push_str(&crate::template_env::render(
            "pyo3_capsule_signature.jinja",
            minijinja::context! {
                sig => sig_names.join(", "),
            },
        ));
    }

    out.push_str(&crate::template_env::render(
        "pyo3_capsule_function_header.jinja",
        minijinja::context! {
            name => func.name.as_str(),
            params => sig_params.join(", "),
            ret => return_type_str,
        },
    ));

    // Emit input capsule extraction for each capsule-typed param.
    for param in &func.params {
        let type_name = match &param.ty {
            TypeRef::Named(n) if capsule_types.contains_key(n.as_str()) => Some((n.as_str(), false)),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    if capsule_types.contains_key(n.as_str()) {
                        Some((n.as_str(), true))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some((capsule_type_name, is_optional)) = type_name {
            let cfg = &capsule_types[capsule_type_name];
            if cfg.is_capsule_roundtrip() {
                let capsule_name_str = cfg.python_type();
                let capsule_cstr = capsule_name_str.replace('.', "_").to_ascii_uppercase();
                // Emit a local constant for the capsule name.
                out.push_str(&crate::template_env::render(
                    "pyo3_capsule_input_const.jinja",
                    minijinja::context! {
                        cstr => capsule_cstr.as_str(),
                        capsule_name => capsule_name_str,
                    },
                ));
                if is_optional {
                    out.push_str(&crate::template_env::render(
                        "pyo3_capsule_input_optional.jinja",
                        minijinja::context! {
                            param => param.name.as_str(),
                            cstr => capsule_cstr.as_str(),
                        },
                    ));
                } else {
                    out.push_str(&crate::template_env::render(
                        "pyo3_capsule_input_required.jinja",
                        minijinja::context! {
                            param => param.name.as_str(),
                            cstr => capsule_cstr.as_str(),
                            capsule_type_name => capsule_type_name,
                        },
                    ));
                }
            }
        }
    }

    // Build the core function call args.
    let core_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let type_name = match &p.ty {
                TypeRef::Named(n) if capsule_types.contains_key(n.as_str()) => Some((n.as_str(), false)),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        if capsule_types.contains_key(n.as_str()) {
                            Some((n.as_str(), true))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some((capsule_type_name, _is_optional)) = type_name {
                let cfg = &capsule_types[capsule_type_name];
                if cfg.is_capsule_roundtrip() {
                    // Pass the raw pointer reconstructed from capsule.
                    format!("{param}_raw", param = p.name)
                } else {
                    // ConstructFrom: pass the Py<PyAny> as-is (not passed to core).
                    p.name.clone()
                }
            } else {
                // For String/Char params marked as references (is_ref=true), the core function
                // expects `&str` — borrow the owned String rather than moving it.
                let needs_borrow = p.is_ref && matches!(p.ty, TypeRef::String | TypeRef::Char);
                if needs_borrow {
                    format!("&{}", p.name)
                } else {
                    p.name.clone()
                }
            }
        })
        .collect();

    // Emit the core call.
    let has_error = func.error_type.is_some();
    let core_fn_path = format!("{core_import}::{fn_name}", fn_name = func.name);

    if let Some(capsule_type_name) = ret_capsule_name {
        let cfg = &capsule_types[capsule_type_name];

        match cfg {
            CapsuleTypeConfig::Capsule(capsule_name_str) => {
                // Capsule round-trip: call into_raw() + PyCapsule_New.
                let capsule_cstr = capsule_name_str.replace('.', "_").to_ascii_uppercase();
                out.push_str(&crate::template_env::render(
                    "pyo3_capsule_input_const.jinja",
                    minijinja::context! {
                        cstr => capsule_cstr.as_str(),
                        capsule_name => capsule_name_str.as_str(),
                    },
                ));
                if has_error {
                    let err_converter = error_converter_name(&func.error_type, error_converters);
                    out.push_str(&crate::template_env::render(
                        "pyo3_capsule_call_result_err.jinja",
                        minijinja::context! {
                            target => "result",
                            core_fn_path => core_fn_path.as_str(),
                            args => core_args.join(", "),
                            err_converter => err_converter,
                        },
                    ));
                } else {
                    out.push_str(&crate::template_env::render(
                        "pyo3_capsule_call_result.jinja",
                        minijinja::context! {
                            target => "result",
                            core_fn_path => core_fn_path.as_str(),
                            args => core_args.join(", "),
                        },
                    ));
                }
                out.push_str(&crate::template_env::render(
                    "pyo3_capsule_into_raw.jinja",
                    minijinja::Value::default(),
                ));
                out.push_str(&crate::template_env::render(
                    "pyo3_capsule_ptr_from_raw.jinja",
                    minijinja::context! {
                        cstr => capsule_cstr.as_str(),
                    },
                ));
            }
            CapsuleTypeConfig::ConstructFrom {
                python_type,
                construct_from,
            } => {
                // Python-side construction: produce the capsule dependency first, then call Python factory.
                // The `construct_from` type must itself be a capsule-roundtrip type.
                let dep_capsule_name = capsule_types.get(construct_from.as_str()).and_then(|c| {
                    if c.is_capsule_roundtrip() {
                        Some(c.python_type())
                    } else {
                        None
                    }
                });

                // For ConstructFrom we deliberately skip calling the core Rust function —
                // its return type (e.g. tree_sitter::Parser) cannot be capsule-wrapped,
                // which is the whole reason ConstructFrom exists.  Calling the core fn and
                // then discarding the result would also double-consume owned String arguments.
                // The Python object is constructed entirely from the dependency capsule below.

                // We need to produce the dependent capsule type to pass to the Python constructor.
                // The core function returned `_result`, but for ConstructFrom the return is the
                // Rust type whose Python equivalent is built from a dependent arg.
                // Strategy: look for the `construct_from` dependency in the function's args.
                // If not present, we need to call the dependency function — this is only used
                // in practice where the function itself returns a `construct_from` type, meaning
                // the core function already returned the dependent-typed value.
                // In the tree-sitter case: get_parser() returns Parser, and Parser needs Language.
                // The convention: the core function `get_parser(name)` returns a tree_sitter::Parser
                // by calling get_language internally, so we call our own get_language bridge.
                if let Some(capsule_dep_name) = dep_capsule_name {
                    let _capsule_cstr = capsule_dep_name.replace('.', "_").to_ascii_uppercase();
                    // Find the function's own args that match the construct_from type.
                    // If the function signature itself takes a `construct_from`-typed arg, use it.
                    let dep_arg = func
                        .params
                        .iter()
                        .find(|p| matches!(&p.ty, TypeRef::Named(n) if n == construct_from));
                    let dep_expr = if let Some(arg) = dep_arg {
                        // The arg is already available as a `Py<PyAny>` (since it's a capsule param).
                        format!("{}.bind(py).clone()", arg.name)
                    } else {
                        // The dependency must be obtained from the result or from another helper.
                        // For the tree-sitter pattern, get_parser returns Parser which needs Language.
                        // We call `get_binding(py, name)` (the capsule-emitting variant of get_language).
                        // Since alef doesn't know about the dependency function name, we emit
                        // a call to `{snake_construct_from}_capsule` helper — callers need to define that.
                        // More practically: we look for a function in the module named `get_{snake_dep}` and call it.
                        let dep_snake = construct_from.to_snake_case();
                        // Call the capsule function for the dependency type.
                        // The generated module will have a `get_{dep_snake}` or similar; we call
                        // the dependency capsule-function by naming convention.
                        // For tree-sitter: the module has `get_language(py, name) -> Py<PyAny>`.
                        // We use: assume the first string param of this function is the key.
                        let first_str_param = func.params.iter().find(|p| matches!(p.ty, TypeRef::String));
                        if let Some(str_param) = first_str_param {
                            // Heuristic: call get_{dep_snake}(py, {str_param}).
                            format!("get_{dep_snake}(py, {param})?.bind(py).clone()", param = str_param.name)
                        } else {
                            // Fallback: emit a comment asking for manual disambiguation.
                            format!("/* TODO: obtain {construct_from} capsule */ unreachable!()")
                        }
                    };

                    // Parse the python_type into module + class.
                    out.push_str(&crate::template_env::render(
                        "pyo3_capsule_construct_comment.jinja",
                        minijinja::context! {
                            python_type => python_type.as_str(),
                        },
                    ));
                    if let Some((module_path, class_name)) = python_type.rsplit_once('.') {
                        out.push_str(&crate::template_env::render(
                            "pyo3_capsule_construct_with_module.jinja",
                            minijinja::context! {
                                dep_expr => dep_expr,
                                module_path => module_path,
                                class_name => class_name,
                            },
                        ));
                    } else {
                        // No dot in python_type — just call it as a builtin.
                        out.push_str(&crate::template_env::render(
                            "pyo3_capsule_construct_with_builtin.jinja",
                            minijinja::context! {
                                dep_expr => dep_expr,
                                python_type => python_type,
                            },
                        ));
                    }
                } else {
                    // No dep capsule name found — can't determine dependency.
                    out.push_str(&crate::template_env::render(
                        "pyo3_capsule_missing_dependency.jinja",
                        minijinja::Value::default(),
                    ));
                }
            }
        }
    } else {
        // No capsule return type — function has only capsule input params.
        // Emit a normal call but with capsule extraction for params already done above.
        if has_error {
            let err_converter = error_converter_name(&func.error_type, error_converters);
            out.push_str(&crate::template_env::render(
                "pyo3_capsule_call_result_err_inline.jinja",
                minijinja::context! {
                    core_fn_path => core_fn_path.as_str(),
                    args => core_args.join(", "),
                    err_converter => err_converter,
                },
            ));
        } else {
            // Unit or primitive return.
            out.push_str(&crate::template_env::render(
                "pyo3_capsule_call_no_capsule_return.jinja",
                minijinja::context! {
                    core_fn_path => core_fn_path.as_str(),
                    args => core_args.join(", "),
                },
            ));
        }
    }

    out.push_str("}\n\n");
    out
}

fn error_converter_name(error_type: &Option<String>, error_converters: &[String]) -> String {
    use heck::ToSnakeCase;
    if let Some(et) = error_type {
        // Check if there's a matching converter in the generated module.
        let short = et.split("::").last().unwrap_or(et.as_str());
        let candidate = format!("{}_to_py_err", short.to_snake_case());
        if error_converters.iter().any(|c| c == &candidate) {
            return format!("|e| {candidate}(e)");
        }
    }
    // Fallback: convert to PyRuntimeError.
    "|e| pyo3::exceptions::PyRuntimeError::new_err(format!(\"{e}\"))".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::CapsuleTypeConfig;
    use std::collections::HashMap;

    fn capsule_map(entries: &[(&str, CapsuleTypeConfig)]) -> HashMap<String, CapsuleTypeConfig> {
        entries.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    /// A function returning a capsule type is detected by return_type_name.
    #[test]
    fn return_type_name_detects_capsule_return() {
        use alef_core::ir::{FunctionDef, TypeRef};
        let func = FunctionDef {
            name: "get_language".to_string(),
            rust_path: "lib::get_language".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Named("Language".to_string()),
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        };
        let capsules = capsule_map(&[(
            "Language",
            CapsuleTypeConfig::Capsule("tree_sitter.Language".to_string()),
        )]);
        let result = return_type_name(&func, &capsules);
        assert_eq!(result, Some("Language"));
    }

    /// A function with a non-capsule return type returns None.
    #[test]
    fn return_type_name_returns_none_for_non_capsule() {
        use alef_core::ir::{FunctionDef, TypeRef};
        let func = FunctionDef {
            name: "get_name".to_string(),
            rust_path: "lib::get_name".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        };
        let capsules = capsule_map(&[(
            "Language",
            CapsuleTypeConfig::Capsule("tree_sitter.Language".to_string()),
        )]);
        let result = return_type_name(&func, &capsules);
        assert_eq!(result, None);
    }

    /// python_type() returns the capsule name for Capsule variant.
    #[test]
    fn python_type_returns_capsule_name() {
        let cfg = CapsuleTypeConfig::Capsule("tree_sitter.Language".to_string());
        assert_eq!(cfg.python_type(), "tree_sitter.Language");
    }

    /// python_type() returns python_type field for ConstructFrom variant.
    #[test]
    fn python_type_returns_construct_from_python_type() {
        let cfg = CapsuleTypeConfig::ConstructFrom {
            python_type: "tree_sitter.Parser".to_string(),
            construct_from: "Language".to_string(),
        };
        assert_eq!(cfg.python_type(), "tree_sitter.Parser");
    }

    /// construct_from() returns None for Capsule variant.
    #[test]
    fn construct_from_returns_none_for_capsule() {
        let cfg = CapsuleTypeConfig::Capsule("tree_sitter.Language".to_string());
        assert_eq!(cfg.construct_from(), None);
    }

    /// construct_from() returns the dependency name for ConstructFrom variant.
    #[test]
    fn construct_from_returns_dependency_name() {
        let cfg = CapsuleTypeConfig::ConstructFrom {
            python_type: "tree_sitter.Parser".to_string(),
            construct_from: "Language".to_string(),
        };
        assert_eq!(cfg.construct_from(), Some("Language"));
    }

    /// is_capsule_roundtrip() is true only for Capsule variant.
    #[test]
    fn is_capsule_roundtrip_discriminates_variants() {
        let capsule = CapsuleTypeConfig::Capsule("tree_sitter.Language".to_string());
        let construct = CapsuleTypeConfig::ConstructFrom {
            python_type: "tree_sitter.Parser".to_string(),
            construct_from: "Language".to_string(),
        };
        assert!(capsule.is_capsule_roundtrip());
        assert!(!construct.is_capsule_roundtrip());
    }
}
