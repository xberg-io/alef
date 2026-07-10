use crate::core::config::TraitBridgeConfig;
use crate::core::ir::TypeRef;
use ahash::AHashSet;
use minijinja::context;

pub(crate) struct PhpParamTypeSets<'a> {
    pub(crate) opaque: &'a AHashSet<String>,
    pub(crate) default: &'a AHashSet<String>,
    /// Flat/unit enum names (surfaced as String in PHP). Used to select the
    /// correct Vec<T> let-binding template (From<String> vs FromZval).
    pub(crate) enums: &'a AHashSet<String>,
}

pub(crate) struct PhpEnumReturnSets<'a> {
    pub(crate) string_enum_names: &'a AHashSet<String>,
    pub(crate) json_string_enum_names: &'a AHashSet<String>,
}

/// Format the `-> ReturnType` part of a function signature.
/// Returns an empty string for unit `()` return types to avoid
/// emitting `-> ()` which triggers `clippy::unused_unit`.
pub(super) fn return_type_sig(annotation: &str) -> String {
    if annotation == "()" {
        String::new()
    } else {
        format!(" -> {annotation}")
    }
}

/// For Bytes return types, ext-php-rs marshals Vec<u8> as a PHP array, not a string.
/// We need to override the return type to String for PHP binary-safe string handling.
/// Replaces "PhpResult<Vec<u8>>" with "PhpResult<String>" or "Vec<u8>" with "String".
pub(super) fn override_bytes_return_type(annotation: &str) -> String {
    annotation
        .replace("PhpResult<Vec<u8>>", "PhpResult<String>")
        .replace("Vec<u8>", "String")
}

/// Build the set of parameter names that are trait bridge params.
/// Bridge params are sanitized to a String/Option<String> in the IR but must be
/// passed as `None` to the core function (the PHP backend has no bridge implementation).
pub(super) fn bridge_param_names(bridges: &[TraitBridgeConfig]) -> AHashSet<&str> {
    bridges.iter().filter_map(|b| b.param_name.as_deref()).collect()
}

/// Replace the argument expression for each bridge param with `None` in the comma-separated
/// call args string.  The replacement is done term-by-term so partial-name matches are avoided.
pub(super) fn apply_bridge_none_substitutions(
    call_args: &str,
    func: &crate::core::ir::FunctionDef,
    bridge_names: &AHashSet<&str>,
) -> String {
    if bridge_names.is_empty() || call_args.is_empty() {
        return call_args.to_string();
    }
    let terms: Vec<&str> = call_args.split(", ").collect();
    let result: Vec<String> = terms
        .into_iter()
        .zip(func.params.iter())
        .map(|(term, param)| {
            if bridge_names.contains(param.name.as_str()) {
                "None".to_string()
            } else {
                term.to_string()
            }
        })
        .collect();
    result.join(", ")
}

pub(super) fn promoted_default_param_names<'a>(
    params: &'a [crate::core::ir::ParamDef],
    default_types: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
) -> AHashSet<&'a str> {
    params
        .iter()
        .filter_map(|param| match &param.ty {
            TypeRef::Named(name)
                if !param.optional
                    && default_types.contains(name.as_str())
                    && !opaque_types.contains(name.as_str()) =>
            {
                Some(param.name.as_str())
            }
            _ => None,
        })
        .collect()
}

pub(super) fn promote_default_params(
    params: &[crate::core::ir::ParamDef],
    default_types: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
) -> Vec<crate::core::ir::ParamDef> {
    params
        .iter()
        .map(|param| {
            let should_promote = matches!(
                &param.ty,
                TypeRef::Named(name)
                    if !param.optional
                        && default_types.contains(name.as_str())
                        && !opaque_types.contains(name.as_str())
            );
            if should_promote {
                let mut promoted = param.clone();
                promoted.optional = true;
                promoted
            } else {
                param.clone()
            }
        })
        .collect()
}

pub(super) fn apply_default_param_substitutions(
    call_args: &str,
    params: &[crate::core::ir::ParamDef],
    promoted_names: &AHashSet<&str>,
) -> String {
    if promoted_names.is_empty() || call_args.is_empty() {
        return call_args.to_string();
    }
    call_args
        .split(", ")
        .zip(params.iter())
        .map(|(term, param)| {
            if promoted_names.contains(param.name.as_str()) {
                let php_name = crate::codegen::naming::to_php_name(&param.name);
                if param.is_ref {
                    if param.is_mut {
                        format!("&mut {php_name}_unwrapped")
                    } else {
                        format!("&{php_name}_core.unwrap_or_default()")
                    }
                } else {
                    format!("{term}.unwrap_or_default()")
                }
            } else {
                term.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Returns true if any Named (non-opaque) param with `is_ref=true` is present.
/// These are the params that would fail the `.clone().into()` path when no `From` impl exists,
/// and for which the serde round-trip is a viable recovery path.
pub(super) fn has_ref_named_params(params: &[crate::core::ir::ParamDef], opaque_types: &AHashSet<String>) -> bool {
    params
        .iter()
        .any(|p| p.is_ref && matches!(&p.ty, TypeRef::Named(n) if !opaque_types.contains(n.as_str())))
}

/// Returns true if any param is a sanitized Vec<String> (originally Vec<tuple>) with
/// `original_type` recorded — meaning we can deserialize each item back to the tuple type.
pub(super) fn has_sanitized_recoverable(params: &[crate::core::ir::ParamDef]) -> bool {
    params.iter().any(|p| {
        p.sanitized
            && p.original_type.is_some()
            && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String))
    })
}

/// Generate serde-based let bindings for Named (non-opaque) params that have `is_ref=true`.
/// These replace the `.clone().into()` bindings when no `From` impl is available.
/// The round-trip works because PHP binding types derive `Serialize` and core types derive
/// `Deserialize`.
pub(super) fn gen_php_serde_let_bindings(
    params: &[crate::core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    core_import: &str,
) -> String {
    let mut out = String::new();
    for p in params {
        let php_param_name = crate::codegen::naming::to_php_name(&p.name);
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                if p.is_ref {
                    if p.optional {
                        out.push_str(&crate::backends::php::template_env::render(
                            "php_serde_ref_named_optional_let_binding.jinja",
                            context! {
                                pname => &php_param_name,
                                core_import => core_import,
                                name => name,
                            },
                        ));
                        if p.is_mut {
                            out.push_str(&crate::backends::php::template_env::render(
                                "php_optional_mut_unwrap_binding.jinja",
                                context! { php_name => &php_param_name },
                            ));
                        }
                    } else {
                        out.push_str(&crate::backends::php::template_env::render(
                            "php_serde_ref_named_let_binding.jinja",
                            context! {
                                pname => &php_param_name,
                                core_import => core_import,
                                name => name,
                            },
                        ));
                    }
                } else {
                    if p.optional {
                        out.push_str(&crate::backends::php::template_env::render(
                            "php_let_binding_named_optional.jinja",
                            minijinja::context! {
                                pname => &php_param_name,
                                core_import => core_import,
                                name => name,
                            },
                        ));
                    } else {
                        out.push_str(&crate::backends::php::template_env::render(
                            "php_let_binding_named.jinja",
                            minijinja::context! {
                                pname => &php_param_name,
                                core_import => core_import,
                                name => name,
                            },
                        ));
                    }
                }
            }
            TypeRef::Vec(inner) => {
                if let TypeRef::Named(name) = inner.as_ref() {
                    if !opaque_types.contains(name.as_str()) {
                        if enum_names.contains(name.as_str()) {
                            out.push_str(&crate::backends::php::template_env::render(
                                "php_let_binding_vec_named.jinja",
                                context! {
                                    pname => php_param_name.as_str(),
                                    core_import => core_import,
                                    name => name.as_str(),
                                },
                            ));
                        } else {
                            out.push_str(&crate::backends::php::template_env::render(
                                "php_vec_named_struct_let_binding.jinja",
                                context! {
                                    php_name => &php_param_name,
                                    core_import => core_import,
                                    struct_name => name,
                                    is_optional => p.optional,
                                },
                            ));
                        }
                    }
                } else if matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some() {
                    if p.optional {
                        out.push_str(&crate::backends::php::template_env::render(
                            "php_let_binding_sanitized_vec_string_optional.jinja",
                            context! {
                                pname => &php_param_name,
                            },
                        ));
                    } else {
                        out.push_str(&crate::backends::php::template_env::render(
                            "php_let_binding_sanitized_vec_string.jinja",
                            context! {
                                pname => &php_param_name,
                            },
                        ));
                    }
                } else if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref {
                    out.push_str(&crate::backends::php::template_env::render(
                        "php_let_binding_string_refs.jinja",
                        context! {
                            pname => &php_param_name,
                        },
                    ));
                }
            }
            TypeRef::Json => {
                let bound_name = format!("{php_param_name}_json");
                if p.optional {
                    out.push_str(&crate::backends::php::template_env::render(
                        "php_json_let_binding_optional.jinja",
                        context! {
                            php_name => &php_param_name,
                            bound_name => &bound_name,
                        },
                    ));
                } else {
                    out.push_str(&crate::backends::php::template_env::render(
                        "php_json_let_binding.jinja",
                        context! {
                            php_name => &php_param_name,
                            bound_name => &bound_name,
                        },
                    ));
                }
            }
            _ => {}
        }
    }
    out
}
